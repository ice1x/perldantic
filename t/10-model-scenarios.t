use v5.36;
use Test2::V0;

# An IT task manager: projects hold tasks, tasks hold subtasks and an assignee.

package Tracker::Person {
    use Perldantic;

    has login => (is => 'ro', isa => Str, required => 1, pattern => '^[a-z][a-z0-9_]*$');
    has email => (is => 'ro', isa => Maybe[Str]);
}

package Tracker::Task {
    use Perldantic;

    has title    => (is => 'ro', isa => Str, required => 1, min_length => 1);
    has kind     => (is => 'ro', isa => Enum[qw(bug feature chore)], default => 'chore');
    has estimate => (is => 'ro', isa => Num, ge => 0, default => 0);
    has assignee => (is => 'rw', isa => Maybe[InstanceOf['Tracker::Person']]);
    has subtasks => (is => 'ro', isa => ArrayRef[InstanceOf['Tracker::Task']], default => sub { [] });
    has labels   => (is => 'ro', isa => Map[Str, Int], default => sub { {} });

    sub total_estimate ($self) {
        my $total = $self->estimate;
        $total += $_->total_estimate for @{$self->subtasks};
        return $total;
    }
}

package Tracker::Project {
    use Perldantic;

    model_config extra => 'forbid';
    has key   => (is => 'ro', isa => Str, required => 1, to_upper => 1, max_length => 5);
    has tasks => (is => 'ro', isa => ArrayRef['Tracker::Task'], default => sub { [] });
}

package main;

my $ann = Tracker::Person->new(login => 'ann', email => 'ann@example.org');

my $project = Tracker::Project->new(
    key   => 'ops',
    tasks => [
        {
            title    => 'Migrate database',
            kind     => 'feature',
            estimate => '3.5',
            assignee => $ann,
            subtasks => [{title => 'Back up', estimate => 1}, {title => 'Switch over', estimate => 0.5}],
            labels   => {priority => '1'},
        },
        {title => 'Rotate logs'},
    ],
);

is $project->key, 'OPS', 'constraints transform values';
my ($migrate, $rotate) = @{$project->tasks};
isa_ok $migrate, 'Tracker::Task';
is $migrate->total_estimate, 5, 'methods work on nested objects';
is $migrate->assignee->login, 'ann';
is $migrate->labels, {priority => 1};
is $rotate->kind, 'chore';
is $rotate->assignee, undef;

$rotate->assignee(Tracker::Person->new(login => 'bob'));
is $rotate->assignee->login, 'bob', 'rw fields take new values';

my $e = dies {
    Tracker::Project->new(
        key   => 'toolong',
        owner => 'x',
        tasks => [{title => '', kind => 'epic', subtasks => [{estimate => -1}]}],
    );
};
isa_ok $e, 'Perldantic::ValidationError';
is $e->title, 'Tracker::Project';
is [map { join('.', @{$_->{loc}}) . " $_->{type}" } @{$e->errors}], [
    'key string_too_long',
    'tasks.0.title string_too_short',
    'tasks.0.kind literal_error',
    'tasks.0.subtasks.0.title missing',
    'tasks.0.subtasks.0.estimate greater_than_equal',
    'owner extra_forbidden',
], 'every error is reported with its path';
like "$e", qr/^6 validation errors for Tracker::Project\n/;

done_testing;
