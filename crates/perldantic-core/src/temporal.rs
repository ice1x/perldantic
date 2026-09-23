//! Dates, times, datetimes and durations as Python shows and compares them: `repr`, `str`,
//! `==` and the normalised `timedelta` form. The values themselves are speedate's types, as
//! upstream uses them for everything that is not a Python object.

use std::cmp::Ordering;
use std::fmt::Write as _;

use speedate::{Date, DateTime, Duration, Time};

const MICROS_PER_SECOND: i128 = 1_000_000;
const MICROS_PER_DAY: i128 = 86_400 * MICROS_PER_SECOND;

/// Python's `timedelta` fields: `days` carries the sign, `0 <= seconds < 86400` and
/// `0 <= microseconds < 1_000_000`.
pub fn timedelta_parts(duration: &Duration) -> (i64, i64, i64) {
    let total = total_micros(duration);
    let days = total.div_euclid(MICROS_PER_DAY);
    let rest = total.rem_euclid(MICROS_PER_DAY);
    (
        i64::try_from(days).expect("speedate durations fit in i64 days"),
        i64::try_from(rest / MICROS_PER_SECOND).expect("less than a day"),
        i64::try_from(rest % MICROS_PER_SECOND).expect("less than a second"),
    )
}

/// The duration in microseconds, signed.
pub fn total_micros(duration: &Duration) -> i128 {
    let magnitude = i128::from(duration.day) * MICROS_PER_DAY
        + i128::from(duration.second) * MICROS_PER_SECOND
        + i128::from(duration.microsecond);
    if duration.positive {
        magnitude
    } else {
        -magnitude
    }
}

/// A duration from Python's `timedelta` fields (any values; they are normalised).
pub fn duration_from_parts(
    days: i64,
    seconds: i64,
    microseconds: i64,
) -> Result<Duration, speedate::ParseError> {
    let total = i128::from(days) * MICROS_PER_DAY
        + i128::from(seconds) * MICROS_PER_SECOND
        + i128::from(microseconds);
    let magnitude = total.unsigned_abs();
    let day = u32::try_from(magnitude / MICROS_PER_DAY.unsigned_abs())
        .map_err(|_| speedate::ParseError::DurationDaysTooLarge)?;
    let rest = magnitude % MICROS_PER_DAY.unsigned_abs();
    Duration::new(
        total >= 0,
        day,
        u32::try_from(rest / MICROS_PER_SECOND.unsigned_abs()).expect("less than a day"),
        u32::try_from(rest % MICROS_PER_SECOND.unsigned_abs()).expect("less than a second"),
    )
}

/// Python's `repr` of pydantic's `TzInfo`.
fn tzinfo_repr(offset: i32) -> String {
    format!("TzInfo({offset})")
}

/// A UTC offset as `isoformat` writes it: `+01:00`, `-05:30`, `+00:00:30`.
pub fn offset_str(offset: i32) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let offset = offset.unsigned_abs();
    let mut out = format!("{sign}{:02}:{:02}", offset / 3600, (offset / 60) % 60);
    if !offset.is_multiple_of(60) {
        write!(out, ":{:02}", offset % 60).expect("writing to a String");
    }
    out
}

pub fn date_repr(date: &Date) -> String {
    format!("datetime.date({}, {}, {})", date.year, date.month, date.day)
}

