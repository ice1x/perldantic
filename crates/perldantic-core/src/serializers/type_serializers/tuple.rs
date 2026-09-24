//! `tuple` serializer. Port of upstream `type_serializers/tuple.rs`.

use std::borrow::Cow;
use std::iter;
use std::sync::Arc;

use serde::ser::SerializeSeq;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::{SerResult, SerializeError, UnexpectedValue, py_err_se_err};
use crate::serializers::extra::{IncludeExclude, SerCheck, SerMode, SerializationState};
use crate::serializers::filter::SchemaFilter;
use crate::serializers::infer::{KeyBuilder, infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{
    BuildSerializer, CombinedSerializer, PydanticSerializer, TypeSerializer,
};
use crate::validators::as_dict;
use crate::value::{Dict, Value};

use super::any::AnySerializer;

#[derive(Debug)]
pub struct TupleSerializer {
    serializers: Vec<Arc<CombinedSerializer>>,
    variadic_item_index: Option<usize>,
    filter: SchemaFilter<usize>,
    name: String,
}

impl BuildSerializer for TupleSerializer {
    const EXPECTED_TYPE: &'static str = "tuple";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let items: Vec<Value> = schema.get_as_req("items_schema")?;
        let serializers: Vec<Arc<CombinedSerializer>> = items
            .iter()
            .map(|item| CombinedSerializer::build(as_dict(item)?, config, definitions))
            .collect::<CoreResult<_>>()?;

        let mut serializer_names = serializers.iter().map(|v| v.get_name()).collect::<Vec<_>>();
        let variadic_item_index: Option<usize> = schema.get_as("variadic_item_index")?;
        if let Some(variadic_item_index) = variadic_item_index {
            // As in the tuple validator, an out-of-range index is a schema error
            // (docs/DIVERGENCES.md #11).
            if variadic_item_index >= serializers.len() {
                return crate::build_tools::schema_err!(
                    "`variadic_item_index` {variadic_item_index} is out of range for {} items",
                    serializers.len()
                );
            }
            serializer_names.insert(variadic_item_index + 1, "...");
        }
        let name = format!("tuple[{}]", serializer_names.join(", "));

        Ok(Arc::new(CombinedSerializer::Tuple(Self {
            serializers,
            variadic_item_index,
            filter: SchemaFilter::from_schema(schema)?,
            name,
        })))
    }
}

