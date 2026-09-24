use v5.36;
use Test2::V0;

# The CPAN distribution is complete and consistent (docs/RELEASING.md).

use ExtUtils::Manifest qw(maniread maniskip);
use FindBin;

chdir "$FindBin::Bin/.." or die $!;

my $manifest = maniread();
ok scalar keys %$manifest, 'MANIFEST lists files';
is [grep { !-e } sort keys %$manifest], [], 'every file in MANIFEST exists';

subtest 'MANIFEST lists every tracked file MANIFEST.SKIP keeps' => sub {
    skip_all 'not a git checkout' if !-d '.git';
    my @tracked = `git ls-files`;
    skip_all 'git is not available' if $?;
    chomp @tracked;
    my $skip = maniskip();
    my %seen;
    my @wanted = sort grep { !$skip->($_) && !$seen{$_}++ } @tracked, 'MANIFEST';
    is [sort keys %$manifest], \@wanted;
};

subtest 'one version' => sub {
    my %versions;
    for my $file (grep {m{^lib/.*\.pm\z}} keys %$manifest) {
        open my $fh, '<', $file or die "$file: $!";
        my ($version) = map { /^our \$VERSION = '([^']+)';/ ? $1 : () } <$fh>;
        $versions{$file} = $version;
    }
    my $version = $versions{'lib/Perldantic.pm'};
    ok defined $version, 'Perldantic has a version';
    is \%versions, {map { $_ => $version } keys %versions}, 'every module has it';

    open my $changes, '<', 'Changes' or die "Changes: $!";
    ok scalar(grep {/^\Q$version\E\s/} <$changes>), 'Changes has an entry for it';

    open my $cargo, '<', 'Cargo.toml' or die "Cargo.toml: $!";
    my ($crate) = map { /^version = "([^"]+)"/ ? $1 : () } <$cargo>;
    my ($major, $minor) = $version =~ /\A(\d+)\.(\d\d)\z/ or die "unexpected version $version";
    is $crate, sprintf('%d.%d.0', $major, $minor), 'the Rust crates are the same release';
};

done_testing;
