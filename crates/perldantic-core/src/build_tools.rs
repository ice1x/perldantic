//! Schema access helpers. Port of upstream `build_tools.rs` and `tools::SchemaDict`.
//!
//! Schemas use pydantic's `core_schema` wire format: a dict with a `type` key plus
//! type-specific keys. Validators read them through [`SchemaDict`], as upstream does.

use std::str::FromStr;

use crate::core_error::{CoreError, CoreResult};
use crate::value::{Dict, Value};

/// Build a `CoreError::Schema` result, like upstream's `py_schema_err!`.
macro_rules! schema_err {
    ($($arg:tt)+) => {
        Err(crate::core_error::CoreError::Schema(format!($($arg)+)))
    };
}
pub(crate) use schema_err;

/// A Rust type that can be read from a schema value.
pub(crate) trait FromSchemaValue: Sized {
    /// How the expected type is described in error messages.
    const EXPECTED: &'static str;
    fn from_schema_value(value: &Value) -> Option<Self>;
}

impl FromSchemaValue for bool {
    const EXPECTED: &'static str = "a bool";
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl FromSchemaValue for String {
    const EXPECTED: &'static str = "a string";
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl FromSchemaValue for i64 {
    const EXPECTED: &'static str = "an int";
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
}

impl FromSchemaValue for usize {
    const EXPECTED: &'static str = "a non-negative int";
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::Int(i) => usize::try_from(*i).ok(),
            _ => None,
        }
    }
}

impl FromSchemaValue for f64 {
    const EXPECTED: &'static str = "a number";
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl FromSchemaValue for Value {
    const EXPECTED: &'static str = "any value";
    fn from_schema_value(value: &Value) -> Option<Self> {
        Some(value.clone())
    }
}

impl FromSchemaValue for Dict {
    const EXPECTED: &'static str = "a dict";
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::Dict(d) => Some(d.clone()),
            _ => None,
        }
    }
}

impl FromSchemaValue for Vec<Value> {
    const EXPECTED: &'static str = "a list";
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::List(items) | Value::Tuple(items) => Some(items.clone()),
            _ => None,
        }
    }
}

/// `None` in the schema reads as `Some(None)`: the key is present but explicitly unset.
impl<T: FromSchemaValue> FromSchemaValue for Option<T> {
    const EXPECTED: &'static str = T::EXPECTED;
    fn from_schema_value(value: &Value) -> Option<Self> {
        match value {
            Value::None => Some(None),
            other => T::from_schema_value(other).map(Some),
        }
    }
}

/// Typed access to schema and config dicts.
pub(crate) trait SchemaDict {
    fn get_as<T: FromSchemaValue>(&self, key: &str) -> CoreResult<Option<T>>;
    fn get_as_req<T: FromSchemaValue>(&self, key: &str) -> CoreResult<T>;
}

fn extract<T: FromSchemaValue>(key: &str, value: &Value) -> CoreResult<T> {
    T::from_schema_value(value).ok_or_else(|| {
        CoreError::Type(format!(
            "'{key}' should be {}, got {}",
            T::EXPECTED,
            value.type_name()
        ))
    })
}

impl SchemaDict for Dict {
    fn get_as<T: FromSchemaValue>(&self, key: &str) -> CoreResult<Option<T>> {
        self.get_str(key).map(|v| extract(key, v)).transpose()
    }

    fn get_as_req<T: FromSchemaValue>(&self, key: &str) -> CoreResult<T> {
        match self.get_str(key) {
            Some(v) => extract(key, v),
            None => Err(CoreError::Key(key.to_owned())),
        }
    }
}

impl SchemaDict for Option<&Dict> {
    fn get_as<T: FromSchemaValue>(&self, key: &str) -> CoreResult<Option<T>> {
        match self {
            Some(d) => d.get_as(key),
            None => Ok(None),
        }
    }

    fn get_as_req<T: FromSchemaValue>(&self, key: &str) -> CoreResult<T> {
        match self {
            Some(d) => d.get_as_req(key),
            None => Err(CoreError::Key(key.to_owned())),
        }
    }
}

/// Read `schema_key` from the schema, falling back to `config_key` in the config.
pub(crate) fn schema_or_config<T: FromSchemaValue>(
    schema: &Dict,
    config: Option<&Dict>,
    schema_key: &str,
    config_key: &str,
) -> CoreResult<Option<T>> {
    match schema.get_as(schema_key)? {
        Some(v) => Ok(Some(v)),
        None => config.get_as(config_key),
    }
}

pub(crate) fn schema_or_config_same<T: FromSchemaValue>(
    schema: &Dict,
    config: Option<&Dict>,
    key: &str,
) -> CoreResult<Option<T>> {
    schema_or_config(schema, config, key, key)
}

pub(crate) fn is_strict(schema: &Dict, config: Option<&Dict>) -> CoreResult<bool> {
    Ok(schema_or_config_same(schema, config, "strict")?.unwrap_or(false))
}

/// What to do with input keys that match no field.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExtraBehavior {
    Allow,
    Forbid,
    Ignore,
}

