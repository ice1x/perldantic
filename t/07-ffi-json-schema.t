use v5.36;
use Test2::V0;

use Perldantic::FFI;
use Perldantic::Wire qw(bytes);

my $model_schema = {
    type   => 'model',
    cls    => 'Point',
    ref    => 'Point:1',
    schema => {
        type   => 'model-fields',
        fields => {
            x => {type => 'model-field', schema => {type => 'int'}},
            y => {
                type                => 'model-field',
                schema              => {type => 'default', schema => {type => 'int'}, default => 0},
                validation_alias    => 'why',
            },
        },
    },
};

subtest 'json_schema returns the JSON Schema as Perl data' => sub {
    is Perldantic::FFI::json_schema({type => 'int'}), {type => 'integer'};
    is Perldantic::FFI::json_schema($model_schema), {
        type       => 'object',
        title      => 'Point',
        properties => {
            x   => {type => 'integer', title => 'X'},
            why => {type => 'integer', title => 'Why', default => 0},
        },
        required => ['x'],
    };
};

subtest 'options and config' => sub {
    is Perldantic::FFI::json_schema($model_schema, undef, {by_alias => 0})->{properties}{y},
        {type => 'integer', title => 'Y', default => 0};
    is Perldantic::FFI::json_schema({type => 'int'}, {title => 'Count'}, {mode => 'serialization'}),
        {type => 'integer'};
    my $list = {type => 'list', items_schema => $model_schema};
    my $js = Perldantic::FFI::json_schema($list, undef, {ref_template => '#/components/{model}'});
    is $js->{items}, {'$ref' => '#/components/Point'};
    ok exists $js->{'$defs'}{Point}, '$-keys survive the wire';
};

subtest 'errors and warnings' => sub {
    my $e = dies { Perldantic::FFI::json_schema({type => 'int'}, undef, {mode => 'x'}) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "invalid value for 'mode': 'x'";
    $e = dies { Perldantic::FFI::json_schema({type => 'callable'}) };
    isa_ok $e, 'Perldantic::SchemaError';
    is $e->message, 'JSON Schema generation for `callable` schemas is not supported yet';

    my $js;
    my $warnings = warnings {
        $js = Perldantic::FFI::json_schema(
            {type => 'default', schema => {type => 'bytes'}, default => bytes("\xff")});
    };
    is $js, {type => 'string', format => 'binary'}, 'the default is left out';
    is scalar @$warnings, 1;
    like $warnings->[0], qr/^Default value b'\\xff' is not JSON serializable; excluding default from JSON schema \[non-serializable-default\] at /;

    my $inf = Perldantic::FFI::json_schema(
        {type => 'default', schema => {type => 'float'}, default => 9**9**9});
    is $inf->{default}, 9**9**9, 'infinite defaults are kept as numbers';
};

done_testing;
