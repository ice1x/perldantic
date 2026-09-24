//! JSON Schema generation from core schemas. Port of pydantic's `pydantic/json_schema.py`
//! (`GenerateJsonSchema`), which lives in pydantic's Python layer rather than in pydantic-core.
//!
//! The output is a JSON Schema (Draft 2020-12) as a `Value`, with pydantic's key order. As in
//! pydantic, configuration only comes from `model` schemas: pydantic reads the model class's
//! `model_config`, the port reads the model schema's `config` (docs/DIVERGENCES.md #15).
//! Class docstrings and the Python callables pydantic accepts (`json_schema_extra` functions,
//! `pydantic_js_functions`, `model_title_generator`) have no equivalent here.

mod refs;

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreError;
use crate::input::Input;
use crate::serializers::{SchemaSerializer, SerMode, SerializeError, SerializeOptions};
use crate::validators::as_dict;
use crate::value::{Dict, Value};

use refs::{DefinitionsRemapping, DefsRefs};

/// The default format string used to generate reference names.
pub const DEFAULT_REF_TEMPLATE: &str = "#/$defs/{model}";

const PRIMITIVE_JSON_SCHEMA_TYPES: [&str; 5] = ["string", "boolean", "null", "integer", "number"];
/// Core schema types describing fields rather than values.
const CORE_SCHEMA_FIELD_TYPES: [&str; 4] = [
    "typed-dict-field",
    "dataclass-field",
    "model-field",
    "computed-field",
];

/// Whether the JSON Schema describes the input of validation or the output of serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum JsonSchemaMode {
    #[default]
    Validation,
    Serialization,
}

impl JsonSchemaMode {
    fn title(self) -> &'static str {
        match self {
            Self::Validation => "Input",
            Self::Serialization => "Output",
        }
    }
}

/// How the schemas of union members are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnionFormat {
    /// The `anyOf` keyword.
    #[default]
    AnyOf,
    /// A `type` array of primitive types when every member is an unconstrained primitive type,
    /// `anyOf` otherwise.
    PrimitiveTypeArray,
}

/// Options of JSON Schema generation (upstream `GenerateJsonSchema` arguments and `mode`).
#[derive(Debug, Clone)]
pub struct JsonSchemaOptions {
    pub mode: JsonSchemaMode,
    /// Whether to use field aliases as property names.
    pub by_alias: bool,
    /// The format string of `$ref` values; `{model}` is replaced by the definition's name.
    pub ref_template: String,
    pub union_format: UnionFormat,
}

impl Default for JsonSchemaOptions {
    fn default() -> Self {
        Self {
            mode: JsonSchemaMode::Validation,
            by_alias: true,
            ref_template: DEFAULT_REF_TEMPLATE.to_owned(),
            union_format: UnionFormat::AnyOf,
        }
    }
}

/// A generated JSON Schema and the warnings raised along the way (upstream emits them as
/// `PydanticJsonSchemaWarning`s).
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedJsonSchema {
    pub schema: Value,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonSchemaError {
    /// Upstream `PydanticInvalidForJsonSchema`.
    InvalidForJsonSchema(String),
    /// Upstream `PydanticSerializationError`, from a value that could not be made JSON-able.
    Serialization(String),
    /// A malformed core schema, or one this port cannot describe yet.
    Core(CoreError),
}

impl JsonSchemaError {
    /// Name of the exception class pydantic raises.
    pub fn python_name(&self) -> &'static str {
        match self {
            Self::InvalidForJsonSchema(_) => "PydanticInvalidForJsonSchema",
            Self::Serialization(_) => "PydanticSerializationError",
            Self::Core(e) => e.kind().python_name(),
        }
    }
}

impl fmt::Display for JsonSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidForJsonSchema(message) | Self::Serialization(message) => {
                write!(f, "{message}")
            }
            Self::Core(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for JsonSchemaError {}

impl From<CoreError> for JsonSchemaError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<SerializeError> for JsonSchemaError {
    fn from(e: SerializeError) -> Self {
        match e {
            SerializeError::Core(e) => Self::Core(e),
            other => Self::Serialization(other.to_string()),
        }
    }
}

pub(crate) type JsResult<T> = Result<T, JsonSchemaError>;

/// Generate the JSON Schema of a core schema (upstream `GenerateJsonSchema().generate()`).
///
/// `config` applies to the whole schema, as the config of a pydantic `TypeAdapter` does; like a
/// `model` schema's config, it uses core config names (docs/DIVERGENCES.md #15).
pub fn generate_json_schema(
    schema: &Value,
    config: Option<&Value>,
    options: &JsonSchemaOptions,
) -> JsResult<GeneratedJsonSchema> {
    let config = match config {
        None | Some(Value::None) => Dict::new(),
        Some(config) => as_dict(config)?.clone(),
    };
    GenerateJsonSchema::new(options, config).generate(as_dict(schema)?)
}

type CoreModeRef = (String, JsonSchemaMode);

struct GenerateJsonSchema<'o> {
    options: &'o JsonSchemaOptions,
    mode: JsonSchemaMode,
    core_to_json_refs: HashMap<CoreModeRef, String>,
    core_to_defs_refs: HashMap<CoreModeRef, String>,
    json_to_defs_refs: HashMap<String, String>,
    definitions: Dict,
    defs_refs: DefsRefs,
    /// The configs in effect: the one for the whole schema, then those of the models being
    /// generated.
    config_stack: Vec<Dict>,
    /// Definitions that failed to build, with the error to raise if they end up being used.
    core_defs_invalid_for_json_schema: HashMap<String, String>,
    warnings: Vec<String>,
}

// Small constructors for JSON Schema dicts.

fn set(schema: &mut Dict, key: &str, value: impl Into<Value>) {
    schema.insert(Value::from(key), value.into());
}

fn has(schema: &Dict, key: &str) -> bool {
    schema.get_str(key).is_some()
}

fn typed(type_: &str) -> Dict {
    let mut schema = Dict::new();
    set(&mut schema, "type", type_);
    schema
}

fn ref_schema(json_ref: &str) -> Dict {
    let mut schema = Dict::new();
    set(&mut schema, "$ref", json_ref);
    schema
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::None | Value::Bool(false) | Value::Int(0)) => false,
        Some(Value::Float(f)) => *f != 0.0,
        Some(Value::Str(s)) => !s.is_empty(),
        Some(Value::List(items) | Value::Tuple(items) | Value::Set(items)) => !items.is_empty(),
        Some(Value::Dict(d)) => !d.is_empty(),
        Some(_) => true,
    }
}

fn is_http(json_ref: &str) -> bool {
    json_ref.starts_with("http://") || json_ref.starts_with("https://")
}

fn is_core_schema(schema: &Dict) -> bool {
    !matches!(schema.get_str("type"), Some(Value::Str(t)) if CORE_SCHEMA_FIELD_TYPES.contains(&t.as_str()))
}

fn sub_schema<'a>(schema: &'a Dict, key: &str) -> JsResult<&'a Dict> {
    match schema.get_str(key) {
        Some(value) => Ok(as_dict(value)?),
        None => Err(CoreError::Key(key.to_owned()).into()),
    }
}

/// Mappings from core schema constraint names to JSON Schema keywords.
mod validations {
    pub const NUMERIC: &[(&str, &str)] = &[
        ("multiple_of", "multipleOf"),
        ("le", "maximum"),
        ("ge", "minimum"),
        ("lt", "exclusiveMaximum"),
        ("gt", "exclusiveMinimum"),
    ];
    pub const BYTES: &[(&str, &str)] = &[("min_length", "minLength"), ("max_length", "maxLength")];
    pub const STRING: &[(&str, &str)] = &[
        ("min_length", "minLength"),
        ("max_length", "maxLength"),
        ("pattern", "pattern"),
    ];
    pub const ARRAY: &[(&str, &str)] = &[("min_length", "minItems"), ("max_length", "maxItems")];
    pub const OBJECT: &[(&str, &str)] = &[
        ("min_length", "minProperties"),
        ("max_length", "maxProperties"),
    ];
}

