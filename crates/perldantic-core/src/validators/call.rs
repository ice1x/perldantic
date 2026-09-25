//! `call` schema. Port of upstream `validators/call.rs`: validate the arguments, call the host
//! function with them, and validate what it returns with `return_schema`.

use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ValError, ValResult};
use crate::host::{Function, HostCall};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, build_validator};

#[derive(Debug)]
pub struct CallValidator {
    function: Function,
    arguments_validator: Arc<CombinedValidator>,
    return_validator: Option<Arc<CombinedValidator>>,
    name: String,
}

impl BuildValidator for CallValidator {
    const EXPECTED_TYPE: &'static str = "call";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let arguments_schema: Dict = schema.get_as_req("arguments_schema")?;
        let arguments_validator = build_validator(&arguments_schema, config, definitions)?;
        let return_validator = match schema.get_as::<Dict>("return_schema")? {
            Some(return_schema) => Some(build_validator(&return_schema, config, definitions)?),
            None => None,
        };
        let function = match schema.get_str("function") {
            Some(Value::Function(function)) => function.clone(),
            Some(other) => {
                return Err(CoreError::Type(format!(
                    "'{}' object is not callable",
                    other.type_name()
                )));
            }
            None => return Err(CoreError::Key("function".to_owned())),
        };
        let function_name: String = schema
            .get_as("function_name")?
            .unwrap_or_else(|| function.name().to_owned());
        Ok(Arc::new(CombinedValidator::FunctionCall(Box::new(Self {
            name: format!("{}[{function_name}]", Self::EXPECTED_TYPE),
            function,
            arguments_validator,
            return_validator,
        }))))
    }
}

impl Validator for CallValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let (args, kwargs) = match self.arguments_validator.validate(input, state)? {
            Value::Tuple(mut parts) if parts.len() == 2 => match (parts.pop(), parts.pop()) {
                (Some(Value::Dict(kwargs)), Some(Value::Tuple(args))) => (args, kwargs),
                _ => return Err(not_arguments()),
            },
            Value::Dict(kwargs) => (Vec::new(), kwargs),
            _ => return Err(not_arguments()),
        };
        let return_value = self
            .function
            .call(HostCall::Call { args, kwargs })
            .map_err(crate::host::HostError::into_call_error)?;
        match &self.return_validator {
            Some(return_validator) => return_validator
                .validate(&return_value, state)
                .map_err(|e| e.with_outer_location("return")),
            None => Ok(return_value),
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

fn not_arguments() -> ValError {
    ValError::InternalErr(CoreError::Type(
        "Arguments validator should return a tuple of (args, kwargs) or a dict of kwargs"
            .to_owned(),
    ))
}
