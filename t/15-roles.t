use v5.36;
use Test2::V0;
use Scalar::Util qw(refaddr);
use Perldantic::Types qw(ArrayRef);

package Test::Role::Named {
    use Perldantic::Role;

    has name => (is => 'ro', isa => Str, required => 1, min_length => 1);
    requires 'kind';

    sub label ($self) { $self->kind . ': ' . $self->name }
}

package Test::Role::Stamped {
    use Perldantic::Role;
    with 'Test::Role::Named';

    has stamp => (is => 'ro', isa => Int, default => 0);
    around label => sub ($orig, $self) { $self->$orig . ' @' . $self->stamp };
}

package Test::Doc {
    use Perldantic;
    with 'Test::Role::Stamped';

    has body => (is => 'ro', isa => Str, default => '');
    sub kind {'doc'}

    our @log;
    before model_dump => sub ($self, @) { push @log, 'dump ' . $self->name };
    after BUILD => sub ($self, @) { push @log, 'built' };
    sub BUILD { }
}

package Test::Folder {
    use Perldantic;
    has docs  => (is => 'ro', isa => ArrayRef['Test::Doc'], default => sub { [] });
    has first => (is => 'ro', isa => Maybe['Test::Doc']);
}

package Test::Always {
    use Perldantic;
    model_config revalidate_instances => 'always';
    has n => (is => 'ro', isa => Int);
}

package Test::Box {
    use Perldantic;
    has item => (is => 'ro', isa => 'Test::Always');
}

package main;

subtest 'roles add fields and methods' => sub {
    my $doc = Test::Doc->new(name => 'readme', stamp => '3');
    is $doc->name, 'readme';
    is $doc->stamp, 3, 'fields of roles are validated';
    is $doc->label, 'doc: readme @3', 'methods and modifiers of roles apply';
    ok $doc->does('Test::Role::Stamped'), 'does';
    ok $doc->does('Test::Role::Named'), 'roles composed into roles too';
    ok !$doc->does('Test::Nope');
    my $e = dies { Test::Doc->new(name => '') };
    is $e->errors->[0]{type}, 'string_too_short', 'with their constraints';
    is Test::Doc->model_json_schema->{required}, ['name'];
};

subtest 'required methods are checked' => sub {
    my $e = dies {
        package Test::NoKind;
        use Perldantic;
        with 'Test::Role::Named';
    };
    isa_ok $e, 'Perldantic::UsageError';
    like $e->message, qr/^with: Can't apply Test::Role::Named to Test::NoKind - missing kind/;
    $e = dies { package Test::NotRole; use Perldantic; with 'Test::Doc' };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'with: Test::Doc is not a role';
};

subtest 'method modifiers in models' => sub {
    @Test::Doc::log = ();
    my $doc = Test::Doc->new(name => 'x');
    $doc->model_dump;
    is \@Test::Doc::log, ['built', 'dump x'];
};

subtest 'model objects given as input are kept, not copied' => sub {
    my $doc = Test::Doc->new(name => 'kept');
    @Test::Doc::log = ();
    my $folder = Test::Folder->new(docs => [$doc, {name => 'new'}], first => $doc);
    is refaddr($folder->docs->[0]), refaddr($doc), 'the same object';
    is refaddr($folder->first), refaddr($doc);
    is \@Test::Doc::log, ['built'], 'BUILD runs only for the new object';
    isa_ok $folder->docs->[1], 'Test::Doc';
    is $folder->model_dump->{docs}[0], {name => 'kept', stamp => 0, body => ''}, 'dumps show no bookkeeping';
    my $again = Test::Folder->model_validate({docs => [$doc]});
    is refaddr($again->docs->[0]), refaddr($doc), 'model_validate keeps them too';

    require Perldantic::TypeAdapter;
    my $adapter = Perldantic::TypeAdapter->new(ArrayRef['Test::Doc']);
    is refaddr($adapter->validate([$doc])->[0]), refaddr($doc), 'and type adapters';
};

subtest 'errors show the objects that were given' => sub {
    my $doc = Test::Doc->new(name => 'd');
    my $e = dies { Test::Box->new(item => $doc) };
    isa_ok $e, 'Perldantic::ValidationError';
    is refaddr($e->errors->[0]{input}), refaddr($doc), 'the input is the object itself';
    unlike "$e", qr/__perldantic/, 'the message has no bookkeeping';
    like "$e", qr/input_value=bless\(\{name => 'd', stamp\.\.\.ody => ''\}, 'Test::Doc'\), input_type=Test::Doc\]/;
    $e = dies { Test::Folder->new(docs => [Test::Always->new(n => 1)]) };
    unlike "$e", qr/__perldantic/;
};

subtest "revalidate_instances => 'always' makes copies" => sub {
    my $item = Test::Always->new(n => 1);
    my $box = Test::Box->new(item => $item);
    isnt refaddr($box->item), refaddr($item);
    is $box->item->n, 1;
};

done_testing;
