package Perldantic::FFI;

use v5.36;
use FFI::Platypus 2.00;

our $VERSION = '0.01';

my $ffi = FFI::Platypus->new(api => 2, lang => 'Rust');
# The shared library belongs to the Perldantic distribution, not to this package.
$ffi->bundle('Perldantic');

$ffi->attach([pd_version => 'version'] => [] => 'string');

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::FFI - low-level binding to the perldantic Rust library

=head1 FUNCTIONS

=head2 version

Returns the version of the bundled Rust core.

=cut
