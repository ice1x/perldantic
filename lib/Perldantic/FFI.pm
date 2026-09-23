package Perldantic::FFI;

use v5.36;
use Carp ();
use Encode ();
use FFI::Platypus 2.00;
use FFI::Platypus::Buffer qw(scalar_to_buffer);

use Perldantic::Error;
use Perldantic::Wire;

our $VERSION = '0.01';

my $ffi = FFI::Platypus->new(api => 2, lang => 'Rust');
# The shared library belongs to the Perldantic distribution, not to this package.
$ffi->bundle('Perldantic');

$ffi->attach([pd_version => 'version'] => [] => 'string');
$ffi->attach([pd_validator_new => '_validator_new'] => ['string', 'string', 'opaque*'] => 'opaque');
$ffi->attach([pd_validator_free => '_validator_free'] => ['opaque'] => 'void');
$ffi->attach([pd_validator_validate => '_validator_validate'] => ['opaque', 'string', 'string'] => 'opaque');
$ffi->attach(
    [pd_validator_validate_json => '_validator_validate_json'] => ['opaque', 'opaque', 'usize', 'string'] =>
        'opaque');
$ffi->attach([pd_serializer_new => '_serializer_new'] => ['string', 'string', 'opaque*'] => 'opaque');
$ffi->attach([pd_serializer_free => '_serializer_free'] => ['opaque'] => 'void');
$ffi->attach([pd_serializer_to_python => '_serializer_to_python'] => ['opaque', 'string', 'string'] => 'opaque');
$ffi->attach([pd_serializer_to_json => '_serializer_to_json'] => ['opaque', 'string', 'string'] => 'opaque');
$ffi->attach([pd_json_schema => '_json_schema'] => ['string', 'string', 'string'] => 'opaque');
$ffi->attach([pd_string_free => '_string_free'] => ['opaque'] => 'void');

# Options the core takes as booleans; Perl callers pass any truth value.
my %FLAG = map { $_ => 1 } qw(
    strict from_attributes by_alias by_name exclude_unset exclude_defaults exclude_none
    serialize_as_any ensure_ascii
);
# Options that are a boolean or one of a few words.
my %FLAG_OR_WORD = (
    allow_partial => {map { $_ => 1 } qw(off on trailing-strings)},
    warnings      => {map { $_ => 1 } qw(none warn error)},
);

# pydantic's include/exclude filters in Perl terms: true leaves become `True`, integer-like keys
# become list indices, and arrays of names become sets.
sub _filter ($filter) {
    if (ref $filter eq 'HASH') {
        return Perldantic::Wire::ordered(map { (/\A-?[0-9]+\z/ ? 0 + $_ : $_) => _filter($filter->{$_}) } sort keys %$filter);
    }
    return Perldantic::Wire::set(map { /\A-?[0-9]+\z/ ? 0 + $_ : $_ } @$filter) if ref $filter eq 'ARRAY';
    return $filter if ref $filter || !defined $filter;
    return $filter ? !!1 : !!0;
}

sub _options ($options) {
    return undef if !defined $options;
    Perldantic::UsageError->throw(message => 'Options must be a hash reference')
        if ref $options ne 'HASH';
    my %wire;
    for my $name (keys %$options) {
        my $value = $options->{$name};
        if (defined $value && !ref $value
            && ($FLAG{$name} || ($FLAG_OR_WORD{$name} && !$FLAG_OR_WORD{$name}{$value})))
        {
            $value = $value ? !!1 : !!0;
        }
        $value = _filter($value) if $name eq 'include' || $name eq 'exclude';
        $wire{$name} = $value;
    }
    return Perldantic::Wire::encode(\%wire);
}

sub _optional ($value) {
    return defined $value ? Perldantic::Wire::encode($value) : undef;
}

# Take ownership of a result envelope: copy it out, free it and decode it.
sub _envelope ($ptr) {
    Perldantic::InternalError->throw(message => 'The core returned no result') if !$ptr;
    my $json = $ffi->cast(opaque => string => $ptr);
    _string_free($ptr);
    return Perldantic::Wire::decode($json);
}

