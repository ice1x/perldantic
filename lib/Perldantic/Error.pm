package Perldantic::Error;

use v5.36;

our $VERSION = '0.01';

use Cpanel::JSON::XS ();

use overload
    '""'     => sub ($self, @) { $self->_as_string },
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

sub _as_string ($self) { $self->{message} }

# JSON text (characters) for plain Perl data; indent > 0 pretty-prints like serde.
my $JSON_VALUE = Cpanel::JSON::XS->new->allow_nonref->allow_bignum->allow_blessed->canonical;

sub _json ($data, $indent = 0, $level = 0) {
    my ($open, $sep, $close, $colon) = ('', ',', '', ':');
    if ($indent) {
        my $pad = ' ' x ($indent * ($level + 1));
        ($open, $sep, $close, $colon) = ("\n$pad", ",\n$pad", "\n" . ' ' x ($indent * $level), ': ');
    }
    if (ref $data eq 'ARRAY') {
        return '[]' if !@$data;
        return "[$open" . join($sep, map { _json($_, $indent, $level + 1) } @$data) . "$close]";
    }
    if (ref $data eq 'Perldantic::Error::Ordered' || ref $data eq 'HASH') {
        my @pairs = ref $data eq 'HASH' ? map { ($_, $data->{$_}) } sort keys %$data : @$data;
        return '{}' if !@pairs;
        my @items;
        while (my ($key, $value) = splice @pairs, 0, 2) {
            push @items, $JSON_VALUE->encode("$key") . $colon . _json($value, $indent, $level + 1);
        }
        return "{$open" . join($sep, @items) . "$close}";
    }
    return $JSON_VALUE->encode($data);
}

package Perldantic::ValidationError {
    our @ISA = ('Perldantic::Error');

    my %JSON_OPTION = map { $_ => 1 } qw(indent include_url include_context include_input);

    sub new ($class, %args) {
        return Perldantic::Error::new($class, errors => [], %args);
    }

    sub title ($self)       { $self->{title} }
    sub errors ($self)      { $self->{errors} }
    sub error_count ($self) { scalar @{$self->{errors}} }

    sub json ($self, %options) {
        for my $key (sort keys %options) {
            Perldantic::UsageError->throw(message => "json: unknown option '$key'") if !$JSON_OPTION{$key};
        }
        my %include = map { $_ => $options{"include_$_"} // 1 } qw(url context input);
        my @errors = map {
            my $e = $_;
            bless [
                type => $e->{type},
                loc  => $e->{loc},
                msg  => $e->{msg},
                ($include{input}   && exists $e->{input} ? (input => $e->{input}) : ()),
                ($include{context} && exists $e->{ctx}   ? (ctx   => $e->{ctx})   : ()),
                ($include{url}     && exists $e->{url}   ? (url   => $e->{url})   : ()),
            ], 'Perldantic::Error::Ordered';
        } @{$self->{errors}};
        return Perldantic::Error::_json(\@errors, $options{indent} // 0);
    }
}

package Perldantic::SchemaError { our @ISA = ('Perldantic::Error') }

package Perldantic::UsageError {
    our @ISA = ('Perldantic::Error');

    # Where the caller outside Perldantic made the mistake.
    sub new ($class, %args) {
        my $level = 0;
        while (my ($package, $file, $line) = caller $level++) {
            next if $package =~ /\APerldantic(?:::|\z)/;
            @args{qw(file line)} = ($file, $line);
            last;
        }
        return Perldantic::Error::new($class, %args);
    }

    sub file ($self) { $self->{file} }
    sub line ($self) { $self->{line} }

    sub _as_string ($self) {
        return $self->{message} if !defined $self->{file};
        return "$self->{message} at $self->{file} line $self->{line}.\n";
    }
}

package Perldantic::InternalError {
    our @ISA = ('Perldantic::Error');

    sub cause ($self) { $self->{cause} }
}
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

=head2 json(%options)

C<Perldantic::ValidationError> only: the errors as JSON text (a character string), like
pydantic's C<ValidationError.json()>, with keys in pydantic's order. Options: C<indent>, and
C<include_url>, C<include_context>, C<include_input> (all true by default).

=head2 file, line

C<Perldantic::UsageError> only: where the mistake was made, i.e. the first caller outside
Perldantic's own packages. A usage error stringifies like C<die>: C<"$message at $file line $line.\n">.

=head2 cause

C<Perldantic::InternalError> only: the underlying error, when there is one.

=cut
