//! `set` serializer. Port of upstream `type_serializers/set_frozenset.rs` (frozensets come
//! when `Value` has them).

use std::borrow::Cow;
use std::sync::Arc;

use serde::ser::SerializeSeq;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_serialize, infer_to_python};
use crate::serializers::shared::{
    BuildSerializer, CombinedSerializer, PydanticSerializer, TypeSerializer,
};
use crate::value::{Dict, Value};

use super::list::sub_serializer;

#[derive(Debug)]
pub struct SetSerializer {
    item_serializer: Arc<CombinedSerializer>,
    name: String,
}

impl BuildSerializer for SetSerializer {
    const EXPECTED_TYPE: &'static str = "set";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let item_serializer = sub_serializer(schema, "items_schema", config, definitions)?;
        let name = format!("{}[{}]", Self::EXPECTED_TYPE, item_serializer.get_name());
        Ok(Arc::new(
            Self {
                item_serializer,
                name,
            }
            .into(),
        ))
    }
}

impl TypeSerializer for SetSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match value {
            Value::Set(set) => {
                let item_serializer = self.item_serializer.as_ref();
                let mut items: Vec<Value> = Vec::with_capacity(set.len());
                for element in set {
                    let item = item_serializer.to_python(element, state)?;
                    if state.extra.mode == SerMode::Json || !items.iter().any(|i| i.py_eq(&item)) {
                        items.push(item);
                    }
                }
                match state.extra.mode {
                    SerMode::Json => Ok(Value::List(items)),
                    _ => Ok(Value::Set(items)),
                }
            }
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
        self.invalid_as_json_key(key, state, Self::EXPECTED_TYPE)
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Value::Set(set) => {
                let mut seq = serializer.serialize_seq(Some(set.len()))?;
                let item_serializer = self.item_serializer.as_ref();
                for element in set {
                    let item_serialize = PydanticSerializer::new(element, item_serializer, state);
                    seq.serialize_element(&item_serialize)?;
                }
                seq.end()
            }
            _ => {
                state.warn_fallback_ser::<S>(self.get_name(), value)?;
                infer_serialize(value, serializer, state)
            }
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
