//! `SchemaSerializer`: `to_python` / `to_json`, warnings, errors, filters and JSON formatting.
//! Expectations come from pydantic-core 2.49.0.

use perldantic_core::{
    JsonOptions, SchemaSerializer, SerMode, SerializeError, SerializeOptions, Serialized, Value,
    WarningsMode,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn serializer(schema: &str) -> SchemaSerializer {
    SchemaSerializer::new(&j(schema), None).unwrap()
}

fn with_config(schema: &str, config: &str) -> SchemaSerializer {
    SchemaSerializer::new(&j(schema), Some(&j(config))).unwrap()
}

fn opts() -> SerializeOptions {
    SerializeOptions::default()
}

fn json_mode() -> SerializeOptions {
    SerializeOptions {
        mode: SerMode::Json,
        ..opts()
    }
}

fn to_json(s: &SchemaSerializer, value: &Value) -> String {
    s.to_json(value, &opts(), &JsonOptions::default())
        .unwrap()
        .output
}

fn unexpected(expected: &str, repr: &str, type_: &str) -> String {
    format!(
        "PydanticSerializationUnexpectedValue(Expected `{expected}` - serialized value may not be as expected [input_value={repr}, input_type={type_}])"
    )
}

#[test]
fn values_of_the_wrong_type_are_serialized_with_a_warning() {
    let s = serializer(r#"{"type": "int"}"#);
    assert_eq!(
        s.to_python(&Value::from("x"), &opts()).unwrap(),
        Serialized {
            output: Value::from("x"),
            warning: Some(format!(
                "Pydantic serializer warnings:\n  {}",
                unexpected("int", "'x'", "str")
            )),
        }
    );

    let list = serializer(r#"{"type": "list", "items_schema": {"type": "int"}}"#);
    let out = list
        .to_json(&j(r#"[1, "a", 3]"#), &opts(), &JsonOptions::default())
        .unwrap();
    assert_eq!(out.output, r#"[1,"a",3]"#);
    assert_eq!(
        out.warning.unwrap(),
        format!(
            "Pydantic serializer warnings:\n  {}",
            unexpected("int", "'a'", "str")
        )
    );
}

#[test]
fn warnings_can_be_errors_or_silenced() {
    let s = serializer(r#"{"type": "int"}"#);
    let error = SerializeOptions {
        warnings: WarningsMode::Error,
        ..opts()
    };
    assert_eq!(
        s.to_python(&Value::from("x"), &error).unwrap_err(),
        SerializeError::Serialization(format!(
            "Pydantic serializer warnings:\n  {}",
            unexpected("int", "'x'", "str")
        ))
    );
    let none = SerializeOptions {
        warnings: WarningsMode::None,
        ..opts()
    };
    assert_eq!(s.to_python(&Value::from("x"), &none).unwrap().warning, None);
}

#[test]
fn json_mode_converts_to_json_types() {
    let tuple =
        serializer(r#"{"type": "tuple", "items_schema": [{"type": "int"}, {"type": "str"}]}"#);
    assert_eq!(
        tuple
            .to_python(
                &Value::Tuple(vec![Value::Int(1), Value::from("a")]),
                &json_mode()
            )
            .unwrap()
            .output,
        j(r#"[1, "a"]"#)
    );
    let set = serializer(r#"{"type": "set", "items_schema": {"type": "int"}}"#);
    assert_eq!(
        set.to_python(&Value::Set(vec![Value::Int(1)]), &json_mode())
            .unwrap()
            .output,
        j("[1]")
    );
    let keys = serializer(r#"{"type": "dict", "keys_schema": {"type": "int"}}"#);
    let mut input = perldantic_core::Dict::new();
    input.insert(Value::Int(1), Value::Int(2));
    assert_eq!(to_json(&keys, &Value::Dict(input)), r#"{"1":2}"#);
}

#[test]
fn inf_nan_and_bytes_modes_come_from_config() {
    let inf = Value::Float(f64::INFINITY);
    assert_eq!(to_json(&serializer(r#"{"type": "float"}"#), &inf), "null");
    assert_eq!(
        to_json(
            &with_config(r#"{"type": "float"}"#, r#"{"ser_json_inf_nan": "strings"}"#),
            &Value::Float(f64::NEG_INFINITY)
        ),
        r#""-Infinity""#
    );
    assert_eq!(
        to_json(
            &with_config(
                r#"{"type": "float"}"#,
                r#"{"ser_json_inf_nan": "constants"}"#
            ),
            &inf
        ),
        "Infinity"
    );
    assert_eq!(
        to_json(
            &with_config(r#"{"type": "bytes"}"#, r#"{"ser_json_bytes": "base64"}"#),
            &Value::Bytes(vec![0xfb, 0xff])
        ),
        r#""-_8=""#
    );
    assert_eq!(
        with_config(r#"{"type": "bytes"}"#, r#"{"ser_json_bytes": "hex"}"#)
            .to_python(&Value::Bytes(vec![1]), &json_mode())
            .unwrap()
            .output,
        Value::from("01")
    );
}

#[test]
fn json_formatting() {
    let any = serializer(r#"{"type": "any"}"#);
    let indented = JsonOptions {
        indent: Some(2),
        ..JsonOptions::default()
    };
    assert_eq!(
        any.to_json(&j(r#"{"a": [1, 2]}"#), &opts(), &indented)
            .unwrap()
            .output,
        "{\n  \"a\": [\n    1,\n    2\n  ]\n}"
    );
    let ascii = JsonOptions {
        ensure_ascii: true,
        ..JsonOptions::default()
    };
    assert_eq!(
        serializer(r#"{"type": "str"}"#)
            .to_json(&Value::from("é💩"), &opts(), &ascii)
            .unwrap()
            .output,
        r#""\u00e9\ud83d\udca9""#
    );
    // tuple keys join their items
    let mut input = perldantic_core::Dict::new();
    input.insert(
        Value::Tuple(vec![Value::Int(1), Value::from("a")]),
        Value::Int(2),
    );
    assert_eq!(to_json(&any, &Value::Dict(input)), r#"{"1,a":2}"#);
}

#[test]
fn include_and_exclude_filter_items() {
    let list = serializer(r#"{"type": "list"}"#);
    let include = SerializeOptions {
        include: Some(Value::Set(vec![Value::Int(0), Value::Int(-1)])),
        ..opts()
    };
    assert_eq!(
        list.to_python(&j("[1, 2, 3, 4]"), &include).unwrap().output,
        j("[1, 4]")
    );
    let exclude = SerializeOptions {
        exclude: Some(Value::Set(vec![Value::Int(1)])),
        ..opts()
    };
    assert_eq!(
        list.to_python(&j("[1, 2, 3]"), &exclude).unwrap().output,
        j("[1, 3]")
    );
    let dict = serializer(r#"{"type": "dict"}"#);
    let nested = SerializeOptions {
        exclude: Some(j(r#"{"b": {"d": true}}"#)),
        ..opts()
    };
    assert_eq!(
        dict.to_python(&j(r#"{"a": 1, "b": {"c": 2, "d": 3}}"#), &nested)
            .unwrap()
            .output,
        j(r#"{"a": 1, "b": {"c": 2}}"#)
    );
}

#[test]
fn unions_pick_the_matching_member() {
    let s = serializer(r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}]}"#);
    assert_eq!(
        s.to_python(&Value::from("x"), &opts()).unwrap(),
        Serialized {
            output: Value::from("x"),
            warning: None
        }
    );
    // No member matches: every member's complaint is reported.
    let out = s.to_python(&Value::Float(1.5), &opts()).unwrap();
    assert_eq!(out.output, Value::Float(1.5));
    assert_eq!(
        out.warning.unwrap(),
        format!(
            "Pydantic serializer warnings:\n  {}\n  {}",
            unexpected("int", "1.5", "float"),
            unexpected("str", "1.5", "float")
        )
    );
}

#[test]
fn schema_errors() {
    assert_eq!(
        SchemaSerializer::new(&j(r#"{"type": "bogus"}"#), None)
            .unwrap_err()
            .to_string(),
        "Unknown serialization schema type: `bogus`"
    );
    let err = serializer(r#"{"type": "bytes"}"#)
        .to_python(&Value::Bytes(vec![0xff]), &json_mode())
        .unwrap_err();
    assert_eq!(err.python_name(), "UnicodeDecodeError");
    assert_eq!(
        err.to_string(),
        "'utf-8' codec can't decode byte 0xff in position 0: invalid utf-8"
    );
}
