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
# The same shape as a Type::Tiny type, for checking without building objects.
my $tt_orders = do {
    use Types::Standard qw(ArrayRef Dict Int Num Str);
    ArrayRef [Dict [id => Int, customer => Str, items => ArrayRef [Dict [sku => Str, qty => Int, price => Num]]]];
};
my $json    = Cpanel::JSON::XS->new->canonical;
my @pd      = @{$adapter->validate(\@orders)};
my @moo     = map { Moo::Order->new($_) } @orders;
my @lazy    = @{$adapter->validate(\@orders, lazy => 1)};

# Every field of every order read, as code using the objects would.
sub read_all ($orders) {
    for my $order (@$orders) {
        $order->id;
        $order->customer;
        for my $item (@{$order->items}) { $item->sku; $item->qty; $item->price }
    }
}

# The median of the calls made in two seconds: other load on the machine skews a mean.
sub timed ($label, $code) {
    $code->() for 1 .. 3;
    my (@times, $start);
    for ($start = time; time - $start < 2;) {
        my $call = time;
        $code->();
        push @times, time - $call;
    }
    my $ms = (sort { $a <=> $b } @times)[@times / 2] * 1000;
    printf "%-34s %9.2f ms\n", $label, $ms;
    return $ms;
}

printf "%d orders x 10 items (Type::Tiny::XS %s)\n", $count, eval { require Type::Tiny::XS; 'on' } // 'off';
timed('build   Perldantic validate', sub { $adapter->validate(\@orders) });
timed('build   Moo + Type::Tiny new',       sub { [map { Moo::Order->new($_) } @orders] });
timed('lazy    Perldantic validate lazy',   sub { $adapter->validate(\@orders, lazy => 1) });
timed('lazy    ... and every field read',   sub { read_all($adapter->validate(\@orders, lazy => 1)) });
timed('lazy    Perldantic, then read',      sub { read_all($adapter->validate(\@orders)) });
timed('lazy    Moo new, then read',         sub { read_all([map { Moo::Order->new($_) } @orders]) });
timed('check   Perldantic check',       sub { $adapter->check(\@orders) });
timed('check   Type::Tiny check',       sub { $tt_orders->check(\@orders) });
timed('dump    Perldantic dump',     sub { $adapter->dump(\@pd) });
timed('dump    Moo to_data',                sub { [map { $_->to_data } @moo] });
timed('JSON    Perldantic dump_json',       sub { $adapter->dump_json(\@pd) });
timed('JSON    Moo to_data + JSON::XS',     sub { $json->encode([map { $_->to_data } @moo]) });
timed('lazy    dump_json of lazy objects',  sub { $adapter->dump_json(\@lazy) });
