//! Keyword options of the calls, passed as a wire JSON object with pydantic's argument names.

use perldantic_core::{
    CoreError, CoreResult, Dict, ExtraBehavior, InputType, JsonOptions, JsonSchemaMode,
    JsonSchemaOptions, PartialMode, SerMode, SerializeOptions, UnionFormat, ValidateOptions, Value,
    WarningsMode,
};

fn unexpected(key: &str) -> CoreError {
    CoreError::Type(format!("got an unexpected keyword argument '{key}'"))
}

fn wrong_type(key: &str, expected: &str, value: &Value) -> CoreError {
    CoreError::Type(format!(
        "'{key}' should be {expected}, got {}",
        value.type_name()
    ))
}

fn entries(options: &Dict) -> impl Iterator<Item = (&str, &Value)> {
    options.iter().map(|(k, v)| match k {
        Value::Str(k) => (k.as_str(), v),
        _ => ("", v),
    })
}

fn flag(key: &str, value: &Value) -> CoreResult<bool> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(wrong_type(key, "a bool", other)),
    }
}

fn optional_flag(key: &str, value: &Value) -> CoreResult<Option<bool>> {
    match value {
        Value::None => Ok(None),
        other => flag(key, other).map(Some),
    }
}

fn optional(value: &Value) -> Option<Value> {
    (!matches!(value, Value::None)).then(|| value.clone())
}

/// Options of `validate_python` / `validate_json`.
pub fn validate_options(options: &Dict) -> CoreResult<ValidateOptions> {
    let mut opts = ValidateOptions::default();
    for (key, value) in entries(options) {
        match key {
            "strict" => opts.strict = optional_flag(key, value)?,
            "extra" => {
                opts.extra_behavior = match value {
                    Value::None => None,
                    Value::Str(s) => Some(s.parse::<ExtraBehavior>()?),
                    other => return Err(wrong_type(key, "a string", other)),
                };
            }
            "from_attributes" => opts.from_attributes = optional_flag(key, value)?,
            "context" => opts.context = optional(value),
            "allow_partial" => {
                opts.allow_partial = match value {
                    Value::Bool(b) => PartialMode::from(*b),
                    Value::Str(s) if s == "off" => PartialMode::Off,
                    Value::Str(s) if s == "on" => PartialMode::On,
                    Value::Str(s) if s == "trailing-strings" => PartialMode::TrailingStrings,
                    other => {
                        return Err(wrong_type(
                            key,
                            "a bool, 'off', 'on' or 'trailing-strings'",
                            other,
                        ));
                    }
                };
            }
            "by_alias" => opts.by_alias = optional_flag(key, value)?,
            "by_name" => opts.by_name = optional_flag(key, value)?,
            other => return Err(unexpected(other)),
        }
    }
    Ok(opts)
}

/// Options of host-data validation: those of `validate_python`, plus `input_type`, which
/// selects how the data is read: `"perl"` (the default; Perl words in messages, arrays as
/// strict tuples) or `"python"` (exactly pydantic's `validate_python`).
pub fn host_validate_options(options: &Dict) -> CoreResult<(ValidateOptions, InputType)> {
    let mut rest = Dict::new();
    let mut input_type = InputType::Perl;
    for (key, value) in options.iter() {
        match (key, value) {
            (Value::Str(k), Value::Str(s))
                if k == "input_type" && (s == "perl" || s == "python") =>
            {
                input_type = InputType::try_from(s.as_str())?;
            }
            (Value::Str(k), other) if k == "input_type" => {
                return Err(CoreError::Value(format!(
                    "invalid value for 'input_type': {}",
                    other.repr()
                )));
            }
            _ => rest.insert(key.clone(), value.clone()),
        }
    }
    Ok((validate_options(&rest)?, input_type))
}

