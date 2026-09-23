//! `default` serializer. Port of upstream `type_serializers/with_default.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::SerializationState;
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::validators::with_default::DefaultType;
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct WithDefaultSerializer {
    default: DefaultType,
    serializer: Arc<CombinedSerializer>,
}

impl BuildSerializer for WithDefaultSerializer {
    const EXPECTED_TYPE: &'static str = "default";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let default = DefaultType::new(schema)?;
        let sub_schema: Dict = schema.get_as_req("schema")?;
        let serializer = CombinedSerializer::build(&sub_schema, config, definitions)?;
        Ok(Arc::new(
            Self {
                default,
                serializer,
            }
            .into(),
        ))
    }
}

impl TypeSerializer for WithDefaultSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        self.serializer.to_python(value, state)
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        self.serializer.json_key(key, state)
    }

    fn serde_serialize<S: serde::ser::Serializer>(
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

    fn retry_with_lax_check(&self) -> bool {
        self.serializer.retry_with_lax_check()
    }

    fn get_default(&self) -> CoreResult<Option<Value>> {
        Ok(self.default.default_value())
    }
}
