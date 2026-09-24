# Perldantic — Project Plan

## 1. Goal

**Bring pydantic to Perl and keep its philosophy.** Take the Rust core of pydantic
(`pydantic-core`), remove its dependency on Python (pyo3), vendor it into this repository as a
standalone crate, and build a Perl library on top of it.

### Inherited pydantic philosophy (the project's contract)

1. **Declared types are the schema.** A model is described once, declaratively, and that
   description drives validation, serialization and JSON Schema.
2. **Parse, don't just validate.** The output is guaranteed to match the declared types. By
   default, lax mode coerces input (`"42"` → `42`). Strict mode is opt-in.
3. **Errors are data.** Validation reports every error it finds, not just the first. Each error
   is a structured record (`type`, `loc`, `msg`, `input`, `ctx`) with the same error codes and
   message templates as pydantic.
4. **Speed comes from a compiled core.** A schema is compiled once into a validator tree. After
   that, validation runs entirely in Rust.
5. **Validation and serialization mirror each other.** `model_dump` / `model_dump_json` follow
   the same schema, with `include`/`exclude`, aliases and `exclude_none`/`exclude_unset`/`exclude_defaults`.
6. **Standard interop.** Perldantic uses the same `CoreSchema` wire format as
   `pydantic_core.core_schema` and emits JSON Schema (Draft 2020-12).

**Compatibility rule:** if Perldantic behaves differently from pydantic for the same schema and
input, that is a bug. The only exceptions are deviations recorded in `docs/DIVERGENCES.md`.

## 2. Upstream analysis (`../pydantic`)

