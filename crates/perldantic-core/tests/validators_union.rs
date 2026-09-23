//! `union` and `custom-error` validators (upstream validators/union.rs,
//! validators/custom_error.rs): the core of Perl unions such as `Int | Str`.
//! Expectations come from pydantic-core 2.49.0.

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

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
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

/// Errors as `(type, loc, msg, ctx)`, with `loc` and `ctx` rendered as JSON.
fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String, String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| {
            (
                d.type_,
                serde_json::to_string(&d.loc).unwrap(),
                d.msg,
                serde_json::to_string(&d.ctx).unwrap(),
            )
        })
        .collect()
}

fn err(type_: &str, loc: &str, msg: &str, ctx: &str) -> (String, String, String, String) {
    (type_.into(), loc.into(), msg.into(), ctx.into())
}

const INT_STR: &str = r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}]}"#;

#[test]
fn smart_mode_prefers_the_exact_match() {
    let v = validator(INT_STR);
    assert_eq!(v.title(), "union[int,str]");
    assert_exact(&ok(&v, Value::from("1")), &Value::from("1"));
    assert_exact(&ok(&v, Value::Int(1)), &Value::Int(1));

    let str_int = validator(r#"{"type": "union", "choices": [{"type": "str"}, {"type": "int"}]}"#);
    assert_exact(&ok(&str_int, Value::Int(1)), &Value::Int(1));

    let int_bool =
        validator(r#"{"type": "union", "choices": [{"type": "int"}, {"type": "bool"}]}"#);
    assert_exact(&ok(&int_bool, Value::Bool(true)), &Value::Bool(true));
    let bool_int =
        validator(r#"{"type": "union", "choices": [{"type": "bool"}, {"type": "int"}]}"#);
    assert_exact(&ok(&bool_int, Value::Int(1)), &Value::Int(1));
}

#[test]
fn smart_mode_falls_back_to_the_first_best_lax_match() {
    let float_int =
        validator(r#"{"type": "union", "choices": [{"type": "float"}, {"type": "int"}]}"#);
    assert_exact(&ok(&float_int, Value::Int(1)), &Value::Int(1));
    assert_exact(
        &float_int.validate_json("1", &lax()).unwrap(),
        &Value::Int(1),
    );

    let int_float =
        validator(r#"{"type": "union", "choices": [{"type": "int"}, {"type": "float"}]}"#);
    assert_exact(&ok(&int_float, Value::from("1.5")), &Value::Float(1.5));

    let list_tuple = validator(
        r#"{"type": "union", "choices": [{"type": "list", "items_schema": {"type": "int"}}, {"type": "tuple", "items_schema": [{"type": "int"}], "variadic_item_index": 0}]}"#,
    );
    assert_exact(
        &ok(
            &list_tuple,
            Value::Tuple(vec![Value::Int(1), Value::Int(2)]),
        ),
        &Value::Tuple(vec![Value::Int(1), Value::Int(2)]),
    );
}

#[test]
fn errors_are_located_by_choice_name_or_label() {
    let v = validator(INT_STR);
    assert_eq!(
        errors(v.validate_value(&j("[1]"), &lax())),
        vec![
            err(
                "int_type",
                r#"["int"]"#,
                "Input should be a valid integer",
                "null"
            ),
            err(
                "string_type",
                r#"["str"]"#,
                "Input should be a valid string",
                "null"
            ),
        ]
    );

    let labelled = SchemaValidator::new(
        &Value::Dict(
            [
                (Value::from("type"), Value::from("union")),
                (
                    Value::from("choices"),
                    Value::List(vec![
                        Value::Tuple(vec![j(r#"{"type": "int"}"#), Value::from("Int")]),
                        Value::Tuple(vec![j(r#"{"type": "str"}"#), Value::from("Str")]),
                    ]),
                ),
            ]
            .into_iter()
            .collect(),
        ),
        None,
    )
    .unwrap();
    assert_eq!(labelled.title(), "union[Int,Str]");
    assert_eq!(
        errors(labelled.validate_value(&j("[1]"), &lax()))
            .into_iter()
            .map(|e| e.1)
            .collect::<Vec<_>>(),
        vec![r#"["Int"]"#, r#"["Str"]"#]
    );
}

#[test]
fn left_to_right_mode_takes_the_first_success() {
    let v = validator(
        r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}], "mode": "left_to_right"}"#,
    );
    assert_exact(&ok(&v, Value::from("1")), &Value::Int(1));
    assert_eq!(errors(v.validate_value(&j("[1]"), &lax())).len(), 2);
}

#[test]
fn single_choice_collapses_unless_disabled() {
    let collapsed = validator(r#"{"type": "union", "choices": [{"type": "int"}]}"#);
    assert_eq!(collapsed.title(), "int");
    assert_eq!(
        errors(collapsed.validate_value(&Value::from("x"), &lax()))[0].1,
        "[]"
    );

    let kept =
        validator(r#"{"type": "union", "choices": [{"type": "int"}], "auto_collapse": false}"#);
    assert_eq!(kept.title(), "union[int]");
    assert_eq!(
        errors(kept.validate_value(&Value::from("x"), &lax()))[0].1,
        r#"["int"]"#
    );
}

#[test]
fn union_schema_errors() {
    assert_eq!(
        build_error(r#"{"type": "union", "choices": []}"#),
        "Error building \"union\" validator:\n  SchemaError: One or more union choices required"
    );
    assert_eq!(
        build_error(
            r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}], "mode": "bad"}"#
        ),
        "Error building \"union\" validator:\n  SchemaError: Invalid union mode: `bad`, expected `smart` or `left_to_right`"
    );
    assert_eq!(
        build_error(
            r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}], "custom_error_type": "int_type", "custom_error_message": "m"}"#
        ),
        "Error building \"union\" validator:\n  SchemaError: custom_error_message should not be provided if 'custom_error_type' matches a known error"
    );
}

#[test]
fn custom_errors_replace_the_choice_errors() {
    let custom = validator(
        r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}], "custom_error_type": "my_err", "custom_error_message": "Bad {x}", "custom_error_context": {"x": 1}}"#,
    );
    assert_eq!(
        errors(custom.validate_value(&j("[1]"), &lax())),
        vec![err("my_err", "[]", "Bad 1", r#"{"x":1}"#)]
    );

    let known = validator(
        r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}], "custom_error_type": "int_type"}"#,
    );
    assert_eq!(
        errors(known.validate_value(&j("[1]"), &lax())),
        vec![err(
            "int_type",
            "[]",
            "Input should be a valid integer",
            "null"
        )]
    );

    let known_with_ctx = validator(
        r#"{"type": "union", "choices": [{"type": "int"}, {"type": "str"}], "custom_error_type": "too_short", "custom_error_context": {"field_type": "X", "min_length": 1, "actual_length": 0}}"#,
    );
    assert_eq!(
        errors(known_with_ctx.validate_value(&j("[1]"), &lax()))[0].2,
        "X should have at least 1 item after validation, not 0"
    );
}

#[test]
fn omit_from_a_choice_is_raised_when_nothing_matches() {
    let v = validator(
        r#"{"type": "union", "choices": [{"type": "default", "schema": {"type": "int"}, "on_error": "omit"}, {"type": "str"}]}"#,
    );
    match v.validate_value(&j("[1]"), &lax()) {
        Err(ValidateError::Core(CoreError::Schema(msg))) => assert_eq!(
            msg,
            "Uncaught Omit error, please check your usage of `default` validators."
        ),
        other => panic!("expected an Omit schema error, got {other:?}"),
    }
}

#[test]
fn custom_error_validator_replaces_any_inner_error() {
    let v = validator(
        r#"{"type": "custom-error", "schema": {"type": "int"}, "custom_error_type": "my_err", "custom_error_message": "Not an int"}"#,
    );
    assert_eq!(v.title(), "custom-error[int]");
    assert_eq!(ok(&v, Value::from("3")), Value::Int(3));
    assert_eq!(
        errors(v.validate_value(&Value::from("x"), &lax())),
        vec![err("my_err", "[]", "Not an int", "null")]
    );
}

#[test]
fn custom_error_validator_schema_errors() {
    // Upstream panics without `custom_error_type` (docs/DIVERGENCES.md #11).
    assert_eq!(
        build_error(r#"{"type": "custom-error", "schema": {"type": "int"}}"#),
        "Error building \"custom-error\" validator:\n  SchemaError: `custom_error_type` is required"
    );
    match SchemaValidator::new(
        &j(r#"{"type": "custom-error", "schema": {"type": "int"}, "custom_error_type": "my"}"#),
        None,
    ) {
        Err(e) => assert_eq!(
            e.to_string(),
            "Error building \"custom-error\" validator:\n  KeyError: 'custom_error_message'"
        ),
        Ok(_) => panic!("expected a schema error"),
    }
}
