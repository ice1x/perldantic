use v5.36;
use Test2::V0;

use Perldantic::Wire qw(tuple set bytes ordered);

skip_all 'the native encoder is not built' if !$Perldantic::Wire::XS;

# The native encoder writes exactly what the pure-Perl one does.
sub both ($value) {
    my $native = Perldantic::Wire::encode($value);
    local $Perldantic::Wire::XS = 0;
    return ($native, Perldantic::Wire::encode($value));
}

package Test::Thing { sub new ($class) { bless {}, $class } }

my @values = (
    undef, !!1, !!0, 0, -1, 42, 18446744073709551615, -9223372036854775808,
    1.5, 0.1 + 0.2, 1e300, -2.5e-12, 5e-324, 9**9**9, -9**9**9, 9**9**9 / 9**9**9, 3.0,
    '', 'plain', '42', "tab\there", "quote\" back\\slash", "ctl\x01\x1f", "caf\x{e9}", "\x{1F600}",
    "bytes \xe9", [1, [2, [3]]], {b => 1, a => [undef, 'x'], 'caf\x{e9}' => 2},
    {'$dollar' => 1, z => 2}, [tuple(1, 'a'), set(3), bytes("\0\xff")], ordered(b => 1, a => 2),
    {nested => {deeper => [{x => 1.25}]}}, Test::Thing->new, sub {1},
);
for my $value (@values) {
    my ($native, $perl) = both($value);
    is $native, $perl, 'same JSON for ' . (defined $value ? "$value" : 'undef');
}

my $number = '7';
my $as_number = $number + 0;
is [both($number)]->[0], '"7"', 'strings stay strings';
is [both($as_number)]->[0], '7', 'numbers stay numbers';

my $deep = 'bottom';
$deep = [$deep] for 1 .. 600;
my ($native, $perl) = both($deep);
is $native, $perl, 'deep nesting goes past the native depth limit through Perl';

my $e = dies { Perldantic::Wire::encode([\1]) };
isa_ok $e, 'Perldantic::UsageError';

done_testing;
