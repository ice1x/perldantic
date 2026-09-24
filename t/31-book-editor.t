use v5.36;
use Test2::V0;

# A story / book editor: a book is a tree of chapters, each with scenes and sub-chapters to any
# depth. Drafts are autosaved as JSON, edited as copies, and checked before publishing.

package Book::Scene {
    use Perldantic;

    has title => (is => 'ro', isa => Str, required => 1, min_length => 1);
    has text  => (is => 'ro', isa => Str, default => '');
    has pov   => (is => 'ro', isa => Maybe[Str]);

    computed_field words => (isa => Int) => sub ($self) { scalar(() = $self->text =~ /\S+/g) };
}

package Book::Chapter {
    use Perldantic;

    has title    => (is => 'ro', isa => Str, required => 1, min_length => 1, max_length => 120);
    has slug     => (is => 'ro', isa => Str, pattern => '^[a-z0-9]+(?:-[a-z0-9]+)*$');
    has scenes   => (is => 'ro', isa => ArrayRef['Book::Scene'], default => sub { [] });
    has chapters => (is => 'ro', isa => ArrayRef['Book::Chapter'], default => sub { [] });

    # a missing slug is made from the title, before the pattern checks it
    model_validator mode => 'before', sub ($class, $data) {
        return $data if ref $data ne 'HASH' || defined $data->{slug} || !defined $data->{title};
        return {%$data, slug => lc($data->{title} =~ s/[^A-Za-z0-9]+/-/gr =~ s/\A-|-\z//gr)};
    };

    computed_field words => (isa => Int) => sub ($self) {
        my $words = 0;
        $words += $_->words for @{$self->scenes}, @{$self->chapters};
        return $words;
    };

    # the table of contents below this chapter: [depth, title]
    sub outline ($self, $depth = 0) {
        return ([$depth, $self->title], map { $_->outline($depth + 1) } @{$self->chapters});
    }
}

package Book::Book {
    use Perldantic;

    has title    => (is => 'ro', isa => Str, required => 1);
    has author   => (is => 'ro', isa => Str, required => 1);
    has status   => (is => 'ro', isa => Enum[qw(draft review published)], default => 'draft');
    has chapters => (is => 'ro', isa => ArrayRef['Book::Chapter'], default => sub { [] });

    # publishing needs every chapter to have some text
    model_validator mode => 'after', sub ($self) {
        return $self if $self->status ne 'published';
        my @empty = grep { $_->words == 0 } map { _all_chapters($_) } @{$self->chapters};
        die 'empty chapters cannot be published: ' . join(', ', map { $_->title } @empty) . "\n" if @empty;
        return $self;
    };

    sub _all_chapters ($chapter) { ($chapter, map { _all_chapters($_) } @{$chapter->chapters}) }

    sub outline ($self) { [map { $_->outline } @{$self->chapters}] }
}

package main;

my $draft = {
    title    => 'The Lighthouse Keeper',
    author   => 'M. Reyes',
    chapters => [
        {
            title  => 'Part One: Arrival',
            scenes => [{title => 'The ferry', text => 'Fog rolled over the bay as the ferry docked.'}],
            chapters => [
                {title => 'The Keeper', scenes => [{title => 'A lamp', text => 'He trimmed the wick twice.', pov => 'Elias'}]},
                {title => 'Storm Warning', chapters => [{title => 'Night Watch', scenes => [{title => 'Radio', text => 'Static.'}]}]},
            ],
        },
        {title => 'Part Two: Departure', scenes => [{title => 'Goodbye', text => 'She left the key under the stone.'}]},
    ],
};

subtest 'a book is a tree of chapters' => sub {
    my $book = Book::Book->new(%$draft);
    is $book->outline, [
        [0, 'Part One: Arrival'], [1, 'The Keeper'], [1, 'Storm Warning'], [2, 'Night Watch'],
        [0, 'Part Two: Departure'],
    ];
    my $part_one = $book->chapters->[0];
    is $part_one->slug, 'part-one-arrival', 'slugs from titles';
    is $part_one->chapters->[1]->chapters->[0]->slug, 'night-watch', 'at any depth';
    is $part_one->words, 9 + 5 + 1, 'word counts add up through the tree';
    isa_ok $part_one->chapters->[1]->chapters->[0], 'Book::Chapter';
};

subtest 'mistakes deep in the tree' => sub {
    my $bad = {%$draft, chapters => [{title => 'Part One', chapters => [
        {title => 'Fine', chapters => [{title => '', slug => 'Not A Slug', scenes => [{text => 'no title'}]}]},
    ]}]};
    my $e = dies { Book::Book->new(%$bad) };
    isa_ok $e, 'Perldantic::ValidationError';
    is [sort map { join('.', @{$_->{loc}}) . ":$_->{type}" } @{$e->errors}], [
        'chapters.0.chapters.0.chapters.0.scenes.0.title:missing',
        'chapters.0.chapters.0.chapters.0.slug:string_pattern_mismatch',
        'chapters.0.chapters.0.chapters.0.title:string_too_short',
    ], 'located down the tree';
};

subtest 'autosave and reload' => sub {
    my $book = Book::Book->new(%$draft);
    my $saved = $book->model_dump_json(exclude_defaults => 1);
    unlike $saved, qr/"status"/, 'defaults are left out of the autosave';
    like $saved, qr/"words":15/, 'computed fields are part of the document';

    # reloading ignores the computed fields (they are recomputed)
    my $back = Book::Book->model_validate_json($saved);
    is $back->outline, $book->outline;
    is $back->model_dump, $book->model_dump, 'nothing lost';
};

subtest 'editing a copy' => sub {
    my $book = Book::Book->new(%$draft);
    my $renamed = $book->model_copy(update => {title => 'The Last Lighthouse'});
    is $renamed->title, 'The Last Lighthouse';
    is $book->title, 'The Lighthouse Keeper', 'the original is untouched';
    ref_is $renamed->chapters, $book->chapters, 'a shallow copy shares the chapters';
    my $deep = $book->model_copy(deep => 1);
    ok $deep->chapters != $book->chapters, 'a deep copy does not';
    is $deep->model_dump, $book->model_dump;
};

subtest 'publishing' => sub {
    my $e = dies { Book::Book->new(%$draft, status => 'published',
        chapters => [@{$draft->{chapters}}, {title => 'Epilogue'}, {title => 'Notes', chapters => [{title => 'Sources'}]}]) };
    is $e->errors->[0]{msg}, 'Value error, empty chapters cannot be published: Epilogue, Notes, Sources';
    ok lives { Book::Book->new(%$draft, status => 'published') }, 'a complete book can be';
};

subtest 'the schema is recursive' => sub {
    my $schema = Book::Book->model_json_schema;
    is $schema->{'$defs'}{Chapter}{properties}{chapters},
        {type => 'array', items => {'$ref' => '#/$defs/Chapter'}, title => 'Chapters'},
        'chapters refer to themselves';
    is $schema->{properties}{status}, {enum => ['draft', 'review', 'published'], type => 'string', default => 'draft',
        title => 'Status'};
};

done_testing;
