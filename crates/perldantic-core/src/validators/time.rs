//! `time` schema. Port of upstream `validators/time.rs`.

use std::sync::Arc;

use speedate::{MicrosecondsPrecisionOverflowBehavior, Time};

use crate::build_tools::{is_strict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, Number, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::datetime::{TZConstraint, extract_microseconds_precision};
use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug, Clone)]
pub struct TimeValidator {
    strict: bool,
    constraints: Option<TimeConstraints>,
    microseconds_precision: MicrosecondsPrecisionOverflowBehavior,
}

impl BuildValidator for TimeValidator {
    const EXPECTED_TYPE: &'static str = "time";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        Ok(Arc::new(
            Self {
                strict: is_strict(schema, config)?,
                constraints: TimeConstraints::from_schema(schema)?,
                microseconds_precision: extract_microseconds_precision(schema, config)?,
            }
            .into(),
        ))
    }
}

impl Validator for TimeValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let time = input
            .validate_time(state.strict_or(self.strict), self.microseconds_precision)?
            .unpack(state);
        if let Some(constraints) = &self.constraints {
            macro_rules! check_constraint {
                ($constraint:ident, $error:ident) => {
                    if let Some(constraint) = &constraints.$constraint
                        && !time.$constraint(constraint)
                    {
                        return Err(ValError::new(
                            ErrorType::$error {
                                $constraint: Number::String(constraint.to_string()),
                                context: None,
                            },
                            input,
                        ));
                    }
                };
            }

            check_constraint!(le, LessThanEqual);
            check_constraint!(lt, LessThan);
            check_constraint!(ge, GreaterThanEqual);
            check_constraint!(gt, GreaterThan);

            if let Some(tz_constraint) = &constraints.tz {
                tz_constraint.tz_check(time.tz_offset, input)?;
            }
        }
        Ok(Value::Time(time))
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

fn convert_time(schema: &Dict, key: &str) -> CoreResult<Option<Time>> {
    match schema.get_str(key) {
        Some(value) => {
            match value.validate_time(false, MicrosecondsPrecisionOverflowBehavior::default()) {
                Ok(v) => Ok(Some(v.into_inner())),
                Err(_) => schema_err!("'{key}' must be coercible to a time instance"),
            }
        }
        None => Ok(None),
    }
}

#[derive(Debug, Clone)]
struct TimeConstraints {
    le: Option<Time>,
    lt: Option<Time>,
    ge: Option<Time>,
    gt: Option<Time>,
    tz: Option<TZConstraint>,
}

impl TimeConstraints {
    fn from_schema(schema: &Dict) -> CoreResult<Option<Self>> {
        let c = Self {
            le: convert_time(schema, "le")?,
            lt: convert_time(schema, "lt")?,
            ge: convert_time(schema, "ge")?,
            gt: convert_time(schema, "gt")?,
            tz: TZConstraint::from_schema(schema)?,
        };
        if c.le.is_some() || c.lt.is_some() || c.ge.is_some() || c.gt.is_some() || c.tz.is_some() {
            Ok(Some(c))
        } else {
            Ok(None)
        }
    }
}
