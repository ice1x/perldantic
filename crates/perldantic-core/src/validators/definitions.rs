//! `definitions` and `definition-ref` schemas. Port of upstream `validators/definitions.rs`.

use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::{DefinitionRef, DefinitionsBuilder};
use crate::errors::{ErrorTypeDefaults, LocItem, ValError, ValResult};
use crate::input::Input;
use crate::recursion_guard::RecursionGuard;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

/// Registers shared definitions, then builds the inner schema.
#[derive(Debug, Clone)]
pub struct DefinitionsValidatorBuilder;

impl BuildValidator for DefinitionsValidatorBuilder {
    const EXPECTED_TYPE: &'static str = "definitions";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let schema_definitions: Vec<Value> = schema.get_as_req("definitions")?;

        for schema_definition in &schema_definitions {
            let schema_definition = as_dict(schema_definition)?;
            let reference: String = schema_definition.get_as_req("ref")?;
            let validator = build_validator(schema_definition, config, definitions)?;
            definitions.add_definition(reference, validator)?;
        }

        let inner_schema: Dict = schema.get_as_req("schema")?;
        build_validator(&inner_schema, config, definitions)
    }
}

/// Validates with a shared definition, guarding against validating the same host value with
/// the same definition again (a cycle) and against unbounded nesting.
#[derive(Debug, Clone)]
pub struct DefinitionRefValidator {
    definition: DefinitionRef<Arc<CombinedValidator>>,
}

impl BuildValidator for DefinitionRefValidator {
    const EXPECTED_TYPE: &'static str = "definition-ref";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let schema_ref: String = schema.get_as_req("schema_ref")?;
        let definition = definitions.get_definition(&schema_ref);
        Ok(Arc::new(CombinedValidator::DefinitionRef(Self {
            definition,
        })))
    }
}

impl Validator for DefinitionRefValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        // this validator does not yet support partial validation, disable it to avoid incorrect
        // results
        state.allow_partial = false.into();

        self.definition.read(|validator| {
            let validator = validator.expect("definitions are filled before validation");
            // Host data is guarded by identity, as upstream guards Python objects (which can be
            // cyclic); a host value reached again with the same definition is a loop.
            if let Some(id) = input.identity() {
                let Ok(mut guard) = RecursionGuard::new(state, id, self.definition.id()) else {
                    return Err(ValError::new(ErrorTypeDefaults::RecursionLoop, input));
                };
                validator.validate(input, guard.state())
            } else {
                validator.validate(input, state)
            }
        })
    }

    fn default_value(
        &self,
        outer_loc: Option<impl Into<LocItem>>,
        state: &mut ValidationState<'_>,
    ) -> ValResult<Option<Value>> {
        self.definition.read(|validator| {
            let validator = validator.expect("definitions are filled before validation");
            validator.default_value(outer_loc, state)
        })
    }

    fn get_name(&self) -> &str {
        self.definition.get_or_init_name(|v| v.get_name().into())
    }
}
