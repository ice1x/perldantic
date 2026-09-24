//! Part of upstream `type_serializers/format.rs`: `when_used`, which serializer functions share.
//! The `format` and `to-string` serializers themselves are not ported yet.

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::serializers::extra::Extra;
use crate::value::{Dict, Value};

/// When a custom serializer applies (a schema's `when_used`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhenUsed {
    Always,
    UnlessNone,
    Json,
    JsonUnlessNone,
}

impl WhenUsed {
    pub fn new(schema: &Dict, default: Self) -> CoreResult<Self> {
        let when_used: Option<String> = schema.get_as("when_used")?;
        match when_used.as_deref() {
            Some("always") => Ok(Self::Always),
            Some("unless-none") => Ok(Self::UnlessNone),
            Some("json") => Ok(Self::Json),
            Some("json-unless-none") => Ok(Self::JsonUnlessNone),
            Some(s) => schema_err!("Invalid value for `when_used`: {:?}", s),
            None => Ok(default),
        }
    }

    pub fn should_use(self, value: &Value, extra: &Extra) -> bool {
        match self {
            Self::Always => true,
            Self::UnlessNone => !matches!(value, Value::None),
            Self::Json => extra.mode.is_json(),
            Self::JsonUnlessNone => extra.mode.is_json() && !matches!(value, Value::None),
        }
    }
}
