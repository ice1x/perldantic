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
