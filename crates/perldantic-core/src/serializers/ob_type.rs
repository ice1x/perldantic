//! The kind of a value, as serializers see it. Replaces upstream `serializers/ob_type.rs`,
//! which looks up Python types: `Value` variants map onto the same kinds.
//!
//! Python's subclass rules that matter for `Value` are kept: `bool` is a subclass of `int`,
//! and an `int` is accepted by float serializers as if it were a subclass of `float`.

use strum::Display;

use crate::value::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum ObType {
    None,
    Int,
    Bool,
    Float,
    Str,
    Bytes,
    List,
    Tuple,
    Set,
    Frozenset,
    Dict,
    Datetime,
    Date,
    Time,
    Timedelta,
    Uuid,
    Url,
    MultiHostUrl,
    Decimal,
    Enum,
    /// A model instance, serialized through its fields.
    PydanticSerializable,
}

/// How a value relates to an expected kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IsType {
    Exact,
    Subclass,
    False,
}

pub(crate) fn get_type(value: &Value) -> ObType {
    match value {
        Value::None => ObType::None,
        Value::Bool(_) => ObType::Bool,
        Value::Int(_) | Value::BigInt(_) => ObType::Int,
        Value::Float(_) => ObType::Float,
        Value::Str(_) => ObType::Str,
        Value::Bytes(_) => ObType::Bytes,
        Value::List(_) => ObType::List,
        Value::Tuple(_) => ObType::Tuple,
        Value::Set(_) => ObType::Set,
        Value::FrozenSet(_) => ObType::Frozenset,
        Value::Dict(_) => ObType::Dict,
        Value::Model(_) => ObType::PydanticSerializable,
        Value::DateTime(_) => ObType::Datetime,
        Value::Date(_) => ObType::Date,
        Value::Time(_) => ObType::Time,
        Value::TimeDelta(_) => ObType::Timedelta,
        Value::Uuid(_) => ObType::Uuid,
        Value::Url(_) => ObType::Url,
        Value::MultiHostUrl(_) => ObType::MultiHostUrl,
        Value::Decimal(_) => ObType::Decimal,
        Value::Enum(_) => ObType::Enum,
    }
}

pub(crate) fn is_type(value: &Value, expected: ObType) -> IsType {
    let actual = get_type(value);
    if actual == expected {
        return IsType::Exact;
    }
    // members of `IntEnum`, `StrEnum` etc. are instances of a subclass of their value's type
    if let Some(mixin_value) = value.mixin_value() {
        return match is_type(mixin_value, expected) {
            IsType::Exact | IsType::Subclass => IsType::Subclass,
            IsType::False => IsType::False,
        };
    }
    match (actual, expected) {
        // bool is a subclass of int; int passes for float (pydantic-core#866)
        (ObType::Bool, ObType::Int | ObType::Float) | (ObType::Int, ObType::Float) => {
            IsType::Subclass
        }
        _ => IsType::False,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_subclass_rules() {
        assert_eq!(is_type(&Value::Int(1), ObType::Int), IsType::Exact);
        assert_eq!(is_type(&Value::Bool(true), ObType::Int), IsType::Subclass);
        assert_eq!(is_type(&Value::Bool(true), ObType::Float), IsType::Subclass);
        assert_eq!(is_type(&Value::Int(1), ObType::Float), IsType::Subclass);
        assert_eq!(is_type(&Value::Int(1), ObType::Bool), IsType::False);
        assert_eq!(is_type(&Value::Float(1.0), ObType::Int), IsType::False);
        assert_eq!(
            ObType::PydanticSerializable.to_string(),
            "pydantic_serializable"
        );
        assert_eq!(ObType::Set.to_string(), "set");
        let member = |value: Value, mixin| {
            Value::Enum(Box::new(crate::value::EnumMember {
                class: "E".into(),
                name: "A".into(),
                value,
                mixin,
                str_is_value: false,
            }))
        };
        let int_enum = member(Value::Int(1), Some(crate::value::EnumMixin::Int));
        assert_eq!(is_type(&int_enum, ObType::Enum), IsType::Exact);
        assert_eq!(is_type(&int_enum, ObType::Int), IsType::Subclass);
        assert_eq!(is_type(&int_enum, ObType::Float), IsType::Subclass);
        assert_eq!(
            is_type(&member(Value::Int(1), None), ObType::Int),
            IsType::False
        );
    }
}
