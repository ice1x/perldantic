//! `chain` and `json` beyond the recorded cases.

use perldantic_core::{
    ErrorsOptions, JsonOptions, SchemaSerializer, SchemaValidator, SerializeOptions, ValidateError,
    ValidateOptions, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn validate(schema: &str, input: &Value) -> Result<Value, ValidateError> {
    SchemaValidator::new(&j(schema), None)
        .unwrap()
        .validate_value(input, &ValidateOptions::default())
}

fn error_types(result: Result<Value, ValidateError>) -> Vec<String> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| d.type_)
        .collect()
}

#[test]
fn chains_feed_each_step_the_previous_output() {
    // str -> json -> list of ints: steps see the output, not the input
    let schema = r#"{"type": "chain", "steps": [
        {"type": "str", "strip_whitespace": true},
        {"type": "json", "schema": {"type": "list", "items_schema": {"type": "int"}}}
    ]}"#;
    assert_eq!(validate(schema, &j(r#"" [1, 2] ""#)).unwrap(), j("[1, 2]"));
    assert_eq!(error_types(validate(schema, &j("1"))), ["string_type"]);
    assert_eq!(
        error_types(validate(schema, &j(r#""[1, \"x\"]""#))),
        ["int_parsing"]
    );
    // nested chains are flattened; a single step is that step
    let nested = r#"{"type": "chain", "steps": [{"type": "chain", "steps": [{"type": "int"}, {"type": "float"}]}, {"type": "str", "coerce_numbers_to_str": true}]}"#;
    assert_eq!(validate(nested, &j("3")).unwrap(), Value::Str("3.0".into()));
    assert!(SchemaValidator::new(&j(r#"{"type": "chain", "steps": []}"#), None).is_err());
}

#[test]
fn json_parses_text_and_bytes() {
    let any = r#"{"type": "json"}"#;
    assert_eq!(
        validate(any, &j(r#""{\"a\": [1, null]}""#)).unwrap(),
        j(r#"{"a": [1, null]}"#)
    );
    assert_eq!(
        validate(any, &Value::Bytes(b"true".to_vec())).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(error_types(validate(any, &j("1"))), ["json_type"]);
    assert_eq!(error_types(validate(any, &j(r#""{""#))), ["json_invalid"]);
    // the inner schema validates JSON, so strings parse as JSON strings do
    let dates = r#"{"type": "json", "schema": {"type": "list", "items_schema": {"type": "date"}}}"#;
    assert_eq!(
        validate(dates, &j(r#""[\"2022-06-08\"]""#)).unwrap().repr(),
        "[datetime.date(2022, 6, 8)]"
    );
}

#[test]
fn serializers_follow_the_last_step_and_the_inner_schema() {
    let chain = SchemaSerializer::new(
        &j(r#"{"type": "chain", "steps": [{"type": "str"}, {"type": "bytes"}]}"#),
        None,
    )
    .unwrap();
    let out = chain
        .to_json(
            &Value::Bytes(b"hi".to_vec()),
            &SerializeOptions::default(),
            &JsonOptions::default(),
        )
        .unwrap();
    assert_eq!((out.output.as_str(), out.warning), (r#""hi""#, None));
    let json =
        SchemaSerializer::new(&j(r#"{"type": "json", "schema": {"type": "int"}}"#), None).unwrap();
    assert_eq!(
        json.to_json(
            &Value::Int(1),
            &SerializeOptions::default(),
            &JsonOptions::default()
        )
        .unwrap()
        .output,
        "1"
    );
}
