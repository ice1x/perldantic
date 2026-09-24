//! Serialization by inspecting the value, used by `any` and as the fallback for values that do
//! not match their serializer. Port of upstream `serializers/infer.rs` for the kinds `Value`
//! has.
//!
//! A model instance found by inference is serialized as a dict of its fields and extra values;
//! upstream would use the serializer of the model's class, which the core has no registry of
//! (docs/DIVERGENCES.md #14).

use std::borrow::Cow;
use std::cell::RefCell;

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use crate::core_error::CoreError;
use crate::value::{Dict, Value};

use super::config::InfNanMode;
use super::errors::{SerResult, SerializeError, py_err_se_err};
use super::extra::{IncludeExclude, SerMode, SerializationState};
use super::filter::AnyFilter;
use super::ob_type::{ObType, get_type};
use super::type_serializers::float::serialize_f64;

// arbitrary id to identify that we recursed through infer_to_{python,json}_known
// We just need them to be different from definition ref slot ids, which start at 0
const INFER_DEF_REF_ID: usize = usize::MAX;

pub(crate) fn infer_to_python(value: &Value, state: &mut SerializationState) -> SerResult<Value> {
    infer_to_python_known(get_type(value), value, state)
}

pub(crate) fn infer_to_python_known(
    ob_type: ObType,
    value: &Value,
    state: &mut SerializationState,
) -> SerResult<Value> {
    let is_json = matches!(state.extra.mode, SerMode::Json);
    let mut guard = match state.recursion_guard(value, INFER_DEF_REF_ID) {
        Ok(v) => v,
        Err(e) => {
            return if is_json {
                Err(e)
            } else {
                // if recursion is detected by we're serializing to python, we just return the value
                Ok(value.clone())
            };
        }
    };
    let state = guard.state();

    let serialize_seq =
        |items: &[Value], state: &mut SerializationState| -> SerResult<Vec<Value>> {
            let state = &mut state.scoped_include_exclude(IncludeExclude::empty());
            items.iter().map(|v| infer_to_python(v, state)).collect()
        };
    let serialize_seq_filter =
        |items: &[Value], state: &mut SerializationState| -> SerResult<Vec<Value>> {
            let len = items.len();
            let mut out = Vec::with_capacity(len);
            let filter = AnyFilter::new();
            for (index, element) in items.iter().enumerate() {
                if let Some(next_include_exclude) = filter.index_filter(index, state, Some(len))? {
                    let state = &mut state.scoped_include_exclude(next_include_exclude);
                    out.push(infer_to_python(element, state)?);
                }
            }
            Ok(out)
        };

    let value = match state.extra.mode {
        SerMode::Json => match (ob_type, value) {
            (ObType::Float, Value::Float(v))
                if (v.is_nan() || v.is_infinite())
                    && state.config.inf_nan_mode == InfNanMode::Null =>
            {
                Value::None
            }
            (ObType::Bytes, Value::Bytes(b)) => {
                Value::Str(state.config.bytes_mode.bytes_to_string(b)?.into_owned())
            }
            (ObType::Tuple, Value::Tuple(items)) | (ObType::List, Value::List(items)) => {
                Value::List(serialize_seq_filter(items, state)?)
            }
            (ObType::Set, Value::Set(items)) => Value::List(serialize_seq(items, state)?),
            (ObType::Dict, Value::Dict(dict)) => Value::Dict(pairs_to_python(dict.iter(), state)?),
            (ObType::PydanticSerializable, Value::Model(model)) => {
                let extra = model.extra.iter().flat_map(Dict::iter);
                Value::Dict(pairs_to_python(model.fields.iter().chain(extra), state)?)
            }
            (ObType::Datetime | ObType::Date | ObType::Time | ObType::Timedelta, _) => state
                .config
                .temporal_mode
                .to_json(value)
                .unwrap_or_else(|| value.clone()),
            (ObType::Uuid | ObType::Url | ObType::MultiHostUrl, _) => Value::Str(value.py_str()),
            _ => value.clone(),
        },
        _ => match (ob_type, value) {
            (ObType::Tuple, Value::Tuple(items)) => {
                Value::Tuple(serialize_seq_filter(items, state)?)
            }
            (ObType::List, Value::List(items)) => Value::List(serialize_seq_filter(items, state)?),
            (ObType::Set, Value::Set(items)) => Value::Set(serialize_seq(items, state)?),
            (ObType::Dict, Value::Dict(dict)) => Value::Dict(pairs_to_python(dict.iter(), state)?),
            (ObType::PydanticSerializable, Value::Model(model)) => {
                let extra = model.extra.iter().flat_map(Dict::iter);
                Value::Dict(pairs_to_python(model.fields.iter().chain(extra), state)?)
            }
            _ => value.clone(),
        },
    };
    Ok(value)
}

