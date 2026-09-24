"""Tests for the tagged JSON encoding of Python values used by conformance cases."""

import datetime
import decimal
import enum
import json
import math
import uuid

import pytest
from faker import Faker
from pydantic_core import MultiHostUrl, Url
from hypothesis import given
from hypothesis import strategies as st

from conformance.encoding import UnsupportedValue, decode, encode

Faker.seed(20260923)
fake = Faker()

# Values the Rust decoder understands, recursively.
scalars = (
    st.none()
    | st.booleans()
    | st.integers()
    | st.floats(allow_nan=False)
    | st.text()
    | st.binary()
)
values = st.recursive(
    scalars,
    lambda children: (
        st.lists(children, max_size=4)
        | st.lists(children, max_size=4).map(tuple)
        | st.dictionaries(st.text(), children, max_size=4)
        | st.dictionaries(st.integers(), children, max_size=4)
    ),
    max_leaves=12,
)


@given(values)
def test_round_trip_through_json(value):
    wire = json.loads(json.dumps(encode(value)))
    assert decode(wire) == value
    assert type(decode(wire)) is type(value)


@given(values)
def test_encoding_is_deterministic(value):
    assert json.dumps(encode(value)) == json.dumps(encode(value))


def test_plain_json_types_are_untagged():
    record = {'name': fake.name(), 'age': 42, 'score': 1.5, 'tags': [fake.word()], 'x': None, 'ok': True}
    assert encode(record) == record


@pytest.mark.parametrize(
    ('value', 'wire'),
    [
        ((1, 'a'), {'$tuple': [1, 'a']}),
        (b'\x00ab', {'$bytes': 'AGFi'}),
        (math.inf, {'$float': 'inf'}),
        (-math.inf, {'$float': '-inf'}),
        ({1: 'x'}, {'$dict': [[1, 'x']]}),
        ({'$ref': 1}, {'$dict': [['$ref', 1]]}),
        ({3, 1, 2}, {'$set': [1, 2, 3]}),
        (frozenset({'b', 'a'}), {'$frozenset': ['a', 'b']}),
        (decimal.Decimal('1.50'), {'$decimal': '1.50'}),
        (datetime.date(2026, 9, 23), {'$date': '2026-09-23'}),
        (datetime.time(12, 30), {'$time': '12:30:00'}),
        (datetime.datetime(2026, 9, 23, 12, 0), {'$datetime': '2026-09-23T12:00:00'}),
        (datetime.timedelta(days=1, seconds=2), {'$timedelta': [1, 2, 0]}),
        (uuid.UUID(int=1), {'$uuid': '00000000-0000-0000-0000-000000000001'}),
        (Url('https://example.com'), {'$url': 'https://example.com/'}),
        (MultiHostUrl('postgres://u:p@h1:5432,h2/db'), {'$multi_host_url': 'postgres://u:p@h1:5432,h2/db'}),
    ],
)
def test_tagged_values(value, wire):
    assert encode(value) == wire
    assert decode(wire) == value


def test_nan_is_tagged():
    wire = encode(math.nan)
    assert wire == {'$float': 'nan'}
    assert math.isnan(decode(wire))


def test_int_keys_and_str_keys_mix():
    assert encode({'a': 1, 2: 'b'}) == {'$dict': [['a', 1], [2, 'b']]}


def test_opaque_python_objects_are_described_not_decoded():
    class Color(enum.Enum):
        RED = 1

    def validator(value):
        return value

    assert encode(Color.RED) == {'$enum': ['Color', 1]}
    assert encode(Color) == {'$class': 'Color'}
    assert encode(validator) == {'$function': 'validator'}
    assert encode(ValueError('boom')) == {'$exception': ['ValueError', 'boom']}
    assert encode(object()) == {'$object': 'object'}
    for wire in ({'$enum': ['Color', 1]}, {'$function': 'f'}, {'$object': 'object'}):
        with pytest.raises(UnsupportedValue):
            decode(wire)


def test_model_instances_keep_fields_and_fields_set():
    class Model:
        pass

    m = Model()
    m.__dict__.update({'a': 1, 'b': 'x'})
    m.__pydantic_fields_set__ = {'b', 'a'}
    m.__pydantic_extra__ = None
    m.__pydantic_private__ = None
    assert encode(m) == {'$model': {'class': 'Model', 'fields': {'a': 1, 'b': 'x'}, 'fields_set': ['a', 'b'], 'extra': None}}


def test_unknown_tag_is_rejected():
    with pytest.raises(UnsupportedValue):
        decode({'$nope': 1})


def test_strings_with_unpaired_surrogates_are_opaque():
    # Neither UTF-8 JSON nor a Rust String can hold these.
    wire = encode('a\ud800b')
    assert wire == {'$object': 'str-with-surrogates'}
    json.dumps(wire, ensure_ascii=False).encode('utf-8')
    assert encode({'k': '\udfff'}) == {'k': {'$object': 'str-with-surrogates'}}
    assert encode({'\udfff': 1}) == {'$dict': [[{'$object': 'str-with-surrogates'}, 1]]}


def test_values_that_fail_to_serialize_are_opaque():
    class BrokenTz(datetime.tzinfo):
        def utcoffset(self, dt):
            raise NotImplementedError('a tzinfo subclass must implement utcoffset()')

    value = datetime.datetime(2026, 1, 1, tzinfo=BrokenTz())
    assert encode(value) == {'$object': 'datetime'}


def test_urls_are_encoded_by_their_text():
    host = fake.domain_name()
    url = Url(f'https://{host}/path?q=1')
    assert encode(url) == {'$url': f'https://{host}/path?q=1'}
    assert type(decode(encode(url))) is Url
    assert type(decode(encode(MultiHostUrl(f'redis://{host}')))) is MultiHostUrl
