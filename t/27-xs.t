use v5.36;
use Test2::V0;

use FFI::Platypus::Buffer ();
use Perldantic::FFI;
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

subtest 'the binary wire format' => sub {
    my $u32 = sub ($n) { pack 'V', $n };
    is Perldantic::Wire::encode_binary([undef, !!1, !!0, 7, 1.5, "caf\x{e9}", {b => 1, '$a' => 2}]),
        join('', "\x06", $u32->(7), "\x00", "\x01", "\x02", "\x03", pack('q<', 7), "\x04", pack('d<', 1.5),
            "\x05", $u32->(5), "caf\xc3\xa9",
            "\x07", $u32->(2), $u32->(2), '$a', "\x03", pack('q<', 2), $u32->(1), 'b', "\x03", pack('q<', 1)),
        'plain data natively; hash keys sorted, $ keys need nothing special';
    is Perldantic::Wire::encode_binary(tuple(1)), "\x08" . $u32->(14) . '{"$tuple":[1]}',
        'anything else as a node of wire JSON';
    is Perldantic::Wire::encode_binary(18446744073709551615), "\x08" . $u32->(20) . '18446744073709551615',
        'integers beyond 64 bits as JSON digits';

    my $decode = sub ($bytes) {
        my ($address, $len) = FFI::Platypus::Buffer::scalar_to_buffer($bytes);
        return [Perldantic::XS::decode_result($address, $len, \&Perldantic::Wire::decode)];
    };
    is $decode->('B' . "\x06" . $u32->(2) . "\x05" . $u32->(2) . "\xc3\xa9" . "\x08" . $u32->(13) . '{"$bytes":""}'),
        ['ok', ["\x{e9}", ''], undef], 'values, with JSON nodes decoded by Perldantic::Wire';
    is $decode->('W' . $u32->(4) . 'warn' . "\x00"), ['ok', undef, 'warn'], 'with a warning';
    is $decode->('J{"error":1}'), ['envelope', '{"error":1}'], 'error envelopes stay JSON';
    my $model = $decode->('B' . "\x0a" . $u32->(1) . 'M' . "\x07" . $u32->(0) . "\x06" . $u32->(0) . "\x00")->[1];
    isa_ok $model, 'Perldantic::Wire::Model';
    is {%$model}, {class => 'M', fields => {}, fields_set => [], extra => undef};
    like dies { $decode->('B' . "\x05" . $u32->(9) . 'a') }, qr/truncated/;
    like dies { $decode->('B' . "\x00\x00") }, qr/trailing/;
};

subtest 'binary and JSON transports agree' => sub {
    require Math::BigFloat;
    require Perldantic::Temporal;
    require Perldantic::Uuid;
    my $any = Perldantic::FFI::Validator->new({type => 'any'});
    my $serializer = Perldantic::FFI::Serializer->new({type => 'any'});
    my @values = (
        undef, !!1, 0, -9223372036854775808, 18446744073709551615, 0.1 + 0.2, 9**9**9, 'text', "\x{1F600}",
        [1, [2, {x => [undef]}]], {'$key' => 'dollar', nested => {deeper => 1}}, tuple(1, 'a'), set(3, 4),
        bytes("\0ab"), Math::BigFloat->new('1.50'), Perldantic::Date->new(year => 2024, month => 2, day => 29),
        Perldantic::Uuid->new('12345678-1234-5678-1234-567812345678'), Test::Point->new(x => 1, y => 'p'),
        [map { Test::Point->new(x => $_, y => 'q') } 1 .. 3],
    );
    for my $value (@values) {
        my $label = defined $value ? "$value" : 'undef';
        # the native decoder builds plain model objects itself; _inflate builds the rest
        my $validate = sub { Perldantic::Model::_inflate($any->validate($value)) };
        my @binary = ($validate->(), $serializer->to_perl($value), $serializer->to_json($value));
        local $Perldantic::Wire::XS = 0;
        my @json = ($validate->(), $serializer->to_perl($value), $serializer->to_json($value));
        is \@binary, \@json, "same results for $label";
    }
};

package Test::Built {
    use Perldantic;
    has n => (is => 'ro', isa => Int);
    sub BUILD ($self, $args) { $self->{built} = 1 }
}
package Test::Triggered {
    use Perldantic;
    has n => (is => 'ro', isa => Int, trigger => sub ($self, $value) { $self->{seen} = $value });
}
package Test::Loose {
    use Perldantic;
    has n => (is => 'ro', isa => Int);
    model_config extra => 'allow';
}

subtest 'plain model objects are built natively' => sub {
    my $calls = 0;
    no warnings 'redefine';
    my $original = \&Perldantic::Model::_inflate;
    local *Perldantic::Model::_inflate = sub { $calls++ if ref $_[0] eq 'Perldantic::Wire::Model'; $original->(@_) };
    my $adapter = Perldantic::TypeAdapter->new(Perldantic::Types::ArrayRef(['Test::Point']));
    $adapter->validate([{x => 0, y => 'warm'}]);    # the first use works the class out in Perl
    $calls = 0;
    my $points = $adapter->validate([{x => 1, y => 'a'}, {x => '2', y => 'b'}]);
    is $calls, 0, 'no model went through Perl';
    isa_ok $points->[1], 'Test::Point';
    is $points->[1]->x, 2;
    is [$points->[1]->model_fields_set], [qw(x y)];
    is $points->[1]->model_extra, undef;
    is $adapter->dump($points), [{x => 1, y => 'a'}, {x => 2, y => 'b'}];

    my $some = Test::Point->new(y => 'only');
    is $calls, 1, 'a model with fields left out goes through Perl';
    is [$some->model_fields_set], ['y'];
    ok !exists $some->{x};
    $calls = 0;
    is Test::Built->new(n => 1)->{built}, 1, 'BUILD runs';
    is Test::Triggered->new(n => 5)->{seen}, 5, 'triggers run';
    my $loose = Test::Loose->new(n => 1, other => 2);
    is $loose->model_extra, {other => 2}, 'extra values are kept';
    is $calls, 3, 'those went through Perl';

    my $kept = Test::Point->new(x => 1, y => 'k');
    my $holder = Perldantic::TypeAdapter->new(Perldantic::Types::ArrayRef(['Test::Point']))->validate([$kept]);
    ok $holder->[0] == $kept, 'model objects given as input are kept, as in pydantic';
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
