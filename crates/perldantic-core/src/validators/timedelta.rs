//! `timedelta` schema. Port of upstream `validators/timedelta.rs`.

use std::sync::Arc;

use speedate::{Duration, MicrosecondsPrecisionOverflowBehavior};

use crate::build_tools::{is_strict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, Number, ValError, ValResult};
use crate::input::Input;
use crate::temporal::timedelta_parts;
use crate::value::{Dict, Value};

use super::datetime::extract_microseconds_precision;
use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug, Clone)]
pub struct TimeDeltaValidator {
    strict: bool,
    constraints: Option<TimedeltaConstraints>,
    microseconds_precision: MicrosecondsPrecisionOverflowBehavior,
}

#[derive(Debug, Clone)]
struct TimedeltaConstraints {
    le: Option<Duration>,
    lt: Option<Duration>,
    ge: Option<Duration>,
    gt: Option<Duration>,
}

fn get_constraint(schema: &Dict, key: &str) -> CoreResult<Option<Duration>> {
    match schema.get_str(key) {
        Some(value) => {
            match value.validate_timedelta(false, MicrosecondsPrecisionOverflowBehavior::default())
            {
                Ok(v) => Ok(Some(v.into_inner())),
                Err(_) => schema_err!("'{key}' must be coercible to a timedelta instance"),
            }
        }
        None => Ok(None),
    }
}

impl BuildValidator for TimeDeltaValidator {
    const EXPECTED_TYPE: &'static str = "timedelta";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let constraints = TimedeltaConstraints {
            le: get_constraint(schema, "le")?,
            lt: get_constraint(schema, "lt")?,
            ge: get_constraint(schema, "ge")?,
            gt: get_constraint(schema, "gt")?,
        };
        Ok(Arc::new(
            Self {
                strict: is_strict(schema, config)?,
                constraints: (constraints.le.is_some()
                    || constraints.lt.is_some()
                    || constraints.ge.is_some()
                    || constraints.gt.is_some())
                .then_some(constraints),
                microseconds_precision: extract_microseconds_precision(schema, config)?,
            }
            .into(),
        ))
    }
}

impl Validator for TimeDeltaValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let timedelta = input
            .validate_timedelta(state.strict_or(self.strict), self.microseconds_precision)?
            .unpack(state);
        let output = Value::TimeDelta(timedelta.clone());
        if let Some(constraints) = &self.constraints {
            macro_rules! check_constraint {
                ($constraint:ident, $error:ident) => {
                    if let Some(constraint) = &constraints.$constraint
                        && !timedelta.$constraint(constraint)
                    {
                        // upstream reports the validated timedelta as the input
                        return Err(ValError::new(
                            ErrorType::$error {
                                context: None,
                                $constraint: Number::String(human_readable(constraint)),
                            },
                            &output,
                        ));
                    }
                };
            }

            check_constraint!(le, LessThanEqual);
            check_constraint!(lt, LessThan);
            check_constraint!(ge, GreaterThanEqual);
            check_constraint!(gt, GreaterThan);
        }
        Ok(output)
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

/// Upstream `pydelta_to_human_readable`: "1 day and 2 hours" from the `timedelta` fields.
fn human_readable(duration: &Duration) -> String {
    let (days, total_seconds, microseconds) = timedelta_parts(duration);
    let plural = |n: i64| if n == 1 { "" } else { "s" };
    let mut parts = Vec::new();
    for (value, unit) in [
        (days, "day"),
        (total_seconds / 3600, "hour"),
        ((total_seconds % 3600) / 60, "minute"),
        (total_seconds % 60, "second"),
        (microseconds, "microsecond"),
    ] {
        if value != 0 {
            parts.push(format!("{value} {unit}{}", plural(value)));
        }
    }
    if parts.is_empty() {
        parts.push("0 seconds".to_owned());
    }
    parts.join(" and ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::temporal::duration_from_parts;

    #[test]
    fn durations_read_like_upstream() {
        let d = |days, seconds, micros| duration_from_parts(days, seconds, micros).unwrap();
        assert_eq!(human_readable(&d(3, 0, 0)), "3 days");
        assert_eq!(human_readable(&d(0, 0, 0)), "0 seconds");
        assert_eq!(
            human_readable(&d(749, 3661, 100_000)),
            "749 days and 1 hour and 1 minute and 1 second and 100000 microseconds"
        );
        assert_eq!(
            human_readable(&d(-2, 86399, 877_000)),
            "-2 days and 23 hours and 59 minutes and 59 seconds and 877000 microseconds"
        );
    }
}
