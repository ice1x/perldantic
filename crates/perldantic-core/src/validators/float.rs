//! `float` schema. Port of upstream `validators/float.rs`.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::build_tools::{SchemaDict, is_strict, schema_or_config_same};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

/// Builds a plain or constrained float validator for `float` schemas.
pub struct FloatBuilder;

impl BuildValidator for FloatBuilder {
    const EXPECTED_TYPE: &'static str = "float";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let use_constrained = ["multiple_of", "le", "lt", "ge", "gt"]
            .iter()
            .any(|key| schema.get_str(key).is_some());
        if use_constrained {
            ConstrainedFloatValidator::build(schema, config, definitions)
        } else {
            FloatValidator::build(schema, config, definitions)
        }
    }
}

fn allow_inf_nan(schema: &Dict, config: Option<&Dict>) -> CoreResult<bool> {
    Ok(schema_or_config_same(schema, config, "allow_inf_nan")?.unwrap_or(true))
}

#[derive(Debug, Clone)]
pub struct FloatValidator {
    strict: bool,
    allow_inf_nan: bool,
}

impl BuildValidator for FloatValidator {
    const EXPECTED_TYPE: &'static str = "float";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        Ok(Arc::new(CombinedValidator::Float(Self {
            strict: is_strict(schema, config)?,
            allow_inf_nan: allow_inf_nan(schema, config)?,
        })))
    }
}

impl Validator for FloatValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let either_float = input
            .validate_float(state.strict_or(self.strict))?
            .unpack(state);
        if !self.allow_inf_nan && !either_float.as_f64().is_finite() {
            return Err(ValError::new(ErrorTypeDefaults::FiniteNumber, input));
        }
        Ok(Value::Float(either_float.as_f64()))
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

#[derive(Debug, Clone)]
pub struct ConstrainedFloatValidator {
    strict: bool,
    allow_inf_nan: bool,
    multiple_of: Option<f64>,
    le: Option<f64>,
    lt: Option<f64>,
    ge: Option<f64>,
    gt: Option<f64>,
}

impl Validator for ConstrainedFloatValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let either_float = input
            .validate_float(state.strict_or(self.strict))?
            .unpack(state);
        let float: f64 = either_float.as_f64();
        if !self.allow_inf_nan && !float.is_finite() {
            return Err(ValError::new(ErrorTypeDefaults::FiniteNumber, input));
        }
        if let Some(multiple_of) = self.multiple_of {
            let tolerance = 1e-9;
            let rounded_div = (float / multiple_of).round();
            let diff = (float - (rounded_div * multiple_of)).abs();
            if diff > tolerance {
                return Err(ValError::new(
                    ErrorType::MultipleOf {
                        multiple_of: multiple_of.into(),
                        context: None,
                    },
                    input,
                ));
            }
        }
        if let Some(le) = self.le
            && !matches!(
                float.partial_cmp(&le),
                Some(Ordering::Less | Ordering::Equal)
            )
        {
            return Err(ValError::new(
                ErrorType::LessThanEqual {
                    le: le.into(),
                    context: None,
                },
                input,
            ));
        }
        if let Some(lt) = self.lt
            && !matches!(float.partial_cmp(&lt), Some(Ordering::Less))
        {
            return Err(ValError::new(
                ErrorType::LessThan {
                    lt: lt.into(),
                    context: None,
                },
                input,
            ));
        }
        if let Some(ge) = self.ge
            && !matches!(
                float.partial_cmp(&ge),
                Some(Ordering::Greater | Ordering::Equal)
            )
        {
            return Err(ValError::new(
                ErrorType::GreaterThanEqual {
                    ge: ge.into(),
                    context: None,
                },
                input,
            ));
        }
        if let Some(gt) = self.gt
            && !matches!(float.partial_cmp(&gt), Some(Ordering::Greater))
        {
            return Err(ValError::new(
                ErrorType::GreaterThan {
                    gt: gt.into(),
                    context: None,
                },
                input,
            ));
        }
        Ok(Value::Float(float))
    }

    fn get_name(&self) -> &'static str {
        "constrained-float"
    }
}

impl BuildValidator for ConstrainedFloatValidator {
    const EXPECTED_TYPE: &'static str = "float";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        Ok(Arc::new(CombinedValidator::ConstrainedFloat(Self {
            strict: is_strict(schema, config)?,
            allow_inf_nan: allow_inf_nan(schema, config)?,
            multiple_of: schema.get_as("multiple_of")?,
            le: schema.get_as("le")?,
            lt: schema.get_as("lt")?,
            ge: schema.get_as("ge")?,
            gt: schema.get_as("gt")?,
        })))
    }
}
