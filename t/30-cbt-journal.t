use v5.36;
use Test2::V0;

# A CBT (cognitive behavioural therapy) journal: thought records rate emotions before and after
# working through a situation, and name the thinking traps that showed up. Entries come from a
# web form (everything is a string), are stored as JSON and shown back to the user.

package CBT::Emotion {
    use Perldantic;

    has name      => (is => 'ro', isa => Enum[qw(anxiety sadness anger shame guilt fear joy)], required => 1);
    has intensity => (is => 'ro', isa => Int, required => 1, ge => 0, le => 100);
}

package CBT::Entry {
    use Perldantic;

    model_config extra => 'forbid', str_strip_whitespace => 1;

    has id                => (is => 'ro', isa => Uuid, required => 1);
    has recorded_at       => (is => 'ro', isa => DateTime, required => 1);
    has situation         => (is => 'ro', isa => Str, required => 1, min_length => 1);
    has automatic_thought => (is => 'ro', isa => Str, required => 1, min_length => 1);
    has emotions_before   => (is => 'ro', isa => ArrayRef['CBT::Emotion'], required => 1, min_length => 1);
    has distortions       => (is => 'ro', isa => Set[Enum[qw(catastrophizing mind_reading all_or_nothing
        overgeneralization should_statements labeling)]], default => sub { [] });
    has balanced_thought  => (is => 'ro', isa => Maybe[Str]);
    has emotions_after    => (is => 'ro', isa => ArrayRef['CBT::Emotion'], default => sub { [] });

    # re-rating only makes sense for the emotions that were rated, once a balanced thought exists
    model_validator mode => 'after', sub ($self) {
        my %before = map { $_->name => 1 } @{$self->emotions_before};
        for my $after (@{$self->emotions_after}) {
            die "re-rate only the emotions rated before, not '@{[$after->name]}'\n" if !$before{$after->name};
        }
        die "a balanced thought comes with the emotions re-rated\n"
            if defined $self->balanced_thought && !@{$self->emotions_after};
        return $self;
    };

    # how many points the re-rated emotions dropped, on average
    computed_field relief => (isa => Maybe[Num]) => sub ($self) {
        my %before = map { $_->name => $_->intensity } @{$self->emotions_before};
        my @drops = map { $before{$_->name} - $_->intensity } @{$self->emotions_after} or return undef;
        my $total = 0;
        $total += $_ for @drops;
        return $total / @drops;
    };
}

package CBT::Journal {
    use Perldantic;

    has owner   => (is => 'ro', isa => Str, required => 1);
    has entries => (is => 'ro', isa => ArrayRef['CBT::Entry'], default => sub { [] });

    # the thinking traps named most often, most frequent first
    sub common_distortions ($self) {
        my %count;
        $count{$_}++ for map { @{$_->distortions} } @{$self->entries};
        return [sort { $count{$b} <=> $count{$a} || $a cmp $b } keys %count];
    }
}

package main;

# What the web form posts: every value is a string.
my %form = (
    id                => '9f1c2b7e-4d3a-4b6f-8e21-0c5d7a9b1e34',
    recorded_at       => '2026-09-21T21:40:00+03:00',
    situation         => '  Presentation at work went quiet after my demo  ',
    automatic_thought => 'Everyone thinks I am incompetent',
    emotions_before   => [{name => 'anxiety', intensity => '80'}, {name => 'shame', intensity => '60'}],
    distortions       => ['mind_reading', 'labeling', 'mind_reading'],
);

subtest 'a thought record from the form' => sub {
    my $entry = CBT::Entry->new(%form);
    isa_ok $entry->id, 'Perldantic::Uuid';
    is $entry->id->version, 4;
    isa_ok $entry->recorded_at, 'Perldantic::DateTime';
    is $entry->recorded_at->tz_offset, 3 * 3600;
    is $entry->situation, 'Presentation at work went quiet after my demo', 'whitespace is stripped';
    is [map { [$_->name, $_->intensity] } @{$entry->emotions_before}], [['anxiety', 80], ['shame', 60]];
    is [sort @{$entry->distortions}], ['labeling', 'mind_reading'], 'a set: each distortion once';
    is $entry->relief, undef, 'nothing re-rated yet';
};

