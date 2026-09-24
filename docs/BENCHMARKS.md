# Benchmarks

How fast the Perl ↔ core round trip is, and where the time goes (stage 00057, docs/PLAN.md §5).

## Method

`tools/bench/compare.pl` builds N orders of 10 items each from plain Perl data, dumps them back
to Perl data and to JSON, with Perldantic and with Moo + Type::Tiny (Types::Standard,
Type::Tiny::XS installed) on equivalent models:

```perl
package Pd::Item  { use Perldantic; has sku => (is => 'ro', isa => Str); has qty => (is => 'ro', isa => Int);
                    has price => (is => 'ro', isa => Num) }
package Pd::Order { use Perldantic; has id => (is => 'ro', isa => Int); has customer => (is => 'ro', isa => Str);
                    has items => (is => 'ro', isa => ArrayRef['Pd::Item']) }
```

The Moo models have the same attributes (`isa` from Types::Standard, `items` coerced from
hashes to objects) and a hand-written `to_data`. Each operation runs for two seconds; the tables
show the median call, which other load on the machine skews less than a mean. Run it after `make`:

```sh
perl -Iblib/lib -Iblib/arch tools/bench/compare.pl 200
```

Machine: Apple M1 Max, Perl 5.42.0, Rust 1.98.1, release build of the core.

## Results

200 orders × 10 items (2,200 objects):

| Operation | Phase 1 (before 00057) | Now | Speed-up | Moo + Type::Tiny |
|---|---:|---:|---:|---:|
| Build: `validate` | 96.5 ms | 10.5 ms | 9.2× | 8.9 ms |
| Dump to Perl data: `dump` | 102.3 ms | 18.0 ms | 5.7× | 1.7 ms |
| Dump to JSON: `dump_json` | 72.6 ms | 17.1 ms | 4.2× | 2.4 ms (with JSON::XS) |

20 orders × 10 items:

| Operation | Phase 1 | Now | Moo + Type::Tiny |
|---|---:|---:|---:|
| `validate` | 9.5 ms | 0.95 ms | 0.8 ms |
| `dump` | 9.9 ms | 1.8 ms | 0.2 ms |
| `dump_json` | 7.1 ms | 1.7 ms | 0.3 ms |

Perldantic does more than the Moo models: pydantic's validation and coercion rules, structured
errors for every invalid value at once, JSON Schema, and serialization options (include /
exclude, aliases, `exclude_unset`, ...). The comparison shows the cost of that, and that it
stays within a small factor of hand-written Moo code for building objects.

## Where the time went

Profiling phase 1 (200 × 10, validation, 98 ms) showed the Rust core took 3 ms. The rest was
the Perl side of the transport:

| Step (phase 1) | Time | What changed |
|---|---:|---|
| Encoding input as JSON in Perl (tag, then write) | 14.7 ms | One pass (00057); then a native encoder in C, `Perldantic.xs` (00058) |
| Decoding the result: Cpanel::JSON::XS with `allow_bignum` | 30 ms | The core tags integers beyond 64 bits, so no bignum parsing |
| Decoding the result: untagging walk in Perl | 29 ms | Tags decoded by the parser itself (single-key object filters) |
| Building objects (`_inflate`) | 21.6 ms | Per-class plans, no recursion into scalars, decoded models blessed in place |

Now (200 × 10, validation, 14.9 ms): encoding 1.6 ms, core 3.2 ms, decoding 2.4 ms, building
objects 7.4 ms. Dumps (17–18 ms) spend 9.7 ms writing the model objects (per-object Perl code)
and 7.1 ms in the core, which reads 364 KB of model data.

## Design outcome

docs/PLAN.md planned phase 2 as an XS `Input` over SVs and SVs built from `Value` without
JSON. The measurements showed the JSON format itself was not the cost: the Rust side reads and
writes it in a few milliseconds, and Cpanel::JSON::XS parses it in C. The cost was the
pure-Perl walks around it. Stage 00057 therefore kept JSON at the boundary and removed those
walks (a native encoder, one-pass decoding, cached per-class plans), reaching the planned
"at least 5× faster than phase 1" for building objects. A direct `Value` ↔ SV bridge stays in
the backlog (task 00080) for when profiles point at the remaining JSON round trip.
