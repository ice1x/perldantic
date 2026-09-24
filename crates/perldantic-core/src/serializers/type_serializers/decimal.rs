//! `decimal` serializer. Port of upstream `type_serializers/decimal.rs`: decimals are
//! serialized by inference (their text in JSON mode).

use std::borrow::Cow;
use std::sync::Arc;

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::SerializationState;
use crate::serializers::infer::{
    infer_json_key, infer_json_key_known, infer_serialize, infer_serialize_known, infer_to_python,
    infer_to_python_known,
};
use crate::serializers::ob_type::ObType;
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct DecimalSerializer;

impl BuildSerializer for DecimalSerializer {
    const EXPECTED_TYPE: &'static str = "decimal";

    fn build(
        _schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        Ok(Arc::new(Self.into()))
    }
}

impl TypeSerializer for DecimalSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match value {
            Value::Decimal(_) => infer_to_python_known(ObType::Decimal, value, state),
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
            Value::Decimal(_) => infer_json_key_known(ObType::Decimal, key, state),
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
            Value::Decimal(_) => infer_serialize_known(ObType::Decimal, value, serializer, state),
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
