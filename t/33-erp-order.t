use v5.36;
use Test2::V0;

use JSON::PP;
use Math::BigFloat;

# An ERP sales order: lines with money as decimals (never floats), discounts, tax, and totals
# that must agree with what the upstream system claims. Orders arrive as JSON from a web shop
# and are posted to accounting as JSON.

sub money ($amount) { Math::BigFloat->new($amount)->bfround(-2, 'common') }

package ERP::Line {
    use Perldantic;

    # defaults become decimals too
    model_config extra => 'forbid', validate_default => 1;

    has sku        => (is => 'ro', isa => Str, required => 1, pattern => '^[A-Z]{3}-\d{4}$');
    has qty        => (is => 'ro', isa => Int, required => 1, gt => 0);
    has unit_price => (is => 'ro', isa => Decimal, required => 1, ge => 0, max_digits => 12, decimal_places => 2);
    has discount   => (is => 'ro', isa => Decimal, default => 0, ge => 0, le => 100, decimal_places => 1);

    # qty × price, less the discount percentage, to the cent
    computed_field amount => (isa => Decimal) => sub ($self) {
        my $gross = $self->unit_price->copy->bmul($self->qty);
        return main::money($gross - $gross * $self->discount / 100);
    };
}

package ERP::Order {
    use Perldantic;

    model_config extra => 'forbid', validate_default => 1;

    has number   => (is => 'ro', isa => Str, required => 1, pattern => '^SO-\d{6}$');
    has customer => (is => 'ro', isa => Str, required => 1, min_length => 1);
    has currency => (is => 'ro', isa => Enum[qw(EUR USD GBP)], default => 'EUR');
    has ordered  => (is => 'ro', isa => Date, required => 1);
    has ships    => (is => 'ro', isa => Maybe[Date]);
    has lines    => (is => 'ro', isa => ArrayRef['ERP::Line'], required => 1, min_length => 1);
    has tax_rate => (is => 'ro', isa => Decimal, default => '0.20', ge => 0, lt => 1);
    # what the shop computed; checked against the lines
    has total    => (is => 'ro', isa => Decimal, required => 1, decimal_places => 2);

    computed_field net => (isa => Decimal) => sub ($self) {
        my $net = Math::BigFloat->bzero;
        $net->badd($_->amount) for @{$self->lines};
        return main::money($net);
    };

    computed_field tax => (isa => Decimal) => sub ($self) {
        main::money($self->net->copy->bmul($self->tax_rate));
    };

    model_validator mode => 'after', sub ($self) {
        die "an order cannot ship before it is placed\n"
            if defined $self->ships && $self->ships->iso lt $self->ordered->iso;
        my $expected = $self->net->copy->badd($self->tax);
        die "the total @{[$self->total->bstr]} does not match the lines (@{[$expected->bstr]})\n"
            if $self->total->bcmp($expected) != 0;
        my %seen;
        $seen{$_->sku}++ && die "@{[$_->sku]} is on more than one line\n" for @{$self->lines};
        return $self;
    };
}

package main;

# From the web shop: prices as strings, the way JSON APIs send money.
my $posted = <<'JSON';
{
  "number": "SO-004217",
  "customer": "Northwind Traders",
  "ordered": "2026-09-22",
  "ships": "2026-09-25",
  "lines": [
    {"sku": "CHR-0001", "qty": 4, "unit_price": "149.90"},
    {"sku": "DSK-0203", "qty": 1, "unit_price": "899.00", "discount": "12.5"},
    {"sku": "CBL-0042", "qty": 25, "unit_price": "0.35"}
  ],
  "total": "1673.98"
}
JSON

subtest 'an order from the shop' => sub {
    my $order = ERP::Order->model_validate_json($posted);
    isa_ok $order->lines->[0]->unit_price, 'Math::BigFloat';
    is [map { $_->amount->bstr } @{$order->lines}], ['599.60', '786.63', '8.75'], 'amounts to the cent';
    is $order->net->bstr, '1394.98';
    is $order->tax->bstr, '279.00';
    is $order->currency, 'EUR';
    is $order->lines->[0]->discount->bstr, '0', 'no discount by default';

    # 0.1 + 0.2 in floats is not 0.3; in decimals it is
    my $cheap = ERP::Order->new(number => 'SO-000001', customer => 'x', ordered => '2026-09-22', tax_rate => 0,
        lines => [{sku => 'AAA-0001', qty => 1, unit_price => '0.10'}, {sku => 'AAA-0002', qty => 1, unit_price => '0.20'}],
        total => '0.30');
    is $cheap->net->bstr, '0.30';
};

