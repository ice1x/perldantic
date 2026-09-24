use v5.36;
use Test2::V0;

package Test::Signup {
    use Perldantic;

    has name     => (is => 'ro', isa => Str);
    has age      => (is => 'ro', isa => Int);
    has tags     => (is => 'ro', isa => ArrayRef[Str], default => sub { [] });
    has password => (is => 'ro', isa => Str);
    has confirm  => (is => 'ro', isa => Str);

    field_validator age => sub ($class, $value) {
        die "must be an adult\n" if $value < 18;
        return $value;
    };

    # comma-separated text is accepted for a list
    field_validator tags => (mode => 'before') => sub ($class, $value) {
        return ref $value ? $value : [split /,/, $value];
    };

    field_validator [qw(name)] => sub ($class, $value, $info) {
        return join ':', $info->field_name, ucfirst $value;
    };

    model_validator mode => 'after', sub ($self) {
        die "passwords do not match\n" if $self->password ne $self->confirm;
        return $self;
    };
}

package main;

subtest 'field validators' => sub {
    my $user = Test::Signup->new(name => 'ada', age => '36', tags => 'math,poetry', password => 'x', confirm => 'x');
    is $user->age, 36, 'after validators see the validated value';
    is $user->tags, ['math', 'poetry'], 'before validators see the input';
    is $user->name, 'name:Ada', 'info names the field';

    my $e = dies { Test::Signup->new(name => 'kid', age => 9, password => 'x', confirm => 'x') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->title, 'Test::Signup';
    is $e->errors->[0]{loc}, ['age'];
    is $e->errors->[0]{type}, 'value_error';
    is $e->errors->[0]{msg}, 'Value error, must be an adult';
    $e = dies { Test::Signup->new(name => 'kid', age => 'old', password => 'x', confirm => 'x') };
    is $e->errors->[0]{type}, 'int_parsing', 'the type is checked before an after validator';
};

subtest 'model validators' => sub {
    my $e = dies { Test::Signup->new(name => 'ada', age => 36, password => 'x', confirm => 'y') };
    is $e->errors->[0]{loc}, [];
    is $e->errors->[0]{msg}, 'Value error, passwords do not match';
    my $json = Test::Signup->model_validate_json('{"name":"ada","age":36,"password":"p","confirm":"p"}');
    is $json->name, 'name:Ada', 'JSON input goes through the same validators';
};

package Test::Order {
    use Perldantic;

    has qty   => (is => 'ro', isa => Int);
    has price => (is => 'ro', isa => Num);
    has total => (is => 'rw', isa => Num, default => 0);
    has seen  => (is => 'rw', isa => Str, default => '');

    our $BUILDS = 0;
    sub BUILD ($self, $args) { $BUILDS++ }

    field_validator qty => (mode => 'wrap') => sub ($class, $value, $handler, $info) {
        my $qty = eval { $handler->($value) };
        return $qty // 1;
    };

    field_validator price => (mode => 'plain') => sub ($class, $value) {
        return $value =~ s/\A\$//r + 0;
    };

    model_validator mode => 'before', sub ($class, $data) {
        return {%$data, qty => $data->{quantity} // $data->{qty}};
    };

    model_validator mode => 'after', sub ($self, $info) {
        $self->total($self->qty * $self->price);
        $self->seen(join ',', sort keys %{$info->context // {}});
        return $self;
    };
}

package main;

subtest 'modes and info' => sub {
    local $Test::Order::BUILDS = 0;
    my $order = Test::Order->model_validate({quantity => '3', price => '$2.5'}, context => {user => 1});
    isa_ok $order, 'Test::Order';
    is $order->qty, 3, 'before model validators rewrite the input';
    is $order->price, 2.5, 'plain field validators replace the type';
    is $order->total, 7.5, 'after model validators get the object';
    is $order->seen, 'user', 'and the context';
    is $Test::Order::BUILDS, 1, 'BUILD runs once';
    is Test::Order->new(qty => 'lots', price => 1)->qty, 1, 'wrap field validators get a handler';
    is Test::Order->model_json_schema->{properties}{qty}, {type => 'integer', title => 'Qty'},
        'the JSON Schema shows the validated type';
};

subtest 'type adapters' => sub {
    require Perldantic::TypeAdapter;
    my $orders = Perldantic::TypeAdapter->new(Perldantic::Types::ArrayRef(['Test::Order']));
    my $list = $orders->validate_json('[{"qty": 2, "price": 3}]');
    isa_ok $list->[0], 'Test::Order';
    is $list->[0]->total, 6, 'model validators run for JSON input';
};

package Test::Admin {
    use Perldantic;
    extends 'Test::Signup';

    has level => (is => 'ro', isa => Int, default => 1);

    field_validator level => sub ($class, $value) { die "too high\n" if $value > 3; $value };
}

package main;

subtest 'inheritance' => sub {
    my $admin = Test::Admin->new(name => 'root', age => 40, password => 'a', confirm => 'a', level => 2);
    is $admin->name, 'name:Root', 'parent field validators apply';
    my $e = dies { Test::Admin->new(name => 'r', age => 40, password => 'a', confirm => 'b') };
    is $e->errors->[0]{msg}, 'Value error, passwords do not match', 'and parent model validators';
    $e = dies { Test::Admin->new(name => 'r', age => 40, password => 'a', confirm => 'a', level => 9) };
    is $e->errors->[0]{loc}, ['level'];
};

subtest 'declaration errors' => sub {
    package Test::Broken {
        use Perldantic;
        has x => (is => 'ro', isa => Int);
        field_validator y => sub ($class, $value) { $value };
    }
    my $e = dies { Test::Broken->new(x => 1) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "field_validator: Test::Broken has no field 'y'";

    for (
        [sub { Test::Broken::field_validator(x => (mode => 'sideways') => sub {1}) },
            "field_validator: mode must be before, after, wrap or plain, got 'sideways'"],
        [sub { Test::Broken::field_validator(x => 'not code') }, 'field_validator: the last argument must be a code reference'],
        [sub { Test::Broken::model_validator(sub {1}) }, 'model_validator: mode must be before, after or wrap'],
        [sub { Test::Broken::model_validator(mode => 'plain', sub {1}) },
            'model_validator: mode must be before, after or wrap'],
    ) {
        my ($code, $message) = @$_;
        my $e = dies { $code->() };
        isa_ok $e, 'Perldantic::UsageError';
        is $e->message, $message;
    }
};

done_testing;
