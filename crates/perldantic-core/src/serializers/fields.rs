//! Serializing the fields of models and typed dicts. Port of upstream `serializers/fields.rs`.
//!
//! `serialization_exclude_if` needs a host callback that is not wired yet and is rejected when
//! the serializer is built.

use std::borrow::Cow;
use std::sync::Arc;

use serde::ser::SerializeMap;

use crate::value::{Dict, Value};

use super::computed_fields::ComputedFields;
use super::errors::{SerResult, SerializeError, UnexpectedValue, py_err_se_err};
use super::extra::{Extra, IncludeExclude, SerCheck, SerializationState};
use super::filter::SchemaFilter;
use super::infer::{infer_serialize, infer_to_python};
use super::shared::{CombinedSerializer, PydanticSerializer, TypeSerializer};
use super::type_serializers::any::AnySerializer;

/// representation of a field for serialization
#[derive(Debug)]
pub(super) struct SerField {
    pub key: String,
    pub alias: Option<String>,
    // None serializer means exclude
    pub serializer: Option<Arc<CombinedSerializer>>,
    pub required: bool,
    pub serialize_by_alias: Option<bool>,
}

impl SerField {
    pub fn new(
        key: String,
        alias: Option<String>,
        serializer: Option<Arc<CombinedSerializer>>,
        required: bool,
        serialize_by_alias: Option<bool>,
    ) -> Self {
        Self {
            key,
            alias,
            serializer,
            required,
            serialize_by_alias,
        }
    }

    pub fn get_key(&self, extra: &Extra) -> &str {
        if extra.serialize_by_alias_or(self.serialize_by_alias)
            && let Some(alias) = &self.alias
        {
            return alias;
        }
        &self.key
    }
}

fn exclude_default(
    value: &Value,
    extra: &Extra,
    serializer: &CombinedSerializer,
) -> SerResult<bool> {
    if extra.exclude_defaults
        && let Some(default) = serializer.get_default()?
        && value.py_eq(&default)
    {
        return Ok(true);
    }
    Ok(false)
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) enum FieldsMode {
    // typeddict with no extra items
    SimpleDict,
    // a model - `GeneralFieldsSerializer` will get a tuple of the fields and the extra values
    ModelExtra,
    // typeddict with extra items: keys that are not fields are extra values
    TypedDictAllow,
}

/// General purpose serializer for fields - used by models and typed dicts
#[derive(Debug)]
pub struct GeneralFieldsSerializer {
    fields: Vec<SerField>,
    mode: FieldsMode,
    extra_serializer: Option<Arc<CombinedSerializer>>,
    filter: SchemaFilter<Value>,
    required_fields: usize,
    computed_fields: Option<ComputedFields>,
}

impl GeneralFieldsSerializer {
    pub(super) fn new(
        fields: Vec<SerField>,
        mode: FieldsMode,
        extra_serializer: Option<Arc<CombinedSerializer>>,
        computed_fields: Option<ComputedFields>,
    ) -> Self {
        let required_fields = fields.iter().filter(|f| f.required).count();
        Self {
            fields,
            mode,
            extra_serializer,
            filter: SchemaFilter::default(),
            required_fields,
            computed_fields,
        }
    }

    /// Whether the value it takes carries extra values (a model that allows them).
    pub(crate) fn takes_extra(&self) -> bool {
        matches!(self.mode, FieldsMode::ModelExtra)
    }

    fn field(&self, key: &str) -> Option<&SerField> {
        self.fields.iter().find(|f| f.key == key)
    }

