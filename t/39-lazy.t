use v5.36;
use Test2::V0;

use Scalar::Util qw(refaddr);

use Perldantic::FFI;
use Perldantic::TypeAdapter;
use Perldantic::Types qw(ArrayRef);

skip_all 'lazy objects need the native part' if !$Perldantic::Wire::XS;

# Lazy objects: validated data stays in the core until a field is read.

package Lz::Item {
    use Perldantic;
    has sku  => (is => 'ro', isa => Str, required => 1);
    has qty  => (is => 'rw', isa => Int, default => 1);
    has note => (is => 'ro', isa => Maybe [Str], predicate => 'has_note');
}

package Lz::Order {
    use Perldantic;
    has id     => (is => 'ro', isa => Int, required => 1);
    has items  => (is => 'ro', isa => ArrayRef ['Lz::Item'], required => 1);
    has by_sku => (is => 'ro', isa => HashRef ['Lz::Item'], default => sub { {} });
}

package Lz::Always {
    use Perldantic;
    model_config lazy => 1;
    has name => (is => 'ro', isa => Str, required => 1);
    has item => (is => 'ro', isa => Maybe ['Lz::Item']);
}

package Lz::Built {
    use Perldantic;
    has n => (is => 'ro', isa => Int, required => 1);
    our $BUILT = 0;
    sub BUILD ($self, $args) { $BUILT++ }
}

package Lz::Open {
    use Perldantic;
    model_config extra => 'allow';
    has a => (is => 'ro', isa => Int, required => 1);
}

package main;

my $data = {
    id     => 7,
    items  => [{sku => 'a', qty => 2}, {sku => 'b', note => 'fragile'}],
    by_sku => {c => {sku => 'c', qty => 3}},
};

# Unread: nothing in the hash yet.
sub unread ($object) { !%$object }

# Every object in a value read in full, as plain data: [class, fields, fields set].
sub read_all ($value) {
    if (Scalar::Util::blessed $value && $value->isa('Perldantic::Model')) {
        my @set = $value->model_fields_set;
        return [ref $value, {map { $_ => read_all($value->{$_}) } keys %$value}, \@set];
    }
    return [map { read_all($_) } @$value] if ref $value eq 'ARRAY';
    return {map { $_ => read_all($value->{$_}) } keys %$value} if ref $value eq 'HASH';
    return $value;
}

subtest 'objects are built in full unless asked' => sub {
    my $order = Lz::Order->model_validate($data);
    ok !unread($order), 'fields are in the hash';
    ok !unread($order->items->[0]);
};

subtest 'lazy => 1 keeps fields in the core until one is read' => sub {
    my $order = Lz::Order->model_validate($data, lazy => 1);
    isa_ok $order, ['Lz::Order'];
    ok unread($order), 'nothing read yet';
    is $order->id, 7, 'a field is read';
    ok !unread($order), 'and the object holds its fields from then on';
    is $order->{id}, 7;

    my $item = $order->items->[0];
    isa_ok $item, ['Lz::Item'];
    ok unread($item), 'nested objects are lazy too';
    is $item->sku, 'a';
    is $item->qty, 2;
    ok !$item->has_note, 'a field not given is not there';
    ok $order->items->[1]->has_note, 'the predicate reads the object';
    is $order->by_sku->{c}->qty, 3, 'objects in hashes';
};

subtest 'a lazy object read in full is the object built in full' => sub {
    my $eager = Lz::Order->model_validate($data);
    my $lazy  = Lz::Order->model_validate($data, lazy => 1);
    is read_all($lazy), read_all($eager);
    is [$lazy->items->[1]->model_fields_set], [qw(note sku)], 'fields set come from validation';
};

subtest 'dumping lazy objects' => sub {
    my $eager = Lz::Order->model_validate($data);
    my $lazy  = Lz::Order->model_validate($data, lazy => 1);
    is $lazy->model_dump, $eager->model_dump;
    is $lazy->model_dump(exclude_unset => 1), $eager->model_dump(exclude_unset => 1);
    is $lazy->model_dump_json, $eager->model_dump_json;
    ok unread($lazy->items->[0]), 'objects of classes that fill no field in Perl are dumped without being read';
    my $adapter = Perldantic::TypeAdapter->new(ArrayRef ['Lz::Order']);
    is $adapter->dump([$lazy]), [$eager->model_dump], 'inside other data';

    my $items = Lz::Order->model_validate($data, lazy => 1)->items;
    is(Perldantic::TypeAdapter->new(ArrayRef ['Lz::Item'])->dump($items), $eager->model_dump->{items});
    ok unread($items->[0]), 'a list of them';
};

