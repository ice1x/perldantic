use v5.36;
use Test2::V0;

use Perldantic::Arguments;
use Perldantic::FFI;
use Perldantic::Wire qw(tuple);

my $int = Perldantic::FFI::Validator->new({type => 'int'});

subtest 'validate returns validated data' => sub {
    is $int->validate(5), 5;
    is $int->validate('5'), 5, 'lax mode parses numeric strings';
    is $int->validate_json('42'), 42;
    is $int->validate_json("  7\n"), 7;
};

subtest 'invalid input raises a ValidationError' => sub {
    my $e = dies { $int->validate('five') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->title, 'int';
    is $e->error_count, 1;
    is $e->errors->[0]{type}, 'int_parsing';
    is $e->errors->[0]{input}, 'five';
    is $e->errors->[0]{loc}, [];
    like "$e", qr/^1 validation error for int\n/;
};

subtest 'options use pydantic names and Perl truth values' => sub {
    my $e = dies { $int->validate('5', {strict => 1}) };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'int_type';
    is $int->validate('5', {strict => 0}), 5;
    is $int->validate_json('5', {strict => undef}), 5;
    my $u = dies { $int->validate(5, {strictly => 1}) };
    isa_ok $u, 'Perldantic::UsageError';
    is $u->type, 'TypeError';
    is $u->message, "got an unexpected keyword argument 'strictly'";
};

subtest 'models are validated into wire models' => sub {
    my $point = Perldantic::FFI::Validator->new({
        type   => 'model',
        cls    => 'My::Point',
        schema => {
            type   => 'model-fields',
            fields => {
                x => {type => 'model-field', schema => {type => 'int'}},
                y => {
                    type   => 'model-field',
                    schema => {type => 'default', schema => {type => 'int'}, default => 0},
                },
            },
        },
    });
    my $m = $point->validate({x => '3'});
    isa_ok $m, 'Perldantic::Wire::Model';
    is $m->class, 'My::Point';
    is $m->fields, {x => 3, y => 0};
    is $m->fields_set, ['x'];
    my $e = dies { $point->validate_json('{"x": "a", "y": []}') };
    is [map { join '.', @{$_->{loc}} } @{$e->errors}], ['x', 'y'];
    is $e->title, 'My::Point';
};

subtest 'tuples and byte strings reach the core' => sub {
    my $pair = Perldantic::FFI::Validator->new(
        {type => 'tuple', items_schema => [{type => 'int'}, {type => 'bytes'}]});
    is $pair->validate(tuple(1, 'ab')), [1, 'ab'];
    is $pair->validate_json(qq{[1, "\xc3\xa9"]}), [1, "\xc3\xa9"], 'bytes come back as octets';
};

subtest 'Perl data is reported in Perl words' => sub {
    my $e = dies { Perldantic::FFI::Validator->new({type => 'list'})->validate({}) };
    is $e->errors->[0]{msg}, 'Input should be an array reference';
    $e = dies { Perldantic::FFI::Validator->new({type => 'dict', min_length => 1})->validate({}) };
    is $e->errors->[0]{msg}, 'Hash should have at least 1 item after validation, not 0';
    $e = dies { Perldantic::FFI::Validator->new({type => 'none'})->validate(1) };
    is $e->errors->[0]{msg}, 'Input should be undef';
    $e = dies { Perldantic::FFI::Validator->new({type => 'list'})->validate({}, {input_type => 'python'}) };
    is $e->errors->[0]{msg}, 'Input should be a valid list', "input_type => 'python' keeps pydantic's words";
    $e = dies { Perldantic::FFI::Validator->new({type => 'list'})->validate_json('{}') };
    is $e->errors->[0]{msg}, 'Input should be a valid array', 'JSON input keeps JSON words';
    my $pair = Perldantic::FFI::Validator->new({type => 'tuple', items_schema => [{type => 'int'}]});
    is $pair->validate([1], {strict => 1}), [1], 'arrays are strict tuples';
};

subtest 'the arguments of a call' => sub {
    my $v = Perldantic::FFI::Validator->new({type => 'arguments', arguments_schema => [
        {name => 'a', mode => 'positional_only', schema => {type => 'int'}},
        {name => 'b', schema => {type => 'default', schema => {type => 'int'}, default => 0}},
    ]});
    is $v->validate(Perldantic::Arguments->new(args => ['1'], kwargs => {b => '2'})), [[1], {b => 2}],
        'Perldantic::Arguments';
    is $v->validate([1, 2]), [[1, 2], {}], 'an array: positional arguments';
    is $v->validate_json('[1]'), [[1], {b => 0}], 'JSON';
    my $e = dies { $v->validate({b => 1}) };
    is $e->errors->[0]{type}, 'missing_positional_only_argument', 'a hash: named arguments';
    $e = dies { $v->validate(1) };
    is $e->errors->[0]{msg}, 'Arguments must be an array reference, a hash reference or a Perldantic::Arguments object';
    $e = dies { $v->validate(Perldantic::Arguments->new(args => [1], kwargs => {c => 1})) };
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['unexpected_keyword_argument', ['c']]];
};

subtest 'bad schemas raise a SchemaError' => sub {
    my $e = dies { Perldantic::FFI::Validator->new({type => 'nope'}) };
    isa_ok $e, 'Perldantic::SchemaError';
    like $e->message, qr/nope/;
    $e = dies { Perldantic::FFI::Validator->new({type => 'int'}, {strict => 'yes'}) };
    isa_ok $e, 'Perldantic::SchemaError';
    like $e->message, qr/'strict' should be a bool, got str/, 'bad config values too';
};

subtest 'validators are freed' => sub {
    my $v = Perldantic::FFI::Validator->new({type => 'str'});
    ok $v->{handle}, 'holds a handle';
    $v->DESTROY;
    ok !$v->{handle}, 'DESTROY releases it once';
    $v->DESTROY;
    pass 'a second DESTROY is harmless';
};

done_testing;
