"""Integration tests for the recording pytest plugin, run through pytest's own `pytester`."""

import json

pytest_plugins = ['pytester']

SAMPLE_TESTS = '''
import pytest
from pydantic_core import SchemaValidator, ValidationError


@pytest.mark.parametrize('value', ['42', 7])
def test_int(value):
    SchemaValidator({'type': 'int'}).validate_python(value)


def test_errors_and_json():
    v = SchemaValidator({'type': 'str', 'min_length': 3}, {'title': 'Short'})
    with pytest.raises(ValidationError):
        v.validate_python('ab')
    assert v.validate_json('"abcd"', strict=True) == 'abcd'


def test_tuple_input_and_duplicate_calls():
    v = SchemaValidator({'type': 'list', 'items_schema': {'type': 'int'}})
    assert v.validate_python((1, 2)) == [1, 2]
    assert v.validate_python((1, 2)) == [1, 2]


def test_function_schema():
    v = SchemaValidator({'type': 'function-plain', 'function': {'type': 'no-info', 'function': abs}})
    v.validate_python(-1)
'''


def run(pytester):
    pytester.makepyfile(test_sample=SAMPLE_TESTS)
    out = pytester.path / 'cases'
    result = pytester.runpytest('-p', 'conformance.recorder', f'--conformance-out={out}', '-q')
    result.assert_outcomes(passed=5)
    return json.loads((out / 'test_sample.json').read_text())


def test_records_python_and_json_calls_with_results(pytester):
    cases = run(pytester)
    by_test = {}
    for case in cases:
        by_test.setdefault(case['test'].split('::')[-1], []).append(case)

    assert [c['input'] for c in by_test['test_int[42]']] == ['42']
    int_case = by_test['test_int[42]'][0]
    assert int_case['schema'] == {'type': 'int'}
    assert int_case['config'] is None
    assert int_case['mode'] == 'python'
    assert int_case['options'] == {}
    assert int_case['expected'] == {'output': 42}

    errors_case, json_case = by_test['test_errors_and_json']
    assert errors_case['config'] == {'title': 'Short'}
    assert errors_case['expected']['title'] == 'Short'
    assert errors_case['expected']['errors'] == [
        {
            'type': 'string_too_short',
            'loc': [],
            'msg': 'String should have at least 3 characters',
            'input': 'ab',
            'ctx': {'min_length': 3},
        }
    ]
    assert json_case['mode'] == 'json'
    assert json_case['input'] == '"abcd"'
    assert json_case['options'] == {'strict': True}
    assert json_case['expected'] == {'output': 'abcd'}


def test_values_are_tagged_and_duplicates_dropped(pytester):
    cases = run(pytester)
    tuple_cases = [c for c in cases if c['test'].endswith('test_tuple_input_and_duplicate_calls')]
    assert len(tuple_cases) == 1
    assert tuple_cases[0]['input'] == {'$tuple': [1, 2]}
    assert tuple_cases[0]['expected'] == {'output': [1, 2]}


def test_schemas_with_python_callables_are_marked(pytester):
    cases = run(pytester)
    (fn_case,) = [c for c in cases if c['test'].endswith('test_function_schema')]
    assert fn_case['schema']['function'] == {'type': 'no-info', 'function': {'$function': 'abs'}}
    assert fn_case['expected'] == {'output': 1}


def test_cases_are_sorted_and_stable(pytester):
    first = run(pytester)
    ids = [c['id'] for c in first]
    assert ids == sorted(ids)
    assert len(set(ids)) == len(ids)


def test_raw_json_input_with_surrogates_is_encoded(pytester):
    pytester.makepyfile(
        test_surrogate=r'''
import pytest
from pydantic_core import SchemaValidator, ValidationError

def test_json_surrogate():
    with pytest.raises(Exception):
        SchemaValidator({'type': 'str'}).validate_json('"a\\udc80"'.encode().decode('unicode_escape'))
'''
    )
    out = pytester.path / 'cases'
    pytester.runpytest('-p', 'conformance.recorder', f'--conformance-out={out}', '-q')
    (case,) = json.loads((out / 'test_surrogate.json').read_text())
    assert case['input'] == {'$object': 'str-with-surrogates'}


def test_proxy_is_transparent_for_repr_and_pickle(pytester):
    pytester.makepyfile(
        test_transparent='''
import pickle
from pydantic_core import SchemaValidator

def test_repr_and_pickle():
    v = SchemaValidator({'type': 'int'})
    assert repr(v).startswith('SchemaValidator(')
    assert pickle.loads(pickle.dumps(v)).validate_python('1') == 1
'''
    )
    out = pytester.path / 'cases'
    result = pytester.runpytest('-p', 'conformance.recorder', f'--conformance-out={out}', '-q')
    result.assert_outcomes(passed=1)
