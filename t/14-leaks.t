use v5.36;
use Test2::V0;

# Leak checks. Test::LeakTrace finds Perl values that are never freed; the Rust side (handles and
# result strings) is checked through the process size over many calls.

BEGIN {
    eval { require Test::LeakTrace; Test::LeakTrace->import('no_leaks_ok'); 1 }
        or skip_all('Test::LeakTrace is not installed');
}

use Perldantic::FFI;
use Perldantic::TypeAdapter;
use Perldantic::Types qw(ArrayRef Maybe Num Str Map DateTime Decimal Uuid Url);
use Perldantic::Wire qw(tuple bytes);

package Leak::Node {
    use Perldantic;
    has name     => (is => 'ro', isa => Str, required => 1);
    has children => (is => 'ro', isa => ArrayRef['Leak::Node'], default => sub { [] });
    has slug     => (is => 'lazy', isa => Str);
    sub _build_slug ($self) { lc $self->name }
}

package Leak::Checked {
    use Perldantic;
    has low  => (is => 'ro', isa => Int);
    has high => (is => 'ro', isa => Int);
    field_validator low => sub ($class, $value, $info) { $value < 0 ? die "negative\n" : $value };
    model_validator mode => 'after', sub ($self) { die "reversed\n" if $self->low > $self->high; $self };
}

package Leak::Shape {
    use Perldantic;
    has w => (is => 'ro', isa => Int);
    has h => (is => 'ro', isa => Int);
    field_serializer w => sub ($self, $value, $info) { $value * 2 };
    computed_field area => (isa => Int) => sub ($self) { $self->w * $self->h };
}

package Leak::Color {
    use Perldantic::Enum RED => 'red', GREEN => 'green';
}

package Leak::Paint {
    use Perldantic;
    has color => (is => 'ro', isa => 'Leak::Color', default => Leak::Color->RED);
}

package main;

my $ints = Perldantic::FFI::Validator->new({type => 'list', items_schema => {type => 'int'}});
my $ser  = Perldantic::FFI::Serializer->new({type => 'int'});
my $tree = {name => 'root', children => [{name => 'a'}, {name => 'b', children => [{name => 'c'}]}]};
my $list = Perldantic::TypeAdapter->new(ArrayRef[Maybe[Num]]);

# Compile everything once: compiled validators are cached on purpose. Blocks that catch errors
# localize $@, which otherwise keeps the last error alive after the block.
Leak::Node->new($tree)->model_dump;
Leak::Node->model_json_schema;
$list->validate([1]);
$list->dump_json([1]);

no_leaks_ok { $ints->validate([1, '2', 3]) } 'validate';
no_leaks_ok { $ints->validate_json('[1, 2]') } 'validate_json';
no_leaks_ok { local $@; eval { $ints->validate(['x', 'y']) } } 'validation errors';
no_leaks_ok { my $v = Perldantic::FFI::Validator->new({type => 'str'}) } 'validator handles';
no_leaks_ok { local $@; eval { Perldantic::FFI::Validator->new({type => 'nope'}) } } 'schema errors';
no_leaks_ok {
    local $SIG{__WARN__} = sub { };
    $ser->to_perl('x');
    $ser->to_json(5, {indent => 2});
} 'serialization and warnings';
no_leaks_ok { Perldantic::FFI::json_schema({type => 'tuple', items_schema => [{type => 'bytes'}]}) } 'json_schema';
no_leaks_ok { Perldantic::Wire::decode(Perldantic::Wire::encode([tuple(1, 0.1), bytes('b'), {a => undef}])) } 'wire codec';

no_leaks_ok {
    my $node = Leak::Node->new($tree);
    $node->children->[1]->slug;
    $node->model_dump;
    $node->model_dump_json;
    $node->model_copy(deep => 1);
} 'models';
no_leaks_ok { local $@; eval { Leak::Node->new(children => [{}]) } } 'model validation errors';
Leak::Node->model_validate($tree, lazy => 1)->children->[1]->name;
no_leaks_ok {
    my $node = Leak::Node->model_validate($tree, lazy => 1);
    $node->model_dump_json;
    my $child = $node->children->[1];
    $child->children->[0]->slug;
    $node->model_dump;
    Leak::Node->model_validate_json('{"name": "j", "children": [{"name": "k"}]}', lazy => 1)->children;
} 'lazy objects';
ok Perldantic::FFI::_lazy_live() == 0, 'the core holds no lazy model once its objects are gone';
no_leaks_ok { Leak::Node->core_schema } 'model schemas (no closure cycles)';
Leak::Paint->new(color => 'green')->model_dump_json;
no_leaks_ok {
    my $paint = Leak::Paint->new(color => 'green');
    Leak::Paint->new(color => Leak::Color->RED)->model_dump;
    $paint->model_dump_json;
    Leak::Paint->model_validate_json('{"color": "red"}', lazy => 1)->color;
} 'enum members';
no_leaks_ok { local $@; eval { Leak::Paint->new(color => 'pink') } } 'enum errors';
my $other = Perldantic::FFI::Validator->new({type => 'enum', cls => 'Leak::Other',
    members => [Perldantic::Wire::Enum->new(class => 'Leak::Other', name => 'A', value => 1)]});
