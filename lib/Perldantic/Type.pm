package Perldantic::Type;

use v5.36;

our $VERSION = '0.01';

use Storable ();

use Perldantic::Error;

use overload
    '""'     => sub ($self, @) { $self->{name} },
    bool     => sub { 1 },
    fallback => 1;

# The constraints each core schema type takes (pydantic's names).
my %CONSTRAINTS = (
    int   => [qw(strict gt ge lt le multiple_of)],
    float => [qw(strict gt ge lt le multiple_of allow_inf_nan)],
    str   => [qw(strict min_length max_length pattern strip_whitespace to_lower to_upper)],
    bytes => [qw(strict min_length max_length)],
    bool  => [qw(strict)],
    list  => [qw(strict min_length max_length)],
    tuple => [qw(strict min_length max_length)],
    dict  => [qw(strict min_length max_length)],
    'typed-dict' => [qw(strict)],
);
my %ALLOWED = map { my $t = $_; ($t => {map { $_ => 1 } @{$CONSTRAINTS{$t}}}) } keys %CONSTRAINTS;

sub new ($class, %args) {
    return bless {parameters => [], %args}, $class;
}

sub name ($self)        { $self->{name} }
sub parameters ($self)  { @{$self->{parameters}} }
sub is_optional ($self) { !!$self->{optional} }
sub is_slurpy ($self)   { !!$self->{slurpy} }

sub core_schema ($self) {
    return $self->{wrap}->($self->{inner}->core_schema) if $self->{inner};
    return Storable::dclone($self->{schema});
}

sub with ($self, %constraints) {
    if ($self->{inner}) {
        return (ref $self)->new(%$self, inner => $self->{inner}->with(%constraints));
    }
    my $allowed = $ALLOWED{$self->{schema}{type}} // {};
    for my $key (sort keys %constraints) {
        Perldantic::UsageError->throw(message => "Constraint '$key' does not apply to $self->{name}")
            if !$allowed->{$key};
    }
    return (ref $self)->new(%$self, schema => {%{$self->core_schema}, %constraints});
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Type - a Perldantic type: a name and a pydantic core schema

=head1 SYNOPSIS

    use Perldantic::Types qw(Int);

    my $positive = Int->with(gt => 0);
    say $positive->name;             # Int
    my $schema = $positive->core_schema;   # {type => 'int', gt => 0}

=head1 DESCRIPTION

Types are built by the functions of L<Perldantic::Types>; they stringify to their name
(C<ArrayRef[Int]>).

=head1 METHODS

=head2 name

The Types::Standard-style name, e.g. C<Map[Int,Str]>.

=head2 core_schema

A fresh copy of the pydantic core schema, as Perl data.

=head2 with(%constraints)

A copy of the type with pydantic constraints added to its schema:

=over

=item C<Int>: C<strict gt ge lt le multiple_of>

=item C<Num>: the same plus C<allow_inf_nan>

=item C<Str>: C<strict min_length max_length pattern strip_whitespace to_lower to_upper>

=item C<Bytes>, C<ArrayRef>, C<Tuple>, C<HashRef>, C<Map>: C<strict min_length max_length>

=item C<Bool>, C<Dict>: C<strict>

=back

C<Maybe[T]> and C<Optional[T]> pass constraints on to C<T>. A constraint that does not apply
to the type raises C<Perldantic::UsageError>.

=head2 parameters

The type parameters (for C<Dict[]>, the names and types in declared order).

=head2 is_optional

True for C<Optional[T]>.

=head2 is_slurpy

True for C<slurpy ArrayRef[T]>.

=cut
