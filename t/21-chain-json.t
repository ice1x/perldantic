use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
use Perldantic::Types qw(Json Chain Str Int ArrayRef HashRef Date);

subtest 'Json[]' => sub {
    my $any = Perldantic::TypeAdapter->new(Json);
    is $any->validate_python('{"a": [1, null, true]}'), {a => [1, undef, !!1]};
    my $e = dies { $any->validate_python('{') };
    is $e->errors->[0]{type}, 'json_invalid';
    $e = dies { $any->validate_python([]) };
    is $e->errors->[0]{type}, 'json_type';

    my $ints = Perldantic::TypeAdapter->new(Json[ArrayRef[Int]]);
    is $ints->validate_python('[1, 2]'), [1, 2];
    $e = dies { $ints->validate_python('[1, "x"]') };
    is $e->errors->[0]{loc}, [1];
    is $ints->dump_json([1, 2]), '[1,2]', 'serialized as the data';
    is $ints->json_schema, {type => 'string', contentMediaType => 'application/json',
        contentSchema => {type => 'array', items => {type => 'integer'}}};
    isa_ok Perldantic::TypeAdapter->new(Json[Date])->validate_python('"2022-06-08"'), 'Perldantic::Date';
};

subtest 'Chain[]' => sub {
    my $trim = Str->with(strip_whitespace => 1);
    my $ta = Perldantic::TypeAdapter->new(Chain[$trim, Json[HashRef[Int]]]);
    is $ta->validate_python('  {"x": 1}  '), {x => 1};
    my $e = dies { $ta->validate_python(5) };
    is $e->errors->[0]{type}, 'string_type', 'the first step sees the input';
    is $ta->json_schema, {type => 'string'}, 'validation schema of the first step';
    my $single = Chain[Int];
    is $single->core_schema, {type => 'chain', steps => [{type => 'int'}]};
    $e = dies { Chain([]) };
    is $e->message, 'Chain[] takes at least 1 type';
};

package Test::Config {
    use Perldantic;
    use Perldantic::Types qw(Json HashRef Str);
    has settings => (is => 'ro', isa => Json[HashRef[Str]]);
}

package main;

subtest 'models' => sub {
    my $c = Test::Config->new(settings => '{"mode": "fast"}');
    is $c->settings, {mode => 'fast'};
    is $c->model_dump_json, '{"settings":{"mode":"fast"}}';
};

done_testing;
