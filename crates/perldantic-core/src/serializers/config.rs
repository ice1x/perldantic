//! Serialization settings read from the core config. Port of upstream `serializers/config.rs`
//! for the P0 types; temporal modes come with the date and time types.

use std::borrow::Cow;
use std::str::{FromStr, from_utf8};

use base64::Engine;
use serde::ser::Error;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::{CoreError, CoreResult};
use speedate::{Date, DateTime, Duration, Time};

use crate::temporal;
use crate::value::{Dict, Value};

/// Settings that apply to a whole serializer.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_field_names)]
pub(crate) struct SerializationConfig {
    pub temporal_mode: TemporalMode,
    pub bytes_mode: BytesMode,
    pub inf_nan_mode: InfNanMode,
}

impl Default for SerializationConfig {
    fn default() -> Self {
        Self {
            temporal_mode: TemporalMode::default(),
            bytes_mode: BytesMode::default(),
            inf_nan_mode: InfNanMode::Constants,
        }
    }
}

impl SerializationConfig {
    pub fn from_config(config: Option<&Dict>) -> CoreResult<Self> {
        // `ser_json_temporal` wins over the older `ser_json_timedelta` when it is set
        let temporal_set = config.is_some_and(|c| c.get_str("ser_json_temporal").is_some());
        let temporal_mode = if temporal_set {
            TemporalMode::from_config(config)?
        } else {
            TimedeltaMode::from_config(config)?.into()
        };
        Ok(Self {
            temporal_mode,
            bytes_mode: BytesMode::from_config(config)?,
            inf_nan_mode: InfNanMode::from_config(config)?,
        })
    }
}

/// Generates a mode enum read from a config key, with upstream's error wording (its trailing
/// "or" included).
macro_rules! serialization_mode {
    ($name:ident, $config_key:expr, $($variant:ident => $value:expr),* $(,)?) => {
        #[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            #[default]
            $($variant,)*
        }

        impl FromStr for $name {
            type Err = CoreError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($value => Ok(Self::$variant),)*
                    s => schema_err!(
                        concat!("Invalid ", stringify!($name), " serialization mode: `{}`, expected ", $($value, " or "),*),
                        s
                    ),
                }
            }
        }

        impl $name {
            pub fn from_config(config: Option<&Dict>) -> CoreResult<Self> {
                match config.get_as::<String>($config_key)? {
                    Some(raw) => Self::from_str(&raw),
                    None => Ok(Self::default()),
                }
            }
        }
    };
}

serialization_mode! {
    TimedeltaMode,
    "ser_json_timedelta",
    Iso8601 => "iso8601",
    Float => "float",
}

serialization_mode! {
    TemporalMode,
    "ser_json_temporal",
    Iso8601 => "iso8601",
    Seconds => "seconds",
    Milliseconds => "milliseconds",
}

