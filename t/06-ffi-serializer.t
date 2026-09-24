use v5.36;
use Test2::V0;

use Perldantic::FFI;
use Perldantic::Wire qw(bytes);

my $model_schema = {
    type   => 'model',
    cls    => 'My::User',
    schema => {
        type   => 'model-fields',
        fields => {
            name => {type => 'model-field', schema => {type => 'str'}},
            nick => {
                type   => 'model-field',
                schema => {type => 'nullable', schema => {type => 'str'}},
                serialization_alias => 'nickname',
            },
        },
    },
};
my $user = Perldantic::Wire::Model->new(
    class      => 'My::User',
    fields     => {name => "Z\x{f6}e", nick => undef},
    fields_set => ['name'],
);
my $ser = Perldantic::FFI::Serializer->new($model_schema);

subtest 'to_python returns Perl data' => sub {
    is $ser->to_perl($user), {name => "Z\x{f6}e", nick => undef};
    is $ser->to_perl($user, {by_alias => 1, exclude_none => 1}), {name => "Z\x{f6}e"};
    is $ser->to_perl($user, {exclude_unset => 1}), {name => "Z\x{f6}e"};
    is $ser->to_perl($user, {by_alias => 1}), {name => "Z\x{f6}e", nickname => undef};
    my $b = Perldantic::FFI::Serializer->new({type => 'bytes'});
    is $b->to_perl(bytes("\xff")), "\xff";
    is $b->to_perl(bytes("hi"), {mode => 'json'}), 'hi', 'json mode decodes bytes as UTF-8';
    my $e = dies { $b->to_perl(bytes("\xff"), {mode => 'json'}) };
    isa_ok $e, 'Perldantic::SerializationError';
    is $e->type, 'UnicodeDecodeError';
};

subtest 'to_json returns UTF-8 encoded JSON text' => sub {
    is $ser->to_json($user), qq({"name":"Z\xc3\xb6e","nick":null});
    is $ser->to_json($user, {ensure_ascii => 1}), '{"name":"Z\u00f6e","nick":null}';
    my $list = Perldantic::FFI::Serializer->new({type => 'list', items_schema => {type => 'int'}});
    is $list->to_json([1, 2], {indent => 2}), "[\n  1,\n  2\n]";
};

subtest 'unexpected values warn' => sub {
    my $int = Perldantic::FFI::Serializer->new({type => 'int'});
    my $got;
    my $warnings = warnings { $got = $int->to_perl('x') };
    is $got, 'x', 'the value is serialized as-is';
    is scalar @$warnings, 1;
    like $warnings->[0], qr/^Perldantic serializer warnings:\n  Expected `Int` - serialized value may not be as expected \[input_value='x', input_type=Str\]/;
    is warns { $int->to_perl('x', {warnings => 0}) }, 0, 'warnings => 0 silences them';
    my $e = dies { $int->to_perl('x', {warnings => 'error'}) };
    isa_ok $e, 'Perldantic::SerializationError';
    is $e->type, 'PydanticSerializationError';
    like $e->message, qr/Expected `Int`/;
};

subtest 'serialization errors are objects' => sub {
    my $f = Perldantic::FFI::Serializer->new({type => 'float'});
    my $u = dies { $f->to_perl(1.5, {nope => 1}) };
    isa_ok $u, 'Perldantic::UsageError';
    my $s = dies { Perldantic::FFI::Serializer->new({type => 'nope'}) };
    isa_ok $s, 'Perldantic::SchemaError';
};

done_testing;
