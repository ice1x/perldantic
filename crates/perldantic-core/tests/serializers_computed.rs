//! Computed fields of models and typed dicts, computed by host functions
//! (docs/DIVERGENCES.md #20); outputs follow pydantic-core 2.49.

use std::fmt;

use perldantic_core::{
    CoreError, Dict, Function, HostCall, HostError, HostException, HostFunction, JsonOptions,
    JsonSchemaMode, JsonSchemaOptions, Model, SchemaSerializer, SerializeError, SerializeOptions,
    Value, generate_json_schema,
};

type Body = dyn Fn(Value, &str) -> Result<Value, HostError> + Send + Sync;

/// A getter: computes a property from the model.
struct Getter(Box<Body>);

impl fmt::Debug for Getter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Getter")
    }
}

impl HostFunction for Getter {
    fn name(&self) -> &'static str {
        "getter"
    }

    fn call(&self, call: HostCall<'_>) -> Result<Value, HostError> {
        match call {
            HostCall::Property { model, name } => (self.0)(model, &name),
            _ => panic!("a property getter"),
        }
    }
}

fn getter(body: impl Fn(Value, &str) -> Result<Value, HostError> + Send + Sync + 'static) -> Value {
    Value::Function(Function::new(Getter(Box::new(body))))
}

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

/// The field `name` of a model or dict.
fn field(model: &Value, name: &str) -> Value {
    let fields = match model {
        Value::Model(model) => &model.fields,
        Value::Dict(dict) => dict,
        other => panic!("not a model: {other:?}"),
    };
    fields
        .iter()
        .find(|(k, _)| **k == Value::from(name))
        .map(|(_, v)| v.clone())
        .unwrap()
}

/// width * height, the model's `area`.
fn area() -> Value {
    getter(|model, name| {
        assert_eq!(name, "area");
        match (field(&model, "width"), field(&model, "height")) {
            (Value::Int(w), Value::Int(h)) => Ok(Value::Int(w * h)),
            other => panic!("{other:?}"),
        }
    })
}

fn computed(name: &str, function: Value, extra: &[(&str, Value)]) -> Value {
    let mut field = Dict::new();
    field.insert("type".into(), "computed-field".into());
    field.insert("property_name".into(), name.into());
    field.insert("return_schema".into(), j(r#"{"type": "int"}"#));
    field.insert("function".into(), function);
    for (k, v) in extra {
        field.insert((*k).into(), v.clone());
    }
    Value::Dict(field)
}

fn rectangle_schema(computed_fields: Vec<Value>) -> Value {
    let Value::Dict(mut fields) = j(r#"{"type": "model-fields", "fields": {
        "width": {"type": "model-field", "schema": {"type": "int"}},
        "height": {"type": "model-field", "schema": {"type": "int"}}
    }}"#) else {
        unreachable!()
    };
    fields.insert("computed_fields".into(), Value::List(computed_fields));
    let mut model = Dict::new();
    model.insert("type".into(), "model".into());
    model.insert("cls".into(), "Rectangle".into());
    model.insert("schema".into(), Value::Dict(fields));
    Value::Dict(model)
}

fn rectangle(width: i64, height: i64) -> Value {
    let mut fields = Dict::new();
    fields.insert("width".into(), Value::Int(width));
    fields.insert("height".into(), Value::Int(height));
    Value::Model(Box::new(Model {
        class: "Rectangle".into(),
        fields,
        fields_set: vec!["width".into(), "height".into()],
        extra: None,
    }))
}

fn to_json(s: &SchemaSerializer, value: &Value, options: &SerializeOptions) -> String {
    s.to_json(value, options, &JsonOptions::default())
        .unwrap()
        .output
}

