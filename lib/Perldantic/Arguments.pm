package Perldantic::Arguments;

use v5.36;

our $VERSION = '0.01';

use Scalar::Util qw(blessed);

use Perldantic::Error;

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

sub new ($class, %options) {
    for my $key (sort keys %options) {
        _usage("Perldantic::Arguments->new: unknown option '$key'") if $key ne 'args' && $key ne 'kwargs';
    }
    my $args = $options{args} // [];
    my $kwargs = $options{kwargs} // {};
    _usage('Perldantic::Arguments->new: args must be an array reference') if ref $args ne 'ARRAY';
    _usage('Perldantic::Arguments->new: kwargs must be a hash reference')
        if ref $kwargs ne 'HASH' && !(blessed $kwargs && $kwargs->isa('Perldantic::Wire::Ordered'));
    return bless {args => $args, kwargs => $kwargs}, $class;
}

sub args ($self)   { $self->{args} }
sub kwargs ($self) { $self->{kwargs} }

# The wire JSON: {"$args_kwargs": [args, kwargs or null]}.
sub _wire_json ($self) {
    my $kwargs = $self->{kwargs};
    my $empty = ref $kwargs eq 'HASH' ? !%$kwargs : !@$kwargs;
    return '{"$args_kwargs":[' . Perldantic::Wire::_emit_any($self->{args}) . ','
        . ($empty ? 'null' : Perldantic::Wire::_emit_any($kwargs)) . ']}';
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Arguments - the arguments of a call: positional and named

=head1 SYNOPSIS

    use Perldantic::Arguments;

    my $arguments = Perldantic::Arguments->new(args => [1, 2], kwargs => {scale => 10});
    $arguments->args;      # [1, 2]
    $arguments->kwargs;    # {scale => 10}

=head1 DESCRIPTION

pydantic-core's C<ArgsKwargs>: what an C<arguments> core schema validates, positional arguments
and keyword (named) arguments together. Such a schema also takes an array reference (positional
arguments only) or a hash reference (named arguments only); this class carries both.

=head1 METHODS

=head2 new(args => \@args, kwargs => \%kwargs)

Both are optional and default to empty. Anything else raises C<Perldantic::UsageError>.

=head2 args, kwargs

The positional arguments (an array reference) and the named arguments (a hash reference).

=head1 SEE ALSO

L<Perldantic::FFI>, L<Perldantic::Wire>

=cut
