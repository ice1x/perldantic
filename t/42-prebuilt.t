use v5.36;
use Test2::V0;

use File::Temp qw(tempdir);

use lib 'inc';
use Perldantic::Prebuilt;

# Installing without Rust (inc/Perldantic/Prebuilt.pm, used by Makefile.PL).

subtest 'targets' => sub {
    is Perldantic::Prebuilt::target('darwin', 'darwin-2level', "arm64\n"), 'aarch64-apple-darwin';
    is Perldantic::Prebuilt::target('darwin', 'darwin-thread-multi-2level', "x86_64\n"), 'x86_64-apple-darwin';
    is Perldantic::Prebuilt::target('linux', 'x86_64-linux-gnu-thread-multi'), 'x86_64-unknown-linux-gnu';
    is Perldantic::Prebuilt::target('linux', 'aarch64-linux'), 'aarch64-unknown-linux-gnu';
    is Perldantic::Prebuilt::target('linux', 'x86_64-linux-musl'), undef, 'musl';
    is Perldantic::Prebuilt::target('linux', 'armv7l-linux'), undef, 'other CPUs';
    is Perldantic::Prebuilt::target('MSWin32', 'MSWin32-x64-multi-thread'), undef, 'other systems';
    is Perldantic::Prebuilt::file_for('aarch64-apple-darwin'), 'libperldantic-aarch64-apple-darwin.dylib';
    is Perldantic::Prebuilt::file_for('x86_64-unknown-linux-gnu'), 'libperldantic-x86_64-unknown-linux-gnu.so';
    is Perldantic::Prebuilt::installed_name('darwin'), 'libPerldantic.dylib', 'the name FFI::Build gives it';
    is Perldantic::Prebuilt::installed_name('linux'), 'libPerldantic.so';
};

subtest 'mode' => sub {
    is Perldantic::Prebuilt::mode({}, 1), 'source', 'Rust when cargo runs';
    is Perldantic::Prebuilt::mode({}, 0), 'prebuilt', 'else prebuilt';
    is Perldantic::Prebuilt::mode({PERLDANTIC_BUILD => 'prebuilt'}, 1), 'prebuilt', 'chosen';
    is Perldantic::Prebuilt::mode({PERLDANTIC_BUILD => 'source'}, 0), 'source';
    like dies { Perldantic::Prebuilt::mode({PERLDANTIC_BUILD => 'fast'}, 0) }, qr/must be source, prebuilt or auto/;
};

my $dir = tempdir(CLEANUP => 1);
my $library = "$dir/release/libperldantic-x86_64-unknown-linux-gnu.so";
mkdir "$dir/release";
open my $fh, '>', $library or die $!;
print $fh "not really a library\n";
close $fh;
my $sum = Perldantic::Prebuilt::sha256_of($library);

sub sums ($text) {
    my $path = "$dir/SHA256SUMS";
    open my $out, '>', $path or die $!;
    print $out $text;
    close $out;
    return $path;
}

subtest 'checksums' => sub {
    my ($url, $sums) = Perldantic::Prebuilt::parse_sums("# https://example.com/v1\n$sum  a.so\n$sum *b.so\n\n");
    is $url, 'https://example.com/v1/';
    is $sums, {'a.so' => $sum, 'b.so' => $sum};
    like dies { Perldantic::Prebuilt::parse_sums("$sum  a.so\n") }, qr/names no release URL/;
    like dies { Perldantic::Prebuilt::parse_sums("# u\nabc a.so\n") }, qr/Malformed line/;
};

subtest 'fetching' => sub {
    my %args = (target => 'x86_64-unknown-linux-gnu', dir => "$dir/out");
    my $path = Perldantic::Prebuilt::fetch(%args, sums => sums("# file://$dir/release/\n$sum  libperldantic-x86_64-unknown-linux-gnu.so\n"));
    is Perldantic::Prebuilt::sha256_of($path), $sum, 'downloaded and checked';

    unlink $path;
    my $wrong = 'f' x 64;
    like dies { Perldantic::Prebuilt::fetch(%args, sums => sums("# file://$dir/release/\n$wrong  libperldantic-x86_64-unknown-linux-gnu.so\n")) },
        qr/Checksum mismatch for libperldantic-x86_64-unknown-linux-gnu\.so/;
    ok !-e $path, 'a bad download is not kept';

    like dies { Perldantic::Prebuilt::fetch(%args, sums => sums("# file://$dir/release/\n$sum  other.so\n")) },
        qr/No prebuilt core library for x86_64-unknown-linux-gnu; install Rust/;
    like dies { Perldantic::Prebuilt::fetch(%args, sums => "$dir/none") }, qr/has no prebuilt core libraries/;
    like dies { Perldantic::Prebuilt::fetch(%args, target => undef, sums => "$dir/none") },
        qr/install Rust/ if !defined Perldantic::Prebuilt::target();
};

subtest 'installing into blib' => sub {
    my $blib = "$dir/blib";
    my $target = Perldantic::Prebuilt::install_blib($library, $blib, 'linux');
    is $target, "$blib/lib/auto/share/dist/Perldantic/lib/libPerldantic.so";
    ok -f $target;
    open my $note, '<', "$blib/arch/auto/Perldantic/Perldantic.txt" or die $!;
    is scalar(<$note>), "FFI::Build\@auto/share/dist/Perldantic/lib/libPerldantic.so\n", 'the note FFI::Platypus reads';
};

done_testing;
