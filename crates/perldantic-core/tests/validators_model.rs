//! `model` validator (upstream validators/model.rs): builds model instances. In the core a
//! model class is its name (a Perl package) and an instance is `Value::Model`.
//! Expectations come from pydantic-core 2.49.0.

use perldantic_core::{
    CoreError, Dict, ErrorsOptions, Model, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn s(v: &str) -> Value {
    Value::from(v)
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
}

fn strict() -> ValidateOptions {
    ValidateOptions {
        strict: Some(true),
        ..lax()
    }
}

fn dict(json: &str) -> Dict {
    match j(json) {
        Value::Dict(d) => d,
        _ => panic!("not a dict"),
    }
}

fn instance(class: &str, fields: &str, fields_set: &[&str], extra: Option<&str>) -> Value {
    Value::Model(Box::new(Model {
        class: class.into(),
        fields: dict(fields),
        fields_set: fields_set.iter().map(|f| s(f)).collect(),
        extra: extra.map(dict),
    }))
}

/// Model `M` with field `a: int` and `b: str = 'x'`, plus `extra` schema keys.
fn schema(extra: &str) -> String {
    format!(
        r#"{{"type": "model", "cls": "M", "schema": {{"type": "model-fields", "fields": {{
            "a": {{"type": "model-field", "schema": {{"type": "int"}}}},
            "b": {{"type": "model-field", "schema": {{"type": "default", "schema": {{"type": "str"}}, "default": "x"}}}}
        }}}}{extra}}}"#
    )
}

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&j(schema), None).unwrap()
}

fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| (d.type_, serde_json::to_string(&d.loc).unwrap(), d.msg))
        .collect()
}

