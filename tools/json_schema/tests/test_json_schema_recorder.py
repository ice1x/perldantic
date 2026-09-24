"""Tests for the pytest plugin recording pydantic's JSON Schema generation."""

import json
import warnings

import pytest
from faker import Faker
from pydantic import BaseModel, ConfigDict, RootModel, TypeAdapter
from pydantic.json_schema import GenerateJsonSchema

from json_schema import recorder
from json_schema.recorder import host_config, host_schema, renumber_ref_ids

pytest_plugins = ['pytester']


@pytest.fixture
def fake():
    Faker.seed(4321)
    return Faker()


def test_model_class_data_moves_into_the_schema(fake):
    doc = fake.sentence()
    title = fake.word()

    class Model(BaseModel):
        model_config = ConfigDict(title=title, json_schema_extra={'examples': [1]}, extra='forbid')
        x: int

    Model.__doc__ = f'\n    {doc}\n    '
    schema = host_schema(Model.__pydantic_core_schema__)

    assert schema['config'] == {
        'title': title,
        'extra_fields_behavior': 'forbid',
        'json_schema_extra': {'examples': [1]},
    }
    # the default `__get_pydantic_json_schema__` hook only calls the handler
    assert schema['metadata'] == {'pydantic_js_updates': {'description': doc}}


def test_typed_dict_class_data_moves_into_the_schema(fake):
    from typing_extensions import TypedDict, deprecated

    doc = fake.sentence()
    title_generator = str.upper

    @deprecated('old')
    class Movie(TypedDict):
        __pydantic_config__ = ConfigDict(model_title_generator=title_generator, extra='forbid')
        name: str

    Movie.__doc__ = doc
    schema = host_schema(TypeAdapter(Movie).core_schema)

    assert schema['config']['model_title_generator'] is title_generator
    assert schema['config']['extra_fields_behavior'] == 'forbid'
    assert schema['metadata']['pydantic_js_updates'] == {'description': doc, 'deprecated': True}


def test_docstring_is_left_out_when_json_schema_extra_sets_a_description(fake):
    class Model(BaseModel):
        """Docstring."""

        model_config = ConfigDict(json_schema_extra={'description': fake.sentence()})

    assert 'metadata' not in host_schema(Model.__pydantic_core_schema__)


def test_custom_hooks_and_callables_are_kept():
    class Model(BaseModel):
        model_config = ConfigDict(json_schema_extra=lambda schema: None)

        @classmethod
        def __get_pydantic_json_schema__(cls, core_schema, handler):
            return handler(core_schema)

    schema = host_schema(Model.__pydantic_core_schema__)
    assert callable(schema['config']['json_schema_extra'])
    assert len(schema['metadata']['pydantic_js_functions']) == 1


def test_root_model_field_extra_is_the_model_extra():
    class Root(RootModel[int]):
        root: int = 0

    Root.model_fields['root'].json_schema_extra = {'examples': [2]}
    assert host_schema(Root.__pydantic_core_schema__)['config']['json_schema_extra'] == {'examples': [2]}


def test_blank_config_title_is_kept():
    assert host_config({'title': ''}) == {'title': ''}


def test_ref_ids_are_renumbered_consistently():
    encoded = {
        'ref': 'm.A:9876543[m.B:1234567, int]',
        'schema': {'schema_ref': 'm.B:1234567', 'title': 'x:1234567'},
        'other': [{'ref': 'm.C:5555'}],
    }
    assert renumber_ref_ids(encoded) == {
        'ref': 'm.A:1[m.B:2, int]',
        'schema': {'schema_ref': 'm.B:2', 'title': 'x:1234567'},
        'other': [{'ref': 'm.C:3'}],
    }


def test_records_generate_calls_of_the_stock_generator(tmp_path, monkeypatch):
    monkeypatch.setattr(recorder, '_current_test', 'tests/test_x.py::test_a')
    monkeypatch.setattr(recorder, '_original_generate', GenerateJsonSchema.generate)
    monkeypatch.setattr(GenerateJsonSchema, 'generate', recorder.recording_generate)
    recorder._cases.clear()
    recorder._seen.clear()

    class Model(BaseModel):
        x: int

    Model.model_json_schema(by_alias=False)
    TypeAdapter(bytes, config=ConfigDict(ser_json_bytes='base64')).json_schema(mode='serialization')

    class Custom(GenerateJsonSchema):
        pass

    Model.model_json_schema(schema_generator=Custom)

    cases = recorder._cases['tests/test_x.py']
    assert [(c['mode'], c['options'], c['config']) for c in cases] == [
        ('validation', {'by_alias': False}, None),
        ('serialization', {}, {'ser_json_bytes': 'base64'}),
    ]
    assert cases[0]['schema']['cls'] == {'$class': 'Model'}
    assert cases[0]['schema']['ref'].endswith(':1')
    assert cases[1]['expected'] == {'json_schema': {'format': 'base64url', 'type': 'string'}}


def test_warnings_are_recorded_and_still_raised(monkeypatch):
    monkeypatch.setattr(recorder, '_current_test', 'tests/test_x.py::test_w')
    monkeypatch.setattr(recorder, '_original_generate', GenerateJsonSchema.generate)
    monkeypatch.setattr(GenerateJsonSchema, 'generate', recorder.recording_generate)
    recorder._cases.clear()
    recorder._seen.clear()

    class Model(BaseModel):
        b: bytes = b'\xfb'

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter('always')
        Model.model_json_schema()

    assert [str(w.message) for w in caught] == recorder._cases['tests/test_x.py'][0]['warnings']
    assert 'non-serializable-default' in caught[0].message.args[0]


def test_plugin_writes_cases_per_test_module(pytester):
    pytester.makepyfile(
        test_models="""
        from pydantic import BaseModel

        class M(BaseModel):
            a: int = 1

        def test_schema():
            M.model_json_schema()
            M.model_json_schema()
        """
    )
    out = pytester.path / 'out'
    result = pytester.runpytest('-p', 'json_schema.recorder', f'--json-schema-out={out}')
    result.assert_outcomes(passed=1)
    cases = json.loads((out / 'test_models.json').read_text())
    assert [c['id'] for c in cases] == ['test_models.py::test_schema#0']
    assert cases[0]['expected']['json_schema']['properties'] == {
        'a': {'default': 1, 'title': 'A', 'type': 'integer'}
    }
