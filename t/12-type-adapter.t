use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
use Perldantic::Types qw(:all);

package Geo::Point {
    use Perldantic;
    has x => (is => 'ro', isa => Num, required => 1);
    has y => (is => 'ro', isa => Num, required => 1);
}

package main;

subtest 'validate and validate_json' => sub {
    my $ints = Perldantic::TypeAdapter->new(ArrayRef[Int]);
    is $ints->validate([1, '2']), [1, 2];
    is $ints->validate_json('[3, 4]'), [3, 4];
    my $e = dies { $ints->validate(['x']) };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->title, 'ArrayRef[Int]', 'errors are titled with the Perl type name';
    like "$e", qr/^1 validation error for ArrayRef\[Int\]\n/;
    $e = dies { $ints->validate(['1'], strict => 1) };
    is $e->errors->[0]{type}, 'int_type', 'options are passed on';
};

subtest 'check' => sub {
    my $ints = Perldantic::TypeAdapter->new(ArrayRef[Int]);
    is $ints->check([1, '2']), T(), 'valid input';
    is $ints->check(['x']), F(), 'invalid input is false, not an error';
    is $ints->check(['1'], strict => 1), F(), 'options are passed on';
    ok $ints->check([1]) eq '1' && $ints->check(['x']) eq '', 'Perl booleans';
    my $points = Perldantic::TypeAdapter->new(ArrayRef['Geo::Point']);
    ok $points->check([{x => 1, y => 2}, Geo::Point->new(x => 0, y => 0)]), 'models, and model objects as input';
    ok !$points->check([{x => 1}]);
    my $e = dies { $ints->check([1], bogus => 1) };
    isa_ok $e, 'Perldantic::Error';    # bad options are still errors
    {
        local $Perldantic::Wire::XS = 0;
        is [$ints->check([1]), $ints->check(['x'])], [T(), F()], 'the same without the native encoder';
    }
};

subtest 'types holding models' => sub {
    my $path = Perldantic::TypeAdapter->new(ArrayRef['Geo::Point']);
    my $points = $path->validate([{x => 1, y => '2'}, Geo::Point->new(x => 0, y => 0)]);
    isa_ok $points->[0], 'Geo::Point';
    is $points->[0]->y, 2;
    is $path->dump($points), [{x => 1, y => 2}, {x => 0, y => 0}];
    is $path->dump_json($points), '[{"x":1.0,"y":2.0},{"x":0.0,"y":0.0}]';
    is $path->json_schema, {
        type  => 'array',
        items => {'$ref' => '#/$defs/Point'},
        '$defs' => {Point => {
            type => 'object', title => 'Geo::Point', required => [qw(x y)],
            properties => {x => {type => 'number', title => 'X'}, y => {type => 'number', title => 'Y'}},
        }},
    };
};

subtest 'a model class' => sub {
    my $adapter = Perldantic::TypeAdapter->new('Geo::Point');
    my $p = $adapter->validate_json('{"x": 1, "y": 2}');
    isa_ok $p, 'Geo::Point';
    is $adapter->dump($p, exclude => ['y']), {x => 1};
    is $adapter->json_schema->{title}, 'Geo::Point';
    my $e = dies { Perldantic::TypeAdapter->new('Geo::Point', config => {strict => 1}) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'TypeAdapter: config cannot be used with a model class; use model_config in Geo::Point';
};

subtest 'config' => sub {
    my $strict = Perldantic::TypeAdapter->new(Int, config => {strict => 1});
    my $e = dies { $strict->validate('1') };
    is $e->errors->[0]{type}, 'int_type';
    my $titled = Perldantic::TypeAdapter->new(Str, config => {title => 'Name', str_to_upper => 1});
    is $titled->validate('ann'), 'ANN';
    $e = dies { $titled->validate(5) };
    is $e->title, 'Name';
    $e = dies { Perldantic::TypeAdapter->new(Int, config => {frozen => 1}) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "TypeAdapter: config: unknown setting 'frozen'";
};

subtest 'dumping' => sub {
    my $maybe = Perldantic::TypeAdapter->new(Map[Str, Maybe[Num]]);
    is $maybe->dump({a => undef, b => 1.5}), {a => undef, b => 1.5};
    is $maybe->dump_json({b => 1.5}, indent => 1), qq({\n "b": 1.5\n});
    is $maybe->json_schema(mode => 'serialization'),
        {type => 'object', additionalProperties => {anyOf => [{type => 'number'}, {type => 'null'}]}};
};

subtest 'models declared after the adapter was built' => sub {
    package Geo::Late { use Perldantic; has a => (is => 'ro', isa => Int) }
    my $adapter = Perldantic::TypeAdapter->new(ArrayRef['Geo::Late']);
    is $adapter->validate([{a => 1}])->[0]->a, 1;
    Geo::Late::has(b => (is => 'ro', isa => Int, required => 1));
    my $e = dies { $adapter->validate([{a => 1}]) };
    is $e->errors->[0]{loc}, [0, 'b'], 'the adapter follows later declarations';
};

subtest 'usage errors' => sub {
    for my $case (
        [sub { Perldantic::TypeAdapter->new }, 'TypeAdapter->new takes a type'],
        [sub { Perldantic::TypeAdapter->new(sub {1}) }, 'TypeAdapter->new takes a Perldantic type or a model class, got CODE'],
        [sub { Perldantic::TypeAdapter->new(Int, bogus => 1) }, "TypeAdapter->new: unknown option 'bogus'"],
        [sub { Perldantic::TypeAdapter->new(Int)->validate(1, 'strict') },
            'validate takes options as key => value pairs'],
    ) {
        my $e = dies { $case->[0]->() };
        isa_ok $e, 'Perldantic::UsageError';
        is $e->message, $case->[1];
    }
};

done_testing;
