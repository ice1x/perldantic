//! Validators and the `SchemaValidator` entry point. Port of upstream `validators/mod.rs`.

use std::fmt::{self, Debug};
use std::sync::Arc;

use enum_dispatch::enum_dispatch;
use jiter::{JsonValue, PartialMode};

use crate::build_tools::{ExtraBehavior, SchemaDict, schema_err};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::{Definitions, DefinitionsBuilder};
use crate::errors::{ErrorType, ValError, ValResult, ValidationError};
use crate::input::{Input, InputType};
use crate::recursion_guard::RecursionState;
use crate::value::{Dict, Value};

mod any;
pub(crate) mod config;
pub(crate) mod validation_state;

use validation_state::ValidationState;

/// Options of a single validation call (upstream `validate_python` / `validate_json` kwargs).
#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    pub strict: Option<bool>,
    pub extra_behavior: Option<ExtraBehavior>,
    pub from_attributes: Option<bool>,
    /// Passed to validator functions as `info.context`.
    pub context: Option<Value>,
    pub allow_partial: PartialMode,
    pub by_alias: Option<bool>,
    pub by_name: Option<bool>,
}

/// Why a validation call failed.
#[derive(Debug)]
pub enum ValidateError {
    /// The input is invalid.
    Validation(ValidationError),
    /// Validation could not run, e.g. a misused default or an internal fault.
    Core(CoreError),
}

impl fmt::Display for ValidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(e) => fmt::Display::fmt(e, f),
            Self::Core(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for ValidateError {}

/// A compiled core schema, ready to validate host data or JSON.
#[derive(Debug)]
pub struct SchemaValidator {
    validator: Arc<CombinedValidator>,
    // Keeps shared (possibly recursive) definitions alive; validators hold weak references.
    _definitions: Definitions<Arc<CombinedValidator>>,
    title: String,
    hide_input_in_errors: bool,
}

/// The error pyo3 raises when a non-dict is passed where a dict is required.
fn as_dict(value: &Value) -> CoreResult<&Dict> {
    match value {
        Value::Dict(d) => Ok(d),
        Value::None => Err(CoreError::Type(
            "'None' is not an instance of 'dict'".into(),
        )),
        other => Err(CoreError::Type(format!(
            "'{}' object is not an instance of 'dict'",
            other.type_name()
        ))),
    }
}

impl SchemaValidator {
    /// Build a validator from a core schema and optional core config (both dicts).
    pub fn new(schema: &Value, config: Option<&Value>) -> CoreResult<Self> {
        let schema = as_dict(schema)?;
        let config = match config {
            None | Some(Value::None) => None,
            Some(c) => Some(as_dict(c)?),
        };
        let mut definitions_builder = DefinitionsBuilder::new();
        let validator = build_validator(schema, config, &mut definitions_builder)?;
        let definitions = definitions_builder.finish()?;
        let title = match config.get_as::<String>("title")? {
            Some(title) => title,
            None => validator.get_name().to_owned(),
        };
        let hide_input_in_errors = config.get_as("hide_input_in_errors")?.unwrap_or(false);
        Ok(Self {
            validator,
            _definitions: definitions,
            title,
            hide_input_in_errors,
        })
    }

    /// Schema types this build can compile.
    pub fn supported_schema_types() -> &'static [&'static str] {
        SUPPORTED_SCHEMA_TYPES
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Validate host data (upstream `validate_python`).
    pub fn validate_value(
        &self,
        input: &Value,
        options: &ValidateOptions,
    ) -> Result<Value, ValidateError> {
        self.validate(input, InputType::Python, options)
            .map_err(|e| self.prepare_error(e, InputType::Python))
    }

    /// Validate a JSON document (upstream `validate_json`).
    pub fn validate_json(
        &self,
        input: &str,
        options: &ValidateOptions,
    ) -> Result<Value, ValidateError> {
        let result =
            match JsonValue::parse_with_config(input.as_bytes(), true, options.allow_partial) {
                Ok(json) => {
                    let options = ValidateOptions {
                        from_attributes: None,
                        ..options.clone()
                    };
                    self.validate(&json, InputType::Json, &options)
                }
                Err(e) => Err(ValError::new(
                    ErrorType::JsonInvalid {
                        error: e.description(input.as_bytes()),
                        context: None,
                    },
                    input,
                )),
            };
        result.map_err(|e| self.prepare_error(e, InputType::Json))
    }

    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        input_type: InputType,
        options: &ValidateOptions,
    ) -> ValResult<Value> {
        let mut recursion_guard = RecursionState::default();
        let extra = Extra::new(
            options.strict,
            options.extra_behavior,
            options.from_attributes,
            options.context.as_ref(),
            input_type,
            options.by_alias,
            options.by_name,
        );
        let mut state = ValidationState::new(extra, &mut recursion_guard, options.allow_partial);
        self.validator.validate(input, &mut state)
    }

    fn prepare_error(&self, error: ValError, input_type: InputType) -> ValidateError {
        match ValidationError::from_val_error(
            self.title.clone(),
            error,
            input_type,
            self.hide_input_in_errors,
        ) {
            Ok(e) => ValidateError::Validation(e),
            Err(e) => ValidateError::Core(e),
        }
    }
}

