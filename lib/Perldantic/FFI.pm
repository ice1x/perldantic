package Perldantic::FFI;

use v5.36;
use Carp ();
use Encode ();
use FFI::Platypus 2.00;
use FFI::Platypus::Buffer qw(scalar_to_buffer);
use Scalar::Util qw(blessed);

use Perldantic::Error;
use Perldantic::Info;
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
$ffi->attach([pd_url_parts => '_url_parts'] => ['string'] => 'opaque');
$ffi->attach([pd_string_free => '_string_free'] => ['opaque'] => 'void');
$ffi->type('(uint64,string,opaque)->void' => 'pd_host_callback');
$ffi->attach([pd_set_host_callback => '_set_host_callback'] => ['pd_host_callback'] => 'void');
$ffi->attach([pd_host_reply => '_host_reply'] => ['opaque', 'string'] => 'void');
$ffi->attach([pd_validator_handler_call => '_validator_handler_call'] => ['opaque', 'string', 'string'] => 'opaque');
$ffi->attach([pd_serializer_handler_call => '_serializer_handler_call'] => ['opaque', 'string', 'string'] => 'opaque');

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

# ---- functions in schemas ------------------------------------------------------------------
#
# The core calls Perl functions (code references in schemas, see Perldantic::Wire) through one
# callback. Exceptions a function raises that are not validation failures reach the caller of
# the core unchanged: they wait here, by id, until the core reports them back.

my %EXCEPTION;
my $LAST_EXCEPTION_ID = 0;
# How many calls into the core are running; exceptions left over when the outermost one ends
# were swallowed by the core (e.g. turned into a serialization error).
our $DEPTH = 0;

# A validator function that dies with a string reports a value error with that message.
sub _error_message ($text) {
    return $text =~ s/ at \S.* line \d+\.?\n\z//sr =~ s/\n\z//r;
}

