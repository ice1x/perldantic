use v5.36;
use Test2::V0;

use Scalar::Util qw(refaddr);

use Perldantic::Call qw(validate_call);
use Perldantic::Types qw(ArrayRef HashRef Int Maybe Num Str slurpy);

# Validating the arguments of subs: pydantic's validate_call, with Type::Params' options.

package Vc::Point {
    use Perldantic;
    has x => (is => 'ro', isa => Int, required => 1);
    has y => (is => 'ro', isa => Int, required => 1);
}

package Vc::Color {
    use Perldantic::Enum RED => 'red', BLUE => 'blue';
}

package Vc::Math {
    use v5.36;
    use Perldantic::Call qw(validate_call);
    use Perldantic::Types qw(Int Num);

    validate_call add => (positional => [Int, Int, {default => 1}], returns => Int);
    sub add ($x, $y) { $x + $y }

    validate_call scale => (named => [by => Num, value => Num, {default => 1}]);
    sub scale (%args) { $args{by} * $args{value} }

    sub new ($class) { bless {factor => 3}, $class }
    validate_call times => (method => 1, positional => [Int]);
    sub times ($self, $n) { $self->{factor} * $n }
}

subtest 'positional arguments' => sub {
    is Vc::Math::add(2, '3'), 5, 'validated and converted';
    is Vc::Math::add(2), 3, 'a default';
    my $e = dies { Vc::Math::add('x', 1) };
    isa_ok $e, ['Perldantic::ValidationError'];
    is $e->title, 'Vc::Math::add';
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['int_parsing', [0]]];
    $e = dies { Vc::Math::add(1, 2, 3) };
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['unexpected_positional_argument', [2]]];
    $e = dies { Vc::Math::add() };
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['missing_positional_only_argument', [0]]];
};

subtest 'the result' => sub {
    my $half = validate_call(sub ($n) { $n / 2 }, positional => [Int], returns => Int);
    is $half->(4), 2;
    my $e = dies { $half->(3) };
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['int_from_fraction', ['return']]];
};

subtest 'named arguments' => sub {
    is Vc::Math::scale(by => 2, value => '1.5'), 3;
    is Vc::Math::scale(by => 2), 2, 'a default';
    is Vc::Math::scale({by => 4}), 4, 'or a hash reference';
    my $e = dies { Vc::Math::scale(by => 'x', size => 1) };
    is [sort map { "$_->{type}:@{$_->{loc}}" } @{$e->errors}],
        ['number_parsing:by', 'unexpected_keyword_argument:size'];
    $e = dies { Vc::Math::scale('by') };
    isa_ok $e, ['Perldantic::UsageError'];
    like $e->message, qr/Vc::Math::scale takes named arguments as name => value pairs/;
};

subtest 'both' => sub {
    my $fill = validate_call(sub ($text, %opt) { $opt{char} x ($opt{width} - length $text) . $text },
        positional => [Str], named => [width => Int, char => Str, {default => ' '}]);
    is $fill->('ab', width => 4), '  ab';
    is $fill->('ab', width => 3, char => '*'), '*ab';
    my $e = dies { $fill->('ab') };
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['missing_keyword_only_argument', ['width']]];
};

subtest 'slurpy' => sub {
    my $sum = validate_call(sub (@n) { my $s = 0; $s += $_ for @n; $s }, positional => [slurpy ArrayRef [Int]]);
    is $sum->(1, '2', 3), 6;
    my $e = dies { $sum->(1, 'x') };
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['int_parsing', [1]]];

    my $tags = validate_call(sub (%t) { join ',', map {"$_=$t{$_}"} sort keys %t },
        named => [slurpy HashRef [Int]]);
    is $tags->(b => '2', a => 1), 'a=1,b=2';
};

subtest 'methods' => sub {
    my $math = Vc::Math->new;
    is $math->times('4'), 12;
    my $e = dies { $math->times('x') };
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['int_parsing', [0]]], 'the invocant is not an argument';
};

subtest 'models and enums' => sub {
    my $norm = validate_call(sub ($p, $c) { [$p, $c] }, positional => ['Vc::Point', 'Vc::Color']);
    my ($point, $color) = @{$norm->({x => 1, y => '2'}, 'blue')};
    isa_ok $point, ['Vc::Point'];
    is $point->y, 2;
    is refaddr($color), refaddr(Vc::Color->BLUE);
    my $given = Vc::Point->new(x => 1, y => 1);
    is refaddr($norm->($given, Vc::Color->RED)->[0]), refaddr($given), 'objects given are kept';
};

subtest 'the call keeps its context' => sub {
    my $list = validate_call(sub ($n) { wantarray ? (1) x $n : 'scalar' }, positional => [Int]);
    is [$list->(3)], [1, 1, 1];
    is scalar($list->(3)), 'scalar';
    my $where = validate_call(sub () { (caller 0)[3] }, positional => []);
    like $where->(), qr/__ANON__/, 'the wrapper is not on the stack';
};

subtest 'usage' => sub {
    my $e = dies { validate_call('Vc::Math::nope', positional => [Int]) };
    isa_ok $e, ['Perldantic::UsageError'];
    like $e->message, qr/validate_call: Vc::Math::nope is not a sub/;
    $e = dies { validate_call(sub {1}, positional => [Int], bogus => 1) };
    like $e->message, qr/validate_call: unknown option 'bogus'/;
    $e = dies { validate_call(sub {1}, positional => ['nope nope']) };
    like $e->message, qr/validate_call: positional\[0\] takes a type/;
    $e = dies { validate_call(sub {1}, positional => [slurpy ArrayRef [Int]], named => [a => Int]) };
    like $e->message, qr/slurpy positional arguments cannot be combined with named ones/;
    $e = dies { validate_call(sub {1}, named => [a => Int, a => Str]) };
    like $e->message, qr/named argument 'a' is declared twice/;
    $e = dies { validate_call(sub {1}, positional => [Int, {default => 1}, Int]) };
    like $e->message, qr/validate_call: positional\[1\] has no default but follows one that does/;
    $e = dies { validate_call(sub {1}, positional => [Int, {deflt => 1}]) };
    like $e->message, qr/validate_call: positional\[0\]: unknown option 'deflt'/;
};

done_testing;
