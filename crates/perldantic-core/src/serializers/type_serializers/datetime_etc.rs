//! `datetime`, `date` and `time` serializers. Port of upstream
//! `type_serializers/datetime_etc.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::{Serialize, Serializer};

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::config::TemporalMode;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

/// Serialize a temporal value in `mode`; floats keep serde's float formatting.
pub(crate) fn serialize_temporal<S: Serializer>(
    mode: TemporalMode,
    value: &Value,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match mode.to_json(value) {
        Some(Value::Float(f)) => serializer.serialize_f64(f),
        Some(json) => json.serialize(serializer),
        None => value.serialize(serializer),
    }
}

macro_rules! build_temporal_serializer {
    ($Struct:ident, $expected_type:literal, $pattern:pat) => {
        #[derive(Debug)]
        pub struct $Struct {
            temporal_mode: TemporalMode,
        }

        impl BuildSerializer for $Struct {
            const EXPECTED_TYPE: &'static str = $expected_type;

            fn build(
                _schema: &Dict,
                config: Option<&Dict>,
                _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
            ) -> CoreResult<Arc<CombinedSerializer>> {
                let temporal_mode = TemporalMode::from_config(config)?;
                Ok(Arc::new(Self { temporal_mode }.into()))
            }
        }

        impl TypeSerializer for $Struct {
            fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
                match value {
                    $pattern => match state.extra.mode {
                        SerMode::Json => Ok(self
                            .temporal_mode
                            .to_json(value)
                            .unwrap_or_else(|| value.clone())),
                        _ => Ok(value.clone()),
                    },
                    _ => {
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
                match key {
                    $pattern => Ok(Cow::Owned(
                        self.temporal_mode
                            .json_key(key)
                            .unwrap_or_else(|| key.py_str()),
                    )),
                    _ => {
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
                match value {
                    $pattern => serialize_temporal(self.temporal_mode, value, serializer),
                    _ => {
                        state.warn_fallback_ser::<S>(self.get_name(), value)?;
                        infer_serialize(value, serializer, state)
                    }
                }
            }

            fn get_name(&self) -> &str {
                Self::EXPECTED_TYPE
            }
        }
    };
}

build_temporal_serializer!(DatetimeSerializer, "datetime", Value::DateTime(_));
// a datetime is a date in Python, but upstream refuses it here to avoid lossy serialization
build_temporal_serializer!(DateSerializer, "date", Value::Date(_));
build_temporal_serializer!(TimeSerializer, "time", Value::Time(_));
