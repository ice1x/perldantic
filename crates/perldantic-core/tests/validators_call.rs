//! `call` schemas: a host function called with validated arguments, its result validated by
//! `return_schema`; expected values come from pydantic-core 2.49.

use std::fmt;

use perldantic_core::{
    CoreError, Dict, ErrorsOptions, Function, HostCall, HostError, HostException, HostFunction,
    SchemaValidator, ValidateError, ValidateOptions, Value,
};

type Body = dyn Fn(Vec<Value>, Dict) -> Result<Value, HostError> + Send + Sync;

/// A host function taking arguments, backed by a Rust closure.
struct Callable {
    name: &'static str,
    body: Box<Body>,
}

impl fmt::Debug for Callable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Callable({})", self.name)
    }
}

impl HostFunction for Callable {
    fn name(&self) -> &str {
        self.name
    }

    fn call(&self, call: HostCall<'_>) -> Result<Value, HostError> {
        match call {
            HostCall::Call { args, kwargs } => (self.body)(args, kwargs),
            _ => panic!("a called function"),
        }
    }
}

fn function(
    name: &'static str,
    body: impl Fn(Vec<Value>, Dict) -> Result<Value, HostError> + Send + Sync + 'static,
) -> Value {
    Value::Function(Function::new(Callable {
        name,
        body: Box::new(body),
    }))
}

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

/// `add(a, b=1)`: the sum, as a string when `text` is given.
fn add() -> Value {
    function("add", |args, kwargs| {
        let mut sum = 0;
        for value in args.iter().chain(kwargs.iter().map(|(_, v)| v)) {
            if let Value::Int(i) = value {
                sum += i;
            }
        }
        Ok(Value::Int(sum))
    })
}

fn call_schema(function: Value, return_schema: Option<&str>) -> Value {
    let Value::Dict(mut schema) = j(
        r#"{"type": "call", "arguments_schema": {"type": "arguments",
        "arguments_schema": [
            {"name": "a", "schema": {"type": "int"}},
            {"name": "b", "schema": {"type": "default", "schema": {"type": "int"}, "default": 1}}
        ]}}"#,
    ) else {
        unreachable!()
    };
    schema.insert("function".into(), function);
    if let Some(return_schema) = return_schema {
        schema.insert("return_schema".into(), j(return_schema));
    }
    Value::Dict(schema)
}

fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| {
            let loc: Vec<String> = d.loc.iter().map(ToString::to_string).collect();
            (d.type_, loc.join("."))
        })
        .collect()
}

#[test]
fn the_function_gets_the_validated_arguments() {
    let v = SchemaValidator::new(&call_schema(add(), None), None).unwrap();
    assert_eq!(v.title(), "call[add]");
    let opts = ValidateOptions::default();
    assert_eq!(
        v.validate_value(&j(r#"["2"]"#), &opts).unwrap(),
        Value::Int(3)
    );
    assert_eq!(
        v.validate_value(&j(r#"{"a": 2, "b": "5"}"#), &opts)
            .unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        errors(v.validate_value(&j(r#"["x"]"#), &opts)),
        vec![("int_parsing".into(), "0".into())],
        "invalid arguments: the function is not called"
    );
}

#[test]
fn the_result_is_validated_by_the_return_schema() {
    let v = SchemaValidator::new(
        &call_schema(add(), Some(r#"{"type": "int", "gt": 5}"#)),
        None,
    )
    .unwrap();
    let opts = ValidateOptions::default();
    assert_eq!(v.validate_value(&j("[10]"), &opts).unwrap(), Value::Int(11));
    assert_eq!(
        errors(v.validate_value(&j("[1]"), &opts)),
        vec![("greater_than".into(), "return".into())]
    );
}

#[test]
fn what_the_function_raises_reaches_the_caller() {
    let failing = function("fail", |_, _| Err(HostError::Value("boom".into())));
    let v = SchemaValidator::new(&call_schema(failing, None), None).unwrap();
    let result = v.validate_value(&j("[1]"), &ValidateOptions::default());
    let Err(ValidateError::Core(CoreError::Value(message))) = result else {
        panic!("expected a ValueError, got {result:?}");
    };
    assert_eq!(message, "boom");

    let raising = function("raise", |_, _| {
        Err(HostError::Other(HostException::new("Oops", 7_u32)))
    });
    let v = SchemaValidator::new(&call_schema(raising, None), None).unwrap();
    let result = v.validate_value(&j("[1]"), &ValidateOptions::default());
    assert!(
        matches!(result, Err(ValidateError::Core(CoreError::Host(_)))),
        "{result:?}"
    );
}

#[test]
fn function_name_names_the_validator() {
    let Value::Dict(mut schema) = call_schema(add(), None) else {
        unreachable!()
    };
    schema.insert("function_name".into(), "plus".into());
    let v = SchemaValidator::new(&Value::Dict(schema), None).unwrap();
    assert_eq!(v.title(), "call[plus]");
}
