//! `date` schema. Port of upstream `validators/date.rs`.

use std::sync::Arc;

use speedate::{Date, MicrosecondsPrecisionOverflowBehavior, Time};
use strum::EnumMessage;

use crate::build_tools::{is_strict, schema_err};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, Number, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::datetime::{NowConstraint, NowOp};
use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, TemporalUnitMode, Validator};

#[derive(Debug, Clone)]
pub struct DateValidator {
    strict: bool,
    constraints: Option<DateConstraints>,
    val_temporal_unit: TemporalUnitMode,
}

impl BuildValidator for DateValidator {
    const EXPECTED_TYPE: &'static str = "date";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        Ok(Arc::new(
            Self {
                strict: is_strict(schema, config)?,
                constraints: DateConstraints::from_schema(schema)?,
                val_temporal_unit: TemporalUnitMode::from_config(config)?,
            }
            .into(),
        ))
    }
}

impl Validator for DateValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let strict = state.strict_or(self.strict);
        let date = match input.validate_date(strict, self.val_temporal_unit) {
            Ok(val_match) => val_match.unpack(state),
            // if the error was a parsing error, in lax mode we allow datetimes at midnight
            Err(line_errors @ ValError::LineErrors(..)) if !strict => {
                state.floor_exactness(Exactness::Lax);
                date_from_datetime(input, self.val_temporal_unit)?.ok_or(line_errors)?
            }
            Err(otherwise) => return Err(otherwise),
        };
        if let Some(constraints) = &self.constraints {
            macro_rules! check_constraint {
                ($constraint:ident, $error:ident) => {
                    if let Some(constraint) = &constraints.$constraint
                        && !date.$constraint(constraint)
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

            if let Some(today_constraint) = &constraints.today {
                let today = Date::today(today_constraint.utc_offset()).map_err(|e| {
                    CoreError::Schema(format!(
                        "Date::today() error: {}",
                        e.get_documentation().unwrap_or("unknown")
                    ))
                })?;
                // `if let Some(c)` to match behaviour of gt/lt/le/ge
                if let Some(c) = date.partial_cmp(&today)
                    && !today_constraint.op.compare(c)
                {
                    let error_type = match today_constraint.op {
                        NowOp::Past => ErrorTypeDefaults::DatePast,
                        NowOp::Future => ErrorTypeDefaults::DateFuture,
                    };
                    return Err(ValError::new(error_type, input));
                }
            }
        }
        if date.year == 0 {
            return Err(ValError::new(
                ErrorType::DateParsing {
                    error: "year 0 is out of range".to_owned(),
                    context: None,
                },
                input,
            ));
        }
        Ok(Value::Date(date))
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

/// In lax mode, if the input is not a date, we try parsing the input as a datetime, then check
/// it is an "exact date", e.g. has a zero time component.
///
/// `Ok(None)` means that this is not relevant to dates (the input was not a datetime nor a
/// string).
fn date_from_datetime(
    input: &(impl Input + ?Sized),
    mode: TemporalUnitMode,
) -> ValResult<Option<Date>> {
    let dt =
        match input.validate_datetime(false, MicrosecondsPrecisionOverflowBehavior::Truncate, mode)
        {
            Ok(val_match) => val_match.into_inner(),
            // if the error was a parsing error, update the error type from DatetimeParsing to
            // DateFromDatetimeParsing and return it
            Err(ValError::LineErrors(mut line_errors)) => {
                if line_errors
                    .iter_mut()
                    .fold(false, |has_parsing_error, line_error| {
                        if let ErrorType::DatetimeParsing { error, .. } = &mut line_error.error_type
                        {
                            line_error.error_type = ErrorType::DateFromDatetimeParsing {
                                error: std::mem::take(error),
                                context: None,
                            };
                            true
                        } else {
                            has_parsing_error
                        }
                    })
                {
                    return Err(ValError::LineErrors(line_errors));
                }
                return Ok(None);
            }
            // for any other error, don't return it
            Err(_) => return Ok(None),
        };
    let zero_time = Time {
        hour: 0,
        minute: 0,
        second: 0,
        microsecond: 0,
        tz_offset: dt.time.tz_offset,
    };
    if dt.time == zero_time {
        Ok(Some(dt.date))
    } else {
        Err(ValError::new(
            ErrorTypeDefaults::DateFromDatetimeInexact,
            input,
        ))
    }
}

#[derive(Debug, Clone)]
struct DateConstraints {
    le: Option<Date>,
    lt: Option<Date>,
    ge: Option<Date>,
    gt: Option<Date>,
    today: Option<NowConstraint>,
}

impl DateConstraints {
    fn from_schema(schema: &Dict) -> CoreResult<Option<Self>> {
        let c = Self {
            le: convert_date(schema, "le")?,
            lt: convert_date(schema, "lt")?,
            ge: convert_date(schema, "ge")?,
            gt: convert_date(schema, "gt")?,
            today: NowConstraint::from_schema(schema)?,
        };
        if c.le.is_some() || c.lt.is_some() || c.ge.is_some() || c.gt.is_some() || c.today.is_some()
        {
            Ok(Some(c))
        } else {
            Ok(None)
        }
    }
}

fn convert_date(schema: &Dict, key: &str) -> CoreResult<Option<Date>> {
    match schema.get_str(key) {
        Some(value) => match value.validate_date(false, TemporalUnitMode::default()) {
            Ok(v) => Ok(Some(v.into_inner())),
            Err(_) => schema_err!("'{key}' must be coercible to a date instance"),
        },
        None => Ok(None),
    }
}
