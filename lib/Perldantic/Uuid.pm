package Perldantic::Uuid;

use v5.36;

our $VERSION = '0.01';

use Scalar::Util ();

use Perldantic::Error;

# A UUID, like Python's `uuid.UUID`: it stringifies to the hyphenated form and compares by value.
use overload
    '""'     => sub ($self, @) { $self->as_string },
    'cmp'    => sub ($a, $b, $swap) { my $c = $a->_compare($b); $swap ? -$c : $c },
    'eq'     => sub ($a, $b, @) { $a->_equal($b) },
    'ne'     => sub ($a, $b, @) { !$a->_equal($b) },
    bool     => sub { 1 },
    fallback => 1;

sub _usage ($message) { Perldantic::UsageError->throw(message => "Perldantic::Uuid->new: $message") }

# `new($text)`, `new(hex => $hex)` or `new(bytes => $sixteen_bytes)`. Text is read as Python's
# `UUID(hex)` reads it: hyphens, braces and a `urn:uuid:` prefix are optional.
sub new ($class, @args) {
    my $hex;
    if (@args == 1) {
        $hex = _parse_text($args[0]);
    }
    elsif (@args == 2 && $args[0] eq 'hex') {
        $hex = _parse_text($args[1]);
    }
    elsif (@args == 2 && $args[0] eq 'bytes') {
        my $bytes = $args[1] // '';
        _usage('bytes must be 16 bytes long, got ' . length $bytes) if length $bytes != 16;
        $hex = unpack 'H32', $bytes;
    }
    else {
        _usage('takes a UUID string, hex => $hex or bytes => $bytes');
    }
    return bless \$hex, $class;
}

sub _parse_text ($text) {
    _usage('invalid UUID undef') if !defined $text;
    my $hex = $text =~ s/\Aurn:uuid://ir;
    $hex =~ s/\A\{(.*)\}\z/$1/s;
    if ($hex =~ /\A([0-9a-f]{8})-([0-9a-f]{4})-([0-9a-f]{4})-([0-9a-f]{4})-([0-9a-f]{12})\z/i) {
        $hex = "$1$2$3$4$5";
    }
    _usage("invalid UUID '$text'") if $hex !~ /\A[0-9a-f]{32}\z/i;
    return lc $hex;
}

sub hex ($self)       { $$self }
sub bytes ($self)     { pack 'H32', $$self }
sub urn ($self)       { 'urn:uuid:' . $self->as_string }
sub as_string ($self) { join '-', unpack 'A8 A4 A4 A4 A12', $$self }

# The 128-bit number, as a Math::BigInt.
sub int ($self) {
    require Math::BigInt;
    return Math::BigInt->from_hex($$self);
}

# Python's `UUID.variant`, from the top bits of the clock sequence.
sub variant ($self) {
    my $octet = CORE::hex substr $$self, 16, 2;
    return 'reserved for NCS compatibility'       if !($octet & 0x80);
    return 'specified in RFC 4122'                if !($octet & 0x40);
    return 'reserved for Microsoft compatibility' if !($octet & 0x20);
    return 'reserved for future definition';
}

# Python's `UUID.version`: undef unless the UUID is an RFC 4122 one.
sub version ($self) {
    return undef if $self->variant ne 'specified in RFC 4122';
    return CORE::hex substr $$self, 12, 1;
}

# The other operand as a UUID; a string is parsed.
sub _other ($other) {
    return $other if Scalar::Util::blessed($other) && $other->isa(__PACKAGE__);
    return eval { __PACKAGE__->new($other) } if defined $other && !ref $other;
    return undef;
}

sub _equal ($self, $other) {
    $other = _other($other) // return 0;
    return $$self eq $$other;
}

sub _compare ($self, $other) {
    my $uuid = _other($other)
        // Perldantic::UsageError->throw(message => "can't compare Perldantic::Uuid with " . (ref $other || "'$other'"));
    return $$self cmp $$uuid;
}

sub _wire_payload ($self) { $self->as_string }

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Uuid - a UUID, the value the Uuid type validates into

=head1 SYNOPSIS

    use Perldantic::Uuid;

    my $id = Perldantic::Uuid->new('0e7ac198-9acd-4c0c-b4b4-761974bf71d7');
    say "$id";              # 0e7ac198-9acd-4c0c-b4b4-761974bf71d7
    say $id->version;       # 4
    say $id->hex;           # 0e7ac1989acd4c0cb4b4761974bf71d7
    say $id->urn;           # urn:uuid:0e7ac198-9acd-4c0c-b4b4-761974bf71d7
    my $raw = $id->bytes;   # 16 bytes

    say 'same' if $id eq uc "$id";                     # compared by value
    my $copy = Perldantic::Uuid->new(bytes => $raw);

=head1 DESCRIPTION

A UUID modelled on Python's C<uuid.UUID>, so it keeps what pydantic keeps. The
L<Uuid|Perldantic::Types/Uuid> type validates UUID text into these objects and serializes them
back to their hyphenated form.

The value stringifies to its hyphenated, lower-case form, and compares with C<eq>, C<ne> and
C<cmp> by value, against other UUIDs or against UUID text (which is parsed first). Sorting orders
UUIDs by their 128-bit number, as Python does.

=head1 METHODS

=head2 new($text), new(hex => $hex), new(bytes => $bytes)

Makes a UUID from its text, as Python's C<UUID(hex)> reads it: hyphens, surrounding braces and a
C<urn:uuid:> prefix are optional, and the case does not matter. C<< bytes => $bytes >> takes the
16 bytes in network order. Anything else raises L<Perldantic::UsageError>.

=head2 as_string

The hyphenated form, C<12345678-1234-5678-1234-567812345678>.

=head2 hex

The 32 hex digits, without hyphens.

=head2 bytes

The 16 bytes, in network order.

=head2 urn

The URN, C<urn:uuid:> followed by the hyphenated form.

=head2 int

The 128-bit number, as a L<Math::BigInt> (loaded on demand).

=head2 variant

Python's name for the variant: C<specified in RFC 4122>, C<reserved for NCS compatibility>,
C<reserved for Microsoft compatibility> or C<reserved for future definition>.

=head2 version

The version number (1 to 8), or C<undef> unless the variant is RFC 4122.

=head1 SEE ALSO

L<Perldantic::Types/Uuid>, L<Perldantic>

=cut
