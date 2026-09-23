use v5.36;
use Test2::V0;

package Shop::Item {
    use Perldantic;

    has sku   => (is => 'ro', isa => Str, required => 1);
    has price => (is => 'ro', isa => Num, required => 1, ge => 0);
    has qty   => (is => 'rw', isa => Int, default => 1);
    has note  => (is => 'ro', isa => Maybe[Str], alias => 'comment');
}

package Shop::Order {
    use Perldantic;

    model_config title => 'Order', extra => 'allow';
    has id    => (is => 'ro', isa => Int, required => 1);
    has items => (is => 'ro', isa => ArrayRef['Shop::Item'], default => sub { [] });
    has tags  => (is => 'ro', isa => ArrayRef[Str]);
}

package main;

my $order = Shop::Order->new(
    id    => 7,
    items => [{sku => "caf\x{e9}", price => '2.5', comment => undef}, {sku => 'b', price => 1, qty => 3}],
    coupon => 'X1',
);

subtest 'model_validate and model_validate_json' => sub {
    my $o = Shop::Order->model_validate({id => '1', items => [{sku => 'a', price => 0}]});
    isa_ok $o, 'Shop::Order';
    isa_ok $o->items->[0], 'Shop::Item';
    my $j = Shop::Order->model_validate_json(qq({"id": 2, "items": [{"sku": "caf\xc3\xa9", "price": 1}]}));
    is $j->items->[0]->sku, "caf\x{e9}", 'JSON bytes are decoded';
    is Shop::Order->model_validate_json(qq({"id": 3, "items": [{"sku": "\x{263a}", "price": 1}]}))->items->[0]->sku,
        "\x{263a}", 'JSON text works too';
    my $e = dies { Shop::Item->model_validate({sku => 'a', price => '1'}, strict => 1) };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'float_type', 'options such as strict are passed on';
    $e = dies { Shop::Item->model_validate_json('{"sku": 1') };
    is $e->errors->[0]{type}, 'json_invalid';
    $e = dies { Shop::Item->model_validate([]) };
    is $e->errors->[0]{type}, 'model_type';
};

subtest 'model_dump' => sub {
    is $order->model_dump, {
        id     => 7,
        items  => [{sku => "caf\x{e9}", price => 2.5, qty => 1, note => undef}, {sku => 'b', price => 1, qty => 3}],
        coupon => 'X1',
    }, 'nested models, extras; absent fields are left out';
    is $order->model_dump(exclude_unset => 1, exclude => {coupon => 1}),
        {id => 7, items => [{sku => "caf\x{e9}", price => 2.5, note => undef}, {sku => 'b', price => 1, qty => 3}]};
    is $order->items->[0]->model_dump(by_alias => 1, exclude_none => 1), {sku => "caf\x{e9}", price => 2.5, qty => 1};
    is $order->model_dump(include => {items => {0 => {sku => 1}}}), {items => [{sku => "caf\x{e9}"}]};
    is [sort keys %{$order->model_dump(exclude => [qw(items coupon)])}], ['id'], 'arrays of names work too';
    is $order->model_dump(exclude_defaults => 1)->{items}[0], {sku => "caf\x{e9}", price => 2.5};
    my $e = dies { $order->model_dump(bogus => 1) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "got an unexpected keyword argument 'bogus'";
};

subtest 'model_dump_json' => sub {
    my $json = $order->items->[1]->model_dump_json;
    is $json, '{"sku":"b","price":1.0,"qty":3}', 'field order is kept';
    is $order->items->[0]->model_dump_json(by_alias => 1),
        qq({"sku":"caf\xc3\xa9","price":2.5,"qty":1,"comment":null}), 'UTF-8 encoded';
    is $order->items->[1]->model_dump_json(indent => 2), qq({\n  "sku": "b",\n  "price": 1.0,\n  "qty": 3\n});
};

subtest 'model_json_schema' => sub {
    my $schema = Shop::Order->model_json_schema;
    is $schema->{title}, 'Order';
    is $schema->{required}, ['id'];
    is $schema->{additionalProperties}, T();
    is $schema->{properties}{items}{items}, {'$ref' => '#/$defs/Item'};
    is $schema->{'$defs'}{Item}{title}, 'Shop::Item', 'nested models are definitions';
    is $schema->{properties}{tags}, {type => 'array', items => {type => 'string'}, title => 'Tags'},
        'fields without a plain default show none';
    is Shop::Item->model_json_schema(by_alias => 0)->{properties}{note}{title}, 'Note';
    is Shop::Item->model_json_schema->{properties}{comment}{title}, 'Comment';
    is Shop::Item->model_json_schema(mode => 'serialization')->{properties}{comment}{title}, 'Comment';
};

subtest 'model_copy, model_fields_set, model_extra' => sub {
    my $item = $order->items->[1];
    is [$item->model_fields_set], [qw(price qty sku)];
    is [$order->items->[0]->model_fields_set], [qw(note price sku)];
    is $order->model_extra, {coupon => 'X1'};
    is $item->model_extra, undef, 'undef without extra => allow';

    my $copy = $item->model_copy(update => {qty => 9});
    isa_ok $copy, 'Shop::Item';
    is $copy->qty, 9;
    is $item->qty, 3, 'the original is unchanged';
    is [$copy->model_fields_set], [qw(price qty sku)];

    my $shallow = $order->model_copy;
    ok $shallow->items == $order->items, 'shallow by default';
    my $deep = $order->model_copy(deep => 1);
    ok $deep->items != $order->items, 'deep copies nested data';
    ok $deep->items->[0] != $order->items->[0];
    isa_ok $deep->items->[0], 'Shop::Item';
    is [$deep->items->[0]->model_fields_set], [qw(note price sku)];

    my $e = dies { $item->model_copy(bogus => 1) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "model_copy: unknown option 'bogus'";

    $item->qty(4);
    is [$item->model_fields_set], [qw(price qty sku)];
    my $fresh = Shop::Item->new(sku => 'z', price => 1);
    $fresh->qty(2);
    is [$fresh->model_fields_set], [qw(price qty sku)], 'writes mark fields as set';
};

subtest 'class and instance methods are checked' => sub {
    my $e = dies { Shop::Item->model_dump };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'model_dump is an object method';
    $e = dies { $order->model_validate({}) };
    is $e->message, 'model_validate is a class method';
    $e = dies { Shop::Item->model_validate({}, 'strict') };
    is $e->message, 'model_validate takes options as key => value pairs';
};

done_testing;
