//! `custom-error` schema and the custom errors other schemas (e.g. `union`) can declare.
//! Port of upstream `validators/custom_error.rs`.

use std::sync::Arc;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ToErrorValue, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, build_validator};

/// The error reported instead of the inner errors: a built-in type (upstream
/// `PydanticKnownError`) or a user-defined one (upstream `PydanticCustomError`).
#[derive(Debug, Clone)]
pub struct CustomError(ErrorType);

impl CustomError {
    pub fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Option<Self>> {
        let error_type: String = match schema.get_as("custom_error_type")? {
            Some(error_type) => error_type,
            None => return Ok(None),
        };
        let context: Option<Dict> = schema.get_as("custom_error_context")?;

        if ErrorType::valid_type(&error_type) {
            if schema.get_str("custom_error_message").is_some() {
                schema_err!(
                    "custom_error_message should not be provided if 'custom_error_type' matches a known error"
                )
            } else {
                Ok(Some(Self(ErrorType::new(&error_type, context.as_ref())?)))
            }
        } else {
            let message: String = schema.get_as_req("custom_error_message")?;
            Ok(Some(Self(ErrorType::new_custom_error(
                error_type, message, context,
            ))))
        }
    }

    pub fn as_val_error(&self, input: impl ToErrorValue) -> ValError {
        ValError::new(self.0.clone(), input)
    }
}

#[derive(Debug)]
pub struct CustomErrorValidator {
    validator: Arc<CombinedValidator>,
    custom_error: CustomError,
    name: String,
}

impl BuildValidator for CustomErrorValidator {
    const EXPECTED_TYPE: &'static str = "custom-error";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        // Upstream unwraps here and panics without a type (docs/DIVERGENCES.md #11).
        let Some(custom_error) = CustomError::build(schema, config, definitions)? else {
            return schema_err!("`custom_error_type` is required");
        };
        let schema: Dict = schema.get_as_req("schema")?;
        let validator = build_validator(&schema, config, definitions)?;
        let name = format!("{}[{}]", Self::EXPECTED_TYPE, validator.get_name());
        Ok(Arc::new(CombinedValidator::CustomError(Self {
            validator,
            custom_error,
            name,
        })))
    }
}

impl Validator for CustomErrorValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        self.validator
            .validate(input, state)
            .map_err(|_| self.custom_error.as_val_error(input))
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
