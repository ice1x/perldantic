//! `datetime` schema, and the `now_op` / `tz_constraint` / `microseconds_precision` settings the
//! temporal validators share. Port of upstream `validators/datetime.rs`.
//!
//! Without a `now_utc_offset`, upstream compares with the local time of the Python process;
//! the core has no access to the local timezone and uses UTC (docs/DIVERGENCES.md #16).

use std::cmp::Ordering;
use std::str::FromStr;
use std::sync::Arc;

use speedate::{DateTime, MicrosecondsPrecisionOverflowBehavior, Time};
use strum::EnumMessage;

use crate::build_tools::{SchemaDict, is_strict, schema_err, schema_or_config_same};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, Number, ToErrorValue, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, TemporalUnitMode, Validator};

#[derive(Debug, Clone)]
pub struct DateTimeValidator {
    strict: bool,
    constraints: Option<DateTimeConstraints>,
    microseconds_precision: MicrosecondsPrecisionOverflowBehavior,
    val_temporal_unit: TemporalUnitMode,
}

pub(crate) fn extract_microseconds_precision(
    schema: &Dict,
    config: Option<&Dict>,
) -> CoreResult<MicrosecondsPrecisionOverflowBehavior> {
    schema_or_config_same::<String>(schema, config, "microseconds_precision")?
        .map_or(Ok(MicrosecondsPrecisionOverflowBehavior::Truncate), |v| {
            MicrosecondsPrecisionOverflowBehavior::from_str(&v)
        })
        .map_err(|_| {
            CoreError::Schema(
                "Invalid `microseconds_precision`, must be one of \"truncate\" or \"error\""
                    .to_owned(),
            )
        })
}

impl BuildValidator for DateTimeValidator {
    const EXPECTED_TYPE: &'static str = "datetime";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        Ok(Arc::new(
            Self {
                strict: is_strict(schema, config)?,
                constraints: DateTimeConstraints::from_schema(schema)?,
                microseconds_precision: extract_microseconds_precision(schema, config)?,
                val_temporal_unit: TemporalUnitMode::from_config(config)?,
            }
            .into(),
        ))
    }
}

