package Perldantic::Wire;

use v5.36;
no warnings 'experimental::builtin';

our $VERSION = '0.01';

use B ();
use Cpanel::JSON::XS ();
use Exporter 'import';
use Hash::Util::FieldHash ();
use Math::BigFloat ();
use MIME::Base64 qw(encode_base64 decode_base64);
use Scalar::Util qw(blessed reftype);

use Perldantic::Error;
use Perldantic::Temporal;
use Perldantic::Url;
use Perldantic::Uuid;

our @EXPORT_OK = qw(tuple set frozenset bytes ordered);

my $JSON = Cpanel::JSON::XS->new->utf8->canonical->allow_nonref->allow_bignum->unblessed_bool;

sub tuple (@items) { bless [@items], 'Perldantic::Wire::Tuple' }
sub set (@items)   { bless [@items], 'Perldantic::Wire::Set' }
sub frozenset (@items) { bless [@items], 'Perldantic::Wire::FrozenSet' }
sub bytes ($octets) { bless \(my $copy = $octets), 'Perldantic::Wire::Bytes' }

sub ordered (@pairs) {
    Perldantic::UsageError->throw(message => 'ordered() takes key => value pairs') if @pairs % 2;
    return bless [@pairs], 'Perldantic::Wire::Ordered';
}

sub encode ($value) {
    my $tagged = _tag($value);
    my $json   = eval { _write($tagged) };
    Perldantic::InternalError->throw(message => "Cannot encode a value for the core: $@", cause => $@)
        if !defined $json;
    return $json;
}

# Strings only: JSON escaping and UTF-8 encoding.
my $STRING = Cpanel::JSON::XS->new->utf8->allow_nonref;

# The shortest decimal form that reads back as the same double, always with a fraction or an
# exponent so that the core sees a float. (JSON modules print 15 significant digits, which
# changes values such as 0.1 + 0.2.)
sub _float ($value) {
    my $text;
    for my $digits (1 .. 17) {
        $text = sprintf "%.${digits}g", $value;
        last if $text == $value;
    }
    return $text =~ /[.eE]/ ? $text : "$text.0";
}

# Write tagged data (see _tag) as JSON: hashes with sorted keys, scalars by their Perl kind.
sub _write ($data) {
    return 'null' if !defined $data;
    if (my $class = blessed $data) {
        return $data ? 'true' : 'false'
            if $data->isa('JSON::PP::Boolean') || $data->isa('Types::Serialiser::Boolean');
        return $data->bstr if $data->isa('Math::BigInt');
        die "unexpected $class object\n";
    }
    my $type = ref $data;
    return '[' . join(',', map { _write($_) } @$data) . ']' if $type eq 'ARRAY';
    return '{' . join(',', map { $STRING->encode("$_") . ':' . _write($data->{$_}) } sort keys %$data) . '}'
        if $type eq 'HASH';
    return $data ? 'true' : 'false' if builtin::is_bool($data);
    my $flags = B::svref_2object(\$data)->FLAGS;
    return $STRING->encode("$data") if $flags & B::SVf_POK || !($flags & (B::SVf_IOK | B::SVf_NOK));
    return "$data" if $flags & B::SVf_IOK;
    return _float($data);
}

sub decode ($json) {
    my $data = eval { $JSON->decode($json) };
    Perldantic::InternalError->throw(message => "Malformed JSON from the core: $@", cause => $@)
        if !defined $data && $@;
    return _untag($data);
}

sub _cannot ($value, $why) {
    Perldantic::UsageError->throw(message => "Cannot pass $value to the core: $why");
}

