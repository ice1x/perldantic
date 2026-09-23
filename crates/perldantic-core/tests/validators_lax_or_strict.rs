//! `lax-or-strict` validator (upstream validators/lax_or_strict.rs): one schema for lax mode,
//! another for strict mode. Expectations come from pydantic-core 2.49.0.

use perldantic_core::{ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value};

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&Value::from_json(schema).unwrap(), None).unwrap()
}

fn opts(strict: Option<bool>) -> ValidateOptions {
    ValidateOptions {
        strict,
        ..ValidateOptions::default()
    }
}

fn error_type(result: Result<Value, ValidateError>) -> String {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    e.errors(&ErrorsOptions::default())[0].type_.clone()
}

const LAX_STR_STRICT_INT: &str =
    r#"{"type": "lax-or-strict", "lax_schema": {"type": "str"}, "strict_schema": {"type": "int"}}"#;

#[test]
fn the_mode_picks_the_schema() {
    let v = validator(LAX_STR_STRICT_INT);
    assert_eq!(v.title(), "lax-or-strict[lax=str,strict=int]");
    assert_eq!(
        error_type(v.validate_value(&Value::Int(1), &opts(None))),
        "string_type"
    );
    assert_eq!(
        v.validate_value(&Value::from("1"), &opts(None)).unwrap(),
        Value::from("1")
    );
    assert_eq!(
        v.validate_value(&Value::Int(1), &opts(Some(true))).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        error_type(v.validate_value(&Value::from("1"), &opts(Some(true)))),
        "int_type"
    );
}

#[test]
fn a_strict_schema_uses_the_strict_side_unless_the_call_says_otherwise() {
    let v = validator(
        r#"{"type": "lax-or-strict", "lax_schema": {"type": "str"}, "strict_schema": {"type": "int"}, "strict": true}"#,
    );
    // The strict side is an ordinary int schema, so it still coerces.
    assert_eq!(
        v.validate_value(&Value::from("1"), &opts(None)).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        error_type(v.validate_value(&Value::Int(1), &opts(Some(false)))),
        "string_type"
    );
}

#[test]
fn in_a_smart_union_the_strict_side_is_preferred() {
    let v = validator(&format!(
        r#"{{"type": "union", "choices": [{LAX_STR_STRICT_INT}, {{"type": "float"}}]}}"#
    ));
    // The strict int side matches exactly, beating the lax float.
    assert_eq!(
        v.validate_value(&Value::Int(1), &opts(None)).unwrap(),
        Value::Int(1)
    );
    // The strict side is an ordinary int schema, so it also takes "1".
    assert_eq!(
        v.validate_value(&Value::from("1"), &opts(None)).unwrap(),
        Value::Int(1)
    );
    // No strict match: the lax side is tried.
    assert_eq!(
        v.validate_value(&Value::from("x"), &opts(None)).unwrap(),
        Value::from("x")
    );
}
