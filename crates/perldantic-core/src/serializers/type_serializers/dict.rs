//! `dict` serializer. Port of upstream `type_serializers/dict.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::ser::SerializeMap;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::{SerResult, py_err_se_err};
use crate::serializers::extra::{IncludeExclude, SerMode, SerializationState};
use crate::serializers::filter::SchemaFilter;
use crate::serializers::infer::{infer_serialize, infer_to_python};
use crate::serializers::shared::{
    BuildSerializer, CombinedSerializer, PydanticSerializer, TypeSerializer,
};
use crate::value::{Dict, Value};

use super::list::sub_serializer;

#[derive(Debug)]
pub struct DictSerializer {
    key_serializer: Arc<CombinedSerializer>,
    value_serializer: Arc<CombinedSerializer>,
    filter: SchemaFilter<Value>,
    name: String,
}

impl BuildSerializer for DictSerializer {
    const EXPECTED_TYPE: &'static str = "dict";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let key_serializer = sub_serializer(schema, "keys_schema", config, definitions)?;
        let value_serializer = sub_serializer(schema, "values_schema", config, definitions)?;
        let filter = match schema.get_as::<Dict>("serialization")? {
            Some(ser) => {
                SchemaFilter::from_set_hash(ser.get_str("include"), ser.get_str("exclude"))?
            }
            None => SchemaFilter::default(),
        };
        let name = format!(
            "{}[{}, {}]",
            Self::EXPECTED_TYPE,
            key_serializer.get_name(),
            value_serializer.get_name()
        );
        Ok(Arc::new(CombinedSerializer::Dict(Self {
            key_serializer,
            value_serializer,
            filter,
            name,
        })))
    }
}

impl TypeSerializer for DictSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match value {
            Value::Dict(dict) => {
                let value_serializer = self.value_serializer.as_ref();
                let mut new_dict = Dict::new();
                for (key, value) in dict.iter() {
                    if let Some(next_include_exclude) = self.filter.key_filter(key, state)? {
                        let key = {
                            // disable include/exclude for keys
                            let state = &mut state.scoped_include_exclude(IncludeExclude::empty());
                            match state.extra.mode {
                                SerMode::Json => Value::Str(
                                    self.key_serializer.json_key(key, state)?.into_owned(),
                                ),
                                _ => self.key_serializer.to_python(key, state)?,
                            }
                        };
                        let state = &mut state.scoped_include_exclude(next_include_exclude);
                        let value = value_serializer.to_python(value, state)?;
                        new_dict.insert(key, value);
                    }
                }
                Ok(Value::Dict(new_dict))
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
            Value::Dict(dict) => {
                let mut map = serializer.serialize_map(Some(dict.len()))?;
                let key_serializer = self.key_serializer.as_ref();
                let value_serializer = self.value_serializer.as_ref();
                for (key, value) in dict.iter() {
                    let op_next = self
                        .filter
                        .key_filter(key, state)
                        .map_err(|e| py_err_se_err(&e))?;
                    if let Some(next_include_exclude) = op_next {
                        let state = &mut state.scoped_include_exclude(next_include_exclude);
                        let key = key_serializer
                            .json_key(key, state)
                            .map_err(|e| py_err_se_err(&e))?;
                        let value_serialize =
                            PydanticSerializer::new(value, value_serializer, state);
                        map.serialize_entry(&key, &value_serialize)?;
                    }
                }
                map.end()
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
