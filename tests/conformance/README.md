# Conformance cases

Replayable records of what pydantic-core actually does. They check the "behaves like pydantic"
contract in [docs/PLAN.md](../../docs/PLAN.md): the Rust core and the Perl library replay them
and must produce the same outcomes.

## Where they come from

`upstream/` is generated, never edited by hand. [`tools/conformance/record.sh`](../../tools/conformance/record.sh)
runs upstream pydantic-core's validator and serializer tests (`upstream/pydantic-core/tests/validators`,
`tests/serializers`) against the matching published release (`pydantic-core==2.49.0`) with a
pytest plugin that records every `SchemaValidator.validate_python` / `validate_json` and
`SchemaSerializer.to_python` / `to_json` call:

- the schema and config the validator was built with;
- the input and the keyword options of the call;
- the outcome pydantic produced: output value, JSON text, validation errors, or another
  exception, plus any serializer warnings.

What is recorded is pydantic's real behaviour, independent of what each test asserts. Tests
driven by Hypothesis (random inputs) are not recorded. The suite is recorded twice with a fixed
hash seed, and only cases identical in both runs are kept, so re-recording is reproducible.

Re-record after moving the upstream base (see `upstream/UPSTREAM.md`):

```bash
tools/conformance/record.sh
```

## File layout

`upstream/<path of the upstream test module>.json`, e.g. `upstream/tests/validators/test_int.json`,
holding a JSON array of cases sorted by `id`.

## Case format

```json
{
  "id": "tests/validators/test_int.py::test_int_py_and_json[python-42_str-42_int]#0",
  "test": "tests/validators/test_int.py::test_int_py_and_json[python-42_str-42_int]",
  "schema": {"type": "int"},
  "config": null,
  "mode": "python",
  "input": "42",
  "options": {},
  "expected": {"output": 42}
}
```

| Field | Meaning |
|---|---|
| `id` | Unique, stable id: the pytest node id plus `#n` for the n-th distinct call in that test |
| `test` | The upstream pytest node id (memory addresses in ids are replaced by `0x...`) |
| `schema` | The core schema, encoded as below |
| `config` | The core config, or `null` |
| `mode` | `python` for `validate_python` (host data), `json` for `validate_json` (JSON text), `to_python` / `to_json` for `SchemaSerializer.to_python` / `to_json` |
| `input` | The input: an encoded value in `python` mode; the JSON document as a string in `json` mode (bytes inputs are encoded as `$bytes`) |
| `options` | Keyword arguments of the call, e.g. `{"strict": true}`, `{"context": ...}`, `{"allow_partial": true}` |
| `expected` | Exactly one of the outcomes below |

Outcomes:

- `{"output": <value>}`: validation succeeded with this output.
- `{"errors": [...], "title": "..."}`: a `ValidationError` with this title. Each error has
  `type`, `loc` (list of strings and ints), `msg`, `input` and, when present, `ctx`, as returned
  by pydantic's `errors(include_url=False)`.
- `{"json": "..."}`: `to_json` succeeded with this JSON text.
- `{"exception": {"type": "...", "message": "..."}}`: any other exception.

A serializer outcome may also carry `"warnings": ["..."]`, the messages of the warnings the call
emitted (e.g. `PydanticSerializationUnexpectedValue`).

Identical calls (same schema, config, mode, input and options) within one run are recorded once.

## Value encoding

Plain JSON types are written as-is: `null`, booleans, integers of any size, finite floats
(always written with a fraction or exponent, e.g. `1.0`), strings, arrays (Python `list`) and
objects with plain string keys (Python `dict`).

Everything else is a single-key object whose key starts with `$`.

Values the core can represent:

| Encoding | Python value |
|---|---|
| `{"$tuple": [...]}` | `tuple` |
| `{"$bytes": "<base64>"}` | `bytes` |
| `{"$float": "inf" \| "-inf" \| "nan"}` | non-finite `float` |
| `{"$dict": [[key, value], ...]}` | `dict` with non-string keys, or keys starting with `$` |
| `{"$set": [...]}` / `{"$frozenset": [...]}` | `set` / `frozenset` (items sorted) |
| `{"$decimal": "1.50"}` | `decimal.Decimal` |
| `{"$date": ...}` / `{"$time": ...}` / `{"$datetime": ...}` | ISO 8601 text |
| `{"$timedelta": [days, seconds, microseconds]}` | `datetime.timedelta` |
| `{"$uuid": "..."}` | `uuid.UUID` |

Values that only describe Python objects:

| Encoding | Python value |
|---|---|
| `{"$class": "Name"}` | a class, e.g. a model schema's `cls`; inside a schema or config the core reads it as the class name |
| `{"$model": {"class", "fields", "fields_set", "extra"}}` | a validated model instance; the core represents it as `Value::Model` |
| `{"$enum": ["Class", value]}` | an enum member |
| `{"$function": "name"}` | a function (validator functions, default factories, ...) |
| `{"$exception": ["Type", "message"]}` | an exception, e.g. in an error `ctx` |
| `{"$subclass": ["Class", base_value]}` | an instance of a subclass of a builtin type |
| `{"$bytearray": "<base64>"}` | `bytearray` |
| `{"$object": "type name"}` | anything else; `str-with-surrogates` marks strings that are not valid Unicode |

## Replaying

A runner builds a validator from `schema`/`config`, validates `input` in `mode` with `options`,
and compares the outcome with `expected`. A case is skipped (and counted) when it uses a schema
type or option that is not implemented yet, or contains a value the target cannot represent,
e.g. `$function` before Perl callbacks exist or `$enum`. As validators are ported, their
cases become active automatically. Skipped cases are listed with the reason; an active case
that does not match is a failure.

Two runners replay the cases:

- `crates/perldantic-core/tests/conformance.rs` against the Rust core;
- `t/conformance.t` through the Perl library (Perldantic::FFI and the wire codec). It reads the
  files with an order-preserving parser (`t/lib/Perldantic/Test/CaseJSON.pm`) and sends dicts in
  their recorded order with `Perldantic::Wire::ordered`, so outputs and errors compare exactly.
  It skips what the core cannot build yet ("Unknown schema type", "is not supported yet"), the
  same divergences as the Rust runner, and outcomes that only reflect the recorder (an exception
  about `RecordingSerializer`). Sets compare without order. Run it with
  `prove -b -v t/conformance.t` to see the summary.
