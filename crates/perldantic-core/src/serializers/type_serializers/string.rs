//! `str` serializer. Port of upstream `type_serializers/string.rs`.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::ob_type::{IsType, ObType, is_type};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct StrSerializer;

static STR_SERIALIZER: LazyLock<Arc<CombinedSerializer>> =
    LazyLock::new(|| Arc::new(StrSerializer.into()));

impl StrSerializer {
    pub fn get() -> &'static Arc<CombinedSerializer> {
        &STR_SERIALIZER
    }
}

impl BuildSerializer for StrSerializer {
    const EXPECTED_TYPE: &'static str = "str";

    fn build(
        _schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        Ok(Self::get().clone())
    }
}

/// The text of a `str`, or of a `str` subclass instance (a `StrEnum` member).
fn as_str(value: &Value) -> Option<&str> {
    match value.mixin_value().unwrap_or(value) {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

impl TypeSerializer for StrSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match is_type(value, ObType::Str) {
            IsType::Exact => Ok(value.clone()),
            IsType::Subclass => match state.extra.mode {
                SerMode::Json => Ok(as_str(value).map_or_else(|| value.clone(), Value::from)),
                _ => Ok(value.clone()),
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
        if let Some(s) = as_str(key) {
            Ok(Cow::Borrowed(s))
        } else {
            state.warn_fallback_py(self.get_name(), key)?;
            infer_json_key(key, state)
        }
    }

    fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        if let Some(s) = as_str(value) {
            serializer.serialize_str(s)
        } else {
            state.warn_fallback_ser::<S>(self.get_name(), value)?;
            infer_serialize(value, serializer, state)
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