    fn extract_dicts<'a>(&self, value: &'a Value) -> Option<(&'a Dict, Option<&'a Dict>)> {
        match self.mode {
            FieldsMode::ModelExtra => match value {
                Value::Tuple(items) => match items.as_slice() {
                    [Value::Dict(main), Value::Dict(extra)] => Some((main, Some(extra))),
                    [Value::Dict(main), Value::None] => Some((main, None)),
                    _ => None,
                },
                _ => None,
            },
            FieldsMode::SimpleDict | FieldsMode::TypedDictAllow => match value {
                Value::Dict(main) => Some((main, None)),
                _ => None,
            },
        }
    }

    /// Visit the entries to output, in order: fields, then extra values. `emit` receives the
    /// output key, the value, its serializer and the state scoped for it.
    fn for_each_entry<E: From<SerializeError>>(
        &self,
        main_dict: &Dict,
        extra_dict: Option<&Dict>,
        state: &mut SerializationState,
        mut emit: impl FnMut(
            &str,
            &Value,
            &CombinedSerializer,
            &mut SerializationState,
        ) -> Result<(), E>,
    ) -> Result<(), E> {
        // fields not set on a model, masked when `exclude_unset` is on; nested serializers must
        // not see them
        let unset_fields = state.unset_fields.take();
        let is_unset = |key: &str| {
            unset_fields
                .as_ref()
                .is_some_and(|unset| unset.iter().any(|k| k == key))
        };
        let mut used_req_fields: usize = 0;

        // If using `serialize_as_any`, extras are always inferred
        let extras_serializer = self
            .extra_serializer
            .as_ref()
            .filter(|_| !state.extra.serialize_as_any)
            .unwrap_or_else(|| AnySerializer::get());

        for (key, value) in main_dict.iter() {
            let Value::Str(key_str) = key else {
                return Err(
                    SerializeError::Core(crate::core_error::CoreError::Type(format!(
                        "'{}' object cannot be converted to 'PyString'",
                        key.type_name()
                    )))
                    .into(),
                );
            };
            let state = &mut state.scoped_set_field_name(Some(key_str.clone()));
            if let Some(field) = self.field(key_str) {
                if field.required {
                    used_req_fields += 1;
                }
                let Some((serializer, next_include_exclude)) =
                    self.prepare_value(key, value, field, state, is_unset(key_str))?
                else {
                    // field was excluded
                    continue;
                };
                let state = &mut state.scoped_include_exclude(next_include_exclude);
                emit(field.get_key(&state.extra), value, serializer, state)?;
            } else if self.mode == FieldsMode::TypedDictAllow {
                if let Some(next_include_exclude) = self.filter.key_filter(key, state)?
                    && !exclude_field_by_value(value, state)
                {
                    let state = &mut state.scoped_include_exclude(next_include_exclude);
                    emit(key_str, value, extras_serializer, state)?;
                }
            } else if state.check == SerCheck::Strict {
                return Err(unexpected_field(key_str, state).into());
            }
        }

        if state.check.enabled() && self.required_fields > used_req_fields {
            return Err(incorrect_field_count(self.required_fields, used_req_fields, state).into());
        }

        for (key, value) in extra_dict.into_iter().flat_map(Dict::iter) {
            if let Some(next_include_exclude) = self.filter.key_filter(key, state)?
                && !exclude_field_by_value(value, state)
            {
                let Value::Str(key_str) = key else {
                    continue;
                };
                let state = &mut state.scoped_include_exclude(next_include_exclude);
                emit(key_str, value, extras_serializer, state)?;
            }
        }

        if let Some(computed_fields) = &self.computed_fields {
            let model = get_model(state)?.clone();
            computed_fields.for_each(&model, &self.filter, state, &mut emit)?;
        }
        Ok(())
    }

    /// Gets the serializer to use for a field, applying `serialize_as_any` logic and applying any
    /// field-level exclusions
    fn prepare_value<'s>(
        &self,
        key: &Value,
        value: &Value,
        field: &'s SerField,
        state: &SerializationState,
        unset: bool,
    ) -> SerResult<Option<(&'s CombinedSerializer, IncludeExclude)>> {
        // if field excluded at schema level, this is the cheapest exclusion
        let Some(serializer) = field.serializer.as_ref() else {
            return Ok(None);
        };

        // filtering on the keys
        let Some(next_include_exclude) = self.filter.key_filter(key, state)? else {
            return Ok(None);
        };

        // filtering on the value
        if unset
            || exclude_field_by_value(value, state)
            || exclude_default(value, &state.extra, serializer)?
        {
            return Ok(None);
        }

        let serializer: &CombinedSerializer = if state.extra.serialize_as_any {
            AnySerializer::get()
        } else {
            serializer
        };

        Ok(Some((serializer, next_include_exclude)))
    }
}

