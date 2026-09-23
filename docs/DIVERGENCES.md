# Divergences from pydantic

Perldantic aims to behave exactly like pydantic for the same schema and input. Any difference
not listed here is a bug.

Each entry records the schema type, the pydantic behaviour, the Perldantic behaviour, and why
they differ.

| # | Area | pydantic | Perldantic | Reason |
|---|---|---|---|---|
| 1 | Python-only schema types | `complex`, `fraction`, `named-tuple`, `dataclass`, `deque`, `counter`, `ordered-dict`, `frozendict`, `is-subclass`, `missing-sentinel`, `json-or-python` | Rejected with a schema error | These types have no Perl counterpart (see `upstream/UPSTREAM.md`) |
