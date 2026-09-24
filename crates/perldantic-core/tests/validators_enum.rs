//! `enum` schemas and enum members beyond the recorded cases; expected values come from
//! pydantic-core 2.49 with Python 3.12.

use perldantic_core::{
    Dict, EnumMember, EnumMixin, ErrorsOptions, JsonOptions, JsonSchemaOptions, SchemaSerializer,
    SchemaValidator, SerMode, SerializeOptions, ValidateError, ValidateOptions, Value,
    generate_json_schema,
};

fn member(class: &str, name: &str, value: Value, mixin: Option<EnumMixin>) -> Value {
    Value::Enum(Box::new(EnumMember {
        class: class.into(),
        name: name.into(),
        value,
        mixin,
        // IntEnum / StrEnum print their value; `class E(str, Enum)` prints `E.NAME`
        str_is_value: mixin == Some(EnumMixin::Int),
    }))
}

fn red() -> Value {
    member("Color", "RED", Value::Int(1), None)
}
fn green() -> Value {
    member("Color", "GREEN", Value::from("g"), None)
}
fn one() -> Value {
    member("Num", "ONE", Value::Int(1), Some(EnumMixin::Int))
}
fn two() -> Value {
    member("Num", "TWO", Value::Int(2), Some(EnumMixin::Int))
}
fn text_a() -> Value {
    member("Txt", "A", Value::from("a"), Some(EnumMixin::Str))
}
fn half() -> Value {
    member("Real", "HALF", Value::Float(0.5), Some(EnumMixin::Float))
}

fn schema(class: &str, members: Vec<Value>, sub_type: Option<&str>) -> Value {
    let mut schema = Dict::new();
    schema.insert("type".into(), "enum".into());
    schema.insert("cls".into(), class.into());
    schema.insert("members".into(), Value::List(members));
    if let Some(sub_type) = sub_type {
        schema.insert("sub_type".into(), sub_type.into());
    }
    Value::Dict(schema)
}

fn color() -> Value {
    schema("Color", vec![red(), green()], None)
}
fn num() -> Value {
    schema("Num", vec![one(), two()], Some("int"))
}

fn strict() -> ValidateOptions {
    ValidateOptions {
        strict: Some(true),
        ..ValidateOptions::default()
    }
}

/// `(type, msg)` of the first error.
fn error(result: Result<Value, ValidateError>) -> (String, String) {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    let details = e.errors(&ErrorsOptions::default()).remove(0);
    (details.type_, details.msg)
}

fn err(type_: &str, msg: &str) -> (String, String) {
    (type_.to_owned(), msg.to_owned())
}

#[test]
fn members_repr_and_compare_like_python() {
    assert_eq!(red().repr(), "<Color.RED: 1>");
    assert_eq!(half().repr(), "<Real.HALF: 0.5>");
    assert_eq!(red().type_name(), "Color");
    assert_eq!(one().py_str(), "1");
    assert_eq!(
        member("Txt", "A", Value::from("a"), Some(EnumMixin::Str)).py_str(),
        "Txt.A"
    );
    // members are singletons; mixed-in members equal their value
    assert!(red().py_eq(&red()));
    assert!(!red().py_eq(&Value::Int(1)));
    assert!(one().py_eq(&Value::Int(1)));
    assert!(!red().py_eq(&member("Other", "RED", Value::Int(1), None)));
}

