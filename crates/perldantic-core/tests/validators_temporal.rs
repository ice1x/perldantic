//! `date`, `time`, `datetime` and `timedelta` beyond the recorded cases: the documented
//! divergence for `now_op` and the Python forms of temporal values in errors.

use perldantic_core::{
    ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value, speedate,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn validate(schema: &str, input: &Value) -> Result<Value, ValidateError> {
    SchemaValidator::new(&j(schema), None)
        .unwrap()
        .validate_value(input, &ValidateOptions::default())
}

#[test]
fn now_op_without_an_offset_compares_in_utc() {
    // docs/DIVERGENCES.md #16: upstream uses the local timezone of the process
    let schema = r#"{"type": "datetime", "now_op": "past"}"#;
    assert!(validate(schema, &j(r#""2000-01-01T00:00:00Z""#)).is_ok());
    let Err(ValidateError::Validation(e)) = validate(schema, &j(r#""2999-01-01T00:00:00Z""#))
    else {
        panic!("expected a validation error");
    };
    assert_eq!(
        e.errors(&ErrorsOptions::default())[0].type_,
        "datetime_past"
    );
}

#[test]
fn outputs_are_temporal_values() {
    assert_eq!(
        validate(r#"{"type": "date"}"#, &j(r#""2022-06-08""#)).unwrap(),
        Value::Date(speedate::Date::parse_str("2022-06-08").unwrap())
    );
    let Value::TimeDelta(d) = validate(r#"{"type": "timedelta"}"#, &j("90")).unwrap() else {
        panic!("expected a timedelta");
    };
    assert_eq!(Value::TimeDelta(d).repr(), "datetime.timedelta(seconds=90)");
}

#[test]
fn errors_show_python_reprs() {
    let time = Value::Time(speedate::Time::parse_str("12:00:00+01:00").unwrap());
    let Err(ValidateError::Validation(e)) = validate(r#"{"type": "date"}"#, &time) else {
        panic!("expected a validation error");
    };
    assert!(
        e.to_string()
            .contains("input_value=datetime.time(12, 0, tzinfo=TzInfo(3600)), input_type=time"),
        "{e}"
    );
}
