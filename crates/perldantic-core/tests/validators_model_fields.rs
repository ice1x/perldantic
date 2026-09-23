//! `model-fields` validator (upstream validators/model_fields.rs): the field-level half of a
//! Perl model. Output is `(fields, extra, fields_set)`. Expectations come from
//! pydantic-core 2.49.0.

use perldantic_core::{
    CoreError, ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn s(v: &str) -> Value {
    Value::from(v)
}

fn validator_with(schema: &str, config: Option<&str>) -> SchemaValidator {
    SchemaValidator::new(&j(schema), config.map(j).as_ref()).unwrap()
}

fn validator(schema: &str) -> SchemaValidator {
    validator_with(schema, None)
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
}

/// `(fields, extra, fields_set)` as model-fields returns it.
fn output(fields: &str, extra: Option<&str>, fields_set: &[&str]) -> Value {
    Value::Tuple(vec![
        j(fields),
        extra.map_or(Value::None, j),
        Value::Set(fields_set.iter().map(|f| s(f)).collect()),
    ])
}

/// Errors as `(type, loc, msg)`, with `loc` rendered as JSON.
fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| (d.type_, serde_json::to_string(&d.loc).unwrap(), d.msg))
        .collect()
}

fn err(type_: &str, loc: &str, msg: &str) -> (String, String, String) {
    (type_.into(), loc.into(), msg.into())
}

const INT_PARSING: &str = "Input should be a valid integer, unable to parse string as an integer";

/// Field `a: int` and field `b: str = 'x'`, plus `extra` schema keys.
fn base(extra: &str) -> String {
    format!(
        r#"{{"type": "model-fields", "fields": {{
            "a": {{"type": "model-field", "schema": {{"type": "int"}}}},
            "b": {{"type": "model-field", "schema": {{"type": "default", "schema": {{"type": "str"}}, "default": "x"}}}}
        }}{extra}}}"#
    )
}

