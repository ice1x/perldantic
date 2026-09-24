# Upstream

Perldantic's Rust core is a Python-free port of `pydantic-core`.

| | |
|---|---|
| Repository | https://github.com/pydantic/pydantic (directory `pydantic-core/`) |
| Base commit | `0384c970e37a59b344e75161eb106ea9996378ba` |
| pydantic-core version | `2.49.0` |
| Snapshot | [`pydantic-core/`](pydantic-core/): `src/`, `tests/`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `LICENSE`, `README.md`, unmodified |

The snapshot is reference material for porting, and the source of the conformance cases.
It is excluded from the Cargo workspace and never built. Do not edit it.

The behavioural oracle is the published `pydantic-core==2.49.0` wheel: `tools/conformance/record.sh`
runs the snapshot's validator tests against it and records the results in `tests/conformance/upstream/`.
The snapshot's tests for `counter` and `ordered_dict` cover schema types newer than that release
and fail against it (47 tests); both types are dropped from the port.

## Sync procedure

1. `git -C ../pydantic fetch && git -C ../pydantic diff <base>..<new> -- pydantic-core/src pydantic-core/tests`
2. Port each changed file to its target in the map below, test-first.
3. Replace the snapshot with the new commit, then update the base commit here and in
   `perldantic_core::UPSTREAM_COMMIT` / `UPSTREAM_VERSION`. `crates/perldantic-core/tests/upstream.rs`
   checks that these stay consistent.
4. Update `pydantic-core==` and the `pydantic @ git+...@<commit>` pin in
   `tools/requirements-record.txt`, then re-record the conformance cases with
   `tools/conformance/record.sh` and the JSON Schema cases with `tools/json_schema/record.sh`
   (which needs a pydantic checkout at the new commit); review the diffs of
   `tests/conformance/upstream/` and `tests/json_schema/`.
5. Add an entry to the sync log.

## Sync log

| Date | Commit | Notes |
|---|---|---|
| 2026-09-23 | `0384c970e` | Initial snapshot |

## File map

Priority follows [docs/PLAN.md](../docs/PLAN.md): P0 = MVP, P1/P2 = later stages, drop = Python-only.
Status: `pending` → `partial` → `ported` (or `dropped`).

