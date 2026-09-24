use v5.36;
use Test2::V0;
use Scalar::Util qw(refaddr);

package Test::Point {
    use Perldantic;

    has x => (is => 'ro', isa => Int, required => 1);
    has y => (is => 'ro', isa => Int, default => 0);
}

package Test::Ticket {
    use Perldantic;

    has id     => (is => 'ro', isa => Int, required => 1, gt => 0);
    has title  => (is => 'rw', isa => Str, required => 1, min_length => 1, max_length => 20);
    has status => (is => 'ro', isa => Enum[qw(open done)], default => 'open');
    has tags   => (is => 'ro', isa => ArrayRef[Str], default => sub { [] });
    has note   => (is => 'rwp', isa => Maybe[Str], predicate => 1, clearer => 1);
    has meta   => (is => 'ro', isa => HashRef, alias => 'metadata');
    has owner  => (is => 'ro', isa => Str, init_arg => 'owned_by', default => 'nobody');
    has slug   => (is => 'lazy', isa => Str, builder => 1);

    sub _build_slug ($self) { lc($self->title) =~ s/\W+/-/gr }
}

package Test::Node {
    use Perldantic;

    has name     => (is => 'ro', isa => Str, required => 1);
    has children => (is => 'ro', isa => ArrayRef[InstanceOf['Test::Node']], default => sub { [] });
}

package Test::Segment {
    use Perldantic;

    has from => (is => 'ro', isa => 'Test::Point', required => 1);
    has to   => (is => 'ro', isa => Maybe[InstanceOf['Test::Point']]);
}

package Test::Point3D {
    use Perldantic;
    extends 'Test::Point';

    has z => (is => 'ro', isa => Int, default => 0);
    our @built;
    sub BUILD ($self, $args) { push @built, [ref $self, 'Point3D'] }
}

package Test::Strict {
    use Perldantic;

    model_config extra => 'forbid', strict => 1;
    has n => (is => 'ro', isa => Int);
}

