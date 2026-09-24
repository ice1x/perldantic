//! Serializer functions (`serialization` schemas of type `function-plain` / `function-wrap`);
//! expected values come from pydantic-core 2.49.

use std::fmt;

use perldantic_core::{
    Dict, Function, HostCall, HostError, HostFunction, JsonOptions, SchemaSerializer, SerMode,
    SerializeError, SerializeOptions, UnexpectedValue, Value,
};

type Body = dyn Fn(HostCall<'_>) -> Result<Value, HostError> + Send + Sync;

struct Closure {
    name: &'static str,
    body: Box<Body>,
}

impl fmt::Debug for Closure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Closure({})", self.name)
    }
}

impl HostFunction for Closure {
    fn name(&self) -> &str {
        self.name
    }

    fn call(&self, call: HostCall<'_>) -> Result<Value, HostError> {
        (self.body)(call)
    }
}

fn function(
    name: &'static str,
    body: impl Fn(HostCall<'_>) -> Result<Value, HostError> + Send + Sync + 'static,
) -> Value {
    Value::Function(Function::new(Closure {
        name,
        body: Box::new(body),
    }))
}

/// A plain serializer function of the value alone.
fn plain(
    name: &'static str,
    body: impl Fn(Value) -> Result<Value, HostError> + Send + Sync + 'static,
) -> Value {
    function(name, move |call| match call {
        HostCall::Serialize { value, .. } => body(value),
        _ => panic!("a plain serializer"),
    })
}

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn dict(pairs: Vec<(&str, Value)>) -> Dict {
    let mut dict = Dict::new();
    for (k, v) in pairs {
        dict.insert(k.into(), v);
    }
    dict
}

/// `schema` with a `serialization` entry.
fn with_serialization(schema: &str, ser: Vec<(&str, Value)>) -> SchemaSerializer {
    let Value::Dict(mut schema) = j(schema) else {
        unreachable!()
    };
    schema.insert("serialization".into(), Value::Dict(dict(ser)));
    SchemaSerializer::new(&Value::Dict(schema), None).unwrap()
}

fn to_python(s: &SchemaSerializer, value: Value) -> Result<Value, SerializeError> {
    s.to_python(&value, &SerializeOptions::default())
        .map(|r| r.output)
}

fn to_json(s: &SchemaSerializer, value: Value) -> Result<String, SerializeError> {
    s.to_json(
        &value,
        &SerializeOptions::default(),
        &JsonOptions::default(),
    )
    .map(|r| r.output)
}

fn times_ten() -> Value {
    plain("times_ten", |v| match v {
        Value::Int(i) => Ok(Value::Int(i * 10)),
        other => Ok(other),
    })
}

#[test]
fn errors_raised_by_the_function() {
    let boom = plain("boom", |_| Err(HostError::Value("nope".into())));
    let s = with_serialization(
        r#"{"type": "int"}"#,
        vec![("type", "function-plain".into()), ("function", boom)],
    );
    assert_eq!(
        to_python(&s, Value::Int(1)).unwrap_err(),
        SerializeError::Serialization("Error calling function `boom`: ValueError: nope".into())
    );
    assert_eq!(
        to_json(&s, Value::Int(1)).unwrap_err(),
        SerializeError::Serialization(
            "Error serializing to JSON: PydanticSerializationError: Error calling function `boom`: ValueError: nope"
                .into()
        )
    );

    let custom = plain("ser_err", |_| {
        Err(HostError::Serialization(SerializeError::Serialization(
            "custom".into(),
        )))
    });
    let s = with_serialization(
        r#"{"type": "int"}"#,
        vec![("type", "function-plain".into()), ("function", custom)],
    );
    assert_eq!(
        to_python(&s, Value::Int(1)).unwrap_err(),
        SerializeError::Serialization("custom".into())
    );

    // an unexpected value is a warning, and the value is serialized by inference
    let odd = plain("unexp", |_| {
        Err(HostError::Serialization(SerializeError::UnexpectedValue(
            UnexpectedValue::new_from_msg(Some("odd".into())),
        )))
    });
    let s = with_serialization(
        r#"{"type": "int"}"#,
        vec![("type", "function-plain".into()), ("function", odd)],
    );
    let out = s
        .to_python(&Value::Int(1), &SerializeOptions::default())
        .unwrap();
    assert_eq!(out.output, Value::Int(1));
    assert_eq!(
        out.warning.as_deref(),
        Some("Pydantic serializer warnings:\n  PydanticSerializationUnexpectedValue(odd)")
    );
}

#[test]
fn info_describes_the_call() {
    let describe = function("fmt", |call| {
        let HostCall::Serialize {
            value,
            info: Some(info),
            model: None,
        } = call
        else {
            panic!("expected info without a model");
        };
        let context = info.context.map_or("None".to_owned(), |c| c.repr());
        Ok(Value::Str(format!(
            "{}:{}:{context}",
            value.repr(),
            info.mode
        )))
    });
    let s = with_serialization(
        r#"{"type": "int"}"#,
        vec![
            ("type", "function-plain".into()),
            ("function", describe),
            ("info_arg", true.into()),
            ("return_schema", j(r#"{"type": "str"}"#)),
        ],
    );
    let options = SerializeOptions {
        context: Some(j(r#"{"a": 1}"#)),
        ..SerializeOptions::default()
    };
    assert_eq!(
        s.to_python(&Value::Int(1), &options).unwrap().output,
        Value::from("1:python:{'a': 1}")
    );
    assert_eq!(to_json(&s, Value::Int(2)).unwrap(), r#""2:json:None""#);
    let json_mode = SerializeOptions {
        mode: SerMode::Json,
        ..SerializeOptions::default()
    };
    assert_eq!(
        s.to_python(&Value::Int(3), &json_mode).unwrap().output,
        Value::from("3:json:None")
    );
}

#[test]
fn when_used_picks_the_calls() {
    let json_only = with_serialization(
        r#"{"type": "int"}"#,
        vec![
            ("type", "function-plain".into()),
            ("function", times_ten()),
            ("when_used", "json".into()),
        ],
    );
    assert_eq!(to_python(&json_only, Value::Int(1)).unwrap(), Value::Int(1));
    assert_eq!(to_json(&json_only, Value::Int(1)).unwrap(), "10");

    let Value::Dict(mut int) = j(r#"{"type": "int"}"#) else {
        unreachable!()
    };
    int.insert(
        "serialization".into(),
        Value::Dict(dict(vec![
            ("type", "function-plain".into()),
            ("function", times_ten()),
            ("when_used", "unless-none".into()),
        ])),
    );
    let nullable = Value::Dict(dict(vec![
        ("type", "nullable".into()),
        ("schema", Value::Dict(int)),
    ]));
    let s = SchemaSerializer::new(&nullable, None).unwrap();
    assert_eq!(to_python(&s, Value::None).unwrap(), Value::None);
    assert_eq!(to_python(&s, Value::Int(2)).unwrap(), Value::Int(20));

    let Value::Dict(mut bad) = j(r#"{"type": "int"}"#) else {
        unreachable!()
    };
    bad.insert(
        "serialization".into(),
        Value::Dict(dict(vec![
            ("type", "function-plain".into()),
            ("function", times_ten()),
            ("when_used", "sometimes".into()),
        ])),
    );
    let err = SchemaSerializer::new(&Value::Dict(bad), None)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(r#"Invalid value for `when_used`: "sometimes""#),
        "{err}"
    );
}

/// A wrap serializer function that runs `body` with the handler.
fn wrap(
    name: &'static str,
    body: impl Fn(Value, &mut dyn perldantic_core::SerializerHandler) -> Result<Value, HostError>
    + Send
    + Sync
    + 'static,
) -> Value {
    function(name, move |call| match call {
        HostCall::SerializeWrap { value, handler, .. } => body(value, handler),
        _ => panic!("a wrap serializer"),
    })
}

#[test]
fn wrap_serializers_get_a_handler() {
    let plus_one = wrap("wrap", |value, handler| {
        match handler.serialize(value, None)? {
            Value::Int(i) => Ok(Value::Int(i + 1)),
            other => Ok(other),
        }
    });
    let s = with_serialization(
        r#"{"type": "int"}"#,
        vec![("type", "function-wrap".into()), ("function", plus_one)],
    );
    assert_eq!(to_python(&s, Value::Int(1)).unwrap(), Value::Int(2));
    assert_eq!(to_json(&s, Value::Int(1)).unwrap(), "2");

    // items by index: include / exclude apply, and filtered items raise Omit
    let per_item = wrap("wrap_filter", |value, handler| {
        let Value::List(items) = value else {
            panic!("a list")
        };
        let out = items
            .into_iter()
            .enumerate()
            .map(|(i, item)| {
                match handler.serialize(item, Some(Value::Int(i64::try_from(i).unwrap()))) {
                    Ok(v) => v,
                    Err(HostError::Omit) => Value::from("PydanticOmit"),
                    Err(e) => panic!("{e}"),
                }
            })
            .collect();
        Ok(Value::List(out))
    });
    let s = with_serialization(
        r#"{"type": "list", "items_schema": {"type": "int"}}"#,
        vec![
            ("type", "function-wrap".into()),
            ("function", per_item),
            ("schema", j(r#"{"type": "int"}"#)),
        ],
    );
    let exclude_1 = SerializeOptions {
        exclude: Some(Value::Set(vec![Value::Int(1)])),
        ..SerializeOptions::default()
    };
    assert_eq!(
        s.to_python(&j("[1, 2, 3]"), &exclude_1).unwrap().output,
        j(r#"[1, "PydanticOmit", 3]"#)
    );

    // the handler's warnings reach the caller
    let bad = wrap("wrap_bad", |_, handler| {
        handler.serialize(Value::from("x"), None)
    });
    let s = with_serialization(
        r#"{"type": "int"}"#,
        vec![("type", "function-wrap".into()), ("function", bad)],
    );
    let out = s
        .to_python(&Value::Int(1), &SerializeOptions::default())
        .unwrap();
    assert_eq!(out.output, Value::from("x"));
    assert_eq!(
        out.warning.as_deref(),
        Some(
            "Pydantic serializer warnings:\n  PydanticSerializationUnexpectedValue(Expected `int` - serialized value may not be as expected [input_value='x', input_type=str])"
        )
    );
}

#[test]
fn function_schemas_serialize_as_their_schema() {
    let validator = plain("f", Ok);
    let function_schema = |kind: &str, inner: Option<&str>| {
        let mut schema = vec![
            ("type", Value::from(kind)),
            (
                "function",
                Value::Dict(dict(vec![
                    ("type", "no-info".into()),
                    ("function", validator.clone()),
                ])),
            ),
        ];
        if let Some(inner) = inner {
            schema.push(("schema", j(inner)));
        }
        SchemaSerializer::new(&Value::Dict(dict(schema)), None).unwrap()
    };
    for kind in ["function-before", "function-after", "function-wrap"] {
        let s = function_schema(kind, Some(r#"{"type": "int"}"#));
        assert_eq!(to_json(&s, Value::Int(1)).unwrap(), "1", "{kind}");
    }
    // plain validators say nothing about their output: inference
    let s = function_schema("function-plain", None);
    assert_eq!(to_json(&s, j(r#"[1, "a"]"#)).unwrap(), r#"[1,"a"]"#);
}