# A float that is not finite; only numbers that were never strings count.
sub _special_float ($value) {
    my $flags = B::svref_2object(\$value)->FLAGS;
    return undef if !($flags & B::SVf_NOK) || ($flags & B::SVf_POK);
    return 'nan' if $value != $value;
    return $value > 0 ? 'inf' : '-inf' if $value * 0 != 0;
    return undef;
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

sub _tag ($value) {
    if (!ref $value) {
        return $value if !defined $value;
        my $special = _special_float($value);
        return defined $special ? {'$float' => $special} : $value;
    }
    if (my $class = blessed $value) {
        return {'$tuple' => [map { _tag($_) } @$value]} if $class eq 'Perldantic::Wire::Tuple';
        return {'$set' => [map { _tag($_) } @$value]}   if $class eq 'Perldantic::Wire::Set';
        return {'$frozenset' => [map { _tag($_) } @$value]} if $class eq 'Perldantic::Wire::FrozenSet';
        return {'$bytes' => encode_base64($$value, '')} if $class eq 'Perldantic::Wire::Bytes';
        return {'$model' => $value->_wire}              if $class eq 'Perldantic::Wire::Model';
        return {'$enum' => $value->_wire}               if $class eq 'Perldantic::Wire::Enum';
        return {$value->_wire_tag => $value->_wire_payload} if $value->isa('Perldantic::Temporal');
        return {'$uuid' => $value->as_string}           if $value->isa('Perldantic::Uuid');
        return {'$multi_host_url' => $value->as_string} if $value->isa('Perldantic::MultiHostUrl');
        return {'$url' => $value->as_string}            if $value->isa('Perldantic::Url');
        return $value->as_string                        if $value->isa('URI');
        return {'$datetime' => _datetime_iso($value)} if $value->isa('DateTime');
        return {'$datetime' => _time_moment_iso($value)} if $value->isa('Time::Moment');
        return _tag(_duration_parts($value)) if $value->isa('DateTime::Duration');
        if ($class eq 'Perldantic::Wire::Ordered') {
            my @pairs = @$value;
            return {'$dict' => [map { [_tag($pairs[2 * $_]), _tag($pairs[2 * $_ + 1])] } 0 .. @pairs / 2 - 1]};
        }
        return _tag($value->_perldantic_wire) if $value->can('_perldantic_wire');
        return $value if $value->isa('JSON::PP::Boolean') || $value->isa('Types::Serialiser::Boolean');
        return {'$decimal' => _decimal_text($value)} if $value->isa('Math::BigFloat');
        return $value if $value->isa('Math::BigInt');
        _cannot("a $class object", "$class has no wire form");
    }
    my $type = reftype $value;
    return [map { _tag($_) } @$value] if $type eq 'ARRAY';
    if ($type eq 'HASH') {
        my @keys = sort keys %$value;
        return {map { $_ => _tag($value->{$_}) } @keys} if !grep {/^\$/} @keys;
        return {'$dict' => [map { [$_, _tag($value->{$_})] } @keys]};
    }
    _cannot("a $type reference", "$type has no wire form");
}

my %UNTAG = (
    tuple => sub ($items) { [map { _untag($_) } @$items] },
    set   => sub ($items) { [map { _untag($_) } @$items] },
    frozenset => sub ($items) { [map { _untag($_) } @$items] },
    bytes => sub ($text)  { decode_base64($text) },
    float => sub ($name)  { $name eq 'nan' ? 9**9**9 / 9**9**9 : $name eq 'inf' ? 9**9**9 : -9**9**9 },
    dict  => sub ($pairs) { +{map { ((ref $_->[0] ? $JSON->encode($_->[0]) : $_->[0] // '') => _untag($_->[1])) } @$pairs} },
    model => sub ($model) { Perldantic::Wire::Model->new(%{_untag($model)}) },
    enum  => sub ($member) { Perldantic::Wire::Enum->new(%{_untag($member)}) },
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

sub _untag ($value) {
    # allow_bignum keeps big integers exact, but also turns floats it cannot hold exactly into
    # Math::BigFloat; the core's floats are f64, so they come back as plain numbers.
    return $value->numify + 0 if blessed $value && $value->isa('Math::BigFloat');
    my $type = reftype $value // '';
    return [map { _untag($_) } @$value] if $type eq 'ARRAY' && !blessed $value;
    return $value if $type ne 'HASH' || blessed $value;
    if (keys %$value == 1) {
        my ($key) = keys %$value;
        if ($key =~ /^\$(.*)/s) {
            my $untag = $UNTAG{$1}
                // Perldantic::InternalError->throw(message => "Unknown wire tag `$key` from the core");
            return $untag->($value->{$key});
        }
    }
    return {map { $_ => _untag($value->{$_}) } keys %$value};
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

    sub _wire ($self) {
        return {
            class      => $self->{class},
            fields     => Perldantic::Wire::_tag($self->{fields}),
            fields_set => Perldantic::Wire::_tag($self->{fields_set}),
            extra      => Perldantic::Wire::_tag($self->{extra}),
        };
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

    sub _wire ($self) {
        return {
            class => $self->{class},
            name  => $self->{name},
            value => Perldantic::Wire::_tag($self->{value}),
            (defined $self->{mixin} ? (mixin => $self->{mixin}) : ()),
            ($self->{str_is_value} ? (str_is_value => !!1) : ()),
        };
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

=item * an object with a C<_perldantic_wire> method is sent as what that method returns.

=back

Decoding turns tuples and sets into array references and bytes into byte strings (a dict key
that is none of string or number is keyed by its wire JSON, and a C<null> key by the empty
string), and returns
native booleans and C<Math::BigInt> for big integers.

A value with no wire form (a code reference, an unknown object) raises
C<Perldantic::UsageError>; malformed JSON from the core raises C<Perldantic::InternalError>.

=cut
