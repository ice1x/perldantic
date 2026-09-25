use v5.36;
use Test2::V0;

use Perldantic::TypeAdapter;
use Perldantic::Types qw(ArrayRef);

# A bug tracker: issues have UUIDs and links, are imported from another tracker's JSON export
# (with its own field names), updated by PATCH requests that name only what changes, and
# exported back. Unknown fields are refused, so typos in API clients are caught.

# Severities are an enum class: issues hold the members, sorted by rank.
package Tracker::Severity {
    use Perldantic::Enum TRIVIAL => 1, MINOR => 2, MAJOR => 3, CRITICAL => 4;
}

package Tracker::Comment {
    use Perldantic;

    model_config extra => 'forbid';

    has author => (is => 'ro', isa => Str, required => 1, pattern => '^[a-z][a-z0-9-]*$');
    has body   => (is => 'ro', isa => Str, required => 1, min_length => 1, strip_whitespace => 1);
    has at     => (is => 'ro', isa => DateTime, required => 1);
}

package Tracker::Issue {
    use Perldantic;

    model_config extra => 'forbid', validate_by_name => 1, str_strip_whitespace => 1;

    has id          => (is => 'ro', isa => Uuid, required => 1, version => 4);
    has title       => (is => 'ro', isa => Str, required => 1, min_length => 5, max_length => 120);
    has status      => (is => 'ro', isa => Enum[qw(open triaged fixed wontfix)], default => 'open');
    has severity    => (is => 'ro', isa => 'Tracker::Severity', default => Tracker::Severity->MINOR);
    has labels      => (is => 'ro', isa => Set[Str], default => sub { [] }, max_length => 10);
    has link        => (is => 'ro', isa => Url, required => 1, alias => 'html_url',
        allowed_schemes => ['https']);
    has attachments => (is => 'ro', isa => ArrayRef[Url], default => sub { [] });
    has duplicate_of => (is => 'ro', isa => Maybe[Uuid]);
    has comments    => (is => 'ro', isa => ArrayRef['Tracker::Comment'], default => sub { [] });

    # a duplicate is closed as won't fix, and never of itself
    model_validator mode => 'after', sub ($self) {
        my $of = $self->duplicate_of // return $self;
        die "an issue cannot duplicate itself\n" if $of eq $self->id;
        die "a duplicate is closed as wontfix\n" if $self->status ne 'wontfix';
        return $self;
    };

    # a PATCH: validated as a whole issue, so every rule still holds
    sub patch ($self, %changes) {
        return ref($self)->new(%{$self->model_dump(exclude_unset => 1)}, %changes);
    }
}

package main;

my $export = <<'JSON';
[
  {"id": "3f0b9a52-8c1e-4c7d-9b6a-2e4f5d1c0a97", "title": "Crash when saving an empty note  ",
   "html_url": "https://tracker.example.com/issues/3f0b9a52", "labels": ["crash", "editor", "crash"], "severity": 4,
   "attachments": ["https://files.example.com/a/trace.txt", "ftp://files.example.com/core"],
   "comments": [{"author": "ada", "body": " Reproduced on 2.4.1 ", "at": "2026-09-20T10:15:00Z"}]},
  {"id": "b1d7c1e2-5a43-4f0e-8c9b-7a6d5e4f3c21", "title": "Save crashes with no text",
   "html_url": "https://tracker.example.com/issues/b1d7c1e2", "status": "wontfix",
   "duplicate_of": "3f0b9a52-8c1e-4c7d-9b6a-2e4f5d1c0a97"}
]
JSON

my $issues = Perldantic::TypeAdapter->new(ArrayRef['Tracker::Issue']);

subtest 'importing issues' => sub {
    my ($crash, $dup) = @{$issues->validate_json($export)};
    isa_ok $crash->id, 'Perldantic::Uuid';
    is $crash->id->version, 4;
    is $crash->title, 'Crash when saving an empty note', 'whitespace is stripped';
    is [sort @{$crash->labels}], ['crash', 'editor'], 'labels are a set';
    isa_ok $crash->link, 'Perldantic::Url';
    is $crash->link->host, 'tracker.example.com';
    is [map { $_->scheme } @{$crash->attachments}], ['https', 'ftp'], 'attachments may be any URL';
    is $crash->comments->[0]->body, 'Reproduced on 2.4.1';
    is $crash->status, 'open';
    ok $dup->duplicate_of eq $crash->id, 'UUIDs compare by value';
};

