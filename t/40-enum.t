use v5.36;
use Test2::V0;

use Cpanel::JSON::XS ();
use Scalar::Util qw(refaddr);

use Perldantic::FFI;
use Perldantic::TypeAdapter;
use Perldantic::Types qw(ArrayRef Maybe);

# Perl enum classes: members declared in a package, validated by the core `enum` schema.

package En::Color {
    use Perldantic::Enum RED => 'red', GREEN => 'green', BLUE => 'blue';
}

package En::Level {
    use Perldantic::Enum LOW => 1, MID => 5, HIGH => 10;
}

package En::Ratio {
    use Perldantic::Enum HALF => 0.5, THIRD => 0.25;
}

package En::State {
    use Perldantic::Enum [qw(open in_progress done)];
}

package En::Mixed {
    use Perldantic::Enum ONE => 1, TWO => 'two';
}

package En::Shirt {
    use Perldantic;
    has color => (is => 'ro', isa => 'En::Color', required => 1);
    has level => (is => 'ro', isa => 'En::Level', default => En::Level->LOW);
    has size  => (is => 'ro', isa => 'En::State', default => sub { En::State->open });
    has alt   => (is => 'ro', isa => Maybe ['En::Color']);
}

package main;

subtest 'members' => sub {
    my $red = En::Color->RED;
    isa_ok $red, ['En::Color', 'Perldantic::Enum'];
    is $red->name,  'RED';
    is $red->value, 'red';
    is refaddr(En::Color->RED), refaddr($red), 'one object per member';
    is [map { $_->name } En::Color->members], [qw(RED GREEN BLUE)], 'in declaration order';
    is "$red", 'red', 'a member reads as its value';
    ok $red eq 'red';
    ok(En::Level->MID == 5, 'numbers too');
    ok !!En::Level->LOW, 'members are true';

    is refaddr(En::Color->from_value('green')), refaddr(En::Color->GREEN), 'from_value';
    is refaddr(En::Color->from_name('BLUE')), refaddr(En::Color->BLUE), 'from_name';
    is En::Color->from_value('pink'), undef, 'no such value';
    is En::Color->from_name('PINK'), undef, 'no such name';

    is [map { [$_->name, $_->value] } En::State->members], [[open => 'open'], [in_progress => 'in_progress'], [done => 'done']],
        'a list of names: each is its own value';
    ok dies { $red->{value} = 'x' }, 'members cannot be changed';
};

subtest 'declaring' => sub {
    my $e = dies { package En::Bad1; Perldantic::Enum->import() };
    isa_ok $e, ['Perldantic::UsageError'];
    like $e->message, qr/at least one member/;

    $e = dies { package En::Bad2; Perldantic::Enum->import(A => 1, B => 1) };
    like $e->message, qr/B and A have the same value/;

    $e = dies { package En::Bad3; Perldantic::Enum->import(A => 1, A => 2) };
    like $e->message, qr/member A is declared twice/;

    $e = dies { package En::Bad4; Perldantic::Enum->import(A => undef) };
    like $e->message, qr/member A: the value must be a string or a number/;

    $e = dies { package En::Bad5; Perldantic::Enum->import('bad name' => 1) };
    like $e->message, qr/'bad name' is not a valid member name/;

    $e = dies { package En::Bad6; Perldantic::Enum->import(A => 1, 'B') };
    like $e->message, qr/names and values in pairs/;
};

subtest 'fields of models' => sub {
    my $shirt = En::Shirt->new(color => 'green');
    is refaddr($shirt->color), refaddr(En::Color->GREEN), 'the value becomes the member';
    is refaddr($shirt->level), refaddr(En::Level->LOW), 'a member as the default';
    is refaddr($shirt->size), refaddr(En::State->open), 'a code default';
    is [$shirt->model_fields_set], ['color'];
    is $shirt->model_dump(exclude_defaults => 1, mode => 'json'), {color => 'green', size => 'open'},
        'the member default is a default';

    my $given = En::Shirt->new(color => En::Color->BLUE, level => En::Level->HIGH);
    is refaddr($given->color), refaddr(En::Color->BLUE), 'members are kept';
    is refaddr($given->level), refaddr(En::Level->HIGH);

    is refaddr(En::Shirt->model_validate_json('{"color": "red", "level": 5}')->level), refaddr(En::Level->MID), 'JSON';
    is refaddr(En::Shirt->new(color => 'red', alt => 'blue')->alt), refaddr(En::Color->BLUE), 'Maybe[]';

    my $e = dies { En::Shirt->new(color => 'pink') };
    isa_ok $e, ['Perldantic::ValidationError'];
    is $e->errors->[0]{type}, 'enum';
    is $e->errors->[0]{loc}, ['color'];
    is $e->errors->[0]{msg}, "Input should be 'red', 'green' or 'blue'";

    $e = dies { En::Shirt->new(color => En::Level->LOW) };
    is $e->errors->[0]{type}, 'enum', 'a member of another enum';
};