/// Makes a value serializable by serde through inference.
pub(crate) struct SerializeInfer<'slf> {
    value: &'slf Value,
    state: RefCell<&'slf mut SerializationState>,
}

impl<'slf> SerializeInfer<'slf> {
    pub(crate) fn new(value: &'slf Value, state: &'slf mut SerializationState) -> Self {
        Self {
            value,
            state: RefCell::new(state),
        }
    }
}

impl Serialize for SerializeInfer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let state = &mut self.state.borrow_mut();
        infer_serialize_known(get_type(self.value), self.value, serializer, state)
    }
}

pub(crate) fn infer_serialize<S: Serializer>(
    value: &Value,
    serializer: S,
    state: &mut SerializationState,
) -> Result<S::Ok, S::Error> {
    infer_serialize_known(get_type(value), value, serializer, state)
}

pub(crate) fn infer_serialize_known<S: Serializer>(
    ob_type: ObType,
    value: &Value,
    serializer: S,
    state: &mut SerializationState,
) -> Result<S::Ok, S::Error> {
    let extra_serialize_unknown = state.extra.serialize_unknown;
    let mut guard = match state.recursion_guard(value, INFER_DEF_REF_ID) {
        Ok(v) => v,
        Err(e) => {
            return if extra_serialize_unknown {
                serializer.serialize_str("...")
            } else {
                Err(py_err_se_err(&e))
            };
        }
    };
    let state = guard.state();

    match (ob_type, value) {
        (ObType::Float, Value::Float(v)) => {
            serialize_f64(*v, serializer, state.config.inf_nan_mode)
        }
        (ObType::Bytes, Value::Bytes(b)) => state.config.bytes_mode.serialize_bytes(b, serializer),
        (ObType::Dict, Value::Dict(dict)) => serialize_pairs(dict.iter(), serializer, state),
        (ObType::PydanticSerializable, Value::Model(model)) => {
            let extra = model.extra.iter().flat_map(Dict::iter);
            serialize_pairs(model.fields.iter().chain(extra), serializer, state)
        }
        (ObType::List, Value::List(items)) | (ObType::Tuple, Value::Tuple(items)) => {
            let len = items.len();
            let mut seq = serializer.serialize_seq(Some(len))?;
            let filter = AnyFilter::new();
            for (index, element) in items.iter().enumerate() {
                let op_next = filter
                    .index_filter(index, state, Some(len))
                    .map_err(|e| py_err_se_err(&e))?;
                if let Some(next_include_exclude) = op_next {
                    let state = &mut state.scoped_include_exclude(next_include_exclude);
                    let item_serializer = SerializeInfer::new(element, state);
                    seq.serialize_element(&item_serializer)?;
                }
            }
            seq.end()
        }
        (ObType::Set, Value::Set(items)) => {
            let state = &mut state.scoped_include_exclude(IncludeExclude::empty());
            let mut seq = serializer.serialize_seq(Some(items.len()))?;
            for element in items {
                let item_serializer = SerializeInfer::new(element, state);
                seq.serialize_element(&item_serializer)?;
            }
            seq.end()
        }
        (ObType::Datetime | ObType::Date | ObType::Time | ObType::Timedelta, _) => {
            match state.config.temporal_mode.to_json(value) {
                Some(Value::Float(f)) => serializer.serialize_f64(f),
                Some(json) => json.serialize(serializer),
                None => value.serialize(serializer),
            }
        }
        // None, bool, int and str serialize as JSON does
        _ => value.serialize(serializer),
    }
}

pub(crate) fn infer_json_key<'a>(
    key: &'a Value,
    state: &mut SerializationState,
) -> SerResult<Cow<'a, str>> {
    infer_json_key_known(get_type(key), key, state)
}

