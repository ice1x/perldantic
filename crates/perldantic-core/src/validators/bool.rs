//! `bool` schema. Port of upstream `validators/bool.rs`.

use std::sync::{Arc, LazyLock};

use crate::build_tools::is_strict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug, Clone)]
pub struct BoolValidator {
    strict: bool,
}

static STRICT_BOOL_VALIDATOR: LazyLock<Arc<CombinedValidator>> =
    LazyLock::new(|| Arc::new(BoolValidator { strict: true }.into()));

static LAX_BOOL_VALIDATOR: LazyLock<Arc<CombinedValidator>> =
    LazyLock::new(|| Arc::new(BoolValidator { strict: false }.into()));

impl BuildValidator for BoolValidator {
    const EXPECTED_TYPE: &'static str = "bool";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        if is_strict(schema, config)? {
            Ok(STRICT_BOOL_VALIDATOR.clone())
        } else {
            Ok(LAX_BOOL_VALIDATOR.clone())
        }
    }
}

impl Validator for BoolValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        input
            .validate_bool(state.strict_or(self.strict))
            .map(|val_match| Value::Bool(val_match.unpack(state)))
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
