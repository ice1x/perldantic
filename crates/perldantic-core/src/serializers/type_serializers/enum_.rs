//! `enum` serializer. Port of upstream `type_serializers/enum_.rs`: members of the schema's
//! class are kept in Python mode and serialized by their value in JSON mode.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Serializer;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::SerializationState;
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, EnumMember, Value};

use super::float::FloatSerializer;
use super::simple::IntSerializer;
use super::string::StrSerializer;

#[derive(Debug)]
pub struct EnumSerializer {
    class: String,
    /// The serializer of the mixed-in type, for `int`, `str` and `float` enums.
    serializer: Option<Arc<CombinedSerializer>>,
}

impl BuildSerializer for EnumSerializer {
    const EXPECTED_TYPE: &'static str = "enum";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let sub_type: Option<String> = schema.get_as("sub_type")?;
        let serializer = match sub_type.as_deref() {
            Some("int") => Some(IntSerializer::get().clone()),
            Some("str") => Some(StrSerializer::get().clone()),
            Some("float") => Some(FloatSerializer::get(config)?.clone()),
            Some(_) => {
                return schema_err!("`sub_type` must be one of: 'int', 'str', 'float' or None");
            }
            None => None,
        };
        Ok(Arc::new(
            Self {
                class: schema.get_as_req("cls")?,
                serializer,
            }
            .into(),
        ))
    }
}

impl EnumSerializer {
    /// The value as a member of this enum's class.
    fn member<'a>(&self, value: &'a Value) -> Option<&'a EnumMember> {
        match value {
            Value::Enum(member) if member.class == self.class => Some(member),
            _ => None,
        }
    }
}

impl TypeSerializer for EnumSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        let Some(member) = self.member(value) else {
            state.warn_fallback_py(self.get_name(), value)?;
            return infer_to_python(value, state);
        };
        if state.extra.mode.is_json() {
            match &self.serializer {
                Some(s) => s.to_python(&member.value, state),
                None => infer_to_python(&member.value, state),
            }
        } else {
            // outside JSON mode the member itself is safe to return
            Ok(value.clone())
        }
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        let Some(member) = self.member(key) else {
            state.warn_fallback_py(self.get_name(), key)?;
            return infer_json_key(key, state);
        };
        match &self.serializer {
            Some(s) => s.json_key(&member.value, state),
            None => infer_json_key(&member.value, state),
        }
    }

    fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        let Some(member) = self.member(value) else {
            state.warn_fallback_ser::<S>(self.get_name(), value)?;
            return infer_serialize(value, serializer, state);
        };
        match &self.serializer {
            Some(s) => s.serde_serialize(&member.value, serializer, state),
            None => infer_serialize(&member.value, serializer, state),
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }

    fn retry_with_lax_check(&self) -> bool {
        self.serializer
            .as_ref()
            .is_some_and(|s| s.retry_with_lax_check())
    }
}