subtest 'severities' => sub {
    my ($crash, $dup) = @{$issues->validate_json($export)};
    ok $crash->severity == Tracker::Severity->CRITICAL, 'read from the export';
    ok $dup->severity == Tracker::Severity->MINOR, 'the default';
    my @triage = sort { $b->severity <=> $a->severity } $dup, $crash;
    is [map { $_->severity->name } @triage], [qw(CRITICAL MINOR)], 'members compare by rank';
    is $crash->patch(severity => '3')->severity->name, 'MAJOR', 'a form sends the rank as text';
    my $e = dies { $crash->patch(severity => 9) };
    is $e->errors->[0]{msg}, 'Input should be 1, 2, 3 or 4';
};

subtest 'what the importer refuses' => sub {
    my $e = dies {
        Tracker::Issue->new(id => '3f0b9a52-8c1e-1c7d-9b6a-2e4f5d1c0a97', title => 'Bug', html_url => 'http://tracker/1',
            priority => 'high', comments => [{author => 'Ada', body => '', at => 'yesterday', votes => 3}])
    };
    is [sort map { join('.', @{$_->{loc}}) . ":$_->{type}" } @{$e->errors}], [
        'comments.0.at:datetime_from_date_parsing',
        'comments.0.author:string_pattern_mismatch',
        'comments.0.body:string_too_short',
        'comments.0.votes:extra_forbidden',
        'html_url:url_scheme',
        'id:uuid_version',
        'priority:extra_forbidden',
        'title:string_too_short',
    ];
    my ($scheme) = grep { $_->{type} eq 'url_scheme' } @{$e->errors};
    is $scheme->{msg}, "URL scheme should be 'https'";

    $e = dies { Tracker::Issue->new(id => '3f0b9a52-8c1e-4c7d-9b6a-2e4f5d1c0a97', title => 'Self dup',
        link => 'https://t/1', duplicate_of => '3F0B9A52-8C1E-4C7D-9B6A-2E4F5D1C0A97', status => 'wontfix') };
    is $e->errors->[0]{msg}, 'Value error, an issue cannot duplicate itself', 'by name as well as by alias';
};

subtest 'patching' => sub {
    my ($crash) = @{$issues->validate_json($export)};
    my $triaged = $crash->patch(status => 'triaged', labels => ['crash', 'p1']);
    is $triaged->status, 'triaged';
    is [sort @{$triaged->labels}], ['crash', 'p1'];
    is $triaged->comments->[0]->author, 'ada', 'the rest is kept';
    is [$triaged->model_fields_set], [qw(attachments comments id labels link severity status title)];

    my $e = dies { $crash->patch(duplicate_of => '0e7ac198-9acd-4c0c-b4b4-761974bf71d7') };
    is $e->errors->[0]{msg}, 'Value error, a duplicate is closed as wontfix';
    $e = dies { $crash->patch(stauts => 'fixed') };
    is $e->errors->[0]{type}, 'extra_forbidden', 'a typo in a PATCH is caught';
};

subtest 'exporting back' => sub {
    my $list = $issues->validate_json($export);
    my $json = $issues->dump_json($list, by_alias => 1, exclude_defaults => 1);
    like $json, qr/"html_url":"https:\/\/tracker.example.com\/issues\/3f0b9a52"/, 'under the other tracker\'s names';
    unlike $json, qr/"status":"open"/, 'defaults are left out';
    like $json, qr/"severity":4/, 'severities as their ranks';
    unlike $json, qr/"severity":2/, 'the default severity is left out';
    my $again = $issues->validate_json($json);
    is $issues->dump($again), $issues->dump($list), 'and read back the same';
    is $issues->dump($list, mode => 'json')->[0]{id}, '3f0b9a52-8c1e-4c7d-9b6a-2e4f5d1c0a97';

    my $schema = $issues->json_schema(mode => 'serialization', by_alias => 1);
    my $issue = $schema->{'$defs'}{Issue};
    is $issue->{properties}{html_url}, {type => 'string', format => 'uri', minLength => 1, title => 'Html Url'};
    is $issue->{properties}{id}, {type => 'string', format => 'uuid4', title => 'Id'};
    is $issue->{additionalProperties}, F();
    is $schema->{'$defs'}{Severity}, {enum => [1, 2, 3, 4], title => 'Tracker::Severity', type => 'integer'};
};

done_testing;
