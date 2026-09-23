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
use Perldantic::Types qw(:all);
use Perldantic::Wire qw(tuple bytes);

package Leak::Node {
    use Perldantic;
    has name     => (is => 'ro', isa => Str, required => 1);
    has children => (is => 'ro', isa => ArrayRef['Leak::Node'], default => sub { [] });
    has slug     => (is => 'lazy', isa => Str);
    sub _build_slug ($self) { lc $self->name }
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
$list->validate_python([1]);
$list->dump_json([1]);

no_leaks_ok { $ints->validate([1, '2', 3]) } 'validate';
no_leaks_ok { $ints->validate_json('[1, 2]') } 'validate_json';
no_leaks_ok { local $@; eval { $ints->validate(['x', 'y']) } } 'validation errors';
no_leaks_ok { my $v = Perldantic::FFI::Validator->new({type => 'str'}) } 'validator handles';
no_leaks_ok { local $@; eval { Perldantic::FFI::Validator->new({type => 'nope'}) } } 'schema errors';
no_leaks_ok {
    local $SIG{__WARN__} = sub { };
    $ser->to_python('x');
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
no_leaks_ok { Leak::Node->core_schema } 'model schemas (no closure cycles)';
my $leaf = Leak::Node->new(name => 'leaf');
no_leaks_ok { my $node = Leak::Node->new(name => 'n', children => [$leaf, {name => 'x'}]) } 'model objects as input';
no_leaks_ok { local $@; eval { Leak::Node->new(name => 'n', children => [$leaf, 5]) } } 'errors with model objects';
no_leaks_ok { local $@; my $e = eval { Leak::Node->new(1) } || $@; "$e" } 'usage errors';
no_leaks_ok {
    my $values = $list->validate_python([1, undef, '2.5']);
    $list->dump_json($values);
    $list->json_schema;
} 'type adapters';
no_leaks_ok {
    my $adapter = Perldantic::TypeAdapter->new(Map[Str, 'Leak::Node']);
    $adapter->validate_python({x => {name => 'n'}});
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