$other->validate(1);
no_leaks_ok {
    my $member = Perldantic::Wire::decode('{"$enum": {"class": "Leak::Other", "name": "A", "value": 1}}');
    $other->validate(1);
} 'members of classes Perl does not know';
my $leaf = Leak::Node->new(name => 'leaf');
no_leaks_ok { my $node = Leak::Node->new(name => 'n', children => [$leaf, {name => 'x'}]) } 'model objects as input';
no_leaks_ok { local $@; eval { Leak::Node->new(name => 'n', children => [$leaf, 5]) } } 'errors with model objects';
no_leaks_ok { local $@; my $e = eval { Leak::Node->new(1) } || $@; "$e" } 'usage errors';
my $when = Perldantic::TypeAdapter->new(ArrayRef[DateTime]);
$when->validate(['2022-06-08T12:00Z']);
no_leaks_ok {
    my $values = $when->validate(['2022-06-08T12:00Z', 1654646400]);
    $when->dump_json($values);
    my $sorted = $values->[0] <=> $values->[1];
} 'dates and times';
my $others = Perldantic::TypeAdapter->new(ArrayRef[Decimal]);
my $ids = Perldantic::TypeAdapter->new(Uuid);
my $urls = Perldantic::TypeAdapter->new(Url);
$others->validate(['1.50']);
$urls->validate('https://example.com')->host;
no_leaks_ok {
    my $values = $others->validate(['1.50', 2]);
    $others->dump_json($values);
    my $id = $ids->validate('12345678123456781234567812345678');
    my $url = $urls->validate('https://example.com/a?b=1');
    my @parts = ($url->host, $url->query_params, "$id");
} 'decimals, UUIDs and URLs';
my $red = Perldantic::Wire::Enum->new(class => 'Color', name => 'RED', value => 1);
my $colors = Perldantic::FFI::Validator->new({type => 'enum', cls => 'Color', members => [$red]});
my $color_dump = Perldantic::FFI::Serializer->new({type => 'enum', cls => 'Color', members => [$red]});
no_leaks_ok {
    my $member = $colors->validate(1);
    $color_dump->to_json($member);
    local $@;
    eval { $colors->validate(2) };
} 'enum members';
Leak::Checked->new(low => 1, high => 2);
no_leaks_ok {
    Leak::Checked->new(low => 1, high => 2);
    local $@;
    eval { Leak::Checked->new(low => -1, high => 2) };
    eval { Leak::Checked->new(low => 3, high => 2) };
} 'model and field validators';
my $checked = Perldantic::FFI::Validator->new({
    type     => 'function-wrap',
    function => {type => 'with-info', function => sub ($value, $handler, $info) {
        my $out = eval { $handler->($value) };
        die "not a number\n" if !defined $out;
        return $out;
    }},
    schema => {type => 'int'},
});
my $tens = Perldantic::FFI::Serializer->new(
    {type => 'int', serialization => {type => 'function-plain', function => sub ($v) { $v * 10 }}});
my $kaput = Perldantic::FFI::Validator->new(
    {type => 'function-plain', function => {type => 'no-info', function => sub ($v) { die bless {}, 'Leak::Kaput' }}});
$checked->validate(1);
no_leaks_ok {
    $checked->validate('2');
    local $@;
    eval { $checked->validate('x') };
    eval { $kaput->validate(1) };
    $tens->to_json(3);
} 'functions in schemas';
no_leaks_ok {
    my $offset = 1;
    my $v = Perldantic::FFI::Validator->new(
        {type => 'function-plain', function => {type => 'no-info', function => sub ($x) { $x + $offset }}});
    $v->validate(1);
} 'validators with their own functions';
my $shape = Leak::Shape->new(w => 2, h => 3);
$shape->model_dump;
no_leaks_ok {
    $shape->model_dump;
    $shape->model_dump_json;
} 'serializers and computed fields';
my $things = Perldantic::TypeAdapter->new(ArrayRef['Leak::Thing']);
my $thing = bless {}, 'Leak::Thing';
$things->validate([$thing]);
no_leaks_ok {
    my $got = $things->validate([$thing]);
    $things->dump($got);
    local $@;
    eval { $things->validate([{}]) };
} 'host objects';
no_leaks_ok {
    my $values = $list->validate([1, undef, '2.5']);
    $list->dump_json($values);
    $list->json_schema;
} 'type adapters';
no_leaks_ok {
    my $adapter = Perldantic::TypeAdapter->new(Map[Str, 'Leak::Node']);
    $adapter->validate({x => {name => 'n'}});
} 'new type adapters';

# Memory owned by the core: a leaked result string or handle per call would add megabytes here.
SKIP: {
    my $rss = sub { my $kb = `ps -o rss= -p $$ 2>/dev/null`; $kb =~ /(\d+)/ ? $1 : undef };
    skip 'ps is not available', 1 if !defined $rss->();
    for (1 .. 2000) { $ints->validate([1, 2]); eval { $ints->validate(['x']) } }
    my $before = $rss->();
    for (1 .. 50_000) {
        $ints->validate([1, '2', 3]);
        eval { $ints->validate(['x']) };
        my $handle = Perldantic::FFI::Validator->new({type => 'int'});
    }
    my $growth = $rss->() - $before;
    ok $growth < 8 * 1024, "the core frees what it returns (grew ${growth} KB over 150k calls)";
}

done_testing;
