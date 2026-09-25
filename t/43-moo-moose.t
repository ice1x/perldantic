use v5.36;
use Test2::V0;

use Scalar::Util qw(refaddr);

use Perldantic::Types qw(ArrayRef Int InstanceOf Str);

# Perldantic types in Moo and Moose classes, and in Type::Tiny: the Type::API constraint
# protocol (check, get_message, has_coercion, coerce) and Moo's code-reference isa.

package Mm::Point {
    use Perldantic;
    has x => (is => 'ro', isa => Int, required => 1);
    has y => (is => 'ro', isa => Int, required => 1);
}

subtest 'the constraint protocol' => sub {
    my $ints = ArrayRef [Int->with(gt => 0)];
    ok $ints->check([1, 2]);
    ok $ints->check([1, '2']), 'what validation accepts';
    ok !$ints->check([1, 0]);
    ok !$ints->check('x');
    like $ints->get_message([1, 0]), qr/validation error for ArrayRef\[Int\].*\n1\n  Input should be greater than 0/s;
    ok $ints->has_coercion;
    is $ints->coerce([1, '2']), [1, 2], 'coerce gives the validated value';
    my $bad = [0];
    ref_is $ints->coerce($bad), $bad, 'and invalid values as they are';
    is $ints->coercion->(['3']), [3], 'coercion: the same as a code reference';
    my $point = InstanceOf ['Mm::Point'];
    isa_ok $point->coerce({x => 1, y => 2}), ['Mm::Point'], 'models are built';
};

subtest 'as a code reference: Moo isa' => sub {
    my $type = ArrayRef [Int];
    my $check = \&$type;
    ok lives { $check->([1]) };
    my $e = dies { $check->(['x']) };
    isa_ok $e, ['Perldantic::ValidationError'];
    is $e->errors->[0]{loc}, [0];
};

subtest 'Moo' => sub {
    skip_all 'Moo is not installed' if !eval { require Moo; 1 };
    my $class = eval q{
        package Mm::Order;
        use Moo;
        use Perldantic::Types qw(ArrayRef Int InstanceOf Str);
        has id    => (is => 'ro', isa => Int->with(gt => 0), required => 1);
        has tags  => (is => 'ro', isa => ArrayRef [Str], default => sub { [] });
        has where => (is => 'ro', isa => InstanceOf ['Mm::Point'], coerce => InstanceOf(['Mm::Point'])->coercion);
        has qty   => (is => 'rw', isa => Int, coerce => Int->coercion);
        __PACKAGE__;
    } or die $@;
    my $order = $class->new(id => 7, tags => ['a'], where => {x => 1, y => '2'}, qty => '3');
    isa_ok $order->where, ['Mm::Point'], 'coerced from a hash';
    is $order->where->y, 2;
    is $order->qty, 3, 'coerced to a number';
    my $e = dies { $class->new(id => 0) };
    like "$e", qr/Input should be greater than 0/, 'Moo reports the validation error';
    $e = dies { $order->qty('many') };
    like "$e", qr/int_parsing|valid integer/;
    my $given = Mm::Point->new(x => 1, y => 1);
    is refaddr($class->new(id => 1, where => $given)->where), refaddr($given), 'objects are kept';
};

subtest 'Type::Tiny' => sub {
    skip_all 'Type::Tiny is not installed' if !eval { require Types::TypeTiny; 1 };
    my $tt = Types::TypeTiny::to_TypeTiny(ArrayRef [Int]);
    isa_ok $tt, ['Type::Tiny'];
    is $tt->display_name, 'ArrayRef[Int]';
    ok $tt->check([1]);
    ok !$tt->check(['x']);
    is $tt->coerce(['2']), [2];
    like dies { $tt->assert_valid(['x']) }, qr/validation error for ArrayRef\[Int\]/;
};

subtest 'Moose' => sub {
    skip_all 'Moose is not installed' if !eval { require Moose; 1 };
    my $class = eval q{
        package Mm::Crate;
        use Moose;
        use Perldantic::Types qw(ArrayRef Int InstanceOf);
        use Types::TypeTiny ();
        has size  => (is => 'ro', isa => Types::TypeTiny::to_TypeTiny(Int->with(gt => 0)), required => 1);
        has items => (is => 'ro', isa => Types::TypeTiny::to_TypeTiny(ArrayRef [Int]), coerce => 1);
        has where => (is => 'ro', isa => Types::TypeTiny::to_TypeTiny(InstanceOf ['Mm::Point']), coerce => 1);
        __PACKAGE__->meta->make_immutable;
        __PACKAGE__;
    } or die $@;
    my $crate = $class->new(size => 2, items => ['1', 2], where => {x => 0, y => 0});
    is $crate->items, [1, 2], 'coerced';
    isa_ok $crate->where, ['Mm::Point'];
    like dies { $class->new(size => 0) }, qr/greater than 0/;
};

done_testing;
