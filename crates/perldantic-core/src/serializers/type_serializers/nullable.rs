//! `nullable` serializer. Port of upstream `type_serializers/nullable.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::SerializationState;
use crate::serializers::infer::infer_json_key_known;
use crate::serializers::ob_type::{IsType, ObType, is_type};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct NullableSerializer {
    serializer: Arc<CombinedSerializer>,
}

impl BuildSerializer for NullableSerializer {
    const EXPECTED_TYPE: &'static str = "nullable";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let sub_schema: Dict = schema.get_as_req("schema")?;
        Ok(Arc::new(CombinedSerializer::Nullable(Self {
            serializer: CombinedSerializer::build(&sub_schema, config, definitions)?,
        })))
    }
}

impl TypeSerializer for NullableSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match is_type(value, ObType::None) {
            IsType::Exact => Ok(Value::None),
            // I don't think subclasses of None can exist
            _ => self.serializer.to_python(value, state),
        }
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        match is_type(key, ObType::None) {
            IsType::Exact => infer_json_key_known(ObType::None, key, state),
            _ => self.serializer.json_key(key, state),
        }
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        match is_type(value, ObType::None) {
            IsType::Exact => serializer.serialize_none(),
            _ => self.serializer.serde_serialize(value, serializer, state),
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }

    fn retry_with_lax_check(&self) -> bool {
        self.serializer.retry_with_lax_check()
    }
}