# What the function raised, as the core's reply.
sub _host_error ($error) {
    my %reply;
    if (!ref $error) {
        %reply = (kind => 'value', message => _error_message($error));
    } elsif (blessed $error && $error->isa('Perldantic::ValidationError')) {
        %reply = (kind => 'validation', title => $error->title, errors => $error->errors);
    } elsif (blessed $error && $error->isa('Perldantic::CustomError')) {
        %reply = (kind => 'custom', error_type => $error->type, message_template => $error->message, context => $error->context);
    } elsif (blessed $error && $error->isa('Perldantic::KnownError')) {
        %reply = (kind => 'known', error_type => $error->type, context => $error->context);
    } elsif (blessed $error && $error->isa('Perldantic::Omit')) {
        %reply = (kind => 'omit');
    } elsif (blessed $error && $error->isa('Perldantic::UseDefault')) {
        %reply = (kind => 'use_default');
    } elsif (blessed $error && $error->isa('Perldantic::SerializationError')) {
        my $unexpected = ($error->type // '') eq 'PydanticSerializationUnexpectedValue';
        %reply = (kind => $unexpected ? 'unexpected_value' : 'serialization', message => $error->message);
    } else {
        my $id = ++$LAST_EXCEPTION_ID;
        $EXCEPTION{$id} = $error;
        %reply = (kind => 'other', message => _error_message("$error"), id => $id);
    }
    return Perldantic::Wire::encode({error => \%reply});
}

# A handler for the duration of one wrap call: it validates or serializes with the wrapped
# schema, and refuses to run once the call is over (the core's handler is gone by then).
sub _handler ($call, $address, $alive) {
    return sub ($value, $where = undef) {
        Perldantic::UsageError->throw(message => 'A handler can only be called while its function runs')
            if !$$alive;
        my $result = _unwrap(_envelope($call->(
            $address, Perldantic::Wire::encode($value), defined $where ? Perldantic::Wire::encode($where) : undef)));
        return $result->{ok};
    };
}

sub _info ($class, $info) {
    return () if !defined $info;
    return $class->new(%$info);
}

# Run the function the core called; returns the reply.
sub _host_call ($id, $json) {
    my $function = Perldantic::Wire::function($id)
        // Perldantic::InternalError->throw(message => "The core called function $id, which no longer exists");
    my $call = Perldantic::Wire::decode($json);
    my $kind = $call->{call};
    my $alive = 1;
    my @args;
    if ($kind eq 'validate') {
        @args = ($call->{input}, _info('Perldantic::ValidationInfo', $call->{info}));
    } elsif ($kind eq 'validate_wrap') {
        @args = ($call->{input}, _handler(\&_validator_handler_call, $call->{handler}, \$alive),
            _info('Perldantic::ValidationInfo', $call->{info}));
    } elsif ($kind eq 'serialize') {
        @args = ((defined $call->{model} ? $call->{model} : ()), $call->{value},
            _info('Perldantic::SerializationInfo', $call->{info}));
    } elsif ($kind eq 'serialize_wrap') {
        @args = ((defined $call->{model} ? $call->{model} : ()), $call->{value},
            _handler(\&_serializer_handler_call, $call->{handler}, \$alive),
            _info('Perldantic::SerializationInfo', $call->{info}));
    } else {
        Perldantic::InternalError->throw(message => "Unknown call `$kind` from the core");
    }
    my $result = eval { $function->(@args) };
    my $error = $@;
    $alive = 0;
    die $error if $error;
    return '{"ok":' . Perldantic::Wire::encode($result) . '}';
}

# The callback must always reply and never die: dying would unwind through Rust.
my $CALLBACK = $ffi->closure(sub ($id, $json, $reply) {
    my $answer = eval { _host_call($id, $json) };
    if (!defined $answer) {
        my $error = $@;
        $answer = eval { _host_error($error) };
    }
    $answer //= '{"error":{"kind":"value","message":"The error of a Perl function could not be reported"}}';
    _host_reply($reply, $answer);
});
$CALLBACK->sticky;
_set_host_callback($CALLBACK);

# Run a call into the core from Perl (not from a function the core called).
sub _enter ($body) {
    local $DEPTH = $DEPTH + 1;
    my ($result, $error);
    eval { $result = $body->(); 1 } or $error = $@;
    %EXCEPTION = () if $DEPTH == 1;
    die $error if defined $error;
    return $result;
}

# The `ok` value of an envelope, or die with the error it holds.
sub _unwrap ($envelope) {
    return $envelope if exists $envelope->{ok};
    if (my $error = $envelope->{validation_error}) {
        Perldantic::ValidationError->throw(%$error);
    }
    my $error = $envelope->{error} // {message => 'Malformed result from the core'};
    if (($error->{type} // '') eq 'HostException' && defined $error->{id}) {
        # a function's own exception, raised again unchanged
        my $exception = delete $EXCEPTION{$error->{id}};
        die $exception if defined $exception;
    }
    die Perldantic::Error->from_core($error);
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

# The accessors of a Perldantic::Url or Perldantic::MultiHostUrl, as pydantic computes them.
sub url_parts ($url) {
    return _unwrap(_envelope(_url_parts(Perldantic::Wire::encode($url))))->{ok};
}

package Perldantic::FFI::Validator {

    sub new ($class, $schema, $config = undef) {
        # the validator keeps the functions its schema holds alive
        local @Perldantic::Wire::FUNCTIONS;
        my $handle = Perldantic::FFI::_compile(\&Perldantic::FFI::_validator_new, $schema, $config);
        return bless {handle => $handle, functions => [@Perldantic::Wire::FUNCTIONS]}, $class;
    }

    sub validate ($self, $input, $options = undef) {
        return Perldantic::FFI::_enter(sub {
            my $envelope = Perldantic::FFI::_envelope(Perldantic::FFI::_validator_validate(
                $self->{handle}, Perldantic::Wire::encode($input), Perldantic::FFI::_options($options)));
            return Perldantic::FFI::_unwrap($envelope)->{ok};
        });
    }

    sub validate_json ($self, $json, $options = undef) {
        $json = Encode::encode('UTF-8', $json) if utf8::is_utf8($json);
        my ($ptr, $len) = FFI::Platypus::Buffer::scalar_to_buffer($json);
        return Perldantic::FFI::_enter(sub {
            my $envelope = Perldantic::FFI::_envelope(Perldantic::FFI::_validator_validate_json(
                $self->{handle}, $ptr, $len, Perldantic::FFI::_options($options)));
            return Perldantic::FFI::_unwrap($envelope)->{ok};
        });
    }

    sub DESTROY ($self) {
        Perldantic::FFI::_validator_free(delete $self->{handle}) if $self->{handle};
    }
}

package Perldantic::FFI::Serializer {

    sub new ($class, $schema, $config = undef) {
        local @Perldantic::Wire::FUNCTIONS;
        my $handle = Perldantic::FFI::_compile(\&Perldantic::FFI::_serializer_new, $schema, $config);
        return bless {handle => $handle, functions => [@Perldantic::Wire::FUNCTIONS]}, $class;
    }

    sub _call ($self, $function, $value, $options) {
        my $result = Perldantic::FFI::_enter(sub {
            Perldantic::FFI::_unwrap(Perldantic::FFI::_envelope($function->(
                $self->{handle}, Perldantic::Wire::encode($value), Perldantic::FFI::_options($options))));
        });
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

=head1 Functions

Code references in a schema are functions the core calls, as pydantic calls Python callables:
validators in C<function-before>, C<function-after>, C<function-plain> and C<function-wrap>
schemas, and serializers in a schema's C<serialization> (C<function-plain> or
C<function-wrap>). They get pydantic's arguments:

=over

=item * validators: C<($input)>, wrap validators C<($input, $handler)>;

=item * serializers: C<($value)>, wrap serializers C<($value, $handler)>; field serializers
get the model first;

=item * functions with C<< type => 'with-info' >> (validators) or C<< info_arg => 1 >>
(serializers) also get an info object last (L<Perldantic::Info>).

=back

A C<$handler> runs the wrapped schema: C<< $handler->($value) >>, or with a location for its
errors (validators) or a list index or dict key that C<include> / C<exclude> apply to
(serializers) as second argument. It only works while the function runs.

Validator functions report invalid input by dying with a string (a C<value_error> with that
message) or with the classes in L<Perldantic::Error> (C<Perldantic::CustomError>,
C<Perldantic::KnownError>, C<Perldantic::Omit>, C<Perldantic::UseDefault>); a
C<Perldantic::ValidationError>, e.g. from a handler, counts as those errors. Any other exception
object ends validation and reaches the caller unchanged. A serializer function's failure is
reported as a C<Perldantic::SerializationError> (C<Error calling function `name`: ...>), as in
pydantic.

A validator or serializer keeps the functions of its schema alive.

=head1 FUNCTIONS

=head2 version

The version of the bundled Rust core.

=head2 json_schema($schema, $config = undef, \%options = undef)

The JSON Schema of a core schema, as Perl data. C<$config> applies to the whole schema (like a
C<TypeAdapter>'s config); options are C<mode>, C<by_alias>, C<ref_template> and
C<union_format>.

=head2 url_parts($url)

The accessors of a L<Perldantic::Url> or L<Perldantic::MultiHostUrl> as a hash reference,
computed by the core as pydantic's C<Url> and C<MultiHostUrl> compute them.

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