subtest 'what the form gets wrong' => sub {
    my $e = dies {
        CBT::Entry->new(%form, emotions_before => [{name => 'anxiety', intensity => '150'}, {name => 'boredom', intensity => 5}],
            distortions => ['fortune_telling'], mood => 'bad')
    };
    isa_ok $e, 'Perldantic::ValidationError';
    is $e->title, 'CBT::Entry';
    is [sort map { join('.', @{$_->{loc}}) . ":$_->{type}" } @{$e->errors}], [
        'distortions.0:literal_error',
        'emotions_before.0.intensity:less_than_equal',
        'emotions_before.1.name:literal_error',
        'mood:extra_forbidden',
    ], 'every mistake at once, where it is';

    $e = dies { CBT::Entry->new(%form, situation => '   ', emotions_before => []) };
    is [map { $_->{type} } @{$e->errors}], ['string_too_short', 'too_short'];
};

subtest 're-rating after a balanced thought' => sub {
    my $e = dies { CBT::Entry->new(%form, balanced_thought => 'One quiet room is not a verdict') };
    is $e->errors->[0]{msg}, 'Value error, a balanced thought comes with the emotions re-rated';
    $e = dies { CBT::Entry->new(%form, balanced_thought => 'x', emotions_after => [{name => 'joy', intensity => 10}]) };
    is $e->errors->[0]{msg}, "Value error, re-rate only the emotions rated before, not 'joy'";

    my $entry = CBT::Entry->new(%form,
        balanced_thought => 'One quiet room is not a verdict on my work',
        emotions_after   => [{name => 'anxiety', intensity => 40}, {name => 'shame', intensity => 30}]);
    is $entry->relief, 35, 'anxiety dropped 40 points, shame 30';
};

subtest 'stored as JSON and read back' => sub {
    my $entry = CBT::Entry->new(%form, balanced_thought => 'Quiet is not judgement',
        emotions_after => [{name => 'anxiety', intensity => 50}]);
    my $json = $entry->model_dump_json;
    like $json, qr/"recorded_at":"2026-09-21T21:40:00\+03:00"/;
    like $json, qr/"relief":30\.0\}\z/, 'the computed field is stored too';

    # the computed field is output only: reading back drops it (extra => forbid would refuse
    # it otherwise), so it is left out of the stored document
    my $stored = $entry->model_dump_json(exclude => ['relief']);
    my $back   = CBT::Entry->model_validate_json($stored);
    is $back->model_dump_json, $json, 'the same entry';
    is $back->relief, 30;
};

subtest 'the journal' => sub {
    my $journal = CBT::Journal->new(owner => 'ada', entries => [
        \%form,
        {%form, id => 'c6b1e0a4-7f2d-4e8a-9b3c-5d1f2a4e6c80', distortions => ['catastrophizing', 'labeling']},
    ]);
    is $journal->common_distortions, ['labeling', 'catastrophizing', 'mind_reading'];
    is [map { $_->{situation} } @{$journal->model_dump(include => {entries => {'__all__' => {situation => 1}}})->{entries}}],
        ['Presentation at work went quiet after my demo', 'Presentation at work went quiet after my demo'],
        'a summary with one field of every entry';

    my $schema = CBT::Journal->model_json_schema;
    is [sort keys %{$schema->{'$defs'}}], ['Emotion', 'Entry'], 'named like pydantic, without the package';
    my $emotion = $schema->{'$defs'}{Emotion}{properties};
    is $emotion->{intensity}, {type => 'integer', minimum => 0, maximum => 100, title => 'Intensity'};
    is $schema->{'$defs'}{Entry}{additionalProperties}, F(), 'forms know unknown fields are refused';
};

done_testing;
