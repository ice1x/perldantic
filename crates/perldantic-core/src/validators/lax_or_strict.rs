//! `lax-or-strict` schema. Port of upstream `validators/lax_or_strict.rs`.

use std::sync::Arc;

use crate::build_tools::{SchemaDict, is_strict};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Validator, build_validator};

#[derive(Debug)]
pub struct LaxOrStrictValidator {
    strict: bool,
    lax_validator: Arc<CombinedValidator>,
    strict_validator: Arc<CombinedValidator>,
    name: String,
}

impl BuildValidator for LaxOrStrictValidator {
    const EXPECTED_TYPE: &'static str = "lax-or-strict";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let lax_schema: Dict = schema.get_as_req("lax_schema")?;
        let lax_validator = build_validator(&lax_schema, config, definitions)?;

        let strict_schema: Dict = schema.get_as_req("strict_schema")?;
        let strict_validator = build_validator(&strict_schema, config, definitions)?;

        let name = format!(
            "{}[lax={},strict={}]",
            Self::EXPECTED_TYPE,
            lax_validator.get_name(),
            strict_validator.get_name()
        );
        Ok(Arc::new(CombinedValidator::LaxOrStrict(Self {
            strict: is_strict(schema, config)?,
            lax_validator,
            strict_validator,
            name,
        })))
    }
}

impl Validator for LaxOrStrictValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        if state.strict_or(self.strict) {
            self.strict_validator.validate(input, state)
        } else {
            // horrible edge case: if doing smart union validation, we need to try the strict
            // validator anyway and prefer that if it succeeds
            if state.exactness.is_some() {
                if let Ok(strict_result) = self.strict_validator.validate(input, state) {
                    return Ok(strict_result);
                }
                // this is now known to be not strict
                state.floor_exactness(Exactness::Lax);
            }
            self.lax_validator.validate(input, state)
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