| Upstream file | Perldantic file | Priority | Status | Notes |
|---|---|---|---|---|
| `argument_markers.rs` | - | drop | dropped | Python argument marker classes |
| `build_tools.rs` | `crates/perldantic-core/src/build_tools.rs` | P0 | ported | `SchemaError` class belongs to the host; `SchemaDict` merged in from `tools.rs` |
| `common/counter.rs` | - | drop | dropped | Python-only type |
| `common/deque.rs` | - | drop | dropped | Python-only type |
| `common/frozendict.rs` | - | drop | dropped | Python-only type |
| `common/missing_sentinel.rs` | - | drop | dropped | Python-only type |
| `common/mod.rs` | `crates/perldantic-core/src/common/mod.rs` | P0 | pending |  |
| `common/ordered_dict.rs` | - | drop | dropped | Python-only type |
| `common/prebuilt.rs` | - | drop | dropped | reuses validators attached to Python classes |
| `common/union.rs` | `crates/perldantic-core/src/validators/union.rs` | P0 | partial | `Discriminator` merged into `union.rs`; lookup paths only, function discriminators wait for host callbacks |
| `definitions.rs` | `crates/perldantic-core/src/definitions.rs` | P0 | ported | GC traversal and prebuilt flag dropped |
| `errors/line_error.rs` | `crates/perldantic-core/src/errors/line_error.rs` | P0 | ported |  |
| `errors/location.rs` | `crates/perldantic-core/src/errors/location.rs` | P0 | ported |  |
| `errors/mod.rs` | `crates/perldantic-core/src/errors/mod.rs` | P0 | ported |  |
| `errors/types.rs` | `crates/perldantic-core/src/errors/types.rs` | P0 | ported |  |
| `errors/validation_exception.rs` | `crates/perldantic-core/src/errors/validation_error.rs` | P0 | ported | renamed; `PyLineError` folded into `ErrorDetails` |
| `errors/value_exception.rs` | `crates/perldantic-core/src/errors/value_exception.rs` | P0 | partial | custom-message formatting ported into `types.rs`; exception classes belong to the host |
| `input/datetime.rs` | `crates/perldantic-core/src/input/datetime.rs` | P1 | done | values are speedate types throughout (no `Either*` wrappers); Python's forms live in `src/temporal.rs` |
| `input/input_abstract.rs` | `crates/perldantic-core/src/input/input_abstract.rs` | P0 | partial | P0 methods; more are added with their validators |
| `input/input_json.rs` | `crates/perldantic-core/src/input/input_json.rs` | P0 | partial | P0 methods for JSON and `str` |
| `input/input_python.rs` | `crates/perldantic-core/src/input/input_value.rs` | P0 | partial | rewritten as `Input for Value`; P0 methods |
| `input/input_string.rs` | `crates/perldantic-core/src/input/input_string.rs` | P1 | pending | string-mode input (env/config values) |
| `input/mod.rs` | `crates/perldantic-core/src/input/mod.rs` | P0 | ported |  |
| `input/return_enums.rs` | `crates/perldantic-core/src/input/return_enums.rs` | P0 | partial | `ValidationMatch`, `Either*`, `Int`, `MaxLengthCheck`, vec iteration helpers; set helpers and Python iterators pending/dropped |
| `input/shared.rs` | `crates/perldantic-core/src/input/shared.rs` | P0 | ported | decimal helpers included; fraction helpers belong to a dropped type |
| `lib.rs` | `crates/perldantic-core/src/lib.rs` | P0 | pending | rewrite: public Rust API, no #[pymodule] |
| `lookup_key.rs` | `crates/perldantic-core/src/lookup_key.rs` | P0 | ported | host data has no attributes, so attribute lookups are dropped |
| `py_gc.rs` | - | drop | dropped | Python GC integration |
| `recursion_guard.rs` | `crates/perldantic-core/src/recursion_guard.rs` | P0 | ported | safe inline array instead of `MaybeUninit` |
| `schema_gather.rs` | `crates/perldantic-core/src/schema_gather.rs` | P1 | pending | schema traversal for cleaning |
| `self_schema.py` | - | drop | dropped | Python-generated self schema; replaced by serde CoreSchema |
| `serializers/computed_fields.rs` | `crates/perldantic-core/src/serializers/computed_fields.rs` | P0 | pending |  |
| `serializers/config.rs` | `crates/perldantic-core/src/serializers/config.rs` | P0 | done | bytes, inf/nan and temporal (`ser_json_temporal`, `ser_json_timedelta`) modes |
| `serializers/errors.rs` | `crates/perldantic-core/src/serializers/errors.rs` | P0 | ported | exceptions are `SerializeError` variants |
| `serializers/extra.rs` | `crates/perldantic-core/src/serializers/extra.rs` | P0 | ported | warnings are returned with the output instead of emitted |
| `serializers/fields.rs` | `crates/perldantic-core/src/serializers/fields.rs` | P0 | partial | model fields; computed fields and `serialization_exclude_if` wait for host callbacks; `exclude_unset` uses the model's unset field names instead of the `MISSING` sentinel |
| `serializers/filter.rs` | `crates/perldantic-core/src/serializers/filter.rs` | P0 | ported | `...` is `true`; membership by Python equality |
| `serializers/infer.rs` | `crates/perldantic-core/src/serializers/infer.rs` | P0 | partial | the kinds `Value` has; model instances as dicts (DIVERGENCES #14); no `fallback` callbacks |
| `serializers/mod.rs` | `crates/perldantic-core/src/serializers/mod.rs` | P0 | ported | `to_json`/`to_jsonable_python` module functions not needed |
| `serializers/ob_type.rs` | `crates/perldantic-core/src/serializers/ob_type.rs` | P0 | ported | rewritten over `Value` variants, keeping Python's bool/int/float subclass rules |
| `serializers/polymorphism_trampoline.rs` | - | drop | dropped | Python subclass dispatch |
| `serializers/prebuilt.rs` | - | drop | dropped | reuses validators attached to Python classes |
| `serializers/ser.rs` | `crates/perldantic-core/src/serializers/ser.rs` | P0 | ported |  |
| `serializers/shared.rs` | `crates/perldantic-core/src/serializers/shared.rs` | P0 | partial | function serializers wait for host callbacks |
| `serializers/type_serializers/any.rs` | `crates/perldantic-core/src/serializers/type_serializers/any.rs` | P0 | ported |  |
| `serializers/type_serializers/bytes.rs` | `crates/perldantic-core/src/serializers/type_serializers/bytes.rs` | P0 | ported |  |
| `serializers/type_serializers/complex.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/counter.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/dataclass.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/datetime_etc.rs` | `crates/perldantic-core/src/serializers/type_serializers/datetime_etc.rs` | P1 | done | the `*_to_seconds` helpers and temporal modes live in `serializers/config.rs` |
| `serializers/type_serializers/decimal.rs` | `crates/perldantic-core/src/serializers/type_serializers/decimal.rs` | P1 | done |  |
| `serializers/type_serializers/definitions.rs` | `crates/perldantic-core/src/serializers/type_serializers/definitions.rs` | P0 | ported |  |
| `serializers/type_serializers/deque.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/dict.rs` | `crates/perldantic-core/src/serializers/type_serializers/dict.rs` | P0 | ported |  |
| `serializers/type_serializers/ellipsis.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/enum_.rs` | `crates/perldantic-core/src/serializers/type_serializers/enum_.rs` | P1 | pending |  |
| `serializers/type_serializers/float.rs` | `crates/perldantic-core/src/serializers/type_serializers/float.rs` | P0 | ported |  |
| `serializers/type_serializers/format.rs` | `crates/perldantic-core/src/serializers/type_serializers/format.rs` | P1 | pending |  |
| `serializers/type_serializers/fraction.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/frozendict.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/function.rs` | `crates/perldantic-core/src/serializers/type_serializers/function.rs` | P1 | pending |  |
| `serializers/type_serializers/generator.rs` | `crates/perldantic-core/src/serializers/type_serializers/generator.rs` | P2 | pending |  |
| `serializers/type_serializers/json.rs` | `crates/perldantic-core/src/serializers/type_serializers/json.rs` | P1 | pending |  |
| `serializers/type_serializers/json_or_python.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/list.rs` | `crates/perldantic-core/src/serializers/type_serializers/list.rs` | P0 | ported |  |
| `serializers/type_serializers/literal.rs` | `crates/perldantic-core/src/serializers/type_serializers/literal.rs` | P0 | ported |  |
| `serializers/type_serializers/missing_sentinel.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/mod.rs` | `crates/perldantic-core/src/serializers/type_serializers/mod.rs` | P0 | partial | modules of the ported serializers |
| `serializers/type_serializers/model.rs` | `crates/perldantic-core/src/serializers/type_serializers/model.rs` | P0 | partial | model classes by name (DIVERGENCES #13); no subclass polymorphism or computed fields |
| `serializers/type_serializers/named_tuple.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/nullable.rs` | `crates/perldantic-core/src/serializers/type_serializers/nullable.rs` | P0 | ported |  |
| `serializers/type_serializers/ordered_dict.rs` | - | drop | dropped | Python-only type |
| `serializers/type_serializers/other.rs` | - | drop | dropped | Python-specific fallbacks |
| `serializers/type_serializers/set_frozenset.rs` | `crates/perldantic-core/src/serializers/type_serializers/set.rs` | P1 | done | Perl arrays accepted for Perl data (DIVERGENCES #19) |
| `serializers/type_serializers/simple.rs` | `crates/perldantic-core/src/serializers/type_serializers/simple.rs` | P0 | ported |  |
| `serializers/type_serializers/string.rs` | `crates/perldantic-core/src/serializers/type_serializers/string.rs` | P0 | ported |  |
| `serializers/type_serializers/timedelta.rs` | `crates/perldantic-core/src/serializers/type_serializers/timedelta.rs` | P1 | done |  |
| `serializers/type_serializers/tuple.rs` | `crates/perldantic-core/src/serializers/type_serializers/tuple.rs` | P0 | ported | out-of-range `variadic_item_index` is a schema error (DIVERGENCES #11) |
| `serializers/type_serializers/typed_dict.rs` | `crates/perldantic-core/src/serializers/type_serializers/typed_dict.rs` | P1 | pending |  |
| `serializers/type_serializers/union.rs` | `crates/perldantic-core/src/serializers/type_serializers/union.rs` | P0 | partial | function discriminators wait for host callbacks |
| `serializers/type_serializers/url.rs` | `crates/perldantic-core/src/serializers/type_serializers/url.rs` | P1 | done |  |
| `serializers/type_serializers/uuid.rs` | `crates/perldantic-core/src/serializers/type_serializers/uuid.rs` | P1 | done |  |
| `serializers/type_serializers/with_default.rs` | `crates/perldantic-core/src/serializers/type_serializers/with_default.rs` | P0 | ported |  |
| `tools.rs` | `crates/perldantic-core/src/tools.rs` | P0 | partial | truncation helpers ported; `SchemaDict` lives in `build_tools.rs` |
| `url.rs` | `crates/perldantic-core/src/url.rs` | P1 | done | `Url` / `MultiHostUrl` accessors without `build` (the Perl classes build URL text); `pd_url_parts` exposes the accessors |
| `validators/any.rs` | `crates/perldantic-core/src/validators/any.rs` | P0 | ported |  |
| `validators/arguments.rs` | `crates/perldantic-core/src/validators/arguments.rs` | P2 | pending |  |
| `validators/arguments_v3.rs` | `crates/perldantic-core/src/validators/arguments_v3.rs` | P2 | pending |  |
| `validators/bool.rs` | `crates/perldantic-core/src/validators/bool.rs` | P0 | ported |  |
| `validators/bytes.rs` | `crates/perldantic-core/src/validators/bytes.rs` | P0 | ported |  |
| `validators/call.rs` | `crates/perldantic-core/src/validators/call.rs` | P2 | pending |  |
| `validators/callable.rs` | `crates/perldantic-core/src/validators/callable.rs` | P2 | pending |  |
| `validators/chain.rs` | `crates/perldantic-core/src/validators/chain.rs` | P1 | pending |  |
| `validators/complex.rs` | - | drop | dropped | Python-only type |
| `validators/config.rs` | `crates/perldantic-core/src/validators/config.rs` | P0 | partial | `ValBytesMode` / `BytesMode`; temporal modes pending |
| `validators/counter.rs` | - | drop | dropped | Python-only type |
| `validators/custom_error.rs` | `crates/perldantic-core/src/validators/custom_error.rs` | P0 | ported | known and custom errors are both an `ErrorType`; a missing `custom_error_type` is a schema error (DIVERGENCES #11) |
| `validators/dataclass.rs` | - | drop | dropped | Python-only type |
| `validators/date.rs` | `crates/perldantic-core/src/validators/date.rs` | P1 | done |  |
| `validators/datetime.rs` | `crates/perldantic-core/src/validators/datetime.rs` | P1 | done | `now_op` without `now_utc_offset` uses UTC (DIVERGENCES #16) |
| `validators/decimal.rs` | `crates/perldantic-core/src/validators/decimal.rs` | P1 | done | Python's `Decimal` is `src/decimal.rs` (exact values, 28-digit division for `multiple_of`); DIVERGENCES #17 |
| `validators/definitions.rs` | `crates/perldantic-core/src/validators/definitions.rs` | P0 | partial | host data is guarded by value identity like Python objects; `validate_assignment` comes with the Perl model API |
| `validators/deque.rs` | - | drop | dropped | Python-only type |
| `validators/dict.rs` | `crates/perldantic-core/src/validators/dict.rs` | P0 | ported |  |
| `validators/ellipsis.rs` | - | drop | dropped | Python-only type |
| `validators/enum_.rs` | `crates/perldantic-core/src/validators/enum_.rs` | P1 | pending |  |
| `validators/float.rs` | `crates/perldantic-core/src/validators/float.rs` | P0 | ported |  |
| `validators/fraction.rs` | - | drop | dropped | Python-only type |
| `validators/frozendict.rs` | - | drop | dropped | Python-only type |
| `validators/frozenset.rs` | `crates/perldantic-core/src/validators/set.rs` | P1 | done | shares `set.rs` |
| `validators/function.rs` | `crates/perldantic-core/src/validators/function.rs` | P1 | pending |  |
| `validators/generator.rs` | `crates/perldantic-core/src/validators/generator.rs` | P2 | pending |  |
| `validators/int.rs` | `crates/perldantic-core/src/validators/int.rs` | P0 | ported |  |
| `validators/is_instance.rs` | `crates/perldantic-core/src/validators/is_instance.rs` | P2 | pending |  |
| `validators/is_subclass.rs` | - | drop | dropped | Python-only type |
| `validators/json.rs` | `crates/perldantic-core/src/validators/json.rs` | P1 | pending |  |
| `validators/json_or_python.rs` | - | drop | dropped | Python-only type |
| `validators/lax_or_strict.rs` | `crates/perldantic-core/src/validators/lax_or_strict.rs` | P0 | ported |  |
| `validators/list.rs` | `crates/perldantic-core/src/validators/list.rs` | P0 | ported |  |
| `validators/literal.rs` | `crates/perldantic-core/src/validators/literal.rs` | P0 | partial | hash lookups replaced by Python-equality tables (`Value::py_eq`); enum lookups come with `enum` |
| `validators/missing_sentinel.rs` | - | drop | dropped | Python-only type |
| `validators/mod.rs` | `crates/perldantic-core/src/validators/mod.rs` | P0 | partial | `SchemaValidator` (new, validate_value, validate_json), `Extra`, build dispatch, `Validator`/`BuildValidator`; more methods as validators need them |
| `validators/model.rs` | `crates/perldantic-core/src/validators/model.rs` | P0 | partial | `cls` is a class name and instances are `Value::Model` (DIVERGENCES #13); `post_init`, `custom_init` and `self_instance` wait for host callbacks; assignment comes with the Perl model API |
| `validators/model_fields.rs` | `crates/perldantic-core/src/validators/model_fields.rs` | P0 | partial | output is `(dict, extra, set)`; `validate_assignment` comes with the Perl model API; no attribute input |
| `validators/named_tuple.rs` | - | drop | dropped | Python-only type |
| `validators/none.rs` | `crates/perldantic-core/src/validators/none.rs` | P0 | ported |  |
| `validators/nullable.rs` | `crates/perldantic-core/src/validators/nullable.rs` | P0 | ported |  |
| `validators/ordered_dict.rs` | - | drop | dropped | Python-only type |
| `validators/prebuilt.rs` | - | drop | dropped | reuses validators attached to Python classes |
| `validators/set.rs` | `crates/perldantic-core/src/validators/set.rs` | P1 | done | items kept in input order (DIVERGENCES #12); Perl arrays are exact sets |
| `validators/shared/lookup_tree.rs` | `crates/perldantic-core/src/validators/shared/lookup_tree.rs` | P0 | ported | index lookups in a `BTreeMap` for a deterministic order |
| `validators/shared/mod.rs` | `crates/perldantic-core/src/validators/shared/mod.rs` | P0 | pending |  |
| `validators/string.rs` | `crates/perldantic-core/src/validators/string.rs` | P0 | ported | `python-re` engine unavailable (divergence 7); no regex LRU cache yet |
| `validators/time.rs` | `crates/perldantic-core/src/validators/time.rs` | P1 | done |  |
| `validators/timedelta.rs` | `crates/perldantic-core/src/validators/timedelta.rs` | P1 | done |  |
| `validators/tuple.rs` | `crates/perldantic-core/src/validators/tuple.rs` | P0 | ported | out-of-range `variadic_item_index` is a schema error (DIVERGENCES #11) |
| `validators/typed_dict.rs` | `crates/perldantic-core/src/validators/typed_dict.rs` | P1 | pending |  |
| `validators/union.rs` | `crates/perldantic-core/src/validators/union.rs` | P0 | partial | `union` and `tagged-union`; function discriminators wait for host callbacks |
| `validators/url.rs` | `crates/perldantic-core/src/validators/url.rs` | P1 | done | no prebuilt simple validators |
| `validators/uuid.rs` | `crates/perldantic-core/src/validators/uuid.rs` | P1 | done | `Value::Uuid`; strict mode wants a UUID object for Perl input too |
| `validators/validation_state.rs` | `crates/perldantic-core/src/validators/validation_state.rs` | P0 | ported | `self_instance` and the Python string cache dropped |
| `validators/with_default.rs` | `crates/perldantic-core/src/validators/with_default.rs` | P0 | partial | `default_factory` waits for host callbacks; defaults are always copied; invalid defaults with `on_error='default'` report their error (DIVERGENCES #10) |

## pydantic layer

JSON Schema generation lives in pydantic's Python package, not in pydantic-core. It is ported
from the same base commit of `pydantic/pydantic` (not vendored here). `tools/json_schema/record.sh`
records its cases in `tests/json_schema/` from that commit's `pydantic`: hand-picked core schemas,
plus every JSON Schema pydantic's own test suite generates.

| Upstream file | Perldantic file | Priority | Status | Notes |
|---|---|---|---|---|
| `pydantic/json_schema.py` | `crates/perldantic-core/src/json_schema/` | P0 | partial | `GenerateJsonSchema.generate` for the P0 schema types, dates/times/datetimes/timedeltas, decimals, UUIDs and URLs; `generate_definitions` (`models_json_schema`), other P1 types and callables later; model config from the schema (DIVERGENCES #15) |
