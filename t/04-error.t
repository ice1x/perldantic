use v5.36;
use Test2::V0;

use Perldantic::Error;

subtest 'every error class is a Perldantic::Error' => sub {
    for my $class (qw(ValidationError SchemaError UsageError InternalError SerializationError)) {
        my $e = "Perldantic::$class"->new(message => "a $class");
        isa_ok $e, "Perldantic::$class", 'Perldantic::Error';
        is $e->message, "a $class";
        if ($class eq 'UsageError') {
            like "$e", qr/^a UsageError at \S+ line \d+\.\n\z/, 'stringifies to its message and location';
        }
        else {
            is "$e", "a $class", 'stringifies to its message';
        }
    }
};

subtest 'throw dies with the object' => sub {
    my $e = dies { Perldantic::SchemaError->throw(message => 'bad schema', type => 'SchemaError') };
    isa_ok $e, 'Perldantic::SchemaError';
    is $e->type, 'SchemaError', 'keeps the exception name the core reported';
};

subtest 'validation errors carry pydantic error details' => sub {
    my $e = Perldantic::ValidationError->new(
        title   => 'int',
        message => "1 validation error for int\n  ...",
        errors  => [{type => 'int_parsing', loc => [], msg => 'bad', input => 'x'}],
    );
    is $e->title, 'int';
    is $e->error_count, 1;
    is $e->errors->[0]{type}, 'int_parsing';
    is Perldantic::ValidationError->new(message => 'm')->errors, [], 'errors default to empty';
};

subtest 'core error envelopes map onto the hierarchy' => sub {
    my %expected = (
        SchemaError                          => 'Perldantic::SchemaError',
        PydanticInvalidForJsonSchema         => 'Perldantic::SchemaError',
        TypeError                            => 'Perldantic::UsageError',
        ValueError                           => 'Perldantic::UsageError',
        KeyError                             => 'Perldantic::UsageError',
        UnicodeDecodeError                   => 'Perldantic::SerializationError',
        PydanticSerializationError           => 'Perldantic::SerializationError',
        PydanticSerializationUnexpectedValue => 'Perldantic::SerializationError',
        InternalError                        => 'Perldantic::InternalError',
        SomethingNew                         => 'Perldantic::InternalError',
    );
    for my $type (sort keys %expected) {
        my $e = Perldantic::Error->from_core({type => $type, message => "m $type"});
        isa_ok $e, $expected{$type};
        is $e->type, $type;
        is $e->message, "m $type";
    }
};

subtest 'ValidationError->json follows pydantic' => sub {
    my $e = Perldantic::ValidationError->new(
        title   => 'M',
        message => 'm',
        errors  => [
            {type => 'int_parsing', loc => ['a', 0], msg => 'Input should be a valid integer', input => "x\x{263a}",
                url => 'https://errors.pydantic.dev/2.12/v/int_parsing'},
            {type => 'greater_than', loc => ['b'], msg => 'Input should be greater than 0', input => -1,
                ctx => {gt => 0}, url => 'https://errors.pydantic.dev/2.12/v/greater_than'},
        ],
    );
    is $e->json,
        '[{"type":"int_parsing","loc":["a",0],"msg":"Input should be a valid integer",'
        . qq{"input":"x\x{263a}",}
        . '"url":"https://errors.pydantic.dev/2.12/v/int_parsing"},'
        . '{"type":"greater_than","loc":["b"],"msg":"Input should be greater than 0","input":-1,"ctx":{"gt":0},'
        . '"url":"https://errors.pydantic.dev/2.12/v/greater_than"}]',
        'keys in pydantic order, text (not bytes)';
    is $e->json(include_url => 0, include_context => 0, include_input => 0),
        '[{"type":"int_parsing","loc":["a",0],"msg":"Input should be a valid integer"},'
        . '{"type":"greater_than","loc":["b"],"msg":"Input should be greater than 0"}]';
    is Perldantic::ValidationError->new(errors => [{type => 't', loc => [], msg => 'm'}])->json(indent => 2),
        qq([\n  {\n    "type": "t",\n    "loc": [],\n    "msg": "m"\n  }\n]);
    my $u = dies { $e->json(bogus => 1) };
    isa_ok $u, 'Perldantic::UsageError';
    is $u->message, "json: unknown option 'bogus'";
};

subtest 'usage errors point at the caller' => sub {
    my $line = __LINE__ + 1;
    my $e = dies { Perldantic::UsageError->throw(message => 'wrong') };
    is $e->file, __FILE__;
    is $e->line, $line;
    is "$e", "wrong at @{[__FILE__]} line $line.\n", 'stringifies like die';

    require Perldantic::FFI;
    $line = __LINE__ + 1;
    $e = dies { Perldantic::FFI::Validator->new({type => 'int'})->validate(1, []) };
    is [$e->file, $e->line], [__FILE__, $line], 'frames inside Perldantic are skipped';
};

subtest 'internal errors keep their cause' => sub {
    require Perldantic::Wire;
    my $e = dies { Perldantic::Wire::decode('{') };
    isa_ok $e, 'Perldantic::InternalError';
    like $e->cause, qr/\S/, 'the underlying error';
    is Perldantic::InternalError->new(message => 'm')->cause, undef;
};

done_testing;
