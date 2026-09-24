package Perldantic::Wire;

use v5.36;
no warnings 'experimental::builtin';
# deeply nested data is written recursively
no warnings 'recursion';

our $VERSION = '0.01';

use B ();
use Cpanel::JSON::XS ();
use Exporter 'import';
use Hash::Util::FieldHash ();
use Math::BigFloat ();
use MIME::Base64 qw(encode_base64 decode_base64);
use Scalar::Util qw(blessed reftype weaken);
use Sub::Util ();

use Perldantic::Error;
use Perldantic::Temporal;
use Perldantic::Url;
use Perldantic::Uuid;

our @EXPORT_OK = qw(tuple set frozenset bytes ordered);

my $JSON = Cpanel::JSON::XS->new->utf8->canonical->allow_nonref->allow_bignum->unblessed_bool;
# The decoder of the core's JSON, set up with %UNTAG below.
my $DECODER;

# Code references travel as host functions the core calls back (Perldantic::FFI), by their id.
# The registry is a field hash: it holds functions weakly and forgets them when they are freed.
# Whoever compiles a schema keeps its functions alive; @FUNCTIONS collects the ones an encode
# call met for that purpose.
Hash::Util::FieldHash::fieldhash(my %FUNCTION);
our @FUNCTIONS;
# Set while a schema is compiled: only then are the functions and objects met collected.
our $COLLECT;

sub _function_id ($code) {
    push @FUNCTIONS, $code if $COLLECT;
    weaken($FUNCTION{$code} = $code) if !exists $FUNCTION{$code};
    return Hash::Util::FieldHash::id($code);
}

# The function registered under an id, if it is still alive.
sub function ($id) { $FUNCTION{$id} }

# Objects of other classes travel as host objects the core hands back unchanged (only their
# class matters to it), by their id; the registry works as for functions, and @OBJECTS collects
# the objects an encode call met (a compiled schema keeps those of its defaults alive).
Hash::Util::FieldHash::fieldhash(my %OBJECT);
our @OBJECTS;

sub _host_object ($object, $class) {
    push @OBJECTS, $object if $COLLECT;
    weaken($OBJECT{$object} = $object) if !exists $OBJECT{$object};
    my $repr = eval { "$object" } // "$class object";
    return {
        id    => Hash::Util::FieldHash::id($object),
        class => $class,
        isa   => [@{mro::get_linear_isa($class)}],
        repr  => $repr,
    };
}

# The object registered under an id, if it is still alive.
sub object ($id) { $OBJECT{$id} }

# A function's name without its package, as pydantic shows `__name__`.
sub _function_name ($code) {
    my $name = Sub::Util::subname($code) // '__ANON__';
    return $name =~ s/\A.*:://sr;
}

sub tuple (@items) { bless [@items], 'Perldantic::Wire::Tuple' }
sub set (@items)   { bless [@items], 'Perldantic::Wire::Set' }
sub frozenset (@items) { bless [@items], 'Perldantic::Wire::FrozenSet' }
sub bytes ($octets) { bless \(my $copy = $octets), 'Perldantic::Wire::Bytes' }

sub ordered (@pairs) {
    Perldantic::UsageError->throw(message => 'ordered() takes key => value pairs') if @pairs % 2;
    return bless [@pairs], 'Perldantic::Wire::Ordered';
}

# The native encoder (Perldantic.xs) writes plain data; objects and other values it hands
# back to _emit. Without it, _emit writes everything.
our $XS = eval { require XSLoader; XSLoader::load('Perldantic', $VERSION); 1 };

# Wire JSON of any value: natively when the encoder is built.
sub _emit_any ($value) {
    return $XS ? Perldantic::XS::encode($value, \&_emit) : _emit($value);
}

# A JSON object with string keys in the given order: [key, value, ...].
sub _object_any ($pairs) {
    return Perldantic::XS::encode_pairs($pairs, \&_emit) if $XS;
    my @entries;
    for (my $i = 0; $i < @$pairs; $i += 2) {
        push @entries, _string($pairs->[$i]) . ':' . _emit($pairs->[$i + 1]);
    }
    return '{' . join(',', @entries) . '}';
}

