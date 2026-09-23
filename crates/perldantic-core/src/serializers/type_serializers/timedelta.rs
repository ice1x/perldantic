//! `timedelta` serializer. Port of upstream `type_serializers/timedelta.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::config::{TemporalMode, TimedeltaMode};
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

use super::datetime_etc::serialize_temporal;

#[derive(Debug)]
pub struct TimeDeltaSerializer {
    temporal_mode: TemporalMode,
}

impl BuildSerializer for TimeDeltaSerializer {
    const EXPECTED_TYPE: &'static str = "timedelta";

    fn build(
        _schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        // `ser_json_temporal` wins over the older `ser_json_timedelta` when it is set
        let temporal_set = config.is_some_and(|c| c.get_str("ser_json_temporal").is_some());
        let temporal_mode = if temporal_set {
            TemporalMode::from_config(config)?
        } else {
            TimedeltaMode::from_config(config)?.into()
        };
        Ok(Arc::new(Self { temporal_mode }.into()))
    }
}

impl TypeSerializer for TimeDeltaSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match value {
            Value::TimeDelta(_) => match state.extra.mode {
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
            Value::TimeDelta(_) => Ok(Cow::Owned(
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
            Value::TimeDelta(_) => serialize_temporal(self.temporal_mode, value, serializer),
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
