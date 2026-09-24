//! `function-*` validators with host functions; expected values come from pydantic-core 2.49.

use std::fmt;
use std::sync::{Arc, Mutex};

use perldantic_core::{
    CoreError, Dict, ErrorType, ErrorsOptions, Function, HostCall, HostError, HostException,
    HostFunction, InputType, LocItem, SchemaValidator, ValidateError, ValidateOptions,
    ValidationInfo, Value,
};

type Body = dyn Fn(HostCall<'_>) -> Result<Value, HostError> + Send + Sync;

/// A host function backed by a Rust closure.
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

/// A function of one value (before / after / plain validators).
fn unary(
    name: &'static str,
    body: impl Fn(Value) -> Result<Value, HostError> + Send + Sync + 'static,
) -> Value {
    function(name, move |call| match call {
        HostCall::Validate { input, .. } => body(input),
        HostCall::ValidateWrap { .. } => panic!("not a wrap validator"),
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

fn function_schema(kind: &str, func: Value, info: bool, inner: Option<Value>) -> Value {
    let mut schema = vec![
        ("type", kind.into()),
        (
            "function",
            Value::Dict(dict(vec![
                ("type", if info { "with-info" } else { "no-info" }.into()),
                ("function", func),
            ])),
        ),
    ];
    if let Some(inner) = inner {
        schema.push(("schema", inner));
    }
    Value::Dict(dict(schema))
}

fn validator(schema: &Value) -> SchemaValidator {
    SchemaValidator::new(schema, None).unwrap()
}

fn validate(v: &SchemaValidator, input: Value) -> Result<Value, ValidateError> {
    v.validate_value(&input, &ValidateOptions::default())
}

/// `(type, loc, msg, input)` of every error.
fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String, String, Value)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| {
            (
                d.type_,
                serde_json::to_string(&d.loc).unwrap(),
                d.msg,
                d.input.unwrap(),
            )
        })
        .collect()
}

fn error(type_: &str, loc: &str, msg: &str, input: Value) -> (String, String, String, Value) {
    (type_.into(), loc.into(), msg.into(), input)
}

fn double(value: Value) -> Result<Value, HostError> {
    Ok(match value {
        Value::Str(s) => Value::Str(s.repeat(2)),
        Value::Int(i) => Value::Int(i * 2),
        Value::List(items) => Value::List([items.clone(), items].concat()),
        other => other,
    })
}

#[test]
fn before_validators_run_the_function_first() {
    let v = validator(&function_schema(
        "function-before",
        unary("double", double),
        false,
        Some(j(r#"{"type": "int"}"#)),
    ));
    assert_eq!(v.title(), "function-before[double(), int]");
    assert_eq!(validate(&v, j(r#""12""#)).unwrap(), Value::Int(1212));
    assert_eq!(
        errors(validate(&v, j("[1]"))),
        [error(
            "int_type",
            "[]",
            "Input should be a valid integer",
            j("[1, 1]")
        )]
    );
}

#[test]
fn after_validators_report_what_the_function_raises() {
    let boom = unary("boom", |_| Err(HostError::Value("boom".into())));
    let v = validator(&function_schema(
        "function-after",
        boom,
        false,
        Some(j(r#"{"type": "int"}"#)),
    ));
    assert_eq!(v.title(), "function-after[boom(), int]");
    // the error points at the original input
    assert_eq!(
        errors(validate(&v, j(r#""3""#))),
        [error("value_error", "[]", "Value error, boom", j(r#""3""#))]
    );
    assert_eq!(
        errors(validate(&v, j(r#""x""#)))[0].0,
        "int_parsing",
        "the inner schema fails first"
    );

    let small = unary("asrt", |_| Err(HostError::Assertion("too small".into())));
    let v = validator(&function_schema(
        "function-after",
        small,
        false,
        Some(j(r#"{"type": "int"}"#)),
    ));
    assert_eq!(
        errors(validate(&v, Value::Int(3))),
        [error(
            "assertion_error",
            "[]",
            "Assertion failed, too small",
            Value::Int(3)
        )]
    );
}

#[test]
fn plain_validators_raise_custom_and_known_errors() {
    let v = validator(&function_schema(
        "function-plain",
        unary("double", double),
        false,
        None,
    ));
    assert_eq!(v.title(), "function-plain[double()]");
    assert_eq!(validate(&v, j(r#""ab""#)).unwrap(), j(r#""abab""#));

    let custom = unary("custom", |value| {
        Err(HostError::Custom {
            error_type: "my_error".into(),
            message_template: "bad {thing}".into(),
            context: Some(dict(vec![("thing", value)])),
        })
    });
    let v = validator(&function_schema("function-plain", custom, false, None));
    assert_eq!(
        errors(validate(&v, Value::Int(1))),
        [error("my_error", "[]", "bad 1", Value::Int(1))]
    );

    let known = unary("known", |_| {
        Err(HostError::Known(
            ErrorType::new("greater_than", Some(&dict(vec![("gt", Value::Int(5))]))).unwrap(),
        ))
    });
    let v = validator(&function_schema("function-plain", known, false, None));
    assert_eq!(
        errors(validate(&v, Value::Int(1))),
        [error(
            "greater_than",
            "[]",
            "Input should be greater than 5",
            Value::Int(1)
        )]
    );
}

#[test]
fn omit_and_use_default_reach_their_containers() {
    let omit = unary("omit", |value| {
        if value == Value::Int(2) {
            Err(HostError::Omit)
        } else {
            Ok(value)
        }
    });
    let list = Value::Dict(dict(vec![
        ("type", "list".into()),
        (
            "items_schema",
            function_schema("function-plain", omit, false, None),
        ),
    ]));
    assert_eq!(
        validate(&validator(&list), j("[1, 2, 3]")).unwrap(),
        j("[1, 3]")
    );

    let use_default = unary("usedef", |_| Err(HostError::UseDefault));
    let field = Value::Dict(dict(vec![
        ("type", "typed-dict-field".into()),
        (
            "schema",
            Value::Dict(dict(vec![
                ("type", "default".into()),
                (
                    "schema",
                    function_schema("function-plain", use_default, false, None),
                ),
                ("default", Value::Int(7)),
            ])),
        ),
    ]));
    let typed_dict = Value::Dict(dict(vec![
        ("type", "typed-dict".into()),
        ("fields", Value::Dict(dict(vec![("a", field)]))),
    ]));
    assert_eq!(
        validate(&validator(&typed_dict), j(r#"{"a": 1}"#)).unwrap(),
        j(r#"{"a": 7}"#)
    );
}

#[test]
fn other_exceptions_pass_through_unchanged() {
    let raise = unary("other", |_| {
        Err(HostError::Other(HostException::new(
            "TypeError: nope",
            42_u32,
        )))
    });
    let v = validator(&function_schema("function-plain", raise, false, None));
    let Err(ValidateError::Core(CoreError::Host(exception))) = validate(&v, Value::Int(1)) else {
        panic!("expected the host exception");
    };
    assert_eq!(exception.message(), "TypeError: nope");
    assert_eq!(exception.payload().downcast_ref::<u32>(), Some(&42));
}

/// A wrap function: calls the handler (optionally with a location) and applies `on_error`.
fn wrap(
    name: &'static str,
    outer_location: Option<&'static str>,
    on_error: fn(HostError) -> Result<Value, HostError>,
) -> Value {
    function(name, move |call| match call {
        HostCall::ValidateWrap { input, handler, .. } => handler
            .validate(input, outer_location.map(LocItem::from))
            .or_else(on_error),
        HostCall::Validate { .. } => panic!("a wrap validator"),
    })
}

#[test]
fn wrap_validators_get_a_handler() {
    let fallback = wrap("wrap", None, |e| match e {
        HostError::Validation(_) => Ok(Value::Int(-1)),
        other => Err(other),
    });
    let int = Some(j(r#"{"type": "int"}"#));
    let v = validator(&function_schema(
        "function-wrap",
        fallback,
        false,
        int.clone(),
    ));
    assert_eq!(v.title(), "function-wrap[wrap()]");
    assert_eq!(validate(&v, j(r#""5""#)).unwrap(), Value::Int(5));
    assert_eq!(validate(&v, j(r#""x""#)).unwrap(), Value::Int(-1));

    let located = wrap("wrap_loc", Some("here"), Err);
    let v = validator(&function_schema(
        "function-wrap",
        located,
        false,
        int.clone(),
    ));
    let parsing = "Input should be a valid integer, unable to parse string as an integer";
    assert_eq!(
        errors(validate(&v, j(r#""x""#))),
        [error("int_parsing", r#"["here"]"#, parsing, j(r#""x""#))]
    );

    // a re-raised handler error is reported as it was
    let seen = Arc::new(Mutex::new(None));
    let seen_in = seen.clone();
    let reraise = function("wrap_reraise", move |call| match call {
        HostCall::ValidateWrap { input, handler, .. } => {
            handler.validate(input, None).inspect_err(|e| {
                if let HostError::Validation(error) = e {
                    *seen_in.lock().unwrap() = Some(error.title().to_owned());
                }
            })
        }
        HostCall::Validate { .. } => panic!("a wrap validator"),
    });
    let v = validator(&function_schema("function-wrap", reraise, false, int));
    assert_eq!(
        errors(validate(&v, j(r#""x""#))),
        [error("int_parsing", "[]", parsing, j(r#""x""#))]
    );
    assert_eq!(seen.lock().unwrap().as_deref(), Some("ValidatorCallable"));
}

/// A function returning the parts of its `info` argument as a list.
fn info_function() -> Value {
    function("info_fn", |call| {
        let HostCall::Validate {
            input,
            info: Some(info),
        } = call
        else {
            panic!("expected info");
        };
        let ValidationInfo {
            config,
            context,
            data,
            field_name,
            mode,
        } = info;
        Ok(Value::List(vec![
            input,
            field_name.map_or(Value::None, Value::from),
            data.map_or(Value::None, Value::Dict),
            context.unwrap_or(Value::None),
            Value::from(match mode {
                InputType::Json => "json",
                _ => "python",
            }),
            config.map_or(Value::None, Value::Dict),
        ]))
    })
}

#[test]
fn info_describes_the_call() {
    let field = |schema| {
        Value::Dict(dict(vec![
            ("type", "typed-dict-field".into()),
            ("schema", schema),
        ]))
    };
    let typed_dict = Value::Dict(dict(vec![
        ("type", "typed-dict".into()),
        (
            "fields",
            Value::Dict(dict(vec![
                ("a", field(j(r#"{"type": "int"}"#))),
                (
                    "b",
                    field(function_schema(
                        "function-after",
                        info_function(),
                        true,
                        Some(j(r#"{"type": "str"}"#)),
                    )),
                ),
            ])),
        ),
    ]));
    let v = SchemaValidator::new(&typed_dict, Some(&j(r#"{"title": "T"}"#))).unwrap();
    let options = ValidateOptions {
        context: Some(j(r#"{"c": 1}"#)),
        ..ValidateOptions::default()
    };
    let out = v
        .validate_value(&j(r#"{"a": 1, "b": "x"}"#), &options)
        .unwrap();
    // typed dicts give their fields their own (here absent) config
    assert_eq!(
        out,
        j(r#"{"a": 1, "b": ["x", "b", {"a": 1}, {"c": 1}, "python", null]}"#)
    );

    let plain = validator(&function_schema(
        "function-plain",
        info_function(),
        true,
        None,
    ));
    assert_eq!(
        plain
            .validate_json("1", &ValidateOptions::default())
            .unwrap(),
        j(r#"[1, null, null, null, "json", null]"#)
    );
}

#[test]
fn schemas_need_a_host_function() {
    let schema = function_schema("function-plain", "abs".into(), false, None);
    let err = SchemaValidator::new(&schema, None).unwrap_err().to_string();
    assert!(
        err.contains("`function` should be a host function, got str"),
        "{err}"
    );
}

#[test]
fn json_schemas_describe_the_wrapped_or_input_schema() {
    use perldantic_core::{
        JsonSchemaError, JsonSchemaMode, JsonSchemaOptions, generate_json_schema,
    };
    let generate = |schema: &Value, mode| {
        let options = JsonSchemaOptions {
            mode,
            ..JsonSchemaOptions::default()
        };
        generate_json_schema(schema, None, &options).map(|g| g.schema)
    };
    let validation = JsonSchemaMode::Validation;
    let int = j(r#"{"type": "int"}"#);
    for kind in ["function-before", "function-after", "function-wrap"] {
        let schema = function_schema(kind, unary("f", Ok), false, Some(int.clone()));
        assert_eq!(
            generate(&schema, validation).unwrap(),
            j(r#"{"type": "integer"}"#),
            "{kind}"
        );
    }
    let with_input = |kind: &str| {
        let Value::Dict(mut schema) =
            function_schema(kind, unary("f", Ok), false, Some(int.clone()))
        else {
            unreachable!()
        };
        schema.insert("json_schema_input_schema".into(), j(r#"{"type": "str"}"#));
        Value::Dict(schema)
    };
    // the input schema describes validation input; serialization still shows the schema
    assert_eq!(
        generate(&with_input("function-before"), validation).unwrap(),
        j(r#"{"type": "string"}"#)
    );
    assert_eq!(
        generate(
            &with_input("function-before"),
            JsonSchemaMode::Serialization
        )
        .unwrap(),
        j(r#"{"type": "integer"}"#)
    );
    assert_eq!(
        generate(&with_input("function-after"), validation).unwrap(),
        j(r#"{"type": "integer"}"#),
        "after validators always validate their schema first"
    );

    let plain = function_schema("function-plain", unary("f", Ok), false, None);
    let Err(JsonSchemaError::InvalidForJsonSchema(message)) = generate(&plain, validation) else {
        panic!("plain validators have no JSON Schema of their own");
    };
    assert!(
        message.starts_with(
            "Cannot generate a JsonSchema for core_schema.PlainValidatorFunctionSchema"
        ),
        "{message}"
    );
}
