# JSON Schema cases

Snapshots of what pydantic's `GenerateJsonSchema` produces. The Rust test
`crates/perldantic-core/tests/json_schema.rs` replays them, and the port's output must match
pydantic exactly, including key order and warnings.

- `cases.json`: hand-picked core schemas for every supported schema type (listed in
  `tools/json_schema/record.py`). Every case runs.
- `upstream/<path of the pydantic test module>.json`: every JSON Schema that pydantic's own
  test suite generates with the stock `GenerateJsonSchema`, recorded by the
  `json_schema.recorder` pytest plugin. A case runs when all its values can be represented and
  every schema type in it is supported. Everything else is skipped and counted, so cases
  activate as the port grows. Run the test with `--nocapture` to see the summary.

Both are generated. Never edit them by hand. JSON Schema generation lives in pydantic's Python
layer, not in pydantic-core. `tools/requirements-record.txt` therefore pins pydantic at the
upstream base commit (see `upstream/UPSTREAM.md`), and recording its test suite needs a checkout
of that commit:

```bash
PYDANTIC_SRC=../pydantic tools/json_schema/record.sh
```

The suite is recorded twice with a fixed hash seed. Only cases that are identical in both runs
are kept.

## Case format

```json
{
  "id": "tests/test_json_schema.py::test_by_alias#0",
  "test": "tests/test_json_schema.py::test_by_alias",
  "schema": {"type": "model", "cls": {"$class": "ApplePie"}, "...": "..."},
  "config": null,
  "mode": "validation",
  "options": {"by_alias": false},
  "expected": {"json_schema": {"...": "..."}},
  "warnings": []
}
```

(`cases.json` entries have no `test`.)

| Field | Meaning |
|---|---|
| `schema` | The core schema, with what pydantic reads off model classes moved into it (see below) |
| `config` | The config for the whole schema (`TypeAdapter(..., config=...)`), or `null` |
| `mode` | `validation` or `serialization` |
| `options` | Non-default `GenerateJsonSchema` arguments: `by_alias`, `ref_template`, `union_format` |
| `expected` | `{"json_schema": ...}`, or `{"error": [type, message]}` |
| `warnings` | Messages of the warnings the call emitted |

Values use the conformance encoding (`tests/conformance/README.md`). A model's `cls` is
`{"$class": name}`, and the ids pydantic appends to core refs are renumbered within each case.

pydantic reads a model's title, `json_schema_extra`, docstring and a few other settings from
its Python class. The recorder hands them to the port the way a host would (docs/DIVERGENCES.md
#15):

- the `model_config` keys go into the `model` schema's `config`, with core config names;
- the docstring and `__deprecated__` go into `metadata.pydantic_js_updates`;
- the default `BaseModel.__get_pydantic_json_schema__` hook, which only calls the handler, is
  dropped;
- a TypedDict's own `__pydantic_config__` becomes the `typed-dict` schema's `config` (never the
  enclosing model's);
- an enum class's docstring goes into the `enum` schema's `metadata.pydantic_js_updates`, and
  the hook pydantic adds to repeat the title and docstring is dropped.

Python callables that are left, such as custom hooks and validators, are encoded as
`{"$function": name}`, and cases containing them are skipped.
