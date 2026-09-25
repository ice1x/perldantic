//! `arguments` schemas beyond the recorded cases: the argument values, their repr, and Perl
//! input; expected values come from pydantic-core 2.49 with Python 3.12.

use perldantic_core::{
    ArgsKwargs, Dict, ErrorsOptions, InputType, LocItem, SchemaSerializer, SchemaValidator,
    ValidateError, ValidateOptions, Value,
};

fn j(text: &str) -> Value {
    Value::from_json(text).unwrap()
}

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&j(schema), None).unwrap()
}

fn args(args: Vec<Value>, kwargs: &[(&str, Value)]) -> Value {
    let kwargs: Dict = kwargs
        .iter()
        .map(|(k, v)| (Value::from(*k), v.clone()))
        .collect();
    Value::ArgsKwargs(Box::new(ArgsKwargs::new(args, kwargs)))
}

/// The `(args, kwargs)` tuple a validation returns.
fn out(positional: Vec<Value>, kwargs: &[(&str, Value)]) -> Value {
    Value::Tuple(vec![
        Value::Tuple(positional),
        Value::Dict(
            kwargs
                .iter()
                .map(|(k, v)| (Value::from(*k), v.clone()))
                .collect(),
        ),
    ])
}

const ADD: &str = r#"{"type": "arguments", "arguments_schema": [
    {"name": "a", "mode": "positional_only", "schema": {"type": "int"}},
    {"name": "b", "schema": {"type": "int"}},
    {"name": "c", "mode": "keyword_only", "schema": {"type": "default", "schema": {"type": "str"}, "default": "x"}}
]}"#;

/// `(type, loc, msg)` of each error.
fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| {
            let loc: Vec<String> = d
                .loc
                .iter()
                .map(|item| match item {
                    LocItem::S(s) => format!("'{s}'"),
                    LocItem::I(i) => i.to_string(),
                })
                .collect();
            (d.type_, format!("[{}]", loc.join(", ")), d.msg)
        })
        .collect()
}

#[test]
fn args_kwargs_values() {
    let value = args(vec![Value::Int(1)], &[("a", Value::from("x"))]);
    assert_eq!(value.repr(), "ArgsKwargs((1,), {'a': 'x'})");
    assert_eq!(args(vec![], &[]).repr(), "ArgsKwargs(())");
    assert_eq!(value.type_name(), "ArgsKwargs");
    assert!(value.py_eq(&args(vec![Value::Float(1.0)], &[("a", Value::from("x"))])));
    assert!(!value.py_eq(&args(vec![Value::Int(1)], &[])));
}

#[test]
fn arguments_come_from_args_kwargs_tuples_lists_and_dicts() {
    let v = validator(ADD);
    let opts = ValidateOptions::default();
    let expected = out(
        vec![Value::Int(1)],
        &[("b", Value::Int(2)), ("c", Value::from("x"))],
    );
    assert_eq!(
        v.validate_value(
            &args(vec![Value::Int(1)], &[("b", Value::from("2"))]),
            &opts
        )
        .unwrap(),
        expected
    );
    assert_eq!(
        v.validate_value(&j("[1, 2]"), &opts).unwrap(),
        out(
            vec![Value::Int(1), Value::Int(2)],
            &[("c", Value::from("x"))]
        )
    );
    assert_eq!(
        v.validate_json("[1, 2]", &opts).unwrap(),
        out(
            vec![Value::Int(1), Value::Int(2)],
            &[("c", Value::from("x"))]
        )
    );
    assert_eq!(
        errors(v.validate_value(&j(r#"{"b": 2}"#), &opts)),
        vec![(
            "missing_positional_only_argument".into(),
            "[0]".into(),
            "Missing required positional only argument".into()
        )]
    );
}

#[test]
fn argument_errors() {
    let v = validator(ADD);
    let opts = ValidateOptions::default();
    let input = args(
        vec![Value::Int(1), Value::Int(2), Value::Int(3)],
        &[("b", Value::Int(4)), ("d", Value::Int(5))],
    );
    assert_eq!(
        errors(v.validate_value(&input, &opts)),
        vec![
            (
                "multiple_argument_values".into(),
                "['b']".into(),
                "Got multiple values for argument".into()
            ),
            (
                "unexpected_positional_argument".into(),
                "[2]".into(),
                "Unexpected positional argument".into()
            ),
            (
                "unexpected_keyword_argument".into(),
                "['d']".into(),
                "Unexpected keyword argument".into()
            ),
        ]
    );
    assert_eq!(
        errors(v.validate_value(&Value::Int(1), &opts)),
        vec![(
            "arguments_type".into(),
            "[]".into(),
            "Arguments must be a tuple, list or a dictionary".into()
        )]
    );
}

#[test]
fn perl_input_takes_arrays_and_hashes() {
    let v = validator(ADD);
    let opts = ValidateOptions::default();
    assert_eq!(
        v.validate_value_as(&j(r#"[1, "2"]"#), InputType::Perl, &opts)
            .unwrap(),
        out(
            vec![Value::Int(1), Value::Int(2)],
            &[("c", Value::from("x"))]
        )
    );
    assert_eq!(
        errors(v.validate_value_as(&Value::Int(1), InputType::Perl, &opts))[0].2,
        "Arguments must be an array reference, a hash reference or a Perldantic::Arguments object"
    );
}

#[test]
fn arguments_need_a_custom_serializer() {
    let err = SchemaSerializer::new(&j(ADD), None)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("`arguments` validators require a custom serializer"),
        "{err}"
    );
}
