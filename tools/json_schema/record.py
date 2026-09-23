"""Record JSON Schema cases: what pydantic's `GenerateJsonSchema` produces for core schemas.

JSON Schema generation lives in pydantic's Python layer (`pydantic/json_schema.py`), not in
pydantic-core, so the oracle is pydantic itself at the upstream base commit; `record.sh` puts
that checkout on the import path. Each recorded case is:

    {"id", "schema", "mode", "options",
     "expected": {"json_schema": ...} | {"error": [type_name, message]},
     "warnings": [message, ...]}

Schemas and outputs use the conformance value encoding (`conformance.encoding`); a model's
`cls` is recorded as `{"$class": name}`.
"""

from __future__ import annotations

import json
import math
import sys
import warnings
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any

from pydantic_core import core_schema as cs

from conformance.encoding import encode

Generate = Callable[[dict[str, Any], Any, str], Any]


def model_class(name: str, **config: Any) -> type:
    """A stand-in model class: `GenerateJsonSchema` only reads its name, config and docstring."""
    return type(name, (), {'model_config': config, '__doc__': None})


def record_case(
    generate: Generate, case_id: str, schema: Any, mode: str = 'validation', **options: Any
) -> dict[str, Any]:
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter('always')
        try:
            expected = {'json_schema': encode(generate(options, schema, mode))}
        except Exception as e:  # noqa: BLE001 - the error is the recorded outcome
            expected = {'error': [type(e).__name__, str(e)]}
    return {
        'id': case_id,
        'schema': encode(schema),
        'mode': mode,
        'options': options,
        'expected': expected,
        'warnings': [str(w.message) for w in caught],
    }


def pydantic_generate(options: dict[str, Any], schema: Any, mode: str) -> Any:
    from pydantic.json_schema import GenerateJsonSchema

    return GenerateJsonSchema(**options).generate(schema, mode=mode)


