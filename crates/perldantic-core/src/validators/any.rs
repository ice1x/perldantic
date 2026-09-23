//! `any` schema. Port of upstream `validators/any.rs`.

use std::sync::{Arc, LazyLock};

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Validator};

/// Accepts anything unchanged. Also used where another validator needs an "any" default.
#[derive(Debug, Clone)]
pub struct AnyValidator;

static ANY_VALIDATOR: LazyLock<Arc<CombinedValidator>> =
    LazyLock::new(|| Arc::new(AnyValidator.into()));

impl BuildValidator for AnyValidator {
    const EXPECTED_TYPE: &'static str = "any";

    fn build(
        _schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        Ok(ANY_VALIDATOR.clone())
    }
}

impl Validator for AnyValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        // In a union, `any` should be preferred to lax coercions.
        state.floor_exactness(Exactness::Strict);
        Ok(input.to_value())
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
