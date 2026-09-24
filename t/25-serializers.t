use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;

package Test::Room {
    use Perldantic;

    has name   => (is => 'ro', isa => Str);
    has width  => (is => 'ro', isa => Int);
    has length => (is => 'ro', isa => Int);
    has secret => (is => 'ro', isa => Str, default => 'x');

    our $SELF;

    field_serializer name => sub ($self, $value) {
        $SELF = $self;
        return uc $value;
    };

    field_serializer secret => (when_used => 'json') => sub ($self, $value, $info) {
        return '*' x length $value;
    };

    field_serializer width => (mode => 'wrap', return_type => Str) => sub ($self, $value, $handler) {
        return $handler->($value) . 'm';
    };

    computed_field area => (isa => Int) => sub ($self) { $self->width * $self->length };

    computed_field shout => (isa => Str, alias => 'SHOUT');
    sub shout ($self) { $self->name . '!' }
}

package main;

subtest 'field serializers' => sub {
    my $room = Test::Room->new(name => 'hall', width => 3, length => 4, secret => 'abc');
    my $dump = $room->model_dump;
    is $dump->{name}, 'HALL';
    ref_is $Test::Room::SELF, $room, 'field serializers get the object itself';
    is $dump->{secret}, 'abc', 'when_used => json keeps Perl dumps as they are';
    is $room->model_dump(mode => 'json')->{secret}, '***';
    is $dump->{width}, '3m', 'wrap serializers get a handler';
};

subtest 'computed fields' => sub {
    my $room = Test::Room->new(name => 'hall', width => 3, length => 4);
    is $room->area, 12, 'a computed field is a method';
    my $dump = $room->model_dump;
    is $dump->{area}, 12;
    is $dump->{shout}, 'hall!', 'an existing method';
    is $room->model_dump(by_alias => 1)->{SHOUT}, 'hall!';
    is [sort keys %{$room->model_dump(exclude => ['area', 'shout'])}], [qw(length name secret width)];
    like $room->model_dump_json, qr/"area":12,"shout":"hall!"\}\z/, 'after the fields';
    ok !exists Test::Room->new(name => 'x', width => 1, length => 1, area => 5)->{area},
        'computed fields are not input';

    my $schema = Test::Room->model_json_schema(mode => 'serialization');
    is $schema->{properties}{area}, {type => 'integer', title => 'Area', readOnly => T()};
    is $schema->{properties}{width}, {type => 'string', title => 'Width'}, 'return types describe serialization';
    ok((grep { $_ eq 'area' } @{$schema->{required}}), 'computed fields are always there');
    ok !exists Test::Room->model_json_schema->{properties}{area}, 'but not in validation mode';
};

package Test::Point {
    use Perldantic;
    has x => (is => 'ro', isa => Int);
    has y => (is => 'ro', isa => Int);

    model_serializer sub ($self, $info) {
        return $info->mode eq 'json' ? join(',', $self->x, $self->y) : [$self->x, $self->y];
    };
}

package Test::Line {
    use Perldantic;
    has from => (is => 'ro', isa => 'Test::Point');
    has to   => (is => 'ro', isa => 'Test::Point');

    model_serializer mode => 'wrap', sub ($self, $handler) {
        my $data = $handler->($self);
        return {%$data, kind => 'line'};
    };
}

package main;

subtest 'model serializers' => sub {
    my $p = Test::Point->new(x => 1, y => 2);
    is $p->model_dump, [1, 2];
    is $p->model_dump_json, '"1,2"';
    my $line = Test::Line->new(from => {x => 0, y => 0}, to => {x => 3, y => 4});
    is $line->model_dump, {from => [0, 0], to => [3, 4], kind => 'line'};
    my $ta = Perldantic::TypeAdapter->new(Perldantic::Types::ArrayRef(['Test::Point']));
    is $ta->dump_json([$p]), '["1,2"]', 'type adapters use them too';
};

package Test::Suite {
    use Perldantic;
    extends 'Test::Room';
    has floor => (is => 'ro', isa => Int, default => 0);
    computed_field level => (isa => Str) => sub ($self) { 'L' . $self->floor };
}

package main;

subtest 'inheritance' => sub {
    my $dump = Test::Suite->new(name => 'top', width => 2, length => 2, floor => 9)->model_dump;
    is [@$dump{qw(name area level)}], ['TOP', 4, 'L9'];
};

subtest 'declaration errors' => sub {
    package Test::BadSer {
        use Perldantic;
        has x => (is => 'ro', isa => Int);
        field_serializer y => sub ($self, $value) { $value };
    }
    my $e = dies { Test::BadSer->new(x => 1)->model_dump };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "field_serializer: Test::BadSer has no field 'y'";

    package Test::NoMethod {
        use Perldantic;
        computed_field missing => (isa => Int);
    }
    $e = dies { Test::NoMethod->new->model_dump };
    is $e->message, "computed_field: Test::NoMethod has no method 'missing'";

    for (
        [sub { Test::BadSer::field_serializer(x => (mode => 'before') => sub {1}) },
            "field_serializer: mode must be plain or wrap, got 'before'"],
        [sub { Test::BadSer::field_serializer(x => (when_used => 'sometimes') => sub {1}) },
            "field_serializer: when_used must be always, unless-none, json or json-unless-none, got 'sometimes'"],
        [sub { Test::BadSer::model_serializer(mode => 'after', sub {1}) },
            "model_serializer: mode must be plain or wrap, got 'after'"],
        [sub { Test::BadSer::computed_field(x => (isa => [])) },
            'computed_field x: isa must be a Perldantic type or a Perldantic model class'],
    ) {
        my ($code, $message) = @$_;
        my $e = dies { $code->() };
        isa_ok $e, 'Perldantic::UsageError';
        is $e->message, $message;
    }
};

done_testing;
