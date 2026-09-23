package Perldantic::Error;

use v5.36;

our $VERSION = '0.01';

use overload
    '""'     => sub ($self, @) { $self->message },
    bool     => sub { 1 },
    fallback => 1;

# Exception names the core reports (pydantic's Python exceptions) and the classes they become.
my %CLASS_OF = (
    SchemaError                          => 'Perldantic::SchemaError',
    PydanticInvalidForJsonSchema         => 'Perldantic::SchemaError',
    TypeError                            => 'Perldantic::UsageError',
    ValueError                           => 'Perldantic::UsageError',
    KeyError                             => 'Perldantic::UsageError',
    UnicodeDecodeError                   => 'Perldantic::SerializationError',
    PydanticSerializationError           => 'Perldantic::SerializationError',
    PydanticSerializationUnexpectedValue => 'Perldantic::SerializationError',
    InternalError                        => 'Perldantic::InternalError',
);

sub new ($class, %args) {
    return bless {%args, message => $args{message} // $class}, $class;
}

sub throw ($class, %args) {
    die $class->new(%args);
}

sub from_core ($class, $error) {
    my $target = $CLASS_OF{$error->{type} // ''} // 'Perldantic::InternalError';
    return $target->new(type => $error->{type}, message => $error->{message});
}

sub message ($self) { $self->{message} }
sub type ($self)    { $self->{type} }

package Perldantic::ValidationError {
    our @ISA = ('Perldantic::Error');

    sub new ($class, %args) {
        return Perldantic::Error::new($class, errors => [], %args);
    }

    sub title ($self)       { $self->{title} }
    sub errors ($self)      { $self->{errors} }
    sub error_count ($self) { scalar @{$self->{errors}} }
}

package Perldantic::SchemaError        { our @ISA = ('Perldantic::Error') }
package Perldantic::UsageError         { our @ISA = ('Perldantic::Error') }
package Perldantic::InternalError      { our @ISA = ('Perldantic::Error') }
package Perldantic::SerializationError { our @ISA = ('Perldantic::Error') }

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Error - exception objects raised by Perldantic

=head1 SYNOPSIS

    use Scalar::Util qw(blessed);

    eval { $validator->validate($input) };
    if (blessed $@ && $@->isa('Perldantic::ValidationError')) {
        for my $error (@{ $@->errors }) {
            say join('.', @{ $error->{loc} }), ": $error->{msg}";
        }
    }

=head1 DESCRIPTION

Perldantic never dies with a plain string: every failure is one of these objects. They
stringify to their message, so an uncaught one still prints a readable error.

=head2 Classes

=over

=item C<Perldantic::Error>

The base class, never thrown directly.

=item C<Perldantic::ValidationError>

Input does not match the schema. C<errors> holds pydantic's error details (C<type>, C<loc>,
C<msg>, C<input>, C<ctx>, C<url>), and the message is pydantic's C<str(ValidationError)>.

=item C<Perldantic::SchemaError>

A schema is invalid, or cannot be turned into a JSON Schema.

=item C<Perldantic::UsageError>

The API was called incorrectly, e.g. with an unknown option or a value the core cannot take.

=item C<Perldantic::SerializationError>

A value could not be serialized.

=item C<Perldantic::InternalError>

The Rust core panicked or the FFI boundary failed.

=back

=head1 METHODS

=head2 new(%args)

Builds an error; C<message> defaults to the class name.

=head2 throw(%args)

Builds an error and dies with it.

=head2 from_core($error)

Builds the error for an C<{type, message}> error envelope of the core. C<type> is the name of
the exception pydantic would raise and picks the class; unknown names become
C<Perldantic::InternalError>.

=head2 message

The message.

=head2 type

The name of the exception the core reported (C<SchemaError>, C<TypeError>, ...), if any.

=head2 title, errors, error_count

C<Perldantic::ValidationError> only: the title (the model or type name), the list of error
details and its length.

=cut