/// Copy the constraints of the core schema into the JSON Schema.
fn update_with_validations(json_schema: &mut Dict, core_schema: &Dict, mapping: &[(&str, &str)]) {
    for (core_key, json_schema_key) in mapping {
        if let Some(value) = core_schema.get_str(core_key) {
            set(json_schema, json_schema_key, value.clone());
        }
    }
}

/// Distinct schemas, by Python equality: each keeps its first position and last value.
fn deduplicate_schemas(schemas: impl IntoIterator<Item = Dict>) -> Vec<Dict> {
    let mut distinct: Vec<Dict> = Vec::new();
    for schema in schemas {
        match distinct.iter_mut().find(|d| d.py_eq(&schema)) {
            Some(existing) => *existing = schema,
            None => distinct.push(schema),
        }
    }
    distinct
}

/// Python's `str.title()`: the first cased character of each run is upper case, the rest lower.
fn python_title(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut previous_is_cased = false;
    for c in name.chars() {
        if previous_is_cased {
            out.extend(c.to_lowercase());
        } else {
            out.extend(c.to_uppercase());
        }
        previous_is_cased = c.is_lowercase() || c.is_uppercase();
    }
    out
}

/// Retrieves a title from a name.
fn get_title_from_name(name: &str) -> String {
    python_title(name).replace('_', " ").trim().to_owned()
}