#[test]
fn fields_are_validated_defaults_filled_and_set_fields_tracked() {
    let v = validator(&base(""));
    assert_eq!(v.title(), "model-fields");
    assert_eq!(
        v.validate_value(&j(r#"{"a": "1"}"#), &lax()).unwrap(),
        output(r#"{"a": 1, "b": "x"}"#, None, &["a"])
    );
    // Extra keys are ignored by default.
    assert_eq!(
        v.validate_value(&j(r#"{"a": "1", "b": "y", "c": 3}"#), &lax())
            .unwrap(),
        output(r#"{"a": 1, "b": "y"}"#, None, &["a", "b"])
    );
    assert_eq!(
        v.validate_json(r#"{"a": "1", "c": 2}"#, &lax()).unwrap(),
        output(r#"{"a": 1, "b": "x"}"#, None, &["a"])
    );
    assert_eq!(
        errors(v.validate_value(&j("{}"), &lax())),
        vec![err("missing", r#"["a"]"#, "Field required")]
    );
}

#[test]
fn non_mappings_are_model_type_errors() {
    let v = validator(&base(""));
    for input in [s("notadict"), j("[1]")] {
        assert_eq!(
            errors(v.validate_value(&input, &lax())),
            vec![err(
                "model_type",
                "[]",
                "Input should be a valid dictionary or instance of Model"
            )]
        );
    }
    assert_eq!(
        errors(v.validate_json("[1]", &lax())),
        vec![err("model_type", "[]", "Input should be an object")]
    );
    let named = validator(&base(r#", "model_name": "M""#));
    assert_eq!(
        errors(named.validate_value(&j("[1]"), &lax()))[0].2,
        "Input should be a valid dictionary or instance of M"
    );
}

#[test]
fn extra_forbid_reports_extra_and_non_string_keys() {
    let v = validator(&base(r#", "extra_behavior": "forbid""#));
    let mut input = perldantic_core::Dict::new();
    input.insert(s("a"), Value::Int(1));
    input.insert(s("c"), Value::Int(3));
    input.insert(Value::Int(4), Value::Int(5));
    assert_eq!(
        errors(v.validate_value(&Value::Dict(input), &lax())),
        vec![
            err(
                "extra_forbidden",
                r#"["c"]"#,
                "Extra inputs are not permitted"
            ),
            err("invalid_key", "[4]", "Keys should be strings"),
        ]
    );
    // The call option overrides the schema.
    let lenient = validator(&base(""));
    assert_eq!(
        errors(lenient.validate_value(
            &j(r#"{"a": 1, "c": 3}"#),
            &ValidateOptions {
                extra_behavior: Some(perldantic_core::ExtraBehavior::Forbid),
                ..lax()
            }
        )),
        vec![err(
            "extra_forbidden",
            r#"["c"]"#,
            "Extra inputs are not permitted"
        )]
    );
}

#[test]
fn extra_allow_collects_extras() {
    let v = validator(&base(r#", "extra_behavior": "allow""#));
    assert_eq!(
        v.validate_value(&j(r#"{"a": 1, "c": 3}"#), &lax()).unwrap(),
        output(r#"{"a": 1, "b": "x"}"#, Some(r#"{"c": 3}"#), &["a", "c"])
    );
    assert_eq!(
        v.validate_json(r#"{"a": 1, "c": 3}"#, &lax()).unwrap(),
        output(r#"{"a": 1, "b": "x"}"#, Some(r#"{"c": 3}"#), &["a", "c"])
    );

    let typed = validator(&base(
        r#", "extra_behavior": "allow", "extras_schema": {"type": "int"}"#,
    ));
    assert_eq!(
        errors(typed.validate_value(&j(r#"{"a": 1, "c": "3", "d": "x"}"#), &lax())),
        vec![err("int_parsing", r#"["d"]"#, INT_PARSING)]
    );
}

#[test]
fn extras_schemas_require_extra_allow() {
    match SchemaValidator::new(&j(&base(r#", "extras_schema": {"type": "int"}"#)), None) {
        Err(e) => assert_eq!(
            e.to_string(),
            "Error building \"model-fields\" validator:\n  SchemaError: extras_schema can only be used if extra_behavior=allow"
        ),
        Ok(_) => panic!("expected a schema error"),
    }
    match SchemaValidator::new(
        &j(
            r#"{"type": "model-fields", "fields": {"a": {"type": "model-field", "schema": {"type": "bogus"}}}}"#,
        ),
        None,
    ) {
        Err(e) => assert_eq!(
            e.to_string(),
            "Error building \"model-fields\" validator:\n  SchemaError: Field \"a\":\n  SchemaError: Unknown schema type: \"bogus\""
        ),
        Ok(_) => panic!("expected a schema error"),
    }
}

const ALIASED: &str = r#"{"type": "model-fields", "fields": {"a": {"type": "model-field", "schema": {"type": "int"}, "validation_alias": "A"}}}"#;

#[test]
fn aliases_replace_names_unless_validating_by_name() {
    let v = validator(ALIASED);
    assert_eq!(
        v.validate_value(&j(r#"{"A": "1"}"#), &lax()).unwrap(),
        output(r#"{"a": 1}"#, None, &["a"])
    );
    assert_eq!(
        errors(v.validate_value(&j(r#"{"a": "1"}"#), &lax())),
        vec![err("missing", r#"["A"]"#, "Field required")]
    );
    assert_eq!(
        errors(v.validate_value(&j(r#"{"A": "x"}"#), &lax())),
        vec![err("int_parsing", r#"["A"]"#, INT_PARSING)]
    );

    let by_field = validator_with(ALIASED, Some(r#"{"loc_by_alias": false}"#));
    assert_eq!(
        errors(by_field.validate_value(&j(r#"{"A": "x"}"#), &lax())),
        vec![err("int_parsing", r#"["a"]"#, INT_PARSING)]
    );

    let by_name = validator_with(ALIASED, Some(r#"{"validate_by_name": true}"#));
    assert_eq!(
        by_name.validate_value(&j(r#"{"a": "1"}"#), &lax()).unwrap(),
        output(r#"{"a": 1}"#, None, &["a"])
    );

    match v.validate_value(
        &j(r#"{"a": "1"}"#),
        &ValidateOptions {
            by_alias: Some(false),
            by_name: Some(false),
            ..lax()
        },
    ) {
        Err(ValidateError::Core(CoreError::Value(msg))) => assert_eq!(
            msg,
            "`validate_by_name` and `validate_by_alias` cannot both be set to `False`."
        ),
        other => panic!("expected a ValueError, got {other:?}"),
    }
}

#[test]
fn alias_paths_and_choices() {
    let path = validator(
        r#"{"type": "model-fields", "fields": {"a": {"type": "model-field", "schema": {"type": "int"}, "validation_alias": ["A", 0]}}}"#,
    );
    assert_eq!(
        errors(path.validate_value(&j(r#"{"A": ["x"]}"#), &lax())),
        vec![err("int_parsing", r#"["A",0]"#, INT_PARSING)]
    );
    assert_eq!(
        errors(path.validate_value(&j("{}"), &lax())),
        vec![err("missing", r#"["A",0]"#, "Field required")]
    );
    assert_eq!(
        path.validate_json(r#"{"A": [5]}"#, &lax()).unwrap(),
        output(r#"{"a": 5}"#, None, &["a"])
    );

    // The earlier alias choice wins, in host data and JSON alike.
    let choices = validator(
        r#"{"type": "model-fields", "fields": {"a": {"type": "model-field", "schema": {"type": "int"}, "validation_alias": [["X"], ["A"]]}}}"#,
    );
    let expected = output(r#"{"a": 2}"#, None, &["a"]);
    assert_eq!(
        choices
            .validate_value(&j(r#"{"A": 1, "X": 2}"#), &lax())
            .unwrap(),
        expected
    );
    assert_eq!(
        choices
            .validate_json(r#"{"A": 1, "X": 2}"#, &lax())
            .unwrap(),
        expected
    );
}

#[test]
fn strict_mode_reaches_the_fields() {
    let v = validator(
        r#"{"type": "model-fields", "fields": {"a": {"type": "model-field", "schema": {"type": "int"}}}}"#,
    );
    assert_eq!(
        errors(v.validate_value(
            &j(r#"{"a": "1"}"#),
            &ValidateOptions {
                strict: Some(true),
                ..lax()
            }
        )),
        vec![err(
            "int_type",
            r#"["a"]"#,
            "Input should be a valid integer"
        )]
    );
}
