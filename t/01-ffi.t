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

subtest 'result envelopes' => sub {
    is Perldantic::FFI::_unwrap({ok => 1}), {ok => 1}, 'ok envelopes are returned';
    my $e = dies {
        Perldantic::FFI::_unwrap(
            {error => {type => 'InternalError', message => 'Rust panic in perldantic-core: boom'}});
    };
    isa_ok $e, 'Perldantic::InternalError';
    is $e->message, 'Rust panic in perldantic-core: boom', 'a Rust panic becomes an InternalError';
    $e = dies { Perldantic::FFI::_unwrap({}) };
    isa_ok $e, 'Perldantic::InternalError';
    is $e->message, 'Malformed result from the core';
    $e = dies { Perldantic::FFI::_envelope(undef) };
    isa_ok $e, 'Perldantic::InternalError';
};

subtest 'options must be a hash reference' => sub {
    my $e = dies { Perldantic::FFI::_options([]) };
    isa_ok $e, 'Perldantic::UsageError';
    is $e->message, 'Options must be a hash reference';
};

done_testing;