| Item | Value |
|---|---|
| Source | `github.com/pydantic/pydantic`, directory `pydantic-core/` (the core now lives in the monorepo) |
| Base commit | `0384c970e` — Catch `OverflowError` during `ByteSize` validation (#13850) |
| Core version | `pydantic-core 2.49.0`, edition 2024, MSRV 1.88 |
| Size | 141 `.rs` files, ~36k lines |
| License | MIT (keep LICENSE + attribution in `NOTICE`) |

How much of the code depends on pyo3 (count of `Bound<`, `Py<`, `PyAny`, `PyResult`, `py:`):

| Module | Lines | pyo3 refs |
|---|---|---|
| `validators/` | ~14,000 | ~820 |
| `serializers/` | ~12,200 | ~770 |
| `input/` | ~4,500 | ~210 |
| `errors/` | ~2,100 | ~70 |

Findings:

- **pyo3 is used throughout the core, not only at its boundary.** `Validator::validate` returns
  `ValResult<Py<PyAny>>`. `BuildValidator::build` reads its schema from `&Bound<PyDict>`.
  `ValError::InternalErr` wraps a `PyErr`, and the `'py` lifetime appears everywhere. Turning off
  a feature flag will not remove Python, so the core needs a controlled port.
- **The architecture itself does not depend on Python, and we keep it as is:** the `Input`
  trait (`strict_*` / `lax_*` per type), `CombinedValidator` with `enum_dispatch`,
  `DefinitionsBuilder` (recursive schemas), `ErrorType` / `ValLineError`, smart / tagged unions
  and `LookupKey` (aliases and paths).
- **Pure-Rust dependencies carry over unchanged:** `jiter` (without the `python` feature),
  `speedate`, `url`, `uuid`, `regex`, `num-bigint`, `base64`, `serde_json`.
- **JSON Schema generation and schema-from-annotations live in pydantic's Python layer**
  (`pydantic/json_schema.py`, `_internal/_generate_schema.py`), not in the core. Perldantic has
  to rebuild that layer itself: JSON Schema in Rust, the model DSL in Perl.

## 3. Architecture

```
Perl: Perldantic (lib/)          DSL, types, Model, TypeAdapter, ValidationError
        │  CoreSchema (hashref → JSON), data, callbacks
perldantic-ffi (ffi/, cdylib)    C ABI, opaque handles; phase 1 JSON I/O, phase 2 native SV bridge
        │
perldantic-core (crates/)        Python-free port: CoreSchema → CombinedValidator,
                                 Input for JsonValue / Value / (SV), serializers, JSON Schema
```

### Repository layout

```
Cargo.toml                    workspace
crates/perldantic-core/       Python-free port of pydantic-core (mirrors upstream file layout)
ffi/                          perldantic-ffi C ABI crate (the location FFI::Build expects)
lib/Perldantic.pm, lib/Perldantic/*.pm
t/                            Perl tests (Test2::V0)
tests/conformance/*.json      shared cases: schema + input → output | errors
upstream/UPSTREAM.md          base commit, file map, sync log
docs/                         PLAN.md, DIVERGENCES.md, BENCHMARKS.md
Makefile.PL, cpanfile, NOTICE
```

### Replacing pyo3 in the core

| pyo3 | perldantic-core |
|---|---|
| `Py<PyAny>` / `Bound<PyAny>` (result) | `Value` (neutral data model, see below) |
| `&Bound<PyDict>` (schema) | `Dict` in pydantic's `core_schema` wire format, read through `SchemaDict` accessors exactly as upstream validators do |
| `PyErr` / `PyResult` | `CoreError` / `CoreResult` (`thiserror`) |
| `Python<'py>` + `'py` lifetimes | removed; host access goes through `&dyn Host` where needed |
| `input_python.rs` | `input_value.rs` (`impl Input for Value`) |
| Python callables in `function-*` | `trait HostCallback` |

```rust
pub enum Value {
    None, Bool(bool), Int(i64), BigInt(BigInt), Float(f64),
    Str(String), Bytes(Vec<u8>), List(Vec<Value>), Tuple(Vec<Value>), Set(Vec<Value>),
    Dict(Dict),             // insertion-ordered, any keys
    Model(Box<Model>),      // { class, fields, fields_set, extra }
    // planned with their validators: Decimal, Date, Time, DateTime, Duration, Uuid, Url,
    // and Host (an opaque host object, e.g. a blessed ref)
}
```

A model class is identified by its name: in Perl, the package the host blesses a
`Value::Model` into. `model` schemas take that name as `cls`; class hierarchies (subclass
instances) will come through the host bridge (see `docs/DIVERGENCES.md` #13).

### Validator scope

| Priority | Validators |
|---|---|
| P0 (MVP) | any, none, bool, int (+bigint), float, str, bytes, literal, nullable, default, list, dict, tuple, union (smart / left_to_right / tagged), model, model_fields, definitions, lax_or_strict, custom_error |
| P1 | date, time, datetime, timedelta, decimal, uuid, url, enum, set, frozenset, chain, json, function-* (Perl callbacks), typed_dict |
| P2 | arguments / call, is_instance (→ `isa`/`DOES`), callable (→ CODE ref), generator (→ iterator) |
| Dropped | complex, fraction, named_tuple, dataclass, deque, counter, ordered_dict, frozendict, is_subclass, missing_sentinel, json_or_python, ellipsis, py_gc |

## 4. Perl API (target)

Two rules shape the public API:

1. **Declarations look like Moo/Moose**, so moving a class to Perldantic is cheap: replace
   `use Moo;` with `use Perldantic;` and keep the `has` lines. Validation happens in `new`.
2. **Pydantic's method names are kept** (`model_validate`, `model_dump`, `model_json_schema`,
   `TypeAdapter`, ...). They do not clash with anything in Perl and they match the pydantic docs.
   **Python's type vocabulary is not kept**: types use Type::Tiny / Types::Standard names and
   error messages use Perl words (see below).

```perl
package Ticket;
use Perldantic;                        # instead of `use Moo;`

has id       => (is => 'ro', isa => Int, required => 1, gt => 0);
has title    => (is => 'ro', isa => Str, required => 1, min_length => 1, max_length => 200);
has status   => (is => 'ro', isa => Enum[qw(open in_progress done)], default => 'open');
has tags     => (is => 'ro', isa => ArrayRef[Str], default => sub { [] });
has due      => (is => 'ro', isa => Maybe[Date]);
has meta     => (is => 'ro', isa => HashRef[Any], alias => 'metadata');

validator title => after => sub ($v, $info) { ucfirst $v };
model_config extra => 'forbid';

package main;
my $t  = Ticket->new(id => "42", title => "bug");               # Moo-style; lax: "42" -> 42
my $t2 = Ticket->model_validate({ id => 42, title => "bug" });  # pydantic-style, same result
my $t3 = Ticket->model_validate_json($json);
say $t->model_dump_json(exclude_undef => 1);
my $h  = $t->model_dump;                                        # plain hashref
say encode_json(Ticket->model_json_schema);
my $ints = Perldantic::TypeAdapter->new(ArrayRef[Int])->validate([1, "2"]);
```

### Moo/Moose compatibility

- `has` takes Moo/Moose options: `is` (`ro`, `rw`, `rwp`, `lazy`), `isa`, `required`,
  `default`, `builder`, `lazy`, `predicate`, `clearer`, `init_arg`, `trigger`, `coerce`,
  `documentation`. Perldantic constraints (`gt`, `min_length`, `pattern`, `alias`, `strict`, ...)
  go in the same list. An option Perldantic does not support is a `Perldantic::UsageError`
  that names it, never a silently ignored key.
- `isa` accepts Perldantic types and existing Type::Tiny constraints; a foreign constraint runs
  as a validator callback.
- `new` accepts a hash or a hashref like Moo, validates, and throws `Perldantic::ValidationError`.
  `BUILDARGS`, `BUILD` and `DEMOLISH` work as in Moo.
- `extends`, `with` (roles, `Perldantic::Role` on Role::Tiny) and method modifiers (`before`,
  `after`, `around`) follow Moo semantics where they apply to models.
- `use Perldantic;` enables `strict` and `warnings`, like Moo.

### Type vocabulary

Perl users see Perl names with their Perl meaning, taken from Types::Standard. The core keeps
pydantic's schema types internally.

| Perldantic type | Core schema (pydantic) | Note |
|---|---|---|
| `Any` | `any` | |
| `Undef` | `none` | Python `None` is Perl `undef` |
| `Maybe[T]` | `nullable` | Python `Optional[T]` |
| `Optional[T]` | a field that may be left out | Types::Standard meaning, inside `Dict[]` (Types::Standard also allows trailing `Tuple[]` items; not supported yet); not Python's `Optional` (that is `Maybe[T]`) |
| `Bool` | `bool` | |
| `Int` | `int` | Arbitrary size; big values as `Math::BigInt` |
| `Num` | `float` | |
| `Str` | `str` | |
| `Bytes` | `bytes` | |
| `ArrayRef[T]` | `list` | Python `list[T]` |
| `Tuple[A, B]`, `Tuple[A, slurpy ArrayRef[B]]` | `tuple` (positional, variadic tail) | A Perl array; accepted in strict mode, like a JSON array |
| `HashRef[V]` | `dict` with `str` keys | Python `dict[str, V]` |
| `Map[K, V]` | `dict` | Python `dict[K, V]`; keys arrive as strings and are validated leniently |
| `Dict[k => T, ...]` | `typed-dict` | Same meaning as Types::Standard `Dict`, **not** Python `dict` |
| `Enum[...]`, `Literal[...]` | `literal` / `enum` | |
| `InstanceOf['Class']` | `is-instance` | |

### Perl data and messages

- Errors for Perl input use Perl's vocabulary throughout (task 00078): codes (`hash_type`,
  `array_type`, `undef_required`, `number_type`, `duration_type`, ... instead of `dict_type`,
  `list_type`, `none_required`, `float_type`, `time_delta_type`), messages ("Input should be a
  hash reference" instead of "a valid dictionary"), contexts (`field_type` `Array` / `Hash`,
  classes `Math::BigFloat`, `Perldantic::Uuid`, ...), input values written as Perl data
  (`undef`, `!!1`, `{a => 1}`) and input types by Types::Standard names (`Str`, `HashRef`, ...).
  There are no links to pydantic's documentation, whose pages are named by pydantic's codes.
  Serializer warnings for Perl data name the expected type the way Perldantic::Types does
  (`ArrayRef[Int]`, `Map[Str, Num]`, `AnyOf[Duration, Undef]`).