sub encode ($value) {
    my $json = eval { _emit_any($value) };
    if (!defined $json) {
        my $e = $@;
        die $e if blessed $e && $e->isa('Perldantic::Error');
        Perldantic::InternalError->throw(message => "Cannot encode a value for the core: $e", cause => $e);
    }
    return $json;
}

# Strings only: JSON escaping and UTF-8 encoding.
my $STRING = Cpanel::JSON::XS->new->utf8->allow_nonref;

# The shortest decimal form that reads back as the same double, always with a fraction or an
# exponent so that the core sees a float. (JSON modules print 15 significant digits, which
# changes values such as 0.1 + 0.2.)
sub _float ($value) {
    my $text;
    # `%g` drops trailing zeros: 15 digits give the shortest form of normal values that need
    # fewer; subnormal ones have less precision and are tried from 1 digit
    my $from = $value != 0 && abs($value) < 2.2250738585072014e-308 ? 1 : 15;
    for my $digits ($from .. 17) {
        $text = sprintf "%.${digits}g", $value;
        last if $text == $value;
    }
    return $text =~ /[.eE]/ ? $text : "$text.0";
}

# JSON text of a string; plain ASCII needs no escaping.
sub _string ($text) {
    return $text =~ /[^\x20-\x21\x23-\x5b\x5d-\x7e]/ ? $STRING->encode("$text") : qq("$text");
}

sub _list ($items) {
    return '[' . join(',', map { _emit_any($_) } @$items) . ']';
}

sub _pairs (@pairs) {
    my @entries;
    while (my ($key, $value) = splice @pairs, 0, 2) {
        push @entries, '[' . _emit_any($key) . ',' . _emit_any($value) . ']';
    }
    return '{"$dict":[' . join(',', @entries) . ']}';
}

sub _tagged ($tag, $json) { qq({"\$$tag":$json}) }

# A dict in a given key order: a JSON object keeps it (the core reads keys in order) when all
# keys are plain strings (not numbers or booleans, which would become strings).
sub _ordered ($pairs) {
    my @entries;
    for (my $i = 0; $i < @$pairs; $i += 2) {
        my $key = $pairs->[$i];
        return _pairs(@$pairs)
            if ref $key || !defined $key || builtin::is_bool($key) || builtin::created_as_number($key)
            || $key =~ /\A\$/;
        push @entries, _string($key) . ':' . _emit_any($pairs->[$i + 1]);
    }
    return '{' . join(',', @entries) . '}';
}