impl From<TimedeltaMode> for TemporalMode {
    fn from(value: TimedeltaMode) -> Self {
        match value {
            TimedeltaMode::Iso8601 => Self::Iso8601,
            TimedeltaMode::Float => Self::Seconds,
        }
    }
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn datetime_to_seconds(dt: &DateTime) -> f64 {
    dt.timestamp_tz() as f64 + f64::from(dt.time.microsecond) / 1_000_000.0
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn datetime_to_milliseconds(dt: &DateTime) -> f64 {
    dt.timestamp_tz() as f64 * 1_000.0 + f64::from(dt.time.microsecond) / 1_000.0
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn date_to_seconds(date: Date) -> f64 {
    date.timestamp() as f64
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn date_to_milliseconds(date: Date) -> f64 {
    date.timestamp_ms() as f64
}

pub(crate) fn time_to_seconds(time: &Time) -> f64 {
    f64::from(time.hour) * 3600.0
        + f64::from(time.minute) * 60.0
        + f64::from(time.second)
        + f64::from(time.microsecond) / 1_000_000.0
}

pub(crate) fn time_to_milliseconds(time: &Time) -> f64 {
    f64::from(time.hour) * 3_600_000.0
        + f64::from(time.minute) * 60_000.0
        + f64::from(time.second) * 1_000.0
        + f64::from(time.microsecond) / 1_000.0
}

/// Upstream `EitherTimedelta::total_seconds`, falling back to floating point on overflow.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn timedelta_total_seconds(duration: &Duration) -> f64 {
    temporal::total_micros(duration) as f64 / 1_000_000.0
}

#[allow(clippy::cast_precision_loss)]
pub(crate) fn timedelta_total_milliseconds(duration: &Duration) -> f64 {
    temporal::total_micros(duration) as f64 / 1_000.0
}

impl TemporalMode {
    /// The JSON form of a temporal value: ISO 8601 text or a number of (milli)seconds. `None`
    /// for other values.
    pub fn to_json(self, value: &Value) -> Option<Value> {
        let (seconds, milliseconds, iso) = match value {
            Value::DateTime(dt) => (
                datetime_to_seconds(dt),
                datetime_to_milliseconds(dt),
                dt.to_string(),
            ),
            Value::Date(d) => (date_to_seconds(*d), date_to_milliseconds(*d), d.to_string()),
            Value::Time(t) => (time_to_seconds(t), time_to_milliseconds(t), t.to_string()),
            Value::TimeDelta(d) => (
                timedelta_total_seconds(d),
                timedelta_total_milliseconds(d),
                d.to_string(),
            ),
            _ => return None,
        };
        Some(match self {
            Self::Iso8601 => Value::Str(iso),
            Self::Seconds => Value::Float(seconds),
            Self::Milliseconds => Value::Float(milliseconds),
        })
    }

    /// The JSON object key for a temporal value.
    pub fn json_key(self, value: &Value) -> Option<String> {
        match self.to_json(value)? {
            Value::Str(s) => Some(s),
            Value::Float(f) => Some(float_key(f)),
            _ => None,
        }
    }
}

/// Rust's `f64` Display, as upstream's `seconds.to_string()` writes object keys.
fn float_key(f: f64) -> String {
    f.to_string()
}

serialization_mode! {
    BytesMode,
    "ser_json_bytes",
    Utf8 => "utf8",
    Base64 => "base64",
    Hex => "hex",
}

serialization_mode! {
    InfNanMode,
    "ser_json_inf_nan",
    Null => "null",
    Constants => "constants",
    Strings => "strings",
}

impl BytesMode {
    pub fn bytes_to_string(self, bytes: &[u8]) -> CoreResult<Cow<'_, str>> {
        match self {
            Self::Utf8 => from_utf8(bytes)
                .map_err(|err| utf8_error(&err, bytes))
                .map(Cow::Borrowed),
            Self::Base64 => Ok(Cow::Owned(
                base64::engine::general_purpose::URL_SAFE.encode(bytes),
            )),
            Self::Hex => Ok(Cow::Owned(hex::encode(bytes))),
        }
    }

    pub fn serialize_bytes<S: serde::ser::Serializer>(
        self,
        bytes: &[u8],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match self {
            Self::Utf8 => match from_utf8(bytes) {
                Ok(s) => serializer.serialize_str(s),
                Err(e) => Err(Error::custom(e.to_string())),
            },
            Self::Base64 => {
                serializer.serialize_str(&base64::engine::general_purpose::URL_SAFE.encode(bytes))
            }
            Self::Hex => serializer.serialize_str(hex::encode(bytes).as_str()),
        }
    }
}

/// The `UnicodeDecodeError` upstream raises (built by pyo3, so its reason is always
/// "invalid utf-8").
fn utf8_error(err: &std::str::Utf8Error, bytes: &[u8]) -> CoreError {
    let start = err.valid_up_to();
    let end = start + err.error_len().unwrap_or(bytes.len() - start);
    let position = if end - start == 1 {
        format!("byte 0x{:02x} in position {start}", bytes[start])
    } else {
        format!("bytes in position {start}-{}", end - 1)
    };
    CoreError::UnicodeDecode(format!(
        "'utf-8' codec can't decode {position}: invalid utf-8"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_read_from_config_with_upstream_errors() {
        let config: Dict = [(
            crate::value::Value::from("ser_json_bytes"),
            crate::value::Value::from("hex"),
        )]
        .into_iter()
        .collect();
        assert_eq!(
            BytesMode::from_config(Some(&config)).unwrap(),
            BytesMode::Hex
        );
        assert_eq!(BytesMode::from_config(None).unwrap(), BytesMode::Utf8);
        assert_eq!(
            InfNanMode::from_str("bogus").unwrap_err().to_string(),
            "Invalid InfNanMode serialization mode: `bogus`, expected null or constants or strings or "
        );
    }

    #[test]
    fn bytes_as_text() {
        assert_eq!(BytesMode::Utf8.bytes_to_string(b"ab").unwrap(), "ab");
        assert_eq!(
            BytesMode::Base64.bytes_to_string(b"\xfb\xff").unwrap(),
            "-_8="
        );
        assert_eq!(BytesMode::Hex.bytes_to_string(b"\x01\xab").unwrap(), "01ab");
    }

    #[test]
    fn invalid_utf8_reads_like_upstream() {
        let err = |b: &[u8]| BytesMode::Utf8.bytes_to_string(b).unwrap_err();
        let prefix = "'utf-8' codec can't decode";
        assert_eq!(
            err(b"\xff"),
            CoreError::UnicodeDecode(format!("{prefix} byte 0xff in position 0: invalid utf-8"))
        );
        assert_eq!(
            err(b"a\xe2\x82").to_string(),
            format!("{prefix} bytes in position 1-2: invalid utf-8")
        );
        assert_eq!(
            err(b"\xe2(\xa1").to_string(),
            format!("{prefix} byte 0xe2 in position 0: invalid utf-8")
        );
        assert_eq!(
            err(b"ok\x81x").to_string(),
            format!("{prefix} byte 0x81 in position 2: invalid utf-8")
        );
    }
}
