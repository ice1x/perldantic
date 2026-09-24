use v5.36;
use Test2::V0;

# Errors speak Perl: codes, messages, contexts, input values and types (docs/DIVERGENCES.md #8).

package Test::Vocab {
    use Perldantic;

    has tags   => (is => 'ro', isa => ArrayRef[Str]);
    has meta   => (is => 'ro', isa => HashRef[Int]);
    has none   => (is => 'ro', isa => Undef);
    has ratio  => (is => 'ro', isa => Num);
    has count  => (is => 'ro', isa => Int);
    has took   => (is => 'ro', isa => Duration);
    has pair   => (is => 'ro', isa => Tuple[Int, Str]);
    has price  => (is => 'ro', isa => Decimal, strict => 1);
    has shape  => (is => 'ro', isa => Int);
    has top    => (is => 'ro', isa => ArrayRef[Int], min_length => 2);

    field_validator shape => (mode => 'before') => sub ($class, $value) {
        Perldantic::KnownError->throw(type => 'hash_type') if ref $value ne 'HASH';
        return 1;
    };
}

package main;

my $e = dies {
    Test::Vocab->new(tags => {a => 1}, meta => [1], none => !!1, ratio => 'x', count => 1.5, took => 'soon',
        pair => 'p', price => '1.5', shape => "a\n")
};
isa_ok $e, 'Perldantic::ValidationError';
is [map { [$_->{loc}[0], $_->{type}, $_->{msg}] } @{$e->errors}], [
    [tags  => 'array_type',        'Input should be an array reference'],
    [meta  => 'hash_type',         'Input should be a hash reference'],
    [none  => 'undef_required',    'Input should be undef'],
    [ratio => 'number_parsing',    'Input should be a valid number, unable to parse string as a number'],
    [count => 'int_from_fraction', 'Input should be a valid integer, got a number with a fractional part'],
    [took  => 'duration_parsing',  'Input should be a valid duration, invalid digit in duration'],
    [pair  => 'array_type',        'Input should be an array reference'],
    [price => 'is_instance_of',    'Input should be an instance of Math::BigFloat'],
    [shape => 'hash_type',         'Input should be a hash reference'],
], 'Perl codes and words';
is $e->errors->[7]{ctx}, {class => 'Math::BigFloat'};
ok !grep({ exists $_->{url} } @{$e->errors}), 'no links to pydantic pages';

my $text = "$e";
unlike $text, qr/pydantic\.dev/;
like $text, qr/\Q[type=array_type, input_value={a => 1}, input_type=HashRef]\E/;
like $text, qr/\Q[type=hash_type, input_value=[1], input_type=ArrayRef]\E/;
like $text, qr/\Q[type=undef_required, input_value=!!1, input_type=Bool]\E/;
like $text, qr/\Q[type=int_from_fraction, input_value=1.5, input_type=Num]\E/;
like $text, qr/\Q[type=hash_type, input_value="a\n", input_type=Str]\E/;

my $too_short = dies { Test::Vocab->new(top => [1]) };
is $too_short->errors->[0]{ctx}{field_type}, 'Array';

done_testing;
