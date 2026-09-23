"""pytest plugin that records pydantic's JSON Schema generation as snapshot cases.

Run pydantic's own tests with it to capture what `GenerateJsonSchema.generate` produces for the
core schemas of real models and types:

    python -m pytest tests/test_json_schema.py -p json_schema.recorder --json-schema-out=<dir>

Every `generate` call made by the stock `GenerateJsonSchema` class (subclasses customise the
output and are skipped) is recorded with its core schema, mode, options, outcome and warnings.

pydantic reads parts of a model's JSON Schema off its Python class; the recorder hands them to the
port the way a host would (docs/DIVERGENCES.md #15):

- the model class's `model_config` keys that shape the JSON Schema (`title`, `json_schema_extra`,
  `json_schema_mode_override`, `json_schema_serialization_defaults_required`,
  `model_title_generator`) go into the `model` schema's `config`;
- a config pushed for the whole schema (`TypeAdapter(..., config=...)`) is recorded as the case's
  `config`, converted to core config names plus the keys above;
- the class docstring becomes `metadata.pydantic_js_updates.description`, and `__deprecated__`
  becomes `deprecated`;
- `BaseModel.__get_pydantic_json_schema__`, which every model lists in
  `metadata.pydantic_js_functions` and which only calls the handler, is dropped.

Everything else stays as pydantic built it; Python callables left in a schema are encoded as
`{"$function": name}`, so the replaying runner skips those cases. The ids pydantic appends to
core refs (`module.Model:140316144828160`) change between runs and are renumbered per case.
"""

from __future__ import annotations

import inspect
import json
import re
import warnings
from pathlib import Path
from typing import Any

import pytest

from conformance.encoding import encode

_ADDRESS = re.compile(r' at 0x[0-9a-fA-F]+')
_REF_ID = re.compile(r':(\d+)(?=$|[\[\],])')
# `model_config` keys pydantic's JSON Schema generation reads that the core config lacks or loses
# (a blank `title` becomes the class name there).
_CONFIG_KEYS = (
    'title',
    'json_schema_extra',
    'json_schema_mode_override',
    'json_schema_serialization_defaults_required',
    'model_title_generator',
)

_current_test: str | None = None
_cases: dict[str, list[dict[str, Any]]] = {}
_seen: set[str] = set()
_original_generate: Any = None


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption('--json-schema-out', required=True, help='directory to write recorded cases to')


def pytest_configure(config: pytest.Config) -> None:
    # pydantic's suite marks tests for pytest-run-parallel, which recording does not need.
    config.addinivalue_line('markers', 'thread_unsafe: not safe to run in parallel threads')


def _stable(text: str) -> str:
    """Drop memory addresses from reprs so recordings are reproducible."""
    return _ADDRESS.sub(' at 0x...', text)


def _safe_encode(value: Any) -> Any:
    try:
        return encode(value)
    except RecursionError:
        return {'$object': 'cyclic'}


def _is_default_model_hook(function: Any) -> bool:
    from pydantic import BaseModel

    return getattr(function, '__func__', None) is BaseModel.__get_pydantic_json_schema__.__func__


def _docstring(cls: type) -> str | None:
    from pydantic import BaseModel

    if cls is BaseModel:
        return None
    return inspect.cleandoc(cls.__doc__) if cls.__doc__ else None


def host_config(config_dict: dict[str, Any], core_config: dict[str, Any] | None = None) -> Any:
    """A pydantic `ConfigDict` as the port reads it: the core config plus `_CONFIG_KEYS`."""
    from pydantic._internal._config import ConfigWrapper

    if core_config is None:
        try:
            core_config = dict(ConfigWrapper(dict(config_dict), check=False).core_config(title=None))
        except Exception:  # noqa: BLE001 - pydantic rejects the config; the case cannot run
            return {'$object': 'invalid-config'}
    config = dict(core_config)
    for key in _CONFIG_KEYS:
        if config_dict.get(key) is not None:
            config[key] = config_dict[key]
    return config


def host_model_schema(schema: dict[str, Any]) -> dict[str, Any]:
    """A `model` core schema with what pydantic reads off the class moved into the schema."""
    cls = schema['cls']
    model_config = getattr(cls, 'model_config', {})
    host = dict(schema)

    config = host_config(model_config, schema.get('config') or {})
    if getattr(cls, '__pydantic_root_model__', False):
        root_extra = cls.model_fields['root'].json_schema_extra
        if root_extra is not None:
            # pydantic raises when both are set; a callable marks the case as unsupported
            config['json_schema_extra'] = root_extra if 'json_schema_extra' not in config else encode
    if config:
        host['config'] = config

    metadata = dict(schema.get('metadata') or {})
    functions = [f for f in metadata.get('pydantic_js_functions', ()) if not _is_default_model_hook(f)]
    if functions:
        metadata['pydantic_js_functions'] = functions
    else:
        metadata.pop('pydantic_js_functions', None)
    updates = dict(metadata.get('pydantic_js_updates') or {})
    extra = config.get('json_schema_extra')
    doc = _docstring(cls)
    # pydantic sets the docstring only when `json_schema_extra` does not override it
    if doc and not (isinstance(extra, dict) and 'description' in extra):
        updates['description'] = doc
    if hasattr(cls, '__deprecated__'):
        updates['deprecated'] = True
    if updates:
        metadata['pydantic_js_updates'] = updates
    if metadata:
        host['metadata'] = metadata
    else:
        host.pop('metadata', None)
    return host


