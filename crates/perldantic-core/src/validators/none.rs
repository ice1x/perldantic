//! `none` schema. Port of upstream `validators/none.rs`.

use std::sync::{Arc, LazyLock};

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorTypeDefaults, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug, Clone)]
pub struct NoneValidator;

static NONE_VALIDATOR: LazyLock<Arc<CombinedValidator>> =
    LazyLock::new(|| Arc::new(NoneValidator.into()));

impl BuildValidator for NoneValidator {
    const EXPECTED_TYPE: &'static str = "none";

    fn build(
        _schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        Ok(NONE_VALIDATOR.clone())
    }
}

impl Validator for NoneValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        _state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        if input.is_none() {
            Ok(Value::None)
        } else {
            Err(ValError::new(ErrorTypeDefaults::NoneRequired, input))
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
