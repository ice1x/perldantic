//! `function-before`, `function-after`, `function-plain` and `function-wrap` schemas. Port of
//! upstream `validators/function.rs`: the Python callables are host functions
//! ([`crate::host`]).
//!
//! Assignment validation (`validate_assignment`) is not ported yet, so the assignment variants
//! of these validators and of the wrap handler are left out.

use std::sync::Arc;

use jiter::PartialMode;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{LocItem, ValError, ValResult, ValidationError};
use crate::host::{Function, HostCall, HostError, ValidationInfo, ValidatorHandler};
use crate::input::Input;
use crate::recursion_guard::RecursionState;
use crate::value::{Dict, Value};

use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Extra, Validator, build_validator};

/// The `function` entry of a function schema: `{"type": "with-info" | "no-info", "function",
/// "field_name"}`.
struct FunctionInfo {
    function: Function,
    field_name: Option<String>,
    info_arg: bool,
}

fn destructure_function_schema(schema: &Dict) -> CoreResult<FunctionInfo> {
    let func_dict: Dict = schema.get_as_req("function")?;
    let function = match func_dict.get_str("function") {
        Some(Value::Function(function)) => function.clone(),
        Some(other) => {
            return schema_err!(
                "`function` should be a host function, got {}",
                other.type_name()
            );
        }
        None => return schema_err!("`function` is required"),
    };
    let func_type: String = func_dict.get_as_req("type")?;
    let info_arg = match func_type.as_str() {
        "with-info" => true,
        "no-info" => false,
        other => {
            return schema_err!(
                "function `type` should be 'with-info' or 'no-info', got '{other}'"
            );
        }
    };
    Ok(FunctionInfo {
        function,
        field_name: func_dict.get_as("field_name")?,
        info_arg,
    })
}

/// What the validators share: the function, how to call it, and the schema's config.
#[derive(Debug, Clone)]
struct FunctionCall {
    func: Function,
    config: Option<Dict>,
    field_name: Option<String>,
    info_arg: bool,
}

impl FunctionCall {
    fn new(info: FunctionInfo, config: Option<&Dict>) -> Self {
        Self {
            func: info.function,
            config: config.cloned(),
            field_name: info.field_name,
            info_arg: info.info_arg,
        }
    }

    /// The `info` argument, when the function takes one.
    fn info(&self, state: &ValidationState<'_>) -> Option<ValidationInfo> {
        self.info_arg.then(|| {
            let extra = state.extra();
            ValidationInfo {
                config: self.config.clone(),
                context: extra.context.cloned(),
                data: state.data.clone(),
                field_name: state
                    .field_name()
                    .map(str::to_owned)
                    .or_else(|| self.field_name.clone()),
                mode: extra.input_type,
            }
        })
    }

    /// `f(value[, info])`; what it raises is reported against `input`.
    fn call(
        &self,
        value: Value,
        input: &(impl Input + ?Sized),
        state: &ValidationState<'_>,
    ) -> ValResult<Value> {
        let info = self.info(state);
        self.func
            .call(HostCall::Validate { input: value, info })
            .map_err(|e| e.into_val_error(&input.as_error_value()))
    }
}

/// The name upstream gives function validators: `function-after[f(), int]`.
fn name(kind: &str, func: &Function, inner: Option<&CombinedValidator>) -> String {
    match inner {
        Some(inner) => format!("{kind}[{}(), {}]", func.name(), inner.get_name()),
        None => format!("{kind}[{}()]", func.name()),
    }
}

macro_rules! impl_build {
    ($impl_name:ident, $name:literal) => {
        impl BuildValidator for $impl_name {
            const EXPECTED_TYPE: &'static str = $name;

            fn build(
                schema: &Dict,
                config: Option<&Dict>,
                definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
            ) -> CoreResult<Arc<CombinedValidator>> {
                let inner: Dict = schema.get_as_req("schema")?;
                let validator = build_validator(&inner, config, definitions)?;
                let call = FunctionCall::new(destructure_function_schema(schema)?, config);
                Ok(Arc::new(
                    Self {
                        name: name($name, &call.func, Some(&validator)),
                        validator,
                        call,
                    }
                    .into(),
                ))
            }
        }
    };
}

#[derive(Debug)]
pub struct FunctionBeforeValidator {
    validator: Arc<CombinedValidator>,
    call: FunctionCall,
    name: String,
}

impl_build!(FunctionBeforeValidator, "function-before");

impl Validator for FunctionBeforeValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let value = self.call.call(input.to_value(), input, state)?;
        self.validator.validate(&value, state)
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug)]
pub struct FunctionAfterValidator {
    validator: Arc<CombinedValidator>,
    call: FunctionCall,
    name: String,
}

