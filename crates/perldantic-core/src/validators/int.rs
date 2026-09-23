//! `int` schema. Port of upstream `validators/int.rs`.

use std::sync::{Arc, LazyLock};

use num_bigint::BigInt;

use crate::build_tools::is_strict;
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ValError, ValResult};
use crate::input::{Input, Int};
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

/// Read a constraint that must be (coercible to) an integer.
fn validate_as_int(schema: &Dict, key: &str) -> CoreResult<Option<Int>> {
    match schema.get_str(key) {
        Some(value) => match value.validate_int(false) {
            Ok(v) => Ok(Some(v.into_inner().as_int())),
            Err(_) => Err(CoreError::Value(format!(
                "'{key}' must be coercible to an integer"
            ))),
        },
        None => Ok(None),
    }
}

fn has_constraints(schema: &Dict) -> bool {
    ["multiple_of", "le", "lt", "ge", "gt"]
        .iter()
        .any(|key| schema.get_str(key).is_some())
}

#[derive(Debug, Clone)]
pub struct IntValidator {
    strict: bool,
}

static STRICT_INT_VALIDATOR: LazyLock<Arc<CombinedValidator>> =
    LazyLock::new(|| Arc::new(IntValidator { strict: true }.into()));

static LAX_INT_VALIDATOR: LazyLock<Arc<CombinedValidator>> =
    LazyLock::new(|| Arc::new(IntValidator { strict: false }.into()));

impl BuildValidator for IntValidator {
    const EXPECTED_TYPE: &'static str = "int";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        if has_constraints(schema) {
            ConstrainedIntValidator::build(schema, config)
        } else if is_strict(schema, config)? {
            Ok(STRICT_INT_VALIDATOR.clone())
        } else {
            Ok(LAX_INT_VALIDATOR.clone())
        }
    }
}

impl Validator for IntValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        input
            .validate_int(state.strict_or(self.strict))
            .map(|val_match| val_match.unpack(state).into_value())
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

#[derive(Debug, Clone)]
pub struct ConstrainedIntValidator {
    strict: bool,
    multiple_of: Option<Int>,
    le: Option<Int>,
    lt: Option<Int>,
    ge: Option<Int>,
    gt: Option<Int>,
}

impl ConstrainedIntValidator {
    fn build(schema: &Dict, config: Option<&Dict>) -> CoreResult<Arc<CombinedValidator>> {
        Ok(Arc::new(CombinedValidator::ConstrainedInt(Box::new(
            Self {
                strict: is_strict(schema, config)?,
                multiple_of: validate_as_int(schema, "multiple_of")?,
                le: validate_as_int(schema, "le")?,
                lt: validate_as_int(schema, "lt")?,
                ge: validate_as_int(schema, "ge")?,
                gt: validate_as_int(schema, "gt")?,
            },
        ))))
    }
}

impl Validator for ConstrainedIntValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let either_int = input
            .validate_int(state.strict_or(self.strict))?
            .unpack(state);
        let int_value = either_int.as_int();

        if let Some(ref multiple_of) = self.multiple_of
            && (&int_value % multiple_of != Int::Big(BigInt::from(0)))
        {
            return Err(ValError::new(
                ErrorType::MultipleOf {
                    multiple_of: multiple_of.clone().into(),
                    context: None,
                },
                input,
            ));
        }
        if let Some(ref le) = self.le
            && (&int_value > le)
        {
            return Err(ValError::new(
                ErrorType::LessThanEqual {
                    le: le.clone().into(),
                    context: None,
                },
                input,
            ));
        }
        if let Some(ref lt) = self.lt
            && (&int_value >= lt)
        {
            return Err(ValError::new(
                ErrorType::LessThan {
                    lt: lt.clone().into(),
                    context: None,
                },
                input,
            ));
        }
        if let Some(ref ge) = self.ge
            && (&int_value < ge)
        {
            return Err(ValError::new(
                ErrorType::GreaterThanEqual {
                    ge: ge.clone().into(),
                    context: None,
                },
                input,
            ));
        }
        if let Some(ref gt) = self.gt
            && (&int_value <= gt)
        {
            return Err(ValError::new(
                ErrorType::GreaterThan {
                    gt: gt.clone().into(),
                    context: None,
                },
                input,
            ));
        }
        Ok(either_int.into_value())
    }

    fn get_name(&self) -> &'static str {
        "constrained-int"
    }
}