subtest 'numbers' => sub {
    my $level = Perldantic::TypeAdapter->new('En::Level');
    is refaddr($level->validate('5')), refaddr(En::Level->MID), 'integer values take numeric strings';
    is refaddr($level->validate(10)), refaddr(En::Level->HIGH);
    ok dies { $level->validate(7) };
    is refaddr(Perldantic::TypeAdapter->new('En::Ratio')->validate('0.25')), refaddr(En::Ratio->THIRD), 'floats';
    my $mixed = Perldantic::TypeAdapter->new('En::Mixed');
    is refaddr($mixed->validate(1)), refaddr(En::Mixed->ONE), 'values of mixed kinds';
    is refaddr($mixed->validate('two')), refaddr(En::Mixed->TWO);
};

subtest 'strict' => sub {
    my $strict = Perldantic::TypeAdapter->new('En::Color', config => {strict => 1});
    is refaddr($strict->validate(En::Color->RED)), refaddr(En::Color->RED), 'members';
    my $e = dies { $strict->validate('red') };
    is $e->errors->[0]{type}, 'is_instance_of', 'not values';
    is refaddr($strict->validate_json('"red"')), refaddr(En::Color->RED), 'JSON has only values';
};

subtest 'dumping' => sub {
    my $shirt = En::Shirt->new(color => 'green', alt => 'red');
    my $dump  = $shirt->model_dump;
    is refaddr($dump->{color}), refaddr(En::Color->GREEN), 'dump keeps members';
    is $shirt->model_dump(mode => 'json'), {color => 'green', level => 1, alt => 'red', size => 'open'}, 'mode => json gives values';
    is Cpanel::JSON::XS::decode_json($shirt->model_dump_json), {color => 'green', level => 1, alt => 'red', size => 'open'};
    my $adapter = Perldantic::TypeAdapter->new(ArrayRef ['En::State']);
    is $adapter->dump_json([En::State->done, En::State->open]), '["done","open"]', 'an adapter';
};

subtest 'JSON Schema' => sub {
    is(Perldantic::TypeAdapter->new('En::Color')->json_schema,
        {enum => [qw(red green blue)], title => 'En::Color', type => 'string'});
    my $schema = En::Shirt->model_json_schema;
    is $schema->{properties}{color}, {'$ref' => '#/$defs/Color'};
    is $schema->{'$defs'}{Level}, {enum => [1, 5, 10], title => 'En::Level', type => 'integer'};
    is $schema->{properties}{level}, {'$ref' => '#/$defs/Level', default => 1}, 'a member default is its value';
};

subtest 'check' => sub {
    ok(En::Shirt->model_check({color => 'red'}));
    ok(!En::Shirt->model_check({color => 'pink'}));
};

subtest 'lazy objects' => sub {
    package En::Tag { use Perldantic; has color => (is => 'ro', isa => 'En::Color', default => En::Color->RED) }
    package main;
    my $tag = En::Tag->model_validate({}, lazy => 1);
    is $tag->model_dump_json, '{"color":"red"}', 'dumped as the core holds it';
    ok !%$tag, 'without being read';
    is refaddr($tag->color), refaddr(En::Color->RED);

    my $shirt = En::Shirt->model_validate({color => 'blue'}, lazy => 1);
    is refaddr($shirt->color), refaddr(En::Color->BLUE);
    is $shirt->model_dump_json, '{"color":"blue","level":1,"size":"open"}';
};

subtest 'members of classes Perl does not know' => sub {
    my $validator = Perldantic::FFI::Validator->new({type => 'enum', cls => 'En::Elsewhere', sub_type => 'int',
        members => [Perldantic::Wire::Enum->new(class => 'En::Elsewhere', name => 'A', value => 1, mixin => 'int')]});
    my $member = $validator->validate('1');
    isa_ok $member, ['Perldantic::Wire::Enum'];
    is [$member->class, $member->name, $member->value, $member->mixin], ['En::Elsewhere', 'A', 1, 'int'];
};

done_testing;
