package Perldantic::Test::CaseJSON;

# Reads conformance case files (tests/conformance/README.md) into Perl data that keeps what the
# cases say: object key order (as Perldantic::Wire::ordered), and the `$`-tagged values as the
# Perldantic::Wire markers. Values Perl cannot represent die with a Skip.

use v5.36;

use Exporter 'import';
use MIME::Base64 qw(decode_base64);
use Math::BigInt;

use Perldantic::Url;
use Perldantic::Uuid;
use Perldantic::Wire qw(tuple set frozenset bytes ordered);

our @EXPORT_OK = qw(parse_json interpret node_get node_has);

package Perldantic::Test::CaseJSON::Skip {
    sub new ($class, $reason) { bless {reason => $reason}, $class }
    sub reason ($self) { $self->{reason} }
}

sub _skip ($reason) { die Perldantic::Test::CaseJSON::Skip->new($reason) }

# Parse JSON text (characters) into raw nodes: objects become ordered() pairs, `$` tags included.
sub parse_json ($text) {
    pos($text) = 0;
    my $value = _value(\$text);
    $text =~ /\G\s*/gc;
    die 'trailing text at ' . pos($text) if pos($text) != length $text;
    return $value;
}

# Turn a raw node into the value it encodes. In a schema, `{"$class": name}` is the class name.
sub interpret ($node, %options) {
    my $in_schema = !!$options{in_schema};
    my $walk = sub ($n) { interpret($n, in_schema => $in_schema) };
    return [map { $walk->($_) } @$node] if ref $node eq 'ARRAY';
    return $node if ref $node ne 'Perldantic::Wire::Ordered';
    my @pairs = @$node;
    return _tagged($pairs[0], $pairs[1], $in_schema) if @pairs == 2 && $pairs[0] =~ /\A\$/;
    return ordered(map { $_ % 2 ? $walk->($pairs[$_]) : $pairs[$_] } 0 .. $#pairs);
}

my %ESCAPE = ('"' => '"', '\\' => '\\', '/' => '/', b => "\b", f => "\f", n => "\n", r => "\r", t => "\t");

sub _string ($ref) {
    $$ref =~ /\G"((?:[^"\\]|\\.)*)"/gcs or die 'bad string at ' . pos($$ref);
    my $s = $1;
    $s =~ s{\\(?:u([0-9a-fA-F]{4})|(.))}{defined $1 ? chr hex $1 : $ESCAPE{$2} // die "bad escape \\$2"}gse;
    # Join UTF-16 surrogate pairs.
    $s =~ s{([\x{D800}-\x{DBFF}])([\x{DC00}-\x{DFFF}])}{chr(0x10000 + ((ord($1) - 0xD800) << 10) + (ord($2) - 0xDC00))}ge;
    utf8::upgrade($s);
    return $s;
}

sub _value ($ref) {
    $$ref =~ /\G\s*/gc;
    if ($$ref =~ /\G\{/gc) {
        my @pairs;
        until ($$ref =~ /\G\s*\}/gc) {
            $$ref =~ /\G\s*,/gc if @pairs;
            $$ref =~ /\G\s*/gc;
            my $key = _string($ref);
            $$ref =~ /\G\s*:/gc or die 'expected : at ' . pos($$ref);
            push @pairs, $key, _value($ref);
        }
        return ordered(@pairs);
    }
    if ($$ref =~ /\G\[/gc) {
        my @items;
        until ($$ref =~ /\G\s*\]/gc) {
            $$ref =~ /\G\s*,/gc if @items;
            push @items, _value($ref);
        }
        return \@items;
    }
    # Not a zero-length /\G(?=")/gc: Perl refuses a second empty match at the same position.
    return _string($ref) if substr($$ref, pos($$ref), 1) eq '"';
    return !!1 if $$ref =~ /\Gtrue/gc;
    return !!0 if $$ref =~ /\Gfalse/gc;
    return undef if $$ref =~ /\Gnull/gc;
    if ($$ref =~ /\G(-?(?:0|[1-9][0-9]*))((?:\.[0-9]+)?(?:[eE][-+]?[0-9]+)?)/gc) {
        return 0 + "$1$2" if length $2;
        return length($1) > 15 ? Math::BigInt->new($1) : 0 + $1;
    }
    die 'bad JSON at ' . pos($$ref);
}

# The value of key $key in an ordered node, or undef.
sub node_get ($node, $key) {
    my @pairs = @$node;
    for (my $i = 0; $i < @pairs; $i += 2) { return $pairs[$i + 1] if $pairs[$i] eq $key }
    return undef;
}

sub node_has ($node, $key) {
    my @pairs = @$node;
    for (my $i = 0; $i < @pairs; $i += 2) { return 1 if $pairs[$i] eq $key }
    return 0;
}

sub _tagged ($tag, $payload, $in_schema) {
    my $walk = sub ($n) { interpret($n, in_schema => $in_schema) };
    return tuple(@{$walk->($payload)}) if $tag eq '$tuple';
    return set(@{$walk->($payload)})   if $tag eq '$set';
    return frozenset(@{$walk->($payload)}) if $tag eq '$frozenset';
    return bytes(decode_base64($payload)) if $tag eq '$bytes';
    if ($tag eq '$float') {
        return 9**9**9 if $payload eq 'inf';
        return -9**9**9 if $payload eq '-inf';
        return 9**9**9 / 9**9**9;
    }
    return ordered(map { @{$walk->($_)} } @$payload) if $tag eq '$dict';
    return Perldantic::Date->from_iso($payload)     if $tag eq '$date';
    return Perldantic::Time->from_iso($payload)     if $tag eq '$time';
    return Perldantic::DateTime->from_iso($payload) if $tag eq '$datetime';
    return Perldantic::Uuid->new($payload)          if $tag eq '$uuid';
    return Perldantic::Wire::decode(qq({"\$decimal":"$payload"})) if $tag eq '$decimal';
    return Perldantic::Url->_from_wire($payload)    if $tag eq '$url';
    return Perldantic::MultiHostUrl->_from_wire($payload) if $tag eq '$multi_host_url';
    if ($tag eq '$timedelta') {
        my ($days, $seconds, $microseconds) = @$payload;
        return Perldantic::Duration->new(days => $days, seconds => $seconds, microseconds => $microseconds);
    }
    return $payload if $tag eq '$class' && $in_schema;
    if ($tag eq '$enum') {
        my ($class, $name, $value, $mixin, $str_is_value) = @$payload;
        return Perldantic::Wire::Enum->new(
            class => $class, name => $name, value => $walk->($value), mixin => $mixin, str_is_value => $str_is_value);
    }
    if ($tag eq '$model') {
        my $extra = node_get($payload, 'extra');
        return Perldantic::Wire::Model->new(
            class      => node_get($payload, 'class'),
            fields     => $walk->(node_get($payload, 'fields')),
            fields_set => $walk->(node_get($payload, 'fields_set')),
            extra      => defined $extra ? $walk->($extra) : undef,
        );
    }
    _skip("value $tag");
}

1;
