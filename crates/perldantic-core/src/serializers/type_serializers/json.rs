//! `json` serializer. Port of upstream `type_serializers/json.rs`: the parsed data is
//! serialized with the inner schema (`round_trip`, which writes JSON text back, needs host
//! callbacks and is not supported yet).

use std::borrow::Cow;
use std::sync::Arc;

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::SerializationState;
use crate::serializers::infer::infer_json_key;
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

use super::any::AnySerializer;

#[derive(Debug)]
pub struct JsonSerializer {
    serializer: Arc<CombinedSerializer>,
}

impl BuildSerializer for JsonSerializer {
    const EXPECTED_TYPE: &'static str = "json";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let serializer = match schema.get_str("schema") {
            Some(Value::Dict(inner)) => CombinedSerializer::build(inner, config, definitions)?,
            _ => AnySerializer::build(schema, config, definitions)?,
        };
        Ok(Arc::new(Self { serializer }.into()))
    }
}

impl TypeSerializer for JsonSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        self.serializer.to_python(value, state)
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
        self.serializer.serde_serialize(value, serializer, state)
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
