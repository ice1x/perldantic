#!/bin/sh
# Re-record the JSON Schema cases in tests/json_schema/:
#
# - cases.json: pydantic's output for the core schemas listed in tools/json_schema/record.py;
# - upstream/: every JSON Schema pydantic's own test suite generates, recorded with the
#   json_schema.recorder pytest plugin.
#
# JSON Schema generation lives in pydantic's Python layer: tools/requirements-record.txt pins
# pydantic at the upstream base commit (upstream/UPSTREAM.md), and running its test suite needs a
# checkout of pydantic/pydantic at that commit, given as PYDANTIC_SRC (default: ../pydantic next
# to this repository). The suite is recorded twice with a fixed hash seed and only cases
# identical in both runs are kept.
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
VENV="$ROOT/tools/.venv"
PYDANTIC_SRC=$(cd "${PYDANTIC_SRC:-"$ROOT/../pydantic"}" && pwd)
BASE=$(sed -n 's/^| Base commit | `\([0-9a-f]*\)` |$/\1/p' "$ROOT/upstream/UPSTREAM.md")
OUT="$ROOT/tests/json_schema/upstream"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

HEAD=$(git -C "$PYDANTIC_SRC" rev-parse HEAD)
if [ "$HEAD" != "$BASE" ]; then
    echo "$PYDANTIC_SRC is at $HEAD, expected the upstream base commit $BASE" >&2
    exit 1
fi

if [ ! -x "$VENV/bin/python" ]; then
    python3 -m venv "$VENV"
fi
"$VENV/bin/pip" install -q -r "$ROOT/tools/requirements-record.txt"

export PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$ROOT/tools"
"$VENV/bin/python" -m json_schema.record "$ROOT/tests/json_schema/cases.json"

cd "$PYDANTIC_SRC"
for run in a b; do
    # Tests failing for missing optional dependencies are fine: what is recorded is pydantic's
    # output, not the tests' verdicts.
    PYTHONHASHSEED=0 "$VENV/bin/python" -m pytest tests -o addopts="" -q -p no:cacheprovider \
        -p json_schema.recorder --json-schema-out="$WORK/$run" --continue-on-collection-errors \
        --ignore=tests/mypy --ignore=tests/benchmarks --ignore=tests/plugin --ignore=tests/pydantic_core \
        >"$WORK/$run.log" 2>&1 || true
    tail -1 "$WORK/$run.log"
done

rm -rf "$OUT"
"$VENV/bin/python" -m conformance.stabilize "$WORK/a" "$WORK/b" "$OUT"
echo "cases written to $ROOT/tests/json_schema/"
