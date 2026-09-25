# Examples

Runnable scripts; from a built checkout run `perl -Iblib/lib -Iblib/arch examples/01-models.pl`
(or plain `perl examples/01-models.pl` once Perldantic is installed). `t/44-examples.t` runs
them all and checks what they print against the comments in them.

| Script | Shows |
|---|---|
| [01-models.pl](01-models.pl) | models, conversions, nested models, validation errors |
| [02-json.pl](02-json.pl) | JSON input and output, dates and decimals, JSON Schema |
| [03-validators.pl](03-validators.pl) | field and model validators, field serializers |
| [04-types.pl](04-types.pl) | TypeAdapter, enum classes, `validate_call` |
| [05-moo.pl](05-moo.pl) | Perldantic types in a Moo class (needs Moo) |

More: `perldoc Perldantic` (and the other modules' documentation),
[docs/MIGRATING_FROM_PYDANTIC.md](../docs/MIGRATING_FROM_PYDANTIC.md) (pydantic next to Perl,
topic by topic), and the scenarios in `t/30-cbt-journal.t` … `t/34-bug-tracker.t`.
