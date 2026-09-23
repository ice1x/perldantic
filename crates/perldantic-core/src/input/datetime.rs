//! Parsing dates, times, datetimes and durations from text and numbers. Port of upstream
//! `input/datetime.rs` without the Python objects: values are speedate's types throughout, so
//! upstream's `EitherDate` / `EitherTime` / `EitherDateTime` / `EitherTimedelta` wrappers are
//! not needed.

use speedate::{
    Date, DateConfig, DateTime, DateTimeConfig, Duration, MicrosecondsPrecisionOverflowBehavior,
    ParseError, Time, TimeConfig,
};
use strum::EnumMessage;

use crate::errors::{ErrorType, ToErrorValue, ValError, ValResult};
use crate::validators::TemporalUnitMode;

fn documentation(err: &ParseError) -> String {
    err.get_documentation().unwrap_or_default().to_owned()
}

pub fn bytes_as_date(
    input: impl ToErrorValue,
    bytes: &[u8],
    mode: TemporalUnitMode,
) -> ValResult<Date> {
    Date::parse_bytes_with_config(
        bytes,
        &DateConfig::builder().timestamp_unit(mode.into()).build(),
    )
    .map_err(|err| {
        ValError::new(
            ErrorType::DateParsing {
                error: documentation(&err),
                context: None,
            },
            input,
        )
    })
}

pub fn bytes_as_time(
    input: impl ToErrorValue,
    bytes: &[u8],
    microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
) -> ValResult<Time> {
    Time::parse_bytes_with_config(
        bytes,
        &TimeConfig {
            microseconds_precision_overflow_behavior: microseconds_overflow_behavior,
            unix_timestamp_offset: Some(0),
        },
    )
    .map_err(|err| {
        ValError::new(
            ErrorType::TimeParsing {
                error: documentation(&err),
                context: None,
            },
            input,
        )
    })
}

fn datetime_error(input: impl ToErrorValue, err: &ParseError) -> ValError {
    ValError::new(
        ErrorType::DatetimeParsing {
            error: documentation(err),
            context: None,
        },
        input,
    )
}

fn timestamp_config(mode: TemporalUnitMode) -> DateTimeConfig {
    DateTimeConfig {
        time_config: TimeConfig {
            unix_timestamp_offset: Some(0),
            ..Default::default()
        },
        timestamp_unit: mode.into(),
    }
}

pub fn bytes_as_datetime(
    input: impl ToErrorValue,
    bytes: &[u8],
    microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    mode: TemporalUnitMode,
) -> ValResult<DateTime> {
    DateTime::parse_bytes_with_config(
        bytes,
        &DateTimeConfig {
            time_config: TimeConfig {
                microseconds_precision_overflow_behavior: microseconds_overflow_behavior,
                unix_timestamp_offset: Some(0),
            },
            timestamp_unit: mode.into(),
        },
    )
    .map_err(|err| datetime_error(input, &err))
}

pub fn int_as_datetime(
    input: impl ToErrorValue,
    timestamp: i64,
    timestamp_microseconds: u32,
    mode: TemporalUnitMode,
) -> ValResult<DateTime> {
    DateTime::from_timestamp_with_config(timestamp, timestamp_microseconds, &timestamp_config(mode))
        .map_err(|err| datetime_error(input, &err))
}

macro_rules! nan_check {
    ($input:ident, $float_value:ident, $error_type:ident) => {
        if $float_value.is_nan() {
            return Err(ValError::new(
                ErrorType::$error_type {
                    error: "NaN values not permitted".to_owned(),
                    context: None,
                },
                $input,
            ));
        }
    };
}

pub fn float_as_datetime(
    input: impl ToErrorValue,
    timestamp: f64,
    mode: TemporalUnitMode,
) -> ValResult<DateTime> {
    nan_check!(input, timestamp, DatetimeParsing);
    DateTime::from_float_with_config(timestamp, &timestamp_config(mode))
        .map_err(|err| datetime_error(input, &err))
}

/// A date as a datetime at midnight, without a timezone (upstream `date_as_datetime`).
pub fn date_as_datetime(date: Date) -> DateTime {
    DateTime {
        date,
        time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            microsecond: 0,
            tz_offset: None,
        },
    }
}

const MAX_U32: i64 = u32::MAX as i64;

pub fn int_as_time(
    input: impl ToErrorValue,
    timestamp: i64,
    timestamp_microseconds: u32,
) -> ValResult<Time> {
    let time_timestamp: u32 = match timestamp {
        t if t < 0_i64 => {
            return Err(ValError::new(
                ErrorType::TimeParsing {
                    error: "time in seconds should be positive".to_owned(),
                    context: None,
                },
                input,
            ));
        }
        // continue and use the speedate error for >86400
        t if t > MAX_U32 => u32::MAX,
        t => u32::try_from(t).expect("checked above"),
    };
    Time::from_timestamp_with_config(
        time_timestamp,
        timestamp_microseconds,
        &TimeConfig {
            unix_timestamp_offset: Some(0),
            ..Default::default()
        },
    )
    .map_err(|err| {
        ValError::new(
            ErrorType::TimeParsing {
                error: documentation(&err),
                context: None,
            },
            input,
        )
    })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn float_as_time(input: impl ToErrorValue, timestamp: f64) -> ValResult<Time> {
    nan_check!(input, timestamp, TimeParsing);
    let microseconds = timestamp.fract().abs() * 1_000_000.0;
    // round for the same reason as upstream: to avoid 0.9999999 microseconds
    int_as_time(input, timestamp.floor() as i64, microseconds.round() as u32)
}

fn map_timedelta_err(input: impl ToErrorValue, err: &ParseError) -> ValError {
    ValError::new(
        ErrorType::TimeDeltaParsing {
            error: documentation(err),
            context: None,
        },
        input,
    )
}

pub fn bytes_as_timedelta(
    input: impl ToErrorValue,
    bytes: &[u8],
    microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
) -> ValResult<Duration> {
    Duration::parse_bytes_with_config(
        bytes,
        &TimeConfig {
            microseconds_precision_overflow_behavior: microseconds_overflow_behavior,
            unix_timestamp_offset: Some(0),
        },
    )
    .map_err(|err| map_timedelta_err(input, &err))
}

#[allow(clippy::cast_possible_truncation)]
pub fn int_as_duration(input: impl ToErrorValue, total_seconds: i64) -> ValResult<Duration> {
    let positive = total_seconds >= 0;
    let total_seconds = total_seconds.unsigned_abs();
    let days = (total_seconds / 86400) as u32;
    let seconds = (total_seconds % 86400) as u32;
    Duration::new(positive, days, seconds, 0).map_err(|err| map_timedelta_err(input, &err))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn float_as_duration(input: impl ToErrorValue, total_seconds: f64) -> ValResult<Duration> {
    nan_check!(input, total_seconds, TimeDeltaParsing);
    let positive = total_seconds >= 0_f64;
    let total_seconds = total_seconds.abs();
    let microsecond = total_seconds.fract() * 1_000_000.0;
    let days = (total_seconds / 86400f64) as u32;
    let seconds = total_seconds as u64 % 86400;
    Duration::new(positive, days, seconds as u32, microsecond.round() as u32)
        .map_err(|err| map_timedelta_err(input, &err))
}
