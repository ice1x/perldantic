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
| `&Bound<PyDict>` (schema) | `CoreSchema`: a serde enum tagged by `type`, using pydantic's wire format |
| `PyErr` / `PyResult` | `CoreError` / `CoreResult` (`thiserror`) |
| `Python<'py>` + `'py` lifetimes | removed; host access goes through `&dyn Host` where needed |
| `input_python.rs` | `input_value.rs` (`impl Input for Value`) |
| Python callables in `function-*` | `trait HostCallback` |

```rust
pub enum Value {
    None, Bool(bool), Int(i64), BigInt(BigInt), Float(f64), Decimal(Decimal),
    Str(Arc<str>), Bytes(Arc<[u8]>), List(Vec<Value>), Tuple(Vec<Value>), Set(Vec<Value>),
    Dict(IndexMap<Key, Value>), Date(Date), Time(Time), DateTime(DateTime), Duration(Duration),
    Uuid(Uuid), Url(Url),
    Model { class: Arc<str>, fields: IndexMap<Arc<str>, Value>, fields_set: BitSet },
    Host(HostRef), // opaque host object (a blessed ref in Perl)
}
```

### Validator scope

| Priority | Validators |
|---|---|
| P0 (MVP) | any, none, bool, int (+bigint), float, str, bytes, literal, nullable, default, list, dict, tuple, union (smart / left_to_right / tagged), model, model_fields, definitions, lax_or_strict, custom_error |
| P1 | date, time, datetime, timedelta, decimal, uuid, url, enum, set, frozenset, chain, json, function-* (Perl callbacks), typed_dict |
| P2 | arguments / call, is_instance (→ `isa`/`DOES`), callable (→ CODE ref), generator (→ iterator) |
| Dropped | complex, fraction, named_tuple, dataclass, deque, counter, ordered_dict, frozendict, is_subclass, missing_sentinel, json_or_python, ellipsis, py_gc |

## 4. Perl API (target)

```perl
package Ticket;
use Perldantic;

field id       => Int,  gt => 0;
field title    => Str,  min_length => 1, max_length => 200;
field status   => Enum[qw(open in_progress done)], default => 'open';
field tags     => ArrayRef[Str], default => sub { [] };
field due      => Optional[Date];
validator title => after => sub ($v, $info) { ucfirst $v };
model_config extra => 'forbid';

package main;
my $t = Ticket->model_validate({ id => "42", title => "bug" });  # lax: "42" → 42
say $t->model_dump_json(exclude_none => 1);
say encode_json(Ticket->model_json_schema);
my $ints = Perldantic::TypeAdapter->new(ArrayRef[Int])->validate_python([1, "2"]);
```

Perl-specific decisions:

| Issue | Decision |
|---|---|
| Untyped scalars (`"5"` vs `5`) | Strict mode reads SV flags (`IOK`/`NOK`/`POK`, `builtin::created_as_number`). Phase 1 relies on Cpanel::JSON::XS encoding |
| Booleans | Accept `builtin::true`/`false`, `JSON::PP::Boolean`, `Types::Serialiser`. Lax coercions follow pydantic |
| Dates | Accept ISO strings, `Time::Moment` and `DateTime`. The output class is configurable |
| Decimal / BigInt | `Math::BigFloat` / `Math::BigInt`, passed across FFI as strings |
| Accessors | Generated read-only accessors, no hard dependency on Moo/Moose |
| Minimum Perl | 5.36 (signatures, `builtin`) |

## 5. Perl ↔ Rust transport

- **Phase 1 (MVP): FFI::Platypus with JSON in and out.** The Rust crate is built at install time
  by `FFI::Build::File::Cargo`. This is simple and safe, but every call pays for a JSON round
  trip, and some scalar type information is lost.
- **Phase 2: native bridge.** A small XS layer implements `Input` over SVs, zero-copy, and builds
  SVs directly from `Value`. Callbacks cross the boundary as FFI closures. Target: at least 5×
  faster than phase 1 on nested models.

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
| 7. Perl MVP | 00040–00049 | 1–1.5 weeks |
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
