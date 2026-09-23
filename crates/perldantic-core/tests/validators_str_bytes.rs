//! `str` and `bytes` validators (upstream validators/string.rs, validators/bytes.rs).
//!
//! Expectations come from the pydantic-core 2.49.0 wheel.

use perldantic_core::{
    CoreError, ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn build(schema: &str, config: Option<&str>) -> Result<SchemaValidator, CoreError> {
    let config = config.map(|c| Value::from_json(c).unwrap());
    SchemaValidator::new(&Value::from_json(schema).unwrap(), config.as_ref())
}

fn validator(schema: &str) -> SchemaValidator {
    build(schema, None).unwrap()
}

fn ok(v: &SchemaValidator, input: Value) -> Value {
    v.validate_value(&input, &ValidateOptions::default())
        .unwrap()
}

/// (type, msg, ctx) of the single error, plus the error title.
fn err(v: &SchemaValidator, input: Value) -> (String, String, Option<Value>, String) {
    let Err(ValidateError::Validation(e)) = v.validate_value(&input, &ValidateOptions::default())
    else {
        panic!("expected a validation error")
    };
    let details = e.errors(&ErrorsOptions::default());
    assert_eq!(details.len(), 1);
    let d = &details[0];
    (
        d.type_.clone(),
        d.msg.clone(),
        d.ctx.clone(),
        e.title().to_owned(),
    )
}

fn ctx(json: &str) -> Option<Value> {
    Some(Value::from_json(json).unwrap())
}

fn s(v: &str) -> Value {
    Value::from(v)
}

#[test]
fn plain_str() {
    let v = validator(r#"{"type": "str"}"#);
    assert_eq!(v.title(), "str");
    assert_eq!(ok(&v, Value::Bytes(b"ab".to_vec())), s("ab"));
    let v = validator(r#"{"type": "str", "coerce_numbers_to_str": true}"#);
    assert_eq!(v.title(), "str");
    assert_eq!(ok(&v, Value::Float(1.0)), s("1.0"));
}

#[test]
fn length_constraints_from_schema_or_config() {
    for v in [
        validator(r#"{"type": "str", "min_length": 3}"#),
        build(r#"{"type": "str"}"#, Some(r#"{"str_min_length": 3}"#)).unwrap(),
    ] {
        assert_eq!(
            err(&v, s("ab")),
            (
                "string_too_short".into(),
                "String should have at least 3 characters".into(),
                ctx(r#"{"min_length": 3}"#),
                "constrained-str".into()
            )
        );
    }
    // Length counts characters, not bytes.
    let v = validator(r#"{"type": "str", "max_length": 1}"#);
    assert_eq!(
        err(&v, s("\u{e9}\u{e9}")).1,
        "String should have at most 1 character"
    );
    let v = validator(r#"{"type": "str", "coerce_numbers_to_str": true, "max_length": 2}"#);
    assert_eq!(err(&v, Value::Int(123)).0, "string_too_long");
}

#[test]
fn pattern_and_transformations() {
    let v = validator(r#"{"type": "str", "pattern": "^\\d+$"}"#);
    assert_eq!(
        err(&v, s("12a")),
        (
            "string_pattern_mismatch".into(),
            "String should match pattern '^\\d+$'".into(),
            ctx(r#"{"pattern": "^\\d+$"}"#),
            "constrained-str".into()
        )
    );
    assert_eq!(ok(&v, s("123")), s("123"));
    let v = validator(r#"{"type": "str", "strip_whitespace": true, "to_upper": true}"#);
    assert_eq!(ok(&v, s("  ab ")), s("AB"));
    let v = validator(r#"{"type": "str", "to_lower": true}"#);
    assert_eq!(ok(&v, s("\u{c0}B")), s("\u{e0}b"));
    let v = validator(r#"{"type": "str", "ascii_only": true}"#);
    assert_eq!(
        err(&v, s("\u{e9}")),
        (
            "string_not_ascii".into(),
            "String should contain only ASCII characters".into(),
            None,
            "constrained-str".into()
        )
    );
}

#[test]
fn invalid_patterns_and_engines_are_schema_errors() {
    assert_eq!(
        build(r#"{"type": "str", "pattern": "("}"#, None).unwrap_err(),
        CoreError::Schema(
            "Error building \"str\" validator:\n  SchemaError: regex parse error:\n    (\n    ^\nerror: unclosed group".into()
        )
    );
    assert_eq!(
        build(
            r#"{"type": "str", "pattern": "a", "regex_engine": "nope"}"#,
            None
        )
        .unwrap_err(),
        CoreError::Schema(
            "Error building \"str\" validator:\n  SchemaError: Invalid regex engine: nope".into()
        )
    );
    // Python's `re` engine does not exist outside Python (docs/DIVERGENCES.md).
    assert_eq!(
        build(
            r#"{"type": "str", "pattern": "a", "regex_engine": "python-re"}"#,
            None
        )
        .unwrap_err(),
        CoreError::Schema(
            "Error building \"str\" validator:\n  SchemaError: Invalid regex engine: python-re"
                .into()
        )
    );
}

#[test]
fn bytes_constraints_and_modes() {
    let v = validator(r#"{"type": "bytes", "max_length": 2}"#);
    assert_eq!(
        err(&v, Value::Bytes(b"abc".to_vec())),
        (
            "bytes_too_long".into(),
            "Data should have at most 2 bytes".into(),
            ctx(r#"{"max_length": 2}"#),
            "constrained-bytes".into()
        )
    );
    let v = validator(r#"{"type": "bytes", "min_length": 2}"#);
    assert_eq!(err(&v, s("a")).1, "Data should have at least 2 bytes");
    let v = build(
        r#"{"type": "bytes"}"#,
        Some(r#"{"val_json_bytes": "base64"}"#),
    )
    .unwrap();
    assert_eq!(v.title(), "bytes");
    assert_eq!(
        v.validate_json(r#""YWI=""#, &ValidateOptions::default())
            .unwrap(),
        Value::Bytes(b"ab".to_vec())
    );
}
