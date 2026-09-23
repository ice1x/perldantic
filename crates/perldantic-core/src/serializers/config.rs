//! Serialization settings read from the core config. Port of upstream `serializers/config.rs`
//! for the P0 types; temporal modes come with the date and time types.

use std::borrow::Cow;
use std::str::{FromStr, from_utf8};

use base64::Engine;
use serde::ser::Error;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::{CoreError, CoreResult};
use crate::value::Dict;

/// Settings that apply to a whole serializer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SerializationConfig {
    pub bytes_mode: BytesMode,
    pub inf_nan_mode: InfNanMode,
}

impl Default for SerializationConfig {
    fn default() -> Self {
        Self {
            bytes_mode: BytesMode::default(),
            inf_nan_mode: InfNanMode::Constants,
        }
    }
}

impl SerializationConfig {
    pub fn from_config(config: Option<&Dict>) -> CoreResult<Self> {
        Ok(Self {
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
