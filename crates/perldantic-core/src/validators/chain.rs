//! `chain` schema. Port of upstream `validators/chain.rs`: each step validates the output of
//! the previous one.

use std::sync::Arc;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
pub struct ChainValidator {
    steps: Vec<Arc<CombinedValidator>>,
    name: String,
}

impl BuildValidator for ChainValidator {
    const EXPECTED_TYPE: &'static str = "chain";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let step_schemas: Vec<Value> = schema.get_as_req("steps")?;
        let mut steps = Vec::with_capacity(step_schemas.len());
        for step in &step_schemas {
            // the steps of a nested chain are flattened into this one
            let validator = build_validator(as_dict(step)?, config, definitions)?;
            match validator.as_ref() {
                CombinedValidator::Chain(chain) => steps.extend(chain.steps.iter().cloned()),
                _ => steps.push(validator),
            }
        }
        match steps.len() {
            0 => schema_err!("One or more steps are required for a chain validator"),
            1 => Ok(steps.pop().expect("one step")),
            _ => {
                let descr = steps
                    .iter()
                    .map(|v| v.get_name())
                    .collect::<Vec<_>>()
                    .join(",");
                Ok(Arc::new(CombinedValidator::Chain(Self {
                    steps,
                    name: format!("{}[{descr}]", Self::EXPECTED_TYPE),
                })))
            }
        }
    }
}

impl Validator for ChainValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let mut steps = self.steps.iter();
        let first = steps.next().expect("chains have steps");
        let value = first.validate(input, state)?;
        steps.try_fold(value, |value, step| step.validate(&value, state))
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
