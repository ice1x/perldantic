//! `decimal` beyond the recorded cases: Python forms of decimals, Perl input, decimals given to
//! the other number validators, serialization and the JSON Schema.

use perldantic_core::{
    Decimal, ErrorsOptions, InputType, JsonOptions, JsonSchemaMode, JsonSchemaOptions,
    SchemaSerializer, SchemaValidator, SerMode, SerializeOptions, ValidateError, ValidateOptions,
    Value, generate_json_schema,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn dec(text: &str) -> Value {
    Value::Decimal(Box::new(Decimal::parse(text).unwrap()))
}

fn validate(schema: &str, input: &Value) -> Result<Value, ValidateError> {
    SchemaValidator::new(&j(schema), None)
        .unwrap()
        .validate_value(input, &ValidateOptions::default())
}

fn first_error(result: Result<Value, ValidateError>) -> (String, String, Option<Value>) {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    let details = e.errors(&ErrorsOptions::default()).remove(0);
    (details.type_, details.msg, details.ctx)
}

#[test]
fn decimals_look_like_python() {
    let value = dec("1.50");
    assert_eq!(value.repr(), "Decimal('1.50')");
    assert_eq!(value.py_str(), "1.50");
    assert_eq!(value.type_name(), "Decimal");
    assert_ne!(value, dec("1.5"), "values keep their representation");
    assert!(value.py_eq(&dec("1.5")), "Python's == compares numbers");
    assert!(dec("2").py_eq(&Value::Int(2)));
    assert!(dec("0.5").py_eq(&Value::Float(0.5)));
    assert!(!dec("0.1").py_eq(&Value::Float(0.1)));
}

#[test]
fn inputs_become_decimals() {
    let schema = r#"{"type": "decimal"}"#;
    assert_eq!(validate(schema, &j(r#""1.50""#)).unwrap(), dec("1.50"));
    assert_eq!(
        validate(schema, &j("0.1")).unwrap(),
        dec("0.1"),
        "floats via str()"
    );
    assert_eq!(validate(schema, &j("12")).unwrap(), dec("12"));
    let tuple = Value::Tuple(vec![
        Value::Int(1),
        Value::Tuple(vec![Value::Int(1), Value::Int(5)]),
        Value::Int(-1),
    ]);
    assert_eq!(validate(schema, &tuple).unwrap(), dec("-1.5"));
    assert_eq!(
        first_error(validate(schema, &Value::Bool(true))).0,
        "decimal_type"
    );
    assert_eq!(
        first_error(validate(schema, &j(r#""x""#))).1,
        "Input should be a valid decimal"
    );
}

#[test]
fn strict_perl_input_must_be_a_decimal() {
    let v = SchemaValidator::new(&j(r#"{"type": "decimal", "strict": true}"#), None).unwrap();
    let opts = ValidateOptions::default();
    let result = v.validate_value_as(&j(r#""1.5""#), InputType::Perl, &opts);
    assert_eq!(
        first_error(result).1,
        "Input should be an instance of Math::BigFloat"
    );
    assert_eq!(
        v.validate_value_as(&dec("1.5"), InputType::Perl, &opts)
            .unwrap(),
        dec("1.5")
    );
    assert_eq!(v.validate_json("1.5", &opts).unwrap(), dec("1.5"));
}

#[test]
fn bounds_report_decimals_in_their_context() {
    let (type_, msg, ctx) = first_error(validate(
        r#"{"type": "decimal", "le": "1.0"}"#,
        &j(r#""1.5""#),
    ));
    assert_eq!(type_, "less_than_equal");
    assert_eq!(msg, "Input should be less than or equal to 1.0");
    assert_eq!(
        ctx.unwrap(),
        Value::Dict([(Value::from("le"), dec("1.0"))].into_iter().collect())
    );
    let (type_, _, _) = first_error(validate(
        r#"{"type": "decimal", "gt": 0, "allow_inf_nan": true}"#,
        &dec("NaN"),
    ));
    assert_eq!(type_, "greater_than", "NaN fails every bound");
}

#[test]
fn other_number_validators_take_decimals() {
    assert_eq!(
        validate(r#"{"type": "int"}"#, &dec("1E+2")).unwrap(),
        Value::Int(100)
    );
    assert_eq!(
        first_error(validate(r#"{"type": "int"}"#, &dec("1.5"))).0,
        "int_from_float"
    );
    assert_eq!(
        first_error(validate(r#"{"type": "int", "strict": true}"#, &dec("1"))).0,
        "int_type"
    );
    // float() works on decimals even in strict mode, as upstream extracts an f64
    assert_eq!(
        validate(r#"{"type": "float", "strict": true}"#, &dec("0.25")).unwrap(),
        Value::Float(0.25)
    );
    assert_eq!(
        validate(r#"{"type": "bool"}"#, &dec("1")).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        validate(
            r#"{"type": "str", "coerce_numbers_to_str": true}"#,
            &dec("1.50")
        )
        .unwrap(),
        Value::Str("1.50".into())
    );
    assert_eq!(
        validate(r#"{"type": "timedelta"}"#, &dec("90.5"))
            .unwrap()
            .py_str(),
        "0:01:30.500000"
    );
}

#[test]
fn decimals_serialize_as_text_in_json() {
    let s = SchemaSerializer::new(&j(r#"{"type": "decimal"}"#), None).unwrap();
    let json_mode = SerializeOptions {
        mode: SerMode::Json,
        ..SerializeOptions::default()
    };
    assert_eq!(
        s.to_python(&dec("1.50"), &SerializeOptions::default())
            .unwrap()
            .output,
        dec("1.50")
    );
    assert_eq!(
        s.to_python(&dec("1.50"), &json_mode).unwrap().output,
        Value::Str("1.50".into())
    );
    assert_eq!(
        s.to_json(
            &dec("-1E+2"),
            &SerializeOptions::default(),
            &JsonOptions::default()
        )
        .unwrap()
        .output,
        r#""-1E+2""#
    );
}

#[test]
fn json_schema_is_a_number_or_a_string() {
    let schema = |mode| {
        generate_json_schema(
            &j(r#"{"type": "decimal", "ge": "0", "multiple_of": "0.01"}"#),
            None,
            &JsonSchemaOptions {
                mode,
                ..JsonSchemaOptions::default()
            },
        )
        .unwrap()
        .schema
    };
    assert_eq!(
        schema(JsonSchemaMode::Validation),
        j(
            r#"{"anyOf": [{"type": "number", "multipleOf": 0.01, "minimum": 0.0}, {"type": "string"}]}"#
        )
    );
    assert_eq!(
        schema(JsonSchemaMode::Serialization),
        j(r#"{"type": "string"}"#)
    );
}
