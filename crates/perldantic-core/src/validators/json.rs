//! `json` schema. Port of upstream `validators/json.rs`: the input is JSON text (a string or
//! bytes), parsed and then validated by the inner schema as JSON, or returned as data.

use std::sync::Arc;

use jiter::JsonValue;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, ValError, ValLineError, ValResult};
use crate::input::{EitherBytes, Input, InputType, ValidationMatch};
use crate::value::{Dict, Value};

use super::config::{BytesMode, ValBytesMode};
use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
pub struct JsonValidator {
    validator: Option<Arc<CombinedValidator>>,
    name: String,
}

impl BuildValidator for JsonValidator {
    const EXPECTED_TYPE: &'static str = "json";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let validator = match schema.get_str("schema") {
            Some(inner) => {
                let validator = build_validator(as_dict(inner)?, config, definitions)?;
                match validator.as_ref() {
                    CombinedValidator::Any(_) => None,
                    _ => Some(validator),
                }
            }
            None => None,
        };
        let name = format!(
            "{}[{}]",
            Self::EXPECTED_TYPE,
            validator.as_ref().map_or("any", |v| v.get_name())
        );
        Ok(Arc::new(CombinedValidator::Json(Self { validator, name })))
    }
}

impl Validator for JsonValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let json_bytes = validate_json_bytes(input)?.unpack(state);
        let json_bytes = json_bytes.as_slice();
        let json_value = JsonValue::parse_with_config(json_bytes, true, state.allow_partial)
            .map_err(|e| map_json_err(input, &e, json_bytes))?;
        match &self.validator {
            Some(validator) => {
                let mut json_state = state.rebind_extra(|e| {
                    e.input_type = InputType::Json;
                });
                validator.validate(&json_value, &mut json_state)
            }
            None => Ok(Value::from(&json_value)),
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

/// The input as JSON bytes: a string or bytes; other input is a `json_type` error.
pub fn validate_json_bytes(
    input: &(impl Input + ?Sized),
) -> ValResult<ValidationMatch<EitherBytes<'_>>> {
    match input.validate_bytes(
        false,
        ValBytesMode {
            ser: BytesMode::Utf8,
        },
    ) {
        Ok(v_match) => Ok(v_match),
        Err(ValError::LineErrors(errors)) => Err(ValError::LineErrors(
            errors.into_iter().map(map_bytes_error).collect(),
        )),
        Err(e) => Err(e),
    }
}

fn map_bytes_error(line_error: ValLineError) -> ValLineError {
    match line_error.error_type {
        ErrorType::BytesType { .. } => ValLineError {
            error_type: ErrorTypeDefaults::JsonType,
            ..line_error
        },
        _ => line_error,
    }
}

pub fn map_json_err(
    input: &(impl Input + ?Sized),
    error: &jiter::JsonError,
    json_bytes: &[u8],
) -> ValError {
    ValError::new(
        ErrorType::JsonInvalid {
            error: error.description(json_bytes),
            context: None,
        },
        input,
    )
}
