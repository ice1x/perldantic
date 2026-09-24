"""Tagged JSON encoding of Python values used by conformance cases.

Plain JSON types (None, bool, int, finite float, str, list, dict with plain string keys) are
written as-is. Everything else is a single-key object whose key starts with `$`:

    {"$tuple": [...]}            tuple
    {"$bytes": "<base64>"}       bytes
    {"$bytearray": "<base64>"}   bytearray
    {"$float": "inf"|"-inf"|"nan"}
    {"$dict": [[k, v], ...]}     dict with non-string keys, or keys starting with "$"
    {"$set": [...]}              set (items sorted for determinism)
    {"$frozenset": [...]}        frozenset
    {"$decimal": "1.50"}         decimal.Decimal
    {"$date": "..."} / {"$time": "..."} / {"$datetime": "..."}   ISO 8601
    {"$timedelta": [days, seconds, microseconds]}
    {"$uuid": "..."}             uuid.UUID
    {"$url": "..."}              pydantic_core.Url (its text, as str() gives it)
    {"$multi_host_url": "..."}   pydantic_core.MultiHostUrl

Values that only describe Python objects (they cannot be rebuilt outside Python):

    {"$enum": [class_name, member_name, value, mixin, str_is_value]}   enum member; mixin is
                                         "int", "str", "float" or "bytes" when the member is
                                         also one (IntEnum, StrEnum, class E(str, Enum)), and
                                         str_is_value is true when str() gives the value's
                                         text (IntEnum, StrEnum), not "Class.NAME"
    {"$class": name}                     a class, e.g. a model's `cls`
    {"$model": {"class", "fields", "fields_set", "extra"}}   a validated model instance
    {"$function": name}                  a function or other callable
    {"$exception": [type_name, message]}
    {"$subclass": [class_name, base_value]}   instance of a subclass of a builtin type
    {"$object": type_name}               anything else; "str-with-surrogates" marks strings
                                         that are not valid Unicode (unpaired surrogates)

The Rust conformance runner decodes the first group into `Value`s and skips cases that contain
anything it cannot represent. `decode()` here mirrors that contract and exists mainly to test it.
"""

from __future__ import annotations

import base64
import datetime
import decimal
import enum
import math
import uuid
from typing import Any

from pydantic_core import MultiHostUrl, Url


class UnsupportedValue(ValueError):
    """The wire value describes a Python object that cannot be rebuilt."""


_BUILTIN_BASES = (bool, int, float, str, bytes, bytearray, list, tuple, dict, set, frozenset)


def _utf8_safe(text: str) -> bool:
    try:
        text.encode('utf-8')
    except UnicodeEncodeError:
        return False
    return True


def _sort_key(item: Any) -> tuple[str, str]:
    return type(item).__name__, repr(item)


def _name(obj: Any) -> str:
    return getattr(obj, '__name__', type(obj).__name__)


def _has_custom_missing(cls: Any) -> bool:
    return isinstance(cls, type) and issubclass(cls, enum.Enum) and cls._missing_.__func__ is not enum.Enum._missing_.__func__


def with_enum_class_data(schema: Any) -> Any:
    """A copy of a core schema with what pydantic-core reads off enum classes moved into `enum` schemas.

    Error messages name the class by `__qualname__` (recorded as `cls_repr`, which upstream
    reads first). pydantic-core calls the class when no member matches, which runs its own
    `_missing_` hook; ports need a host callback for that, so the hook is recorded as the
    schema's `missing`.
    """
    if isinstance(schema, dict):
        copy = {k: with_enum_class_data(v) for k, v in schema.items()}
        cls = schema.get('cls')
        if schema.get('type') == 'enum' and isinstance(cls, type):
            copy.setdefault('cls_repr', cls.__qualname__)
            if 'missing' not in schema and _has_custom_missing(cls):
                copy['missing'] = cls._missing_
        return copy
    if isinstance(schema, list):
        return [with_enum_class_data(v) for v in schema]
    return schema


def encode(value: Any) -> Any:
    """Encode a Python value into JSON-compatible data; never raises for odd objects."""
    try:
        return _encode(value)
    except RecursionError:
        raise
    except Exception:
        return {'$object': type(value).__name__}


