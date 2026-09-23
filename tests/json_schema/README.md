# JSON Schema cases

What pydantic's `GenerateJsonSchema` produces for a set of core schemas. The Rust test
`crates/perldantic-core/tests/json_schema.rs` replays every case, and the output must match
exactly, including key order and warnings.

`cases.json` is generated. Never edit it by hand. JSON Schema generation lives in pydantic's
Python layer, not in pydantic-core, so the recorder needs a checkout of `pydantic/pydantic` at
the upstream base commit (see `upstream/UPSTREAM.md`):

```bash
PYDANTIC_SRC=../pydantic tools/json_schema/record.sh
```

The cases themselves are listed in `tools/json_schema/record.py`. Each one holds:

- the core schema;
- the mode (`validation` / `serialization`);
- the `GenerateJsonSchema` options;
- the outcome: `{"json_schema": ...}` or `{"error": [type, message]}`;
- any warnings.

Values use the conformance encoding (`tests/conformance/README.md`). A model's `cls` is
`{"$class": name}`.