impl ExtraBehavior {
    pub(crate) fn from_schema_or_config(
        schema: &Dict,
        config: Option<&Dict>,
        default: Self,
    ) -> CoreResult<Self> {
        let extra_behavior = schema_or_config::<Option<String>>(
            schema,
            config,
            "extra_behavior",
            "extra_fields_behavior",
        )?
        .flatten();
        match extra_behavior {
            Some(s) => Self::from_str(&s),
            None => Ok(default),
        }
    }
}

impl FromStr for ExtraBehavior {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "allow" => Ok(Self::Allow),
            "forbid" => Ok(Self::Forbid),
            "ignore" => Ok(Self::Ignore),
            s => schema_err!("Invalid extra_behavior: `{s}`"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CoreError, Dict, Value};

    fn dict(json: &str) -> Dict {
        match Value::from_json(json).unwrap() {
            Value::Dict(d) => d,
            other => panic!("expected a dict, got {other:?}"),
        }
    }

    #[test]
    fn get_as_reads_typed_values() {
        let schema =
            dict(r#"{"strict": true, "max_length": 5, "pattern": "a+", "ge": 1.5, "items": [1]}"#);
        assert_eq!(schema.get_as::<bool>("strict").unwrap(), Some(true));
        assert_eq!(schema.get_as::<usize>("max_length").unwrap(), Some(5));
        assert_eq!(schema.get_as::<i64>("max_length").unwrap(), Some(5));
        assert_eq!(
            schema.get_as::<String>("pattern").unwrap(),
            Some("a+".to_owned())
        );
        assert_eq!(schema.get_as::<f64>("ge").unwrap(), Some(1.5));
        assert_eq!(schema.get_as::<f64>("max_length").unwrap(), Some(5.0));
        assert_eq!(
            schema.get_as::<Vec<Value>>("items").unwrap(),
            Some(vec![Value::Int(1)])
        );
        assert_eq!(schema.get_as::<bool>("missing").unwrap(), None);
    }

    #[test]
    fn get_as_rejects_wrong_types() {
        let schema = dict(r#"{"strict": "yes", "max_length": -1}"#);
        assert_eq!(
            schema.get_as::<bool>("strict").unwrap_err(),
            CoreError::Type("'strict' should be a bool, got str".into())
        );
        assert_eq!(
            schema.get_as::<usize>("max_length").unwrap_err(),
            CoreError::Type("'max_length' should be a non-negative int, got int".into())
        );
    }

    #[test]
    fn get_as_req_requires_the_key() {
        let schema = dict(r#"{"type": "int"}"#);
        assert_eq!(schema.get_as_req::<String>("type").unwrap(), "int");
        assert_eq!(
            schema.get_as_req::<String>("items_schema").unwrap_err(),
            CoreError::Key("items_schema".into())
        );
        let none: Option<&Dict> = None;
        assert_eq!(none.get_as::<bool>("strict").unwrap(), None);
        assert_eq!(
            none.get_as_req::<bool>("strict").unwrap_err(),
            CoreError::Key("strict".into())
        );
    }

    #[test]
    fn nested_dicts_are_extracted() {
        let schema = dict(r#"{"items_schema": {"type": "int"}}"#);
        let items = schema.get_as_req::<Dict>("items_schema").unwrap();
        assert_eq!(items.get_as_req::<String>("type").unwrap(), "int");
    }

    #[test]
    fn schema_value_takes_precedence_over_config() {
        let schema = dict(r#"{"strict": false}"#);
        let config = dict(r#"{"strict": true, "extra_fields_behavior": "forbid"}"#);
        assert_eq!(
            schema_or_config::<bool>(&schema, Some(&config), "strict", "strict").unwrap(),
            Some(false)
        );
        let empty = dict("{}");
        assert_eq!(
            schema_or_config_same::<bool>(&empty, Some(&config), "strict").unwrap(),
            Some(true)
        );
        assert_eq!(
            schema_or_config_same::<bool>(&empty, None, "strict").unwrap(),
            None
        );
    }

    #[test]
    fn is_strict_defaults_to_false() {
        assert!(!is_strict(&dict("{}"), None).unwrap());
        assert!(is_strict(&dict("{}"), Some(&dict(r#"{"strict": true}"#))).unwrap());
        assert!(is_strict(&dict(r#"{"strict": true}"#), None).unwrap());
    }

    #[test]
    fn extra_behavior_from_schema_or_config() {
        let config = dict(r#"{"extra_fields_behavior": "forbid"}"#);
        assert_eq!(
            ExtraBehavior::from_schema_or_config(&dict("{}"), Some(&config), ExtraBehavior::Ignore)
                .unwrap(),
            ExtraBehavior::Forbid
        );
        assert_eq!(
            ExtraBehavior::from_schema_or_config(
                &dict(r#"{"extra_behavior": "allow"}"#),
                Some(&config),
                ExtraBehavior::Ignore
            )
            .unwrap(),
            ExtraBehavior::Allow
        );
        assert_eq!(
            ExtraBehavior::from_schema_or_config(&dict("{}"), None, ExtraBehavior::Ignore).unwrap(),
            ExtraBehavior::Ignore
        );
        assert_eq!(
            ExtraBehavior::from_schema_or_config(
                &dict(r#"{"extra_behavior": "sometimes"}"#),
                None,
                ExtraBehavior::Ignore
            )
            .unwrap_err(),
            CoreError::Schema("Invalid extra_behavior: `sometimes`".into())
        );
    }
}
