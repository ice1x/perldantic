#!/usr/bin/env perl
# Models: declare fields the Moo way, get validated, converted objects and structured errors.
use v5.36;
use lib 'lib';
use Scalar::Util qw(blessed);

package Person {
    use Perldantic;
    has name  => (is => 'ro', isa => Str, required => 1, min_length => 1);
    has email => (is => 'ro', isa => Str, pattern => '^[^@]+@[^@]+$');
}

package Ticket {
    use Perldantic;
    model_config extra => 'forbid';

    has id     => (is => 'ro', isa => Int, required => 1, gt => 0);
    has title  => (is => 'rw', isa => Str, required => 1, max_length => 200);
    has status => (is => 'ro', isa => Enum[qw(open done)], default => 'open');
    has tags   => (is => 'ro', isa => ArrayRef[Str], default => sub { [] });
    has owner  => (is => 'ro', isa => Maybe['Person']);
}

package main;

# input is converted: "42" becomes 42, the hash becomes a Person
my $ticket = Ticket->new(id => '42', title => 'Login fails', owner => {name => 'Ann'});
say $ticket->id + 1;              # 43
say $ticket->owner->name;         # Ann
say $ticket->status;              # open

# invalid input is a Perldantic::ValidationError with every problem in it
eval { Ticket->new(id => 0, title => 'x', colour => 'red') };
if (blessed $@ && $@->isa('Perldantic::ValidationError')) {
    say $@->error_count;                                          # 2
    say join('.', @{$_->{loc}}), ": $_->{msg}" for @{$@->errors};
    # id: Input should be greater than 0
    # colour: Extra inputs are not permitted
}
