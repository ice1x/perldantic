#!/bin/sh
# Re-record tests/json_schema/cases.json: pydantic's JSON Schema output for a set of core
# schemas. JSON Schema generation lives in pydantic's Python layer, so this needs a checkout of
# pydantic/pydantic at the upstream base commit (upstream/UPSTREAM.md), given as PYDANTIC_SRC
# (default: ../pydantic next to this repository).
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
VENV="$ROOT/tools/.venv"
PYDANTIC_SRC=${PYDANTIC_SRC:-"$ROOT/../pydantic"}
BASE=$(sed -n 's/^| Base commit | `\([0-9a-f]*\)` |$/\1/p' "$ROOT/upstream/UPSTREAM.md")

HEAD=$(git -C "$PYDANTIC_SRC" rev-parse HEAD)
if [ "$HEAD" != "$BASE" ]; then
    echo "$PYDANTIC_SRC is at $HEAD, expected the upstream base commit $BASE" >&2
    exit 1
fi

if [ ! -x "$VENV/bin/python" ]; then
    python3 -m venv "$VENV"
fi
"$VENV/bin/pip" install -q -r "$ROOT/tools/requirements-record.txt"

PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$ROOT/tools:$PYDANTIC_SRC" "$VENV/bin/python" -m json_schema.record \
    "$ROOT/tests/json_schema/cases.json"
echo "cases written to $ROOT/tests/json_schema/cases.json"
