package Perldantic;

use v5.36;

our $VERSION = '0.01';

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic - pydantic for Perl, powered by a Rust core

=head1 DESCRIPTION

Perldantic brings pydantic's data validation to Perl and keeps its philosophy: declared types
are the schema, input is parsed into guaranteed types, and errors are structured data. The
engine is a Python-free port of C<pydantic-core>.

This release is a scaffold; see C<docs/PLAN.md> in the distribution for the roadmap.

=head1 LICENSE

MIT. Includes software derived from pydantic-core (MIT); see F<NOTICE>.

=cut
