//! `literal`, `nullable` and `default` validators (upstream validators/literal.rs,
//! validators/nullable.rs, validators/with_default.rs). Expectations come from pydantic-core 2.49.0.

use num_bigint::BigInt;
use perldantic_core::{
    CoreError, ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&Value::from_json(schema).unwrap(), None).unwrap()
}

fn build_error(schema: &str) -> String {
    match SchemaValidator::new(&Value::from_json(schema).unwrap(), None) {
        Err(CoreError::Schema(msg)) => msg,
        other => panic!("expected a schema error, got {other:?}"),
    }
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
}

/// `(type, msg, ctx.expected)` of the only error.
fn only_error(result: Result<Value, ValidateError>) -> (String, String, Option<Value>) {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    let errors = e.errors(&ErrorsOptions::default());
    assert_eq!(errors.len(), 1, "{errors:?}");
    let ctx = errors[0].ctx.as_ref().and_then(|c| match c {
        Value::Dict(d) => d.get_str("expected").cloned(),
        _ => None,
    });
    (errors[0].type_.clone(), errors[0].msg.clone(), ctx)
}

fn ok(v: &SchemaValidator, input: Value) -> Value {
    v.validate_value(&input, &lax()).unwrap()
}

/// Asserts equality including the variant, so `1` and `True` are told apart.
fn assert_exact(actual: &Value, expected: &Value) {
    assert_eq!(
        (actual.type_name(), actual),
        (expected.type_name(), expected)
    );
}

