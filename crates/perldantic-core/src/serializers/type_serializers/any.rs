//! `any` serializer: serializes by inference. Port of upstream `type_serializers/any.rs`.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::SerializationState;
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug, Clone, Default)]
pub struct AnySerializer;

impl AnySerializer {
    pub fn get() -> &'static Arc<CombinedSerializer> {
        static ANY_SERIALIZER: OnceLock<Arc<CombinedSerializer>> = OnceLock::new();
        ANY_SERIALIZER.get_or_init(|| Arc::new(Self.into()))
    }
}

impl BuildSerializer for AnySerializer {
    const EXPECTED_TYPE: &'static str = "any";

    fn build(
        _schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        Ok(Self::get().clone())
    }
}

impl TypeSerializer for AnySerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        infer_to_python(value, state)
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        infer_json_key(key, state)
    }

    fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        infer_serialize(value, serializer, state)
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
