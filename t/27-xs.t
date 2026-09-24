use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
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

package Test::Point {
    use Perldantic;
    has x => (is => 'rw', isa => Int, clearer => 1);
    has y => (is => 'ro', isa => Str);
}

subtest 'model objects without state are written natively' => sub {
    my $point = Test::Point->new(x => 1, y => "caf\x{e9}");
    Perldantic::Model::_plan('Test::Point');
    is Perldantic::Wire::encode($point), qq({"\$model":{"class":"Test::Point","fields":{"x":1,"y":"caf\xc3\xa9"}}}),
        'fields in declared order; the core takes every held field as set';
    is Test::Point->new(x => 2, y => 'b')->model_dump(exclude_unset => 1), {x => 2, y => 'b'};

    my $some = Test::Point->new(y => 'only');
    like Perldantic::Wire::encode($some), qr/"fields_set":\["y"\]/, 'objects with state go through Perl';
    is $some->model_dump(exclude_unset => 1), {y => 'only'};
    $point->clear_x;
    like Perldantic::Wire::encode($point), qr/"fields_set":\["y"\]/, 'and so does a changed one';

    my $tracked = Test::Point->new(x => 3, y => 't');
    local $Perldantic::Model::TRACK_OBJECTS = 1;
    like Perldantic::Wire::encode($tracked), qr/perldantic object/, 'objects tracked by a call carry their token';
};

subtest 'a changed class is written with its new fields' => sub {
    package Test::Grows { use Perldantic; has a => (is => 'ro', isa => Int) }
    my $old = Test::Grows->new(a => 1);
    Perldantic::Model::_plan('Test::Grows');
    is Perldantic::Wire::encode($old), qq({"\$model":{"class":"Test::Grows","fields":{"a":1}}});
    package Test::Grows { has b => (is => 'ro', isa => Int) }
    my $new = Test::Grows->new(a => 1, b => 2);
    like Perldantic::Wire::encode($new), qr/"fields":\{"a":1,"b":2\}/;
};

subtest 'dumps without serializer functions write objects natively' => sub {
    my $calls = 0;
    no warnings 'redefine';
    my $original = \&Perldantic::Model::_wire_json;
    local *Perldantic::Model::_wire_json = sub { $calls++; $original->(@_) };
    my $points = [map { Test::Point->new(x => $_, y => 'p') } 1 .. 3];
    is Test::Point->new(x => 1, y => 'p')->model_dump_json, '{"x":1,"y":"p"}';
    Perldantic::TypeAdapter->new(Perldantic::Types::ArrayRef(['Test::Point']))->dump($points);
    is $calls, 0, 'no object went through Perl';
};

done_testing;