/// Settings that stay constant for one validation call.
#[derive(Debug, Clone)]
pub struct Extra<'a> {
    /// Validation mode
    pub input_type: InputType,
    /// Whether we're in strict or lax mode
    pub strict: Option<bool>,
    /// Whether to ignore, allow, or forbid extra data during model validation
    #[allow(clippy::struct_field_names)]
    pub extra_behavior: Option<ExtraBehavior>,
    /// Validation-time setting of `from_attributes`
    pub from_attributes: Option<bool>,
    /// Context passed to validator functions
    pub context: Option<&'a Value>,
    /// Whether to use the field's alias to match input data to an attribute
    by_alias: Option<bool>,
    /// Whether to use the field's name to match input data to an attribute
    by_name: Option<bool>,
}

impl<'a> Extra<'a> {
    pub fn new(
        strict: Option<bool>,
        extra_behavior: Option<ExtraBehavior>,
        from_attributes: Option<bool>,
        context: Option<&'a Value>,
        input_type: InputType,
        by_alias: Option<bool>,
        by_name: Option<bool>,
    ) -> Self {
        Extra {
            input_type,
            strict,
            extra_behavior,
            from_attributes,
            context,
            by_alias,
            by_name,
        }
    }
}

/// Builds a validator from its schema; `EXPECTED_TYPE` is the schema `type` it handles.
pub(crate) trait BuildValidator: Sized {
    const EXPECTED_TYPE: &'static str;

    /// Build from the schema. Returning a `CombinedValidator` lets a builder pick a more
    /// specific validator (e.g. a constrained variant).
    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>>;
}

/// Registers validators: generates the build dispatch and the list of supported schema types.
macro_rules! validators {
    ($($validator:path,)+) => {
        const SUPPORTED_SCHEMA_TYPES: &[&str] = &[$(<$validator>::EXPECTED_TYPE,)+];

        fn build_by_type(
            type_: &str,
            schema: &Dict,
            config: Option<&Dict>,
            definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
        ) -> CoreResult<Arc<CombinedValidator>> {
            // Only errors raised while building a known type are wrapped with that type, as
            // upstream does; unknown and invalid types are reported as they are.
            match type_ {
                $(<$validator>::EXPECTED_TYPE => <$validator>::build(schema, config, definitions)
                    .map_err(|err| failed_to_build_validator(type_, &err)),)+
                "invalid" => schema_err!("Cannot construct schema with `InvalidSchema` member."),
                _ => schema_err!("Unknown schema type: \"{type_}\""),
            }
        }
    };
}

validators!(any::AnyValidator,);

/// Build the validator for a schema dict.
pub(crate) fn build_validator(
    schema: &Dict,
    config: Option<&Dict>,
    definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
) -> CoreResult<Arc<CombinedValidator>> {
    let type_: String = schema.get_as_req("type")?;
    build_by_type(&type_, schema, config, definitions)
}

#[cold]
fn failed_to_build_validator(val_type: &str, err: &CoreError) -> CoreError {
    CoreError::Schema(format!(
        "Error building \"{val_type}\" validator:\n  {}: {err}",
        err.kind().python_name()
    ))
}

/// Every validator, dispatched statically.
#[derive(Debug)]
#[enum_dispatch]
pub enum CombinedValidator {
    Any(any::AnyValidator),
}

/// Implemented by all validators.
#[enum_dispatch(CombinedValidator)]
pub(crate) trait Validator: Send + Sync + Debug {
    /// Validate `input`, returning the validated value.
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value>;

    /// The name used in union error locations and as the default error title.
    fn get_name(&self) -> &str;
}