/// Sort a value like Python's `sorted()` when its items are comparable; `None` otherwise.
fn sorted_items(items: &[Value]) -> Option<Vec<Value>> {
    let mut sorted = items.to_vec();
    let numeric = |v: &Value| match v {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        #[allow(clippy::cast_precision_loss)]
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    };
    if items.iter().all(|v| numeric(v).is_some()) {
        sorted.sort_by(|a, b| {
            numeric(a)
                .partial_cmp(&numeric(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else if items.iter().all(|v| matches!(v, Value::Str(_))) {
        sorted.sort_by_key(Value::py_str);
    } else {
        return None;
    }
    Some(sorted)
}

/// Alphabetically sort the keys of the JSON schema, recursively, skipping the `properties`
/// and `default` keys to preserve field definition order.
fn sort(value: Value, parent_key: Option<&str>) -> Value {
    match value {
        Value::Dict(dict) => {
            let mut entries: Vec<(Value, Value)> = dict.into_iter().collect();
            if !matches!(parent_key, Some("properties" | "default")) {
                entries.sort_by_key(|(k, _)| k.py_str());
            }
            Value::Dict(
                entries
                    .into_iter()
                    .map(|(k, v)| {
                        let v = match &k {
                            Value::Str(key) => sort(v, Some(key)),
                            _ => sort(v, None),
                        };
                        (k, v)
                    })
                    .collect(),
            )
        }
        Value::List(items) => Value::List(items.into_iter().map(|v| sort(v, parent_key)).collect()),
        other => other,
    }
}

/// All the definitions references of a JSON schema.
fn get_all_json_refs(schema: &Value) -> HashSet<String> {
    let mut refs = HashSet::new();
    let mut stack = vec![schema];
    while let Some(current) = stack.pop() {
        match current {
            Value::Dict(dict) => {
                for (key, value) in dict.iter() {
                    match (key, value) {
                        // Skip examples that may contain arbitrary values and references
                        (Value::Str(k), Value::List(_)) if k == "examples" => {}
                        (Value::Str(k), Value::Str(json_ref)) if k == "$ref" => {
                            refs.insert(json_ref.clone());
                        }
                        (_, Value::Dict(_) | Value::List(_)) => stack.push(value),
                        _ => {}
                    }
                }
            }
            Value::List(items) => stack.extend(items),
            _ => {}
        }
    }
    refs
}

/// Serialize a value to JSON-able data (upstream `to_jsonable_python`), with the serialization
/// config keys (`ser_json_bytes`, `ser_json_inf_nan`, `ser_json_temporal`,
/// `ser_json_timedelta`) in `ser_config`.
fn to_jsonable(value: &Value, ser_config: Dict, by_alias: bool) -> JsResult<Value> {
    let mut schema = Dict::new();
    set(&mut schema, "type", "any");
    let serializer = SchemaSerializer::new(&Value::Dict(schema), Some(&Value::Dict(ser_config)))?;
    let options = SerializeOptions {
        mode: SerMode::Json,
        by_alias: Some(by_alias),
        ..SerializeOptions::default()
    };
    Ok(serializer.to_python(value, &options)?.output)
}

/// `to_jsonable_python` with its defaults: bytes as UTF-8, infinities and NaN kept.
fn to_jsonable_python(value: &Value) -> JsResult<Value> {
    let mut config = Dict::new();
    set(&mut config, "ser_json_inf_nan", "constants");
    to_jsonable(value, config, true)
}

impl<'o> GenerateJsonSchema<'o> {
    fn new(options: &'o JsonSchemaOptions, config: Dict) -> Self {
        Self {
            options,
            mode: options.mode,
            core_to_json_refs: HashMap::new(),
            core_to_defs_refs: HashMap::new(),
            json_to_defs_refs: HashMap::new(),
            definitions: Dict::new(),
            defs_refs: DefsRefs::default(),
            config_stack: vec![config],
            core_defs_invalid_for_json_schema: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    fn config(&self) -> &Dict {
        self.config_stack
            .last()
            .expect("the stack has a bottom config")
    }

    fn mode(&self) -> JsonSchemaMode {
        match self.config().get_str("json_schema_mode_override") {
            Some(Value::Str(m)) if m == "validation" => JsonSchemaMode::Validation,
            Some(Value::Str(m)) if m == "serialization" => JsonSchemaMode::Serialization,
            _ => self.mode,
        }
    }

    fn json_ref(&self, defs_ref: &str) -> String {
        self.options.ref_template.replace("{model}", defs_ref)
    }

    fn generate(mut self, schema: &Dict) -> JsResult<GeneratedJsonSchema> {
        let mut json_schema = self.generate_inner(schema)?;
        let json_ref_counts = self.get_json_ref_counts(&Value::Dict(json_schema.clone()))?;

        // "Unpack" the ref if it is the only reference and there are no sibling keys; upstream
        // loops, but its loop always stops after the first level
        if let Some(Value::Str(json_ref)) = json_schema.get_str("$ref") {
            let json_ref = json_ref.clone();
            let ref_json_schema = self.get_schema_from_definitions(&json_ref)?;
            if json_ref_counts.get(&json_ref) == Some(&1)
                && json_schema.len() == 1
                && let Some(ref_json_schema) = ref_json_schema
            {
                json_schema = ref_json_schema;
            }
        }

        let mut json_schema = Value::Dict(json_schema);
        self.garbage_collect_definitions(&json_schema)?;
        let definitions_remapping = self.build_definitions_remapping()?;

        if let Value::Dict(dict) = &mut json_schema
            && !self.definitions.is_empty()
        {
            set(dict, "$defs", std::mem::take(&mut self.definitions));
        }

        definitions_remapping.remap_json_schema(&mut json_schema);

        Ok(GeneratedJsonSchema {
            schema: sort(json_schema, None),
            warnings: self.warnings,
        })
    }

    fn generate_inner(&mut self, schema: &Dict) -> JsResult<Dict> {
        // If a schema with the same core ref has been handled, just return a reference to it
        if let Some(core_ref) = schema.get_as::<String>("ref")? {
            let core_mode_ref = (core_ref, self.mode());
            if let Some(defs_ref) = self.core_to_defs_refs.get(&core_mode_ref)
                && self.definitions.get_str(defs_ref).is_some()
            {
                return Ok(ref_schema(&self.core_to_json_refs[&core_mode_ref]));
            }
        }

        let metadata = match schema.get_str("metadata") {
            Some(Value::Dict(metadata)) => Some(metadata),
            _ => None,
        };
        for key in ["pydantic_js_functions", "pydantic_js_annotation_functions"] {
            if truthy(metadata.and_then(|m| m.get_str(key))) {
                return Err(CoreError::Schema(format!(
                    "`{key}` are not supported yet: host callbacks are not implemented"
                ))
                .into());
            }
        }

        let mut json_schema = self.handler(schema)?;

        if let Some(Value::Dict(js_updates)) =
            metadata.and_then(|m| m.get_str("pydantic_js_updates"))
            && !js_updates.is_empty()
        {
            for (key, value) in js_updates.iter() {
                json_schema.insert(key.clone(), value.clone());
            }
        }
        match metadata.and_then(|m| m.get_str("pydantic_js_extra")) {
            None | Some(Value::None) => {}
            Some(Value::Dict(js_extra)) => {
                if !js_extra.is_empty()
                    && let Value::Dict(js_extra) =
                        to_jsonable_python(&Value::Dict(js_extra.clone()))?
                {
                    for (key, value) in js_extra {
                        json_schema.insert(key, value);
                    }
                }
            }
            Some(_) => {
                return Err(CoreError::Schema(
                    "a callable `pydantic_js_extra` is not supported yet: host callbacks are not implemented".to_owned(),
                )
                .into());
            }
        }

        if is_core_schema(schema) {
            json_schema = self.populate_defs(schema, json_schema)?;
        }
        Ok(json_schema)
    }

    fn populate_defs(&mut self, core_schema: &Dict, json_schema: Dict) -> JsResult<Dict> {
        let Some(core_ref) = core_schema.get_as::<String>("ref")? else {
            return Ok(json_schema);
        };
        let (defs_ref, ref_json_schema) = self.get_cache_defs_ref_schema(&core_ref);
        let json_ref = ref_json_schema.get_str("$ref");
        // Replace the schema if it's not a reference to itself; what we want to avoid is having
        // the def be just a ref to itself
        if json_schema.get_str("$ref") != json_ref {
            self.definitions
                .insert(Value::Str(defs_ref.clone()), Value::Dict(json_schema));
            self.core_defs_invalid_for_json_schema.remove(&defs_ref);
        }
        Ok(ref_json_schema)
    }

    /// Generate the core-schema-type-specific bits of the schema.
    fn handler(&mut self, schema: &Dict) -> JsResult<Dict> {
        let type_: String = schema.get_as_req("type")?;
        if self.mode() == JsonSchemaMode::Serialization
            && let Some(ser_schema) = schema.get_str("serialization")
        {
            // Use the `serialization` schema instead (canonical example:
            // `Annotated[int, PlainSerializer(str)]`)
            let ser_schema = as_dict(ser_schema)?;
            if let Some(json_schema) = self.ser_schema(ser_schema)? {
                // It might be that the `serialization` is skipped depending on `when_used`.
                // This is only relevant for `nullable` schemas though, so we special case here.
                let skips_none = matches!(
                    ser_schema.get_str("when_used"),
                    Some(Value::Str(w)) if w == "unless-none" || w == "json-unless-none"
                );
                if skips_none && type_ == "nullable" {
                    return Ok(self.get_union_of_schemas(vec![typed("null"), json_schema]));
                }
                return Ok(json_schema);
            }
        }

        match type_.as_str() {
            "any" => Ok(Dict::new()),
            "none" => Ok(typed("null")),
            "bool" => Ok(typed("boolean")),
            "int" => Ok(Self::numeric_schema(schema, "integer")),
            "float" => Ok(Self::numeric_schema(schema, "number")),
            "str" => Ok(self.str_schema(schema)),
            "bytes" => Ok(self.bytes_schema(schema)),
            "date" => Ok(self.common_temporal_schema("date", self.ser_json_temporal())),
            "time" => Ok(self.common_temporal_schema("time", self.ser_json_temporal())),
            "datetime" => Ok(self.common_temporal_schema("date-time", self.ser_json_temporal())),
            "timedelta" => Ok(self.timedelta_schema()),
            "decimal" => Ok(self.decimal_schema(schema)),
            "url" => Ok(Self::url_schema(schema, "uri")),
            "multi-host-url" => Ok(Self::url_schema(schema, "multi-host-uri")),
            "uuid" => {
                let mut json_schema = typed("string");
                set(&mut json_schema, "format", "uuid");
                Ok(json_schema)
            }
            "literal" => self.literal_schema(schema),
            "enum" => Self::enum_schema(schema),
            "list" => self.list_schema(schema),
            "tuple" => self.tuple_schema(schema),
            "set" | "frozenset" => self.set_schema(schema),
            "dict" => self.dict_schema(schema),
            "default" => self.default_schema(schema),
            "nullable" => self.nullable_schema(schema),
            "union" => self.union_schema(schema),
            "tagged-union" => self.tagged_union_schema(schema),
            "lax-or-strict" => self.lax_or_strict_schema(schema),
            "custom-error" | "model-field" | "typed-dict-field" => {
                self.generate_inner(sub_schema(schema, "schema")?)
            }
            "chain" => self.chain_schema(schema),
            "json" => self.json_schema(schema),
            "computed-field" => self.generate_inner(sub_schema(schema, "return_schema")?),
            "model" => self.model_schema(schema),
            "typed-dict" => self.typed_dict_schema(schema),
            "model-fields" => self.model_fields_schema(schema),
            "definitions" => self.definitions_schema(schema),
            "definition-ref" => {
                let core_ref: String = schema.get_as_req("schema_ref")?;
                Ok(self.get_cache_defs_ref_schema(&core_ref).1)
            }
            other => Err(CoreError::Schema(format!(
                "JSON Schema generation for `{other}` schemas is not supported yet"
            ))
            .into()),
        }
    }

    fn numeric_schema(schema: &Dict, type_: &str) -> Dict {
        let mut json_schema = typed(type_);
        update_with_validations(&mut json_schema, schema, validations::NUMERIC);
        json_schema
            .into_iter()
            .filter(|(_, v)| !matches!(v, Value::Float(f) if f.is_infinite()))
            .collect()
    }

    fn str_schema(&self, schema: &Dict) -> Dict {
        let mut json_schema = typed("string");
        update_with_validations(&mut json_schema, schema, validations::STRING);
        if !has(&json_schema, "minLength")
            && let Some(min_length) = self.config().get_str("str_min_length")
            && truthy(Some(min_length))
        {
            set(&mut json_schema, "minLength", min_length.clone());
        }
        if !has(&json_schema, "maxLength")
            && let Some(max_length) = self.config().get_str("str_max_length")
            && !matches!(max_length, Value::None)
        {
            set(&mut json_schema, "maxLength", max_length.clone());
        }
        json_schema
    }

    fn bytes_schema(&self, schema: &Dict) -> Dict {
        let base64 =
            matches!(self.config().get_str("ser_json_bytes"), Some(Value::Str(m)) if m == "base64");
        let mut json_schema = typed("string");
        set(
            &mut json_schema,
            "format",
            if base64 { "base64url" } else { "binary" },
        );
        update_with_validations(&mut json_schema, schema, validations::BYTES);
        json_schema
    }

    fn config_str(&self, key: &str) -> Option<&str> {
        match self.config().get_str(key) {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }

    fn ser_json_temporal(&self) -> &str {
        self.config_str("ser_json_temporal").unwrap_or("iso8601")
    }

    /// The first step's schema for validation, the last one's for serialization.
    fn chain_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let steps: Vec<Value> = schema.get_as_req("steps")?;
        let step = match self.mode() {
            JsonSchemaMode::Validation => steps.first(),
            JsonSchemaMode::Serialization => steps.last(),
        };
        match step {
            Some(step) => self.generate_inner(as_dict(step)?),
            None => Err(CoreError::Schema("`chain` schemas need steps".to_owned()).into()),
        }
    }

    /// JSON text holding the inner schema's data (validation), or that data (serialization).
    fn json_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let content = match schema.get_str("schema") {
            Some(inner) => self.generate_inner(as_dict(inner)?)?,
            None => Dict::new(),
        };
        if self.mode() == JsonSchemaMode::Serialization {
            return Ok(content);
        }
        let mut json_schema = typed("string");
        set(&mut json_schema, "contentMediaType", "application/json");
        set(&mut json_schema, "contentSchema", content);
        Ok(json_schema)
    }

    /// A string, or in validation mode also a number with the bounds as floats.
    fn decimal_schema(&self, schema: &Dict) -> Dict {
        let json_schema = self.str_schema(&Dict::new());
        if self.mode() != JsonSchemaMode::Validation {
            return json_schema;
        }
        let mut float_core = Dict::new();
        for key in ["multiple_of", "le", "ge", "lt", "gt"] {
            if let Some(bound) = schema.get_str(key)
                && let Ok(decimal) = bound.validate_decimal(false)
            {
                set(&mut float_core, key, decimal.into_inner().to_f64());
            }
        }
        let float_schema = Self::numeric_schema(&float_core, "number");
        let mut any_of = Dict::new();
        set(
            &mut any_of,
            "anyOf",
            Value::List(vec![Value::Dict(float_schema), Value::Dict(json_schema)]),
        );
        any_of
    }

    /// `url_schema` and `multi_host_url_schema`; `multi-host-uri` is a pydantic-specific format.
    fn url_schema(schema: &Dict, format: &str) -> Dict {
        let mut json_schema = typed("string");
        set(&mut json_schema, "format", format);
        set(&mut json_schema, "minLength", 1_i64);
        update_with_validations(&mut json_schema, schema, validations::STRING);
        json_schema
    }

    fn timedelta_schema(&self) -> Dict {
        // `ser_json_temporal` supersedes `ser_json_timedelta`, which only applies when the
        // former isn't explicitly set.
        let temporal_format = match self.config_str("ser_json_temporal") {
            Some(format) => format,
            None if self.config_str("ser_json_timedelta") == Some("float") => "seconds",
            None => "iso8601",
        };
        self.common_temporal_schema("duration", temporal_format)
    }

    fn common_temporal_schema(&self, format: &str, temporal_format: &str) -> Dict {
        if self.mode() == JsonSchemaMode::Serialization && temporal_format != "iso8601" {
            // Both `'seconds'` and `'milliseconds'` serialize to a number:
            return typed("number");
        }
        let mut json_schema = typed("string");
        set(&mut json_schema, "format", format);
        json_schema
    }

    fn literal_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let expected: Vec<Value> = schema.get_as_req("expected")?;
        let expected = expected
            .iter()
            .map(to_jsonable_python)
            .collect::<JsResult<Vec<Value>>>()?;

        let mut result = Dict::new();
        let type_of = |v: &Value| match v {
            Value::Str(_) => Some("string"),
            Value::Int(_) | Value::BigInt(_) => Some("integer"),
            Value::Float(_) => Some("number"),
            Value::Bool(_) => Some("boolean"),
            Value::List(_) => Some("array"),
            Value::None => Some("null"),
            _ => None,
        };
        let types: HashSet<Option<&str>> = expected.iter().map(type_of).collect();
        if let [single] = expected.as_slice() {
            set(&mut result, "const", single.clone());
        } else {
            set(&mut result, "enum", Value::List(expected.clone()));
        }
        if types.len() == 1
            && let Some(Some(type_)) = types.into_iter().next()
        {
            set(&mut result, "type", type_);
        }
        Ok(result)
    }

    /// The class name is the title; the docstring comes through `metadata.pydantic_js_updates`
    /// (docs/DIVERGENCES.md #15).
    fn enum_schema(schema: &Dict) -> JsResult<Dict> {
        let class: String = schema.get_as_req("cls")?;
        let members: Vec<Value> = schema.get_as_req("members")?;
        let expected = members
            .iter()
            .map(|member| match member {
                Value::Enum(member) => to_jsonable_python(&member.value),
                other => to_jsonable_python(other),
            })
            .collect::<JsResult<Vec<Value>>>()?;

        let mut result = Dict::new();
        set(&mut result, "title", class);
        let type_of = |v: &Value| match v {
            Value::Str(_) => Some("string"),
            Value::Int(_) | Value::BigInt(_) => Some("integer"),
            Value::Float(_) => Some("number"),
            Value::Bool(_) => Some("boolean"),
            Value::List(_) => Some("array"),
            _ => None,
        };
        let types: HashSet<Option<&str>> = expected.iter().map(type_of).collect();
        set(&mut result, "enum", Value::List(expected));
        if types.len() == 1
            && let Some(Some(type_)) = types.into_iter().next()
        {
            set(&mut result, "type", type_);
        }
        Ok(result)
    }

    fn items_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        match schema.get_str("items_schema") {
            Some(items) => self.generate_inner(as_dict(items)?),
            None => Ok(Dict::new()),
        }
    }

    fn list_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let items_schema = self.items_schema(schema)?;
        let mut json_schema = typed("array");
        set(&mut json_schema, "items", items_schema);
        update_with_validations(&mut json_schema, schema, validations::ARRAY);
        Ok(json_schema)
    }

    fn tuple_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let mut json_schema = typed("array");
        let items: Vec<Value> = schema.get_as_req("items_schema")?;
        let generate_all = |this: &mut Self, items: &[Value]| -> JsResult<Vec<Value>> {
            items
                .iter()
                .map(|item| Ok(Value::Dict(this.generate_inner(as_dict(item)?)?)))
                .collect()
        };
        if let Some(variadic_item_index) = schema.get_as::<usize>("variadic_item_index")? {
            if variadic_item_index > 0 {
                set(&mut json_schema, "minItems", variadic_item_index);
                let prefix_items =
                    generate_all(self, &items[..variadic_item_index.min(items.len())])?;
                set(&mut json_schema, "prefixItems", Value::List(prefix_items));
            }
            if variadic_item_index + 1 == items.len() {
                // if the variadic item is the last item, then represent it faithfully
                let variadic = self.generate_inner(as_dict(&items[variadic_item_index])?)?;
                set(&mut json_schema, "items", variadic);
            } else {
                // otherwise, 'items' represents the schema for the variadic item plus the suffix,
                // so just allow anything for simplicity for now
                set(&mut json_schema, "items", true);
            }
        } else {
            let prefix_items = generate_all(self, &items)?;
            let count = prefix_items.len();
            if !prefix_items.is_empty() {
                set(&mut json_schema, "prefixItems", Value::List(prefix_items));
            }
            set(&mut json_schema, "minItems", count);
            set(&mut json_schema, "maxItems", count);
        }
        update_with_validations(&mut json_schema, schema, validations::ARRAY);
        Ok(json_schema)
    }

    fn set_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let items_schema = self.items_schema(schema)?;
        let mut json_schema = typed("array");
        set(&mut json_schema, "uniqueItems", true);
        set(&mut json_schema, "items", items_schema);
        update_with_validations(&mut json_schema, schema, validations::ARRAY);
        Ok(json_schema)
    }

    fn dict_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let mut json_schema = typed("object");

        let mut keys_schema = match schema.get_str("keys_schema") {
            Some(keys) => self.generate_inner(as_dict(keys)?)?,
            None => Dict::new(),
        };
        let keys_pattern = if has(&keys_schema, "$ref") {
            // If the keys schema is a definition reference, it can't be a simple string core
            // schema (and thus no pattern can exist), in practice
            None
        } else {
            let pattern = keys_schema.remove_str("pattern");
            // Don't give a title to patternProperties/propertyNames:
            keys_schema.remove_str("title");
            pattern
        };

        let mut values_schema = match schema.get_str("values_schema") {
            Some(values) => self.generate_inner(as_dict(values)?)?,
            None => Dict::new(),
        };
        // don't give a title to additionalProperties:
        values_schema.remove_str("title");

        if !values_schema.is_empty() || keys_pattern.is_some() {
            match keys_pattern {
                None => set(&mut json_schema, "additionalProperties", values_schema),
                Some(pattern) => {
                    let mut pattern_properties = Dict::new();
                    pattern_properties.insert(pattern, Value::Dict(values_schema));
                    set(&mut json_schema, "patternProperties", pattern_properties);
                }
            }
        } else {
            // for `dict[str, Any]`, we allow any key and any value, since `str` is the default
            // key type
            set(&mut json_schema, "additionalProperties", true);
        }

        let constrained_string_keys = matches!(keys_schema.get_str("type"), Some(Value::Str(t)) if t == "string")
            && keys_schema.len() > 1;
        if constrained_string_keys || has(&keys_schema, "$ref") {
            keys_schema.remove_str("type");
            set(&mut json_schema, "propertyNames", keys_schema);
        }

        update_with_validations(&mut json_schema, schema, validations::OBJECT);
        Ok(json_schema)
    }

    fn default_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let mut json_schema = self.generate_inner(sub_schema(schema, "schema")?)?;

        let Some(default) = schema.get_str("default") else {
            return Ok(json_schema);
        };
        // Upstream applies plain serializer functions to defaults in serialization mode; those
        // need host callbacks, which core schemas here cannot hold.

        // Sort set defaults to ensure deterministic JSON schema generation
        let default = match default {
            Value::Set(items) if items.len() > 1 => {
                sorted_items(items).map_or_else(|| default.clone(), Value::List)
            }
            other => other.clone(),
        };

        match self.encode_default(&default) {
            Ok(encoded) => set(&mut json_schema, "default", encoded),
            Err(JsonSchemaError::Core(e)) if e.kind() == crate::CoreErrorKind::Schema => {
                return Err(e.into());
            }
            Err(_) => self.emit_warning(
                "non-serializable-default",
                &format!(
                    "Default value {} is not JSON serializable; excluding default from JSON schema",
                    default.py_str()
                ),
            ),
        }
        Ok(json_schema)
    }

    /// Encode a default value to JSON-able data. Upstream dumps it with a `TypeAdapter` of its
    /// type and the model config: a float keeps infinities and NaN, other values follow the
    /// config's bytes and inf/nan modes.
    fn encode_default(&self, default: &Value) -> JsResult<Value> {
        if let Value::Float(_) = default {
            return Ok(default.clone());
        }
        let ser_config: Dict = self
            .config()
            .iter()
            .filter(|(k, _)| {
                matches!(k, Value::Str(k) if matches!(
                    k.as_str(),
                    "ser_json_bytes" | "ser_json_inf_nan" | "ser_json_temporal" | "ser_json_timedelta"
                ))
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        to_jsonable(default, ser_config, self.options.by_alias)
    }

    fn nullable_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let null_schema = typed("null");
        let inner_json_schema = self.generate_inner(sub_schema(schema, "schema")?)?;
        if inner_json_schema == null_schema {
            Ok(null_schema)
        } else {
            Ok(self.get_union_of_schemas(vec![inner_json_schema, null_schema]))
        }
    }

    fn union_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let choices: Vec<Value> = schema.get_as_req("choices")?;
        let mut generated = Vec::new();
        for choice in &choices {
            // a choice is a schema, or a (schema, label) pair
            let choice = match choice {
                Value::Tuple(pair) | Value::List(pair) if !pair.is_empty() => &pair[0],
                other => other,
            };
            match self.generate_inner(as_dict(choice)?) {
                Ok(json_schema) => generated.push(json_schema),
                Err(JsonSchemaError::InvalidForJsonSchema(message)) => {
                    self.emit_warning("skipped-choice", &message);
                }
                Err(e) => return Err(e),
            }
        }
        if generated.len() == 1 {
            return Ok(generated.remove(0));
        }
        Ok(self.get_union_of_schemas(generated))
    }

    /// The JSON Schema of the union of the given schemas, in the configured union format.
    fn get_union_of_schemas(&self, schemas: Vec<Dict>) -> Dict {
        if self.options.union_format == UnionFormat::PrimitiveTypeArray
            && let Some(types) = Self::primitive_types(&schemas)
        {
            let mut distinct: Vec<Value> = Vec::new();
            for t in types {
                if !distinct.contains(&t) {
                    distinct.push(t);
                }
            }
            let mut json_schema = Dict::new();
            set(&mut json_schema, "type", Value::List(distinct));
            return json_schema;
        }
        Self::get_flattened_anyof(schemas)
    }

    /// The types of the schemas when all are unconstrained primitive types.
    fn primitive_types(schemas: &[Dict]) -> Option<Vec<Value>> {
        let mut types = Vec::new();
        for schema in schemas {
            // No type means it can be a ref or an empty schema.
            let schema_types = match schema.get_str("type")? {
                Value::List(types) => types.clone(),
                single => vec![single.clone()],
            };
            let all_primitive = schema_types.iter().all(
                |t| matches!(t, Value::Str(t) if PRIMITIVE_JSON_SCHEMA_TYPES.contains(&t.as_str())),
            );
            // Types with constraints or metadata are kept as they are.
            if !all_primitive || schema.len() != 1 {
                return None;
            }
            types.extend(schema_types);
        }
        Some(types)
    }

    fn get_flattened_anyof(schemas: Vec<Dict>) -> Dict {
        let mut members = Vec::new();
        for mut schema in schemas {
            if schema.len() == 1
                && let Some(Value::List(any_of)) = schema.remove_str("anyOf")
            {
                members.extend(any_of.into_iter().filter_map(|m| match m {
                    Value::Dict(d) => Some(d),
                    _ => None,
                }));
            } else {
                members.push(schema);
            }
        }
        let mut members = deduplicate_schemas(members);
        if members.len() == 1 {
            return members.remove(0);
        }
        let mut json_schema = Dict::new();
        set(
            &mut json_schema,
            "anyOf",
            Value::List(members.into_iter().map(Value::Dict).collect()),
        );
        json_schema
    }

    fn tagged_union_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let choices: Dict = schema.get_as_req("choices")?;
        let mut generated = Dict::new();
        for (k, v) in choices.iter() {
            // Use the JSON representation so that the discriminator mapping can be matched
            // against the serialized payload value; keys must be strings for JSON
            let k = match k {
                Value::Enum(member) => &member.value,
                other => other,
            };
            let key = match k {
                Value::Bool(true) => "true".to_owned(),
                Value::Bool(false) => "false".to_owned(),
                Value::None => "null".to_owned(),
                other => other.py_str(),
            };
            match self.generate_inner(as_dict(v)?) {
                Ok(json_schema) => generated.insert(Value::Str(key), Value::Dict(json_schema)),
                Err(JsonSchemaError::InvalidForJsonSchema(message)) => {
                    self.emit_warning("skipped-choice", &message);
                }
                Err(e) => return Err(e),
            }
        }

        let one_of_choices = deduplicate_schemas(generated.iter().filter_map(|(_, v)| match v {
            Value::Dict(d) => Some(d.clone()),
            _ => None,
        }));
        let mut json_schema = Dict::new();
        set(
            &mut json_schema,
            "oneOf",
            Value::List(one_of_choices.iter().cloned().map(Value::Dict).collect()),
        );

        // This reflects the v1 behavior
        if let Some(discriminator) = self.extract_discriminator(schema, &one_of_choices)? {
            let mapping: Dict = generated
                .iter()
                .map(|(k, v)| {
                    let target = match v {
                        Value::Dict(d) => d.get_str("$ref").cloned().unwrap_or_else(|| v.clone()),
                        other => other.clone(),
                    };
                    (k.clone(), target)
                })
                .collect();
            let mut openapi = Dict::new();
            set(&mut openapi, "propertyName", discriminator);
            set(&mut openapi, "mapping", mapping);
            set(&mut json_schema, "discriminator", openapi);
        }
        Ok(json_schema)
    }

    /// A compatible OpenAPI discriminator from the schema and the `oneOf` choices.
    fn extract_discriminator(
        &mut self,
        schema: &Dict,
        one_of_choices: &[Dict],
    ) -> JsResult<Option<String>> {
        let paths = match schema.get_str("discriminator") {
            Some(Value::Str(discriminator)) => return Ok(Some(discriminator.clone())),
            Some(Value::List(paths)) => paths,
            _ => return Ok(None),
        };
        // A single item list containing a string is equivalent to the string case
        if let [Value::Str(discriminator)] = paths.as_slice() {
            return Ok(Some(discriminator.clone()));
        }
        // When an alias is used that is different from the field name, the discriminator will
        // be a list of single str lists, one for the attribute and one for the actual alias.
        // Look for whether a single alias choice is present as a documented property on all
        // choices.
        for alias_path in paths {
            let Value::List(alias_path) = alias_path else {
                break; // the discriminator is not a list of alias paths
            };
            let [Value::Str(alias)] = alias_path.as_slice() else {
                continue; // the "alias" does not represent a single field
            };
            let mut alias_is_present_on_all_choices = true;
            for choice in one_of_choices {
                let choice = match self.resolve_ref_schema(choice.clone())? {
                    Ok(choice) => choice,
                    Err(message) => {
                        self.emit_warning("skipped-discriminator", &message);
                        Dict::new()
                    }
                };
                let present = matches!(
                    choice.get_str("properties"),
                    Some(Value::Dict(properties)) if properties.get_str(alias).is_some()
                );
                if !present {
                    alias_is_present_on_all_choices = false;
                    break;
                }
            }
            if alias_is_present_on_all_choices {
                return Ok(Some(alias.clone()));
            }
        }
        Ok(None)
    }

    /// Resolve a `$ref` schema to the schema it references; the inner error is upstream's
    /// `RuntimeError` for a reference whose definition does not exist yet.
    fn resolve_ref_schema(&self, mut json_schema: Dict) -> JsResult<Result<Dict, String>> {
        while let Some(Value::Str(json_ref)) = json_schema.get_str("$ref") {
            let json_ref = json_ref.clone();
            match self.get_schema_from_definitions(&json_ref)? {
                Some(schema) => json_schema = schema,
                None => {
                    return Ok(Err(format!(
                        "Cannot update undefined schema for $ref={json_ref}"
                    )));
                }
            }
        }
        Ok(Ok(json_schema))
    }

    fn lax_or_strict_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let key = if truthy(schema.get_str("strict")) {
            "strict_schema"
        } else {
            "lax_schema"
        };
        self.generate_inner(sub_schema(schema, key)?)
    }

    fn model_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let class: String = schema.get_as_req("cls")?;
        let config = match schema.get_str("config") {
            Some(Value::Dict(config)) => config.clone(),
            _ => Dict::new(),
        };
        let inner = sub_schema(schema, "schema")?;

        self.config_stack.push(config.clone());
        let json_schema = self.generate_inner(inner);
        self.config_stack.pop();
        let mut json_schema = json_schema?;

        Self::update_class_schema(&mut json_schema, &class, &config)?;
        Ok(json_schema)
    }

    /// Update the model's JSON schema with the title, additional properties and
    /// `json_schema_extra` of its config.
    fn update_class_schema(json_schema: &mut Dict, class: &str, config: &Dict) -> JsResult<()> {
        if let Some(title) = config.get_str("title")
            && !matches!(title, Value::None)
            && !has(json_schema, "title")
        {
            set(json_schema, "title", title.clone());
        }
        if !has(json_schema, "title") {
            set(json_schema, "title", class);
        }

        if !has(json_schema, "additionalProperties") {
            match config.get_str("extra_fields_behavior") {
                Some(Value::Str(extra)) if extra == "allow" => {
                    set(json_schema, "additionalProperties", true);
                }
                Some(Value::Str(extra)) if extra == "forbid" => {
                    set(json_schema, "additionalProperties", false);
                }
                _ => {}
            }
        }

        match config.get_str("json_schema_extra") {
            None | Some(Value::None) => {}
            Some(Value::Dict(extra)) => {
                for (key, value) in extra.iter() {
                    json_schema.insert(key.clone(), value.clone());
                }
            }
            Some(other) => {
                return Err(CoreError::Value(format!(
                    "model_config['json_schema_extra']={} should be a dict, callable, or None",
                    other.py_str()
                ))
                .into());
            }
        }
        Ok(())
    }

    /// Port of `GenerateJsonSchema.typed_dict_schema`; the config is the schema's own
    /// (docs/DIVERGENCES.md #15) and `cls` a class name.
    fn typed_dict_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let total = schema.get_as("total")?.unwrap_or(true);
        let fields: Dict = schema.get_as_req("fields")?;
        let mut named_required_fields = Vec::new();
        for (name, field) in fields.iter() {
            let field = as_dict(field)?;
            if self.field_is_present(field) {
                named_required_fields.push((
                    name.py_str(),
                    self.field_is_required(field, total)?,
                    field.clone(),
                ));
            }
        }
        let config = match schema.get_str("config") {
            Some(Value::Dict(config)) => config.clone(),
            _ => Dict::new(),
        };
        self.config_stack.push(config.clone());
        let json_schema = self.named_required_fields_schema(named_required_fields);
        self.config_stack.pop();
        let mut json_schema = json_schema?;

        let allow_additional_props = match schema.get_str("extras_schema") {
            Some(Value::Dict(extras))
                if extras.get_str("type") != Some(&Value::from("any")) || extras.len() != 1 =>
            {
                Value::Dict(self.generate_inner(extras)?)
            }
            _ => Value::Bool(true),
        };
        match schema.get_str("extra_behavior") {
            Some(Value::Str(extra)) if extra == "forbid" => {
                set(&mut json_schema, "additionalProperties", false);
            }
            Some(Value::Str(extra)) if extra == "allow" => {
                set(
                    &mut json_schema,
                    "additionalProperties",
                    allow_additional_props.clone(),
                );
            }
            _ => {}
        }

        match schema.get_str("cls") {
            Some(Value::Str(class)) => Self::update_class_schema(&mut json_schema, class, &config)?,
            _ if !has(&json_schema, "additionalProperties") => {
                match config.get_str("extra_fields_behavior") {
                    Some(Value::Str(extra)) if extra == "forbid" => {
                        set(&mut json_schema, "additionalProperties", false);
                    }
                    Some(Value::Str(extra)) if extra == "allow" => {
                        set(
                            &mut json_schema,
                            "additionalProperties",
                            allow_additional_props,
                        );
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(json_schema)
    }

    fn model_fields_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let fields: Dict = schema.get_as_req("fields")?;
        let mut named_required_fields = Vec::new();
        for (name, field) in fields.iter() {
            let field = as_dict(field)?;
            if self.field_is_present(field) {
                named_required_fields.push((
                    name.py_str(),
                    self.field_is_required(field, true)?,
                    field.clone(),
                ));
            }
        }
        if self.mode() == JsonSchemaMode::Serialization {
            let computed_fields: Vec<Value> = schema.get_as("computed_fields")?.unwrap_or_default();
            for field in &computed_fields {
                let field = as_dict(field)?;
                let name: String = field.get_as_req("property_name")?;
                let required = matches!(
                    field.get_str("serialization_exclude_if"),
                    None | Some(Value::None)
                );
                named_required_fields.push((name, required, field.clone()));
            }
        }
        let mut json_schema = self.named_required_fields_schema(named_required_fields)?;
        if let Some(extras_schema) = schema.get_str("extras_schema")
            && !matches!(extras_schema, Value::None)
        {
            let extras = self.generate_inner(as_dict(extras_schema)?)?;
            set(&mut json_schema, "additionalProperties", extras);
        }
        Ok(json_schema)
    }

    fn named_required_fields_schema(
        &mut self,
        named_required_fields: Vec<(String, bool, Dict)>,
    ) -> JsResult<Dict> {
        let mut properties = Dict::new();
        let mut required_fields = Vec::new();
        for (name, required, field) in named_required_fields {
            let name = if self.options.by_alias {
                self.get_alias_name(&field, name)
            } else {
                name
            };
            let mut field_json_schema = self.generate_inner(&field)?;
            if !has(&field_json_schema, "title") && Self::field_title_should_be_set(&field)? {
                set(&mut field_json_schema, "title", get_title_from_name(&name));
            }
            let field_json_schema = self.handle_ref_overrides(field_json_schema)?;
            properties.insert(Value::Str(name.clone()), Value::Dict(field_json_schema));
            if required {
                required_fields.push(Value::Str(name));
            }
        }

        let mut json_schema = typed("object");
        set(&mut json_schema, "properties", properties);
        if !required_fields.is_empty() {
            set(&mut json_schema, "required", Value::List(required_fields));
        }
        Ok(json_schema)
    }

    fn get_alias_name(&self, field: &Dict, name: String) -> String {
        let alias = if matches!(field.get_str("type"), Some(Value::Str(t)) if t == "computed-field")
        {
            field.get_str("alias")
        } else if self.mode() == JsonSchemaMode::Validation {
            let validate_by_alias = !matches!(
                self.config().get_str("validate_by_alias"),
                Some(Value::Bool(false))
            );
            if validate_by_alias {
                field.get_str("validation_alias")
            } else {
                None
            }
        } else {
            field.get_str("serialization_alias")
        };
        match alias {
            Some(Value::Str(alias)) => alias.clone(),
            Some(Value::List(paths)) => paths
                .iter()
                .find_map(|path| match path {
                    // Use the first valid single-item string path
                    Value::List(path) => match path.as_slice() {
                        [Value::Str(alias)] => Some(alias.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .unwrap_or(name),
            _ => name,
        }
    }

    /// Whether the field should be included in the generated JSON schema.
    fn field_is_present(&self, field: &Dict) -> bool {
        match self.mode() {
            JsonSchemaMode::Serialization => !truthy(field.get_str("serialization_exclude")),
            JsonSchemaMode::Validation => true,
        }
    }

    /// Whether the field should be marked as required in the generated JSON schema.
    fn field_is_required(&self, field: &Dict, total: bool) -> JsResult<bool> {
        let required = if matches!(field.get_str("type"), Some(Value::Str(t)) if t == "typed-dict-field")
        {
            field.get_as::<bool>("required")?.unwrap_or(total)
        } else {
            !matches!(sub_schema(field, "schema")?.get_str("type"), Some(Value::Str(t)) if t == "default")
        };
        if self.mode() == JsonSchemaMode::Serialization {
            let has_exclude_if = !matches!(
                field.get_str("serialization_exclude_if"),
                None | Some(Value::None)
            );
            let defaults_required = truthy(
                self.config()
                    .get_str("json_schema_serialization_defaults_required"),
            );
            Ok(if defaults_required {
                !has_exclude_if
            } else {
                required && !has_exclude_if
            })
        } else {
            Ok(required)
        }
    }

    /// Whether a field with the given schema should have a title set based on the field name:
    /// true for schemas that wouldn't otherwise provide their own title (e.g. int, str), false
    /// for those that would (e.g. models).
    fn field_title_should_be_set(schema: &Dict) -> JsResult<bool> {
        let type_: String = schema.get_as_req("type")?;
        if CORE_SCHEMA_FIELD_TYPES.contains(&type_.as_str()) {
            let key = if type_ == "computed-field" {
                "return_schema"
            } else {
                "schema"
            };
            return Self::field_title_should_be_set(sub_schema(schema, key)?);
        }
        // things with refs, such as models and enums, should not have titles set
        if truthy(schema.get_str("ref")) {
            return Ok(false);
        }
        match type_.as_str() {
            "default" | "nullable" | "definitions" | "function-before" | "function-after"
            | "function-wrap" => Self::field_title_should_be_set(sub_schema(schema, "schema")?),
            // Referenced schemas should not have titles set for the same reason schemas with
            // refs should not
            "definition-ref" => Ok(false),
            _ => Ok(true),
        }
    }

    fn definitions_schema(&mut self, schema: &Dict) -> JsResult<Dict> {
        let definitions: Vec<Value> = schema.get_as_req("definitions")?;
        for definition in &definitions {
            let definition = as_dict(definition)?;
            match self.generate_inner(definition) {
                Ok(_) => {}
                Err(JsonSchemaError::InvalidForJsonSchema(message)) => {
                    let core_ref: String = definition.get_as_req("ref")?;
                    let defs_ref = self.defs_refs.get_defs_ref(&core_ref, self.mode());
                    self.core_defs_invalid_for_json_schema
                        .insert(defs_ref, message);
                }
                Err(e) => return Err(e),
            }
        }
        self.generate_inner(sub_schema(schema, "schema")?)
    }

    fn ser_schema(&mut self, schema: &Dict) -> JsResult<Option<Dict>> {
        match schema.get_str("type") {
            Some(Value::Str(t)) if t == "function-plain" || t == "function-wrap" => {
                match schema.get_str("return_schema") {
                    Some(return_schema) if !matches!(return_schema, Value::None) => {
                        Ok(Some(self.generate_inner(as_dict(return_schema)?)?))
                    }
                    _ => Ok(None),
                }
            }
            Some(Value::Str(t)) if t == "format" || t == "to-string" => {
                Ok(Some(self.str_schema(&typed("str"))))
            }
            Some(Value::Str(t)) if t == "model" => {
                Ok(Some(self.generate_inner(sub_schema(schema, "schema")?)?))
            }
            _ => Ok(None),
        }
    }

    /// The definitions key for a core ref and the schema referencing it, created on first use.
    fn get_cache_defs_ref_schema(&mut self, core_ref: &str) -> (String, Dict) {
        let core_mode_ref = (core_ref.to_owned(), self.mode());
        if let Some(defs_ref) = self.core_to_defs_refs.get(&core_mode_ref) {
            return (
                defs_ref.clone(),
                ref_schema(&self.core_to_json_refs[&core_mode_ref]),
            );
        }

        let defs_ref = self.defs_refs.get_defs_ref(core_ref, self.mode());

        // populate the ref translation mappings
        self.core_to_defs_refs
            .insert(core_mode_ref.clone(), defs_ref.clone());
        let json_ref = self.json_ref(&defs_ref);
        self.core_to_json_refs
            .insert(core_mode_ref, json_ref.clone());
        self.json_to_defs_refs
            .insert(json_ref.clone(), defs_ref.clone());
        (defs_ref, ref_schema(&json_ref))
    }

    /// Remove any sibling keys that are redundant with the referenced schema.
    fn handle_ref_overrides(&self, mut json_schema: Dict) -> JsResult<Dict> {
        let Some(Value::Str(json_ref)) = json_schema.get_str("$ref") else {
            return Ok(json_schema);
        };
        let Some(referenced) = self.get_schema_from_definitions(&json_ref.clone())? else {
            // This can happen when building schemas for models with not-yet-defined references
            return Ok(json_schema);
        };
        json_schema = json_schema
            .into_iter()
            .filter(|(k, v)| match k {
                Value::Str(key) if key == "$ref" => true,
                // redundant key
                key => !referenced.get(key).is_some_and(|r| r.py_eq(v)),
            })
            .collect();
        Ok(json_schema)
    }

    fn get_schema_from_definitions(&self, json_ref: &str) -> JsResult<Option<Dict>> {
        let Some(defs_ref) = self.json_to_defs_refs.get(json_ref) else {
            if is_http(json_ref) {
                return Ok(None);
            }
            return Err(CoreError::Key(json_ref.to_owned()).into());
        };
        if let Some(message) = self.core_defs_invalid_for_json_schema.get(defs_ref) {
            return Err(JsonSchemaError::InvalidForJsonSchema(message.clone()));
        }
        Ok(match self.definitions.get_str(defs_ref) {
            Some(Value::Dict(schema)) => Some(schema.clone()),
            _ => None,
        })
    }

    /// Count the `$ref`s anywhere in the JSON schema, following each definition once.
    fn get_json_ref_counts(&self, json_schema: &Value) -> JsResult<HashMap<String, usize>> {
        let mut json_refs = HashMap::new();
        self.add_json_refs(json_schema, &mut json_refs)?;
        Ok(json_refs)
    }

    fn add_json_refs(
        &self,
        schema: &Value,
        json_refs: &mut HashMap<String, usize>,
    ) -> JsResult<()> {
        match schema {
            Value::Dict(dict) => {
                if let Some(json_ref) = dict.get_str("$ref") {
                    let Value::Str(json_ref) = json_ref else {
                        return Ok(()); // '$ref' might have been the name of a property
                    };
                    let already_visited = json_refs.contains_key(json_ref);
                    *json_refs.entry(json_ref.clone()).or_insert(0) += 1;
                    if already_visited {
                        return Ok(()); // prevent recursion on a definition already visited
                    }
                    match self.json_to_defs_refs.get(json_ref) {
                        Some(defs_ref) => {
                            if let Some(message) =
                                self.core_defs_invalid_for_json_schema.get(defs_ref)
                            {
                                return Err(JsonSchemaError::InvalidForJsonSchema(message.clone()));
                            }
                            let definition = self
                                .definitions
                                .get_str(defs_ref)
                                .ok_or_else(|| CoreError::Key(defs_ref.clone()))?;
                            self.add_json_refs(definition, json_refs)?;
                        }
                        None if is_http(json_ref) => {}
                        None => return Err(CoreError::Key(json_ref.clone()).into()),
                    }
                }
                for (key, value) in dict.iter() {
                    // Skip examples that may contain arbitrary values and references
                    if matches!((key, value), (Value::Str(k), Value::List(_)) if k == "examples") {
                        continue;
                    }
                    self.add_json_refs(value, json_refs)?;
                }
            }
            Value::List(items) => {
                for item in items {
                    self.add_json_refs(item, json_refs)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn emit_warning(&mut self, kind: &str, detail: &str) {
        // upstream's `ignored_warning_kinds`
        if kind != "skipped-choice" {
            self.warnings.push(format!("{detail} [{kind}]"));
        }
    }

    fn build_definitions_remapping(&self) -> JsResult<DefinitionsRemapping> {
        let mut defs_to_json = HashMap::new();
        for defs_refs in self.defs_refs.prioritized_choices.values() {
            for defs_ref in defs_refs {
                defs_to_json.insert(defs_ref.clone(), self.json_ref(defs_ref));
            }
        }
        DefinitionsRemapping::from_prioritized_choices(
            &self.defs_refs.prioritized_choices,
            &defs_to_json,
            &self.definitions,
        )
    }

    /// Drop the definitions the schema does not reference, directly or indirectly.
    fn garbage_collect_definitions(&mut self, schema: &Value) -> JsResult<()> {
        let mut visited_defs_refs: HashSet<String> = HashSet::new();
        let mut unvisited_json_refs: Vec<String> = get_all_json_refs(schema).into_iter().collect();
        while let Some(next_json_ref) = unvisited_json_refs.pop() {
            match self.json_to_defs_refs.get(&next_json_ref) {
                Some(next_defs_ref) => {
                    if !visited_defs_refs.insert(next_defs_ref.clone()) {
                        continue;
                    }
                    let definition = self
                        .definitions
                        .get_str(next_defs_ref)
                        .ok_or_else(|| CoreError::Key(next_defs_ref.clone()))?;
                    unvisited_json_refs.extend(get_all_json_refs(definition));
                }
                None if is_http(&next_json_ref) => {}
                None => return Err(CoreError::Key(next_json_ref).into()),
            }
        }

        self.definitions = std::mem::take(&mut self.definitions)
            .into_iter()
            .filter(|(k, _)| visited_defs_refs.contains(&k.py_str()))
            .collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_python_str_title() {
        assert_eq!(get_title_from_name("user_id"), "User Id");
        assert_eq!(get_title_from_name("fooBar"), "Foobar");
        assert_eq!(get_title_from_name("a1b_c"), "A1B C");
        assert_eq!(get_title_from_name("_private_"), "Private");
        assert_eq!(get_title_from_name("élan vital"), "Élan Vital");
    }

    #[test]
    fn sorting_skips_properties_and_defaults() {
        let value = Value::from_json(
            r#"{"type": "object", "properties": {"b": {"z": 1, "a": 2}, "a": {}}, "default": {"y": 1, "x": 2}}"#,
        )
        .unwrap();
        assert_eq!(
            format!("{:?}", sort(value, None)),
            format!(
                "{:?}",
                Value::from_json(
                    r#"{"default": {"y": 1, "x": 2}, "properties": {"b": {"a": 2, "z": 1}, "a": {}}, "type": "object"}"#
                )
                .unwrap()
            )
        );
    }

    #[test]
    fn set_defaults_are_sorted_when_comparable() {
        let items = [Value::Int(3), Value::Float(1.5), Value::Bool(true)];
        assert_eq!(
            sorted_items(&items),
            Some(vec![Value::Bool(true), Value::Float(1.5), Value::Int(3)])
        );
        assert_eq!(
            sorted_items(&[Value::from("b"), Value::from("a")]),
            Some(vec![Value::from("a"), Value::from("b")])
        );
        assert_eq!(sorted_items(&[Value::from("b"), Value::Int(1)]), None);
    }

    #[test]
    fn deduplication_uses_python_equality() {
        let a = Value::from_json(r#"{"const": 1}"#).unwrap();
        let b = Value::from_json(r#"{"const": true}"#).unwrap();
        let c = Value::from_json(r#"{"const": 2}"#).unwrap();
        let dicts: Vec<Dict> = [a, b.clone(), c.clone()]
            .into_iter()
            .map(|v| match v {
                Value::Dict(d) => d,
                _ => unreachable!(),
            })
            .collect();
        let distinct: Vec<Value> = deduplicate_schemas(dicts)
            .into_iter()
            .map(Value::Dict)
            .collect();
        assert_eq!(format!("{distinct:?}"), format!("{:?}", [b, c]));
    }
}
