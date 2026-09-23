//! `int` and `float` validators (upstream validators/int.rs, validators/float.rs).
//!
//! Expectations come from the pydantic-core 2.49.0 wheel.

use num_bigint::BigInt;
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

fn big() -> BigInt {
    BigInt::from(2).pow(70)
}

#[test]
fn int_plain_and_big() {
    let v = validator(r#"{"type": "int"}"#);
    assert_eq!(v.title(), "int");
    assert_eq!(ok(&v, Value::from("12")), Value::Int(12));
    assert_eq!(ok(&v, Value::BigInt(big())), Value::BigInt(big()));
    assert_eq!(
        v.validate_json("1180591620717411303424", &ValidateOptions::default())
            .unwrap(),
        Value::BigInt(big())
    );
}

#[test]
fn int_constraints() {
    let v = validator(r#"{"type": "int", "gt": 1}"#);
    assert_eq!(
        err(&v, Value::Int(1)),
        (
            "greater_than".into(),
            "Input should be greater than 1".into(),
            ctx(r#"{"gt": 1}"#),
            "constrained-int".into()
        )
    );
    let v = validator(r#"{"type": "int", "ge": 0, "le": 10}"#);
    assert_eq!(
        err(&v, Value::Int(11)).1,
        "Input should be less than or equal to 10"
    );
    assert_eq!(ok(&v, Value::Int(0)), Value::Int(0));
    let v = validator(r#"{"type": "int", "lt": 5}"#);
    assert_eq!(err(&v, Value::Int(5)).0, "less_than");
    let v = validator(r#"{"type": "int", "multiple_of": 3}"#);
    assert_eq!(err(&v, Value::Int(7)).1, "Input should be a multiple of 3");
    let three_big = Value::BigInt(big() * 3);
    assert_eq!(ok(&v, three_big.clone()), three_big);
}

#[test]
fn int_constraints_accept_coercible_values_and_big_ints() {
    // Constraint values are validated as lax ints: a numeric string works.
    let v = validator(r#"{"type": "int", "gt": "1"}"#);
    assert_eq!(ok(&v, Value::Int(2)), Value::Int(2));
    let v = validator(r#"{"type": "int", "gt": 1180591620717411303424}"#);
    let (type_, msg, ctx_value, _) = err(&v, Value::Int(5));
    assert_eq!(type_, "greater_than");
    assert_eq!(msg, "Input should be greater than 1180591620717411303424");
    assert_eq!(ctx_value, ctx(r#"{"gt": 1180591620717411303424}"#));
}

#[test]
fn int_bad_constraint_is_a_schema_error() {
    for (schema, key) in [
        (r#"{"type": "int", "gt": "x"}"#, "gt"),
        (r#"{"type": "int", "multiple_of": 1.5}"#, "multiple_of"),
    ] {
        assert_eq!(
            build(schema, None).unwrap_err(),
            CoreError::Schema(format!(
                "Error building \"int\" validator:\n  ValueError: '{key}' must be coercible to an integer"
            ))
        );
    }
}

#[test]
fn float_constraints_and_formatting() {
    let v = validator(r#"{"type": "float", "gt": 1.0}"#);
    assert_eq!(
        err(&v, Value::Float(1.0)),
        (
            "greater_than".into(),
            "Input should be greater than 1".into(),
            ctx(r#"{"gt": 1.0}"#),
            "constrained-float".into()
        )
    );
    // An int bound is read as a float.
    let v = validator(r#"{"type": "float", "gt": 1}"#);
    assert_eq!(err(&v, Value::Int(1)).2, ctx(r#"{"gt": 1.0}"#));
    let v = validator(r#"{"type": "float", "multiple_of": 0.1}"#);
    assert_eq!(ok(&v, Value::Float(0.3)), Value::Float(0.3));
    let v = validator(r#"{"type": "float", "multiple_of": 0.5}"#);
    assert_eq!(
        err(&v, Value::Float(0.3)).1,
        "Input should be a multiple of 0.5"
    );
    let v = validator(r#"{"type": "float", "le": 1.5}"#);
    assert_eq!(
        err(&v, Value::Int(2)).1,
        "Input should be less than or equal to 1.5"
    );
    // NaN fails every comparison.
    let v = validator(r#"{"type": "float", "gt": 0}"#);
    assert_eq!(err(&v, Value::Float(f64::NAN)).0, "greater_than");
}

#[test]
fn float_allow_inf_nan() {
    let v = validator(r#"{"type": "float"}"#);
    assert_eq!(v.title(), "float");
    assert_eq!(
        ok(&v, Value::Float(f64::INFINITY)),
        Value::Float(f64::INFINITY)
    );
    for v in [
        validator(r#"{"type": "float", "allow_inf_nan": false}"#),
        build(r#"{"type": "float"}"#, Some(r#"{"allow_inf_nan": false}"#)).unwrap(),
    ] {
        assert_eq!(
            err(&v, Value::from("nan")),
            (
                "finite_number".into(),
                "Input should be a finite number".into(),
                None,
                "float".into()
            )
        );
    }
    let v = validator(r#"{"type": "float", "gt": 0, "allow_inf_nan": false}"#);
    let Err(ValidateError::Validation(e)) = v.validate_json("NaN", &ValidateOptions::default())
    else {
        panic!("expected a validation error")
    };
    assert_eq!(
        e.errors(&ErrorsOptions::default())[0].type_,
        "finite_number"
    );
}

#[test]
fn float_bad_constraint_is_a_schema_error() {
    let err = build(r#"{"type": "float", "gt": "x"}"#, None).unwrap_err();
    // The wording differs from pydantic's (see docs/DIVERGENCES.md #6); the kind matches.
    assert_eq!(
        err,
        CoreError::Schema(
            "Error building \"float\" validator:\n  TypeError: 'gt' should be a number, got str"
                .into()
        )
    );
}
