//! `uuid` beyond the recorded cases: the Python forms of UUID values, Perl input in strict
//! mode, serialization and the JSON Schema.

use perldantic_core::{
    ErrorsOptions, InputType, JsonOptions, JsonSchemaOptions, SchemaSerializer, SchemaValidator,
    SerMode, SerializeOptions, ValidateError, ValidateOptions, Value, generate_json_schema,
    uuid::Uuid,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn uuid(text: &str) -> Value {
    Value::Uuid(Uuid::parse_str(text).unwrap())
}

const U: &str = "12345678-1234-5678-1234-567812345678";

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&j(schema), None).unwrap()
}

fn error_types(result: Result<Value, ValidateError>) -> Vec<(String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| (d.type_, d.msg))
        .collect()
}

#[test]
fn uuid_values_look_like_python() {
    let value = uuid(U);
    assert_eq!(value.repr(), format!("UUID('{U}')"));
    assert_eq!(value.py_str(), U);
    assert_eq!(value.type_name(), "UUID");
    assert_eq!(value, uuid(&U.to_uppercase()));
    assert_ne!(value, Value::Str(U.to_owned()));
}

#[test]
fn strings_bytes_and_uuids_validate_to_uuids() {
    let v = validator(r#"{"type": "uuid"}"#);
    let opts = ValidateOptions::default();
    assert_eq!(
        v.validate_value(&Value::Str(U.into()), &opts).unwrap(),
        uuid(U)
    );
    assert_eq!(v.validate_value(&uuid(U), &opts).unwrap(), uuid(U));
    assert_eq!(
        v.validate_value(
            &Value::Bytes(Uuid::parse_str(U).unwrap().as_bytes().to_vec()),
            &opts
        )
        .unwrap(),
        uuid(U)
    );
    assert_eq!(
        v.validate_json(&format!("\"{U}\""), &opts).unwrap(),
        uuid(U)
    );
}

#[test]
fn strict_host_input_must_be_a_uuid() {
    let v = validator(r#"{"type": "uuid", "strict": true}"#);
    let opts = ValidateOptions::default();
    for (input_type, class) in [
        (InputType::Python, "UUID"),
        (InputType::Perl, "Perldantic::Uuid"),
    ] {
        assert_eq!(
            error_types(v.validate_value_as(&Value::Str(U.into()), input_type, &opts)),
            [(
                "is_instance_of".to_owned(),
                format!("Input should be an instance of {class}")
            )]
        );
        assert_eq!(
            v.validate_value_as(&uuid(U), input_type, &opts).unwrap(),
            uuid(U)
        );
    }
    // JSON has no UUID type: a string is an exact match
    assert_eq!(
        v.validate_json(&format!("\"{U}\""), &opts).unwrap(),
        uuid(U)
    );
}

#[test]
fn version_constraints_check_the_variant() {
    let v = validator(r#"{"type": "uuid", "version": 4}"#);
    let opts = ValidateOptions::default();
    let not_rfc = "00000000-7fff-4000-7fff-000000000000";
    for input in [Value::Str(not_rfc.into()), uuid(not_rfc)] {
        assert_eq!(
            error_types(v.validate_value(&input, &opts)),
            [(
                "uuid_version".to_owned(),
                "UUID version 4 expected".to_owned()
            )]
        );
    }
    let rfc = "00000000-8000-4000-8000-000000000000";
    assert_eq!(v.validate_value(&uuid(rfc), &opts).unwrap(), uuid(rfc));
}

#[test]
fn errors_show_python_reprs() {
    let v = validator(r#"{"type": "int"}"#);
    let Err(ValidateError::Validation(e)) = v.validate_value(&uuid(U), &ValidateOptions::default())
    else {
        panic!("expected a validation error");
    };
    assert!(
        e.to_string()
            .contains(&format!("input_value=UUID('{U}'), input_type=UUID")),
        "{e}"
    );
}

#[test]
fn uuids_serialize_as_strings_in_json_mode() {
    let s = SchemaSerializer::new(&j(r#"{"type": "uuid"}"#), None).unwrap();
    let python = SerializeOptions::default();
    let json = SerializeOptions {
        mode: SerMode::Json,
        ..SerializeOptions::default()
    };
    let to_python = |s: &SchemaSerializer, v: &Value, o| s.to_python(v, o).unwrap().output;
    let to_json = |s: &SchemaSerializer, v: &Value| {
        s.to_json(v, &SerializeOptions::default(), &JsonOptions::default())
            .unwrap()
            .output
    };
    assert_eq!(to_python(&s, &uuid(U), &python), uuid(U));
    assert_eq!(to_python(&s, &uuid(U), &json), Value::Str(U.into()));
    assert_eq!(to_json(&s, &uuid(U)), format!("\"{U}\""));

    // inferred, e.g. under `any`, and as dict keys
    let any = SchemaSerializer::new(&j(r#"{"type": "any"}"#), None).unwrap();
    let dict = Value::Dict([(uuid(U), uuid(U))].into_iter().collect());
    assert_eq!(to_json(&any, &dict), format!("{{\"{U}\":\"{U}\"}}"));
    assert_eq!(to_python(&any, &uuid(U), &json), Value::Str(U.into()));
    // a value of another type warns and falls back to inference
    let warned = s.to_python(&Value::Int(1), &python).unwrap();
    assert_eq!(warned.output, Value::Int(1));
    assert!(warned.warning.unwrap().contains("Expected `uuid`"));
}

#[test]
fn json_schema_is_a_uuid_string() {
    let generated = generate_json_schema(
        &j(r#"{"type": "uuid"}"#),
        None,
        &JsonSchemaOptions::default(),
    )
    .unwrap();
    assert_eq!(
        generated.schema,
        j(r#"{"type": "string", "format": "uuid"}"#)
    );
}
