//! `uuid` serializer. Port of upstream `type_serializers/uuid.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct UuidSerializer;

impl BuildSerializer for UuidSerializer {
    const EXPECTED_TYPE: &'static str = "uuid";

    fn build(
        _schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        Ok(Arc::new(Self.into()))
    }
}

impl TypeSerializer for UuidSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match value {
            Value::Uuid(uuid) => match state.extra.mode {
                SerMode::Json => Ok(Value::Str(uuid.to_string())),
                _ => Ok(value.clone()),
            },
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
        match key {
            Value::Uuid(uuid) => Ok(Cow::Owned(uuid.to_string())),
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
        match value {
            Value::Uuid(uuid) => serializer.serialize_str(&uuid.to_string()),
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
