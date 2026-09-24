#!/usr/bin/env perl
# Perldantic against Moo + Type::Tiny on the same nested models: build N orders with 10 items
# each from plain data (validating and turning nested hashes into objects), and dump them
# back to plain data / JSON. Run after `make` (Moo and Types::Standard must be installed):
#   perl -Iblib/lib -Iblib/arch tools/bench/compare.pl [N]
use v5.36;
use Time::HiRes qw(time);
use Cpanel::JSON::XS ();

use Perldantic::TypeAdapter;

package Pd::Item {
    use Perldantic;
    has sku   => (is => 'ro', isa => Str);
    has qty   => (is => 'ro', isa => Int);
    has price => (is => 'ro', isa => Num);
}

package Pd::Order {
    use Perldantic;
    has id       => (is => 'ro', isa => Int);
    has customer => (is => 'ro', isa => Str);
    has items    => (is => 'ro', isa => ArrayRef['Pd::Item']);
}

package Moo::Item {
    use Moo;
    use Types::Standard qw(Str Int Num);
    has sku   => (is => 'ro', isa => Str, required => 1);
    has qty   => (is => 'ro', isa => Int, required => 1);
    has price => (is => 'ro', isa => Num, required => 1);
    sub to_data ($self) { +{sku => $self->sku, qty => $self->qty, price => $self->price} }
}

package Moo::Order {
    use Moo;
    use Types::Standard qw(Str Int ArrayRef InstanceOf);
    has id       => (is => 'ro', isa => Int, required => 1);
    has customer => (is => 'ro', isa => Str, required => 1);
    has items    => (
        is       => 'ro',
        isa      => ArrayRef[InstanceOf['Moo::Item']],
        required => 1,
        coerce   => sub ($items) { [map { ref $_ eq 'HASH' ? Moo::Item->new($_) : $_ } @$items] },
    );
    sub to_data ($self) { +{id => $self->id, customer => $self->customer, items => [map { $_->to_data } @{$self->items}]} }
}

package main;

my $count  = shift // 200;
my @orders = map {
    my $id = $_;
    {id => $id, customer => "customer $id", items => [map { {sku => "sku-$_", qty => $_, price => $_ * 1.5} } 1 .. 10]};
} 1 .. $count;

my $adapter = Perldantic::TypeAdapter->new(Perldantic::Types::ArrayRef(['Pd::Order']));
my $json    = Cpanel::JSON::XS->new->canonical;
my @pd      = @{$adapter->validate_python(\@orders)};
my @moo     = map { Moo::Order->new($_) } @orders;

sub timed ($label, $code) {
    $code->() for 1 .. 3;
    my ($runs, $start) = (0, time);
    $code->(), $runs++ while time - $start < 1;
    my $ms = (time - $start) / $runs * 1000;
    printf "%-34s %9.2f ms\n", $label, $ms;
    return $ms;
}

printf "%d orders x 10 items (Type::Tiny::XS %s)\n", $count, eval { require Type::Tiny::XS; 'on' } // 'off';
timed('build   Perldantic validate_python', sub { $adapter->validate_python(\@orders) });
timed('build   Moo + Type::Tiny new',       sub { [map { Moo::Order->new($_) } @orders] });
timed('dump    Perldantic dump_python',     sub { $adapter->dump_python(\@pd) });
timed('dump    Moo to_data',                sub { [map { $_->to_data } @moo] });
timed('JSON    Perldantic dump_json',       sub { $adapter->dump_json(\@pd) });
timed('JSON    Moo to_data + JSON::XS',     sub { $json->encode([map { $_->to_data } @moo]) });
