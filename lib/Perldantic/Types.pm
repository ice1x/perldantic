package Perldantic::Types;

use v5.36;

our $VERSION = '0.01';

use Exporter 'import';
use Scalar::Util qw(blessed);

use Perldantic::Error;
use Perldantic::Type;
use Perldantic::Wire;

my @SIMPLE = qw(Any Undef Bool Int Num Str Bytes Decimal Date Time DateTime Duration Uuid Url MultiHostUrl);
my @PARAMETERIZED = qw(Maybe Optional ArrayRef Set FrozenSet Json Chain Tuple HashRef Map Dict Enum Literal InstanceOf);

our @EXPORT_OK   = (@SIMPLE, @PARAMETERIZED, 'slurpy');
our %EXPORT_TAGS = (all => \@EXPORT_OK);

my %CORE_TYPE = (
    Any => 'any', Undef => 'none', Bool => 'bool', Int => 'int', Num => 'float', Str => 'str',
    Bytes => 'bytes',
);

sub _usage ($message) { Perldantic::UsageError->throw(message => $message) }

sub Any ()   { Perldantic::Type->new(name => 'Any',   schema => {type => 'any'}) }
sub Undef () { Perldantic::Type->new(name => 'Undef', schema => {type => 'none'}) }
sub Bool ()  { Perldantic::Type->new(name => 'Bool',  schema => {type => 'bool'}) }
sub Int ()   { Perldantic::Type->new(name => 'Int',   schema => {type => 'int'}) }
sub Num ()   { Perldantic::Type->new(name => 'Num',   schema => {type => 'float'}) }
sub Str ()   { Perldantic::Type->new(name => 'Str',   schema => {type => 'str'}) }
sub Bytes () { Perldantic::Type->new(name => 'Bytes', schema => {type => 'bytes'}) }
sub Decimal () { Perldantic::Type->new(name => 'Decimal', schema => {type => 'decimal'}) }
sub Date ()  { Perldantic::Type->new(name => 'Date',  schema => {type => 'date'}) }
sub Time ()  { Perldantic::Type->new(name => 'Time',  schema => {type => 'time'}) }
# Inside a package that imports this type, call the DateTime class as `DateTime::->new`.
sub DateTime () { Perldantic::Type->new(name => 'DateTime', schema => {type => 'datetime'}) }
sub Duration () { Perldantic::Type->new(name => 'Duration', schema => {type => 'timedelta'}) }
sub Uuid ()     { Perldantic::Type->new(name => 'Uuid',     schema => {type => 'uuid'}) }
sub Url ()      { Perldantic::Type->new(name => 'Url',      schema => {type => 'url'}) }
sub MultiHostUrl () { Perldantic::Type->new(name => 'MultiHostUrl', schema => {type => 'multi-host-url'}) }

# The parameters of `Name[...]`, or undef for a bare `Name`.
sub _params ($name, @args) {
    return undef if !@args;
    _usage("$name takes its parameters as $name\[...]") if @args > 1 || ref $args[0] ne 'ARRAY';
    return $args[0];
}

sub _count ($name, $params, $count) {
    my $got = @$params;
    _usage("$name\[] takes $count parameter" . ($count == 1 ? '' : 's') . ", got $got")
        if $got != $count;
}

