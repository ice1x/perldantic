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


SERIALIZER_TESTS = '''
import pytest
from pydantic_core import PydanticSerializationError, SchemaSerializer


def test_to_python_and_json():
    s = SchemaSerializer({'type': 'int'})
    assert s.to_python(1) == 1
    assert s.to_python(1, mode='json') == 1
    assert s.to_json(1) == b'1'


def test_bytes_from_to_python_are_values():
    assert SchemaSerializer({'type': 'bytes'}).to_python(b'ab') == b'ab'


def test_warnings_are_recorded_and_still_raised():
    s = SchemaSerializer({'type': 'int'})
    with pytest.warns(UserWarning, match='Expected `int`'):
        assert s.to_python('x') == 'x'


def test_errors():
    with pytest.raises(PydanticSerializationError):
        SchemaSerializer({'type': 'any'}).to_json(object())
'''


def test_records_serializer_calls(pytester):
    pytester.makepyfile(test_ser=SERIALIZER_TESTS)
    out = pytester.path / 'cases'
    result = pytester.runpytest('-p', 'conformance.recorder', f'--conformance-out={out}', '-q')
    result.assert_outcomes(passed=4)
    cases = json.loads((out / 'test_ser.json').read_text())
    by_test = {}
    for case in cases:
        by_test.setdefault(case['test'].split('::')[-1], []).append(case)

    python_case, python_json_case, json_case = by_test['test_to_python_and_json']
    assert python_case['mode'] == 'to_python'
    assert python_case['schema'] == {'type': 'int'}
    assert python_case['input'] == 1
    assert python_case['options'] == {}
    assert python_case['expected'] == {'output': 1}
    assert python_json_case['options'] == {'mode': 'json'}
    assert json_case['mode'] == 'to_json'
    # JSON output is recorded as text.
    assert json_case['expected'] == {'json': '1'}

    (bytes_case,) = by_test['test_bytes_from_to_python_are_values']
    assert bytes_case['expected'] == {'output': {'$bytes': 'YWI='}}

    (warn_case,) = by_test['test_warnings_are_recorded_and_still_raised']
    assert warn_case['expected']['output'] == 'x'
    assert warn_case['expected']['warnings'] == [
        'Pydantic serializer warnings:\n  PydanticSerializationUnexpectedValue(Expected `int` - '
        "serialized value may not be as expected [input_value='x', input_type=str])"
    ]

    (error_case,) = by_test['test_errors']
    assert error_case['expected'] == {
        'exception': {
            'type': 'PydanticSerializationError',
            'message': "Unable to serialize unknown type: <class 'object'>",
        }
    }