pub fn date_str(date: &Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

/// `hour, minute[, second[, microsecond]]` as `time.__repr__` writes them.
fn time_fields(time: &Time) -> String {
    let mut out = format!("{}, {}", time.hour, time.minute);
    if time.microsecond != 0 {
        write!(out, ", {}, {}", time.second, time.microsecond).expect("writing to a String");
    } else if time.second != 0 {
        write!(out, ", {}", time.second).expect("writing to a String");
    }
    out
}

fn with_tzinfo(mut repr: String, tz_offset: Option<i32>) -> String {
    if let Some(offset) = tz_offset {
        repr.pop();
        write!(repr, ", tzinfo={})", tzinfo_repr(offset)).expect("writing to a String");
    }
    repr
}

pub fn time_repr(time: &Time) -> String {
    with_tzinfo(
        format!("datetime.time({})", time_fields(time)),
        time.tz_offset,
    )
}

pub fn time_str(time: &Time) -> String {
    let mut out = format!("{:02}:{:02}:{:02}", time.hour, time.minute, time.second);
    if time.microsecond != 0 {
        write!(out, ".{:06}", time.microsecond).expect("writing to a String");
    }
    if let Some(offset) = time.tz_offset {
        out.push_str(&offset_str(offset));
    }
    out
}

pub fn datetime_repr(dt: &DateTime) -> String {
    let (date, time) = (&dt.date, &dt.time);
    with_tzinfo(
        format!(
            "datetime.datetime({}, {}, {}, {})",
            date.year,
            date.month,
            date.day,
            time_fields(time)
        ),
        time.tz_offset,
    )
}

pub fn datetime_str(dt: &DateTime) -> String {
    format!("{} {}", date_str(&dt.date), time_str(&dt.time))
}

pub fn timedelta_repr(duration: &Duration) -> String {
    let (days, seconds, microseconds) = timedelta_parts(duration);
    let mut args = Vec::new();
    if days != 0 {
        args.push(format!("days={days}"));
    }
    if seconds != 0 {
        args.push(format!("seconds={seconds}"));
    }
    if microseconds != 0 {
        args.push(format!("microseconds={microseconds}"));
    }
    if args.is_empty() {
        args.push("0".to_owned());
    }
    format!("datetime.timedelta({})", args.join(", "))
}

pub fn timedelta_str(duration: &Duration) -> String {
    let (days, seconds, microseconds) = timedelta_parts(duration);
    let mut out = String::new();
    if days != 0 {
        let plural = if days.abs() == 1 { "" } else { "s" };
        write!(out, "{days} day{plural}, ").expect("writing to a String");
    }
    write!(
        out,
        "{}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
    .expect("writing to a String");
    if microseconds != 0 {
        write!(out, ".{microseconds:06}").expect("writing to a String");
    }
    out
}

/// Seconds since midnight, shifted to UTC when the time has an offset.
fn time_utc_micros(time: &Time) -> i64 {
    let local = i64::from(time.hour) * 3_600_000_000
        + i64::from(time.minute) * 60_000_000
        + i64::from(time.second) * 1_000_000
        + i64::from(time.microsecond);
    local - i64::from(time.tz_offset.unwrap_or(0)) * 1_000_000
}

/// Python's `==` on times: naive and aware times are never equal; aware ones compare in UTC.
pub fn time_py_eq(a: &Time, b: &Time) -> bool {
    match (a.tz_offset, b.tz_offset) {
        (None, None) => a == b,
        (Some(_), Some(_)) => time_utc_micros(a) == time_utc_micros(b),
        _ => false,
    }
}

/// Python's `==` on datetimes: naive and aware datetimes are never equal; aware ones compare as
/// instants.
pub fn datetime_py_eq(a: &DateTime, b: &DateTime) -> bool {
    match (a.time.tz_offset, b.time.tz_offset) {
        (None, None) => a == b,
        (Some(_), Some(_)) => a.partial_cmp(b) == Some(Ordering::Equal),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(hour: u8, minute: u8, second: u8, microsecond: u32, tz_offset: Option<i32>) -> Time {
        Time {
            hour,
            minute,
            second,
            microsecond,
            tz_offset,
        }
    }

    fn date(year: u16, month: u8, day: u8) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn dates_and_times_print_like_python() {
        assert_eq!(date_repr(&date(2022, 6, 8)), "datetime.date(2022, 6, 8)");
        assert_eq!(date_str(&date(2022, 6, 8)), "2022-06-08");
        assert_eq!(time_repr(&time(12, 0, 0, 0, None)), "datetime.time(12, 0)");
        assert_eq!(
            time_repr(&time(12, 0, 5, 0, None)),
            "datetime.time(12, 0, 5)"
        );
        assert_eq!(
            time_repr(&time(12, 0, 0, 7, Some(3600))),
            "datetime.time(12, 0, 0, 7, tzinfo=TzInfo(3600))"
        );
        assert_eq!(
            time_str(&time(1, 2, 3, 40, Some(-19800))),
            "01:02:03.000040-05:30"
        );
        let dt = DateTime {
            date: date(2022, 6, 8),
            time: time(0, 0, 0, 0, Some(0)),
        };
        assert_eq!(
            datetime_repr(&dt),
            "datetime.datetime(2022, 6, 8, 0, 0, tzinfo=TzInfo(0))"
        );
        assert_eq!(datetime_str(&dt), "2022-06-08 00:00:00+00:00");
        assert_eq!(offset_str(30), "+00:00:30");
    }

    #[test]
    fn durations_print_like_timedelta() {
        let d = |days, seconds, micros| duration_from_parts(days, seconds, micros).unwrap();
        assert_eq!(timedelta_repr(&d(0, 0, 0)), "datetime.timedelta(0)");
        assert_eq!(
            timedelta_repr(&d(1, 1, 0)),
            "datetime.timedelta(days=1, seconds=1)"
        );
        assert_eq!(
            timedelta_repr(&d(0, 0, -123_000)),
            "datetime.timedelta(days=-1, seconds=86399, microseconds=877000)"
        );
        assert_eq!(timedelta_str(&d(0, 3600, 0)), "1:00:00");
        assert_eq!(timedelta_str(&d(2, 61, 5)), "2 days, 0:01:01.000005");
        assert_eq!(timedelta_str(&d(0, -1, 0)), "-1 day, 23:59:59");
        assert_eq!(timedelta_parts(&d(0, -1, 0)), (-1, 86399, 0));
        assert!(duration_from_parts(1_000_000_000, 0, 0).is_err());
    }

    #[test]
    fn equality_follows_python() {
        let naive = time(12, 0, 0, 0, None);
        let utc = time(12, 0, 0, 0, Some(0));
        let plus_one = time(13, 0, 0, 0, Some(3600));
        assert!(!time_py_eq(&naive, &utc));
        assert!(time_py_eq(&utc, &plus_one));
        let at = |time| DateTime {
            date: date(2022, 1, 1),
            time,
        };
        assert!(datetime_py_eq(&at(utc), &at(plus_one)));
        assert!(!datetime_py_eq(&at(naive), &at(utc)));
    }
}
