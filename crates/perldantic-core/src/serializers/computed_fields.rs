//! Computed fields of models and typed dicts. Port of upstream `serializers/computed_fields.rs`.
//!
//! Upstream reads a computed field with `getattr(model, property_name)`; the core has no host
//! objects, so each `computed-field` schema also names the host function that computes it
//! (`function`), called with the model and the property name (docs/DIVERGENCES.md #20).

use std::sync::Arc;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::host::{Function, HostCall, HostError};
use crate::value::{Dict, Value};

use super::errors::{SerResult, SerializeError};
use super::extra::SerializationState;
use super::filter::SchemaFilter;
use super::shared::{BuildSerializer, CombinedSerializer};

#[derive(Debug)]
pub(crate) struct ComputedFields(Vec<ComputedField>);

#[derive(Debug)]
struct ComputedField {
    property_name: String,
    serializer: Arc<CombinedSerializer>,
    alias: String,
    serialize_by_alias: Option<bool>,
    function: Function,
}

impl ComputedFields {
    /// The schema's `computed_fields`, if it lists any.
    pub(crate) fn new(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Option<Self>> {
        let computed_fields: Vec<Value> = schema.get_as("computed_fields")?.unwrap_or_default();
        if computed_fields.is_empty() {
            return Ok(None);
        }
        let serialize_by_alias: Option<bool> = config.get_as("serialize_by_alias")?;
        let fields = computed_fields
            .iter()
            .map(|field| {
                let Value::Dict(field) = field else {
                    return schema_err!("computed fields should be dicts");
                };
                ComputedField::new(field, config, definitions, serialize_by_alias)
            })
            .collect::<CoreResult<Vec<_>>>()?;
        Ok(Some(Self(fields)))
    }

    /// The computed fields to output for `model`, with their output key and serializer; `emit`
    /// gets them in declaration order, the state scoped for each.
    pub fn for_each<E: From<SerializeError>>(
        &self,
        model: &Value,
        filter: &SchemaFilter<Value>,
        state: &mut SerializationState,
        mut emit: impl FnMut(
            &str,
            &Value,
            &CombinedSerializer,
            &mut SerializationState,
        ) -> Result<(), E>,
    ) -> Result<(), E> {
        for field in &self.0 {
            let key = Value::Str(field.property_name.clone());
            let Some(next_include_exclude) = filter.key_filter(&key, state)? else {
                continue;
            };
            let value = field.value(model)?;
            if state.extra.exclude_none && matches!(value, Value::None) {
                continue;
            }
            let state = &mut state.scoped_set_field_name(Some(field.property_name.clone()));
            let state = &mut state.scoped_include_exclude(next_include_exclude);
            let key = if state.extra.serialize_by_alias_or(field.serialize_by_alias) {
                &field.alias
            } else {
                &field.property_name
            };
            emit(key, &value, &field.serializer, state)?;
        }
        Ok(())
    }
}

impl ComputedField {
    fn new(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
        serialize_by_alias: Option<bool>,
    ) -> CoreResult<Self> {
        let property_name: String = schema.get_as_req("property_name")?;
        let return_schema: Dict = schema.get_as_req("return_schema")?;
        let serializer = CombinedSerializer::build(&return_schema, config, definitions)
            .map_err(|e| CoreError::Schema(format!("Computed field `{property_name}`:\n  {e}")))?;
        let function = match schema.get_str("function") {
            Some(Value::Function(function)) => function.clone(),
            _ => {
                return schema_err!(
                    "Computed field `{property_name}` needs the host `function` computing it"
                );
            }
        };
        if schema
            .get_str("serialization_exclude_if")
            .is_some_and(|v| !matches!(v, Value::None))
        {
            return schema_err!(
                "Computed field `{property_name}`: `serialization_exclude_if` is not supported yet"
            );
        }
        let alias = schema
            .get_as("alias")?
            .unwrap_or_else(|| property_name.clone());
        Ok(Self {
            property_name,
            serializer,
            alias,
            serialize_by_alias,
            function,
        })
    }

    /// The field's value: what the host's function computes from the model. A failure is the
    /// host's own exception (upstream's `getattr` raises it unchanged).
    fn value(&self, model: &Value) -> SerResult<Value> {
        self.function
            .call(HostCall::Property {
                model: model.clone(),
                name: self.property_name.clone(),
            })
            .map_err(|e| match e {
                HostError::Serialization(e) => e,
                HostError::Core(e) => SerializeError::Core(e),
                HostError::Other(exception) => SerializeError::Core(CoreError::Host(exception)),
                other => SerializeError::Serialization(format!(
                    "Error calling function `{}`: {other}",
                    self.function.name()
                )),
            })
    }
}
