"""Keep only conformance cases that are identical across two independent recording runs.

Some upstream tests depend on the current time (e.g. `now()`-relative datetimes) or other
run-specific state; their recordings change every run. Recording twice and intersecting the
results keeps the committed cases reproducible without a hand-maintained denylist.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def merge_stable(run_a: Path, run_b: Path, out_dir: Path) -> list[str]:
    """Write cases present and identical in both runs to `out_dir`; return the dropped ids."""
    dropped: list[str] = []
    for file_a in sorted(run_a.rglob('*.json')):
        relative = file_a.relative_to(run_a)
        cases_a = json.loads(file_a.read_text())
        file_b = run_b / relative
        cases_b = {c['id']: c for c in json.loads(file_b.read_text())} if file_b.exists() else {}
        kept = []
        for case in cases_a:
            if cases_b.get(case['id']) == case:
                kept.append(case)
            else:
                dropped.append(case['id'])
        if kept:
            target = out_dir / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps(kept, indent=1, ensure_ascii=False) + '\n')
    return sorted(dropped)


if __name__ == '__main__':
    a, b, out = (Path(p) for p in sys.argv[1:4])
    removed = merge_stable(a, b, out)
    print(f'dropped {len(removed)} non-reproducible cases')
    for case_id in removed:
        print(f'  {case_id}')
