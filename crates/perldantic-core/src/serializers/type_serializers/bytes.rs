//! `bytes` serializer. Port of upstream `type_serializers/bytes.rs`.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::config::BytesMode;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct BytesSerializer {
    bytes_mode: BytesMode,
}

static BYTES_SERIALIZER_UTF8: LazyLock<Arc<CombinedSerializer>> = LazyLock::new(|| {
    Arc::new(
        BytesSerializer {
            bytes_mode: BytesMode::Utf8,
        }
        .into(),
    )
});

static BYTES_SERIALIZER_BASE64: LazyLock<Arc<CombinedSerializer>> = LazyLock::new(|| {
    Arc::new(
        BytesSerializer {
            bytes_mode: BytesMode::Base64,
        }
        .into(),
    )
});

static BYTES_SERIALIZER_HEX: LazyLock<Arc<CombinedSerializer>> = LazyLock::new(|| {
    Arc::new(
        BytesSerializer {
            bytes_mode: BytesMode::Hex,
        }
        .into(),
    )
});

impl BuildSerializer for BytesSerializer {
    const EXPECTED_TYPE: &'static str = "bytes";

    fn build(
        _schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        match BytesMode::from_config(config)? {
            BytesMode::Utf8 => Ok(BYTES_SERIALIZER_UTF8.clone()),
            BytesMode::Base64 => Ok(BYTES_SERIALIZER_BASE64.clone()),
            BytesMode::Hex => Ok(BYTES_SERIALIZER_HEX.clone()),
        }
    }
}

/// The octets of `bytes`, or of a `bytes` subclass instance (a member of a bytes enum).
fn as_bytes(value: &Value) -> Option<&[u8]> {
    match value.mixin_value().unwrap_or(value) {
        Value::Bytes(bytes) => Some(bytes),
        _ => None,
    }
}

impl TypeSerializer for BytesSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match as_bytes(value) {
            Some(bytes) => match state.extra.mode {
                SerMode::Json => Ok(Value::Str(
                    self.bytes_mode.bytes_to_string(bytes)?.into_owned(),
                )),
                _ => Ok(value.clone()),
            },
            None => {
                state.warn_fallback_py(self.get_name(), value)?;
                infer_to_python(value, state)
            }
        }
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        match as_bytes(key) {
            Some(bytes) => Ok(self.bytes_mode.bytes_to_string(bytes)?),
            None => {
                state.warn_fallback_py(self.get_name(), key)?;
                infer_json_key(key, state)
            }
        }
    }

    fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        match as_bytes(value) {
            Some(bytes) => self.bytes_mode.serialize_bytes(bytes, serializer),
            None => {
                state.warn_fallback_ser::<S>(self.get_name(), value)?;
                infer_serialize(value, serializer, state)
            }
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