impl TypeSerializer for TupleSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        let tuple = match value {
            Value::Tuple(tuple) => Some(tuple.as_slice()),
            other => state.extra.perl_array(other),
        };
        match tuple {
            Some(tuple) => {
                let mut items = Vec::with_capacity(tuple.len());
                self.for_each_tuple_item_and_serializer(tuple, state, |entry| {
                    entry
                        .serializer
                        .to_python(entry.item, entry.state)
                        .map(|item| items.push(item))
                })??;
                match state.extra.mode {
                    SerMode::Json => Ok(Value::List(items)),
                    _ => Ok(Value::Tuple(items)),
                }
            }
            None => {
                state.warn_fallback_py(&self.name, value)?;
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
            Value::Tuple(tuple) => {
                let mut key_builder = KeyBuilder::new();
                let state = &mut state.scoped_include_exclude(IncludeExclude::empty());
                self.for_each_tuple_item_and_serializer(tuple, state, |entry| {
                    entry
                        .serializer
                        .json_key(entry.item, entry.state)
                        .map(|key| key_builder.push(&key))
                })??;
                Ok(Cow::Owned(key_builder.finish()))
            }
            _ => {
                state.warn_fallback_py(&self.name, key)?;
                infer_json_key(key, state)
            }
        }
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        let tuple = match value {
            Value::Tuple(tuple) => Some(tuple.as_slice()),
            other => state.extra.perl_array(other),
        };
        match tuple {
            Some(tuple) => {
                let mut seq = serializer.serialize_seq(Some(tuple.len()))?;
                self.for_each_tuple_item_and_serializer(tuple, state, |entry| {
                    seq.serialize_element(&PydanticSerializer::new(
                        entry.item,
                        entry.serializer,
                        entry.state,
                    ))
                })
                .map_err(|e| py_err_se_err(&e))??;
                seq.end()
            }
            None => {
                state.warn_fallback_ser::<S>(&self.name, value)?;
                infer_serialize(value, serializer, state)
            }
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn retry_with_lax_check(&self) -> bool {
        true
    }
}

struct TupleSerializerEntry<'a, 'v> {
    item: &'v Value,
    serializer: &'a CombinedSerializer,
    state: &'a mut SerializationState,
}

impl TupleSerializer {
    /// Try to serialize each item in the tuple with the corresponding serializer.
    ///
    /// If the tuple doesn't match the length of the serializer, in strict mode, an error is
    /// returned.
    ///
    /// The error type E is the type of the error returned by the closure, which is why there are
    /// two levels of `Result`.
    fn for_each_tuple_item_and_serializer<'v, E>(
        &self,
        tuple: &'v [Value],
        state: &mut SerializationState,
        mut f: impl for<'a> FnMut(TupleSerializerEntry<'a, 'v>) -> Result<(), E>,
    ) -> SerResult<Result<(), E>> {
        let n_items = tuple.len();
        let mut tuple_iter = tuple.iter();

        macro_rules! use_serializers {
            ($serializers_iter:expr) => {
                for (index, serializer) in $serializers_iter.enumerate() {
                    let Some(element) = tuple_iter.next() else {
                        break;
                    };
                    if let Some(next_include_exclude) =
                        self.filter.index_filter(index, state, Some(n_items))?
                    {
                        let state = &mut state.scoped_include_exclude(next_include_exclude);
                        if let Err(e) = f(TupleSerializerEntry {
                            item: element,
                            serializer,
                            state,
                        }) {
                            return Ok(Err(e));
                        };
                    }
                }
            };
        }

        if let Some(variadic_item_index) = self.variadic_item_index {
            // Need `saturating_sub` to handle items with too few elements without panicking
            let n_variadic_items = (n_items + 1).saturating_sub(self.serializers.len());
            let serializers_iter = self.serializers[..variadic_item_index]
                .iter()
                .chain(iter::repeat_n(
                    &self.serializers[variadic_item_index],
                    n_variadic_items,
                ))
                .chain(self.serializers[variadic_item_index + 1..].iter());
            use_serializers!(serializers_iter);
        } else if state.check == SerCheck::Strict && n_items != self.serializers.len() {
            return Err(SerializeError::UnexpectedValue(
                UnexpectedValue::new_from_msg(Some(format!(
                    "Expected {} items, but got {}",
                    self.serializers.len(),
                    n_items
                ))),
            ));
        } else {
            use_serializers!(self.serializers.iter());
            if n_items < self.serializers.len() {
                state
                    .warnings
                    .register_warning(UnexpectedValue::new_from_msg(Some(
                        "Unexpected too few items present in tuple".to_string(),
                    )));
            }
            let mut warned = false;
            for (i, element) in tuple_iter.enumerate() {
                if !warned {
                    state
                        .warnings
                        .register_warning(UnexpectedValue::new_from_msg(Some(
                            "Unexpected extra items present in tuple".to_string(),
                        )));
                    warned = true;
                }
                let index = i + self.serializers.len();
                if let Some(next_include_exclude) =
                    self.filter.index_filter(index, state, Some(n_items))?
                {
                    let state = &mut state.scoped_include_exclude(next_include_exclude);
                    if let Err(e) = f(TupleSerializerEntry {
                        item: element,
                        serializer: AnySerializer::get(),
                        state,
                    }) {
                        return Ok(Err(e));
                    }
                }
            }
        }
        Ok(Ok(()))
    }
}
