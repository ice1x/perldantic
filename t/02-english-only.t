use v5.36;
use Test2::V0;
use File::Find;

# Project policy: docs, comments and code are English only.
# The vendored upstream snapshot, data recorded from upstream tests and build output are excluded.
my %top_skip = map { $_ => 1 } qw(.git upstream local);
my %path_skip = map { $_ => 1 } qw(./tests/conformance/upstream ./tools/.venv);
my %any_skip = map { $_ => 1 } qw(target blib _build);
my @offenders;

find(
    {
        preprocess => sub {
            grep {
                !$any_skip{$_}
                    && !($File::Find::dir eq '.' && $top_skip{$_})
                    && !$path_skip{"$File::Find::dir/$_"}
            } @_;
        },
        wanted => sub {
            return unless -f $_ && -T $_;
            open my $fh, '<:encoding(UTF-8)', $_ or return;
            while (my $line = <$fh>) {
                if ($line =~ /\p{Cyrillic}/) {
                    push @offenders, "$File::Find::name:$.";
                    last;
                }
            }
        },
    },
    '.'
);

is \@offenders, [], 'no non-English (Cyrillic) text outside upstream/';

done_testing;
