//! `url` and `multi-host-url` serializers. Port of upstream `type_serializers/url.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Serializer;

use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

macro_rules! build_url_serializer {
    ($Struct:ident, $expected_type:literal, $Variant:ident) => {
        #[derive(Debug)]
        pub struct $Struct;

        impl BuildSerializer for $Struct {
            const EXPECTED_TYPE: &'static str = $expected_type;

            fn build(
                _schema: &Dict,
                _config: Option<&Dict>,
                _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
            ) -> CoreResult<Arc<CombinedSerializer>> {
                Ok(Arc::new(Self.into()))
            }
        }

        impl TypeSerializer for $Struct {
            fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
                match value {
                    Value::$Variant(url) => match state.extra.mode {
                        SerMode::Json => Ok(Value::Str(url.as_str().into_owned())),
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
                    Value::$Variant(url) => Ok(Cow::Owned(url.as_str().into_owned())),
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
                    Value::$Variant(url) => serializer.serialize_str(&url.as_str()),
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

build_url_serializer!(UrlSerializer, "url", Url);
build_url_serializer!(MultiHostUrlSerializer, "multi-host-url", MultiHostUrl);
