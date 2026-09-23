use v5.36;
use Test2::V0;

use Perldantic::Error;

subtest 'every error class is a Perldantic::Error' => sub {
    for my $class (qw(ValidationError SchemaError UsageError InternalError SerializationError)) {
        my $e = "Perldantic::$class"->new(message => "a $class");
        isa_ok $e, "Perldantic::$class", 'Perldantic::Error';
        is $e->message, "a $class";
        is "$e", "a $class", 'stringifies to its message';
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

done_testing;
