package Perldantic::Type;

use v5.36;

our $VERSION = '0.01';

use Hash::Util::FieldHash ();
use Scalar::Util ();

use Perldantic::Error;

use overload
    '""'     => sub ($self, @) { $self->{name} },
    # Moo's isa: a sub that dies for invalid values
    '&{}'    => sub ($self, @) { sub ($value, @) { $self->_adapter->validate($value); return } },
    '|'      => sub ($self, $other, $swap, @) { Perldantic::Types::_union($swap ? ($other, $self) : ($self, $other)) },
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
    set       => [qw(strict min_length max_length fail_fast)],
    frozenset => [qw(strict min_length max_length fail_fast)],
    dict  => [qw(strict min_length max_length)],
    'typed-dict' => [qw(strict extra_behavior)],
    decimal   => [qw(strict allow_inf_nan multiple_of le lt ge gt max_digits decimal_places)],
    date      => [qw(strict le lt ge gt now_op now_utc_offset)],
    time      => [qw(strict le lt ge gt tz_constraint microseconds_precision)],
    datetime  => [qw(strict le lt ge gt now_op now_utc_offset tz_constraint microseconds_precision)],
    timedelta => [qw(strict le lt ge gt microseconds_precision)],
    uuid      => [qw(strict version)],
    url       => [qw(strict max_length allowed_schemes host_required default_host default_port default_path preserve_empty_path)],
    'multi-host-url' => [qw(strict max_length allowed_schemes host_required default_host default_port default_path preserve_empty_path)],
);
# Constraints the core takes as booleans; any Perl truth value is accepted.
my %FLAG = map { $_ => 1 } qw(strict fail_fast allow_inf_nan strip_whitespace to_lower to_upper host_required preserve_empty_path);
# Constraints that take one of a few strings.
my %CHOICE = (extra_behavior => [qw(allow ignore forbid)]);
# Every constraint name, whatever the type.
our %ANY_CONSTRAINT = map { $_ => 1 } map {@$_} values %CONSTRAINTS;
my %ALLOWED = map { my $t = $_; ($t => {map { $_ => 1 } @{$CONSTRAINTS{$t}}}) } keys %CONSTRAINTS;

sub new ($class, %args) {
    if (ref $class) {
        # `DateTime->new(...)` in a package that imports the DateTime type
        my $name = $class->name;
        Perldantic::UsageError->throw(message =>
                "$name->new(...) called the Perldantic type $name; to call the class, write ${name}::->new(...)");
    }
    return bless {parameters => [], %args}, $class;
}

# `DateTime->now` in a package that imports the DateTime type calls the type, not the class.
our $AUTOLOAD;

sub AUTOLOAD ($self, @) {
    my ($method) = $AUTOLOAD =~ /::(\w+)\z/;
    my $name = ref $self ? $self->name : $self;
    Perldantic::UsageError->throw(message => ref $self
        ? "$name->$method(...) called the Perldantic type $name; to call the class, write ${name}::->$method(...)"
        : "Can't locate method $method in $name");
}

sub DESTROY { }

sub name ($self)        { $self->{name} }
sub parameters ($self)  { @{$self->{parameters}} }
sub is_optional ($self) { !!$self->{optional} }
sub is_slurpy ($self)   { !!$self->{slurpy} }

sub core_schema ($self) {
    return $self->{build}->() if $self->{build};
    return $self->{wrap}->($self->{inner}->core_schema) if $self->{inner};
    return _clone($self->{schema});
}

# A deep copy of plain data. Unlike Storable::dclone on Perl 5.36, it keeps booleans booleans.
sub _clone ($data) {
    return {map { $_ => _clone($data->{$_}) } keys %$data} if ref $data eq 'HASH';
    return [map { _clone($_) } @$data] if ref $data eq 'ARRAY';
    return $data;
}

# The Type::API constraint protocol, which Type::Tiny (Types::TypeTiny::to_TypeTiny) and through
# it Moose understand: validation decides, and its conversion is the coercion.
Hash::Util::FieldHash::fieldhash my %ADAPTER;

# A TypeAdapter of the type, kept per type object. It holds a copy of the type: holding the type
# itself would keep both alive for good.
sub _adapter ($self) {
    return $ADAPTER{$self} //= do {
        require Perldantic::TypeAdapter;
        Perldantic::TypeAdapter->new(bless {%$self}, ref $self);
    };
}

# (valid, the validated value, the error)
sub _validated ($self, $value) {
    local $@;
    my $result = eval { $self->_adapter->validate($value) };
    return $@ ? (0, undef, $@) : (1, $result, undef);
}

# Whether two values are the same data: scalars by their text, references by address or,
# unblessed, by their contents.
sub _same ($x, $y) {
    return defined $x == defined $y && (!defined $x || $x eq $y) if !ref $x || !ref $y;
    return 1 if Scalar::Util::refaddr($x) == Scalar::Util::refaddr($y);
    return 0 if ref $x ne ref $y || Scalar::Util::blessed $x;
    return @$x == @$y && !grep { !_same($x->[$_], $y->[$_]) } 0 .. $#$x if ref $x eq 'ARRAY';
    return keys %$x == keys %$y && !grep { !exists $y->{$_} || !_same($x->{$_}, $y->{$_}) } keys %$x
        if ref $x eq 'HASH';
    return 0;
}

# A value of the type is one validation accepts as it is: what it would convert (a hash into a
# model object, a string into a date) is for the coercion, as Type::Tiny and Moose expect.
sub check ($self, $value) {
    my ($ok, $result) = $self->_validated($value);
    return $ok && _same($value, $result) ? !!1 : !!0;
}

sub get_message ($self, $value) {
    my ($ok, $result, $error) = $self->_validated($value);
    return "$error" if !$ok;
    return undef if _same($value, $result);
    return "Validation converts the value to a $self->{name}; coerce it first";
}

