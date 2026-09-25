"""What changed in pydantic-core since Perldantic's base commit, mapped to the port.

    python -m upstream.report --pydantic ../pydantic [--head origin/main] [--latest 2.50.1]

reads upstream/UPSTREAM.md (the base commit and the file map), lists the files changed under
`pydantic-core/` between the base commit and `--head` in a pydantic checkout, and writes a
Markdown report: changed source files with the Perldantic file each is ported to and its
status (the ported ones first: they need the change too), files the map does not know, and
changed tests (the conformance cases need re-recording). The Upstream workflow keeps it in an
issue (.github/workflows/upstream.yml); the sync itself follows UPSTREAM.md.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

CORE = 'pydantic-core'
ROOT = Path(__file__).resolve().parents[2]

# Change letters of `git diff --name-status`, as words.
KIND = {'A': 'added', 'M': 'modified', 'D': 'deleted', 'R': 'renamed', 'T': 'type changed'}
# The order the source table lists statuses in: what has a port to update first.
STATUS_ORDER = {'ported': 0, 'done': 0, 'partial': 1, 'pending': 2, 'dropped': 3}


@dataclass(frozen=True)
class FileRow:
    upstream: str
    target: str | None
    priority: str
    status: str


def parse_base(text: str) -> str:
    match = re.search(r'^\|\s*Base commit\s*\|\s*`([0-9a-f]{7,40})`', text, re.M)
    if not match:
        raise ValueError('UPSTREAM.md names no Base commit')
    return match.group(1)


def parse_version(text: str) -> str | None:
    match = re.search(r'^\|\s*pydantic-core version\s*\|\s*`([^`]+)`', text, re.M)
    return match.group(1) if match else None


def parse_file_map(text: str) -> dict[str, FileRow]:
    """The rows of the "File map" table, by upstream file (relative to `pydantic-core/src`)."""
    _, _, section = text.partition('## File map')
    rows = {}
    for line in section.splitlines():
        cells = [cell.strip() for cell in line.strip().strip('|').split('|')]
        if len(cells) < 4 or not cells[0].startswith('`'):
            continue
        upstream = cells[0].strip('`')
        target = cells[1].strip('`')
        rows[upstream] = FileRow(upstream, None if target in ('-', '') else target, cells[2], cells[3])
    return rows


def changed_files(repo: Path, base: str, head: str) -> list[tuple[str, str]]:
    """(change letter, path relative to pydantic-core) of each file changed between two commits;
    a rename is `old -> new`."""
    out = subprocess.run(
        ['git', '-C', str(repo), 'diff', '--name-status', '-M', base, head, '--', f'{CORE}/'],
        check=True, capture_output=True, text=True,
    ).stdout
    changes = []
    for line in out.splitlines():
        parts = line.split('\t')
        kind = parts[0][0]
        paths = [p.removeprefix(f'{CORE}/') for p in parts[1:]]
        changes.append((kind, ' -> '.join(paths)))
    return sorted(changes, key=lambda change: change[1])


def _source(path: str) -> str | None:
    """The file-map name of a source path (`src/x.rs` -> `x.rs`)."""
    return path.removeprefix('src/') if path.startswith('src/') else None


def _release_line(pinned: str | None, latest: str | None) -> str | None:
    if not pinned or not latest:
        return None
    if pinned == latest:
        return f'pydantic-core {pinned} is pinned and is the latest release.'
    return (f'pydantic-core {pinned} is pinned; the latest release is {latest} '
            '(the conformance cases are recorded against the pinned release).')


def build_report(changes, rows, *, base: str, head: str, pinned: str | None = None,
                 latest: str | None = None) -> str:
    lines = [f'Upstream changes to `{CORE}/` in `{base[:9]}`..`{head[:9]}`.', '']
    release = _release_line(pinned, latest)
    if release:
        lines += [release, '']
    if not changes:
        lines.append('No changes to pydantic-core since the base commit.')
        return '\n'.join(lines) + '\n'

    sources, tests, others = [], [], []
    for kind, path in changes:
        old = path.split(' -> ')[0]
        if _source(old) is not None:
            sources.append((kind, path))
        elif old.startswith('tests/'):
            tests.append(path)
        else:
            others.append(path)

    if sources:
        def order(change):
            row = rows.get(_source(change[1].split(' -> ')[0]))
            return (STATUS_ORDER.get(row.status, 9) if row else 10, change[1])

        lines += ['## Source files', '', '| Change | Upstream file | Perldantic file | Status |',
                  '|---|---|---|---|']
        for kind, path in sorted(sources, key=order):
            names = [_source(p) or p for p in path.split(' -> ')]
            row = rows.get(names[0])
            shown = ' -> '.join(f'`{name}`' for name in names)
            if row:
                target = f'`{row.target}`' if row.target else '-'
                lines.append(f'| {KIND.get(kind, kind)} | {shown} | {target} | {row.status} |')
            else:
                lines.append(f'| {KIND.get(kind, kind)} | {shown} | not in the file map | |')
        lines.append('')
    if tests:
        lines += ['## Tests', '',
                  'Re-record the conformance cases (`tools/conformance/record.sh`) and review their diff.', '']
        lines += [f'- `{path}`' for path in tests]
        lines.append('')
    if others:
        lines += ['## Other files', ''] + [f'- `{path}`' for path in others] + ['']
    lines.append('Sync as `upstream/UPSTREAM.md` describes.')
    return '\n'.join(lines) + '\n'


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--pydantic', required=True, type=Path, help='a pydantic checkout')
    parser.add_argument('--head', default='HEAD', help='the upstream commit to compare with')
    parser.add_argument('--latest', help='the latest pydantic-core release, for the report')
    parser.add_argument('--upstream-md', type=Path, default=ROOT / 'upstream' / 'UPSTREAM.md')
    args = parser.parse_args(argv)

    text = args.upstream_md.read_text()
    base = parse_base(text)
    head = subprocess.run(['git', '-C', str(args.pydantic), 'rev-parse', args.head],
                          check=True, capture_output=True, text=True).stdout.strip()
    report = build_report(changed_files(args.pydantic, base, head), parse_file_map(text),
                          base=base, head=head, pinned=parse_version(text), latest=args.latest)
    sys.stdout.write(report)
    return 0


if __name__ == '__main__':
    sys.exit(main())
