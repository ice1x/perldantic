# Migrating from pydantic

This guide is for people who know pydantic v2 and are writing Perl with Perldantic. The engine is
a Python-free port of `pydantic-core`, so validation rules, lax/strict conversions, error codes,
serialization and JSON Schema follow pydantic's. What changes is the surface: classes are
declared the Moo way, types have Perl names, and errors use Perl's words for Perl data.

Every Perl example below runs as written (`t/37-migrating-guide.t` checks them). Intentional
behaviour differences are listed in [DIVERGENCES.md](DIVERGENCES.md).

## Contents

1. [Models](#models)
2. [Fields](#fields)
3. [Types](#types)
4. [Constraints](#constraints)
5. [Validators](#validators)
6. [Serialization](#serialization)
7. [Model config](#model-config)
8. [Model methods](#model-methods)
9. [TypeAdapter](#typeadapter)
10. [Errors](#errors)
11. [Unions](#unions)
12. [Dates, UUIDs, URLs and decimals](#dates-uuids-urls-and-decimals)
13. [Enum classes](#enum-classes)
14. [Not available](#not-available)

## Models

A pydantic model is a class that inherits from `BaseModel`:

```python
from pydantic import BaseModel

class User(BaseModel):
    id: int
    name: str
    tags: list[str] = []

user = User(id='42', name='Ann')
```

A Perldantic model is a package that says `use Perldantic` instead of `use Moo`. Fields are
declared with `has`, and the types come with it:

```perl
package User;
use Perldantic;

has id   => (is => 'ro', isa => Int, required => 1);
has name => (is => 'ro', isa => Str, required => 1);
has tags => (is => 'ro', isa => ArrayRef[Str], default => sub { [] });

package main;
my $user = User->new(id => '42', name => 'Ann');   # id is now the number 42
say $user->id;
```

`new` validates exactly as pydantic's constructor does and raises a
`Perldantic::ValidationError` on invalid input. It takes a list of pairs or a hash reference.

Inheritance is `extends`, and `with` consumes roles (`Perldantic::Role`, a Moo-style role that
can declare fields). `BUILD` runs after validation, as pydantic's `model_post_init` does, and
the method modifiers `before`, `after` and `around` are available:

```perl
package Animal;
use Perldantic;
has name => (is => 'ro', isa => Str, required => 1);

package Dog;
use Perldantic;
extends 'Animal';
has good => (is => 'ro', isa => Bool, default => !!1);   # defaults are not validated

sub BUILD ($self, $args) { $self->{greeting} = 'Woof, ' . $self->name }

package main;
my $dog = Dog->new(name => 'Rex');
say $dog->{greeting};
say $dog->model_dump_json;          # {"name":"Rex","good":true}
```

## Fields

The main difference: **a pydantic field without a default is required; a Perldantic field is
optional unless it says `required => 1`**, as in Moo. A field that is neither given nor
defaulted is absent from the object: its accessor returns `undef`, and dumps leave it out.

| pydantic | Perldantic |
|---|---|
| `name: str` | `has name => (is => 'ro', isa => Str, required => 1)` |
| `name: str = 'x'` | `has name => (is => 'ro', isa => Str, default => 'x')` |
| `tags: list[str] = Field(default_factory=list)` | `has tags => (..., isa => ArrayRef[Str], default => sub { [] })` |
| `nick: str \| None = None` | `has nick => (..., isa => Maybe[Str], default => undef)` |
| `Field(alias='userName')` | `alias => 'userName'` |
| `Field(gt=0, max_length=10, ...)` | `gt => 0, max_length => 10, ...` (see [Constraints](#constraints)) |
| `Field(validate_default=True)` | `validate_default => 1` (plain defaults only) |
| `Field(strict=True)` | `strict => 1` |
| `model_config = ConfigDict(frozen=True)` | `is => 'ro'` on every field |

`is` is required by Moo's conventions: `ro`, `rw`, `rwp`, `lazy` or `bare`. Moo's `builder`,
`lazy`, `predicate`, `clearer`, `init_arg`, `trigger` and `documentation` work too. Writing a
field through an `rw` accessor does not validate the value (pydantic's `validate_assignment`
is not available).

```perl
package Account;
use Perldantic;

has id    => (is => 'ro', isa => Int, required => 1, gt => 0);
has name  => (is => 'ro', isa => Str, required => 1, alias => 'userName');
has email => (is => 'ro', isa => Maybe[Str], default => undef);
has age   => (is => 'ro', isa => Int, ge => 0);
model_config validate_by_name => 1;

package main;
my $a = Account->new(id => 1, userName => 'ann');
say defined $a->age ? 'has an age' : 'no age';           # no age
say $a->model_dump_json;                                  # {"id":1,"name":"ann","email":null}
say $a->model_dump_json(by_alias => 1);                   # {"id":1,"userName":"ann","email":null}
say Account->new(id => 2, name => 'bob')->name;           # by name, as validate_by_name allows
```

## Types

Perldantic uses the names and meanings of Perl's Types::Standard, not Python's. Import them
with `use Perldantic` (in a model) or `use Perldantic::Types qw(...)` (anywhere else;
`qw(:all)` imports every type).

| Python / pydantic | Perldantic | Notes |
|---|---|---|
| `Any` | `Any` | |
| `None` | `Undef` | |
| `bool` | `Bool` | |
| `int` | `Int` | any size; big values are `Math::BigInt` |
| `float` | `Num` | |
| `str` | `Str` | |
| `bytes` | `Bytes` | |
| `Decimal` | `Decimal` | `Math::BigFloat` values |
| `date`, `time`, `datetime`, `timedelta` | `Date`, `Time`, `DateTime`, `Duration` | `Perldantic::Temporal` values |
| `UUID`, `UUID4` | `Uuid`, `Uuid->with(version => 4)` | `Perldantic::Uuid` values |
| `AnyUrl`, `MultiHostUrl` | `Url`, `MultiHostUrl` | `Perldantic::Url` values |
| `T \| None`, `Optional[T]` | `Maybe[T]` | Perldantic's `Optional[T]` means something else, see below |
| `list[T]` | `ArrayRef[T]` | |
| `tuple[A, B]` | `Tuple[A, B]` | |
| `tuple[T, ...]` | `Tuple[slurpy ArrayRef[T]]` | |
| `set[T]`, `frozenset[T]` | `Set[T]` | an array reference of distinct items |
| `dict[str, V]` | `HashRef[V]` | |
| `dict[K, V]` | `Map[K, V]` | keys validated as `K` |
| `TypedDict` | `Dict[name => T, ...]` | `Optional[T]` marks a key that may be missing |
| `Literal['a', 1]` | `Literal['a', 1]` | |
| `Literal['a', 'b']` used as an enum | `Enum[qw(a b)]` | plain strings |
| an `Enum`, `IntEnum` or `StrEnum` class `E` | `'E'` (a class declared with `use Perldantic::Enum`) | see [Enum classes](#enum-classes) |
| `Union[A, B]`, `A \| B` | `A \| B`, `AnyOf[A, B]` | |
| `Json[T]` | `Json[T]` | |
| a model class `M` | `'M'` (the class name) | also `ArrayRef['M']`, `Maybe['M']`, ... |
| an arbitrary class (`arbitrary_types_allowed`) | `InstanceOf['Class']` | |
| `Annotated[T, AfterValidator(f)]` chains | `Chain[A, B]`, or a `field_validator` | |

A Type::Tiny constraint (for example from Types::Standard) can be used wherever a type is
expected; it runs in Perl and reports a `value_error`.

```perl
use Perldantic::TypeAdapter;
use Perldantic::Types qw(:all);

my $point = Perldantic::TypeAdapter->new(Tuple[Num, Num]);
my $ints  = Perldantic::TypeAdapter->new(Tuple[slurpy ArrayRef[Int]]);
my $index = Perldantic::TypeAdapter->new(Map[Int, Str]);
my $row   = Perldantic::TypeAdapter->new(Dict[id => Int, note => Optional[Str]]);

say join ',', @{ $point->validate(['1.5', 2]) };           # 1.5,2
say join ',', @{ $ints->validate([1, '2', 3]) };           # 1,2,3
say join ',', sort keys %{ $index->validate({1 => 'a'}) }; # 1
say $row->validate({id => '7'})->{id};                     # 7; note may be missing
say scalar @{ Perldantic::TypeAdapter->new(Set[Int])->validate([1, 1, 2]) };   # 2
```

## Constraints

pydantic puts constraints in `Field(...)`, in `Annotated[...]` or in `con*` functions. In
Perldantic they are options of `has`, or arguments of a type's `with` method:

| pydantic | Perldantic |
|---|---|
| `Field(gt=0)` | `has n => (..., isa => Int, gt => 0)` |
| `Annotated[int, Field(gt=0)]`, `conint(gt=0)`, `PositiveInt` | `Int->with(gt => 0)` |
| `constr(pattern=r'^\d+$', max_length=5)` | `Str->with(pattern => '^\d+$', max_length => 5)` |
| `conlist(int, min_length=1)` | `(ArrayRef[Int])->with(min_length => 1)` |
| `StrictInt`, `StrictStr`, ... | `Int->with(strict => 1)`, `Str->with(strict => 1)`, ... |
| `condecimal(max_digits=5, decimal_places=2)` | `Decimal->with(max_digits => 5, decimal_places => 2)` |
| `HttpUrl` | `Url->with(allowed_schemes => ['http', 'https'])` |

The constraint names are pydantic's. A constraint that does not apply to the type raises a
`Perldantic::UsageError` at declaration time. As with Type::Tiny, put a parameterized type in
parentheses before calling a method on it: `(ArrayRef[Int])->with(...)`.

```perl
use Perldantic::TypeAdapter;
use Perldantic::Types qw(Int Str ArrayRef);

my $code  = Perldantic::TypeAdapter->new(Str->with(pattern => '^\d+$', max_length => 5));
my $batch = Perldantic::TypeAdapter->new((ArrayRef[Int])->with(min_length => 1));

say $code->validate('123');
eval { $batch->validate([]) };
say $@->errors->[0]{type};                                 # too_short
eval { Perldantic::TypeAdapter->new(Int->with(strict => 1))->validate('5') };
say $@->errors->[0]{type};                                 # int_type
```

## Validators

The decorators become declarations with the same names and modes. Validators are called as
class methods, so the first argument is the class; a validator that takes one more argument
gets an `info` object (`Perldantic::ValidationInfo`, with `data`, `field_name`, `context`,
`mode` and `config`).

```python
class Range(BaseModel):
    lo: int
    hi: int

    @field_validator('hi')
    @classmethod
    def check_hi(cls, v, info):
        if v < info.data['lo']:
            raise ValueError('hi is below lo')
        return v

    @model_validator(mode='before')
    @classmethod
    def from_pair(cls, data):
        return {'lo': data[0], 'hi': data[1]} if isinstance(data, list) else data
```

```perl
package Range;
use Perldantic;

has lo => (is => 'ro', isa => Int, required => 1);
has hi => (is => 'ro', isa => Int, required => 1);

field_validator hi => sub ($class, $value, $info) {
    die "hi is below lo\n" if $value < $info->data->{lo};
    return $value;
};

model_validator mode => 'before', sub ($class, $data, $info) {
    return ref $data eq 'ARRAY' ? {lo => $data->[0], hi => $data->[1]} : $data;
};

model_validator mode => 'after', sub ($self, $info) {
    die "the range is too wide\n" if $self->hi - $self->lo > 100;
    return $self;
};

package main;
say Range->model_validate([1, 5])->hi;                     # 5
eval { Range->new(lo => 5, hi => 1) };
say $@->errors->[0]{msg};                                  # Value error, hi is below lo
```

- `field_validator` takes `mode => 'after'` (the default), `before`, `plain` or `wrap`, and one
  field name or an array reference of names. A `wrap` validator gets `$handler` before `$info`.
- `model_validator` takes `mode => 'before'`, `after` or `wrap`.
- Dying with a string is pydantic's `raise ValueError(...)`: a `value_error` with that message.
  Pydantic's other exceptions have Perl classes (see [Errors](#errors)).

## Serialization

`field_serializer`, `model_serializer` and `computed_field` follow pydantic's decorators.
Serializers are called as object methods; one that takes one more argument gets a
`Perldantic::SerializationInfo`.

```python
class Invoice(BaseModel):
    net: Decimal
    tax: Decimal

    @field_serializer('net', 'tax', when_used='json')
    def as_cents(self, v):
        return int(v * 100)

    @computed_field
    @property
    def total(self) -> Decimal:
        return self.net + self.tax
```

```perl
package Invoice;
use Perldantic;

has net => (is => 'ro', isa => Decimal, required => 1);
has tax => (is => 'ro', isa => Decimal, required => 1);

field_serializer [qw(net tax)] => (when_used => 'json') => sub ($self, $value) {
    return ($value * 100)->as_int;               # a Math::BigInt, written as a JSON number
};

computed_field total => (isa => Decimal) => sub ($self) { $self->net + $self->tax };

package main;
my $invoice = Invoice->new(net => '10.50', tax => '2.10');
say $invoice->model_dump_json;             # {"net":1050,"tax":210,"total":"12.6"}
say $invoice->total;                       # 12.6
```

Decimals are `Math::BigFloat` objects, which drop trailing zeros once computed with: the
total is `12.6`, where pydantic writes `12.60` (DIVERGENCES.md #18). A validated value that has
not changed keeps its digits.

`when_used` takes pydantic's values in Perl's words: `always`, `unless-undef` (pydantic's
`unless-none`), `json` and `json-unless-undef`.

## Model config

`model_config = ConfigDict(...)` becomes the `model_config` declaration, with pydantic's setting
names: `title`, `strict`, `extra` (`allow`, `ignore` or `forbid`), `str_strip_whitespace`,
`str_to_lower`, `str_to_upper`, `str_min_length`, `str_max_length`, `validate_by_name`,
`validate_by_alias`, `serialize_by_alias`, `revalidate_instances`, `validate_default`,
`url_preserve_empty_path` and `json_schema_extra` (a hash). `populate_by_name` is
`validate_by_name`. One setting is Perl's own: `temporal_class` (`Perldantic`, `DateTime` or
`Time::Moment`) picks the classes dates and times validate into.

```perl
package Tag;
use Perldantic;

has name => (is => 'ro', isa => Str, required => 1);
model_config extra => 'allow', str_strip_whitespace => 1, str_to_lower => 1;

package main;
my $tag = Tag->new(name => '  Perl ', colour => 'blue');
say $tag->name;                            # perl
say $tag->model_extra->{colour};           # blue
```

## Model methods

The `model_*` methods keep pydantic's names and options. Option values that name Python types
use Perl's words:

| pydantic | Perldantic |
|---|---|
| `Model(**data)` | `Model->new(%data)` or `Model->new(\%data)` |
| `Model.model_validate(data, strict=..., context=...)` | `Model->model_validate($data, strict => ..., context => ...)` |
| `Model.model_validate_json(text)` | `Model->model_validate_json($text)` |
| `m.model_dump()` | `$m->model_dump` (Perl data) |
| `m.model_dump(mode='python')` | `$m->model_dump(mode => 'perl')` (the default) |
| `m.model_dump(mode='json')` | `$m->model_dump(mode => 'json')` (JSON-compatible Perl data) |
| `m.model_dump(exclude_none=True)` | `$m->model_dump(exclude_undef => 1)` |
| `include={'a'}`, `exclude={'a': {'b'}}` | `include => ['a']`, `exclude => {a => {b => 1}}` |
| `m.model_dump_json(indent=2)` | `$m->model_dump_json(indent => 2)` (UTF-8 encoded bytes) |
| `Model.model_json_schema()` | `Model->model_json_schema` (Perl data) |
| `m.model_copy(update={...}, deep=True)` | `$m->model_copy(update => {...}, deep => 1)` |
| `m.model_fields_set` | `$m->model_fields_set` (a sorted list of names) |
| `m.model_extra` | `$m->model_extra` |
| `isinstance(m, Model)` | `$m->isa('Model')` |

```perl
package Point;
use Perldantic;
has x => (is => 'ro', isa => Int, required => 1);
has y => (is => 'ro', isa => Maybe[Int]);

package main;
use JSON::PP ();

my $p = Point->model_validate_json('{"x": 1, "y": null}');
my $data = $p->model_dump(exclude_undef => 1);             # {x => 1}
my @set = $p->model_fields_set;                            # ('x', 'y')
my $moved = $p->model_copy(update => {x => 5});
say JSON::PP->new->canonical->encode(Point->model_json_schema);
```

## TypeAdapter

`TypeAdapter` validates and serializes any type, not just models. Its methods drop the
`_python` suffix, since the data is Perl's:

| pydantic | Perldantic |
|---|---|
| `TypeAdapter(list[int])` | `Perldantic::TypeAdapter->new(ArrayRef[Int])` |
| `ta.validate_python(data)` | `$ta->validate($data)` |
| `ta.validate_json(text)` | `$ta->validate_json($text)` |
| `ta.dump_python(value)` | `$ta->dump($value)` |
| `ta.dump_json(value)` | `$ta->dump_json($value)` |
| `ta.json_schema()` | `$ta->json_schema` |

```perl
use Perldantic::TypeAdapter;
use Perldantic::Types qw(HashRef ArrayRef Int);

my $ta = Perldantic::TypeAdapter->new(HashRef[ArrayRef[Int]]);
my $scores = $ta->validate({ann => ['1', 2]});             # {ann => [1, 2]}
say $ta->dump_json($scores);                               # {"ann":[1,2]}
my $schema = $ta->json_schema;       # {type => 'object', additionalProperties => {type => 'array', ...}}
```

## Errors

Perldantic dies with exception objects, never with strings. `Perldantic::ValidationError` is
pydantic's `ValidationError`, with the same methods in Perl form:

| pydantic | Perldantic |
|---|---|
| `except ValidationError as e:` | `if (blessed $@ && $@->isa('Perldantic::ValidationError'))` |
| `e.errors()` | `$e->errors` (an array reference of hashes: `type`, `loc`, `msg`, `input`, `ctx`) |
| `e.error_count()` | `$e->error_count` |
| `e.title` | `$e->title` |
| `e.json(indent=2)` | `$e->json(indent => 2)` |
| `str(e)` | `"$e"` |

Other failures have their own classes: `Perldantic::SchemaError`, `Perldantic::UsageError` (the
API was called wrongly; it knows the caller's `file` and `line`),
`Perldantic::SerializationError` and `Perldantic::InternalError`, all subclasses of
`Perldantic::Error`.

For Perl data, errors use Perl's words: the codes, messages and input descriptions that name a
Python type are replaced, and there are no links to pydantic's documentation. Every other code
is pydantic's.

| pydantic code | Perldantic code |
|---|---|
| `dict_type`, `mapping_type` | `hash_type` |
| `list_type`, `tuple_type`, `set_type`, `frozen_set_type`, `iterable_type` | `array_type` |
| `none_required` | `undef_required` |
| `float_type`, `float_parsing` | `number_type`, `number_parsing` |
| `int_from_float` | `int_from_fraction` |
| `time_delta_type`, `time_delta_parsing` | `duration_type`, `duration_parsing` |
| `callable_type` | `code_type` |

```perl
use Scalar::Util qw(blessed);

package Order;
use Perldantic;
has id    => (is => 'ro', isa => Int, required => 1);
has items => (is => 'ro', isa => ArrayRef[Str], required => 1);

package main;
eval { Order->new(id => 'x', items => {a => 1}) };
my $e = $@;
if (blessed $e && $e->isa('Perldantic::ValidationError')) {
    say $e->error_count;                                   # 2
    say join ' ', map { $_->{type} } @{ $e->errors };      # int_parsing array_type
    print "$e";
    # 2 validation errors for Order
    # id
    #   Input should be a valid integer, unable to parse string as an integer [type=int_parsing, input_value='x', input_type=Str]
    # items
    #   Input should be an array reference [type=array_type, input_value={a => 1}, input_type=HashRef]
}
```

Validators raise pydantic's special exceptions through these classes:

| pydantic | Perldantic |
|---|---|
| `raise ValueError('msg')` | `die "msg\n"` |
| `PydanticCustomError('code', 'template {x}', {'x': 1})` | `Perldantic::CustomError->throw(type => 'code', message => 'template {x}', context => {x => 1})` |
| `PydanticKnownError('greater_than', {'gt': 5})` | `Perldantic::KnownError->throw(type => 'greater_than', context => {gt => 5})` |
| `PydanticOmit` | `Perldantic::Omit->throw` |
| `PydanticUseDefault` | `Perldantic::UseDefault->throw` |

```perl
package Even;
use Perldantic;
has n => (is => 'ro', isa => Int, required => 1);

field_validator n => sub ($class, $n) {
    Perldantic::CustomError->throw(type => 'not_even', message => '{n} is odd', context => {n => $n})
        if $n % 2;
    return $n;
};

package main;
eval { Even->new(n => 3) };
say $@->errors->[0]{type}, ': ', $@->errors->[0]{msg};     # not_even: 3 is odd
```

## Unions

`A | B` works on Perldantic types too, and `AnyOf[...]` takes class names. Unions use
pydantic's smart mode. A discriminated union is `->with(discriminator => $field)`, where each
alternative is a model or a `Dict[]` whose field has `Literal[]` or `Enum[]` values. The
alternatives may be models declared later in the file.

```python
class Owner(BaseModel):
    pet: Annotated[Cat | Dog, Field(discriminator='kind')]
```

```perl
package Owner;
use Perldantic;
has pet => (is => 'ro', isa => (AnyOf['Cat', 'Dog'])->with(discriminator => 'kind'), required => 1);
has id  => (is => 'ro', isa => Int | Str);

package Cat;
use Perldantic;
has kind => (is => 'ro', isa => Literal['cat'], required => 1);

package Dog;
use Perldantic;
has kind => (is => 'ro', isa => Literal['dog'], required => 1);

package main;
say ref Owner->new(pet => {kind => 'dog'}, id => 'a1')->pet;   # Dog
eval { Owner->new(pet => {kind => 'fish'}) };
say $@->errors->[0]{type};                                     # union_tag_invalid
```

## Dates, UUIDs, URLs and decimals

Values that Python keeps in library classes validate into Perl classes that keep what pydantic
keeps:

| Python | Perldantic type | Validated value |
|---|---|---|
| `datetime.date`, `time`, `datetime`, `timedelta` | `Date`, `Time`, `DateTime`, `Duration` | `Perldantic::Date`, `::Time`, `::DateTime`, `::Duration` |
| `uuid.UUID` | `Uuid` | `Perldantic::Uuid` |
| `pydantic.AnyUrl`, `MultiHostUrl` | `Url`, `MultiHostUrl` | `Perldantic::Url`, `Perldantic::MultiHostUrl` |
| `decimal.Decimal` | `Decimal` | `Math::BigFloat` |

They stringify to their ISO 8601 or text form and compare with Perl's operators. Input may also
be `DateTime`, `Time::Moment`, `DateTime::Duration` and `URI` objects, and
`model_config temporal_class => 'DateTime'` makes a model return `DateTime` objects instead.

```perl
package Event;
use Perldantic;
has at    => (is => 'ro', isa => DateTime, required => 1);
has id    => (is => 'ro', isa => Uuid);
has site  => (is => 'ro', isa => Url);
has price => (is => 'ro', isa => Decimal);

package main;
my $event = Event->new(
    at    => '2024-01-02T03:04:05Z',
    id    => '0e7ac198-9acd-4c0c-b4b4-761974bf71d7',
    site  => 'https://example.com',
    price => '1.10',
);
say $event->at->epoch;                     # 1704164645
say $event->id->version;                   # 4
say $event->site;                          # https://example.com/
say $event->model_dump_json;
# {"at":"2024-01-02T03:04:05Z","id":"0e7ac198-9acd-4c0c-b4b4-761974bf71d7","site":"https://example.com/","price":"1.10"}
```

## Enum classes

`use Perldantic::Enum` declares an enum class in the current package, with its members as
name => value pairs. The class name is then a type; input is looked up by value, and the field
holds the member, one object per member that reads as its value. There is no separate
`IntEnum` or `StrEnum`: when every value is an integer, numeric strings match as they do for
`IntEnum`, and likewise for strings and floats.

```python
class Size(IntEnum):
    S = 1
    M = 2
    L = 3

class Shirt(BaseModel):
    size: Size = Size.M
```

```perl
package Size;
use Perldantic::Enum S => 1, M => 2, L => 3;

package Shirt;
use Perldantic;
has size => (is => 'ro', isa => 'Size', default => Size->M);

package main;
my $shirt = Shirt->new(size => '3');
say $shirt->size->name;                        # L
say $shirt->size == Size->L ? 'large' : '';    # large
say $shirt->model_dump_json;                   # {"size":3}
say Size->from_value(1)->name;                 # S
```

`model_dump` keeps members, and `mode => 'json'` writes their values. Strict validation
takes only members from Perl data (JSON input is always a value).

## Not available

These pydantic features have no Perldantic counterpart yet:

- `validate_assignment`, `frozen` models (use `is => 'ro'`), `model_construct`,
  `model_rebuild` (not needed: models are compiled on first use and recompiled when a class
  they use changes);
- generic models, dataclasses, `@validate_call`, `RootModel`;
- enum member aliases (two names for one value) and `use_enum_values`;
- `SecretStr`, `EmailStr` and other types from `pydantic.networks` and `pydantic.types` beyond
  those listed above (a `pattern` constraint or a Type::Tiny constraint covers most of them);
- `json_schema_extra` functions and custom `GenerateJsonSchema` classes;
- the Moo `has` options `coerce`, `handles`, `weak_ref`, `reader` and `writer`.

Python-only core types (`complex`, `fraction`, `named-tuple`, `dataclass`, `deque`, ...) are
rejected; see [DIVERGENCES.md](DIVERGENCES.md) for this and the other intentional differences.
