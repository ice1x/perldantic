use v5.36;
use Test2::V0;

use Scalar::Util qw(weaken);
use Perldantic::FFI;
use Perldantic::Error;

sub compile ($schema, $config = undef) { Perldantic::FFI::Validator->new($schema, $config) }
sub serializer ($schema) { Perldantic::FFI::Serializer->new($schema) }

sub function_schema ($kind, $function, %args) {
    my $info = delete $args{info};
    return {
        type     => "function-$kind",
        function => {type => $info ? 'with-info' : 'no-info', function => $function},
        ($kind eq 'plain' ? () : (schema => delete $args{schema} // {type => 'int'})),
        %args,
    };
}

sub double ($value) { $value * 2 }

subtest 'before, after and plain validators' => sub {
    is compile(function_schema(before => \&double))->validate(21), 42;
    my $upper = compile(function_schema(plain => sub ($value) { uc $value }));
    is $upper->validate('abc'), 'ABC';
    my $after = compile(function_schema(after => sub ($value) { $value + 1 }));
    is $after->validate('41'), 42, 'the inner schema runs first';
};

subtest 'string exceptions are value errors' => sub {
    my $even = compile(function_schema(after => sub ($value) {
        die "must be even\n" if $value % 2;
        return $value;
    }));
    is $even->validate(4), 4;
    my $e = dies { $even->validate('3') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{type}, 'value_error';
    is $e->errors->[0]{msg}, 'Value error, must be even';
    is $e->errors->[0]{input}, '3', 'the error points at the original input';

    my $where = compile(function_schema(plain => sub ($value) { die 'bad value' }));
    $e = dies { $where->validate(1) };
    is $e->errors->[0]{msg}, 'Value error, bad value', 'the location Perl adds is dropped';
};

subtest 'pydantic errors raised from Perl' => sub {
    my $custom = compile(function_schema(plain => sub ($value) {
        Perldantic::CustomError->throw(type => 'my_error', message => 'bad {thing}', context => {thing => $value});
    }));
    my $e = dies { $custom->validate(1) };
    is [$e->errors->[0]{type}, $e->errors->[0]{msg}, $e->errors->[0]{ctx}], ['my_error', 'bad 1', {thing => 1}];

    my $known = compile(function_schema(plain => sub ($value) {
        Perldantic::KnownError->throw(type => 'greater_than', context => {gt => 5});
    }));
    $e = dies { $known->validate(1) };
    is $e->errors->[0]{msg}, 'Input should be greater than 5';

    my $omit = compile({
        type         => 'list',
        items_schema => function_schema(plain => sub ($value) {
            Perldantic::Omit->throw if $value == 2;
            return $value;
        }),
    });
    is $omit->validate([1, 2, 3]), [1, 3];

    my $default = compile({
        type   => 'typed-dict',
        fields => {a => {
            type   => 'typed-dict-field',
            schema => {
                type    => 'default',
                default => 7,
                schema  => function_schema(plain => sub ($value) { Perldantic::UseDefault->throw }),
            },
        }},
    });
    is $default->validate({a => 1}), {a => 7};
};

package Test::Kaput { sub new ($class) { bless {}, $class } }

package main;

subtest 'other exceptions pass through unchanged' => sub {
    my $object = Test::Kaput->new;
    my $v = compile(function_schema(plain => sub ($value) { die $object }));
    my $e = dies { $v->validate(1) };
    ref_is $e, $object, 'the very same object';
    $e = dies { $v->validate(1) };
    ref_is $e, $object, 'again';
};

subtest 'wrap validators get a handler' => sub {
    my $fallback = compile(function_schema(wrap => sub ($value, $handler) {
        my $out = eval { $handler->($value) };
        return $out // -1;
    }));
    is $fallback->validate('5'), 5;
    is $fallback->validate('x'), -1;

    my $located = compile(function_schema(wrap => sub ($value, $handler) { $handler->($value, 'here') }));
    my $e = dies { $located->validate('x') };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{loc}, ['here'];
    is $e->errors->[0]{type}, 'int_parsing';

    my $seen;
    my $reraise = compile(function_schema(wrap => sub ($value, $handler) {
        my $out = eval { $handler->($value) };
        $seen = $@;
        die $@ if $@;
        return $out;
    }));
    $e = dies { $reraise->validate('x') };
    isa_ok $seen, 'Perldantic::ValidationError';
    is $seen->title, 'ValidatorCallable';
    is $e->errors->[0]{type}, 'int_parsing', 're-raised handler errors are reported as they were';
};

subtest 'info' => sub {
    my $info;
    my $v = compile({
        type   => 'typed-dict',
        fields => Perldantic::Wire::ordered(
            a => {type => 'typed-dict-field', schema => {type => 'int'}},
            b => {type => 'typed-dict-field', schema => function_schema(
                after => sub ($value, $i) { $info = $i; $value }, info => 1, schema => {type => 'str'})},
        ),
    });
    is $v->validate({a => 1, b => 'x'}, {context => {c => 1}}), {a => 1, b => 'x'};
    isa_ok $info, 'Perldantic::ValidationInfo';
    is $info->field_name, 'b';
    is $info->data, {a => 1};
    is $info->context, {c => 1};
    is $info->mode, 'perl';
    is $info->config, undef;
};

subtest 'serializer functions' => sub {
    my $ten = serializer({type => 'int', serialization => {type => 'function-plain', function => sub ($v) { $v * 10 }}});
    is $ten->to_python(1), 10;
    is $ten->to_json(2), '20';

    my $info;
    my $described = serializer({
        type          => 'int',
        serialization => {type => 'function-plain', info_arg => !!1, function => sub ($v, $i) { $info = $i; "$v" }},
    });
    is $described->to_json(3, {context => {a => 1}}), '"3"';
    isa_ok $info, 'Perldantic::SerializationInfo';
    is $info->mode, 'json';
    is $info->context, {a => 1};
    ok !$info->exclude_none;

    my $wrap = serializer({
        type          => 'int',
        serialization => {type => 'function-wrap', function => sub ($v, $handler) { $handler->($v) + 1 }},
    });
    is $wrap->to_python(1), 2;

    my $boom = serializer({type => 'int', serialization => {type => 'function-plain', function => sub ($v) { die "nope\n" }}});
    my $e = dies { $boom->to_python(1) };
    isa_ok $e, 'Perldantic::SerializationError';
    like $e->message, qr/\AError calling function `__ANON__`: /;
};

subtest 'handlers only work while their function runs' => sub {
    my $kept;
    my $v = compile(function_schema(wrap => sub ($value, $handler) { $kept = $handler; $handler->($value) }));
    is $v->validate('7'), 7;
    my $e = dies { $kept->(1) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'A handler can only be called while its function runs';

    my $items = serializer({
        type          => 'list',
        items_schema  => {type => 'int'},
        serialization => {
            type     => 'function-wrap',
            schema   => {type => 'int'},
            function => sub ($list, $handler) {
                my @out;
                for my $i (0 .. $#$list) {
                    my $item = eval { $handler->($list->[$i], $i) };
                    push @out, $item // ref $@;
                }
                return \@out;
            },
        },
    });
    is $items->to_python([1, 2, 3], {exclude => [1]}), [1, 'Perldantic::Omit', 3],
        'items filtered out by index raise Perldantic::Omit';
};

subtest 'functions live as long as their validators' => sub {
    my $offset = 1;
    # a closure: a sub that captures nothing is shared by Perl and never freed
    my $v = compile(function_schema(plain => sub ($value) { $value + $offset }));
    is $v->validate(1), 2;
    my $sub = $v->{functions}[0];
    weaken $sub;
    ok $sub, 'kept by the validator';
    undef $v;
    ok !$sub, 'released with it';
};

done_testing;