def _encode(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, enum.Enum):
        mixin = next((t.__name__ for t in (int, str, float, bytes) if isinstance(value, t)), None)
        str_is_value = isinstance(value, enum.ReprEnum)
        return {'$enum': [type(value).__name__, value.name, encode(value.value), mixin, str_is_value]}

    kind = type(value)
    if kind is str:
        return value if _utf8_safe(value) else {'$object': 'str-with-surrogates'}
    if kind is bool or kind is int:
        return value
    if kind is float:
        if math.isnan(value):
            return {'$float': 'nan'}
        if math.isinf(value):
            return {'$float': 'inf' if value > 0 else '-inf'}
        return value
    if kind is bytes:
        return {'$bytes': base64.b64encode(value).decode()}
    if kind is bytearray:
        return {'$bytearray': base64.b64encode(bytes(value)).decode()}
    if kind is list:
        return [encode(v) for v in value]
    if kind is tuple:
        return {'$tuple': [encode(v) for v in value]}
    if kind is dict:
        if all(type(k) is str and not k.startswith('$') and _utf8_safe(k) for k in value):
            return {k: encode(v) for k, v in value.items()}
        return {'$dict': [[encode(k), encode(v)] for k, v in value.items()]}
    if kind is set:
        return {'$set': [encode(v) for v in sorted(value, key=_sort_key)]}
    if kind is frozenset:
        return {'$frozenset': [encode(v) for v in sorted(value, key=_sort_key)]}
    if kind is decimal.Decimal:
        return {'$decimal': str(value)}
    if kind is datetime.datetime:
        return {'$datetime': value.isoformat()}
    if kind is datetime.date:
        return {'$date': value.isoformat()}
    if kind is datetime.time:
        return {'$time': value.isoformat()}
    if kind is datetime.timedelta:
        return {'$timedelta': [value.days, value.seconds, value.microseconds]}
    if kind is uuid.UUID:
        return {'$uuid': str(value)}
    if kind is Url:
        return {'$url': str(value)}
    if kind is MultiHostUrl:
        return {'$multi_host_url': str(value)}

    if isinstance(value, type):
        return {'$class': value.__name__}
    if hasattr(value, '__pydantic_fields_set__'):
        fields = {k: v for k, v in vars(value).items() if not k.startswith('__pydantic_')}
        return {
            '$model': {
                'class': kind.__name__,
                'fields': encode(fields),
                'fields_set': sorted(value.__pydantic_fields_set__),
                'extra': encode(getattr(value, '__pydantic_extra__', None)),
            }
        }
    if isinstance(value, BaseException):
        return {'$exception': [kind.__name__, str(value)]}
    if isinstance(value, _BUILTIN_BASES):
        base = next(b for b in _BUILTIN_BASES if isinstance(value, b))
        return {'$subclass': [kind.__name__, encode(base(value))]}
    if callable(value):
        return {'$function': _name(value)}
    return {'$object': kind.__name__}


_DECODERS = {
    '$tuple': lambda v: tuple(decode(x) for x in v),
    '$bytes': base64.b64decode,
    '$bytearray': lambda v: bytearray(base64.b64decode(v)),
    '$float': float,
    '$dict': lambda v: {decode(k): decode(x) for k, x in v},
    '$set': lambda v: {decode(x) for x in v},
    '$frozenset': lambda v: frozenset(decode(x) for x in v),
    '$decimal': decimal.Decimal,
    '$date': datetime.date.fromisoformat,
    '$time': datetime.time.fromisoformat,
    '$datetime': datetime.datetime.fromisoformat,
    '$timedelta': lambda v: datetime.timedelta(days=v[0], seconds=v[1], microseconds=v[2]),
    '$uuid': uuid.UUID,
    '$url': Url,
    '$multi_host_url': MultiHostUrl,
}


def decode(wire: Any) -> Any:
    """Rebuild a Python value from its encoding; raises UnsupportedValue for descriptive tags."""
    if isinstance(wire, list):
        return [decode(v) for v in wire]
    if isinstance(wire, dict):
        if len(wire) == 1:
            (key, payload), = wire.items()
            if key.startswith('$'):
                try:
                    decoder = _DECODERS[key]
                except KeyError:
                    raise UnsupportedValue(f'cannot rebuild a {key} value') from None
                return decoder(payload)
        return {k: decode(v) for k, v in wire.items()}
    return wire
