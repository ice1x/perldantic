use v5.36;
use Test2::V0;

use Perldantic::Uuid;
use Perldantic::TypeAdapter;
use Perldantic::Types qw(Uuid ArrayRef);
use Perldantic::Wire;

my $TEXT = '12345678-1234-5678-1234-567812345678';

subtest 'value class' => sub {
    my $u = Perldantic::Uuid->new($TEXT);
    is "$u", $TEXT, 'stringifies to the hyphenated form';
    is $u->hex, '12345678123456781234567812345678';
    is $u->urn, "urn:uuid:$TEXT";
    is length $u->bytes, 16;
    is $u->int, '24197857161011715162171839636988778104', 'int is the 128-bit number';
    isa_ok $u->int, 'Math::BigInt';
    is $u->version, undef, 'no version outside RFC 4122';
    is $u->variant, 'reserved for NCS compatibility';

    for my $form ('{12345678-1234-5678-1234-567812345678}', "urn:uuid:$TEXT", uc $TEXT,
        '12345678123456781234567812345678')
    {
        is Perldantic::Uuid->new($form)->as_string, $TEXT, "reads $form";
    }
    is Perldantic::Uuid->new(bytes => $u->bytes), $u, 'from 16 bytes';
    is Perldantic::Uuid->new(hex => $u->hex), $u, 'from hex';

    my $v4 = Perldantic::Uuid->new('0e7ac198-9acd-4c0c-b4b4-761974bf71d7');
    is $v4->version, 4;
    is $v4->variant, 'specified in RFC 4122';
    ok $u eq Perldantic::Uuid->new(uc $TEXT), 'equal UUIDs compare equal';
    ok $u eq $TEXT, 'and equal to their text';
    ok $u ne $v4;
    ok $v4 lt $u, 'ordered like Python: by value';
    is [sort { $a cmp $b } $u, $v4], [$v4, $u];

    my $e = dies { Perldantic::Uuid->new('nope') };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "Perldantic::Uuid->new: invalid UUID 'nope'";
    $e = dies { Perldantic::Uuid->new(bytes => 'short') };
    is $e->message, 'Perldantic::Uuid->new: bytes must be 16 bytes long, got 5';
};

subtest 'wire' => sub {
    my $u = Perldantic::Uuid->new($TEXT);
    is Perldantic::Wire::encode([$u]), qq([{"\$uuid":"$TEXT"}]);
    my $back = Perldantic::Wire::decode(qq({"\$uuid":"$TEXT"}));
    isa_ok $back, 'Perldantic::Uuid';
    is "$back", $TEXT;
};

subtest 'the Uuid type' => sub {
    my $ta = Perldantic::TypeAdapter->new(Uuid);
    my $u = $ta->validate($TEXT);
    isa_ok $u, 'Perldantic::Uuid';
    is "$u", $TEXT;
    is $ta->validate(Perldantic::Uuid->new($TEXT))->as_string, $TEXT, 'UUID objects pass';
    is $ta->validate_json(qq("$TEXT"))->as_string, $TEXT;
    is $ta->dump_json($u), qq("$TEXT");
    is $ta->dump($u, mode => 'json'), $TEXT;
    is $ta->json_schema, {type => 'string', format => 'uuid'};
    is Perldantic::TypeAdapter->new(Uuid->with(version => 4))->json_schema, {type => 'string', format => 'uuid4'},
        'a version shows in the format, as for pydantic UUID4';

    my $e = dies { $ta->validate('nope') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'uuid_parsing';
    is $e->errors->[0]{msg}, 'Input should be a valid UUID, invalid character: found `n` at 0';

    my $strict = Perldantic::TypeAdapter->new(Uuid->with(strict => 1));
    $e = dies { $strict->validate($TEXT) };
    is $e->errors->[0]{type}, 'is_instance_of', 'strict mode takes only UUID objects';

    my $v4 = Perldantic::TypeAdapter->new(Uuid->with(version => 4));
    $e = dies { $v4->validate($TEXT) };
    is $e->errors->[0]{msg}, 'UUID version 4 expected';
    $e = dies { Uuid->with(max_length => 1) };
    is $e->message, "Constraint 'max_length' does not apply to Uuid";

    is [map {"$_"} @{Perldantic::TypeAdapter->new(ArrayRef[Uuid])->validate([$TEXT, uc $TEXT])}],
        [$TEXT, $TEXT], 'nested';
};

package Test::Order {
    use Perldantic;
    use Perldantic::Types qw(Uuid);
    has id => (is => 'ro', isa => Uuid->with(version => 4));
}

package main;

subtest 'models' => sub {
    my $id = '0e7ac198-9acd-4c0c-b4b4-761974bf71d7';
    my $order = Test::Order->new(id => uc $id);
    isa_ok $order->id, 'Perldantic::Uuid';
    is $order->model_dump_json, qq({"id":"$id"});
    is Test::Order->model_validate_json($order->model_dump_json)->id, $order->id, 'JSON round trip';
    is Test::Order->model_json_schema->{properties}{id}, {type => 'string', format => 'uuid4', title => 'Id'};
    my $e = dies { Test::Order->new(id => $TEXT) };
    is $e->errors->[0]{loc}, ['id'];
};

done_testing;
