use v5.36;
use Test2::V0;

use Perldantic::FFI;
use Perldantic::Types qw(:all);

subtest 'simple types' => sub {
    my %expected = (
        Any => 'any', Undef => 'none', Bool => 'bool', Int => 'int', Num => 'float',
        Str => 'str', Bytes => 'bytes',
    );
    for my $name (sort keys %expected) {
        my $type = Perldantic::Types->can($name)->();
        isa_ok $type, 'Perldantic::Type';
        is $type->name, $name;
        is "$type", $name, 'stringifies to its name';
        is $type->core_schema, {type => $expected{$name}};
    }
};

subtest 'parameterized types' => sub {
    my @cases = (
        [ArrayRef,                  'ArrayRef',            {type => 'list'}],
        [ArrayRef[Int],             'ArrayRef[Int]',       {type => 'list', items_schema => {type => 'int'}}],
        [Maybe[Str],                'Maybe[Str]',          {type => 'nullable', schema => {type => 'str'}}],
        [HashRef,                   'HashRef',             {type => 'dict', keys_schema => {type => 'str'}}],
        [HashRef[Num],              'HashRef[Num]',
            {type => 'dict', keys_schema => {type => 'str'}, values_schema => {type => 'float'}}],
        [Map,                       'Map',                 {type => 'dict'}],
        [Map[Int, ArrayRef[Bool]],  'Map[Int,ArrayRef[Bool]]',
            {type => 'dict', keys_schema => {type => 'int'},
                values_schema => {type => 'list', items_schema => {type => 'bool'}}}],
        [Tuple,                     'Tuple',
            {type => 'tuple', items_schema => [{type => 'any'}], variadic_item_index => 0}],
        [Tuple[Int, Str],           'Tuple[Int,Str]',
            {type => 'tuple', items_schema => [{type => 'int'}, {type => 'str'}]}],
        [Tuple[Int, slurpy ArrayRef[Str]], 'Tuple[Int,slurpy ArrayRef[Str]]',
            {type => 'tuple', items_schema => [{type => 'int'}, {type => 'str'}], variadic_item_index => 1}],
        [Tuple[Int, slurpy ArrayRef], 'Tuple[Int,slurpy ArrayRef]',
            {type => 'tuple', items_schema => [{type => 'int'}, {type => 'any'}], variadic_item_index => 1}],
        [Enum[qw(open done)],       'Enum["open","done"]', {type => 'literal', expected => ['open', 'done']}],
        [Literal[1, 'a', undef],    'Literal[1,"a",undef]', {type => 'literal', expected => [1, 'a', undef]}],
        [Dict[name => Str, age => Optional[Int]], 'Dict[name=>Str,age=>Optional[Int]]',
            {type => 'typed-dict', fields => {
                name => {type => 'typed-dict-field', schema => {type => 'str'}, required => T()},
                age  => {type => 'typed-dict-field', schema => {type => 'int'}, required => F()},
            }}],
        [InstanceOf['My::Class'],   'InstanceOf["My::Class"]', {type => 'is-instance', cls => 'My::Class'}],
        [Optional[Int],             'Optional[Int]',       {type => 'int'}],
    );
    for my $case (@cases) {
        my ($type, $name, $schema) = @$case;
        is $type->name, $name;
        is $type->core_schema, $schema, "$name schema";
    }
    my $optional = Optional[Int];
    ok $optional->is_optional, 'Optional[] marks a slot that may be left out';
    ok !Int->is_optional;
    my $dict = Dict[b => Int, a => Str];
    is [map {"$_"} $dict->parameters], ['b', 'Int', 'a', 'Str'],
        'Dict[] keeps the declared field order';
};

subtest 'core_schema returns a fresh copy' => sub {
    my $type = ArrayRef[Int];
    $type->core_schema->{items_schema}{type} = 'str';
    is $type->core_schema->{items_schema}, {type => 'int'};
};

subtest 'constraints' => sub {
    is Int->with(gt => 0, le => 10)->core_schema, {type => 'int', gt => 0, le => 10};
    is Str->with(max_length => 3)->name, 'Str', 'constraints keep the name';
    my $maybe = Maybe[Str];
    is $maybe->with(pattern => '^a')->core_schema,
        {type => 'nullable', schema => {type => 'str', pattern => '^a'}},
        'Maybe[] passes constraints to its type';
    my $list = ArrayRef[Int];
    is $list->with(min_length => 1)->core_schema,
        {type => 'list', items_schema => {type => 'int'}, min_length => 1};
    is Int->core_schema, {type => 'int'}, 'with() does not change the original';
    my $e = dies { Int->with(max_length => 3) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, "Constraint 'max_length' does not apply to Int";
    my $enum = Enum[qw(a)];
    $e = dies { $enum->with(strict => 1) };
    is $e->message, q{Constraint 'strict' does not apply to Enum["a"]};
};

subtest 'bad parameters are usage errors' => sub {
    my @cases = (
        [sub { ArrayRef[1] },            'ArrayRef[] takes a type, got 1'],
        [sub { ArrayRef[Int, Str] },     'ArrayRef[] takes 1 parameter, got 2'],
        [sub { Maybe[] },                'Maybe[] takes 1 parameter, got 0'],
        [sub { Map[Int] },               'Map[] takes 2 parameters, got 1'],
        [sub { Tuple[slurpy ArrayRef, Int] }, 'Tuple[] takes slurpy only as its last parameter'],
        [sub { Tuple[Optional[Int]] },   'Optional[] is only supported inside Dict[]'],
        [sub { ArrayRef[Optional[Int]] }, 'Optional[] is only supported inside Dict[]'],
        [sub { Enum[] },                 'Enum[] takes at least 1 value'],
        [sub { Enum[[1]] },              'Enum[] takes strings, got ARRAY'],
        [sub { Literal[{}] },            'Literal[] takes plain values, got HASH'],
        [sub { Dict['a'] },              'Dict[] takes name => type pairs'],
        [sub { Dict[a => 'Int'] },       'Dict[] takes a type for a, got Int'],
        [sub { InstanceOf[Int] },        'InstanceOf[] takes a class name'],
        [sub { slurpy Int },             'slurpy takes ArrayRef or ArrayRef[T], got Int'],
        [sub { ArrayRef('x') },          'ArrayRef takes its parameters as ArrayRef[...]'],
    );
    for my $case (@cases) {
        my $e = dies { $case->[0]->() };
        isa_ok $e, 'Perldantic::UsageError';
        is $e->message, $case->[1];
    }
};

subtest 'types validate through the core' => sub {
    my $v = sub ($type) { Perldantic::FFI::Validator->new($type->core_schema) };
    is $v->(ArrayRef[Int])->validate([1, '2']), [1, 2];
    is $v->(Maybe[Str])->validate(undef), undef;
    is $v->(Map[Int, Str])->validate({1 => 'a'}), {1 => 'a'};
    is $v->(Tuple[Int, slurpy ArrayRef[Str]])->validate([1, 'a', 'b']), [1, 'a', 'b'];
    is $v->(Enum[qw(open done)])->validate('done'), 'done';
    my $e = dies { $v->(Enum[qw(open done)])->validate('nope') };
    is $e->errors->[0]{type}, 'literal_error';
    $e = dies { $v->(Int->with(gt => 0))->validate(0) };
    is $e->errors->[0]{type}, 'greater_than';
};

done_testing;
