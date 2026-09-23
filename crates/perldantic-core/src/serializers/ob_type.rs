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
    Dict,
    Datetime,
    Date,
    Time,
    Timedelta,
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
        Value::Dict(_) => ObType::Dict,
        Value::Model(_) => ObType::PydanticSerializable,
        Value::DateTime(_) => ObType::Datetime,
        Value::Date(_) => ObType::Date,
        Value::Time(_) => ObType::Time,
        Value::TimeDelta(_) => ObType::Timedelta,
    }
}

pub(crate) fn is_type(value: &Value, expected: ObType) -> IsType {
    let actual = get_type(value);
    if actual == expected {
        return IsType::Exact;
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
    }
}
