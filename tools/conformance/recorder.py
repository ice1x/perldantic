"""pytest plugin that records `SchemaValidator` and `SchemaSerializer` calls as conformance cases.

Run upstream's own tests with it to capture what pydantic-core actually does:

    python -m pytest tests/validators -p conformance.recorder --conformance-out=<dir>

After collection, every `SchemaValidator` name bound in a test module (under the rootdir) is
wrapped, and so is every `SchemaSerializer` name. Each `validate_python` / `validate_json` /
`to_python` / `to_json` call is recorded with its schema, config, input, keyword options and
outcome (output value, JSON text, validation errors or exception, plus serializer warnings),
encoded with `conformance.encoding`. Cases are deduplicated, sorted by id and written to one JSON file per
test module, mirroring the test paths under the output directory.

Hypothesis-driven tests are not recorded: their inputs are random and would make the recorded
cases change between runs.
"""

from __future__ import annotations

import json
import re
import sys
import warnings
from pathlib import Path
from typing import Any

import pytest

from conformance.encoding import encode

_ADDRESS = re.compile(r' at 0x[0-9a-fA-F]+')

_current_test: str | None = None
_cases: dict[str, list[dict[str, Any]]] = {}
_seen: set[str] = set()


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption('--conformance-out', required=True, help='directory to write recorded cases to')


def pytest_sessionstart(session: pytest.Session) -> None:
    _cases.clear()
    _seen.clear()


def _stable(text: str) -> str:
    """Drop memory addresses from reprs so recordings are reproducible."""
    return _ADDRESS.sub(' at 0x...', text)


def _safe_encode(value: Any) -> Any:
    try:
        return encode(value)
    except RecursionError:
        return {'$object': 'cyclic'}


def _encode_errors(errors: list[dict[str, Any]]) -> list[dict[str, Any]]:
    encoded = []
    for error in errors:
        entry = {k: _safe_encode(v) for k, v in error.items() if k != 'loc'}
        entry['loc'] = [_safe_encode(item) for item in error['loc']]
        encoded.append({k: entry[k] for k in ('type', 'loc', 'msg', 'input', 'ctx') if k in entry})
    return encoded


def _record(schema: Any, config: Any, mode: str, input_value: Any, options: dict[str, Any], expected: dict[str, Any]) -> None:
    if _current_test is None:
        return
    case = {
        'test': _current_test,
        'schema': _safe_encode(schema),
        'config': _safe_encode(config),
        'mode': mode,
        'input': input_value,
        'options': _safe_encode(options),
        'expected': expected,
    }
    key = json.dumps({k: v for k, v in case.items() if k != 'expected'}, sort_keys=True)
    if key in _seen:
        return
    _seen.add(key)
    file_key = _current_test.split('::', 1)[0]
    _cases.setdefault(file_key, []).append(case)


class RecordingValidator:
    """Proxy around a real SchemaValidator that records validation calls."""

    def __init__(self, inner: Any, schema: Any, config: Any) -> None:
        self._inner = inner
        self._schema = schema
        self._config = config

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def __repr__(self) -> str:
        return repr(self._inner)

    def __reduce__(self) -> Any:
        # Pickle as the real validator; unpickled copies are not recorded.
        return self._inner.__reduce__()

    def _call(self, method: str, mode: str, input_value: Any, recorded_input: Any, kwargs: dict[str, Any]) -> Any:
        from pydantic_core import ValidationError

        try:
            output = getattr(self._inner, method)(input_value, **kwargs)
        except ValidationError as exc:
            expected = {
                'errors': _encode_errors(exc.errors(include_url=False)),
                'title': _stable(str(exc.title)),
            }
            _record(self._schema, self._config, mode, recorded_input, kwargs, expected)
            raise
        except Exception as exc:
            expected = {'exception': {'type': type(exc).__name__, 'message': _stable(str(exc))}}
            _record(self._schema, self._config, mode, recorded_input, kwargs, expected)
            raise
        _record(self._schema, self._config, mode, recorded_input, kwargs, {'output': _safe_encode(output)})
        return output

    def validate_python(self, input_value: Any, **kwargs: Any) -> Any:
        return self._call('validate_python', 'python', input_value, _safe_encode(input_value), kwargs)

    def validate_json(self, input_value: Any, **kwargs: Any) -> Any:
        recorded = _safe_encode(input_value)
        return self._call('validate_json', 'json', input_value, recorded, kwargs)


class RecordingSerializer:
    """Proxy around a real SchemaSerializer that records serialization calls."""

    def __init__(self, inner: Any, schema: Any, config: Any) -> None:
        self._inner = inner
        self._schema = schema
        self._config = config

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)

    def __repr__(self) -> str:
        return repr(self._inner)

    def __reduce__(self) -> Any:
        # Pickle as the real serializer; unpickled copies are not recorded.
        return self._inner.__reduce__()

    def _call(self, method: str, value: Any, kwargs: dict[str, Any]) -> Any:
        recorded_input = _safe_encode(value)
        try:
            with warnings.catch_warnings(record=True) as caught:
                warnings.simplefilter('always')
                output = getattr(self._inner, method)(value, **kwargs)
        except Exception as exc:
            expected: dict[str, Any] = {
                'exception': {'type': type(exc).__name__, 'message': _stable(str(exc))}
            }
            _record(self._schema, self._config, method, recorded_input, kwargs, expected)
            raise
        finally:
            # Re-emit outside the recording block, so the tests' own warning checks still see
            # them (inside it they would be caught again).
            for w in caught:
                warnings.warn_explicit(w.message, w.category, w.filename, w.lineno)
        messages = [_stable(str(w.message)) for w in caught]
        if method == 'to_json':
            expected = {'json': output.decode()}
        else:
            expected = {'output': _safe_encode(output)}
        if messages:
            expected['warnings'] = messages
        _record(self._schema, self._config, method, recorded_input, kwargs, expected)
        return output

    def to_python(self, value: Any, **kwargs: Any) -> Any:
        return self._call('to_python', value, kwargs)

    def to_json(self, value: Any, **kwargs: Any) -> Any:
        return self._call('to_json', value, kwargs)


class RecordingFactory:
    """Stands in for the `SchemaValidator` or `SchemaSerializer` class in test modules."""

    def __init__(self, cls: Any, proxy: type = RecordingValidator) -> None:
        self._cls = cls
        self._proxy = proxy

    def __getattr__(self, name: str) -> Any:
        return getattr(self._cls, name)

    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        inner = self._cls(*args, **kwargs)
        schema = args[0] if args else kwargs.get('schema')
        config = args[1] if len(args) > 1 else kwargs.get('config')
        return self._proxy(inner, schema, config)


@pytest.hookimpl(trylast=True)
def pytest_collection_finish(session: pytest.Session) -> None:
    root = str(session.config.rootpath)
    for module in list(sys.modules.values()):
        path = getattr(module, '__file__', None) or ''
        if not path.startswith(root):
            continue
        if 'SchemaValidator' in vars(module):
            module.SchemaValidator = RecordingFactory(module.SchemaValidator)
        if 'SchemaSerializer' in vars(module):
            module.SchemaSerializer = RecordingFactory(module.SchemaSerializer, RecordingSerializer)


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_call(item: pytest.Item):
    global _current_test
    is_hypothesis = getattr(getattr(item, 'obj', None), 'is_hypothesis_test', False)
    _current_test = None if is_hypothesis else _stable(item.nodeid)
    try:
        yield
    finally:
        _current_test = None


def pytest_sessionfinish(session: pytest.Session) -> None:
    out_dir = Path(session.config.getoption('--conformance-out'))
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
