#!/bin/sh
# Re-record the conformance cases in tests/conformance/upstream/ by running upstream
# pydantic-core's validator tests against the matching pydantic-core release.
#
# The suite is recorded twice and only cases identical in both runs are kept, so time-dependent
# tests do not make the committed cases change between recordings.
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
VENV="$ROOT/tools/.venv"
OUT="$ROOT/tests/conformance/upstream"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$VENV/bin/python" ]; then
    python3 -m venv "$VENV"
fi
"$VENV/bin/pip" install -q -r "$ROOT/tools/requirements-record.txt"

cd "$ROOT/upstream/pydantic-core"
for run in a b; do
    # Some upstream tests cover schema types newer than the published release and fail against
    # it; their outcomes are still recorded as the release behaves, so failures are tolerated.
    # A fixed hash seed makes set iteration order (and so set -> list outputs) reproducible.
    PYTHONHASHSEED=0 PYTHONDONTWRITEBYTECODE=1 PYTHONPATH="$ROOT/tools" "$VENV/bin/python" -m pytest tests/validators \
        -q -p no:cacheprovider -p conformance.recorder --conformance-out="$WORK/$run" >"$WORK/$run.log" 2>&1 || true
    tail -1 "$WORK/$run.log"
done

rm -rf "$OUT"
PYTHONPATH="$ROOT/tools" "$VENV/bin/python" -m conformance.stabilize "$WORK/a" "$WORK/b" "$OUT"
echo "cases written to $OUT"
