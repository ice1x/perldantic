# Divergences from pydantic

Perldantic aims to behave exactly like pydantic for the same schema and input. Any difference
not listed here is a bug.

Each entry records the schema type, the pydantic behaviour, the Perldantic behaviour, and why
they differ.

| # | Area | pydantic | Perldantic | Reason |
|---|---|---|---|---|
| 1 | Python-only schema types | `complex`, `fraction`, `named-tuple`, `dataclass`, `deque`, `counter`, `ordered-dict`, `frozendict`, `is-subclass`, `missing-sentinel`, `json-or-python` | Rejected with a schema error | These types have no Perl counterpart (see `upstream/UPSTREAM.md`) |
| 2 | Error URLs | `https://errors.pydantic.dev/<pydantic major.minor>/v/<type>` | `https://errors.pydantic.dev/latest/v/<type>` | The core has no pydantic version; `latest` is upstream's own fallback |
| 3 | URL toggle in `str(ValidationError)` | `PYDANTIC_ERRORS_INCLUDE_URL` | `PERLDANTIC_ERRORS_INCLUDE_URL` (same values) | Separate namespace for a separate library |
| 4 | `value_error` / `assertion_error` context | `ctx.error` holds the Python exception object | `ctx.error` holds the exception message string | There is no Python exception object; the message is what gets rendered |
| 5 | `repr()` of non-ASCII strings in error output | Uses Python's full Unicode `isprintable()` table | Escapes controls, separators, common format characters and private-use code points; other code points print as-is | No Unicode database in the core; differs only for rare unassigned or format characters |
| 6 | Invalid schemas | Validated against a generated self-schema first; errors read `Invalid Schema: ...` with a structured error list | Reported while building validators, as the first `SchemaError` / `KeyError` / `TypeError` found, e.g. `'strict' should be a bool, got str` | There is no Python-generated self-schema; the core schema format itself is unchanged |
| 7 | `regex_engine="python-re"` | Patterns run with Python's `re` module | Schema error `Invalid regex engine: python-re`; only `rust-regex` (the default) is available | There is no Python `re` outside Python; a Perl-regex engine through host callbacks may follow |
| 8 | Error messages for Perl input (planned, 00076) | "Input should be a valid dictionary / list / None" | "Input should be a hash reference / an array reference / undef"; error codes unchanged | Perl has hashes, arrays and undef, not dicts, lists and None |
| 9 | Order of Perl hash keys (planned, 00076) | Dicts keep insertion order | Perl hashes are converted with sorted keys | Perl hashes have no order; sorting makes outputs and errors deterministic |
| 10 | Invalid `default` with `validate_default` and `on_error='default'` | Recurses until the process crashes (stack overflow) | The default's own validation error is raised | A crash is never the intended behaviour |
| 11 | Schema values upstream never expects (`on_error` outside `raise`/`omit`/`default`, `variadic_item_index` out of range, `custom-error` without `custom_error_type`) | Rust panic (`PanicException`) | Schema error naming the bad value | Upstream relies on its self-schema to reject them first; a panic must never reach the host |
| 12 | Order of set items turned into a list or tuple | Python's hash order (`[1, '2', b'3']` from `{b'3', 1, '2'}`) | The order the set was given in | Hash order is an accident of CPython; Perl has no sets, so hosts pass them in a defined order |
| 13 | Model classes (`model` schemas) | `cls` is a Python class; instances of subclasses count as instances, and attribute access follows Python (proxies, `__getattr__`) | `cls` is a class name (a Perl package); only instances with that exact name count, attributes are the instance's fields | The core has no class objects; Perl class hierarchies will come through the host bridge |
| 14 | Model instances serialized by inference (`any` schemas, unexpected values) | Serialized with the model class's own serializer | Serialized as a dict of the instance's fields and extra values | The core has no registry of model classes and their serializers |
| 15 | Model config in JSON Schema generation | Read from the model class's `model_config` (`title`, `extra`, `json_schema_extra`, `json_schema_mode_override`, …); the class docstring becomes the `description`; callables (`json_schema_extra` functions, `model_title_generator`, `pydantic_js_functions`) customise the schema | Read from the `model` schema's `config` (and, for the whole schema, the `config` argument that stands in for a `TypeAdapter`'s), with core config names (`title`, `extra_fields_behavior`, `str_min_length`, `ser_json_bytes`, `validate_by_alias`) plus pydantic's `json_schema_extra` (dict only), `json_schema_mode_override` and `json_schema_serialization_defaults_required`; descriptions come from `metadata.pydantic_js_updates`; callables are a schema error | The core has no class objects; the host passes what pydantic reads off the class as schema data |
