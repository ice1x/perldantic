use v5.36;
use Test2::V0;

# An IT task manager: a sprint board holds bugs, features and chores, told apart by `kind`
# (a tagged union), each with a deadline and an estimate. Boards are imported from another
# tracker's JSON API and exported back.

package Tasks::Task {
    use Perldantic;

    model_config extra => 'forbid';

    has id         => (is => 'ro', isa => Int, required => 1, gt => 0);
    has title      => (is => 'ro', isa => Str, required => 1, min_length => 3, max_length => 200);
    has created_at => (is => 'ro', isa => DateTime, required => 1);
    has deadline   => (is => 'ro', isa => Maybe[Date]);
    has estimate   => (is => 'ro', isa => Maybe[Duration]);
    has assignee   => (is => 'rw', isa => Maybe[Str], pattern => '^@[a-z][a-z0-9_]*$');

    # a deadline cannot come before the task was created
    field_validator deadline => sub ($class, $deadline, $info) {
        my $created = $info->data->{created_at} // return $deadline;
        die "the deadline is before the task was created\n"
            if defined $deadline && $deadline->iso lt $created->date->iso;
        return $deadline;
    };
}

package Tasks::Bug {
    use Perldantic;
    extends 'Tasks::Task';

    has kind     => (is => 'ro', isa => Literal['bug'], required => 1);
    has severity => (is => 'ro', isa => Enum[qw(low medium high critical)], required => 1);
    has steps    => (is => 'ro', isa => ArrayRef[Str], required => 1, min_length => 1);
    has version  => (is => 'ro', isa => Str, pattern => '^\d+\.\d+\.\d+$');

    # critical bugs are always scheduled
    model_validator mode => 'after', sub ($self) {
        die "a critical bug needs a deadline\n" if $self->severity eq 'critical' && !defined $self->deadline;
        return $self;
    };
}

package Tasks::Feature {
    use Perldantic;
    extends 'Tasks::Task';

    has kind         => (is => 'ro', isa => Literal['feature'], required => 1);
    has story_points => (is => 'ro', isa => Int, required => 1);
    has epic         => (is => 'ro', isa => Maybe[Str]);

    field_validator story_points => sub ($class, $points) {
        Perldantic::CustomError->throw(type => 'not_fibonacci', message => '{points} is not on the planning scale',
            context => {points => $points}) if !grep { $_ == $points } 1, 2, 3, 5, 8, 13, 21;
        return $points;
    };
}

package Tasks::Chore {
    use Perldantic;
    extends 'Tasks::Task';

    has kind      => (is => 'ro', isa => Literal['chore'], required => 1);
    has recurring => (is => 'ro', isa => Bool, default => 0);
}