#[test]
fn builds_instances_from_dicts_and_json() {
    let v = validator(&schema(""));
    assert_eq!(v.title(), "M");
    assert_eq!(
        v.validate_value(&j(r#"{"a": "1"}"#), &lax()).unwrap(),
        instance("M", r#"{"a": 1, "b": "x"}"#, &["a"], None)
    );
    assert_eq!(
        v.validate_json(r#"{"a": 2}"#, &lax()).unwrap(),
        instance("M", r#"{"a": 2, "b": "x"}"#, &["a"], None)
    );
    assert_eq!(
        errors(v.validate_value(&s("x"), &lax())),
        vec![(
            "model_type".into(),
            "[]".into(),
            "Input should be a valid dictionary or instance of Model".into()
        )]
    );
}

#[test]
fn instances_pass_through_unless_revalidated() {
    let bad = instance("M", r#"{"a": "bad", "b": "x"}"#, &["a"], None);
    let v = validator(&schema(""));
    assert_eq!(v.validate_value(&bad, &lax()).unwrap(), bad);
    assert_eq!(v.validate_value(&bad, &strict()).unwrap(), bad);

    let always = validator(&schema(r#", "revalidate_instances": "always""#));
    assert_eq!(
        errors(always.validate_value(&bad, &lax())),
        vec![(
            "int_parsing".into(),
            r#"["a"]"#.into(),
            "Input should be a valid integer, unable to parse string as an integer".into()
        )]
    );
    // Revalidation keeps the instance's fields_set.
    let good = instance("M", r#"{"a": "5", "b": "y"}"#, &["a"], None);
    assert_eq!(
        always.validate_value(&good, &lax()).unwrap(),
        instance("M", r#"{"a": 5, "b": "y"}"#, &["a"], None)
    );

    // An instance of another class is not an instance of M.
    let other = instance("Other", r#"{"a": 1}"#, &["a"], None);
    assert_eq!(errors(v.validate_value(&other, &lax()))[0].0, "model_type");
}

#[test]
fn generic_origin_instances_are_revalidated() {
    let v = validator(&schema(r#", "generic_origin": "Base""#));
    let origin = instance("Base", r#"{"a": "7"}"#, &["a"], None);
    assert_eq!(
        v.validate_value(&origin, &lax()).unwrap(),
        instance("M", r#"{"a": 7, "b": "x"}"#, &["a"], None)
    );
}

#[test]
fn strict_mode_reaches_the_fields() {
    let v = validator(&schema(""));
    assert_eq!(
        errors(v.validate_value(&j(r#"{"a": "1"}"#), &strict()))[0].0,
        "int_type"
    );
}

#[test]
fn extra_values_are_kept_on_the_instance() {
    let v = validator(
        r#"{"type": "model", "cls": "M", "schema": {"type": "model-fields", "extra_behavior": "allow", "fields": {"a": {"type": "model-field", "schema": {"type": "int"}}}}}"#,
    );
    assert_eq!(
        v.validate_value(&j(r#"{"a": 1, "z": 2}"#), &lax()).unwrap(),
        instance("M", r#"{"a": 1}"#, &["a", "z"], Some(r#"{"z": 2}"#))
    );
    // Revalidation feeds the extra values back in.
    let always = validator(
        r#"{"type": "model", "cls": "M", "revalidate_instances": "always", "schema": {"type": "model-fields", "extra_behavior": "allow", "fields": {"a": {"type": "model-field", "schema": {"type": "int"}}}}}"#,
    );
    let existing = instance("M", r#"{"a": "1"}"#, &["a", "z"], Some(r#"{"z": 2}"#));
    assert_eq!(
        always.validate_value(&existing, &lax()).unwrap(),
        instance("M", r#"{"a": 1}"#, &["a", "z"], Some(r#"{"z": 2}"#))
    );
}

#[test]
fn models_use_their_own_config_only() {
    let own = validator(&schema(
        r#", "config": {"extra_fields_behavior": "forbid"}"#,
    ));
    assert_eq!(
        errors(own.validate_value(&j(r#"{"a": 1, "z": 2}"#), &lax()))[0].0,
        "extra_forbidden"
    );
    let parent = SchemaValidator::new(
        &j(&schema("")),
        Some(&j(r#"{"extra_fields_behavior": "forbid"}"#)),
    )
    .unwrap();
    assert!(
        parent
            .validate_value(&j(r#"{"a": 1, "z": 2}"#), &lax())
            .is_ok()
    );
}

#[test]
fn root_models_hold_a_single_root_value() {
    let v = validator(
        r#"{"type": "model", "cls": "R", "root_model": true, "schema": {"type": "int"}}"#,
    );
    assert_eq!(
        v.validate_value(&s("3"), &lax()).unwrap(),
        instance("R", r#"{"root": 3}"#, &["root"], None)
    );
    assert_eq!(errors(v.validate_value(&s("x"), &lax()))[0].1, "[]");
}

#[test]
fn a_union_prefers_the_model_for_its_fields() {
    let v = SchemaValidator::new(
        &j(&format!(
            r#"{{"type": "union", "choices": [{}, {{"type": "dict"}}]}}"#,
            schema("")
        )),
        None,
    )
    .unwrap();
    assert_eq!(
        v.validate_value(&j(r#"{"a": 1}"#), &lax()).unwrap(),
        instance("M", r#"{"a": 1, "b": "x"}"#, &["a"], None)
    );
}

fn build_error(schema: &str) -> String {
    match SchemaValidator::new(&j(schema), None) {
        Err(e @ CoreError::Schema(_)) => e.to_string(),
        other => panic!("expected a schema error, got {other:?}"),
    }
}

#[test]
fn model_schema_errors() {
    assert_eq!(
        build_error(&schema(r#", "revalidate_instances": "bogus""#)),
        "Error building \"model\" validator:\n  SchemaError: Invalid revalidate_instances value: bogus"
    );
    // Hooks into the host class need host callbacks.
    assert_eq!(
        build_error(&schema(r#", "post_init": "check""#)),
        "Error building \"model\" validator:\n  SchemaError: `post_init` is not supported yet: host callbacks are not implemented"
    );
    assert_eq!(
        build_error(&schema(r#", "custom_init": true"#)),
        "Error building \"model\" validator:\n  SchemaError: `custom_init` is not supported yet: host callbacks are not implemented"
    );
}
