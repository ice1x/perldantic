//! `ErrorType` must reproduce pydantic's error codes, message templates and contexts.
//!
//! The oracle is upstream's own `all_errors` table (tests/test_errors.py), extracted into
//! `fixtures/upstream_error_messages.json` by `tools/extract_upstream_errors.py`.

use std::collections::BTreeSet;

use perldantic_core::{CoreError, Dict, ErrorType, InputType, Value};

#[derive(serde::Deserialize)]
struct Case {
    #[serde(rename = "type")]
    type_: String,
    message: String,
    context: Option<serde_json::Value>,
}

fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("fixtures/upstream_error_messages.json")).unwrap()
}

fn to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::None,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Float(n.as_f64().unwrap()), Value::Int),
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(items) => Value::List(items.iter().map(to_value).collect()),
        serde_json::Value::Object(map) => Value::Dict(
            map.iter()
                .map(|(k, v)| (Value::Str(k.clone()), to_value(v)))
                .collect(),
        ),
    }
}

fn to_dict(json: Option<&serde_json::Value>) -> Option<Dict> {
    json.map(|j| match to_value(j) {
        Value::Dict(d) => d,
        other => panic!("context must be an object, got {other:?}"),
    })
}

#[test]
fn every_upstream_message_renders_identically() {
    for case in cases() {
        let context = to_dict(case.context.as_ref());
        let error = ErrorType::new(&case.type_, context.as_ref())
            .unwrap_or_else(|e| panic!("{}: {e}", case.type_));
        assert_eq!(error.type_string(), case.type_);
        assert_eq!(
            error.render_message(InputType::Python).unwrap(),
            case.message,
            "message for {}",
            case.type_
        );
    }
}

#[test]
fn context_round_trips() {
    for case in cases() {
        let context = to_dict(case.context.as_ref());
        let error = ErrorType::new(&case.type_, context.as_ref()).unwrap();
        assert_eq!(error.context(), context, "context for {}", case.type_);
    }
}

#[test]
fn all_error_types_are_covered_by_the_upstream_table() {
    let listed: BTreeSet<String> = cases().into_iter().map(|c| c.type_).collect();
    let actual: BTreeSet<String> = ErrorType::all_type_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(listed, actual);
}

#[test]
fn unknown_error_type_is_a_key_error() {
    let err = ErrorType::new("not_a_real_error", None).unwrap_err();
    assert_eq!(
        err,
        CoreError::Key("Invalid error type: 'not_a_real_error'".into())
    );
}

#[test]
fn missing_context_field_is_a_type_error() {
    let err = ErrorType::new("greater_than", None).unwrap_err();
    assert_eq!(
        err,
        CoreError::Type("GreaterThan: 'gt' required in context".into())
    );
}

#[test]
fn wrong_context_type_is_a_type_error() {
    let mut ctx = Dict::new();
    ctx.insert(Value::from("min_length"), Value::from("three"));
    let err = ErrorType::new("string_too_short", Some(&ctx)).unwrap_err();
    assert_eq!(
        err,
        CoreError::Type("StringTooShort: 'min_length' context value must be a usize".into())
    );
}

#[test]
fn json_input_uses_json_vocabulary() {
    let cases = [
        ("none_required", "Input should be null"),
        ("list_type", "Input should be a valid array"),
        ("tuple_type", "Input should be a valid array"),
        ("dict_type", "Input should be an object"),
        ("time_delta_type", "Input should be a valid duration"),
        ("int_type", "Input should be a valid integer"),
    ];
    for (type_, message) in cases {
        let error = ErrorType::new(type_, None).unwrap();
        assert_eq!(error.render_message(InputType::Json).unwrap(), message);
    }
}

#[test]
fn too_long_without_actual_length_says_more() {
    let mut ctx = Dict::new();
    ctx.insert(Value::from("field_type"), Value::from("List"));
    ctx.insert(Value::from("max_length"), Value::from(1_i64));
    ctx.insert(Value::from("actual_length"), Value::None);
    let error = ErrorType::new("too_long", Some(&ctx)).unwrap();
    assert_eq!(
        error.render_message(InputType::Python).unwrap(),
        "List should have at most 1 item after validation, not more"
    );
}

#[test]
fn custom_error_formats_template_from_context() {
    let mut ctx = Dict::new();
    ctx.insert(Value::from("foo"), Value::from("FOOBAR"));
    ctx.insert(Value::from("bar"), Value::from(42_i64));
    let error = ErrorType::new_custom_error(
        "my_error",
        "this is a custom error {foo} {bar}",
        Some(ctx.clone()),
    );
    assert_eq!(error.type_string(), "my_error");
    assert_eq!(
        error.render_message(InputType::Python).unwrap(),
        "this is a custom error FOOBAR 42"
    );
    assert_eq!(error.context(), Some(ctx));
}

#[test]
fn custom_error_without_context_keeps_placeholders() {
    let error = ErrorType::new_custom_error("my_error", "this is a custom error {missed}", None);
    assert_eq!(
        error.render_message(InputType::Python).unwrap(),
        "this is a custom error {missed}"
    );
    assert_eq!(error.context(), None);
}
