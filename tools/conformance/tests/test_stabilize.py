"""Tests for keeping only cases that are identical across two recording runs."""

import json

from conformance.stabilize import merge_stable


def write(path, cases):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(cases))


def test_keeps_identical_cases_and_drops_changing_ones(tmp_path):
    same = {'id': 'a#0', 'input': 1, 'expected': {'output': 1}}
    moving_a = {'id': 'b#0', 'input': {'$datetime': '2026-01-01T00:00:00'}, 'expected': {'output': 1}}
    moving_b = {'id': 'b#0', 'input': {'$datetime': '2026-01-01T00:00:01'}, 'expected': {'output': 1}}
    only_a = {'id': 'c#0', 'input': 2, 'expected': {'output': 2}}
    write(tmp_path / 'a' / 'tests' / 't.json', [same, moving_a, only_a])
    write(tmp_path / 'b' / 'tests' / 't.json', [same, moving_b])

    dropped = merge_stable(tmp_path / 'a', tmp_path / 'b', tmp_path / 'out')

    assert json.loads((tmp_path / 'out' / 'tests' / 't.json').read_text()) == [same]
    assert dropped == ['b#0', 'c#0']


def test_files_left_empty_are_not_written(tmp_path):
    write(tmp_path / 'a' / 'x.json', [{'id': 'x#0', 'input': 1}])
    write(tmp_path / 'b' / 'x.json', [{'id': 'x#0', 'input': 2}])
    merge_stable(tmp_path / 'a', tmp_path / 'b', tmp_path / 'out')
    assert not (tmp_path / 'out' / 'x.json').exists()


def test_output_is_formatted_like_the_recorder(tmp_path):
    case = {'id': 'a#0', 'input': 'é'}
    write(tmp_path / 'a' / 'x.json', [case])
    write(tmp_path / 'b' / 'x.json', [case])
    merge_stable(tmp_path / 'a', tmp_path / 'b', tmp_path / 'out')
    assert (tmp_path / 'out' / 'x.json').read_text() == json.dumps([case], indent=1, ensure_ascii=False) + '\n'