def host_schema(value: Any) -> Any:
    """Rewrite every `model` schema in a core schema with `host_model_schema`."""
    if isinstance(value, dict):
        if value.get('type') == 'model' and isinstance(value.get('cls'), type):
            value = host_model_schema(value)
        return {k: host_schema(v) for k, v in value.items()}
    if isinstance(value, list):
        return [host_schema(v) for v in value]
    if isinstance(value, tuple):
        return tuple(host_schema(v) for v in value)
    return value


def renumber_ref_ids(encoded: Any) -> Any:
    """Replace the ids in core refs by their order of appearance, consistently within a case."""
    ids: dict[str, str] = {}

    def renumber(match: re.Match[str]) -> str:
        return ':' + ids.setdefault(match.group(1), str(len(ids) + 1))

    def walk(value: Any, key: str | None = None) -> Any:
        if isinstance(value, dict):
            return {k: walk(v, k) for k, v in value.items()}
        if isinstance(value, list):
            return [walk(v) for v in value]
        if isinstance(value, str) and key in ('ref', 'schema_ref'):
            return _REF_ID.sub(renumber, value)
        return value

    return walk(encoded)


def _record(
    schema: Any, config: Any, mode: str, options: dict[str, Any], expected: dict[str, Any], messages: list[str]
) -> None:
    if _current_test is None:
        return
    case = {
        'test': _current_test,
        'schema': renumber_ref_ids(_safe_encode(host_schema(schema))),
        'config': None if config is None else _safe_encode(config),
        'mode': mode,
        'options': options,
        'expected': expected,
        'warnings': messages,
    }
    key = json.dumps({k: v for k, v in case.items() if k != 'test'}, sort_keys=True)
    if key in _seen:
        return
    _seen.add(key)
    _cases.setdefault(_current_test.split('::', 1)[0], []).append(case)


def _options(generator: Any) -> dict[str, Any]:
    from pydantic.json_schema import DEFAULT_REF_TEMPLATE

    options: dict[str, Any] = {}
    if generator.by_alias is not True:
        options['by_alias'] = generator.by_alias
    if generator.ref_template != DEFAULT_REF_TEMPLATE:
        options['ref_template'] = generator.ref_template
    if generator.union_format != 'any_of':
        options['union_format'] = generator.union_format
    return options


def recording_generate(self: Any, schema: Any, mode: str = 'validation') -> Any:
    from pydantic.json_schema import GenerateJsonSchema

    # Subclasses customise the output; a reused generator fails whatever the schema.
    if type(self) is not GenerateJsonSchema or self._used:
        return _original_generate(self, schema, mode)
    options = _options(self)
    stack = self._config_wrapper_stack._config_wrapper_stack
    config = host_config(stack[-1].config_dict) if len(stack) > 1 else None
    caught: list[warnings.WarningMessage] = []
    try:
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter('always')
            output = _original_generate(self, schema, mode)
    except Exception as exc:
        expected: dict[str, Any] = {'error': [type(exc).__name__, _stable(str(exc))]}
        _record(schema, config, mode, options, expected, [_stable(str(w.message)) for w in caught])
        raise
    finally:
        # Re-emit outside the recording block, so the tests' own warning checks still see them.
        for w in caught:
            warnings.warn_explicit(w.message, w.category, w.filename, w.lineno)
    _record(schema, config, mode, options, {'json_schema': _safe_encode(output)}, [_stable(str(w.message)) for w in caught])
    return output


def pytest_sessionstart(session: pytest.Session) -> None:
    global _original_generate
    from pydantic.json_schema import GenerateJsonSchema

    _cases.clear()
    _seen.clear()
    _original_generate = GenerateJsonSchema.generate
    GenerateJsonSchema.generate = recording_generate


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_call(item: pytest.Item):
    global _current_test
    _current_test = _stable(item.nodeid)
    try:
        yield
    finally:
        _current_test = None


def pytest_sessionfinish(session: pytest.Session) -> None:
    from pydantic.json_schema import GenerateJsonSchema

    if _original_generate is not None:
        GenerateJsonSchema.generate = _original_generate
    out_dir = Path(session.config.getoption('--json-schema-out'))
    for file_key, cases in _cases.items():
        counters: dict[str, int] = {}
        for case in cases:
            n = counters.get(case['test'], 0)
            counters[case['test']] = n + 1
            case['id'] = f'{case["test"]}#{n}'
        ordered = sorted(({'id': c.pop('id'), **c} for c in cases), key=lambda c: c['id'])
        target = out_dir / Path(file_key).with_suffix('.json')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(ordered, indent=1, ensure_ascii=False) + '\n')
