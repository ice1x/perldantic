use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
use Perldantic::Types qw(InstanceOf ArrayRef Maybe Int);
use Perldantic::Wire;

package My::Shape { sub new ($class, %args) { bless {%args}, $class } }
package My::Point { our @ISA = ('My::Shape') }
package My::Other { sub new ($class) { bless [], $class } }

package main;

subtest 'objects of any class travel as themselves' => sub {
    my $point = My::Point->new(x => 1);
    my $json = Perldantic::Wire::encode($point);
    like $json, qr/\A\{"\$host":\{"class":"My::Point","id":\d+,"isa":\["My::Point","My::Shape"\],"repr":"My::Point=HASH/;
    ref_is Perldantic::Wire::decode($json), $point, 'decoded back into the object';
};

subtest 'InstanceOf[]' => sub {
    my $points = Perldantic::TypeAdapter->new(InstanceOf['My::Shape']);
    my $point = My::Point->new(x => 1);
    ref_is $points->validate($point), $point, 'instances of subclasses count';

    my $e = dies { $points->validate(My::Other->new) };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'is_instance_of';
    is $e->errors->[0]{msg}, 'Input should be an instance of My::Shape';
    ok lives { $e->message }, 'errors can show the object';

    $e = dies { $points->validate({x => 1}) };
    is $e->errors->[0]{type}, 'is_instance_of', 'a hash is no object';
    $e = dies { $points->validate_json('{}') };
    is $e->errors->[0]{type}, 'needs_python_object';

    my $list = Perldantic::TypeAdapter->new(ArrayRef[Maybe[InstanceOf['My::Point']]]);
    my $got = $list->validate([$point, undef]);
    ref_is $got->[0], $point;
    is $got->[1], undef;
    ref_is $list->dump([$point])->[0], $point, 'Perl dumps keep the object';
    $e = dies { $list->dump_json([$point]) };
    isa_ok $e, 'Perldantic::SerializationError';
    is $e->message, "Unable to serialize unknown type: <class 'My::Point'>";

    $e = dies { Perldantic::TypeAdapter->new(InstanceOf['My::Point'])->json_schema };
    isa_ok $e, 'Perldantic::SchemaError';
};

subtest 'classes Perldantic turns into data' => sub {
    for my $class (qw(DateTime Time::Moment DateTime::Duration URI Math::BigInt Math::BigFloat Perldantic::Url)) {
        my $e = dies { InstanceOf[$class] };
        isa_ok $e, 'Perldantic::UsageError';
        like $e->message, qr/\AInstanceOf\[\Q$class\E\]: \Q$class\E objects are sent as data/, $class;
    }
};

package Test::Canvas {
    use Perldantic;
    has shapes => (is => 'ro', isa => ArrayRef['My::Shape'], default => sub { [] });
    has origin => (is => 'ro', isa => 'My::Point');
}

package main;

subtest 'model fields' => sub {
    my $origin = My::Point->new(x => 0);
    my $canvas = Test::Canvas->new(origin => $origin, shapes => [$origin, My::Point->new(x => 2)]);
    ref_is $canvas->origin, $origin;
    is scalar @{$canvas->shapes}, 2;
    my $e = dies { Test::Canvas->new(origin => My::Other->new) };
    is $e->errors->[0]{loc}, ['origin'];
    ref_is $canvas->model_dump->{origin}, $origin;
};

SKIP: {
    skip 'Type::Tiny is not installed', 1 if !eval { require Types::Standard; 1 };

    subtest 'Type::Tiny constraints' => sub {
        my $tt_int = Types::Standard::Int();
        my $ints = Perldantic::TypeAdapter->new(ArrayRef[$tt_int]);
        is $ints->validate([1, '2']), [1, 2];
        my $e = dies { $ints->validate(['x']) };
        isa_ok $e, 'Perldantic::ValidationError';
        is $e->errors->[0]{type}, 'value_error';
        is $e->errors->[0]{loc}, [0];
        like $e->errors->[0]{msg}, qr/\AValue error, .*did not pass type constraint "Int"/;

        my $rounded = Types::Standard::Int()->plus_coercions(Types::Standard::Num(), sub { int $_ });
        is Perldantic::TypeAdapter->new($rounded)->validate(2.7), 2, 'coercions apply';

        package Test::TT {
            use Perldantic;
            has count => (is => 'ro', isa => Types::Standard::Int());
            has tags  => (is => 'ro', isa => Types::Standard::ArrayRef([Types::Standard::Str()]));
        }
        my $object = Test::TT->new(count => 3, tags => ['a']);
        is [$object->count, $object->tags], [3, ['a']];
        $e = dies { Test::TT->new(count => 'many', tags => []) };
        is $e->errors->[0]{loc}, ['count'];
        is Test::TT->model_json_schema->{properties}{count}, {title => 'Count'},
            'Type::Tiny constraints take any input in JSON Schema';
    };
}

done_testing;
