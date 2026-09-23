package Perldantic::Wire;

use v5.36;
no warnings 'experimental::builtin';

our $VERSION = '0.01';

use B ();
use Cpanel::JSON::XS ();
use Exporter 'import';
use MIME::Base64 qw(encode_base64 decode_base64);
use Scalar::Util qw(blessed reftype);

use Perldantic::Error;

our @EXPORT_OK = qw(tuple set bytes);

my $JSON = Cpanel::JSON::XS->new->utf8->canonical->allow_nonref->allow_bignum->unblessed_bool;

sub tuple (@items) { bless [@items], 'Perldantic::Wire::Tuple' }
sub set (@items)   { bless [@items], 'Perldantic::Wire::Set' }
sub bytes ($octets) { bless \(my $copy = $octets), 'Perldantic::Wire::Bytes' }

sub encode ($value) {
    my $tagged = _tag($value);
    my $json   = eval { $JSON->encode($tagged) };
    Perldantic::InternalError->throw(message => "Cannot encode a value for the core: $@") if !defined $json;
    return $json;
}

sub decode ($json) {
    my $data = eval { $JSON->decode($json) };
    Perldantic::InternalError->throw(message => "Malformed JSON from the core: $@")
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

sub _tag ($value) {
    if (!ref $value) {
        return $value if !defined $value;
        # Native booleans become JSON booleans whatever the JSON module knows about them.
        return $value ? $Cpanel::JSON::XS::true : $Cpanel::JSON::XS::false if builtin::is_bool($value);
        my $special = _special_float($value);
        return defined $special ? {'$float' => $special} : $value;
    }
    if (my $class = blessed $value) {
        return {'$tuple' => [map { _tag($_) } @$value]} if $class eq 'Perldantic::Wire::Tuple';
        return {'$set' => [map { _tag($_) } @$value]}   if $class eq 'Perldantic::Wire::Set';
        return {'$bytes' => encode_base64($$value, '')} if $class eq 'Perldantic::Wire::Bytes';
        return {'$model' => $value->_wire}              if $class eq 'Perldantic::Wire::Model';
        return $value if $value->isa('JSON::PP::Boolean') || $value->isa('Types::Serialiser::Boolean');
        return $value if $value->isa('Math::BigInt') || $value->isa('Math::BigFloat');
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
    bytes => sub ($text)  { decode_base64($text) },
    float => sub ($name)  { $name eq 'nan' ? 9**9**9 / 9**9**9 : $name eq 'inf' ? 9**9**9 : -9**9**9 },
    dict  => sub ($pairs) { +{map { ($_->[0] => _untag($_->[1])) } @$pairs} },
    model => sub ($model) { Perldantic::Wire::Model->new(%{_untag($model)}) },
);

sub _untag ($value) {
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

=item * C<tuple(@items)> and C<set(@items)> mark an array as a tuple or a set;

=item * C<bytes($octets)> marks a byte string (Perl strings are text otherwise);

=item * infinite and NaN floats are C<{"$float": ...}>;

=item * a hash with a key starting with C<$> is sent as a list of pairs;

=item * C<Perldantic::Wire::Model> is a model instance: C<class>, C<fields>, C<fields_set>
(defaults to the field names) and C<extra>.

=back

Decoding turns tuples and sets into array references and bytes into byte strings, and returns
native booleans and C<Math::BigInt> for big integers.

A value with no wire form (a code reference, an unknown object) raises
C<Perldantic::UsageError>; malformed JSON from the core raises C<Perldantic::InternalError>.

=cut