# The `ok` value of an envelope, or die with the error it holds.
sub _unwrap ($envelope) {
    return $envelope if exists $envelope->{ok};
    if (my $error = $envelope->{validation_error}) {
        Perldantic::ValidationError->throw(%$error);
    }
    die Perldantic::Error->from_core($envelope->{error} // {message => 'Malformed result from the core'});
}

sub _compile ($new, $schema, $config) {
    my $error;
    my $handle = $new->(Perldantic::Wire::encode($schema), _optional($config), \$error);
    return $handle if $handle;
    _unwrap(_envelope($error));
    Perldantic::InternalError->throw(message => 'The core returned neither a handle nor an error');
}

sub json_schema ($schema, $config = undef, $options = undef) {
    my $result = _unwrap(_envelope(_json_schema(Perldantic::Wire::encode($schema), _optional($config), _options($options))));
    Carp::carp($_) for @{$result->{warnings} // []};
    return $result->{ok};
}

package Perldantic::FFI::Validator {

    sub new ($class, $schema, $config = undef) {
        return bless {handle => Perldantic::FFI::_compile(\&Perldantic::FFI::_validator_new, $schema, $config)}, $class;
    }

    sub validate ($self, $input, $options = undef) {
        my $envelope = Perldantic::FFI::_envelope(Perldantic::FFI::_validator_validate(
            $self->{handle}, Perldantic::Wire::encode($input), Perldantic::FFI::_options($options)));
        return Perldantic::FFI::_unwrap($envelope)->{ok};
    }

    sub validate_json ($self, $json, $options = undef) {
        $json = Encode::encode('UTF-8', $json) if utf8::is_utf8($json);
        my ($ptr, $len) = FFI::Platypus::Buffer::scalar_to_buffer($json);
        my $envelope = Perldantic::FFI::_envelope(Perldantic::FFI::_validator_validate_json(
            $self->{handle}, $ptr, $len, Perldantic::FFI::_options($options)));
        return Perldantic::FFI::_unwrap($envelope)->{ok};
    }

    sub DESTROY ($self) {
        Perldantic::FFI::_validator_free(delete $self->{handle}) if $self->{handle};
    }
}

package Perldantic::FFI::Serializer {

    sub new ($class, $schema, $config = undef) {
        return bless {handle => Perldantic::FFI::_compile(\&Perldantic::FFI::_serializer_new, $schema, $config)}, $class;
    }

    sub _call ($self, $function, $value, $options) {
        my $result = Perldantic::FFI::_unwrap(Perldantic::FFI::_envelope($function->(
            $self->{handle}, Perldantic::Wire::encode($value), Perldantic::FFI::_options($options))));
        Carp::carp($result->{warning}) if defined $result->{warning};
        return $result->{ok};
    }

    sub to_python ($self, $value, $options = undef) {
        return $self->_call(\&Perldantic::FFI::_serializer_to_python, $value, $options);
    }

    sub to_json ($self, $value, $options = undef) {
        return Encode::encode('UTF-8', $self->_call(\&Perldantic::FFI::_serializer_to_json, $value, $options));
    }

    sub DESTROY ($self) {
        Perldantic::FFI::_serializer_free(delete $self->{handle}) if $self->{handle};
    }
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::FFI - low-level binding to the perldantic Rust core

=head1 SYNOPSIS

    use Perldantic::FFI;

    my $validator = Perldantic::FFI::Validator->new({type => 'int'});
    my $n = $validator->validate('42');                  # 42
    my $m = $validator->validate_json('42', {strict => 1});

    my $serializer = Perldantic::FFI::Serializer->new({type => 'list', items_schema => {type => 'int'}});
    my $json = $serializer->to_json([1, 2], {indent => 2});

    my $js = Perldantic::FFI::json_schema({type => 'int'});   # {type => 'integer'}

=head1 DESCRIPTION

A thin layer over the C ABI of F<ffi/> (see F<ffi/include/perldantic.h>). Schemas are
pydantic C<core_schema> dicts written as Perl data, values travel in the wire format of
L<Perldantic::Wire>, and options take pydantic's keyword names. Options the core expects as
booleans accept any Perl truth value.

Failures are exception objects (L<Perldantic::Error>): invalid input raises
C<Perldantic::ValidationError>, a bad schema C<Perldantic::SchemaError>, a bad option or
argument C<Perldantic::UsageError>, a serialization failure C<Perldantic::SerializationError>
and a Rust panic C<Perldantic::InternalError>. Serializer and JSON Schema warnings are emitted
with C<warn>.

=head1 FUNCTIONS

=head2 version

The version of the bundled Rust core.

=head2 json_schema($schema, $config = undef, \%options = undef)

The JSON Schema of a core schema, as Perl data. C<$config> applies to the whole schema (like a
C<TypeAdapter>'s config); options are C<mode>, C<by_alias>, C<ref_template> and
C<union_format>.

=head1 CLASSES

=head2 Perldantic::FFI::Validator

=over

=item new($schema, $config = undef)

Compiles a validator.

=item validate($input, \%options = undef)

pydantic's C<validate_python>: validates Perl data and returns the validated data. Options:
C<strict>, C<extra>, C<from_attributes>, C<context>, C<allow_partial>, C<by_alias>,
C<by_name>.

=item validate_json($json, \%options = undef)

pydantic's C<validate_json>: validates JSON text (bytes or a character string).

=back

=head2 Perldantic::FFI::Serializer

=over

=item new($schema, $config = undef)

Compiles a serializer.

=item to_python($value, \%options = undef)

Serializes to Perl data. Options: C<mode>, C<include>, C<exclude>, C<by_alias>,
C<exclude_unset>, C<exclude_defaults>, C<exclude_none>, C<serialize_as_any>, C<warnings>.
C<include> and C<exclude> take pydantic's filters in Perl terms: hashes of names with true
values (integer-like keys are list indices), or arrays of names.

=item to_json($value, \%options = undef)

Serializes to UTF-8 encoded JSON; also takes C<indent> and C<ensure_ascii>.

=back

Both classes release their Rust handle when the object is destroyed.

=cut