impl_build!(FunctionAfterValidator, "function-after");

impl Validator for FunctionAfterValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let value = self.validator.validate(input, state)?;
        self.call.call(value, input, state)
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone)]
pub struct FunctionPlainValidator {
    call: FunctionCall,
    name: String,
}

impl BuildValidator for FunctionPlainValidator {
    const EXPECTED_TYPE: &'static str = "function-plain";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let call = FunctionCall::new(destructure_function_schema(schema)?, config);
        Ok(Arc::new(
            Self {
                name: name(Self::EXPECTED_TYPE, &call.func, None),
                call,
            }
            .into(),
        ))
    }
}

impl Validator for FunctionPlainValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        self.call.call(input.to_value(), input, state)
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug)]
pub struct FunctionWrapValidator {
    validator: Arc<CombinedValidator>,
    call: FunctionCall,
    name: String,
    hide_input_in_errors: bool,
}

impl BuildValidator for FunctionWrapValidator {
    const EXPECTED_TYPE: &'static str = "function-wrap";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let inner: Dict = schema.get_as_req("schema")?;
        let validator = build_validator(&inner, config, definitions)?;
        let call = FunctionCall::new(destructure_function_schema(schema)?, config);
        Ok(Arc::new(
            Self {
                name: name(Self::EXPECTED_TYPE, &call.func, None),
                validator,
                call,
                hide_input_in_errors: config.get_as("hide_input_in_errors")?.unwrap_or(false),
            }
            .into(),
        ))
    }
}

impl Validator for FunctionWrapValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let info = self.call.info(state);
        let mut handler = InternalValidator::new(
            "ValidatorCallable",
            &self.validator,
            state,
            self.hide_input_in_errors,
        );
        let result = self
            .call
            .func
            .call(HostCall::ValidateWrap {
                input: input.to_value(),
                handler: &mut handler,
                info,
            })
            .map_err(|e| e.into_val_error(&input.as_error_value()));
        state.exactness = handler.exactness;
        state.fields_set_count = handler.fields_set_count;
        result
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

/// Port of upstream `InternalValidator`: validation with a wrapped schema, run on behalf of a
/// host function (the wrap validator's `handler`) with the settings of the enclosing call.
struct InternalValidator<'v, 'a> {
    name: &'static str,
    validator: &'v CombinedValidator,
    extra: Extra<'a>,
    data: Option<Dict>,
    field_name: Option<String>,
    recursion_guard: RecursionState,
    exactness: Option<Exactness>,
    fields_set_count: Option<usize>,
    hide_input_in_errors: bool,
}

impl<'v, 'a> InternalValidator<'v, 'a> {
    fn new(
        name: &'static str,
        validator: &'v CombinedValidator,
        state: &ValidationState<'a>,
        hide_input_in_errors: bool,
    ) -> Self {
        Self {
            name,
            validator,
            extra: state.extra().clone(),
            data: state.data.clone(),
            field_name: state.field_name().map(str::to_owned),
            recursion_guard: (*state.recursion_guard).clone(),
            exactness: state.exactness,
            fields_set_count: state.fields_set_count,
            hide_input_in_errors,
        }
    }
}

impl ValidatorHandler for InternalValidator<'_, '_> {
    fn validate(
        &mut self,
        input: Value,
        outer_location: Option<LocItem>,
    ) -> Result<Value, HostError> {
        let mut state = ValidationState::new(
            self.extra.clone(),
            &mut self.recursion_guard,
            PartialMode::Off,
        );
        let state = &mut state.scoped_set_field_name(self.field_name.clone());
        state.data.clone_from(&self.data);
        state.exactness = self.exactness;
        state.fields_set_count = self.fields_set_count;
        let result = self.validator.validate(&input, state);
        self.exactness = state.exactness;
        self.fields_set_count = state.fields_set_count;
        result.map_err(|e| match e {
            ValError::LineErrors(errors) => {
                let errors = match outer_location {
                    Some(loc) => errors
                        .into_iter()
                        .map(|e| e.with_outer_location(loc.clone()))
                        .collect(),
                    None => errors,
                };
                HostError::Validation(ValidationError::new(
                    self.name,
                    errors,
                    self.extra.input_type,
                    self.hide_input_in_errors,
                ))
            }
            ValError::InternalErr(crate::core_error::CoreError::Host(exception)) => {
                HostError::Other(exception)
            }
            ValError::InternalErr(error) => HostError::Core(error),
            ValError::Omit => HostError::Omit,
            ValError::UseDefault => HostError::UseDefault,
        })
    }
}
