# Benchmarks

How fast Perldantic validates and dumps Perl data, against Moo + Type::Tiny, and where the time
goes.

## Method

`tools/bench/compare.pl` builds N orders of 10 items each from plain Perl data, checks them,
dumps them back to Perl data and to JSON, with Perldantic and with Moo + Type::Tiny
(Types::Standard, Type::Tiny::XS installed) on equivalent models:

```perl
package Pd::Item  { use Perldantic; has sku => (is => 'ro', isa => Str); has qty => (is => 'ro', isa => Int);
                    has price => (is => 'ro', isa => Num) }
package Pd::Order { use Perldantic; has id => (is => 'ro', isa => Int); has customer => (is => 'ro', isa => Str);
                    has items => (is => 'ro', isa => ArrayRef['Pd::Item']) }
```

The Moo models have the same attributes (`isa` from Types::Standard, `items` coerced from
hashes to objects) and a hand-written `to_data`; `check` is compared with Type::Tiny's `check`
of the same shape, `ArrayRef[Dict[id => Int, customer => Str, items => ArrayRef[Dict[...]]]]`.
Each operation runs for two seconds; the tables show the median call, which other load on the
machine skews less than a mean. Run it after `make`:

```sh
perl -Iblib/lib -Iblib/arch tools/bench/compare.pl 200
```

Machine: Apple M1 Max, Perl 5.42.0, Rust 1.98.1, release build of the core.

## Results

200 orders × 10 items (2,200 objects):

| Operation | Perldantic | Moo + Type::Tiny | |
|---|---:|---:|---:|
| Build objects: `validate` / `new` | 1.62 ms | 8.02 ms | 5.0× faster |
| Check only: `check` / Type::Tiny `check` | 0.77 ms | 1.92 ms | 2.5× faster |
| Dump to Perl data: `dump` / `to_data` | 1.89 ms | 1.65 ms | 1.15× slower |
| Dump to JSON: `dump_json` / `to_data` + JSON::XS | 1.42 ms | 2.42 ms | 1.7× faster |

With lazy objects (`lazy => 1`, see LAZY OBJECTS in Perldantic::Model's documentation), on
the same 200 × 10:

| Operation | Perldantic | Moo + Type::Tiny | |
|---|---:|---:|---:|
| Build objects: `validate(..., lazy => 1)` / `new` | 0.89 ms | 8.74 ms | 9.8× faster |
| Dump lazy objects to JSON / `to_data` + JSON::XS | 0.55 ms | 2.64 ms | 4.8× faster |
| Build, then read every field: lazy | 4.86 ms | 9.47 ms | 1.9× faster |
| Build, then read every field: built in full | 2.98 ms | 9.47 ms | 3.2× faster |

Reading every field of every lazy object costs more than building the objects at once, so lazy
objects are for data that is mostly passed on, dumped, or read in part.

20 orders × 10 items:

| Operation | Perldantic | Moo + Type::Tiny |
|---|---:|---:|
| `validate` / `new` | 0.16 ms | 0.79 ms |
| `check` | 0.08 ms | 0.19 ms |
| `dump` / `to_data` | 0.19 ms | 0.16 ms |
| `dump_json` / `to_data` + JSON::XS | 0.15 ms | 0.24 ms |

Perldantic does more than the Moo models: pydantic's validation and coercion rules, structured
errors for every invalid value at once, JSON Schema, and serialization options (include /
exclude, aliases, `exclude_unset`, ...). The Moo `to_data` is hand-written code that checks
nothing, which is why a plain dump to Perl data stays close to it rather than ahead.

## Where the time goes

`validate` on 200 × 10 (1.6 ms): the core reads the Perl data in place and validates it
(~0.7 ms), writes the result in the binary wire format (~0.2 ms), and the native decoder builds
the Perl objects (~0.6 ms). Building 2,200 Perl objects is the floor of any library that returns
them: copying and blessing the same hashes in plain Perl, checking nothing, takes 0.7 ms.
`check` builds no objects, so it goes below that floor.

## How it got here

| 200 × 10 | First version | Now |
|---|---:|---:|
| `validate` | 96.5 ms | 1.62 ms |
| `dump` | 102.3 ms | 1.89 ms |
| `dump_json` | 72.6 ms | 1.42 ms |

1. **Perl-side walks (stage 00057).** Profiling the first version showed the Rust core taking
   3 ms of 98: the rest was encoding JSON in Perl, decoding it with a second walk, and building
   objects. A native JSON encoder, one-pass decoding and per-class plans brought validation to
   ~16 ms.
2. **Building objects.** `_inflate` works in place, and objects built with every field given
   and no extra values keep no per-object state (their fields set are the fields they hold);
   plain models are then blessed by the native decoder itself.
3. **Dumping.** The native encoder writes model objects without state itself, and dumps track
   objects only when serializer functions need them back.
4. **The core's allocations.** mimalloc as the core's allocator; model values are shared
   (`Arc<Model>`) instead of copied by the serializer.
5. **No JSON at the boundary.** Values cross in a binary wire format (`ffi/src/binary.rs`):
   tagged nodes with raw strings and numbers, a JSON node only for rare types.
6. **No copy of the input.** The core reads Perl arrays and hashes in place through a table of
   C functions (`HostData`, `ffi/src/host_input.rs`); scalars are described without allocating.
   Every host-data case of the conformance suite is validated both ways and must agree.

7. **Lazy objects (opt-in).** Validation leaves the data in the core and returns objects that
   hold a handle to it (`ffi/src/lazy.rs`); an object reads its fields in when first used, and
   one not read yet is dumped from the core's data directly. This goes below the
   object-building floor.