impl Validator for DateTimeValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let strict = state.strict_or(self.strict);
        let datetime = match input.validate_datetime(
            strict,
            self.microseconds_precision,
            self.val_temporal_unit,
        ) {
            Ok(val_match) => val_match.unpack(state),
            // if the error was a parsing error, in lax mode we allow dates and add the time 00:00:00
            Err(line_errors @ ValError::LineErrors(..)) if !strict => {
                state.floor_exactness(Exactness::Lax);
                datetime_from_date(input)?.ok_or(line_errors)?
            }
            Err(otherwise) => return Err(otherwise),
        };
        if let Some(constraints) = &self.constraints {
            macro_rules! check_constraint {
                ($constraint:ident, $error:ident) => {
                    if let Some(constraint) = &constraints.$constraint
                        && !datetime.$constraint(constraint)
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

            if let Some(now_constraint) = &constraints.now {
                let now = DateTime::now(now_constraint.utc_offset()).map_err(|e| {
                    CoreError::Schema(format!(
                        "DateTime::now() error: {}",
                        e.get_documentation().unwrap_or("unknown")
                    ))
                })?;
                // `if let Some(c)` to match behaviour of gt/lt/le/ge
                if let Some(c) = datetime.partial_cmp(&now)
                    && !now_constraint.op.compare(c)
                {
                    let error_type = match now_constraint.op {
                        NowOp::Past => ErrorTypeDefaults::DatetimePast,
                        NowOp::Future => ErrorTypeDefaults::DatetimeFuture,
                    };
                    return Err(ValError::new(error_type, input));
                }
            }

            if let Some(tz_constraint) = &constraints.tz {
                tz_constraint.tz_check(datetime.time.tz_offset, input)?;
            }
        }
        if datetime.date.year == 0 {
            return Err(ValError::new(
                ErrorType::DatetimeParsing {
                    error: "year 0 is out of range".to_owned(),
                    context: None,
                },
                input,
            ));
        }
        Ok(Value::DateTime(datetime))
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

/// In lax mode, if the input is not a datetime, we try parsing the input as a date and add the
/// "00:00:00" time. `Ok(None)` means that this is not relevant to datetimes (the input was not a
/// date nor a string).
fn datetime_from_date(input: &(impl Input + ?Sized)) -> ValResult<Option<DateTime>> {
    let date = match input.validate_date(false, TemporalUnitMode::default()) {
        Ok(val_match) => val_match.into_inner(),
        // if the error was a parsing error, update the error type from DateParsing to
        // DatetimeFromDateParsing
        Err(ValError::LineErrors(mut line_errors)) => {
            if line_errors
                .iter_mut()
                .fold(false, |has_parsing_error, line_error| {
                    if let ErrorType::DateParsing { error, .. } = &mut line_error.error_type {
                        line_error.error_type = ErrorType::DatetimeFromDateParsing {
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
        tz_offset: None,
    };
    Ok(Some(DateTime {
        date,
        time: zero_time,
    }))
}

#[derive(Debug, Clone)]
struct DateTimeConstraints {
    le: Option<DateTime>,
    lt: Option<DateTime>,
    ge: Option<DateTime>,
    gt: Option<DateTime>,
    now: Option<NowConstraint>,
    tz: Option<TZConstraint>,
}

impl DateTimeConstraints {
    fn from_schema(schema: &Dict) -> CoreResult<Option<Self>> {
        let c = Self {
            le: convert_datetime(schema, "le")?,
            lt: convert_datetime(schema, "lt")?,
            ge: convert_datetime(schema, "ge")?,
            gt: convert_datetime(schema, "gt")?,
            now: NowConstraint::from_schema(schema)?,
            tz: TZConstraint::from_schema(schema)?,
        };
        if c.le.is_some()
            || c.lt.is_some()
            || c.ge.is_some()
            || c.gt.is_some()
            || c.now.is_some()
            || c.tz.is_some()
        {
            Ok(Some(c))
        } else {
            Ok(None)
        }
    }
}

fn convert_datetime(schema: &Dict, key: &str) -> CoreResult<Option<DateTime>> {
    match schema.get_str(key) {
        Some(value) => match value.validate_datetime(
            false,
            MicrosecondsPrecisionOverflowBehavior::Truncate,
            TemporalUnitMode::default(),
        ) {
            Ok(v) => Ok(Some(v.into_inner())),
            Err(_) => schema_err!("'{key}' must be coercible to a datetime instance"),
        },
        None => Ok(None),
    }
}

#[derive(Debug, Clone)]
pub enum NowOp {
    Past,
    Future,
}

impl NowOp {
    pub fn compare(&self, ordering: Ordering) -> bool {
        match ordering {
            Ordering::Less => matches!(self, Self::Past),
            Ordering::Equal => false,
            Ordering::Greater => matches!(self, Self::Future),
        }
    }

    fn parse(s: &str) -> CoreResult<Self> {
        match s {
            "past" => Ok(Self::Past),
            "future" => Ok(Self::Future),
            _ => schema_err!("Invalid now_op {s:?}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NowConstraint {
    pub op: NowOp,
    utc_offset: Option<i32>,
}

impl NowConstraint {
    /// The UTC offset of "now" in seconds: `now_utc_offset`, or UTC (see the module docs).
    pub fn utc_offset(&self) -> i32 {
        self.utc_offset.unwrap_or(0)
    }

    pub fn from_schema(schema: &Dict) -> CoreResult<Option<Self>> {
        match schema.get_as::<String>("now_op")? {
            Some(op) => Ok(Some(Self {
                op: NowOp::parse(&op)?,
                utc_offset: schema
                    .get_as::<i64>("now_utc_offset")?
                    .map(offset_seconds)
                    .transpose()?,
            })),
            None => Ok(None),
        }
    }
}

fn offset_seconds(value: i64) -> CoreResult<i32> {
    i32::try_from(value).map_err(|_| CoreError::Schema(format!("Invalid UTC offset {value}")))
}

#[derive(Debug, Clone)]
pub(crate) enum TZConstraint {
    Naive,
    Aware(Option<i32>),
}

impl TZConstraint {
    pub(crate) fn from_schema(schema: &Dict) -> CoreResult<Option<Self>> {
        match schema.get_str("tz_constraint") {
            None => Ok(None),
            Some(Value::Str(s)) if s == "naive" => Ok(Some(Self::Naive)),
            Some(Value::Str(s)) if s == "aware" => Ok(Some(Self::Aware(None))),
            Some(Value::Str(s)) => schema_err!("Invalid tz_constraint {s:?}"),
            Some(_) => Ok(Some(Self::Aware(Some(offset_seconds(
                schema.get_as_req::<i64>("tz_constraint")?,
            )?)))),
        }
    }

    pub(crate) fn tz_check(
        &self,
        tz_offset: Option<i32>,
        input: impl ToErrorValue,
    ) -> ValResult<()> {
        match (self, tz_offset) {
            (Self::Aware(_), None) => {
                return Err(ValError::new(ErrorTypeDefaults::TimezoneAware, input));
            }
            (Self::Aware(Some(tz_expected)), Some(tz_actual)) => {
                let tz_expected = *tz_expected;
                if tz_expected != tz_actual {
                    return Err(ValError::new(
                        ErrorType::TimezoneOffset {
                            tz_expected,
                            tz_actual,
                            context: None,
                        },
                        input,
                    ));
                }
            }
            (Self::Naive, Some(_)) => {
                return Err(ValError::new(ErrorTypeDefaults::TimezoneNaive, input));
            }
            _ => (),
        }
        Ok(())
    }
}
