//! Parsing JSON text into `Value`, the entry point for schemas and JSON input.

use num_bigint::BigInt;
use perldantic_core::{CoreError, Dict, Value};

#[test]
fn parses_scalars() {
    assert_eq!(Value::from_json("null").unwrap(), Value::None);
    assert_eq!(Value::from_json("true").unwrap(), Value::Bool(true));
    assert_eq!(Value::from_json("-12").unwrap(), Value::Int(-12));
    assert_eq!(Value::from_json("1.5").unwrap(), Value::Float(1.5));
    assert_eq!(Value::from_json("1e3").unwrap(), Value::Float(1000.0));
    assert_eq!(Value::from_json(r#""a\nb""#).unwrap(), Value::from("a\nb"));
}

#[test]
fn big_integers_stay_exact() {
    assert_eq!(
        Value::from_json("1180591620717411303424").unwrap(),
        Value::BigInt(BigInt::from(2).pow(70))
    );
}

#[test]
fn parses_containers_preserving_key_order() {
    let value =
        Value::from_json(r#"{"type": "list", "items_schema": {"type": "int"}, "a": [1, "x"]}"#)
            .unwrap();
    let Value::Dict(dict) = &value else {
        panic!("expected dict")
    };
    let keys: Vec<String> = dict.iter().map(|(k, _)| k.py_str()).collect();
    assert_eq!(keys, ["type", "items_schema", "a"]);
    let mut items = Dict::new();
    items.insert(Value::from("type"), Value::from("int"));
    assert_eq!(dict.get_str("items_schema"), Some(&Value::Dict(items)));
    assert_eq!(
        dict.get_str("a"),
        Some(&Value::List(vec![Value::Int(1), Value::from("x")]))
    );
}

#[test]
fn invalid_json_is_a_value_error_with_position() {
    let err = Value::from_json("{\"a\": }").unwrap_err();
    assert!(
        matches!(&err, CoreError::Value(m) if m.starts_with("Invalid JSON: ")),
        "{err:?}"
    );
    assert!(err.message().contains("line 1 column"), "{err:?}");
}
