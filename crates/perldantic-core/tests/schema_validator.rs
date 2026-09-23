//! The public entry point: build a validator from a core schema, validate host data or JSON.
//!
//! Expected messages come from the pydantic-core 2.49.0 wheel.

use perldantic_core::{
    CoreError, ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn schema(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn any() -> SchemaValidator {
    SchemaValidator::new(&schema(r#"{"type": "any"}"#), None).unwrap()
}

fn validation_error(result: Result<Value, ValidateError>) -> perldantic_core::ValidationError {
    match result {
        Err(ValidateError::Validation(e)) => e,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[test]
fn schema_must_be_a_dict_with_a_known_type() {
    let cases = [
        (
            "null",
            CoreError::Type("'None' is not an instance of 'dict'".into()),
        ),
        (
            "1",
            CoreError::Type("'int' object is not an instance of 'dict'".into()),
        ),
        ("{}", CoreError::Key("type".into())),
        (
            r#"{"type": "foo"}"#,
            CoreError::Schema("Unknown schema type: \"foo\"".into()),
        ),
        (
            r#"{"type": "invalid"}"#,
            CoreError::Schema("Cannot construct schema with `InvalidSchema` member.".into()),
        ),
    ];
    for (json, expected) in cases {
        assert_eq!(
            SchemaValidator::new(&schema(json), None).unwrap_err(),
            expected,
            "schema {json}"
        );
    }
}

#[test]
fn config_must_be_a_dict() {
    assert_eq!(
        SchemaValidator::new(&schema(r#"{"type": "any"}"#), Some(&Value::Int(1))).unwrap_err(),
        CoreError::Type("'int' object is not an instance of 'dict'".into())
    );
}

#[test]
fn any_returns_the_input() {
    let v = any();
    let input = Value::Tuple(vec![Value::Int(1), Value::Bytes(b"x".to_vec())]);
    assert_eq!(
        v.validate_value(&input, &ValidateOptions::default())
            .unwrap(),
        input
    );
}

#[test]
fn json_input_is_parsed_with_inf_and_nan() {
    let v = any();
    let opts = ValidateOptions::default();
    assert_eq!(
        v.validate_json(r#"[1, {"a": null}]"#, &opts).unwrap(),
        schema(r#"[1, {"a": null}]"#)
    );
    assert_eq!(
        v.validate_json("Infinity", &opts).unwrap(),
        Value::Float(f64::INFINITY)
    );
    let Value::Dict(d) = v.validate_json(r#"{"a": NaN}"#, &opts).unwrap() else {
        panic!("expected a dict")
    };
    assert!(matches!(d.get_str("a"), Some(Value::Float(f)) if f.is_nan()));
}

#[test]
fn invalid_json_is_a_json_invalid_error() {
    let v = any();
    let error = validation_error(v.validate_json("[1,", &ValidateOptions::default()));
    assert_eq!(error.title(), "any");
    let details = error.errors(&ErrorsOptions {
        include_url: false,
        ..ErrorsOptions::default()
    });
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].type_, "json_invalid");
    assert_eq!(
        details[0].msg,
        "Invalid JSON: EOF while parsing a value at line 1 column 3"
    );
    assert_eq!(details[0].input, Some(Value::from("[1,")));
}

#[test]
fn config_sets_title_and_hides_input() {
    let v = SchemaValidator::new(
        &schema(r#"{"type": "any"}"#),
        Some(&schema(r#"{"title": "T", "hide_input_in_errors": true}"#)),
    )
    .unwrap();
    assert_eq!(v.title(), "T");
    let error = validation_error(v.validate_json("x", &ValidateOptions::default()));
    // `Display` honours the config's hide_input_in_errors.
    assert_eq!(
        error.to_string(),
        "1 validation error for T\n  Invalid JSON: expected value at line 1 column 1 [type=json_invalid]\n    For further information visit https://errors.pydantic.dev/latest/v/json_invalid"
    );
}

#[test]
fn registry_lists_supported_schema_types() {
    assert!(SchemaValidator::supported_schema_types().contains(&"any"));
    assert!(!SchemaValidator::supported_schema_types().contains(&"foo"));
}
