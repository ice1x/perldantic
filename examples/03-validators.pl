#!/usr/bin/env perl
# Validators and serializers: pydantic's decorators as declarations.
use v5.36;
use lib 'lib';

package Signup {
    use Perldantic;
    has username => (is => 'ro', isa => Str, required => 1);
    has password => (is => 'ro', isa => Str, required => 1, min_length => 8);
    has confirm  => (is => 'ro', isa => Str, required => 1);

    # one field: normalize, or die with a message
    field_validator username => sub ($class, $name) {
        die "must be alphanumeric\n" if $name !~ /\A\w+\z/;
        return lc $name;
    };

    # the whole object, after the fields
    model_validator mode => 'after', sub ($self) {
        die "passwords do not match\n" if $self->password ne $self->confirm;
        return $self;
    };

    # never write the password out
    field_serializer password => sub ($self, $value) { '********' };
}

package main;

my $signup = Signup->new(username => 'Ann', password => 'correct horse', confirm => 'correct horse');
say $signup->username;                       # ann
say $signup->model_dump(exclude => ['confirm'])->{password};   # ********

eval { Signup->new(username => 'ann!', password => 'short', confirm => 'other') };
say "$_->{loc}[0]: $_->{msg}" for @{$@->errors};
# username: Value error, must be alphanumeric
# password: String should have at least 8 characters