sub has_coercion ($self) { !!1 }

sub coerce ($self, $value) {
    my ($ok, $result) = $self->_validated($value);
    return $ok ? $result : $value;
}

sub coercion ($self) {
    return sub ($value, @) { $self->coerce($value) };
}

sub with ($self, %constraints) {
    if ($self->{members} && exists $constraints{discriminator}) {
        my $field = delete $constraints{discriminator};
        Perldantic::UsageError->throw(message => "Constraint '$_' does not apply to a union with a discriminator")
            for sort keys %constraints;
        return Perldantic::Types::_tagged_union($self, $field);
    }
    # Nothing to add: keep the type, so a union's alternatives are still looked at only when first
    # used (their models may be declared later).
    return $self if !%constraints;
    if ($self->{inner}) {
        return (ref $self)->new(%$self, inner => $self->{inner}->with(%constraints));
    }
    # Types built on first use (unions) take no constraints.
    my $allowed = $self->{schema} ? $ALLOWED{$self->{schema}{type}} // {} : {};
    for my $key (sort keys %constraints) {
        Perldantic::UsageError->throw(message => "Constraint '$key' does not apply to $self->{name}")
            if !$allowed->{$key};
        $constraints{$key} = $constraints{$key} ? !!1 : !!0 if $FLAG{$key};
        if (my $choices = $CHOICE{$key}) {
            my $value = $constraints{$key} // 'undef';
            Perldantic::UsageError->throw(message => "$key takes "
                    . join(', ', map {"'$_'"} @$choices[0 .. $#$choices - 1])
                    . " or '$choices->[-1]', got '$value'")
                if !grep { $_ eq $value } @$choices;
        }
    }
    my $schema = {%{$self->core_schema}, %constraints};
    # pydantic's UUID1..UUID8 annotations name the version in the JSON Schema format
    $schema->{metadata} = {%{$schema->{metadata} // {}}, pydantic_js_updates => {format => "uuid$constraints{version}"}}
        if $schema->{type} eq 'uuid' && defined $constraints{version};
    return (ref $self)->new(%$self, schema => $schema);
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

=head2 new(%fields)

Used by L<Perldantic::Types>; build types with its functions instead. Calling C<new> (or any
other method a class has) on a type, as C<< DateTime->new(...) >> does in a package that imports
the C<DateTime> type, raises C<Perldantic::UsageError> with a hint to write
C<< DateTime::->new(...) >>.

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

=item C<Set>: C<strict min_length max_length fail_fast>

=item C<Bool>, C<Dict>: C<strict>

=item C<Decimal>: C<strict allow_inf_nan multiple_of le lt ge gt max_digits decimal_places>

=item C<Date>: C<strict le lt ge gt now_op now_utc_offset>

=item C<Time>: C<strict le lt ge gt tz_constraint microseconds_precision>

=item C<DateTime>: the C<Date> and C<Time> constraints

=item C<Duration>: C<strict le lt ge gt microseconds_precision>

=item C<Uuid>: C<strict version>

=item C<Url>, C<MultiHostUrl>: C<strict max_length allowed_schemes host_required default_host
default_port default_path preserve_empty_path>

=back

C<Maybe[T]> and C<Optional[T]> pass constraints on to C<T>. A constraint that does not apply
to the type raises C<Perldantic::UsageError>.

=head2 parameters

The type parameters (for C<Dict[]>, the names and types in declared order).

=head2 is_optional

True for C<Optional[T]>.

=head2 is_slurpy

True for C<slurpy ArrayRef[T]> and C<slurpy HashRef[T]>.

=head1 IN MOO, MOOSE AND TYPE::TINY

A type is a type constraint for classes that are not Perldantic models too. It implements the
constraint protocol of L<Type::API> (C<check>, C<get_message>, C<has_coercion>, C<coerce>),
which L<Types::TypeTiny>'s C<to_TypeTiny> turns into a L<Type::Tiny> type, and it can be called
as a code reference that dies with a C<Perldantic::ValidationError> for invalid values, which
is what L<Moo>'s C<isa> takes:

    package Order;
    use Moo;
    use Perldantic::Types qw(ArrayRef InstanceOf Int Str);

    has id    => (is => 'ro', isa => Int->with(gt => 0), required => 1);
    has tags  => (is => 'ro', isa => ArrayRef[Str]);
    has where => (is => 'ro', isa => InstanceOf['Point'], coerce => InstanceOf(['Point'])->coercion);

In L<Moose>, give C<isa> the Type::Tiny type:
C<< isa => Types::TypeTiny::to_TypeTiny(ArrayRef[Int]), coerce => 1 >>.

Called as a code reference (Moo's C<isa>), a type accepts what validation accepts. The coercion
is validation's conversion, so a hash reference becomes a model object and C<'1.50'> becomes
C<1.5>.

=head2 check($value)

Whether C<$value> is a value of the type: validation accepts it and gives it back as it is.
Values that validation converts (a hash reference that becomes a model object, a string that
becomes a date) are not, and the coercion converts them, as Type::Tiny and Moose expect.
Scalars count as the same when their text is (C<'2'> passes C<Int>, as in Types::Standard).

=head2 get_message($value)

Why C<$value> is not a value of the type (the validation error, or that validation would
convert it), or C<undef> when it is.

=head2 has_coercion, coerce($value), coercion

C<has_coercion> is true. C<coerce> returns the validated value, or C<$value> as it is when it
is invalid; C<coercion> is C<coerce> as a code reference (for Moo's C<coerce>).


=head1 SEE ALSO

L<Perldantic::Types>, L<Perldantic::TypeAdapter>, L<Perldantic>

=cut