pub(crate) fn infer_json_key_known<'a>(
    ob_type: ObType,
    key: &'a Value,
    state: &mut SerializationState,
) -> SerResult<Cow<'a, str>> {
    match (ob_type, key) {
        (ObType::None, _) => Ok(Cow::Borrowed("None")),
        (ObType::Int | ObType::Uuid | ObType::Url | ObType::MultiHostUrl, _) => {
            Ok(Cow::Owned(key.py_str()))
        }
        (ObType::Float, Value::Float(v)) => {
            if (v.is_nan() || v.is_infinite()) && state.config.inf_nan_mode == InfNanMode::Null {
                Ok(Cow::Borrowed("None"))
            } else {
                Ok(Cow::Owned(key.py_str()))
            }
        }
        (ObType::Bool, Value::Bool(b)) => Ok(Cow::Borrowed(if *b { "true" } else { "false" })),
        (ObType::Str, Value::Str(s)) => Ok(Cow::Borrowed(s)),
        (ObType::Bytes, Value::Bytes(b)) => Ok(state.config.bytes_mode.bytes_to_string(b)?),
        (ObType::Datetime | ObType::Date | ObType::Time | ObType::Timedelta, _) => Ok(Cow::Owned(
            state
                .config
                .temporal_mode
                .json_key(key)
                .unwrap_or_else(|| key.py_str()),
        )),
        (ObType::Tuple, Value::Tuple(items)) => {
            let mut key_build = KeyBuilder::new();
            for element in items {
                key_build.push(&infer_json_key(element, state)?);
            }
            Ok(Cow::Owned(key_build.finish()))
        }
        // model instances are unhashable, like pydantic models that are not frozen
        (ObType::PydanticSerializable, _) => Err(SerializeError::Core(CoreError::Type(format!(
            "unhashable type: '{}'",
            key.type_name()
        )))),
        _ => Err(SerializeError::Core(CoreError::Type(format!(
            "`{ob_type}` not valid as object key"
        )))),
    }
}

/// Joins the keys of a tuple key with commas.
pub(crate) struct KeyBuilder {
    key: String,
    first: bool,
}

impl KeyBuilder {
    pub fn new() -> Self {
        Self {
            key: String::with_capacity(31),
            first: true,
        }
    }

    pub fn push(&mut self, key: &str) {
        if self.first {
            self.first = false;
        } else {
            self.key.push(',');
        }
        self.key.push_str(key);
    }

    pub fn finish(self) -> String {
        self.key
    }
}

/// Serialize key-value pairs by inference into a dict (JSON mode: string keys).
fn pairs_to_python<'a>(
    pairs: impl Iterator<Item = (&'a Value, &'a Value)>,
    state: &mut SerializationState,
) -> SerResult<Dict> {
    let mut out = Dict::new();
    let filter = AnyFilter::new();
    for (key, value) in pairs {
        if let Some(next_include_exclude) = filter.key_filter(key, state)? {
            let state = &mut state.scoped_include_exclude(next_include_exclude);
            if state.extra.mode.is_json() {
                let key = infer_json_key(key, state)?.into_owned();
                let value = infer_to_python(value, state)?;
                out.insert(Value::Str(key), value);
            } else {
                let key = infer_to_python(key, state)?;
                let value = infer_to_python(value, state)?;
                out.insert(key, value);
            }
        }
    }
    Ok(out)
}

/// Serialize key-value pairs by inference as a JSON object.
fn serialize_pairs<'a, S: Serializer>(
    pairs: impl Iterator<Item = (&'a Value, &'a Value)>,
    serializer: S,
    state: &mut SerializationState,
) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(None)?;
    let filter = AnyFilter::new();
    for (key, value) in pairs {
        let op_next = filter
            .key_filter(key, state)
            .map_err(|e| py_err_se_err(&e))?;
        if let Some(next_include_exclude) = op_next {
            let state = &mut state.scoped_include_exclude(next_include_exclude);
            let key = infer_json_key(key, state).map_err(|e| py_err_se_err(&e))?;
            let value_serializer = SerializeInfer::new(value, state);
            map.serialize_entry(&key, &value_serializer)?;
        }
    }
    map.end()
}