/// Options of `to_python` / `to_json`.
pub fn serialize_options(options: &Dict) -> CoreResult<(SerializeOptions, JsonOptions)> {
    let mut opts = SerializeOptions {
        // the host is Perl unless the caller says otherwise
        input_type: InputType::Perl,
        ..SerializeOptions::default()
    };
    let mut json = JsonOptions::default();
    for (key, value) in entries(options) {
        match key {
            "mode" => {
                opts.mode = match value {
                    Value::None => SerMode::Python,
                    Value::Str(s) => SerMode::from(Some(s.as_str())),
                    other => return Err(wrong_type(key, "a string", other)),
                };
            }
            "include" => opts.include = optional(value),
            "exclude" => opts.exclude = optional(value),
            "by_alias" => opts.by_alias = optional_flag(key, value)?,
            "exclude_unset" => opts.exclude_unset = flag(key, value)?,
            "exclude_defaults" => opts.exclude_defaults = flag(key, value)?,
            "exclude_none" => opts.exclude_none = flag(key, value)?,
            "serialize_as_any" => opts.serialize_as_any = flag(key, value)?,
            "context" => opts.context = optional(value),
            "input_type" => {
                opts.input_type = match value {
                    Value::Str(s) if s == "perl" || s == "python" => {
                        InputType::try_from(s.as_str())?
                    }
                    other => return Err(wrong_type(key, "'perl' or 'python'", other)),
                };
            }
            "warnings" => {
                opts.warnings = match value {
                    Value::Bool(b) => WarningsMode::from(*b),
                    Value::Str(s) if s == "none" => WarningsMode::None,
                    Value::Str(s) if s == "warn" => WarningsMode::Warn,
                    Value::Str(s) if s == "error" => WarningsMode::Error,
                    other => {
                        return Err(wrong_type(key, "a bool, 'none', 'warn' or 'error'", other));
                    }
                };
            }
            "indent" => {
                json.indent = match value {
                    Value::None => None,
                    Value::Int(i) if *i >= 0 => Some(usize::try_from(*i).expect("non-negative")),
                    other => return Err(wrong_type(key, "a non-negative int", other)),
                };
            }
            "ensure_ascii" => json.ensure_ascii = flag(key, value)?,
            other => return Err(unexpected(other)),
        }
    }
    Ok((opts, json))
}

