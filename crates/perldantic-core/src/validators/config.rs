//! Validation settings read from the core config. Port of the P0 parts of upstream
//! `validators/config.rs`.

use std::str::FromStr;

use base64::engine::general_purpose::GeneralPurpose;
use base64::engine::{DecodePaddingMode, GeneralPurposeConfig};
use base64::{DecodeError, Engine, alphabet};

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::errors::ErrorType;
use crate::input::EitherBytes;
use crate::value::Dict;

const URL_SAFE_OPTIONAL_PADDING: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);
const STANDARD_OPTIONAL_PADDING: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

pub(crate) use crate::serializers::config::BytesMode;

/// How numbers are read as timestamps (`val_temporal_unit` config).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalUnitMode {
    Seconds,
    Milliseconds,
    #[default]
    Infer,
}

impl FromStr for TemporalUnitMode {
    type Err = crate::core_error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "seconds" => Ok(Self::Seconds),
            "milliseconds" => Ok(Self::Milliseconds),
            "infer" => Ok(Self::Infer),
            s => Err(crate::core_error::CoreError::Schema(format!(
                "Invalid temporal_unit_mode serialization mode: `{s}`, expected seconds, milliseconds or infer"
            ))),
        }
    }
}

impl TemporalUnitMode {
    pub fn from_config(config: Option<&Dict>) -> CoreResult<Self> {
        match config.get_as::<String>("val_temporal_unit")? {
            Some(raw) => Self::from_str(&raw),
            None => Ok(Self::default()),
        }
    }
}

impl From<TemporalUnitMode> for speedate::TimestampUnit {
    fn from(value: TemporalUnitMode) -> Self {
        match value {
            TemporalUnitMode::Seconds => Self::Second,
            TemporalUnitMode::Milliseconds => Self::Millisecond,
            TemporalUnitMode::Infer => Self::Infer,
        }
    }
}

/// How strings are decoded into bytes during validation (`val_json_bytes` config).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValBytesMode {
    pub ser: BytesMode,
}

impl ValBytesMode {
    pub fn from_config(config: Option<&Dict>) -> CoreResult<Self> {
        let ser = match config.get_as::<String>("val_json_bytes")? {
            Some(raw) => BytesMode::from_str(&raw)?,
            None => BytesMode::default(),
        };
        Ok(Self { ser })
    }

    pub fn deserialize_string(self, s: &str) -> Result<EitherBytes<'_>, ErrorType> {
        match self.ser {
            BytesMode::Utf8 => Ok(EitherBytes::from(s.as_bytes())),
            BytesMode::Base64 => URL_SAFE_OPTIONAL_PADDING
                .decode(s)
                .or_else(|err| match err {
                    DecodeError::InvalidByte(_, b'/' | b'+') => STANDARD_OPTIONAL_PADDING.decode(s),
                    _ => Err(err),
                })
                .map(EitherBytes::from)
                .map_err(|err| ErrorType::BytesInvalidEncoding {
                    encoding: "base64".to_string(),
                    encoding_error: err.to_string(),
                    context: None,
                }),
            BytesMode::Hex => match hex::decode(s) {
                Ok(vec) => Ok(EitherBytes::from(vec)),
                Err(err) => Err(ErrorType::BytesInvalidEncoding {
                    encoding: "hex".to_string(),
                    encoding_error: err.to_string(),
                    context: None,
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use crate::core_error::CoreError;

    fn config(json: &str) -> Dict {
        match Value::from_json(json).unwrap() {
            Value::Dict(d) => d,
            other => panic!("expected dict, got {other:?}"),
        }
    }

    #[test]
    fn mode_from_config() {
        assert_eq!(
            ValBytesMode::from_config(None).unwrap().ser,
            BytesMode::Utf8
        );
        assert_eq!(
            ValBytesMode::from_config(Some(&config(r#"{"val_json_bytes": "base64"}"#)))
                .unwrap()
                .ser,
            BytesMode::Base64
        );
        assert_eq!(
            ValBytesMode::from_config(Some(&config(r#"{"val_json_bytes": "hex"}"#)))
                .unwrap()
                .ser,
            BytesMode::Hex
        );
        assert_eq!(
            ValBytesMode::from_config(Some(&config(r#"{"val_json_bytes": "rot13"}"#))).unwrap_err(),
            CoreError::Schema(
                "Invalid BytesMode serialization mode: `rot13`, expected utf8 or base64 or hex or "
                    .into()
            )
        );
    }

    #[test]
    fn invalid_base64_reports_the_decoder_error() {
        let err = ValBytesMode {
            ser: BytesMode::Base64,
        }
        .deserialize_string("a")
        .unwrap_err();
        assert_eq!(err.type_string(), "bytes_invalid_encoding");
    }
}
