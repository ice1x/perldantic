"""Tests for recording JSON Schema cases, with a stand-in generator instead of pydantic."""

import json
import math
import warnings

import pytest
from faker import Faker
from pydantic_core import core_schema as cs

from json_schema.record import cases, model_class, record, record_case, write


@pytest.fixture
def fake():
    Faker.seed(1234)
    return Faker()


def test_records_the_schema_options_and_encoded_output(fake):
    name = fake.pystr()
    schema = cs.model_schema(model_class(name), cs.model_fields_schema({}))

    def generate(options, schema, mode):
        return {'title': schema['cls'].__name__, 'default': math.inf, 'mode': mode, **options}

    case = record_case(generate, 'c', schema, mode='serialization', by_alias=False)

    assert case['schema']['cls'] == {'$class': name}
    assert case['options'] == {'by_alias': False}
    assert case['expected'] == {'json_schema': {
        'title': name, 'default': {'$float': 'inf'}, 'mode': 'serialization', 'by_alias': False,
    }}
    assert case['warnings'] == []


def test_records_errors_and_warnings(fake):
    message = fake.sentence()

    def generate(options, schema, mode):
        warnings.warn(message)
        raise KeyError('x')

    case = record_case(generate, 'c', cs.int_schema())

    assert case['expected'] == {'error': ['KeyError', "'x'"]}
    assert case['warnings'] == [message]


def test_case_ids_are_unique():
    ids = [case_id for case_id, _, _ in cases()]
    assert len(ids) == len(set(ids))


def test_writes_every_case(tmp_path):
    out = tmp_path / 'nested' / 'cases.json'
    write(record(lambda options, schema, mode: {}), out)
    recorded = json.loads(out.read_text())
    assert [c['id'] for c in recorded] == [case_id for case_id, _, _ in cases()]
