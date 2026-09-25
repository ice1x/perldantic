#!/usr/bin/env perl
# Any type, without a model: TypeAdapter; enum classes; validated subs.
use v5.36;
use lib 'lib';
use Perldantic::TypeAdapter;
use Perldantic::Call qw(validate_call);
use Perldantic::Types qw(ArrayRef HashRef Int Str);

# TypeAdapter: validate, dump and describe any type
my $scores = Perldantic::TypeAdapter->new(HashRef[ArrayRef[Int]]);
my $valid  = $scores->validate({ann => ['1', 2], bob => []});
say $valid->{ann}[0] + 10;                   # 11
say $scores->check({ann => ['x']}) ? 'ok' : 'invalid';   # invalid

# enum classes
package Priority {
    use Perldantic::Enum LOW => 1, HIGH => 2;
}
package main;
my $priority = Perldantic::TypeAdapter->new('Priority')->validate('2');
say $priority->name;                         # HIGH

# validated subs (pydantic's validate_call)
validate_call repeat => (positional => [Str], named => [times => Int, {default => 2}], returns => Str);
sub repeat ($text, %opt) { $text x $opt{times} }
say repeat('ab', times => '3');              # ababab
