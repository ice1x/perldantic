//! `list` serializer. Port of upstream `type_serializers/list.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::ser::SerializeSeq;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::{SerResult, py_err_se_err};
use crate::serializers::extra::SerializationState;
use crate::serializers::filter::SchemaFilter;
use crate::serializers::infer::{infer_serialize, infer_to_python};
use crate::serializers::shared::{
    BuildSerializer, CombinedSerializer, PydanticSerializer, TypeSerializer,
};
use crate::validators::as_dict;
use crate::value::{Dict, Value};

use super::any::AnySerializer;

#[derive(Debug)]
pub struct ListSerializer {
    item_serializer: Arc<CombinedSerializer>,
    filter: SchemaFilter<usize>,
    name: String,
}

/// The serializer of an `items_schema` (or `keys_schema` / `values_schema`), `any` without one.
pub(crate) fn sub_serializer(
    schema: &Dict,
    key: &str,
    config: Option<&Dict>,
    definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
) -> CoreResult<Arc<CombinedSerializer>> {
    match schema.get_str(key) {
        Some(sub_schema) => CombinedSerializer::build(as_dict(sub_schema)?, config, definitions),
        None => AnySerializer::build(schema, config, definitions),
    }
}

impl BuildSerializer for ListSerializer {
    const EXPECTED_TYPE: &'static str = "list";

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
                filter: SchemaFilter::from_schema(schema)?,
                name,
            }
            .into(),
        ))
    }
}

impl TypeSerializer for ListSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match value {
            Value::List(list) => {
                let item_serializer = self.item_serializer.as_ref();
                let mut items = Vec::with_capacity(list.len());
                for (index, element) in list.iter().enumerate() {
                    let op_next = self.filter.index_filter(index, state, Some(list.len()))?;
                    if let Some(next_include_exclude) = op_next {
                        let state = &mut state.scoped_include_exclude(next_include_exclude);
                        items.push(item_serializer.to_python(element, state)?);
                    }
                }
                Ok(Value::List(items))
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
            Value::List(list) => {
                let mut seq = serializer.serialize_seq(Some(list.len()))?;
                let item_serializer = self.item_serializer.as_ref();
                for (index, element) in list.iter().enumerate() {
                    let op_next = self
                        .filter
                        .index_filter(index, state, Some(list.len()))
                        .map_err(|e| py_err_se_err(&e))?;
                    if let Some(next_include_exclude) = op_next {
                        let state = &mut state.scoped_include_exclude(next_include_exclude);
                        let item_serialize =
                            PydanticSerializer::new(element, item_serializer, state);
                        seq.serialize_element(&item_serialize)?;
                    }
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

    fn retry_with_lax_check(&self) -> bool {
        self.item_serializer.retry_with_lax_check()
    }
}
