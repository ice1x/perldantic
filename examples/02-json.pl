#!/usr/bin/env perl
# JSON in and out, and the JSON Schema of a model.
use v5.36;
use lib 'lib';

package Order {
    use Perldantic;
    has number => (is => 'ro', isa => Str, required => 1, pattern => '^SO-\d+$');
    has placed => (is => 'ro', isa => Date, required => 1);
    has total  => (is => 'ro', isa => Decimal, required => 1, decimal_places => 2);
    has lines  => (is => 'ro', isa => ArrayRef[Dict[sku => Str, qty => Int]], default => sub { [] });
}

package main;

my $order = Order->model_validate_json(
    '{"number": "SO-17", "placed": "2026-09-22", "total": "19.90", "lines": [{"sku": "A-1", "qty": 2}]}');
say $order->placed->year;                    # 2026: a Perldantic::Date
say $order->total->bstr;                     # 19.9: a Math::BigFloat, never a float
say $order->model_dump_json;                 # money stays text: "total":"19.90"
say $order->model_dump(mode => 'json')->{placed};   # 2026-09-22

my $schema = Order->model_json_schema;       # JSON Schema (Draft 2020-12), as Perl data
say join ', ', @{$schema->{required}};       # number, placed, total
