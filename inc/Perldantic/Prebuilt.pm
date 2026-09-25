package Perldantic::Prebuilt;

# Installing without Rust: Makefile.PL uses this module (it is not installed) to fetch the core
# library built for the platform by the release workflow (.github/workflows/prebuilt.yml) and
# put it where FFI::Build would have built it. Downloads are checked against the checksums in
# prebuilt/SHA256SUMS, which the distribution ships (docs/RELEASING.md).

use v5.36;

use Config;
use Digest::SHA ();
use File::Basename qw(dirname);
use File::Copy ();
use File::Path ();
use File::Spec;
use HTTP::Tiny;

our $SUMS = 'prebuilt/SHA256SUMS';

# The Rust target the core is built for on this platform, or undef when none is prebuilt. The
# CPU is the machine's on macOS (`uname -m`, which Rosetta reports as x86_64), the archname's on
# Linux.
sub target ($os = $^O, $arch = $Config{archname}, $machine = undef) {
    my $cpu_of = sub ($text) {
        $text =~ /\A(?:aarch64|arm64)\b/ ? 'aarch64' : $text =~ /\A(?:x86_64|amd64)\b/ ? 'x86_64' : undef;
    };
    if ($os eq 'darwin') {
        my $cpu = $cpu_of->($machine // `uname -m`) // return undef;
        return "$cpu-apple-darwin";
    }
    if ($os eq 'linux') {
        # musl systems cannot load a glibc library
        return undef if $arch =~ /musl/;
        my $cpu = $cpu_of->($arch) // return undef;
        return "$cpu-unknown-linux-gnu";
    }
    return undef;
}

# The library file of a target, as the release workflow names it.
sub file_for ($target) {
    return "libperldantic-$target." . ($target =~ /darwin/ ? 'dylib' : 'so');
}

# The name FFI::Build gives the library it builds (and FFI::Platypus's bundle looks for).
sub installed_name ($os = $^O) {
    return 'libPerldantic.' . ($os eq 'darwin' ? 'dylib' : 'so');
}

# How to get the core library: `source` (cargo builds it) or `prebuilt`. PERLDANTIC_BUILD
# chooses; by default Rust is used when cargo runs.
sub mode ($env = \%ENV, $have_cargo = undef) {
    my $wanted = $env->{PERLDANTIC_BUILD} // 'auto';
    die "PERLDANTIC_BUILD must be source, prebuilt or auto, got '$wanted'\n" if $wanted !~ /\A(?:source|prebuilt|auto)\z/;
    return $wanted if $wanted ne 'auto';
    $have_cargo //= _have_cargo();
    return $have_cargo ? 'source' : 'prebuilt';
}

sub _have_cargo () {
    my $null = File::Spec->devnull;
    return system("cargo --version >$null 2>&1") == 0;
}

# The release's base URL and file => checksum, from the SHA256SUMS text: a `# <url>` line, then
# `<sha256>  <file>` lines.
sub parse_sums ($text) {
    my ($url, %sum);
    for my $line (split /\n/, $text) {
        next if $line =~ /\A\s*\z/;
        if ($line =~ /\A#\s*(\S+)\s*\z/) { $url = $1; next }
        $line =~ /\A([0-9a-f]{64})\s+\*?(\S+)\s*\z/ or die "Malformed line in $SUMS: $line\n";
        $sum{$2} = $1;
    }
    die "$SUMS names no release URL\n" if !$url;
    $url .= '/' if $url !~ m{/\z};
    return ($url, \%sum);
}

sub sha256_of ($path) {
    return Digest::SHA->new(256)->addfile($path, 'b')->hexdigest;
}

# Download `$url` to `$path`: file:// URLs are copied, others fetched with HTTP::Tiny when it can
# do TLS, else with curl or wget.
sub download ($url, $path) {
    File::Path::make_path(dirname($path));
    if ($url =~ m{\Afile://(.*)\z}) {
        File::Copy::copy($1, $path) or die "Cannot copy $1: $!\n";
        return;
    }
    if (HTTP::Tiny->can_ssl) {
        my $response = HTTP::Tiny->new(agent => 'Perldantic-Makefile.PL')->mirror($url, $path);
        return if $response->{success};
        die "Cannot download $url: $response->{status} $response->{reason}\n";
    }
    for my $command (["curl", "-fsSL", "-o", $path, $url], ["wget", "-q", "-O", $path, $url]) {
        return if system(@$command) == 0;
    }
    die "Cannot download $url: install IO::Socket::SSL, curl or wget\n";
}

# Fetch the library for this platform into `$dir` and check it; returns its path.
sub fetch (%options) {
    my $sums_file = $options{sums} // $SUMS;
    my $target = $options{target} // target();
    die "No prebuilt core library for $^O ($Config{archname}); install Rust (https://rustup.rs) to build it\n"
        if !$target;
    open my $fh, '<', $sums_file
        or die "This distribution has no prebuilt core libraries ($sums_file); install Rust (https://rustup.rs) to build it\n";
    my ($url, $sum) = parse_sums(do { local $/; <$fh> });
    $url = $options{url} if defined $options{url};
    my $file = file_for($target);
    my $expected = $sum->{$file} // die "No prebuilt core library for $target; install Rust (https://rustup.rs) to build it\n";
    my $path = File::Spec->catfile($options{dir} // 'prebuilt', $file);
    download("$url$file", $path) if !-f $path || sha256_of($path) ne $expected;
    my $got = sha256_of($path);
    if ($got ne $expected) {
        unlink $path;
        die "Checksum mismatch for $file: expected $expected, got $got\n";
    }
    return $path;
}

# Put the library where FFI::Build puts the one it builds, under `$blib`, with the note that
# tells FFI::Platypus's bundle where it is.
sub install_blib ($library, $blib = 'blib', $os = $^O) {
    my $relative = "auto/share/dist/Perldantic/lib/" . installed_name($os);
    my $target = "$blib/lib/$relative";
    File::Path::make_path(dirname($target), "$blib/arch/auto/Perldantic");
    File::Copy::copy($library, $target) or die "Cannot copy $library to $target: $!\n";
    chmod 0755, $target;
    open my $fh, '>', "$blib/arch/auto/Perldantic/Perldantic.txt" or die "Cannot write the bundle note: $!\n";
    print $fh "FFI::Build\@$relative\n";
    close $fh or die "Cannot write the bundle note: $!\n";
    return $target;
}

1;
