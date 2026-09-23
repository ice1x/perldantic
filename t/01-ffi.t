use v5.36;
use Test2::V0;

use Perldantic::FFI;

my $core_version = do {
    open my $fh, '<', 'Cargo.toml' or die "Cargo.toml: $!";
    my ($v) = join('', <$fh>) =~ /^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"/ms;
    $v;
};

is Perldantic::FFI::version(), $core_version, 'Rust core version is reachable through FFI';
is Perldantic::FFI::version(), Perldantic::FFI::version(), 'version is stable across calls';

done_testing;