#[test]
fn computed_fields_follow_the_fields() {
    let s = SchemaSerializer::new(&rectangle_schema(vec![computed("area", area(), &[])]), None)
        .unwrap();
    let defaults = SerializeOptions::default();
    assert_eq!(
        to_json(&s, &rectangle(3, 4), &defaults),
        r#"{"width":3,"height":4,"area":12}"#
    );
    assert_eq!(
        s.to_python(&rectangle(2, 5), &defaults).unwrap().output,
        j(r#"{"width": 2, "height": 5, "area": 10}"#)
    );
    // include / exclude apply to computed fields by name
    let exclude = SerializeOptions {
        exclude: Some(Value::Set(vec!["area".into()])),
        ..SerializeOptions::default()
    };
    assert_eq!(
        to_json(&s, &rectangle(3, 4), &exclude),
        r#"{"width":3,"height":4}"#
    );
    let include = SerializeOptions {
        include: Some(Value::Set(vec!["area".into()])),
        ..SerializeOptions::default()
    };
    assert_eq!(to_json(&s, &rectangle(3, 4), &include), r#"{"area":12}"#);
}

#[test]
fn aliases_and_none_values() {
    let s = SchemaSerializer::new(
        &rectangle_schema(vec![
            computed("area", area(), &[("alias", "Area".into())]),
            computed("nothing", getter(|_, _| Ok(Value::None)), &[]),
        ]),
        None,
    )
    .unwrap();
    let by_alias = SerializeOptions {
        by_alias: Some(true),
        exclude_none: true,
        ..SerializeOptions::default()
    };
    assert_eq!(
        to_json(&s, &rectangle(1, 1), &by_alias),
        r#"{"width":1,"height":1,"Area":1}"#
    );
    assert_eq!(
        to_json(&s, &rectangle(1, 1), &SerializeOptions::default()),
        r#"{"width":1,"height":1,"area":1,"nothing":null}"#
    );
}

#[test]
fn getter_failures() {
    let raise = getter(|_, _| {
        Err(HostError::Other(HostException::new(
            "AttributeError: nope",
            7_u64,
        )))
    });
    let s =
        SchemaSerializer::new(&rectangle_schema(vec![computed("area", raise, &[])]), None).unwrap();
    let Err(SerializeError::Core(CoreError::Host(exception))) =
        s.to_python(&rectangle(1, 1), &SerializeOptions::default())
    else {
        panic!("the getter's own exception");
    };
    assert_eq!(exception.payload().downcast_ref::<u64>(), Some(&7));
}

#[test]
fn typed_dicts_have_computed_fields_too() {
    let Value::Dict(mut schema) = j(r#"{"type": "typed-dict", "fields": {
        "width": {"type": "typed-dict-field", "schema": {"type": "int"}},
        "height": {"type": "typed-dict-field", "schema": {"type": "int"}}
    }}"#) else {
        unreachable!()
    };
    // pydantic marks computed fields read-only through metadata, as the Perl layer does
    let read_only = j(r#"{"pydantic_js_updates": {"readOnly": true}}"#);
    schema.insert(
        "computed_fields".into(),
        Value::List(vec![computed("area", area(), &[("metadata", read_only)])]),
    );
    let schema = Value::Dict(schema);
    let s = SchemaSerializer::new(&schema, None).unwrap();
    assert_eq!(
        to_json(
            &s,
            &j(r#"{"width": 2, "height": 2}"#),
            &SerializeOptions::default()
        ),
        r#"{"width":2,"height":2,"area":4}"#
    );

    let options = JsonSchemaOptions {
        mode: JsonSchemaMode::Serialization,
        ..JsonSchemaOptions::default()
    };
    let generated = generate_json_schema(&schema, None, &options)
        .unwrap()
        .schema;
    let Value::Dict(generated) = generated else {
        unreachable!()
    };
    assert_eq!(
        generated.get_str("properties").cloned(),
        Some(j(r#"{
            "width": {"type": "integer", "title": "Width"},
            "height": {"type": "integer", "title": "Height"},
            "area": {"type": "integer", "title": "Area", "readOnly": true}
        }"#))
    );
    assert_eq!(
        generated.get_str("required").cloned(),
        Some(j(r#"["width", "height", "area"]"#))
    );
}
