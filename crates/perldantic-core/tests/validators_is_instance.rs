//! `is-instance` with classes by name (docs/DIVERGENCES.md #13) and host objects.

use perldantic_core::{
    Dict, ErrorsOptions, HostObject, JsonSchemaError, JsonSchemaOptions, Model, SchemaSerializer,
    SchemaValidator, SerializeError, SerializeOptions, ValidateError, ValidateOptions, Value,
    generate_json_schema,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn validator(class: &str) -> SchemaValidator {
    SchemaValidator::new(
        &j(&format!(r#"{{"type": "is-instance", "cls": "{class}"}}"#)),
        None,
    )
    .unwrap()
}

fn host(class: &str, isa: &[&str]) -> Value {
    Value::Host(Box::new(HostObject {
        id: 1,
        class: class.into(),
        isa: isa.iter().map(|c| (*c).to_owned()).collect(),
        repr: format!("{class}=HASH(0x1)"),
    }))
}

fn error(result: Result<Value, ValidateError>) -> (String, String) {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    let details = e.errors(&ErrorsOptions::default()).remove(0);
    (details.type_, details.msg)
}

#[test]
fn host_objects_are_instances_of_their_ancestors() {
    let point = host("My::Point", &["My::Point", "My::Shape"]);
    let options = ValidateOptions::default();
    for class in ["My::Point", "My::Shape", "object"] {
        assert_eq!(
            validator(class).validate_value(&point, &options).unwrap(),
            point,
            "{class}"
        );
    }
    let v = validator("My::Circle");
    assert_eq!(v.title(), "is-instance[My::Circle]");
    assert_eq!(
        error(v.validate_value(&point, &options)),
        (
            "is_instance_of".into(),
            "Input should be an instance of My::Circle".into()
        )
    );
    // JSON has no objects
    assert_eq!(
        error(v.validate_json("{}", &options)).0,
        "needs_python_object"
    );
}

#[test]
fn builtin_values_by_type_name() {
    let options = ValidateOptions::default();
    let ok = |class: &str, value: &Value| validator(class).validate_value(value, &options).is_ok();
    assert!(ok("int", &Value::Int(1)));
    assert!(ok("int", &Value::Bool(true)), "bool is a subclass of int");
    assert!(!ok("bool", &Value::Int(1)));
    assert!(ok("str", &j(r#""x""#)));
    assert!(ok("list", &j("[1]")));
    assert!(!ok("dict", &j("[1]")));
    let model = Value::Model(std::sync::Arc::new(Model {
        class: "Point".into(),
        fields: Dict::new(),
        fields_set: vec![],
        extra: None,
    }));
    assert!(ok("Point", &model));
    assert!(!ok("Other", &model));
}

#[test]
fn class_repr_names_the_class_in_errors() {
    let v = SchemaValidator::new(
        &j(r#"{"type": "is-instance", "cls": "Point", "cls_repr": "geo.Point"}"#),
        None,
    )
    .unwrap();
    assert_eq!(
        error(v.validate_value(&Value::Int(1), &ValidateOptions::default())).1,
        "Input should be an instance of geo.Point"
    );
}

#[test]
fn host_objects_serialize_as_unknown_types() {
    let any = SchemaSerializer::new(&j(r#"{"type": "any"}"#), None).unwrap();
    let object = host("My::Point", &["My::Point"]);
    // Perl-side dumps return the object itself
    assert_eq!(
        any.to_python(&object, &SerializeOptions::default())
            .unwrap()
            .output,
        object
    );
    assert_eq!(
        any.to_json(
            &object,
            &SerializeOptions::default(),
            &perldantic_core::JsonOptions::default()
        )
        .unwrap_err(),
        SerializeError::Serialization(
            "Unable to serialize unknown type: <class 'My::Point'>".into()
        )
    );
}

#[test]
fn is_instance_schemas_serialize_by_inference() {
    let s =
        SchemaSerializer::new(&j(r#"{"type": "is-instance", "cls": "My::Point"}"#), None).unwrap();
    let object = host("My::Point", &["My::Point"]);
    assert_eq!(
        s.to_python(&object, &SerializeOptions::default())
            .unwrap()
            .output,
        object
    );
    assert_eq!(
        s.to_python(&Value::Int(1), &SerializeOptions::default())
            .unwrap()
            .output,
        Value::Int(1)
    );
}

#[test]
fn no_json_schema_for_instances() {
    let Err(JsonSchemaError::InvalidForJsonSchema(message)) = generate_json_schema(
        &j(r#"{"type": "is-instance", "cls": "My::Point"}"#),
        None,
        &JsonSchemaOptions::default(),
    ) else {
        panic!("is-instance has no JSON Schema");
    };
    assert_eq!(
        message,
        "Cannot generate a JsonSchema for core_schema.IsInstanceSchema (<class 'My::Point'>)"
    );
}
