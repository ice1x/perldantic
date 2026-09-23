//! `literal` serializer. Port of upstream `type_serializers/literal.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Serialize;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerMode, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct LiteralSerializer {
    expected_int: Vec<i64>,
    expected_str: Vec<String>,
    expected_py: Option<Vec<Value>>,
    name: String,
}

impl BuildSerializer for LiteralSerializer {
    const EXPECTED_TYPE: &'static str = "literal";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let expected: Vec<Value> = schema.get_as_req("expected")?;
        if expected.is_empty() {
            return schema_err!("`expected` should have length > 0");
        }
        let mut expected_int = Vec::new();
        let mut expected_str = Vec::new();
        let mut expected_py = Vec::new();
        let mut repr_args: Vec<String> = Vec::new();
        for item in expected {
            repr_args.push(item.repr());
            match item {
                Value::Bool(_) => expected_py.push(item),
                Value::Int(int) => expected_int.push(int),
                Value::Str(s) => expected_str.push(s),
                other => expected_py.push(other),
            }
        }
        Ok(Arc::new(
            Self {
                expected_int,
                expected_str,
                expected_py: (!expected_py.is_empty()).then_some(expected_py),
                name: format!("{}[{}]", Self::EXPECTED_TYPE, repr_args.join(",")),
            }
            .into(),
        ))
    }
}

enum OutputValue<'a> {
    OkInt(i64),
    OkStr(&'a str),
    Ok,
    Fallback,
}

impl LiteralSerializer {
    fn check<'a>(&self, value: &'a Value, state: &SerializationState) -> OutputValue<'a> {
        if state.check.enabled() {
            if let Value::Int(int) = value
                && self.expected_int.contains(int)
            {
                return OutputValue::OkInt(*int);
            }
            if let Value::Str(s) = value
                && self.expected_str.iter().any(|e| e == s)
            {
                return OutputValue::OkStr(s);
            }
            if let Some(ref expected_py) = self.expected_py
                && expected_py.iter().any(|e| e.py_eq(value))
            {
                return OutputValue::Ok;
            }
            OutputValue::Fallback
        } else {
            OutputValue::Ok
        }
    }
}

impl TypeSerializer for LiteralSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match self.check(value, state) {
            OutputValue::OkInt(int) => match state.extra.mode {
                SerMode::Json => Ok(Value::Int(int)),
                _ => Ok(value.clone()),
            },
            OutputValue::OkStr(s) => match state.extra.mode {
                SerMode::Json => Ok(Value::from(s)),
                _ => Ok(value.clone()),
            },
            OutputValue::Ok => infer_to_python(value, state),
            OutputValue::Fallback => {
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
        match self.check(key, state) {
            OutputValue::OkInt(int) => Ok(Cow::Owned(int.to_string())),
            OutputValue::OkStr(s) => Ok(Cow::Borrowed(s)),
            OutputValue::Ok => infer_json_key(key, state),
            OutputValue::Fallback => {
                state.warn_fallback_py(self.get_name(), key)?;
                infer_json_key(key, state)
            }
        }
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        match self.check(value, state) {
            OutputValue::OkInt(int) => int.serialize(serializer),
            OutputValue::OkStr(s) => s.serialize(serializer),
            OutputValue::Ok => infer_serialize(value, serializer, state),
            OutputValue::Fallback => {
                state.warn_fallback_ser::<S>(self.get_name(), value)?;
                infer_serialize(value, serializer, state)
            }
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