package Test::Misc {
    use Perldantic;

    our @triggered;
    has [qw(a b)] => (is => 'rw', isa => Int, trigger => sub ($self, $v) { push @triggered, $v });
    has c => (is => 'ro', isa => Str, lazy => 1, default => sub ($self) { 'c' . ($self->a // '') });
    has d => (is => 'ro', isa => Str, builder => '_make_d');

    sub _make_d ($self) {'d'}

    sub BUILDARGS ($class, @args) {
        return @args == 1 && !ref $args[0] ? {a => $args[0]} : $class->SUPER::BUILDARGS(@args);
    }
}

package main;

subtest 'new validates and coerces like pydantic lax mode' => sub {
    my $p = Test::Point->new(x => '3');
    isa_ok $p, 'Test::Point', 'Perldantic::Model';
    is $p->x, 3;
    is $p->y, 0;
    is Test::Point->new({x => 1, y => 2})->y, 2, 'a hash reference works too';
    is {%$p}, {x => 3, y => 0}, 'objects are hashes of their fields, like Moo';
};

subtest 'invalid input raises a ValidationError' => sub {
    my $e = dies { Test::Point->new(x => 'a', y => []) };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->title, 'Test::Point';
    is [map { [$_->{type}, $_->{loc}] } @{$e->errors}], [['int_parsing', ['x']], ['int_type', ['y']]];
    $e = dies { Test::Point->new };
    is $e->errors->[0]{type}, 'missing';
    $e = dies { Test::Ticket->new(id => 0, title => '') };
    is [map { $_->{type} } @{$e->errors}], ['greater_than', 'string_too_short'],
        'has constraints reach the core';
};

subtest 'Moo options' => sub {
    my $t = Test::Ticket->new(id => 1, title => 'Fix It', metadata => {a => 1}, owned_by => 'ann');
    is $t->status, 'open';
    is $t->tags, [], 'code defaults are called';
    isnt refaddr(Test::Ticket->new(id => 1, title => 'x')->tags), refaddr($t->tags), 'for every object';
    is $t->meta, {a => 1}, 'alias names the input key';
    is $t->owner, 'ann', 'init_arg names the input key';
    ok !$t->has_note, 'predicate';
    ok !exists $t->{note}, 'optional fields left out stay absent';
    $t->_set_note('n');
    is $t->note, 'n', 'rwp writer';
    ok $t->has_note;
    $t->clear_note;
    ok !$t->has_note, 'clearer';
    ok !exists $t->{slug}, 'lazy fields wait';
    is $t->slug, 'fix-it', 'lazy builder runs on first read';
    $t->title('New');
    is $t->title, 'New', 'rw accessor';
    is $t->slug, 'fix-it', 'lazy values are kept';

    my $e = dies { $t->id(5) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'id is a read-only accessor of Test::Ticket';
};

subtest 'nested and recursive models' => sub {
    my $tree = Test::Node->new(name => 'root', children => [{name => 'a', children => [{name => 'b'}]}]);
    isa_ok $tree->children->[0], 'Test::Node';
    is $tree->children->[0]->children->[0]->name, 'b';
    my $e = dies { Test::Node->new(name => 'root', children => [{children => []}]) };
    is $e->errors->[0]{loc}, ['children', 0, 'name'];
    $e = dies { Test::Node->new(name => 'root', children => {}) };
    is $e->errors->[0]{msg}, 'Input should be an array reference', 'messages use Perl words';
    $e = dies { Test::Segment->new(from => [1]) };
    is $e->errors->[0]{msg}, 'Input should be a hash reference or an instance of Test::Point';

    my $s = Test::Segment->new(from => {x => 1}, to => Test::Point->new(x => 2, y => 3));
    isa_ok $s->from, 'Test::Point';
    is $s->to->y, 3, 'model instances are accepted';
    is Test::Segment->new(from => {x => 1})->to, undef;
};

subtest 'extends and BUILD' => sub {
    @Test::Point3D::built = ();
    my $p = Test::Point3D->new(x => 1, z => '2');
    isa_ok $p, 'Test::Point3D', 'Test::Point';
    is {%$p}, {x => 1, y => 0, z => 2}, 'parent fields come first';
    is \@Test::Point3D::built, [['Test::Point3D', 'Point3D']];
    is Test::Point->new(x => 1)->can('z'), undef, 'the parent is unchanged';
};

subtest 'model_config' => sub {
    my $e = dies { Test::Strict->new(n => '1', other => 1) };
    is [map { $_->{type} } @{$e->errors}], ['int_type', 'extra_forbidden'];
    is Test::Strict->new(n => 1)->n, 1;
};

subtest 'validate_default' => sub {
    package Test::Defaults {
        use Perldantic;
        has plain   => (is => 'ro', isa => Decimal, default => '1.5');
        has checked => (is => 'ro', isa => Decimal, default => '2.5', validate_default => 1);
        has maybe   => (is => 'ro', isa => Int);
    }
    my $d = Test::Defaults->new;
    is $d->plain, '1.5', 'a default is used as given, as in pydantic';
    isa_ok $d->checked, 'Math::BigFloat';
    is $d->checked->bstr, '2.5', 'validate_default validates it';
    ok !exists $d->{maybe}, 'fields without a default stay out';

    package Test::DefaultsBad {
        use Perldantic;
        has n => (is => 'ro', isa => Int, default => 'x', validate_default => 1);
    }
    my $e = dies { Test::DefaultsBad->new };
    is [map { [$_->{loc}, $_->{type}] } @{$e->errors}], [[['n'], 'int_parsing']], 'an invalid default is an error';
    ok lives { Test::DefaultsBad->new(n => 1) }, 'only when it is used';

    package Test::DefaultsAll {
        use Perldantic;
        model_config validate_default => 1;
        has n     => (is => 'ro', isa => Int, default => '7');
        has m     => (is => 'ro', isa => Int, default => 'x', validate_default => 0);
        has maybe => (is => 'ro', isa => Int);
    }
    my $all = Test::DefaultsAll->new;
    is $all->n, 7, 'model_config sets it for every field';
    is $all->m, 'x', 'a field can opt out';
    ok !exists $all->{maybe};

    $e = dies { package Test::DefaultsCode; use Perldantic; has n => (is => 'ro', isa => Int, default => sub { 1 }, validate_default => 1) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'has n: validate_default needs a plain default (code defaults and builders are not validated)';
};

subtest 'more Moo options' => sub {
    @Test::Misc::triggered = ();
    my $m = Test::Misc->new(7);
    is $m->a, 7, 'BUILDARGS can reshape the arguments';
    ok !exists $m->{b}, 'has [names] declares each';
    is \@Test::Misc::triggered, [7], 'triggers run for given values';
    $m->b(2);
    is \@Test::Misc::triggered, [7, 2], 'and on writes';
    is $m->c, 'c7', 'lazy code defaults get the object';
    is $m->d, 'd', 'named builders';
};

subtest 'declarations after the first object rebuild the validator' => sub {
    package Test::Late { use Perldantic; has a => (is => 'ro', isa => Int) }
    is Test::Late->new(a => 1)->a, 1;
    Test::Late::has(b => (is => 'ro', isa => Perldantic::Types::Int(), required => 1));
    my $e = dies { Test::Late->new(a => 1) };
    is $e->errors->[0]{loc}, ['b'];
};

subtest 'declaration errors are usage errors' => sub {
    my @cases = (
        [sub { package Test::Bad1; use Perldantic; has a => (is => 'ro', isa => Int, bogus => 1) },
            q{has a: unknown option 'bogus'}],
        [sub { package Test::Bad2; use Perldantic; has a => (is => 'ro', isa => Int, max_length => 1) },
            q{has a: Constraint 'max_length' does not apply to Int}],
        [sub { package Test::Bad3; use Perldantic; has a => (is => 'ro', isa => Int, default => []) },
            q{has a: a reference default must be wrapped in a code reference}],
        [sub { package Test::Bad4; use Perldantic; has a => (is => 'xx') },
            q{has a: 'is' must be ro, rw, rwp, lazy or bare, got 'xx'}],
        [sub { package Test::Bad5; use Perldantic; has a => (isa => sub {1}) },
            q{has a: isa must be a Perldantic type or a Perldantic model class}],
        [sub { package Test::Bad6; use Perldantic; has a => (coerce => 1) },
            q{has a: option 'coerce' is not supported yet}],
        [sub { package Test::Bad7; use Perldantic; extends 'Test::NotAModel' },
            q{extends: Test::NotAModel is not a Perldantic model}],
        [sub { package Test::Bad8; use Perldantic; model_config frozen => 1 },
            q{model_config: unknown setting 'frozen'}],
        [sub { Test::Point->new(1) }, 'Test::Point->new expects a hash or a hash reference'],
    );
    for my $case (@cases) {
        my $e = dies { $case->[0]->() };
        isa_ok $e, 'Perldantic::UsageError';
        is $e->message, $case->[1];
    }
};

subtest 'a model referring to an unknown class' => sub {
    package Test::Dangling { use Perldantic; has p => (is => 'ro', isa => 'Test::Missing') }
    my $e = dies { Test::Dangling->new(p => {}) };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->errors->[0]{msg}, 'Input should be an instance of Test::Missing',
        'a class that is not a model takes instances only';
};

done_testing;