# Write a Perl value as wire JSON, in one pass: plain data as JSON (hashes with sorted keys,
# scalars by their Perl kind), everything JSON cannot express as tagged objects.
sub _emit ($value) {
    my $ref = ref $value;
    if (!$ref) {
        return 'null' if !defined $value;
        return $value ? 'true' : 'false' if builtin::is_bool($value);
        return _string($value) if !builtin::created_as_number($value);
        my $flags = B::svref_2object(\$value)->FLAGS;
        return "$value" if $flags & B::SVf_IOK;
        return _tagged(float => '"nan"') if $value != $value;
        return _tagged(float => $value > 0 ? '"inf"' : '"-inf"') if $value * 0 != 0;
        return _float($value);
    }
    return _list($value) if $ref eq 'ARRAY';
    if ($ref eq 'HASH') {
        my @keys = sort keys %$value;
        return _pairs(map { ($_ => $value->{$_}) } @keys) if grep {/^\$/} @keys;
        return '{' . join(',', map { _string($_) . ':' . _emit_any($value->{$_}) } @keys) . '}';
    }
    if (blessed $value) {
        # models first: the objects met most often
        return Perldantic::Model::_wire_json($value)          if $value->isa('Perldantic::Model');
        return _tagged(tuple => _list($value))                if $ref eq 'Perldantic::Wire::Tuple';
        return _tagged(set => _list($value))                  if $ref eq 'Perldantic::Wire::Set';
        return _tagged(frozenset => _list($value))            if $ref eq 'Perldantic::Wire::FrozenSet';
        return _ordered($value)                               if $ref eq 'Perldantic::Wire::Ordered';
        return _tagged(bytes => _string(encode_base64($$value, ''))) if $ref eq 'Perldantic::Wire::Bytes';
        return _tagged(model => $value->_json)                if $ref eq 'Perldantic::Wire::Model';
        return _tagged(enum => $value->_json)                 if $ref eq 'Perldantic::Wire::Enum';
        return qq({"@{[$value->_wire_tag]}":) . _emit_any($value->_wire_payload) . '}' if $value->isa('Perldantic::Temporal');
        return _tagged(uuid => _string($value->as_string))    if $value->isa('Perldantic::Uuid');
        return _tagged(multi_host_url => _string($value->as_string)) if $value->isa('Perldantic::MultiHostUrl');
        return _tagged(url => _string($value->as_string))     if $value->isa('Perldantic::Url');
        return _string($value->as_string)                     if $value->isa('URI');
        return _tagged(datetime => _string(_datetime_iso($value)))     if $value->isa('DateTime');
        return _tagged(datetime => _string(_time_moment_iso($value)))  if $value->isa('Time::Moment');
        return _emit_any(_duration_parts($value))                 if $value->isa('DateTime::Duration');
        return _emit_any($value->_perldantic_wire)                if $value->can('_perldantic_wire');
        return $value ? 'true' : 'false'
            if $value->isa('JSON::PP::Boolean') || $value->isa('Types::Serialiser::Boolean');
        return _tagged(decimal => _string(_decimal_text($value))) if $value->isa('Math::BigFloat');
        return $value->bstr                                   if $value->isa('Math::BigInt');
        return _tagged(host => _emit_any(_host_object($value, $ref)));
    }
    return _tagged(function => _emit_any({id => _function_id($value), name => _function_name($value)}))
        if $ref eq 'CODE';
    _cannot("a $ref reference", "$ref has no wire form");
}

sub decode ($json) {
    my $data = eval { $DECODER->decode($json) };
    Perldantic::InternalError->throw(message => "Malformed JSON from the core: $@", cause => $@)
        if !defined $data && $@;
    return $data;
}

sub _cannot ($value, $why) {
    Perldantic::UsageError->throw(message => "Cannot pass $value to the core: $why");
}

# ISO 8601 text of a DateTime object, naive when its timezone is floating.
sub _datetime_iso ($dt) {
    my $iso = $dt->ymd . 'T' . $dt->hms;
    $iso .= sprintf '.%06d', int($dt->nanosecond / 1000) if $dt->nanosecond >= 1000;
    return $iso if $dt->time_zone->is_floating;
    my $offset = $dt->offset;
    return $iso . Perldantic::Temporal::_offset_iso($offset);
}

sub _time_moment_iso ($tm) {
    my $iso = $tm->strftime('%Y-%m-%dT%H:%M:%S');
    $iso .= sprintf '.%06d', $tm->microsecond if $tm->microsecond;
    return $iso . Perldantic::Temporal::_offset_iso($tm->offset * 60);
}

sub _duration_parts ($d) {
    my ($months, $days, $minutes, $seconds, $nanoseconds) = $d->in_units(qw(months days minutes seconds nanoseconds));
    _cannot('a DateTime::Duration with months or years', 'months have no fixed length') if $months;
    return Perldantic::Duration->new(
        days         => $days,
        minutes      => $minutes,
        seconds      => $seconds,
        microseconds => $nanoseconds / 1000,
    );
}

# The core's text of each decimal decoded from it, with the value it had: Math::BigFloat drops
# trailing zeros (Decimal('2.0') becomes 2), so while an object keeps its value, the text it
# came with goes back to the core and the decimal keeps its exponent, as in pydantic.
Hash::Util::FieldHash::fieldhash(my %DECIMAL_TEXT);

# A Math::BigFloat as Python's Decimal reads it.
sub _decimal_text ($value) {
    my $known = $DECIMAL_TEXT{$value};
    return $known->[0] if $known && $known->[1] eq $value->bsstr;
    return 'NaN' if $value->is_nan;
    return $value->is_negative ? '-Infinity' : 'Infinity' if $value->is_inf;
    return $value->bsstr;
}

sub _decimal ($text) {
    # copy the sign: `$1` is passed by alias and binf matches regexes itself
    my ($infinity) = $text =~ /\A([-+]?)Infinity\z/;
    my $value = $text =~ /nan/i ? Math::BigFloat->bnan
        : defined $infinity ? Math::BigFloat->binf($infinity eq '-' ? '-' : '+')
        : Math::BigFloat->new($text);
    $DECIMAL_TEXT{$value} = [$text, $value->bsstr];
    return $value;
}

# Decoding the core's JSON: tagged objects are turned into Perl values by the parser itself
# (single-key object filters), so the result needs no second walk. Integers beyond 64 bits
# come tagged ($bigint), which spares the parser allow_bignum and its slow floats.
my %UNTAG = (
    tuple     => sub ($items) { $items },
    set       => sub ($items) { $items },
    frozenset => sub ($items) { $items },
    bytes     => sub ($text)  { decode_base64($text) },
    float     => sub ($name)  { $name eq 'nan' ? 9**9**9 / 9**9**9 : $name eq 'inf' ? 9**9**9 : -9**9**9 },
    bigint    => sub ($text)  { Math::BigInt->new($text) },
    dict      => sub ($pairs) { +{map { ((ref $_->[0] ? _key_text($_->[0]) : $_->[0] // '') => $_->[1]) } @$pairs} },
    # the decoded object already has the fields of a Wire::Model
    model     => sub ($model) {
        $model->{fields_set} //= [sort keys %{$model->{fields} //= {}}];
        bless $model, 'Perldantic::Wire::Model';
    },
    enum      => sub ($member) { Perldantic::Wire::Enum->new(%$member) },
    host => sub ($host) {
        $OBJECT{$host->{id} // ''}
            // Perldantic::InternalError->throw(message => "The core returned an unknown object of class $host->{class}");
    },
    function => sub ($function) {
        $FUNCTION{$function->{id} // ''}
            // Perldantic::InternalError->throw(message => "The core returned an unknown function `$function->{name}`");
    },
    date      => sub ($iso)   { Perldantic::Date->from_iso($iso) },
    time      => sub ($iso)   { Perldantic::Time->from_iso($iso) },
    datetime  => sub ($iso)   { Perldantic::DateTime->from_iso($iso) },
    uuid      => sub ($text)  { Perldantic::Uuid->new($text) },
    decimal   => \&_decimal,
    url            => sub ($text) { Perldantic::Url->_from_wire($text) },
    multi_host_url => sub ($text) { Perldantic::MultiHostUrl->_from_wire($text) },
    timedelta => sub ($parts) {
        my ($days, $seconds, $microseconds) = @$parts;
        Perldantic::Duration->new(days => $days, seconds => $seconds, microseconds => $microseconds);
    },
);

$DECODER = Cpanel::JSON::XS->new->utf8->allow_nonref->unblessed_bool;
# The filters return one value: returning an empty list would keep the object as it is.
$DECODER->filter_json_single_key_object('$' . $_ => $UNTAG{$_}) for keys %UNTAG;

# A dict key that is no string or number: its wire JSON, with sorted keys.
sub _key_text ($key) {
    return $JSON->encode($JSON->decode(encode($key)));
}

package Perldantic::Wire::Model {

    sub new ($class, %args) {
        my $fields = $args{fields} // {};
        return bless {
            class      => $args{class},
            fields     => $fields,
            fields_set => $args{fields_set} // [sort keys %$fields],
            extra      => $args{extra},
        }, $class;
    }

    sub class ($self)      { $self->{class} }
    sub fields ($self)     { $self->{fields} }
    sub fields_set ($self) { $self->{fields_set} }
    sub extra ($self)      { $self->{extra} }

    sub _json ($self) {
        return '{"class":' . Perldantic::Wire::_string($self->{class})
            . ',"extra":' . Perldantic::Wire::_emit_any($self->{extra})
            . ',"fields":' . Perldantic::Wire::_emit_any($self->{fields})
            . ',"fields_set":' . Perldantic::Wire::_emit_any($self->{fields_set}) . '}';
    }
}

package Perldantic::Wire::Enum {

    sub new ($class, %args) {
        return bless {
            class        => $args{class},
            name         => $args{name},
            value        => $args{value},
            mixin        => $args{mixin},
            str_is_value => !!$args{str_is_value},
        }, $class;
    }

    sub class ($self)        { $self->{class} }
    sub name ($self)         { $self->{name} }
    sub value ($self)        { $self->{value} }
    sub mixin ($self)        { $self->{mixin} }
    sub str_is_value ($self) { $self->{str_is_value} }

    sub _json ($self) {
        return '{"class":' . Perldantic::Wire::_string($self->{class})
            . (defined $self->{mixin} ? ',"mixin":' . Perldantic::Wire::_string($self->{mixin}) : '')
            . ',"name":' . Perldantic::Wire::_string($self->{name})
            . ($self->{str_is_value} ? ',"str_is_value":true' : '')
            . ',"value":' . Perldantic::Wire::_emit_any($self->{value}) . '}';
    }
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Wire - the JSON wire format between Perl and the Rust core

=head1 SYNOPSIS

    use Perldantic::Wire qw(tuple set bytes);

    my $json = Perldantic::Wire::encode({point => tuple(1, 2), raw => bytes("\xff")});
    my $data = Perldantic::Wire::decode($json);

=head1 DESCRIPTION

Values cross the FFI boundary as UTF-8 JSON (see F<ffi/src/wire.rs>). Plain Perl data maps onto
JSON: C<undef> is C<null>, native booleans (and C<JSON::PP::Boolean>) are booleans, numbers stay
numbers (C<Math::BigInt> included), strings are text, array references are lists and hash
references are dicts with sorted keys.

Encoding runs in C (F<Perldantic.xs>) for plain data, which is most of it, and in Perl for
objects; both write the same JSON (a number is a value that was never used as a string, as
C<builtin::created_as_number> says). Decoding is done by L<Cpanel::JSON::XS> in one pass.

What JSON cannot express is tagged:

=over

=item * C<tuple(@items)>, C<set(@items)> and C<frozenset(@items)> mark an array as a tuple, a set
or a frozenset;

=item * C<bytes($octets)> marks a byte string (Perl strings are text otherwise);

=item * infinite and NaN floats are C<{"$float": ...}>;

=item * a hash with a key starting with C<$> is sent as a list of pairs, and so is
C<ordered(key =E<gt> value, ...)>, which keeps the given key order;

=item * C<Perldantic::Wire::Model> is a model instance: C<class>, C<fields>, C<fields_set>
(defaults to the field names) and C<extra>.

=item * C<Perldantic::Wire::Enum> is an enum member: C<class>, C<name>, C<value> and, for
members of enums that mix in a builtin type (Python's C<IntEnum>, C<StrEnum>), C<mixin>
(C<int>, C<str>, C<float> or C<bytes>) and C<str_is_value>.

=item * dates, times, datetimes and durations are L<Perldantic::Temporal> values (decoded as
such too); L<DateTime> and L<Time::Moment> objects are sent as datetimes (a floating DateTime as
a naive one) and L<DateTime::Duration> objects without months as durations;

=item * L<Math::BigFloat> objects are decimals, C<{"$decimal": "..."}> (decoded as such too;
C<Math::BigInt> objects are integers). Math::BigFloat drops trailing zeros, so a decoded object
remembers the core's text (C<2.0>) and sends it back while its value is unchanged;

=item * UUIDs are L<Perldantic::Uuid> values, C<{"$uuid": "..."}> (decoded as such too);

=item * URLs are L<Perldantic::Url> and L<Perldantic::MultiHostUrl> values, C<{"$url": "..."}>
and C<{"$multi_host_url": "..."}> (decoded as such too); L<URI> objects are sent as their text;

=item * any other object is a host object the core only checks the class of and hands back,
C<{"$host": {"id", "class", "isa", "repr"}}>, and decodes back to the same object
(C<Perldantic::Wire::object($id)>);

=item * a code reference is a function the core calls back (see L<Perldantic::FFI>),
C<{"$function": {"id": ..., "name": ...}}>; the id is the code reference's while it lives, and
C<Perldantic::Wire::function($id)> gives it back;

=item * an object with a C<_perldantic_wire> method is sent as what that method returns.

=back

Decoding turns tuples and sets into array references and bytes into byte strings, and returns
native booleans and C<Math::BigInt> for integers beyond 64 bits (which the core tags,
C<{"$bigint": "..."}>). A dict key becomes what its value decodes to when that is a string or a
number (bytes keys are their byte strings), its wire JSON otherwise (a tuple C<(1, 2)> is keyed
C<[1,2]>, with sorted keys in objects), and a C<null> key the empty string. Decoding is done by
the JSON parser itself (tagged objects are single-key object filters), in one pass.

A value with no wire form (a reference to a scalar or a glob) raises
C<Perldantic::UsageError>; malformed JSON from the core raises C<Perldantic::InternalError>.

=cut