/// Common logic for excluding fields during serialization
fn exclude_field_by_value(value: &Value, state: &SerializationState) -> bool {
    state.extra.exclude_none && matches!(value, Value::None)
}

impl TypeSerializer for GeneralFieldsSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        let Some((main_dict, extra_dict)) = self.extract_dicts(value) else {
            state.warn_fallback_py(self.get_name(), value)?;
            return infer_to_python(value, state);
        };
        self.dicts_to_python(main_dict, extra_dict, state)
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        self.invalid_as_json_key(key, state, "fields")
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        let Some((main_dict, extra_dict)) = self.extract_dicts(value) else {
            state.warn_fallback_ser::<S>(self.get_name(), value)?;
            return infer_serialize(value, serializer, state);
        };
        self.dicts_serde_serialize(main_dict, extra_dict, serializer, state)
    }

    fn get_name(&self) -> &'static str {
        "general-fields"
    }
}

impl GeneralFieldsSerializer {
    /// Serialize the fields and extra values of a model (or the keys of a typed dict), given as
    /// dicts: the model serializer hands its model's own dicts over without copying them.
    pub(crate) fn dicts_to_python(
        &self,
        main_dict: &Dict,
        extra_dict: Option<&Dict>,
        state: &mut SerializationState,
    ) -> SerResult<Value> {
        get_model(state)?;
        let mut out = Dict::new();
        self.for_each_entry(
            main_dict,
            extra_dict,
            state,
            |key, value, serializer, state| {
                let value = serializer.to_python_no_infer(value, state)?;
                // fields and extra values have distinct names
                out.push_new(Value::from(key), value);
                Ok::<(), SerializeError>(())
            },
        )?;
        Ok(Value::Dict(out))
    }

    /// As [`Self::dicts_to_python`], to a serde serializer.
    pub(crate) fn dicts_serde_serialize<S: serde::ser::Serializer>(
        &self,
        main_dict: &Dict,
        extra_dict: Option<&Dict>,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        get_model(state).map_err(|e| py_err_se_err(&e))?;
        let mut map = serializer.serialize_map(None)?;
        let result = self.for_each_entry(
            main_dict,
            extra_dict,
            state,
            |key, value, serializer, state| {
                let value_ser = PydanticSerializer::new_no_infer(value, serializer, state);
                map.serialize_entry(key, &value_ser)
                    .map_err(SerErrorOrSe::Se)
            },
        );
        match result {
            Ok(()) => map.end(),
            Err(SerErrorOrSe::Se(e)) => Err(e),
            Err(SerErrorOrSe::Ser(e)) => Err(py_err_se_err(&e)),
        }
    }
}

/// An error from our code or from the serde serializer, while writing entries.
enum SerErrorOrSe<E> {
    Ser(SerializeError),
    Se(E),
}

impl<E> From<SerializeError> for SerErrorOrSe<E> {
    fn from(err: SerializeError) -> Self {
        Self::Ser(err)
    }
}

fn get_model(state: &SerializationState) -> SerResult<&Value> {
    state.model.as_ref().ok_or_else(|| {
        SerializeError::UnexpectedValue(UnexpectedValue::new_from_msg(Some(
            "No model found for fields serialization".to_string(),
        )))
    })
}

fn model_type_name(state: &SerializationState) -> Option<String> {
    state.model.as_ref().map(|m| m.type_name().to_owned())
}

#[cold]
fn unexpected_field(key: &str, state: &SerializationState) -> SerializeError {
    SerializeError::UnexpectedValue(UnexpectedValue::new(
        Some(format!("Unexpected field `{key}`")),
        Some(key.to_owned()),
        model_type_name(state),
        None,
    ))
}

#[cold]
fn incorrect_field_count(
    expected_fields: usize,
    used_fields: usize,
    state: &SerializationState,
) -> SerializeError {
    SerializeError::UnexpectedValue(UnexpectedValue::new(
        Some(format!(
            "Expected {expected_fields} fields but got {used_fields}"
        )),
        state.field_name().map(str::to_owned),
        model_type_name(state),
        state.model.clone(),
    ))
}