#[test]
fn literal_name_and_error_list_the_expected_values() {
    let v = validator(r#"{"type": "literal", "expected": ["a", "b", 1, true]}"#);
    assert_eq!(v.title(), "literal['a','b',1,True]");
    assert_eq!(
        only_error(v.validate_value(&Value::from("c"), &lax())),
        (
            "literal_error".into(),
            "Input should be 'a', 'b', 1 or True".into(),
            Some(Value::from("'a', 'b', 1 or True"))
        )
    );

    let single = validator(r#"{"type": "literal", "expected": ["a"]}"#);
    assert_eq!(
        only_error(single.validate_value(&Value::Bytes(b"a".to_vec()), &lax())).1,
        "Input should be 'a'"
    );
}

#[test]
fn literal_requires_a_non_empty_expected_list() {
    assert_eq!(
        build_error(r#"{"type": "literal", "expected": []}"#),
        "Error building \"literal\" validator:\n  SchemaError: `expected` should have length > 0"
    );
}

#[test]
fn literal_returns_the_expected_value_not_the_input() {
    // Python hash/equality: 1 == 1.0 == True, and the last equal expected value wins.
    let mixed = validator(r#"{"type": "literal", "expected": ["a", "b", 1, true]}"#);
    assert_exact(&ok(&mixed, Value::Float(1.0)), &Value::Bool(true));
    assert_exact(&ok(&mixed, Value::Bool(true)), &Value::Bool(true));

    let int = validator(r#"{"type": "literal", "expected": [1]}"#);
    assert_exact(&ok(&int, Value::Bool(true)), &Value::Int(1));
    assert_exact(&ok(&int, Value::Float(1.0)), &Value::Int(1));

    let float = validator(r#"{"type": "literal", "expected": [1.0]}"#);
    assert_exact(&ok(&float, Value::Int(1)), &Value::Float(1.0));

    let bool_ = validator(r#"{"type": "literal", "expected": [true]}"#);
    assert_exact(&ok(&bool_, Value::Int(1)), &Value::Bool(true));

    let both = validator(r#"{"type": "literal", "expected": [1, true]}"#);
    assert_exact(&ok(&both, Value::Bool(true)), &Value::Bool(true));
}

#[test]
fn literal_matches_other_values_by_python_equality() {
    let none = validator(r#"{"type": "literal", "expected": [null]}"#);
    assert_exact(&ok(&none, Value::None), &Value::None);

    let big = BigInt::from(2).pow(70);
    let big_literal = SchemaValidator::new(
        &Value::Dict(
            [
                (Value::from("type"), Value::from("literal")),
                (
                    Value::from("expected"),
                    Value::List(vec![Value::from(big.clone())]),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        None,
    )
    .unwrap();
    assert_eq!(ok(&big_literal, Value::from(big.clone())), Value::from(big));

    let list = validator(r#"{"type": "literal", "expected": [[1]]}"#);
    assert_eq!(
        ok(&list, Value::List(vec![Value::Int(1)])),
        Value::List(vec![Value::Int(1)])
    );
    assert_eq!(
        only_error(list.validate_value(&Value::Tuple(vec![Value::Int(1)]), &lax())).1,
        "Input should be [1]"
    );
}

#[test]
fn literal_from_json() {
    let ints = validator(r#"{"type": "literal", "expected": [1, 2]}"#);
    assert_exact(&ints.validate_json("1", &lax()).unwrap(), &Value::Int(1));
    assert_exact(&ints.validate_json("1.0", &lax()).unwrap(), &Value::Int(1));

    let strs = validator(r#"{"type": "literal", "expected": ["a"]}"#);
    assert_eq!(
        strs.validate_json(r#""a""#, &lax()).unwrap(),
        Value::from("a")
    );

    let floats = validator(r#"{"type": "literal", "expected": [1.5]}"#);
    assert_eq!(
        floats.validate_json("1.5", &lax()).unwrap(),
        Value::Float(1.5)
    );
}

#[test]
fn nullable_accepts_none_or_the_inner_type() {
    let v = validator(r#"{"type": "nullable", "schema": {"type": "int"}}"#);
    assert_eq!(v.title(), "nullable[int]");
    assert_eq!(ok(&v, Value::None), Value::None);
    assert_eq!(ok(&v, Value::from("3")), Value::Int(3));
    assert_eq!(v.validate_json("null", &lax()).unwrap(), Value::None);
    assert_eq!(
        only_error(v.validate_value(&Value::from("x"), &lax())).1,
        "Input should be a valid integer, unable to parse string as an integer"
    );
}

#[test]
fn default_raises_inner_errors_by_default() {
    let v = validator(r#"{"type": "default", "schema": {"type": "int"}, "default": 5}"#);
    assert_eq!(v.title(), "default[int]");
    assert_eq!(ok(&v, Value::from("7")), Value::Int(7));
    assert_eq!(
        only_error(v.validate_value(&Value::from("x"), &lax())).0,
        "int_parsing"
    );
}

#[test]
fn default_on_error_default_returns_the_default() {
    let v = validator(
        r#"{"type": "default", "schema": {"type": "int"}, "default": 5, "on_error": "default"}"#,
    );
    assert_eq!(ok(&v, Value::from("x")), Value::Int(5));
}

#[test]
fn default_on_error_omit_is_an_error_outside_a_container() {
    let v = validator(
        r#"{"type": "default", "schema": {"type": "int"}, "default": 5, "on_error": "omit"}"#,
    );
    match v.validate_value(&Value::from("x"), &lax()) {
        Err(ValidateError::Core(CoreError::Schema(msg))) => assert_eq!(
            msg,
            "Uncaught Omit error, please check your usage of `default` validators."
        ),
        other => panic!("expected an Omit schema error, got {other:?}"),
    }
}

#[test]
fn default_schema_errors() {
    assert_eq!(
        build_error(r#"{"type": "default", "schema": {"type": "int"}, "on_error": "default"}"#),
        "Error building \"default\" validator:\n  SchemaError: 'on_error = default' requires a `default` or `default_factory`"
    );
    assert_eq!(
        build_error(
            r#"{"type": "default", "schema": {"type": "int"}, "default": 1, "default_factory": 2}"#
        ),
        "Error building \"default\" validator:\n  SchemaError: 'default' and 'default_factory' cannot be used together"
    );
    // Upstream panics here (its schema validation rejects the value first).
    assert_eq!(
        build_error(r#"{"type": "default", "schema": {"type": "int"}, "on_error": "bogus"}"#),
        "Error building \"default\" validator:\n  SchemaError: `on_error` should be 'raise', 'omit' or 'default', got 'bogus'"
    );
    // Host callbacks do not exist yet, so a factory cannot be called.
    assert_eq!(
        build_error(r#"{"type": "default", "schema": {"type": "int"}, "default_factory": 2}"#),
        "Error building \"default\" validator:\n  SchemaError: `default_factory` is not supported yet: host callbacks are not implemented"
    );
}

#[test]
fn literal_big_int_input_with_int_literals_is_a_size_error() {
    // Upstream quirk kept as is: the i64 lookup fails before the other lookups run.
    let big = BigInt::from(2).pow(70);
    let v = SchemaValidator::new(
        &Value::Dict(
            [
                (Value::from("type"), Value::from("literal")),
                (
                    Value::from("expected"),
                    Value::List(vec![Value::Int(1), Value::from(big.clone())]),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        None,
    )
    .unwrap();
    assert_eq!(
        only_error(v.validate_value(&Value::from(big), &lax())).0,
        "int_parsing_size"
    );
}

#[test]
fn literal_unhashable_values_compare_by_equality() {
    let v = validator(r#"{"type": "literal", "expected": [{"a": [1]}]}"#);
    let input = Value::from_json(r#"{"a": [1.0]}"#).unwrap();
    assert_eq!(ok(&v, input), Value::from_json(r#"{"a": [1]}"#).unwrap());
}

#[test]
fn default_is_validated_only_with_validate_default() {
    let plain = validator(
        r#"{"type": "default", "schema": {"type": "int"}, "default": "7", "on_error": "default"}"#,
    );
    assert_eq!(ok(&plain, Value::from("y")), Value::from("7"));

    let from_schema = validator(
        r#"{"type": "default", "schema": {"type": "int"}, "default": "7", "on_error": "default", "validate_default": true}"#,
    );
    assert_eq!(ok(&from_schema, Value::from("y")), Value::Int(7));

    let from_config = SchemaValidator::new(
        &Value::from_json(
            r#"{"type": "default", "schema": {"type": "int"}, "default": "7", "on_error": "default"}"#,
        )
        .unwrap(),
        Some(&Value::from_json(r#"{"validate_default": true}"#).unwrap()),
    )
    .unwrap();
    assert_eq!(ok(&from_config, Value::from("y")), Value::Int(7));
}

#[test]
fn invalid_default_reports_its_own_error() {
    // Upstream recurses until the stack overflows (docs/DIVERGENCES.md #10).
    let v = validator(
        r#"{"type": "default", "schema": {"type": "int"}, "default": "x", "on_error": "default", "validate_default": true}"#,
    );
    let Err(ValidateError::Validation(e)) = v.validate_value(&Value::from("y"), &lax()) else {
        panic!("expected a validation error")
    };
    let errors = e.errors(&ErrorsOptions::default());
    assert_eq!(errors[0].type_, "int_parsing");
    assert_eq!(errors[0].input, Some(Value::from("x")));
}
