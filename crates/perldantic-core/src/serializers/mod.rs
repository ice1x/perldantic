//! Serializers and the `SchemaSerializer` entry point. Port of upstream `serializers/mod.rs`.
//!
//! Serialization is driven by the same core schemas as validation: `to_python` produces a
//! `Value` (in `json` mode, one that only holds JSON types) and `to_json` produces JSON text.
//! Values that do not match their serializer are serialized by inference and reported as a
//! warning, as pydantic does.

use std::sync::Arc;

use crate::core_error::CoreResult;
use crate::definitions::{Definitions, DefinitionsBuilder};
use crate::validators::as_dict;
use crate::value::Value;

pub(crate) mod config;
mod errors;
mod extra;
mod fields;
mod filter;
mod infer;
mod ob_type;
mod ser;
mod shared;
mod type_serializers;

pub use errors::{SerializeError, UnexpectedValue};
pub use extra::{SerMode, WarningsMode};

use config::SerializationConfig;
use extra::{Extra, SerializationState};
use shared::{BuildSerializer, CombinedSerializer, to_json_bytes};

/// Options of a serialization call (upstream `to_python` / `to_json` keyword arguments).
///
/// Upstream's `round_trip`, `context`, `exclude_computed_fields` and `polymorphic_serialization`
/// only matter for serializer functions, computed fields and model subclasses, which need host
/// callbacks; they come with those.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct SerializeOptions {
    /// `to_python` only; `to_json` always serializes in `json` mode.
    pub mode: SerMode,
    pub include: Option<Value>,
    pub exclude: Option<Value>,
    pub by_alias: Option<bool>,
    pub exclude_unset: bool,
    pub exclude_defaults: bool,
    pub exclude_none: bool,
    pub warnings: WarningsMode,
    pub serialize_as_any: bool,
}

/// JSON formatting options of `to_json`.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonOptions {
    pub indent: Option<usize>,
    pub ensure_ascii: bool,
}

/// A serialization result and the warning raised along the way, if any: the text of the
/// `UserWarning` pydantic emits (it lists every unexpected value).
#[derive(Debug, Clone, PartialEq)]
pub struct Serialized<T> {
    pub output: T,
    pub warning: Option<String>,
}

/// A compiled core schema, ready to serialize values.
#[derive(Debug)]
pub struct SchemaSerializer {
    serializer: Arc<CombinedSerializer>,
    // Keeps shared (possibly recursive) definitions alive; serializers hold weak references.
    _definitions: Definitions<Arc<CombinedSerializer>>,
    config: SerializationConfig,
}

impl SchemaSerializer {
    /// Build a serializer from a core schema and optional core config (both dicts).
    pub fn new(schema: &Value, config: Option<&Value>) -> CoreResult<Self> {
        let schema = as_dict(schema)?;
        let config = match config {
            None | Some(Value::None) => None,
            Some(c) => Some(as_dict(c)?),
        };
        let mut definitions_builder = DefinitionsBuilder::new();
        let serializer = CombinedSerializer::build(schema, config, &mut definitions_builder)?;
        Ok(Self {
            serializer,
            _definitions: definitions_builder.finish()?,
            config: SerializationConfig::from_config(config)?,
        })
    }

    /// Schema types this build can serialize.
    pub fn supported_schema_types() -> &'static [&'static str] {
        shared::supported_serializer_types()
    }

    /// Serialize to host data (upstream `to_python`).
    pub fn to_python(
        &self,
        value: &Value,
        options: &SerializeOptions,
    ) -> Result<Serialized<Value>, SerializeError> {
        let mut state = self.state(options, options.mode.clone());
        let output = self.serializer.to_python(value, &mut state)?;
        let warning = state.warnings.final_check()?;
        Ok(Serialized { output, warning })
    }

    /// Serialize to JSON text (upstream `to_json`).
    pub fn to_json(
        &self,
        value: &Value,
        options: &SerializeOptions,
        json: &JsonOptions,
    ) -> Result<Serialized<String>, SerializeError> {
        let mut state = self.state(options, SerMode::Json);
        let bytes = to_json_bytes(
            value,
            &self.serializer,
            &mut state,
            json.indent,
            json.ensure_ascii,
        )?;
        let warning = state.warnings.final_check()?;
        // The writer only emits UTF-8.
        let output = String::from_utf8(bytes).expect("JSON output is UTF-8");
        Ok(Serialized { output, warning })
    }

    fn state(&self, options: &SerializeOptions, mode: SerMode) -> SerializationState {
        let extra = Extra {
            mode,
            by_alias: options.by_alias,
            exclude_unset: options.exclude_unset,
            exclude_defaults: options.exclude_defaults,
            exclude_none: options.exclude_none,
            serialize_unknown: false,
            serialize_as_any: options.serialize_as_any,
        };
        SerializationState::new(
            self.config,
            options.warnings,
            options.include.clone(),
            options.exclude.clone(),
            extra,
        )
    }
}
