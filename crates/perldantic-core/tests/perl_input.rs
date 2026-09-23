//! Perl input (docs/DIVERGENCES.md #8): host data validated with `InputType::Perl` gets Perl
//! wording in error messages, while error codes and contexts stay pydantic's, and a Perl array
//! satisfies a strict tuple.

use perldantic_core::{
    ErrorsOptions, InputType, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&j(schema), None).unwrap()
}

fn strict() -> ValidateOptions {
    ValidateOptions {
        strict: Some(true),
        ..ValidateOptions::default()
    }
}

/// `(type, msg)` of each error.
fn errors(v: &SchemaValidator, input: &Value, input_type: InputType) -> Vec<(String, String)> {
    errors_with(v, input, input_type, &ValidateOptions::default())
}

fn errors_with(
    v: &SchemaValidator,
    input: &Value,
    input_type: InputType,
    options: &ValidateOptions,
) -> Vec<(String, String)> {
    match v.validate_value_as(input, input_type, options) {
        Err(ValidateError::Validation(e)) => e
            .errors(&ErrorsOptions::default())
            .into_iter()
            .map(|d| (d.type_, d.msg))
            .collect(),
        other => panic!("expected a validation error, got {other:?}"),
    }
}

fn one(type_: &str, msg: &str) -> Vec<(String, String)> {
    vec![(type_.to_owned(), msg.to_owned())]
}

#[test]
fn perl_input_uses_perl_words() {
    let cases = [
        (
            r#"{"type": "none"}"#,
            "1",
            "none_required",
            "Input should be undef",
        ),
        (
            r#"{"type": "list"}"#,
            "1",
            "list_type",
            "Input should be an array reference",
        ),
        (
            r#"{"type": "tuple", "items_schema": [{"type": "int"}]}"#,
            "1",
            "tuple_type",
            "Input should be an array reference",
        ),
        (
            r#"{"type": "dict"}"#,
            "1",
            "dict_type",
            "Input should be a hash reference",
        ),
        (
            r#"{"type": "model", "cls": "My::Point", "schema": {"type": "model-fields", "model_name": "My::Point", "fields": {}}}"#,
            "1",
            "model_type",
            "Input should be a hash reference or an instance of My::Point",
        ),
        (
            r#"{"type": "list", "min_length": 2}"#,
            "[1]",
            "too_short",
            "Array should have at least 2 items after validation, not 1",
        ),
        (
            r#"{"type": "dict", "max_length": 1}"#,
            r#"{"a": 1, "b": 2}"#,
            "too_long",
            "Hash should have at most 1 item after validation, not 2",
        ),
        (
            r#"{"type": "int"}"#,
            r#""x""#,
            "int_parsing",
            "Input should be a valid integer, unable to parse string as an integer",
        ),
    ];
    for (schema, input, type_, msg) in cases {
        assert_eq!(
            errors(&validator(schema), &j(input), InputType::Perl),
            one(type_, msg),
            "{schema}"
        );
    }
}

#[test]
fn python_input_keeps_pydantic_words() {
    assert_eq!(
        errors(
            &validator(r#"{"type": "list"}"#),
            &j("1"),
            InputType::Python
        ),
        one("list_type", "Input should be a valid list")
    );
    assert_eq!(
        errors(
            &validator(r#"{"type": "none"}"#),
            &j("1"),
            InputType::Python
        ),
        one("none_required", "Input should be None")
    );
}

#[test]
fn contexts_stay_pydantic() {
    let v = validator(r#"{"type": "list", "min_length": 2}"#);
    let Err(ValidateError::Validation(e)) =
        v.validate_value_as(&j("[1]"), InputType::Perl, &ValidateOptions::default())
    else {
        panic!("expected an error");
    };
    let ctx = e.errors(&ErrorsOptions::default())[0].ctx.clone().unwrap();
    assert_eq!(
        ctx,
        j(r#"{"field_type": "List", "min_length": 2, "actual_length": 1}"#)
    );
}

#[test]
fn perl_arrays_are_strict_tuples() {
    let v = validator(r#"{"type": "tuple", "items_schema": [{"type": "int"}, {"type": "str"}]}"#);
    let array = j(r#"[1, "a"]"#);
    assert_eq!(
        v.validate_value_as(&array, InputType::Perl, &strict())
            .unwrap(),
        Value::Tuple(vec![Value::Int(1), Value::from("a")])
    );
    assert_eq!(
        errors_with(&v, &array, InputType::Python, &strict()),
        one("tuple_type", "Input should be a valid tuple")
    );
    // Only arrays: strict still refuses other kinds of input.
    assert_eq!(
        errors_with(&v, &j(r#"{"a": 1}"#), InputType::Perl, &strict()),
        one("tuple_type", "Input should be an array reference")
    );
}

#[test]
fn validate_value_is_python_input() {
    let v = validator(r#"{"type": "none"}"#);
    let Err(ValidateError::Validation(e)) = v.validate_value(&j("1"), &ValidateOptions::default())
    else {
        panic!("expected an error");
    };
    assert_eq!(e.input_type(), InputType::Python);
    assert_eq!(InputType::try_from("perl"), Ok(InputType::Perl));
    assert_eq!(InputType::Perl.as_str(), "perl");
}
