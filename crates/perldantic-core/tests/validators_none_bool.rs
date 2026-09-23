//! `none` and `bool` validators (upstream validators/none.rs, validators/bool.rs).

use perldantic_core::{ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value};

fn validator(schema: &str, config: Option<&str>) -> SchemaValidator {
    let config = config.map(|c| Value::from_json(c).unwrap());
    SchemaValidator::new(&Value::from_json(schema).unwrap(), config.as_ref()).unwrap()
}

fn first_error(result: Result<Value, ValidateError>) -> (String, String) {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    let d = &e.errors(&ErrorsOptions::default())[0];
    (d.type_.clone(), d.msg.clone())
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
}

fn strict(strict: bool) -> ValidateOptions {
    ValidateOptions {
        strict: Some(strict),
        ..ValidateOptions::default()
    }
}

#[test]
fn none_accepts_only_none() {
    let v = validator(r#"{"type": "none"}"#, None);
    assert_eq!(v.title(), "none");
    assert_eq!(v.validate_value(&Value::None, &lax()).unwrap(), Value::None);
    assert_eq!(v.validate_json("null", &lax()).unwrap(), Value::None);
    assert_eq!(
        first_error(v.validate_value(&Value::Int(1), &lax())),
        ("none_required".into(), "Input should be None".into())
    );
    assert_eq!(
        first_error(v.validate_json("1", &lax())),
        ("none_required".into(), "Input should be null".into())
    );
}

#[test]
fn bool_is_lax_by_default() {
    let v = validator(r#"{"type": "bool"}"#, None);
    assert_eq!(v.title(), "bool");
    assert_eq!(
        v.validate_value(&Value::from("yes"), &lax()).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(v.validate_json("0", &lax()).unwrap(), Value::Bool(false));
    assert_eq!(
        first_error(v.validate_value(&Value::from("maybe"), &lax())),
        (
            "bool_parsing".into(),
            "Input should be a valid boolean, unable to interpret input".into()
        )
    );
}

#[test]
fn bool_strictness_from_schema_config_and_call() {
    for v in [
        validator(r#"{"type": "bool", "strict": true}"#, None),
        validator(r#"{"type": "bool"}"#, Some(r#"{"strict": true}"#)),
    ] {
        assert_eq!(
            v.validate_value(&Value::Bool(true), &lax()).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            first_error(v.validate_value(&Value::from("yes"), &lax())),
            ("bool_type".into(), "Input should be a valid boolean".into())
        );
        // The call-level option wins over the schema.
        assert_eq!(
            v.validate_value(&Value::from("yes"), &strict(false))
                .unwrap(),
            Value::Bool(true)
        );
    }
    let v = validator(r#"{"type": "bool"}"#, None);
    assert_eq!(
        first_error(v.validate_value(&Value::Int(1), &strict(true))).0,
        "bool_type"
    );
}