subtest 'totals and dates across fields' => sub {
    my %order = (number => 'SO-000002', customer => 'Acme', ordered => '2026-09-22',
        lines => [{sku => 'CHR-0001', qty => 2, unit_price => '10.00'}]);
    ok lives { ERP::Order->new(%order, total => '24.00') }, 'net 20 + 20% tax';
    my $e = dies { ERP::Order->new(%order, total => '24.01') };
    is $e->errors->[0]{msg}, 'Value error, the total 24.01 does not match the lines (24.00)';
    $e = dies { ERP::Order->new(%order, total => '24.00', ships => '2026-09-21') };
    is $e->errors->[0]{msg}, 'Value error, an order cannot ship before it is placed';
    $e = dies { ERP::Order->new(%order, total => '48.00', lines => [($order{lines}[0]) x 2]) };
    is $e->errors->[0]{msg}, 'Value error, CHR-0001 is on more than one line';
};

subtest 'money is checked where it comes in' => sub {
    my $e = dies {
        ERP::Order->new(number => 'SO-42', customer => '', ordered => '2026-09-22', total => '1.005', tax_rate => '1.5',
            lines => [{sku => 'chr-1', qty => 0, unit_price => 'free', discount => '120', note => 'rush'}])
    };
    is [sort map { join('.', @{$_->{loc}}) . ":$_->{type}" } @{$e->errors}], [
        'customer:string_too_short',
        'lines.0.discount:less_than_equal',
        'lines.0.note:extra_forbidden',
        'lines.0.qty:greater_than',
        'lines.0.sku:string_pattern_mismatch',
        'lines.0.unit_price:decimal_parsing',
        'number:string_pattern_mismatch',
        'tax_rate:less_than',
        'total:decimal_max_places',
    ];
    $e = dies { ERP::Line->new(sku => 'CHR-0001', qty => 1, unit_price => 1.1) };
    ok !$e, 'a Perl number is fine when its digits are' or note $e;
    $e = dies { ERP::Line->new(sku => 'CHR-0001', qty => 1, unit_price => '1234567890123.00') };
    is $e->errors->[0]{type}, 'decimal_max_digits';
};

subtest 'posting to accounting' => sub {
    my $order = ERP::Order->model_validate_json($posted);
    my $json = $order->model_dump_json(exclude => ['ships']);
    like $json, qr/"unit_price":"149.90"/, 'money stays text, with its cents';
    like $json, qr/"net":"1394.98","tax":"279.00"\}\z/, 'computed totals are posted too';
    my $data = $order->model_dump;
    isa_ok $data->{total}, 'Math::BigFloat';
    is $order->model_dump(mode => 'json')->{total}, '1673.98';

    my $schema = ERP::Order->model_json_schema;
    is $schema->{'$defs'}{Line}{properties}{unit_price},
        {anyOf => [{type => 'number', minimum => 0}, {type => 'string'}], title => 'Unit Price'};
};

# An accounting service: its entry points validate their arguments like the orders themselves.
package ERP::Ledger {
    use v5.36;
    use Perldantic::Call qw(validate_call);
    use Perldantic::Types qw(ArrayRef Date Decimal Str);

    our @POSTED;

    # post(order, on => date, memo => text): the order is validated as the model
    validate_call post => (positional => ['ERP::Order'], named => [on => Date, memo => Str, {default => ''}],
        returns => Str);
    sub post ($order, %entry) {
        push @POSTED, [$order, $entry{on}, $entry{memo}];
        return 'JE-' . $order->number =~ s/\D//gr;
    }

    # a correction: an amount of money for an order number
    validate_call correct => (named => [order => Str, {default => 'SO-000000'}, amount => Decimal]);
    sub correct (%c) { "$c{order}: " . $c{amount}->bstr }
}

subtest 'posting through the ledger service' => sub {
    local @ERP::Ledger::POSTED;
    my $order = ERP::Order->model_validate_json($posted);
    is ERP::Ledger::post($order, on => '2026-09-26'), 'JE-004217';
    my ($entry) = @ERP::Ledger::POSTED;
    ref_is $entry->[0], $order, 'the order object as it was given';
    isa_ok $entry->[1], ['Perldantic::Date'];
    is $entry->[2], '', 'the memo default';

    my $json_order = JSON::PP->new->decode($posted);
    is ERP::Ledger::post($json_order, on => '2026-09-26', memo => 'from the shop'), 'JE-004217';
    isa_ok $ERP::Ledger::POSTED[-1][0], ['ERP::Order'], 'an order built from plain data';
    my $e = dies { ERP::Ledger::post({%$json_order, total => '1.00'}, on => 'someday') };
    is [sort map { join('.', @{$_->{loc}}) . ":$_->{type}" } @{$e->errors}],
        ['0:value_error', 'on:date_from_datetime_parsing'], 'the order and the date are both checked';
    is $e->title, 'ERP::Ledger::post';

    is ERP::Ledger::correct(amount => '12.50'), 'SO-000000: 12.5', 'a decimal';
    $e = dies { ERP::Ledger::correct(amount => 'lots') };
    is $e->errors->[0]{loc}, ['amount'];
};

done_testing;
