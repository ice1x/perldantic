use v5.36;
use Test2::V0;

use Perldantic::FFI;

# Count compilations by wrapping the FFI constructors.
my %compiled;
for my $kind (qw(Validator Serializer)) {
    my $class = "Perldantic::FFI::$kind";
    my $new   = $class->can('new');
    no strict 'refs';
    no warnings 'redefine';
    *{"${class}::new"} = sub ($c, $schema, @rest) {
        my $name = $schema->{schema}{schema_ref} // $schema->{type};
        $compiled{"$kind $name"}++;
        return $new->($c, $schema, @rest);
    };
}

package Cache::Leaf { use Perldantic; has v => (is => 'ro', isa => Int) }
package Cache::Tree { use Perldantic; has leaf => (is => 'ro', isa => 'Cache::Leaf') }
package Cache::Other { use Perldantic; has w => (is => 'ro', isa => Int) }
package Cache::Child { use Perldantic; extends 'Cache::Leaf'; has c => (is => 'ro', isa => Int) }

package main;

subtest 'each class compiles once, on first use' => sub {
    %compiled = ();
    is \%compiled, {}, 'nothing is compiled at declaration time';
    Cache::Leaf->new(v => 1) for 1 .. 3;
    Cache::Leaf->model_validate({v => 2});
    is \%compiled, {'Validator Cache.Leaf' => 1};
    my $leaf = Cache::Leaf->new(v => 1);
    $leaf->model_dump for 1 .. 2;
    $leaf->model_dump_json;
    is \%compiled, {'Validator Cache.Leaf' => 1, 'Serializer Cache.Leaf' => 1};
};

subtest 'a declaration drops only the classes that depend on it' => sub {
    "Cache::$_"->new for qw(Tree Other Child Leaf);
    %compiled = ();
    Cache::Leaf::has(extra => (is => 'ro', isa => Perldantic::Types::Str()));
    "Cache::$_"->new for qw(Tree Other Child Leaf);
    is \%compiled, {
        'Validator Cache.Leaf'  => 1,
        'Validator Cache.Tree'  => 1,
        'Validator Cache.Child' => 1,
    }, 'users, subclasses and the class itself recompile; others do not';
};

done_testing;