- The Perl API has no names of Python types either: `TypeAdapter->validate` / `dump`
  (pydantic's `validate_python` / `dump_python`), dump option `exclude_undef` (`exclude_none`),
  `mode => 'perl'` (`python`), `when_used => 'unless-undef'` / `'json-unless-undef'`,
  `SerializationInfo->exclude_undef` and `mode` `perl`; there is no `FrozenSet` type (in Perl it
  would be `Set`). The Perl layer translates to the core's pydantic-shaped options, and refuses
  the Python names with a `Perldantic::UsageError`. The raw FFI layer (`Perldantic::FFI`) takes
  the core's options as they are.
  The core gets a `Perl` input type with its own templates, as upstream already has for JSON;
  Perl's codes are accepted wherever an error type is named (`Perldantic::KnownError`).
  Recorded as divergence #8.
- Perl arrays satisfy `Tuple` in strict mode, as JSON arrays do in pydantic.
- Hash keys are strings. Keys of `Map[Int, ...]` are validated from strings, and integer keys
  come back as strings.
- Hash order is random. Perl hashes are converted with sorted keys, so outputs and error order
  are deterministic (divergence #9). Model fields keep schema order.
- Self-referencing input data is reported as a `Perldantic::ValidationError`
  (`recursion_loop`), never a crash.
- Blessed references are objects: they are validated as models or through `InstanceOf`, not as
  plain arrays or hashes.

### Errors are exception objects

Perldantic never dies with a plain string. Every failure is raised as an exception object
(`die $obj`) from one class hierarchy. Each object carries a descriptive, Perl-oriented message,
and every class documents in POD when it is thrown.

| Class | Raised when | Payload |
|---|---|---|
| `Perldantic::Error` | Base class, never thrown directly | `message`, `type` (the exception the core reported), `throw`, stringification via `overload` |
| `Perldantic::ValidationError` | Input does not match the schema | `errors` (list of `{type, loc, msg, input, ctx}`, same codes as pydantic), `error_count`, `title`, `json`; stringifies like pydantic's `str(ValidationError)` |
| `Perldantic::SchemaError` | A model or type definition is invalid, or uses an unsupported schema type | `message`; the core's message names the path to the bad schema |
| `Perldantic::UsageError` | The API is called incorrectly, e.g. an unknown `has` option or a bad argument | `message`, `file`, `line` (the first caller outside Perldantic); stringifies like `die` |
| `Perldantic::SerializationError` | A value cannot be serialized (pydantic's `PydanticSerializationError`, `UnicodeDecodeError`) | `message`, `type` |
| `Perldantic::InternalError` | The Rust core panics or the FFI boundary fails | `message`, `cause` |

Messages name what failed and where, in Perl terms: the model package, the field and the
expected type, e.g. `Ticket->model_validate: 2 validation errors for Ticket`.

Perl-specific decisions:

| Issue | Decision |
|---|---|
| Untyped scalars (`"5"` vs `5`) | Strict mode reads SV flags (`IOK`/`NOK`/`POK`, `builtin::created_as_number`): the encoder writes numbers that were never strings as JSON numbers |
| Booleans | Accept `builtin::true`/`false`, `JSON::PP::Boolean`, `Types::Serialiser`. Lax coercions follow pydantic |
| Dates | Accept ISO strings, numbers (timestamps), `DateTime`, `Time::Moment` and `DateTime::Duration`. Output: Perldantic's own `Perldantic::Date/Time/DateTime/Duration` (1:1 with Python's naive/aware datetimes, times of day and timedeltas; no dependency), or `DateTime` / `Time::Moment` with `model_config temporal_class => ...` |
| Decimal / BigInt | `Math::BigFloat` / `Math::BigInt`, passed across FFI as strings |
| Accessors | Generated like Moo (`is => 'ro'/'rw'/...`), without a hard dependency on Moo/Moose |
| Minimum Perl | 5.36 (signatures, `builtin`) |

## 5. Perl ↔ Rust transport

- **Phase 1 (MVP): FFI::Platypus with JSON in and out.** The Rust crate is built at install time
  by `FFI::Build::File::Cargo`. This is simple and safe, but every call pays for a JSON round
  trip, and some scalar type information is lost.
- **Phase 2: native bridge.** Planned as a small XS layer implementing `Input` over SVs and
  building SVs directly from `Value`, at least 5× faster than phase 1 on nested models.
  Profiling (docs/BENCHMARKS.md) showed the time went to pure-Perl walks around the JSON, not to
  JSON itself, so phase 2 kept JSON at the boundary: a native encoder in C (`Perldantic.xs`,
  numbers told from strings by SV flags), one-pass decoding by Cpanel::JSON::XS, and cached
  per-class plans for building objects. Building objects became 6× faster than phase 1.
  Callbacks cross the boundary as FFI closures. A direct `Value` ↔ SV bridge is backlog task
  00080.

## 6. Stages

Tasks are tracked in `README.md` (IDs `00001`…). Every task follows TDD: tests first, then the
implementation, then integration tests. `cargo fmt --check`, `cargo clippy -- -D warnings`,
`cargo test` and `prove -lr t` must pass before each commit.

| Stage | Tasks | Estimate |
|---|---|---|
| 1. Scaffold & upstream import | 00001–00007 | 1–2 days |
| 2. Python-free core foundation | 00008–00016 | 1 week |
| 3. Conformance suite | 00017–00020 | 2–3 days |
| 4. P0 validators | 00021–00030 | 1.5–2 weeks |
| 5. Serialization & JSON Schema | 00031–00035 | 1 week |
| 6. FFI (C ABI) | 00036–00039 | 3 days |
| 7. Perl MVP | 00040–00049, 00076 | 1–1.5 weeks |
| 8. P1 types & callbacks | 00050–00056 | 1.5 weeks |
| 9. Native SV bridge | 00057–00060 | 1.5–2 weeks |
| 10. Integration scenarios | 00061–00066 | 3–4 days |
| 11. Docs & release 0.1.0 | 00067–00070 | 3 days |
| Backlog (post-0.1) | 00071–00075 | — |

The MVP (stages 1–7) takes about 5–6 weeks, and 0.1.0 about 9–12 weeks, for one developer.

## 7. Upstream sync

- `upstream/UPSTREAM.md` records the base commit and a map from upstream files to Perldantic files.
- To sync, run `git -C ../pydantic diff <old>..<new> -- pydantic-core/src`, then port the changes
  file by file using the map. The conformance cases catch behavioural drift.
- Keep the file layout mirrored (`src/validators/int.rs` ↔ `crates/perldantic-core/src/validators/int.rs`)
  so upstream diffs map onto our files directly.

## 8. Risks

| Risk | Mitigation |
|---|---|
| The port takes longer because pyo3 is everywhere | Port P0 first. Unsupported schema types fail with an explicit `SchemaError` |
| Python-style lax/strict rules do not fit Perl scalars | Default to lax. Document the strict-mode limits in phase 1 and fix them with SV flags in phase 2 |
| Installing from CPAN needs a Rust toolchain | Fail with a clear message. Later, ship prebuilt binaries or an Alien dist |
| Drift from upstream | Mirrored layout, the conformance suite and a sync log |
| A Rust panic crosses the FFI boundary | `catch_unwind` on every export, mapped to `Perldantic::InternalError` |
| Leaked handles | `DESTROY` → `pd_*_free`, with `Test::LeakTrace` tests |
