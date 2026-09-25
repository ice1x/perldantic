"""Tests for the upstream sync report: what changed in pydantic-core since the base commit,
mapped to where Perldantic ported it."""

import subprocess

import pytest
from faker import Faker
from hypothesis import given
from hypothesis import strategies as st

from upstream.report import FileRow, build_report, changed_files, parse_base, parse_file_map

Faker.seed(20260925)
fake = Faker()

UPSTREAM_MD = """# Upstream

| | |
|---|---|
| Repository | https://github.com/pydantic/pydantic (directory `pydantic-core/`) |
| Base commit | `{base}` |
| pydantic-core version | `2.49.0` |

## File map

| Upstream file | Perldantic file | Priority | Status | Notes |
|---|---|---|---|---|
| `validators/int.rs` | `crates/perldantic-core/src/validators/int.rs` | P0 | ported | |
| `validators/arguments_v3.rs` | `crates/perldantic-core/src/validators/arguments_v3.rs` | P2 | pending |  |
| `common/deque.rs` | - | drop | dropped | Python-only type |
"""


def test_the_base_commit_and_the_file_map_are_read():
    text = UPSTREAM_MD.format(base='0384c970e37a59b344e75161eb106ea9996378ba')
    assert parse_base(text) == '0384c970e37a59b344e75161eb106ea9996378ba'
    rows = parse_file_map(text)
    assert rows['validators/int.rs'] == FileRow(
        'validators/int.rs', 'crates/perldantic-core/src/validators/int.rs', 'P0', 'ported')
    assert rows['common/deque.rs'].target is None
    assert set(rows) == {'validators/int.rs', 'validators/arguments_v3.rs', 'common/deque.rs'}


def test_a_file_without_a_base_commit_is_an_error():
    with pytest.raises(ValueError, match='Base commit'):
        parse_base('# Upstream\n')


paths = st.lists(
    st.from_regex(r'[a-z_]{1,8}(/[a-z_]{1,8})?\.rs', fullmatch=True), min_size=1, max_size=6, unique=True)


@given(paths)
def test_every_row_of_a_map_is_read_back(names):
    table = '\n'.join(f'| `{n}` | `crates/x/{n}` | P1 | partial | note |' for n in names)
    text = UPSTREAM_MD.format(base='a' * 40).split('## File map')[0] + (
        '## File map\n\n| Upstream file | Perldantic file | Priority | Status | Notes |\n'
        '|---|---|---|---|---|\n' + table + '\n')
    rows = parse_file_map(text)
    assert sorted(rows) == sorted(names)
    assert all(rows[n].target == f'crates/x/{n}' and rows[n].status == 'partial' for n in names)


def git(repo, *args):
    return subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True, text=True).stdout


@pytest.fixture
def repo(tmp_path):
    repo = tmp_path / 'pydantic'
    repo.mkdir()
    git(repo, 'init', '-q', '-b', 'main')
    git(repo, 'config', 'user.email', fake.email())
    git(repo, 'config', 'user.name', fake.name())
    return repo


def commit(repo, files, message):
    for path, text in files.items():
        target = repo / path
        if text is None:
            target.unlink()
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text)
    git(repo, 'add', '-A')
    git(repo, 'commit', '-q', '-m', message)
    return git(repo, 'rev-parse', 'HEAD').strip()


def test_changes_under_pydantic_core_are_listed(repo):
    base = commit(repo, {
        'pydantic-core/src/validators/int.rs': 'fn int() {}\n' * 20,
        'pydantic-core/src/common/deque.rs': 'deque\n',
        'pydantic-core/tests/validators/test_int.py': 'def test(): pass\n',
        'pydantic/main.py': 'x = 1\n',
    }, 'base')
    head = commit(repo, {
        'pydantic-core/src/validators/int.rs': 'fn int() {}\n' * 20 + 'fn more() {}\n',
        'pydantic-core/src/common/deque.rs': None,
        'pydantic-core/src/validators/complex_new.rs': 'new\n',
        'pydantic-core/tests/validators/test_int.py': 'def test(): assert 1\n',
        'pydantic/main.py': 'x = 2\n',
    }, fake.sentence())
    changes = changed_files(repo, base, head)
    assert changes == [
        ('D', 'src/common/deque.rs'),
        ('A', 'src/validators/complex_new.rs'),
        ('M', 'src/validators/int.rs'),
        ('M', 'tests/validators/test_int.py'),
    ], 'only pydantic-core, relative to it'


def test_renames_are_reported_under_the_old_name(repo):
    base = commit(repo, {'pydantic-core/src/a.rs': 'fn a() {}\n' * 30}, 'base')
    head = commit(repo, {'pydantic-core/src/a.rs': None, 'pydantic-core/src/b.rs': 'fn a() {}\n' * 30}, 'move')
    assert changed_files(repo, base, head) == [('R', 'src/a.rs -> src/b.rs')]


def test_the_report_maps_changes_to_the_port():
    rows = parse_file_map(UPSTREAM_MD.format(base='b' * 40))
    report = build_report(
        [('M', 'src/validators/int.rs'), ('D', 'src/common/deque.rs'), ('A', 'src/validators/new.rs'),
         ('M', 'src/validators/arguments_v3.rs'), ('M', 'tests/validators/test_int.py')],
        rows, base='b' * 40, head='c' * 40, pinned='2.49.0', latest='2.50.1')
    assert '`bbbbbbbbb`..`ccccccccc`' in report
    assert 'pydantic-core 2.49.0 is pinned; the latest release is 2.50.1' in report
    assert '| modified | `validators/int.rs` | `crates/perldantic-core/src/validators/int.rs` | ported |' in report
    assert '| deleted | `common/deque.rs` | - | dropped |' in report
    assert '| modified | `validators/arguments_v3.rs` | `crates/perldantic-core/src/validators/arguments_v3.rs` | pending |' in report
    assert '| added | `validators/new.rs` | not in the file map | |' in report
    assert '`tests/validators/test_int.py`' in report
    assert 'Re-record the conformance cases' in report


def test_ported_files_come_first():
    text = UPSTREAM_MD.format(base='b' * 40) + '| `url.rs` | `crates/perldantic-core/src/url.rs` | P1 | done | |\n'
    rows = parse_file_map(text)
    report = build_report([('M', 'src/common/deque.rs'), ('M', 'src/validators/arguments_v3.rs'), ('M', 'src/url.rs'),
                           ('M', 'src/validators/int.rs')], rows, base='b' * 40, head='c' * 40)
    order = [report.index(name) for name in ('url.rs', 'validators/int.rs', 'arguments_v3.rs', 'common/deque.rs')]
    assert order == sorted(order), 'what has a port to update first; done counts as ported'


def test_no_changes():
    report = build_report([], {}, base='b' * 40, head='c' * 40, pinned='2.49.0', latest='2.49.0')
    assert 'No changes to pydantic-core since the base commit' in report
    assert 'pydantic-core 2.49.0 is pinned and is the latest release' in report
