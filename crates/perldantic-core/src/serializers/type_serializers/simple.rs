//! `none`, `int` and `bool` serializers. Port of upstream `type_serializers/simple.rs`.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

use serde::{Serialize, Serializer};

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::{SerResult, SerializeError, UnexpectedValue};
use crate::serializers::extra::{SerCheck, SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::ob_type::{IsType, ObType, is_type};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct NoneSerializer;

static NONE_SERIALIZER: LazyLock<Arc<CombinedSerializer>> =
    LazyLock::new(|| Arc::new(NoneSerializer.into()));

impl BuildSerializer for NoneSerializer {
    const EXPECTED_TYPE: &'static str = "none";

    fn build(
        _schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        Ok(NONE_SERIALIZER.clone())
    }
}

pub(crate) fn none_json_key() -> Cow<'static, str> {
    Cow::Borrowed("None")
}

impl TypeSerializer for NoneSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match is_type(value, ObType::None) {
            IsType::Exact => Ok(Value::None),
            // I don't think subclasses of None can exist
            _ => {
                state.warn_fallback_py(self.get_name(), value)?;
                infer_to_python(value, state)
            }
        }
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        match is_type(key, ObType::None) {
            IsType::Exact => Ok(none_json_key()),
            _ => {
                state.warn_fallback_py(self.get_name(), key)?;
                infer_json_key(key, state)
            }
        }
    }

    fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        match is_type(value, ObType::None) {
            IsType::Exact => serializer.serialize_none(),
            _ => {
                state.warn_fallback_ser::<S>(self.get_name(), value)?;
                infer_serialize(value, serializer, state)
            }
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

/// Python's `str(key)` as an object key.
pub(crate) fn to_str_json_key(key: &Value) -> Cow<'_, str> {
    Cow::Owned(key.py_str())
}

pub(crate) fn bool_json_key(key: &Value) -> Cow<'_, str> {
    let truthy = match key {
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        _ => true,
    };
    Cow::Borrowed(if truthy { "true" } else { "false" })
}

/// pyo3's `extract::<Int>()`: ints, including bools.
fn extract_int(value: &Value) -> Option<Value> {
    match value {
        Value::Int(_) | Value::BigInt(_) => Some(value.clone()),
        Value::Bool(b) => Some(Value::Int(i64::from(*b))),
        _ => None,
    }
}

/// pyo3's `extract::<bool>()`: only bools.
fn extract_bool(value: &Value) -> Option<Value> {
    match value {
        Value::Bool(_) => Some(value.clone()),
        _ => None,
    }
}

macro_rules! build_simple_serializer {
    ($struct_name:ident, $expected_type:literal, $extract:ident, $ob_type:expr, $key_method:ident, $subtypes_allowed:expr) => {
        #[derive(Debug)]
        pub struct $struct_name;

        impl $struct_name {
            pub fn get() -> &'static Arc<CombinedSerializer> {
                static INSTANCE: LazyLock<Arc<CombinedSerializer>> =
                    LazyLock::new(|| Arc::new($struct_name.into()));
                &INSTANCE
            }
        }

        impl BuildSerializer for $struct_name {
            const EXPECTED_TYPE: &'static str = $expected_type;

            fn build(
                _schema: &Dict,
                _config: Option<&Dict>,
                _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
            ) -> CoreResult<Arc<CombinedSerializer>> {
                Ok(Self::get().clone())
            }
        }

        impl TypeSerializer for $struct_name {
            fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
                match is_type(value, $ob_type) {
                    IsType::Exact => Ok(value.clone()),
                    IsType::Subclass => match state.check {
                        SerCheck::Strict => Err(SerializeError::UnexpectedValue(
                            UnexpectedValue::new_from_msg(None),
                        )),
                        SerCheck::Lax | SerCheck::None => match state.extra.mode {
                            SerMode::Json => Ok($extract(value).unwrap_or_else(|| value.clone())),
                            _ => infer_to_python(value, state),
                        },
                    },
                    IsType::False => {
                        state.warn_fallback_py(self.get_name(), value)?;
                        infer_to_python(value, state)
                    }
                }
            }

            fn json_key<'a>(
                &self,
                key: &'a Value,
                state: &mut SerializationState,
            ) -> SerResult<Cow<'a, str>> {
                match is_type(key, $ob_type) {
                    IsType::Exact | IsType::Subclass => Ok($key_method(key)),
                    IsType::False => {
                        state.warn_fallback_py(self.get_name(), key)?;
                        infer_json_key(key, state)
                    }
                }
            }

            fn serde_serialize<S: Serializer>(
                &self,
                value: &Value,
                serializer: S,
                state: &mut SerializationState,
            ) -> Result<S::Ok, S::Error> {
                match $extract(value) {
                    Some(v) => v.serialize(serializer),
                    None => {
                        state.warn_fallback_ser::<S>(self.get_name(), value)?;
                        infer_serialize(value, serializer, state)
                    }
                }
            }

            fn get_name(&self) -> &str {
                Self::EXPECTED_TYPE
            }

            fn retry_with_lax_check(&self) -> bool {
                $subtypes_allowed
            }
        }
    };
}

build_simple_serializer!(
    IntSerializer,
    "int",
    extract_int,
    ObType::Int,
    to_str_json_key,
    true
);
build_simple_serializer!(
    BoolSerializer,
    "bool",
    extract_bool,
    ObType::Bool,
    bool_json_key,
    false
);
