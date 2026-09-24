//! `float` serializer. Port of upstream `type_serializers/float.rs`.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

use num_traits::ToPrimitive;
use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::config::InfNanMode;
use crate::serializers::errors::{SerResult, SerializeError, UnexpectedValue};
use crate::serializers::extra::{SerCheck, SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::ob_type::{IsType, ObType, is_type};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

use super::simple::to_str_json_key;

#[derive(Debug)]
pub struct FloatSerializer {
    inf_nan_mode: InfNanMode,
}

static FLOAT_SERIALIZER_NULL: LazyLock<Arc<CombinedSerializer>> = LazyLock::new(|| {
    Arc::new(CombinedSerializer::Float(FloatSerializer {
        inf_nan_mode: InfNanMode::Null,
    }))
});

static FLOAT_SERIALIZER_CONSTANTS: LazyLock<Arc<CombinedSerializer>> = LazyLock::new(|| {
    Arc::new(CombinedSerializer::Float(FloatSerializer {
        inf_nan_mode: InfNanMode::Constants,
    }))
});

static FLOAT_SERIALIZER_STRINGS: LazyLock<Arc<CombinedSerializer>> = LazyLock::new(|| {
    Arc::new(CombinedSerializer::Float(FloatSerializer {
        inf_nan_mode: InfNanMode::Strings,
    }))
});

impl FloatSerializer {
    pub fn get(config: Option<&Dict>) -> CoreResult<&'static Arc<CombinedSerializer>> {
        match InfNanMode::from_config(config)? {
            InfNanMode::Null => Ok(&FLOAT_SERIALIZER_NULL),
            InfNanMode::Constants => Ok(&FLOAT_SERIALIZER_CONSTANTS),
            InfNanMode::Strings => Ok(&FLOAT_SERIALIZER_STRINGS),
        }
    }
}

pub fn serialize_f64<S: Serializer>(
    v: f64,
    serializer: S,
    inf_nan_mode: InfNanMode,
) -> Result<S::Ok, S::Error> {
    if v.is_nan() || v.is_infinite() {
        match inf_nan_mode {
            InfNanMode::Null => serializer.serialize_none(),
            InfNanMode::Constants => serializer.serialize_f64(v),
            InfNanMode::Strings => {
                if v.is_nan() {
                    serializer.serialize_str("NaN")
                } else {
                    serializer.serialize_str(if v.is_sign_positive() {
                        "Infinity"
                    } else {
                        "-Infinity"
                    })
                }
            }
        }
    } else {
        serializer.serialize_f64(v)
    }
}

/// pyo3's `extract::<f64>()`: floats, ints (converted) and bools.
fn extract_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Float(f) => Some(*f),
        #[allow(clippy::cast_precision_loss)] // as Python's float(int)
        Value::Int(i) => Some(*i as f64),
        Value::BigInt(i) => i.to_f64().filter(|f| f.is_finite()),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Enum(_) => value.mixin_value().and_then(extract_f64),
        _ => None,
    }
}

impl BuildSerializer for FloatSerializer {
    const EXPECTED_TYPE: &'static str = "float";

    fn build(
        _schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        Self::get(config).cloned()
    }
}

impl TypeSerializer for FloatSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match is_type(value, ObType::Float) {
            IsType::Exact => Ok(value.clone()),
            IsType::Subclass => match state.check {
                SerCheck::Strict => Err(SerializeError::UnexpectedValue(
                    UnexpectedValue::new_from_msg(None),
                )),
                SerCheck::Lax | SerCheck::None => match state.extra.mode {
                    SerMode::Json => {
                        Ok(extract_f64(value).map_or_else(|| value.clone(), Value::Float))
                    }
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
        match is_type(key, ObType::Float) {
            IsType::Exact | IsType::Subclass => Ok(to_str_json_key(key)),
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
        match extract_f64(value) {
            Some(v) => serialize_f64(v, serializer, self.inf_nan_mode),
            None => {
                state.warn_fallback_ser::<S>(self.get_name(), value)?;
                infer_serialize(value, serializer, state)
            }
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