# Check a type parameter; Optional[] is only meaningful where a slot may be left out.
sub _type ($name, $param, $optional_ok = 0) {
    # A class name stands for InstanceOf[class], as in `isa => 'Class'`.
    $param = InstanceOf([$param]) if defined $param && !ref $param && $param =~ /\A[A-Za-z_]\w*(?:::\w+)+\z|\A[A-Z]\w*\z/;
    _usage("$name\[] takes a type, got " . ($param // 'undef'))
        if !blessed $param || !$param->isa('Perldantic::Type');
    _usage('Optional[] is only supported inside Dict[]') if $param->is_optional && !$optional_ok;
    _usage("$name\[] takes slurpy only as its last parameter") if $param->is_slurpy;
    return $param;
}

sub _list ($params) { join ',', map { $_->name } @$params }

sub _quote ($value) {
    return 'undef' if !defined $value;
    return $value if $value =~ /\A-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?\z/;
    return '"' . $value =~ s/(["\\])/\\$1/gr . '"';
}

sub Maybe :prototype(;$) (@args) {
    my $params = _params('Maybe', @args) // return &Maybe([Any()]);
    _count('Maybe', $params, 1);
    my $inner = _type('Maybe', $params->[0]);
    return Perldantic::Type->new(
        name   => "Maybe[$inner]",
        inner  => $inner,
        wrap   => sub ($schema) { {type => 'nullable', schema => $schema} },
    );
}

sub Optional :prototype(;$) (@args) {
    my $params = _params('Optional', @args) // return &Optional([Any()]);
    _count('Optional', $params, 1);
    my $inner = _type('Optional', $params->[0]);
    return Perldantic::Type->new(
        name     => "Optional[$inner]",
        inner    => $inner,
        wrap     => sub ($schema) {$schema},
        optional => 1,
    );
}

sub ArrayRef :prototype(;$) (@args) {
    my $params = _params('ArrayRef', @args)
        // return Perldantic::Type->new(name => 'ArrayRef', schema => {type => 'list'});
    _count('ArrayRef', $params, 1);
    my $items = _type('ArrayRef', $params->[0]);
    return Perldantic::Type->new(
        name       => "ArrayRef[$items]",
        schema     => {type => 'list', items_schema => $items->core_schema},
        parameters => [$items],
    );
}

# `Set[T]` and `FrozenSet[T]`: arrays of distinct items (core `set` / `frozenset`).
sub _set ($name, $core, @args) {
    my $params = _params($name, @args) // return Perldantic::Type->new(name => $name, schema => {type => $core});
    _count($name, $params, 1);
    my $items = _type($name, $params->[0]);
    return Perldantic::Type->new(
        name       => "$name\[$items]",
        schema     => {type => $core, items_schema => $items->core_schema},
        parameters => [$items],
    );
}

sub Set :prototype(;$) (@args)       { _set('Set', 'set', @args) }
sub FrozenSet :prototype(;$) (@args) { _set('FrozenSet', 'frozenset', @args) }

# `Json[T]`: JSON text, parsed and validated as T (core `json`).
sub Json :prototype(;$) (@args) {
    my $params = _params('Json', @args) // return Perldantic::Type->new(name => 'Json', schema => {type => 'json'});
    _count('Json', $params, 1);
    my $inner = _type('Json', $params->[0]);
    return Perldantic::Type->new(
        name       => "Json[$inner]",
        schema     => {type => 'json', schema => $inner->core_schema},
        parameters => [$inner],
    );
}

# `Chain[A, B, ...]`: each type validates the output of the previous one (core `chain`).
sub Chain :prototype(;$) (@args) {
    my $params = _params('Chain', @args) // _usage('Chain[] takes at least 1 type');
    _usage('Chain[] takes at least 1 type') if !@$params;
    my @steps = map { _type('Chain', $_) } @$params;
    return Perldantic::Type->new(
        name       => 'Chain[' . _list(\@steps) . ']',
        schema     => {type => 'chain', steps => [map { $_->core_schema } @steps]},
        parameters => \@steps,
    );
}

sub HashRef :prototype(;$) (@args) {
    my $params = _params('HashRef', @args)
        // return Perldantic::Type->new(name => 'HashRef', schema => {type => 'dict', keys_schema => {type => 'str'}});
    _count('HashRef', $params, 1);
    my $values = _type('HashRef', $params->[0]);
    return Perldantic::Type->new(
        name   => "HashRef[$values]",
        schema => {type => 'dict', keys_schema => {type => 'str'}, values_schema => $values->core_schema},
        parameters => [$values],
    );
}

sub Map :prototype(;$) (@args) {
    my $params = _params('Map', @args) // return Perldantic::Type->new(name => 'Map', schema => {type => 'dict'});
    _count('Map', $params, 2);
    my ($keys, $values) = map { _type('Map', $_) } @$params;
    return Perldantic::Type->new(
        name   => "Map[$keys,$values]",
        schema => {type => 'dict', keys_schema => $keys->core_schema, values_schema => $values->core_schema},
        parameters => [$keys, $values],
    );
}

sub slurpy :prototype($) ($type) {
    my $name = blessed $type && $type->isa('Perldantic::Type') ? $type->name : $type // 'undef';
    my ($kind) = $name =~ /\A(ArrayRef|HashRef)(?:\[|\z)/;
    _usage("slurpy takes ArrayRef, ArrayRef[T], HashRef or HashRef[T], got $name") if !$kind;
    my ($items) = $type->parameters;
    return Perldantic::Type->new(
        name   => "slurpy $name",
        schema => ($items ? $items->core_schema : {type => 'any'}),
        slurpy => $kind,
        # slurpy HashRef keeps unknown keys as they are
        typed  => !!$items,
    );
}

# Check that a trailing slurpy parameter is of the kind a container takes.
sub _slurpy_kind ($name, $param, $kind) {
    _usage("$name\[] takes slurpy $kind or $kind\[T], got $param") if $param->{slurpy} ne $kind;
    return $param;
}

sub _is_slurpy ($param) { blessed $param && $param->isa('Perldantic::Type') && $param->is_slurpy }

sub Tuple :prototype(;$) (@args) {
    my $params = _params('Tuple', @args) // return Perldantic::Type->new(
        name   => 'Tuple',
        schema => {type => 'tuple', items_schema => [{type => 'any'}], variadic_item_index => 0},
    );
    my @items = @$params;
    my $variadic;
    if (@items && _is_slurpy($items[-1])) {
        _slurpy_kind('Tuple', $items[-1], 'ArrayRef');
        $variadic = $#items;
    }
    _type('Tuple', $_) for defined $variadic ? @items[0 .. $variadic - 1] : @items;
    return Perldantic::Type->new(
        name   => 'Tuple[' . _list(\@items) . ']',
        schema => {
            type         => 'tuple',
            items_schema => [map { $_->core_schema } @items],
            (defined $variadic ? (variadic_item_index => $variadic) : ()),
        },
        parameters => \@items,
    );
}

sub Dict :prototype(;$) (@args) {
    my $params = _params('Dict', @args)
        // return Perldantic::Type->new(name => 'Dict', schema => {type => 'typed-dict', fields => {}});
    my @pairs = @$params;
    my $rest = @pairs % 2 && _is_slurpy($pairs[-1]) ? _slurpy_kind('Dict', pop @pairs, 'HashRef') : undef;
    _usage('Dict[] takes name => type pairs') if @pairs % 2;
    my (@fields, @names);
    for (my $i = 0; $i < @pairs; $i += 2) {
        my ($field, $type) = @pairs[$i, $i + 1];
        _usage("Dict[] takes a type for $field, got " . ($type // 'undef'))
            if !blessed $type || !$type->isa('Perldantic::Type');
        _type('Dict', $type, 1);
        push @names, "$field=>$type";
        # the core sees the fields in their declared order
        push @fields, $field => {
            type     => 'typed-dict-field',
            schema   => $type->core_schema,
            required => $type->is_optional ? !!0 : !!1,
        };
    }
    push @names, "$rest" if $rest;
    return Perldantic::Type->new(
        name   => 'Dict[' . join(',', @names) . ']',
        schema => {
            type   => 'typed-dict',
            fields => Perldantic::Wire::ordered(@fields),
            ($rest ? (extra_behavior => 'allow', ($rest->{typed} ? (extras_schema => $rest->core_schema) : ())) : ()),
        },
        parameters => [@$params],
    );
}

sub _literal ($name, $values, $check) {
    _usage("$name\[] takes at least 1 value") if !@$values;
    $check->($_) for @$values;
    return Perldantic::Type->new(
        name   => "$name\[" . join(',', map { _quote($_) } @$values) . ']',
        schema => {type => 'literal', expected => [@$values]},
    );
}

sub Enum :prototype(;$) (@args) {
    my $values = _params('Enum', @args) // _usage('Enum[] takes at least 1 value');
    return _literal('Enum', $values, sub ($v) {
        _usage('Enum[] takes strings, got ' . (ref $v || 'undef')) if !defined $v || ref $v;
    });
}

sub Literal :prototype(;$) (@args) {
    my $values = _params('Literal', @args) // _usage('Literal[] takes at least 1 value');
    return _literal('Literal', $values, sub ($v) {
        _usage('Literal[] takes plain values, got ' . ref $v) if ref $v;
    });
}

sub InstanceOf :prototype(;$) (@args) {
    my $params = _params('InstanceOf', @args) // _usage('InstanceOf[] takes a class name');
    _usage('InstanceOf[] takes a class name')
        if @$params != 1 || !defined $params->[0] || ref $params->[0];
    my $class = $params->[0];
    return Perldantic::Type->new(
        name   => 'InstanceOf[' . _quote($class) . ']',
        schema => {type => 'is-instance', cls => $class},
    );
}

1;

__END__

=pod

=encoding UTF-8

=head1 NAME

Perldantic::Types - Perldantic's type vocabulary, in Types::Standard terms

=head1 SYNOPSIS

    use Perldantic::Types qw(:all);

    my $tags   = ArrayRef[Str];
    my $point  = Tuple[Num, Num];
    my $status = Enum[qw(open in_progress done)];
    my $person = Dict[name => Str, age => Optional[Int]];
    my $count  = Int->with(ge => 0);

    my $schema = $tags->core_schema;    # {type => 'list', items_schema => {type => 'str'}}

=head1 DESCRIPTION

Every type is a L<Perldantic::Type> that knows its pydantic C<core_schema>. Names and meanings
follow Types::Standard. Python's names are not used, because several of them mean something else
in Perl (see F<docs/PLAN.md>, "Type vocabulary"). Nothing is exported by default: import names
one by one, or all of them with C<:all>.

As with Type::Tiny, C<Name[...]> is a function call on an array reference. To call a method on
a parameterized type, put it in a variable or in parentheses first: C<< (ArrayRef[Int])->with(...) >>.

=head1 TYPES

=over

=item C<Any>, C<Undef>, C<Bool>, C<Int>, C<Num>, C<Str>, C<Bytes>

Core C<any>, C<none>, C<bool>, C<int> (any size), C<float>, C<str> and C<bytes>.

=item C<Decimal>

Core C<decimal> (Python's C<Decimal>). It takes numbers, numeric strings and L<Math::BigFloat>
objects and validates into L<Math::BigFloat> objects, keeping every digit; in strict mode only
Math::BigFloat objects are accepted. Math::BigFloat values given to C<Num> or C<Int> are
converted as pydantic converts a C<Decimal>.

=item C<Date>, C<Time>, C<DateTime>, C<Duration>

Core C<date>, C<time>, C<datetime> and C<timedelta>. They take ISO 8601 text, numbers
(timestamps, or seconds for durations), L<DateTime>, L<Time::Moment> and L<DateTime::Duration>
objects, and validate into L<Perldantic::Temporal> values. A package that imports C<DateTime>
must call the L<DateTime> class as C<< DateTime::->new(...) >>, as with Types::DateTime.

=item C<Uuid>

Core C<uuid>. It takes UUID text (hyphenated or not), 16 bytes (C<Perldantic::Wire::bytes>) and
L<Perldantic::Uuid> objects, and validates into L<Perldantic::Uuid> values; in strict mode only
UUID objects are accepted.

=item C<Url>, C<MultiHostUrl>

Core C<url> and C<multi-host-url>. They take URL text, L<URI> objects and
L<Perldantic::Url> / L<Perldantic::MultiHostUrl> objects, and validate into the latter. A
model's C<url_preserve_empty_path> config (or the C<preserve_empty_path> constraint) keeps an
empty path empty instead of normalising it to C</>.

=item C<Maybe[T]>

C<T> or C<undef> (core C<nullable>; Python's C<Optional[T]>).

=item C<Optional[T]>

A C<Dict[]> field that may be left out. It is the Types::Standard meaning, not Python's; this
release supports it only inside C<Dict[]>.

=item C<ArrayRef>, C<ArrayRef[T]>

An array reference (core C<list>).

=item C<Set>, C<Set[T]>, C<FrozenSet>, C<FrozenSet[T]>

An array reference of distinct items (core C<set> / C<frozenset>): repeated items (by pydantic's
equality, so C<1> and C<1.0> are the same) are kept once, in the order given; items that could
not be in a Python set (array or hash references) are errors. Input may be any array
reference; output is an array reference.

=item C<Json>, C<Json[T]>

JSON text (a string or bytes) parsed into Perl data, which C<T> then validates as JSON input
(core C<json>). Serialization writes the data, not the text.

=item C<Chain[A, B, ...]>

Each type validates the output of the previous one (core C<chain>), e.g.
C<Chain[Str-E<gt>with(strip_whitespace =E<gt> 1), Json[ArrayRef[Int]]]>. Serialization and JSON
Schema follow pydantic: the last type for output, the first for validation schemas.

=item C<Tuple>, C<Tuple[A, B, ...]>, C<Tuple[A, slurpy ArrayRef[T]]>

A positional array (core C<tuple>). A trailing C<slurpy ArrayRef[T]> takes any number of further
C<T> items; bare C<Tuple> takes any items.

=item C<HashRef>, C<HashRef[V]>

A hash reference with string keys (core C<dict>).

=item C<Map>, C<Map[K, V]>

A hash reference whose keys are validated as C<K> (core C<dict>).

=item C<Dict[name =E<gt> T, ...]>, C<Dict[name =E<gt> T, ..., slurpy HashRef[T]]>

A hash reference with known keys (core C<typed-dict>), as in Types::Standard. Keys typed
C<Optional[T]> may be left out. Unknown keys are dropped, as in pydantic; a trailing
C<slurpy HashRef> keeps them and C<slurpy HashRef[T]> validates them as C<T>. The constraint
C<extra_behavior> (C<'ignore'>, C<'allow'> or C<'forbid'>) sets the behaviour directly.

=item C<Enum[...]>

One of the given strings (core C<literal>).

=item C<Literal[...]>

One of the given plain values, C<undef> included (core C<literal>).

=item C<InstanceOf['Class']>

An object of the class (core C<is-instance>). The core validates it once host classes are
supported (task 00056).

=back

=head2 slurpy

Marks the last parameter of C<Tuple[]> (C<slurpy ArrayRef[T]>) as variadic, or the unknown keys
of C<Dict[]> (C<slurpy HashRef[T]>) as allowed.

Wrong parameters, such as C<ArrayRef[1]> or C<Map[Int]>, raise C<Perldantic::UsageError>.

=cut
