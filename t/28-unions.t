use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
use Perldantic::Types qw(Int Str Num Undef ArrayRef Dict Enum Literal AnyOf InstanceOf Maybe);

subtest 'unions of types' => sub {
    my $id = Int | Str;
    isa_ok $id, 'Perldantic::Type';
    is $id->name, 'Int|Str';
    my $ta = Perldantic::TypeAdapter->new($id);
    is $ta->validate(5), 5;
    is $ta->validate('5'), '5', 'the exact match wins, as in pydantic smart mode';
    is $ta->validate('abc'), 'abc';
    my $e = dies { $ta->validate([]) };
    isa_ok $e, 'Perldantic::ValidationError';
    is [map { [$_->{loc}, $_->{type}] } @{$e->errors}], [[['Int'], 'int_type'], [['Str'], 'string_type']],
        'each alternative reports under its Perl name';
    is $ta->json_schema, {anyOf => [{type => 'integer'}, {type => 'string'}]};

    is ((Int | Str | Undef)->name, 'Int|Str|Undef', 'unions flatten');
    is ((Int | (Str | Num))->name, 'Int|Str|Num');
    is Perldantic::TypeAdapter->new(ArrayRef[Int | Str])->validate([1, 'a']), [1, 'a'], 'as parameters too';
};

package Test::Cat { use Perldantic; has kind => (is => 'ro', isa => Literal['cat']); has lives => (is => 'ro', isa => Int, required => 1) }
package Test::Dog { use Perldantic; has kind => (is => 'ro', isa => Enum[qw(dog puppy)]); has name => (is => 'ro', isa => Str, required => 1) }
package Test::Fish { use Perldantic; has fins => (is => 'ro', isa => Int) }

package main;

subtest 'model classes' => sub {
    my $pet = AnyOf['Test::Cat', 'Test::Dog'];
    is $pet->name, 'Test::Cat|Test::Dog';
    is ((InstanceOf['Test::Cat'] | 'Test::Dog')->name, 'Test::Cat|Test::Dog', 'a class name next to a type');
    my $ta = Perldantic::TypeAdapter->new($pet);
    isa_ok $ta->validate({kind => 'dog', name => 'Rex'}), 'Test::Dog';
    my $e = dies { $ta->validate({kind => 'dog'}) };
    is [sort map { join '.', @{$_->{loc}} } @{$e->errors}], ['Test::Cat.kind', 'Test::Cat.lives', 'Test::Dog.name'];
};

subtest 'tagged unions' => sub {
    my $pet = (AnyOf['Test::Cat', 'Test::Dog'])->with(discriminator => 'kind');
    my $ta = Perldantic::TypeAdapter->new(ArrayRef[$pet]);
    my $pets = $ta->validate([{kind => 'cat', lives => 9}, {kind => 'puppy', name => 'Bo'}]);
    isa_ok $pets->[0], 'Test::Cat';
    isa_ok $pets->[1], 'Test::Dog';

    my $e = dies { $ta->validate([{kind => 'dog'}]) };
    is [map { [$_->{loc}, $_->{type}] } @{$e->errors}], [[[0, 'dog', 'name'], 'missing']],
        'only the tagged alternative is tried, and located by its tag';
    $e = dies { $ta->validate([{kind => 'bird'}, {}]) };
    is [map { $_->{type} } @{$e->errors}], ['union_tag_invalid', 'union_tag_not_found'];
    is $e->errors->[0]{msg},
        "Input tag 'bird' found using 'kind' does not match any of the expected tags: 'cat', 'dog', 'puppy'";

    my $schema = Perldantic::TypeAdapter->new($pet)->json_schema;
    is $schema->{discriminator}, {propertyName => 'kind', mapping => {
        cat => '#/$defs/Cat', dog => '#/$defs/Dog', puppy => '#/$defs/Dog'}};

    my $shapes = (Dict[kind => Literal['circle'], r => Num] | Dict[kind => Literal['square'], side => Num])
        ->with(discriminator => 'kind');
    is Perldantic::TypeAdapter->new($shapes)->validate({kind => 'square', side => '2'}), {kind => 'square', side => 2},
        'Dict[] alternatives';
};

subtest 'models inside Dict[]' => sub {
    my $owned = Perldantic::TypeAdapter->new(Dict[owner => Str, pet => 'Test::Cat']);
    isa_ok $owned->validate({owner => 'ada', pet => {kind => 'cat', lives => 3}})->{pet}, 'Test::Cat';
};

subtest 'mistakes' => sub {
    for (
        [sub { Int | 'not a type' }, 'An alternative of a union must be a type or a class name, got not a type'],
        [sub { (Int | Str)->with(discriminator => 'kind')->core_schema },
            "discriminator 'kind': Int has no 'kind' field with Enum[] or Literal[] values"],
        [sub { (AnyOf['Test::Cat', 'Test::Fish'])->with(discriminator => 'kind')->core_schema },
            "discriminator 'kind': Test::Fish has no 'kind' field with Enum[] or Literal[] values"],
        [sub { (AnyOf['Test::Cat', 'Test::Cat'])->with(discriminator => 'kind')->core_schema },
            "discriminator 'kind': tag 'cat' is used by Test::Cat and Test::Cat"],
        [sub { AnyOf['Test::Cat'] }, 'AnyOf[] takes at least 2 types'],
    ) {
        my ($code, $message) = @$_;
        my $e = dies { $code->() };
        isa_ok $e, 'Perldantic::UsageError';
        is $e->message, $message;
    }
};

done_testing;
