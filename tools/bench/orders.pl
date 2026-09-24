#!/usr/bin/env perl
# Time the Perl <-> core round trip on nested models: validation and dumps of N orders with
# 10 items each (docs/PLAN.md stage 00057). Run after `make`:
#   perl -Iblib/lib -Iblib/arch tools/bench/orders.pl [N]
use v5.36;
use Time::HiRes qw(time);

use Perldantic::Model;
use Perldantic::TypeAdapter;

package Bench::Item {
    use Perldantic;
    has sku   => (is => 'ro', isa => Str);
    has qty   => (is => 'ro', isa => Int);
    has price => (is => 'ro', isa => Num);
}

package Bench::Order {
    use Perldantic;
    has id       => (is => 'ro', isa => Int);
    has customer => (is => 'ro', isa => Str);
    has items    => (is => 'ro', isa => ArrayRef['Bench::Item']);
}

package main;

my $count  = shift // 200;
my @orders = map {
    my $id = $_;
    {id => $id, customer => "customer $id", items => [map { {sku => "sku-$_", qty => $_, price => $_ * 1.5} } 1 .. 10]};
} 1 .. $count;
my $adapter = Perldantic::TypeAdapter->new(Perldantic::Types::ArrayRef(['Bench::Order']));
my $objects = $adapter->validate_python(\@orders);

sub timed ($label, $code) {
    $code->() for 1 .. 3;
    my ($runs, $start) = (0, time);
    $code->(), $runs++ while time - $start < 1;
    printf "%-22s %9.2f ms\n", $label, (time - $start) / $runs * 1000;
}

printf "%d orders x 10 items\n", $count;
timed(validate_python => sub { $adapter->validate_python(\@orders) });
timed(dump_python     => sub { $adapter->dump_python($objects) });
timed(dump_json       => sub { $adapter->dump_json($objects) });
