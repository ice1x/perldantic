use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
use Perldantic::Types qw(Dict Optional Maybe Str Int ArrayRef HashRef slurpy);

subtest 'hash references with known keys' => sub {
    my $ta = Perldantic::TypeAdapter->new(Dict[name => Str, age => Optional[Int]]);
    is $ta->validate_python({name => 'Ada', age => '36'}), {name => 'Ada', age => 36};
    is $ta->validate_python({name => 'Ada'}), {name => 'Ada'}, 'an Optional[] key may be left out';
    is $ta->validate_python({name => 'Ada', admin => 1}), {name => 'Ada'}, 'unknown keys are dropped';
    is $ta->validate_json('{"name": "Ada", "age": 36}'), {name => 'Ada', age => 36};

    my $e = dies { $ta->validate_python({age => 'old'}) };
    isa_ok $e, 'Perldantic::ValidationError';
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}],
        [['missing', ['name']], ['int_parsing', ['age']]], 'errors follow the declared key order';
    $e = dies { $ta->validate_python([name => 'Ada']) };
    is $e->errors->[0]{type}, 'dict_type';
    is $e->errors->[0]{msg}, 'Input should be a hash reference', 'Perl wording (docs/DIVERGENCES.md #8)';
};

subtest 'declared key order' => sub {
    my $ta = Perldantic::TypeAdapter->new(Dict[zeta => Int, alpha => Int]);
    my $e = dies { $ta->validate_python({}) };
    is [map { $_->{loc}[0] } @{$e->errors}], ['zeta', 'alpha'];
    is $ta->json_schema->{required}, ['zeta', 'alpha'];
};

subtest 'dumping' => sub {
    my $ta = Perldantic::TypeAdapter->new(Dict[name => Str, tags => ArrayRef[Str]]);
    my $data = {name => 'Ada', tags => ['math']};
    is $ta->dump_python($data), $data;
    is $ta->dump_json($data), '{"name":"Ada","tags":["math"]}';
    is $ta->dump_python($data, exclude => {tags => 1}), {name => 'Ada'};
    is $ta->dump_python($data, include => {tags => 1}), {tags => ['math']};
    is $ta->dump_json({name => 'Ada', tags => [], extra => 1}), '{"name":"Ada","tags":[]}',
        'unknown keys are not dumped';

    my $maybe = Perldantic::TypeAdapter->new(Dict[name => Str, nick => Optional[Maybe[Str]]]);
    is $maybe->dump_json({name => 'Ada', nick => undef}, exclude_none => 1), '{"name":"Ada"}';
};

subtest 'strict mode' => sub {
    my $type = Dict[age => Int];
    my $ta = Perldantic::TypeAdapter->new($type);
    my $e = dies { $ta->validate_python({age => '36'}, strict => 1) };
    is $e->errors->[0]{type}, 'int_type', 'strict validation reaches the keys';
    is $ta->validate_python({age => 36}, strict => 1), {age => 36};
    my $strict = Perldantic::TypeAdapter->new($type->with(strict => 1));
    is $strict->validate_python({age => '36'}), {age => 36},
        'the strict constraint applies to the hash itself, as in pydantic';
    is $type->with(strict => 1)->core_schema->{strict}, T();
};

subtest 'unknown keys' => sub {
    my $type = Dict[name => Str];
    my $forbid = Perldantic::TypeAdapter->new($type->with(extra_behavior => 'forbid'));
    my $e = dies { $forbid->validate_python({name => 'Ada', admin => 1}) };
    is $e->errors->[0]{type}, 'extra_forbidden';
    is $e->errors->[0]{loc}, ['admin'];
    is $forbid->json_schema->{additionalProperties}, F();

    my $any = Perldantic::TypeAdapter->new(Dict[name => Str, slurpy HashRef]);
    is $any->validate_python({name => 'Ada', admin => 1}), {name => 'Ada', admin => 1},
        'slurpy HashRef keeps unknown keys';
    is $any->dump_json({name => 'Ada', admin => 1}), '{"admin":1,"name":"Ada"}',
        'hash keys are dumped sorted (docs/DIVERGENCES.md #9)';

    my $ints = Perldantic::TypeAdapter->new(Dict[name => Str, slurpy HashRef[Int]]);
    is $ints->validate_python({name => 'Ada', level => '3'}), {name => 'Ada', level => 3},
        'slurpy HashRef[T] validates unknown keys as T';
    $e = dies { $ints->validate_python({name => 'Ada', level => 'high'}) };
    is $e->errors->[0]{loc}, ['level'];
    is $ints->json_schema->{additionalProperties}, {type => 'integer'};
};

subtest 'usage errors' => sub {
    for (
        [sub { Dict[a => Int, slurpy ArrayRef] }, 'Dict[] takes slurpy HashRef or HashRef[T], got slurpy ArrayRef'],
        [sub { Tuple_with_hash() },                'Tuple[] takes slurpy ArrayRef or ArrayRef[T], got slurpy HashRef'],
        [sub { slurpy Int },                       'slurpy takes ArrayRef, ArrayRef[T], HashRef or HashRef[T], got Int'],
        [sub { my $type = Dict[a => Int]; $type->with(extra_behavior => 'loose') },
            "extra_behavior takes 'allow', 'ignore' or 'forbid', got 'loose'"],
    ) {
        my ($code, $message) = @$_;
        my $e = dies { $code->() };
        isa_ok $e, 'Perldantic::UsageError';
        is $e->message, $message;
    }
};

sub Tuple_with_hash { Perldantic::Types::Tuple([Int, slurpy HashRef]) }

package Test::Person {
    use Perldantic;
    use Perldantic::Types qw(Dict Str Int Optional);
    has name    => (is => 'ro', isa => Str);
    has address => (is => 'ro', isa => Dict[city => Str, zip => Optional[Str]]);
}

package main;

subtest 'model fields' => sub {
    my $person = Test::Person->new(name => 'Ada', address => {city => 'London', zip => 'N1'});
    is $person->address, {city => 'London', zip => 'N1'};
    is $person->model_dump_json, '{"name":"Ada","address":{"city":"London","zip":"N1"}}';
    my $e = dies { Test::Person->new(name => 'Ada', address => {zip => 'N1'}) };
    is $e->errors->[0]{loc}, ['address', 'city'];
    is Test::Person->model_json_schema->{properties}{address}, {
        type       => 'object',
        title      => 'Address',
        properties => {city => {type => 'string', title => 'City'}, zip => {type => 'string', title => 'Zip'}},
        required   => ['city'],
    };
};

done_testing;