package Tasks::Board {
    use Perldantic;

    has sprint => (is => 'ro', isa => Str, required => 1);
    has ends   => (is => 'ro', isa => Date, required => 1);
    has tasks  => (is => 'ro', isa => ArrayRef[(AnyOf['Tasks::Bug', 'Tasks::Feature', 'Tasks::Chore'])
        ->with(discriminator => 'kind')], default => sub { [] });

    # tasks due after the sprint ends, by id
    sub late ($self) {
        return [map { $_->id } grep { defined $_->deadline && $_->deadline->iso gt $self->ends->iso } @{$self->tasks}];
    }

    sub by_deadline ($self) {
        return [map { $_->id } sort {
            ($a->deadline // $self->ends)->iso cmp ($b->deadline // $self->ends)->iso || $a->id <=> $b->id
        } @{$self->tasks}];
    }
}

package main;

# What the other tracker's API returns.
my $export = <<'JSON';
{
  "sprint": "2026-S19",
  "ends": "2026-10-02",
  "tasks": [
    {"kind": "bug", "id": 101, "title": "Login fails with SSO", "created_at": "2026-09-18T09:12:00Z",
     "deadline": "2026-09-26", "severity": "critical", "steps": ["open /login", "choose SSO"],
     "version": "2.4.1", "assignee": "@ada", "estimate": "PT6H"},
    {"kind": "feature", "id": 102, "title": "Export to CSV", "created_at": "2026-09-19T14:00:00Z",
     "story_points": 5, "epic": "reporting", "deadline": "2026-10-05"},
    {"kind": "chore", "id": 103, "title": "Rotate API keys", "created_at": "2026-09-20T08:00:00Z",
     "recurring": true, "estimate": "PT30M"}
  ]
}
JSON

subtest 'importing a board' => sub {
    my $board = Tasks::Board->model_validate_json($export);
    is [map { ref } @{$board->tasks}], ['Tasks::Bug', 'Tasks::Feature', 'Tasks::Chore'], 'each task by its kind';
    my ($bug, $feature, $chore) = @{$board->tasks};
    isa_ok $bug, 'Tasks::Task';
    is $bug->severity, 'critical';
    is $bug->estimate->total_seconds, 6 * 3600;
    is $bug->created_at->tz_offset, 0;
    ok $chore->recurring;
    is $chore->estimate->total_seconds, 1800;
    is $board->late, [102], 'the feature is due after the sprint';
    is $board->by_deadline, [101, 103, 102];
};

subtest 'bad tasks, located by kind' => sub {
    my $e = dies {
        Tasks::Board->new(sprint => 'S', ends => '2026-10-02', tasks => [
            {kind => 'bug', id => 0, title => 'x', created_at => '2026-09-18T09:00:00Z', severity => 'urgent', steps => []},
            {kind => 'feature', id => 7, title => 'Dark mode', created_at => '2026-09-18T09:00:00Z', story_points => 4},
            {kind => 'epic', id => 8, title => 'Big'},
        ])
    };
    is [map { join('.', @{$_->{loc}}) . ":$_->{type}" } @{$e->errors}], [
        'tasks.0.bug.id:greater_than',
        'tasks.0.bug.title:string_too_short',
        'tasks.0.bug.severity:literal_error',
        'tasks.0.bug.steps:too_short',
        'tasks.1.feature.story_points:not_fibonacci',
        'tasks.2:union_tag_invalid',
    ];
    is $e->errors->[4]{msg}, '4 is not on the planning scale';
};

subtest 'rules across fields' => sub {
    my %bug = (kind => 'bug', id => 1, title => 'Crash on save', created_at => '2026-09-20T10:00:00+02:00',
        severity => 'critical', steps => ['save']);
    my $e = dies { Tasks::Bug->new(%bug) };
    is $e->errors->[0]{msg}, 'Value error, a critical bug needs a deadline';
    $e = dies { Tasks::Bug->new(%bug, deadline => '2026-09-01') };
    is $e->errors->[0]{loc}, ['deadline'];
    is $e->errors->[0]{msg}, 'Value error, the deadline is before the task was created';
    ok lives { Tasks::Bug->new(%bug, deadline => '2026-09-20') }, 'the same day is fine';
    $e = dies { Tasks::Bug->new(%bug, deadline => '2026-09-21', points => 3) };
    is $e->errors->[0]{type}, 'extra_forbidden', 'the kinds keep the base class settings';
};

subtest 'reassigning and exporting' => sub {
    my $board = Tasks::Board->model_validate_json($export);
    $board->tasks->[1]->assignee('@lin');
    my $json = $board->model_dump_json(exclude_none => 1);
    like $json, qr/"assignee":"\@lin"/;
    like $json, qr/"estimate":"PT6H"/, 'durations as ISO 8601';
    my $again = Tasks::Board->model_validate_json($json);
    is $again->model_dump, $board->model_dump, 'the export imports back';
};

subtest 'schema for API clients' => sub {
    my $schema = Tasks::Board->model_json_schema;
    my $items = $schema->{properties}{tasks}{items};
    is $items->{discriminator}{propertyName}, 'kind';
    is $items->{discriminator}{mapping}, {bug => '#/$defs/Bug', feature => '#/$defs/Feature', chore => '#/$defs/Chore'};
    is [map { $_->{'$ref'} } @{$items->{oneOf}}], ['#/$defs/Bug', '#/$defs/Feature', '#/$defs/Chore'];
    is $schema->{'$defs'}{Bug}{properties}{deadline}{anyOf}, [{type => 'string', format => 'date'}, {type => 'null'}];
};

done_testing;