#[test]
fn plain_enums_match_values_laxly() {
    let v = SchemaValidator::new(&color(), None).unwrap();
    assert_eq!(v.title(), "enum[Color]");
    let lax = ValidateOptions::default();
    for (input, expected) in [
        (red(), red()),
        (Value::Int(1), red()),
        (Value::from("g"), green()),
        (Value::Float(1.0), red()),
        (Value::Bool(true), red()),
    ] {
        assert_eq!(
            v.validate_value(&input, &lax).unwrap(),
            expected,
            "{input:?}"
        );
    }
    assert_eq!(
        error(v.validate_value(&Value::Int(3), &lax)),
        err("enum", "Input should be 1 or 'g'")
    );
    // strict Python input must be a member; strict JSON still looks values up
    assert_eq!(v.validate_value(&red(), &strict()).unwrap(), red());
    assert_eq!(
        error(v.validate_value(&Value::Int(1), &strict())),
        err("is_instance_of", "Input should be an instance of Color")
    );
    assert_eq!(v.validate_json("1", &strict()).unwrap(), red());
    assert_eq!(v.validate_json(r#""g""#, &lax).unwrap(), green());
}

#[test]
fn sub_typed_enums_validate_as_their_type() {
    let n = SchemaValidator::new(&num(), None).unwrap();
    assert_eq!(n.title(), "int-enum[Num]");
    let lax = ValidateOptions::default();
    for input in [Value::Int(2), Value::from("2"), Value::Float(2.0)] {
        assert_eq!(n.validate_value(&input, &lax).unwrap(), two(), "{input:?}");
    }
    assert_eq!(
        error(n.validate_value(&Value::from("2"), &strict())),
        err("is_instance_of", "Input should be an instance of Num")
    );
    assert_eq!(n.validate_json("2", &strict()).unwrap(), two());
    assert_eq!(
        error(n.validate_json(r#""2""#, &strict())),
        err("enum", "Input should be 1 or 2")
    );

    let t = SchemaValidator::new(&schema("Txt", vec![text_a()], Some("str")), None).unwrap();
    assert_eq!(t.validate_value(&Value::from("a"), &lax).unwrap(), text_a());
    assert_eq!(
        t.validate_value(&Value::Bytes(b"a".to_vec()), &lax)
            .unwrap(),
        text_a()
    );
    assert_eq!(
        error(t.validate_value(&Value::from("b"), &lax)),
        err("enum", "Input should be 'a'")
    );

    let r = SchemaValidator::new(&schema("Real", vec![half()], Some("float")), None).unwrap();
    assert_eq!(r.validate_value(&Value::from("0.5"), &lax).unwrap(), half());
    assert_eq!(
        error(r.validate_value(&Value::Int(1), &lax)),
        err("enum", "Input should be 0.5")
    );
}

#[test]
fn members_are_accepted_by_their_value_types() {
    let validate = |schema: &str, input: &Value, options: &ValidateOptions| {
        SchemaValidator::new(&Value::from_json(schema).unwrap(), None)
            .unwrap()
            .validate_value(input, options)
    };
    let lax = ValidateOptions::default();
    let int = r#"{"type": "int"}"#;
    assert_eq!(validate(int, &one(), &lax).unwrap(), Value::Int(1));
    assert_eq!(validate(int, &one(), &strict()).unwrap(), Value::Int(1));
    assert_eq!(validate(int, &red(), &lax).unwrap(), Value::Int(1));
    assert_eq!(
        error(validate(int, &red(), &strict())),
        err("int_type", "Input should be a valid integer")
    );
    let str_ = r#"{"type": "str"}"#;
    assert_eq!(
        validate(str_, &text_a(), &strict()).unwrap(),
        Value::from("a")
    );
    assert_eq!(validate(str_, &green(), &lax).unwrap(), Value::from("g"));
    assert_eq!(validate(str_, &red(), &lax).unwrap(), Value::from("1"));
    assert_eq!(
        validate(r#"{"type": "float"}"#, &two(), &strict()).unwrap(),
        Value::Float(2.0)
    );
    assert_eq!(
        validate(r#"{"type": "literal", "expected": [1]}"#, &one(), &lax).unwrap(),
        Value::Int(1)
    );
}

#[test]
fn invalid_schemas_are_rejected() {
    let message = |schema: Value| SchemaValidator::new(&schema, None).unwrap_err().to_string();
    assert!(message(schema("Color", vec![], None)).contains("`members` should have length > 0"));
    assert!(
        message(schema("Color", vec![red()], Some("list")))
            .contains("`sub_type` must be one of: 'int', 'str', 'float' or None")
    );
    let Value::Dict(mut missing) = color() else {
        unreachable!()
    };
    missing.insert("missing".into(), "hook".into());
    assert!(message(Value::Dict(missing)).contains("host callbacks are not implemented"));
}

#[test]
fn serializers_keep_members_and_dump_values() {
    let s = SchemaSerializer::new(&num(), None).unwrap();
    let python = SerializeOptions::default();
    let json_mode = SerializeOptions {
        mode: SerMode::Json,
        ..SerializeOptions::default()
    };
    assert_eq!(s.to_python(&one(), &python).unwrap().output, one());
    assert_eq!(
        s.to_python(&one(), &json_mode).unwrap().output,
        Value::Int(1)
    );
    let json = |s: &SchemaSerializer, value: &Value| {
        s.to_json(value, &SerializeOptions::default(), &JsonOptions::default())
            .unwrap()
    };
    assert_eq!(json(&s, &one()).output, "1");

    let plain = SchemaSerializer::new(&color(), None).unwrap();
    assert_eq!(json(&plain, &green()).output, r#""g""#);
    let fallback = json(&plain, &Value::Int(1));
    assert_eq!(fallback.output, "1");
    assert!(
        fallback
            .warning
            .unwrap()
            .contains("Expected `enum` - serialized value may not be as expected")
    );

    let mut keyed = Dict::new();
    keyed.insert("type".into(), "dict".into());
    keyed.insert("keys_schema".into(), color());
    keyed.insert(
        "values_schema".into(),
        Value::from_json(r#"{"type": "int"}"#).unwrap(),
    );
    let mut data = Dict::new();
    data.insert(green(), Value::Int(1));
    let dict = SchemaSerializer::new(&Value::Dict(keyed), None).unwrap();
    assert_eq!(json(&dict, &Value::Dict(data)).output, r#"{"g":1}"#);

    let any =
        SchemaSerializer::new(&Value::from_json(r#"{"type": "any"}"#).unwrap(), None).unwrap();
    assert_eq!(
        json(&any, &Value::List(vec![red(), two(), text_a()])).output,
        r#"[1,2,"a"]"#
    );
}

#[test]
fn json_schema_lists_the_values() {
    let generated = generate_json_schema(&color(), None, &JsonSchemaOptions::default())
        .unwrap()
        .schema;
    assert_eq!(
        generated,
        Value::from_json(r#"{"title": "Color", "enum": [1, "g"]}"#).unwrap()
    );
    let generated = generate_json_schema(&num(), None, &JsonSchemaOptions::default())
        .unwrap()
        .schema;
    assert_eq!(
        generated,
        Value::from_json(r#"{"title": "Num", "enum": [1, 2], "type": "integer"}"#).unwrap()
    );
}