/// Options of JSON Schema generation (`mode` and the `GenerateJsonSchema` arguments).
pub fn json_schema_options(options: &Dict) -> CoreResult<JsonSchemaOptions> {
    let mut opts = JsonSchemaOptions::default();
    for (key, value) in entries(options) {
        match (key, value) {
            ("mode", Value::Str(s)) if s == "validation" => opts.mode = JsonSchemaMode::Validation,
            ("mode", Value::Str(s)) if s == "serialization" => {
                opts.mode = JsonSchemaMode::Serialization;
            }
            ("by_alias", value) => opts.by_alias = flag(key, value)?,
            ("ref_template", Value::Str(s)) => opts.ref_template.clone_from(s),
            ("union_format", Value::Str(s)) if s == "any_of" => {
                opts.union_format = UnionFormat::AnyOf;
            }
            ("union_format", Value::Str(s)) if s == "primitive_type_array" => {
                opts.union_format = UnionFormat::PrimitiveTypeArray;
            }
            ("mode" | "ref_template" | "union_format", other) => {
                return Err(CoreError::Value(format!(
                    "invalid value for '{key}': {}",
                    other.repr()
                )));
            }
            (other, _) => return Err(unexpected(other)),
        }
    }
    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(json: &str) -> Dict {
        match crate::wire::decode(json).unwrap() {
            Value::Dict(d) => d,
            other => panic!("not a dict: {other:?}"),
        }
    }

    #[test]
    fn validate_options_use_pydantic_names() {
        let opts = validate_options(&dict(
            r#"{"strict": true, "extra": "forbid", "context": {"$tuple": [1]},
                "allow_partial": "trailing-strings", "by_alias": null, "by_name": false,
                "from_attributes": true}"#,
        ))
        .unwrap();
        assert_eq!(opts.strict, Some(true));
        assert_eq!(opts.extra_behavior, Some(ExtraBehavior::Forbid));
        assert_eq!(opts.context, Some(Value::Tuple(vec![Value::Int(1)])));
        assert!(matches!(opts.allow_partial, PartialMode::TrailingStrings));
        assert_eq!(opts.by_alias, None);
        assert_eq!(opts.by_name, Some(false));
        assert_eq!(opts.from_attributes, Some(true));
    }

    #[test]
    fn host_options_choose_the_input_type() {
        let (opts, input_type) = host_validate_options(&dict(r#"{"strict": true}"#)).unwrap();
        assert_eq!((opts.strict, input_type), (Some(true), InputType::Perl));
        let (_, input_type) = host_validate_options(&dict(r#"{"input_type": "python"}"#)).unwrap();
        assert_eq!(input_type, InputType::Python);
        assert_eq!(
            host_validate_options(&dict(r#"{"input_type": "json"}"#))
                .unwrap_err()
                .to_string(),
            "invalid value for 'input_type': 'json'"
        );
    }

    #[test]
    fn serializers_treat_data_as_perl_by_default() {
        let (opts, _) = serialize_options(&dict("{}")).unwrap();
        assert_eq!(opts.input_type, InputType::Perl);
        let (opts, _) = serialize_options(&dict(r#"{"input_type": "python"}"#)).unwrap();
        assert_eq!(opts.input_type, InputType::Python);
        assert!(serialize_options(&dict(r#"{"input_type": 1}"#)).is_err());
    }

    #[test]
    fn serialize_options_use_pydantic_names() {
        let (opts, json) = serialize_options(&dict(
            r#"{"mode": "json", "include": {"$set": ["a"]}, "exclude": null, "by_alias": true,
                "exclude_unset": true, "exclude_defaults": true, "exclude_none": true,
                "serialize_as_any": true, "warnings": "error", "indent": 2, "ensure_ascii": true,
                "context": {"a": 1}}"#,
        ))
        .unwrap();
        assert_eq!(opts.mode, SerMode::Json);
        assert_eq!(opts.include, Some(Value::Set(vec![Value::from("a")])));
        assert_eq!(opts.exclude, None);
        assert_eq!(opts.by_alias, Some(true));
        assert!(opts.exclude_unset && opts.exclude_defaults && opts.exclude_none);
        assert!(opts.serialize_as_any);
        assert_eq!(opts.context, Some(Value::from_json(r#"{"a": 1}"#).unwrap()));
        assert_eq!(opts.warnings, WarningsMode::Error);
        assert_eq!(json.indent, Some(2));
        assert!(json.ensure_ascii);
    }

    #[test]
    fn json_schema_options_use_pydantic_names() {
        let opts = json_schema_options(&dict(
            r##"{"mode": "serialization", "by_alias": false, "ref_template": "#/d/{model}",
                "union_format": "primitive_type_array"}"##,
        ))
        .unwrap();
        assert_eq!(opts.mode, JsonSchemaMode::Serialization);
        assert!(!opts.by_alias);
        assert_eq!(opts.ref_template, "#/d/{model}");
        assert_eq!(opts.union_format, UnionFormat::PrimitiveTypeArray);
    }

    #[test]
    fn bad_options_are_errors() {
        assert_eq!(
            validate_options(&dict(r#"{"strictly": true}"#))
                .unwrap_err()
                .to_string(),
            "got an unexpected keyword argument 'strictly'"
        );
        assert_eq!(
            validate_options(&dict(r#"{"strict": 1}"#))
                .unwrap_err()
                .to_string(),
            "'strict' should be a bool, got int"
        );
        assert_eq!(
            serialize_options(&dict(r#"{"warnings": 1}"#))
                .unwrap_err()
                .to_string(),
            "'warnings' should be a bool, 'none', 'warn' or 'error', got int"
        );
        assert_eq!(
            json_schema_options(&dict(r#"{"mode": "x"}"#))
                .unwrap_err()
                .to_string(),
            "invalid value for 'mode': 'x'"
        );
    }
}
