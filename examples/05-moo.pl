#!/usr/bin/env perl
# Perldantic types in a plain Moo class: isa, and coercions that build model objects.
use v5.36;
use lib 'lib';

package Address {
    use Perldantic;
    has city => (is => 'ro', isa => Str, required => 1);
    has zip  => (is => 'ro', isa => Str, required => 1, pattern => '^\d{5}$');
}

package Customer {
    use Moo;
    use Perldantic::Types qw(InstanceOf Int Str);

    has name    => (is => 'ro', isa => Str->with(min_length => 1), required => 1);
    has age     => (is => 'ro', isa => Int->with(ge => 0), coerce => Int->coercion);
    has address => (is => 'ro', isa => InstanceOf['Address'], coerce => InstanceOf(['Address'])->coercion);
}

package main;

my $customer = Customer->new(name => 'Ann', age => '41', address => {city => 'Riga', zip => '10101'});
say ref $customer->address;                  # Address
say $customer->age + 1;                      # 42
eval { Customer->new(name => 'Bob', address => {city => 'Riga', zip => 'none'}) };
say $@ =~ /String should match pattern/ ? 'bad zip rejected' : $@;