def cases() -> Iterator[tuple[str, Any, dict[str, Any]]]:
    """(id, core schema, keyword arguments of `record_case`) for every recorded case."""
    f = cs.model_field

    yield 'scalars', cs.tuple_schema([
        cs.any_schema(), cs.none_schema(), cs.bool_schema(),
        cs.int_schema(ge=1, lt=10, multiple_of=2), cs.float_schema(le=math.inf, gt=0.5),
        cs.str_schema(min_length=1, max_length=5, pattern='^a'), cs.bytes_schema(max_length=3),
    ]), {}
    yield 'literals', cs.tuple_schema([
        cs.literal_schema(['a']), cs.literal_schema([1, 2]), cs.literal_schema(['a', 1]),
        cs.literal_schema([None]), cs.literal_schema([True, False]), cs.literal_schema([1.5]),
        cs.literal_schema([b'x']), cs.literal_schema([[1], [2]]),
    ]), {}
    yield 'arrays', cs.tuple_schema([
        cs.list_schema(cs.int_schema(), max_length=3), cs.list_schema(),
        cs.tuple_schema([cs.int_schema(), cs.str_schema()]),
        cs.tuple_schema([cs.int_schema()], variadic_item_index=0),
        cs.tuple_schema([cs.str_schema(), cs.int_schema()], variadic_item_index=1, min_length=2),
        cs.tuple_schema([cs.int_schema(), cs.str_schema(), cs.bool_schema()], variadic_item_index=1),
        cs.tuple_schema([]), cs.set_schema(cs.str_schema(), min_length=1),
    ]), {}
    yield 'dicts', cs.tuple_schema([
        cs.dict_schema(cs.str_schema(), cs.int_schema()), cs.dict_schema(),
        cs.dict_schema(cs.str_schema(pattern='^x'), cs.int_schema(), max_length=2),
        cs.dict_schema(cs.str_schema(max_length=3)), cs.dict_schema(cs.int_schema(), cs.any_schema()),
    ]), {}
    yield 'unions', cs.tuple_schema([
        cs.nullable_schema(cs.int_schema()), cs.nullable_schema(cs.none_schema()),
        cs.union_schema([cs.int_schema(), cs.str_schema()]),
        cs.union_schema([cs.int_schema(), cs.nullable_schema(cs.str_schema())]),
        cs.union_schema([(cs.int_schema(), 'i'), (cs.int_schema(), 'j')]),
        cs.union_schema([cs.bool_schema()]), cs.union_schema([]),
        cs.lax_or_strict_schema(cs.int_schema(), cs.str_schema()),
        cs.lax_or_strict_schema(cs.int_schema(), cs.str_schema(), strict=True),
        cs.custom_error_schema(cs.int_schema(), custom_error_type='e', custom_error_message='m'),
    ]), {}
    yield 'primitive_type_array', cs.tuple_schema([
        cs.nullable_schema(cs.int_schema()),
        cs.union_schema([cs.int_schema(), cs.str_schema(), cs.int_schema()]),
        cs.union_schema([cs.int_schema(ge=1), cs.str_schema()]),
        cs.union_schema([cs.list_schema(), cs.str_schema()]),
    ]), {'union_format': 'primitive_type_array'}

    user = cs.model_schema(
        model_class('User', title='The User', extra='forbid', json_schema_extra={'examples': [{'id': 1}]}),
        cs.model_fields_schema(
            {
                'id': f(cs.int_schema()),
                'full_name': f(cs.with_default_schema(cs.str_schema(), default='x'),
                               validation_alias='fullName', serialization_alias='FULL'),
                'nick': f(cs.with_default_schema(cs.nullable_schema(cs.str_schema()), default=None),
                          validation_alias=[['nk'], ['n', 0]]),
                'tags': f(cs.with_default_schema(cs.set_schema(cs.int_schema()), default={3, 1, 2})),
                'raw': f(cs.with_default_schema(cs.bytes_schema(), default=b'ab')),
                'secret': f(cs.str_schema(), serialization_exclude=True),
                'described': f(cs.int_schema(),
                               metadata={'pydantic_js_updates': {'title': 'Custom', 'description': 'D'}}),
                'inf': f(cs.with_default_schema(cs.float_schema(), default=math.inf)),
            },
            computed_fields=[cs.computed_field('area', cs.int_schema(), alias='AREA')],
        ),
        config={'title': 'The User', 'extra_fields_behavior': 'forbid',
                'json_schema_extra': {'examples': [{'id': 1}]}},
        ref='app.models.User:1',
    )
    yield 'model_validation', user, {}
    yield 'model_serialization', user, {'mode': 'serialization'}
    yield 'model_validation_no_alias', user, {'by_alias': False}
    yield 'model_serialization_no_alias', user, {'mode': 'serialization', 'by_alias': False}

    node = cs.definitions_schema(cs.definition_reference_schema('app.Node:2'), [
        cs.model_schema(model_class('Node'), cs.model_fields_schema({
            'value': f(cs.int_schema()),
            'children': f(cs.with_default_schema(
                cs.list_schema(cs.definition_reference_schema('app.Node:2')), default=[])),
        }), ref='app.Node:2'),
    ])
    yield 'recursive_model', node, {}
    yield 'recursive_model_ref_template', node, {'ref_template': '#/components/schemas/{model}'}
    yield 'undefined_reference', cs.definition_reference_schema('missing:1'), {}

    pet = cs.model_schema(model_class('Pet'), cs.model_fields_schema({'name': f(cs.str_schema())}),
                          ref='zoo.Pet:3')
    yield 'shared_submodel', cs.model_schema(model_class('Owner'), cs.model_fields_schema({
        'a': f(pet), 'b': f(cs.nullable_schema(cs.definition_reference_schema('zoo.Pet:3'))),
    }), ref='zoo.Owner:4'), {}

    first = cs.model_schema(model_class('Model'), cs.model_fields_schema({'x': f(cs.int_schema())}),
                            ref='a.Model:5')
    second = cs.model_schema(model_class('Model'), cs.model_fields_schema({'y': f(cs.str_schema())}),
                             ref='b.Model:6')
    yield 'name_collision', cs.tuple_schema([first, second]), {}
    yield 'repeated_reference', cs.tuple_schema([first, first]), {'mode': 'serialization'}
    yield 'generic_reference', cs.list_schema(cs.int_schema(ref='pkg.Box[pkg.Item:7, int]:8')), {}

    cat = cs.model_schema(model_class('Cat'), cs.model_fields_schema({
        'kind': f(cs.literal_schema(['cat'])), 'meow': f(cs.int_schema()),
    }), ref='z.Cat:7')
    dog = cs.model_schema(model_class('Dog'), cs.model_fields_schema({
        'kind': f(cs.literal_schema(['dog']), validation_alias='Kind'), 'bark': f(cs.float_schema()),
    }), ref='z.Dog:8')
    yield 'tagged_union', cs.tagged_union_schema({'cat': cat, 'dog': dog}, discriminator='kind'), {}
    yield 'tagged_union_alias_paths', cs.tagged_union_schema(
        {'cat': cat, 'dog': dog}, discriminator=[['Kind'], ['kind']]), {}
    yield 'tagged_union_single_alias_path', cs.tagged_union_schema(
        {'cat': cat, 'dog': dog}, discriminator=[['kind']]), {}
    yield 'tagged_union_keys', cs.tagged_union_schema(
        {True: cs.int_schema(), None: cs.str_schema(), 2: cs.float_schema()}, discriminator=['k', 0]), {}

    yield 'extras_schema', cs.model_schema(
        model_class('Extra', extra='allow', str_min_length=2, str_max_length=9),
        cs.model_fields_schema({'a': f(cs.str_schema())}, extras_schema=cs.int_schema()),
        config={'extra_fields_behavior': 'allow', 'str_min_length': 2, 'str_max_length': 9},
    ), {}
    yield 'extra_allow_base64_default', cs.model_schema(
        model_class('Ext2', extra='allow', ser_json_bytes='base64'),
        cs.model_fields_schema({'b': f(cs.with_default_schema(cs.bytes_schema(), default=b'\xfb'))}),
        config={'extra_fields_behavior': 'allow', 'ser_json_bytes': 'base64'},
    ), {}
    yield 'non_serializable_default', cs.model_schema(
        model_class('Bad'),
        cs.model_fields_schema({'b': f(cs.with_default_schema(cs.bytes_schema(), default=b'\xfb'))}),
    ), {}
    yield 'root_model', cs.model_schema(model_class('Root'), cs.list_schema(cs.int_schema()), root_model=True), {}
    yield 'mode_override', cs.model_schema(
        model_class('Ovr', json_schema_mode_override='serialization',
                    json_schema_serialization_defaults_required=True),
        cs.model_fields_schema({
            'a': f(cs.with_default_schema(cs.int_schema(), default=1)),
            'b': f(cs.int_schema(), serialization_exclude=True),
        }),
        config={'json_schema_mode_override': 'serialization',
                'json_schema_serialization_defaults_required': True},
    ), {}
    yield 'validate_by_alias_false', cs.model_schema(
        model_class('NoAlias', validate_by_alias=False),
        cs.model_fields_schema({'user_id': f(cs.int_schema(), validation_alias='uid')}),
        config={'validate_by_alias': False},
    ), {}
    yield 'field_titles', cs.model_schema(model_class('T'), cs.model_fields_schema({
        'user_id': f(cs.int_schema()), 'fooBar': f(cs.int_schema()), 'a1b_c': f(cs.int_schema()),
        'sub': f(cs.nullable_schema(cs.with_default_schema(cs.int_schema(), default=1))),
        'model': f(cs.model_schema(model_class('Sub'), cs.model_fields_schema({}), ref='m.Sub:9')),
    })), {}
    yield 'metadata', cs.int_schema(metadata={
        'pydantic_js_updates': {'description': 'u'}, 'pydantic_js_extra': {'examples': [1, b'x']},
    }), {}
    to_string = cs.nullable_schema(cs.int_schema(), serialization=cs.to_string_ser_schema(when_used='unless-none'))
    yield 'serialization_schema', to_string, {'mode': 'serialization'}
    yield 'serialization_schema_in_validation_mode', to_string, {}
    yield 'plain_reference', cs.list_schema(cs.int_schema(ref='my.Int:9')), {}
    yield 'dict_reference_keys', cs.definitions_schema(
        cs.dict_schema(cs.definition_reference_schema('k:1'), cs.int_schema()),
        [cs.str_schema(min_length=2, ref='k:1')],
    ), {}


def record(generate: Generate = pydantic_generate) -> list[dict[str, Any]]:
    return [record_case(generate, case_id, schema, **kwargs) for case_id, schema, kwargs in cases()]


def write(recorded: list[dict[str, Any]], out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(recorded, indent=1) + '\n')


if __name__ == '__main__':
    write(record(), Path(sys.argv[1]))
