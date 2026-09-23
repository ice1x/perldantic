//! `definitions` and `definition-ref` validators (upstream validators/definitions.rs): the core
//! of recursive Perl types. Expectations come from pydantic-core 2.49.0.

use perldantic_core::{
    CoreError, ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
}

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&j(schema), None).unwrap()
}

fn build_error(schema: &str) -> String {
    match SchemaValidator::new(&j(schema), None) {
        Err(e @ CoreError::Schema(_)) => e.to_string(),
        other => panic!("expected a schema error, got {other:?}"),
    }
}

fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| (d.type_, serde_json::to_string(&d.loc).unwrap()))
        .collect()
}

/// A tree: a list of trees.
const TREE: &str = r#"{"type": "definitions", "schema": {"type": "definition-ref", "schema_ref": "node"},
    "definitions": [{"type": "list", "items_schema": {"type": "definition-ref", "schema_ref": "node"}, "ref": "node"}]}"#;

#[test]
fn recursive_schemas_validate_nested_data() {
    let v = validator(TREE);
    assert_eq!(v.title(), "list[...]");
    let input = j("[[[]], []]");
    assert_eq!(v.validate_value(&input, &lax()).unwrap(), input);
    assert_eq!(v.validate_json("[[[]]]", &lax()).unwrap(), j("[[[]]]"));
    assert_eq!(
        errors(v.validate_value(&j("[[1]]"), &lax())),
        vec![("list_type".into(), "[0,0]".into())]
    );
}

#[test]
fn a_schema_that_revisits_the_same_input_is_a_recursion_loop() {
    let v = validator(
        r#"{"type": "definitions", "schema": {"type": "definition-ref", "schema_ref": "a"},
            "definitions": [{"type": "nullable", "schema": {"type": "definition-ref", "schema_ref": "a"}, "ref": "a"}]}"#,
    );
    assert_eq!(v.title(), "nullable[...]");
    assert_eq!(
        errors(v.validate_value(&Value::Int(1), &lax())),
        vec![("recursion_loop".into(), "[]".into())]
    );
    assert_eq!(v.validate_value(&Value::None, &lax()).unwrap(), Value::None);
}

#[test]
fn nesting_deeper_than_the_limit_is_a_recursion_loop() {
    let v = validator(TREE);
    let mut deep = Value::List(vec![]);
    for _ in 0..300 {
        deep = Value::List(vec![deep]);
    }
    let errors = errors(v.validate_value(&deep, &lax()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, "recursion_loop");
}

#[test]
fn defaults_are_reached_through_references() {
    let v = validator(
        r#"{"type": "definitions", "schema": {"type": "tuple", "items_schema": [{"type": "definition-ref", "schema_ref": "i"}]},
            "definitions": [{"type": "default", "schema": {"type": "int"}, "default": 3, "ref": "i"}]}"#,
    );
    assert_eq!(
        v.validate_value(&j("[]"), &lax()).unwrap(),
        Value::Tuple(vec![Value::Int(3)])
    );
}

#[test]
fn definition_errors() {
    assert_eq!(
        build_error(
            r#"{"type": "definitions", "schema": {"type": "definition-ref", "schema_ref": "missing"}, "definitions": []}"#
        ),
        "Definitions error: definition `missing` was never filled"
    );
    assert_eq!(
        build_error(r#"{"type": "definition-ref", "schema_ref": "x"}"#),
        "Definitions error: definition `x` was never filled"
    );
    assert_eq!(
        build_error(
            r#"{"type": "definitions", "schema": {"type": "int"}, "definitions": [{"type": "int", "ref": "a"}, {"type": "str", "ref": "a"}]}"#
        ),
        "Error building \"definitions\" validator:\n  SchemaError: Duplicate ref: `a`"
    );
    match SchemaValidator::new(
        &j(
            r#"{"type": "definitions", "schema": {"type": "int"}, "definitions": [{"type": "int"}]}"#,
        ),
        None,
    ) {
        Err(e) => assert_eq!(
            e.to_string(),
            "Error building \"definitions\" validator:\n  KeyError: 'ref'"
        ),
        Ok(_) => panic!("expected an error"),
    }
}
