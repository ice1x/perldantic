use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
use Perldantic::Types qw(Set FrozenSet Int Str);
use Perldantic::Wire qw(set frozenset);

subtest 'Set[] keeps distinct items in order' => sub {
    my $ta = Perldantic::TypeAdapter->new(Set[Int]);
    is $ta->validate_python([3, '1', 3, 1.0, 2]), [3, 1, 2], 'duplicates by value are dropped';
    is $ta->validate_json('[1, 1, 2]'), [1, 2];
    is $ta->dump_json([1, 2]), '[1,2]';
    ok no_warnings { $ta->dump_python([1, 2]) }, 'Perl arrays serialize as sets without warnings';
    is $ta->json_schema, {type => 'array', uniqueItems => !!1, items => {type => 'integer'}};
    my $e = dies { $ta->validate_python([1, 'x']) };
    is $e->errors->[0]{loc}, [1];
    is $e->errors->[0]{type}, 'int_parsing';
    $e = dies { $ta->validate_python('abc') };
    is $e->errors->[0]{msg}, 'Input should be an array reference', 'Perl wording';
};

subtest 'unhashable items and lengths' => sub {
    my $any = Perldantic::TypeAdapter->new(Set);
    my $e = dies { $any->validate_python([[1], 2]) };
    is $e->errors->[0]{type}, 'set_item_not_hashable';
    my $strs = Set[Str];
    my $short = Perldantic::TypeAdapter->new($strs->with(max_length => 2));
    $e = dies { $short->validate_python([qw(a b c)]) };
    is $e->errors->[0]{type}, 'too_long';
    ok lives { $short->validate_python([qw(a a b b)]) }, 'the limit applies to distinct items';
    $e = dies { Perldantic::TypeAdapter->new($strs->with(min_length => 2))->validate_python([qw(a a)]) };
    is $e->errors->[0]{msg}, 'Array should have at least 2 items after validation, not 1', 'Perl wording';
};

subtest 'strict mode and FrozenSet[]' => sub {
    my $ints = Set[Int];
    my $strict = Perldantic::TypeAdapter->new($ints->with(strict => 1));
    is $strict->validate_python([1, 2, 1]), [1, 2], 'a Perl array is a set even in strict mode';
    my $frozen = Perldantic::TypeAdapter->new(FrozenSet[Str]);
    is $frozen->validate_python(['a', 'b', 'a']), ['a', 'b'];
    is $frozen->json_schema, {type => 'array', uniqueItems => !!1, items => {type => 'string'}};
    is Perldantic::Wire::encode([set(1), frozenset(2)]), '[{"$set":[1]},{"$frozenset":[2]}]';
    is Perldantic::Wire::decode('{"$frozenset":[1,2]}'), [1, 2];
};

package Test::Post {
    use Perldantic;
    use Perldantic::Types qw(Set Str);
    has tags => (is => 'ro', isa => Set[Str]);
}

package main;

subtest 'models' => sub {
    my $post = Test::Post->new(tags => [qw(perl rust perl)]);
    is $post->tags, [qw(perl rust)];
    is $post->model_dump_json, '{"tags":["perl","rust"]}';
    is Test::Post->model_json_schema->{properties}{tags},
        {type => 'array', uniqueItems => !!1, items => {type => 'string'}, title => 'Tags'};
};

done_testing;
