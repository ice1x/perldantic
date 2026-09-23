//! `nullable` schema. Port of upstream `validators/nullable.rs`.

use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, build_validator};

/// `None` or whatever the inner schema accepts.
#[derive(Debug)]
pub struct NullableValidator {
    validator: Arc<CombinedValidator>,
    name: String,
}

impl BuildValidator for NullableValidator {
    const EXPECTED_TYPE: &'static str = "nullable";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let schema: Dict = schema.get_as_req("schema")?;
        let validator = build_validator(&schema, config, definitions)?;
        let name = format!("{}[{}]", Self::EXPECTED_TYPE, validator.get_name());
        Ok(Arc::new(CombinedValidator::Nullable(Self {
            validator,
            name,
        })))
    }
}

impl Validator for NullableValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        if input.is_none() {
            Ok(Value::None)
        } else {
            self.validator.validate(input, state)
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
