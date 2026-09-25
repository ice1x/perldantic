package Perldantic::Enum;

use v5.36;
no warnings 'experimental::builtin';

our $VERSION = '0.01';

use Hash::Util ();
use Scalar::Util qw(blessed);

use Perldantic::Error;
use Perldantic::Wire;

# Declared enum classes: class => {members => [...], by_name => {...}, by_value => {...},
# sub_type => 'int' | 'str' | 'float' | undef}.
our %CLASSES;

use overload
    '""'     => sub ($self, @) { $self->{value} },
    'bool'   => sub { 1 },
    fallback => 1;

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

sub import ($class, @args) {
    return if $class ne __PACKAGE__;
    my $target = caller;
    _declare($target, @args);
}

# Members as name => value pairs, or one array reference of names that are their own values.
sub _declare ($class, @args) {
    my @pairs = @args == 1 && ref $args[0] eq 'ARRAY' ? map { ($_ => $_) } @{$args[0]} : @args;
    _usage("Perldantic::Enum ($class): declare at least one member") if !@pairs;
    _usage("Perldantic::Enum ($class): give member names and values in pairs") if @pairs % 2;
    my (@members, %by_name, %by_value);
    while (my ($name, $value) = splice @pairs, 0, 2) {
        _usage("Perldantic::Enum ($class): '" . ($name // 'undef') . "' is not a valid member name")
            if !defined $name || ref $name || $name !~ /\A[A-Za-z_]\w*\z/;
        _usage("Perldantic::Enum ($class): member $name is declared twice") if $by_name{$name};
        _usage("Perldantic::Enum ($class): member $name: the value must be a string or a number")
            if !defined $value || ref $value;
        if (my $same = $by_value{$value}) {
            _usage("Perldantic::Enum ($class): members $name and $same->{name} have the same value");
        }
        my $member = bless {name => $name, value => $value}, $class;
        Hash::Util::lock_hash(%$member);
        push @members, $member;
        $by_name{$name} = $by_value{$value} = $member;
    }
    $CLASSES{$class} = {
        members  => \@members,
        by_name  => \%by_name,
        by_value => \%by_value,
        sub_type => _sub_type(map { $_->{value} } @members),
    };
    no strict 'refs';
    push @{"${class}::ISA"}, __PACKAGE__ if !$class->isa(__PACKAGE__);
    for my $member (@members) {
        no warnings 'redefine';
        *{"${class}::$member->{name}"} = sub { $member };
    }
    return;
}

# How input is matched against the values (core `sub_type`): integers alone take numeric
# strings, and so on; values of mixed kinds are matched as they are.
sub _sub_type (@values) {
    my %kinds;
    for my $value (@values) {
        my $kind = !builtin::created_as_number($value) ? 'str'
            : $value == int($value) && $value !~ /[.eE]/ ? 'int'
            : 'float';
        $kinds{$kind} = 1;
    }
    delete $kinds{int} if $kinds{float};
    return keys %kinds == 1 ? (keys %kinds)[0] : undef;
}

sub _enum ($class) {
    my $enum = $CLASSES{ref $class || $class};
    _usage("$class is not a Perldantic enum class") if !$enum;
    return $enum;
}

sub members ($class)             { @{_enum($class)->{members}} }
sub from_name ($class, $name)   { defined $name ? _enum($class)->{by_name}{$name} : undef }
sub from_value ($class, $value) { defined $value && !ref $value ? _enum($class)->{by_value}{$value} : undef }

sub name ($self)  { $self->{name} }
sub value ($self) { $self->{value} }

sub _is_enum ($class) { !ref $class && defined $class && exists $CLASSES{$class} }

# The member of a class the core returned, by name; members of classes that are not (or no
# longer) declared stay as the core describes them.
sub _from_wire ($member) {
    my $enum = $CLASSES{$member->{class} // ''};
    my $found = $enum && defined $member->{name} && $enum->{by_name}{$member->{name}};
    return $found || Perldantic::Wire::Enum->new(%$member);
}

sub _perldantic_wire ($self) {
    my $sub_type = _enum($self)->{sub_type};
    return Perldantic::Wire::Enum->new(
        class        => ref $self,
        name         => $self->{name},
        value        => $self->{value},
        mixin        => $sub_type,
        str_is_value => 1,
    );
}

# The core `enum` schema of a class.
sub _core_schema ($class, $ref) {
    my $enum = _enum($class);
    return {
        type    => 'enum',
        cls     => $class,
        members => [map { $_->_perldantic_wire } @{$enum->{members}}],
        (defined $enum->{sub_type} ? (sub_type => $enum->{sub_type}) : ()),
        ref     => $ref,
    };
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Enum - enum classes: a fixed set of named members

=head1 SYNOPSIS

    package Shop::Color {
        use Perldantic::Enum RED => 'red', GREEN => 'green', BLUE => 'blue';
    }

    package Shop::State {
        use Perldantic::Enum [qw(open in_progress done)];   # each name is its own value
    }

    package Shop::Shirt {
        use Perldantic;
        has color => (is => 'ro', isa => 'Shop::Color', required => 1);
    }

    my $shirt = Shop::Shirt->new(color => 'green');
    $shirt->color;                         # Shop::Color->GREEN, the member object
    $shirt->color->name;                   # 'GREEN'
    $shirt->color->value;                  # 'green'
    "$shirt->{color}";                     # 'green'
    $shirt->model_dump(mode => 'json');    # {color => 'green'}

=head1 DESCRIPTION

pydantic validates Python C<Enum> classes; C<use Perldantic::Enum> declares the Perl
equivalent in the current package. Each member is one object of the class, reached with a
class method named after it (C<< Shop::Color->RED >>), and the class inherits from
C<Perldantic::Enum>.

A class name of an enum class is a type (C<< isa => 'Shop::Color' >>, C<ArrayRef['Shop::Color']>,
C<< Perldantic::TypeAdapter->new('Shop::Color') >>), validated by the core C<enum> schema:

=over

=item * a member of the class is kept as it is;

=item * any other input is looked up among the values, and the member with that value is
returned. When every value is an integer (a number, as written in the declaration), numeric
strings match too, as for pydantic's C<IntEnum>; likewise for floats and for strings (C<StrEnum>).
Values of mixed kinds are matched as given;

=item * in strict mode Perl input must be a member; JSON input is always a value.

=back

Anything else fails with an error of type C<enum> listing the values. C<model_dump> keeps
members; C<< mode => 'json' >> and C<model_dump_json> write their values. In JSON Schema an
enum class is a definition with its values under C<enum>.

=head1 DECLARING

    use Perldantic::Enum NAME => $value, ...;
    use Perldantic::Enum [qw(name ...)];

Names are Perl identifiers and values defined strings or numbers; names and values must be
unique within a class (pydantic's aliases, members sharing a value, are not supported).
Mistakes raise C<Perldantic::UsageError>.

=head1 METHODS

=head2 members

The members in declaration order.

=head2 from_name($name), from_value($value)

The member with that name or value, or C<undef> when there is none. Validate with a
L<Perldantic::TypeAdapter> to raise an error instead.

=head2 name, value

A member's name and value. A member reads as its value in string and numeric contexts, so
C<< $color eq 'red' >> works; members are always true, and cannot be changed.

=head1 SEE ALSO

L<Perldantic>, L<Perldantic::Types>, L<Perldantic::TypeAdapter>

=cut