subtest 'changing a lazy object' => sub {
    my $order = Lz::Order->model_validate($data, lazy => 1);
    my $item  = $order->items->[1];
    $item->qty(9);
    is $item->qty, 9;
    is [$item->model_fields_set], [qw(note qty sku)];
    is $order->model_dump->{items}[1], {sku => 'b', qty => 9, note => 'fragile'}, 'dumps see the change';

    my $copy = Lz::Order->model_validate($data, lazy => 1)->items->[0]->model_copy(update => {qty => 5});
    is $copy->model_dump, {sku => 'a', qty => 5}, 'copies';
};

subtest 'model_extra and model_fields_set read the object' => sub {
    my $open = Lz::Open->model_validate({a => 1, b => 2}, lazy => 1);
    is $open->model_extra, {b => 2};
    is [$open->model_fields_set], [qw(a b)];
};

subtest 'model_config lazy => 1, overridden per call' => sub {
    my $always = Lz::Always->model_validate({name => 'x', item => {sku => 'a'}});
    ok unread($always), 'model_validate';
    ok unread(Lz::Always->new(name => 'x')), 'new';
    ok unread(Lz::Always->model_validate_json('{"name": "x"}')), 'model_validate_json';
    ok !unread(Lz::Always->model_validate({name => 'x'}, lazy => 0)), 'lazy => 0';
    ok unread($always->item), 'the call decides for every object it returns';
    is $always->item->sku, 'a';
    ok !unread(Lz::Order->new(id => 1, items => [])), 'other classes are not lazy';
};

subtest 'JSON input' => sub {
    my $order = Lz::Order->model_validate_json('{"id": 1, "items": [{"sku": "a"}]}', lazy => 1);
    ok unread($order);
    is $order->items->[0]->sku, 'a';
};

subtest 'TypeAdapter' => sub {
    my $items = [{sku => 'a'}, {sku => 'b'}];
    my $adapter = Perldantic::TypeAdapter->new(ArrayRef ['Lz::Item'], config => {lazy => 1});
    my $lazy = $adapter->validate($items);
    ok unread($lazy->[0]), 'config => {lazy => 1}';
    is $lazy->[1]->sku, 'b';
    ok !unread($adapter->validate($items, lazy => 0)->[0]), 'lazy => 0';
    ok unread(Perldantic::TypeAdapter->new(ArrayRef ['Lz::Item'])->validate($items, lazy => 1)->[0]), 'lazy => 1';
    ok unread(Perldantic::TypeAdapter->new('Lz::Always')->validate({name => 'x'})), 'a model class follows its config';
};

subtest 'classes with work to do on building are built in full' => sub {
    local $Lz::Built::BUILT = 0;
    my $built = Lz::Built->model_validate({n => 1}, lazy => 1);
    ok !unread($built);
    is $Lz::Built::BUILT, 1, 'BUILD ran';
};

subtest 'objects given as input are kept' => sub {
    my $item  = Lz::Item->new(sku => 'x');
    my $order = Lz::Order->model_validate({id => 1, items => [$item]}, lazy => 1);
    is refaddr($order->items->[0]), refaddr($item), 'an object given';

    my $lazy_item = Lz::Item->model_validate({sku => 'y', qty => 4}, lazy => 1);
    my $again = Lz::Order->model_validate({id => 2, items => [$lazy_item]});
    is refaddr($again->items->[0]), refaddr($lazy_item), 'a lazy object given';
    is $again->items->[0]->qty, 4;
    is $again->model_dump->{items}, [{sku => 'y', qty => 4}];
};

subtest 'check takes the option and ignores it' => sub {
    ok(Lz::Order->model_check($data, lazy => 1));
    ok(Perldantic::TypeAdapter->new(ArrayRef ['Lz::Item'])->check([{sku => 'a'}], lazy => 1));
};

subtest 'invalid input' => sub {
    my $e = dies { Lz::Order->model_validate({id => 'x', items => []}, lazy => 1) };
    isa_ok $e, ['Perldantic::ValidationError'];
    is [map { $_->{loc} } @{$e->errors}], [['id']];
};

subtest 'the core lets go of the data with the objects' => sub {
    my $before = Perldantic::FFI::_lazy_live();
    {
        my $order = Lz::Order->model_validate($data, lazy => 1);
        ok Perldantic::FFI::_lazy_live() > $before, 'held while the objects live';
        my $item = $order->items->[0];
        $order->model_dump;
        is $item->sku, 'a';
    }
    is Perldantic::FFI::_lazy_live(), $before, 'released with them';
};

done_testing;
