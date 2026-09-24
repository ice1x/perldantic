//! `is-instance` schema. Port of upstream `validators/is_instance.rs`: the class is a name
//! (docs/DIVERGENCES.md #13), checked with [`Value::is_instance`].

use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug, Clone)]
pub struct IsInstanceValidator {
    class: String,
    class_repr: String,
    name: String,
}

impl BuildValidator for IsInstanceValidator {
    const EXPECTED_TYPE: &'static str = "is-instance";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let class: String = schema.get_as_req("cls")?;
        let class_repr: String = schema.get_as("cls_repr")?.unwrap_or_else(|| class.clone());
        Ok(Arc::new(CombinedValidator::IsInstance(Self {
            name: format!("{}[{class_repr}]", Self::EXPECTED_TYPE),
            class,
            class_repr,
        })))
    }
}

impl Validator for IsInstanceValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        _state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        // JSON holds no objects of any class
        let Some(value) = input.as_value() else {
            return Err(ValError::new(
                ErrorType::NeedsPythonObject {
                    context: None,
                    method_name: "isinstance".to_owned(),
                },
                input,
            ));
        };
        if value.is_instance(&self.class) {
            Ok(value.clone())
        } else {
            Err(ValError::new(
                ErrorType::IsInstanceOf {
                    class: self.class_repr.clone(),
                    context: None,
                },
                input,
            ))
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
