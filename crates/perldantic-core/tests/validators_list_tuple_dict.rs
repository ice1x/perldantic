//! `list`, `tuple` and `dict` validators (upstream validators/list.rs, validators/tuple.rs,
//! validators/dict.rs): the core of Perl's `ArrayRef[]`, `Tuple[]` and `HashRef[]` / `Map[]`.
//! Expectations come from pydantic-core 2.49.0.

use perldantic_core::{
    ErrorsOptions, PartialMode, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&Value::from_json(schema).unwrap(), None).unwrap()
}

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn s(v: &str) -> Value {
    Value::from(v)
}

fn tuple(items: Vec<Value>) -> Value {
    Value::Tuple(items)
}

fn dict(items: Vec<(Value, Value)>) -> Value {
    Value::Dict(items.into_iter().collect())
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
}

fn partial() -> ValidateOptions {
    ValidateOptions {
        allow_partial: PartialMode::On,
        ..ValidateOptions::default()
    }
}

/// Errors as `(type, loc, msg)`, with `loc` rendered as JSON.
fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| (d.type_, serde_json::to_string(&d.loc).unwrap(), d.msg))
        .collect()
}

fn err(type_: &str, loc: &str, msg: &str) -> (String, String, String) {
    (type_.into(), loc.into(), msg.into())
}

const INT_PARSING: &str = "Input should be a valid integer, unable to parse string as an integer";

#[test]
fn list_of_any_accepts_tuples_in_lax_mode() {
    let v = validator(r#"{"type": "list"}"#);
    assert_eq!(v.title(), "list[any]");
    assert_eq!(
        v.validate_value(&tuple(vec![Value::Int(1), s("a")]), &lax())
            .unwrap(),
        j(r#"[1, "a"]"#)
    );
}

#[test]
fn list_validates_items_and_collects_all_errors() {
    let v = validator(r#"{"type": "list", "items_schema": {"type": "int"}}"#);
    assert_eq!(v.title(), "list[int]");
    assert_eq!(
        v.validate_value(&j(r#"[1, "2"]"#), &lax()).unwrap(),
        j("[1, 2]")
    );
    assert_eq!(
        errors(v.validate_value(&j(r#"[1, "x", "2", "y"]"#), &lax())),
        vec![
            err("int_parsing", "[1]", INT_PARSING),
            err("int_parsing", "[3]", INT_PARSING)
        ]
    );
    assert_eq!(
        v.validate_json(r#"[1, "2", 3]"#, &lax()).unwrap(),
        j("[1, 2, 3]")
    );
}

#[test]
fn list_fail_fast_stops_at_the_first_error() {
    let v = validator(r#"{"type": "list", "items_schema": {"type": "int"}, "fail_fast": true}"#);
    assert_eq!(
        errors(v.validate_value(&j(r#"[1, "x", "2", "y"]"#), &lax())),
        vec![err("int_parsing", "[1]", INT_PARSING)]
    );
}

#[test]
fn list_rejects_non_sequences_and_tuples_when_strict() {
    let v = validator(r#"{"type": "list", "items_schema": {"type": "int"}}"#);
    for input in [s("abc"), j(r#"{"1": 2}"#)] {
        assert_eq!(
            errors(v.validate_value(&input, &lax())),
            vec![err("list_type", "[]", "Input should be a valid list")]
        );
    }
    let strict = validator(r#"{"type": "list", "items_schema": {"type": "int"}, "strict": true}"#);
    assert_eq!(
        errors(strict.validate_value(&tuple(vec![Value::Int(1)]), &lax())),
        vec![err("list_type", "[]", "Input should be a valid list")]
    );
}

#[test]
fn list_length_constraints() {
    let short = validator(r#"{"type": "list", "items_schema": {"type": "int"}, "min_length": 3}"#);
    assert_eq!(
        errors(short.validate_value(&j("[1]"), &lax())),
        vec![err(
            "too_short",
            "[]",
            "List should have at least 3 items after validation, not 1"
        )]
    );
    for schema in [
        r#"{"type": "list", "items_schema": {"type": "int"}, "max_length": 1}"#,
        r#"{"type": "list", "max_length": 1}"#,
    ] {
        let long = validator(schema);
        assert_eq!(
            errors(long.validate_value(&j("[1, 2, 3]"), &lax())),
            vec![err(
                "too_long",
                "[]",
                "List should have at most 1 item after validation, not 3"
            )]
        );
    }
    // The length error wins over item errors found later.
    let long = validator(r#"{"type": "list", "items_schema": {"type": "int"}, "max_length": 1}"#);
    assert_eq!(
        errors(long.validate_value(&j(r#"[1, "x", "y"]"#), &lax()))[0].0,
        "too_long"
    );
}

#[test]
fn list_partial_json_drops_an_invalid_last_item() {
    let v = validator(r#"{"type": "list", "items_schema": {"type": "int"}}"#);
    assert_eq!(v.validate_json(r#"[1, "a""#, &partial()).unwrap(), j("[1]"));
    assert_eq!(v.validate_json("[1, 2", &partial()).unwrap(), j("[1, 2]"));
}

#[test]
fn list_drops_omitted_items() {
    let v = validator(
        r#"{"type": "list", "items_schema": {"type": "default", "schema": {"type": "int"}, "on_error": "omit"}}"#,
    );
    assert_eq!(v.title(), "list[default[int]]");
    assert_eq!(
        v.validate_value(&j(r#"[1, "x", 3]"#), &lax()).unwrap(),
        j("[1, 3]")
    );
}

#[test]
fn tuple_positional_items() {
    let v = validator(r#"{"type": "tuple", "items_schema": [{"type": "int"}, {"type": "str"}]}"#);
    assert_eq!(v.title(), "tuple[int, str]");
    assert_eq!(
        v.validate_value(&j(r#"[1, "a"]"#), &lax()).unwrap(),
        tuple(vec![Value::Int(1), s("a")])
    );
    assert_eq!(
        errors(v.validate_value(&tuple(vec![Value::Int(1)]), &lax())),
        vec![err("missing", "[1]", "Field required")]
    );
    assert_eq!(
        errors(v.validate_value(&tuple(vec![Value::Int(1), s("a"), Value::Int(3)]), &lax())),
        vec![err(
            "too_long",
            "[]",
            "Tuple should have at most 2 items after validation, not 3"
        )]
    );
    assert_eq!(
        errors(v.validate_value(&s("ab"), &lax())),
        vec![err("tuple_type", "[]", "Input should be a valid tuple")]
    );
    assert_eq!(
        v.validate_json(r#"[1, "a"]"#, &lax()).unwrap(),
        tuple(vec![Value::Int(1), s("a")])
    );
}

#[test]
fn tuple_empty() {
    let v = validator(r#"{"type": "tuple", "items_schema": []}"#);
    assert_eq!(v.title(), "tuple[]");
    assert_eq!(
        v.validate_value(&tuple(vec![]), &lax()).unwrap(),
        tuple(vec![])
    );
}

#[test]
fn tuple_variadic_items() {
    let v = validator(
        r#"{"type": "tuple", "items_schema": [{"type": "int"}], "variadic_item_index": 0}"#,
    );
    assert_eq!(v.title(), "tuple[int, ...]");
    assert_eq!(
        errors(v.validate_value(&tuple(vec![Value::Int(1), s("2"), s("x")]), &lax())),
        vec![err("int_parsing", "[2]", INT_PARSING)]
    );

    let middle = validator(
        r#"{"type": "tuple", "items_schema": [{"type": "str"}, {"type": "int"}, {"type": "str"}], "variadic_item_index": 1}"#,
    );
    assert_eq!(middle.title(), "tuple[str, int, ..., str]");
    assert_eq!(
        middle
            .validate_value(&tuple(vec![s("a"), Value::Int(1), s("2"), s("b")]), &lax())
            .unwrap(),
        tuple(vec![s("a"), Value::Int(1), Value::Int(2), s("b")])
    );
    assert_eq!(
        errors(middle.validate_value(&tuple(vec![s("a")]), &lax())),
        vec![err("missing", "[1]", "Field required")]
    );
}

#[test]
fn tuple_length_constraints() {
    let long = validator(
        r#"{"type": "tuple", "items_schema": [{"type": "int"}], "variadic_item_index": 0, "max_length": 2}"#,
    );
    assert_eq!(
        errors(long.validate_value(&j("[1, 2, 3]"), &lax())),
        vec![err(
            "too_long",
            "[]",
            "Tuple should have at most 2 items after validation, not 3"
        )]
    );
    let short = validator(
        r#"{"type": "tuple", "items_schema": [{"type": "int"}], "variadic_item_index": 0, "min_length": 2}"#,
    );
    assert_eq!(
        errors(short.validate_value(&j("[1]"), &lax())),
        vec![err(
            "too_short",
            "[]",
            "Tuple should have at least 2 items after validation, not 1"
        )]
    );
}

#[test]
fn tuple_missing_items_use_defaults() {
    let v = validator(
        r#"{"type": "tuple", "items_schema": [{"type": "int"}, {"type": "default", "schema": {"type": "int"}, "default": 9}]}"#,
    );
    assert_eq!(
        v.validate_value(&j("[1]"), &lax()).unwrap(),
        tuple(vec![Value::Int(1), Value::Int(9)])
    );
}

#[test]
fn dict_validates_keys_and_values() {
    let v = validator(
        r#"{"type": "dict", "keys_schema": {"type": "int"}, "values_schema": {"type": "str"}}"#,
    );
    assert_eq!(v.title(), "dict[int,str]");
    let input = dict(vec![
        (s("1"), s("a")),
        (s("x"), Value::Int(2)),
        (Value::Int(3), s("c")),
    ]);
    assert_eq!(
        errors(v.validate_value(&input, &lax())),
        vec![
            err("int_parsing", r#"["x","[key]"]"#, INT_PARSING),
            err("string_type", r#"["x"]"#, "Input should be a valid string"),
        ]
    );
    let ok = dict(vec![(s("1"), s("a")), (Value::Int(3), s("c"))]);
    assert_eq!(
        v.validate_value(&ok, &lax()).unwrap(),
        dict(vec![(Value::Int(1), s("a")), (Value::Int(3), s("c"))])
    );
}

#[test]
fn dict_of_any_rejects_non_mappings() {
    let v = validator(r#"{"type": "dict"}"#);
    assert_eq!(v.title(), "dict[any,any]");
    assert_eq!(
        errors(v.validate_value(&j("[[1, 2]]"), &lax())),
        vec![err("dict_type", "[]", "Input should be a valid dictionary")]
    );
}

#[test]
fn dict_keys_that_validate_equal_merge() {
    let v = validator(
        r#"{"type": "dict", "keys_schema": {"type": "int"}, "values_schema": {"type": "int"}}"#,
    );
    let input = dict(vec![
        (s("1"), Value::Int(1)),
        (Value::Int(1), Value::Int(2)),
    ]);
    assert_eq!(
        v.validate_value(&input, &lax()).unwrap(),
        dict(vec![(Value::Int(1), Value::Int(2))])
    );
}

#[test]
fn dict_length_constraints() {
    let short = validator(
        r#"{"type": "dict", "keys_schema": {"type": "int"}, "values_schema": {"type": "int"}, "min_length": 2}"#,
    );
    assert_eq!(
        errors(short.validate_value(&j(r#"{"1": 1}"#), &lax())),
        vec![err(
            "too_short",
            "[]",
            "Dictionary should have at least 2 items after validation, not 1"
        )]
    );
    let long = validator(
        r#"{"type": "dict", "keys_schema": {"type": "int"}, "values_schema": {"type": "int"}, "max_length": 1}"#,
    );
    assert_eq!(
        errors(long.validate_value(&j(r#"{"1": 1, "2": 3}"#), &lax())),
        vec![err(
            "too_long",
            "[]",
            "Dictionary should have at most 1 item after validation, not 2"
        )]
    );
}

#[test]
fn dict_from_json_fail_fast_and_partial() {
    let v = validator(
        r#"{"type": "dict", "keys_schema": {"type": "int"}, "values_schema": {"type": "int"}}"#,
    );
    assert_eq!(
        errors(v.validate_json(r#"{"1": 1, "a": "b"}"#, &lax())),
        vec![
            err("int_parsing", r#"["a","[key]"]"#, INT_PARSING),
            err("int_parsing", r#"["a"]"#, INT_PARSING),
        ]
    );
    assert_eq!(
        v.validate_json(r#"{"1": 1, "2": "b"#, &partial()).unwrap(),
        dict(vec![(Value::Int(1), Value::Int(1))])
    );

    let fast = validator(
        r#"{"type": "dict", "keys_schema": {"type": "int"}, "values_schema": {"type": "int"}, "fail_fast": true}"#,
    );
    assert_eq!(
        errors(fast.validate_value(&j(r#"{"a": "b", "c": "d"}"#), &lax())),
        vec![err("int_parsing", r#"["a","[key]"]"#, INT_PARSING)]
    );
}

fn build_error(schema: &str) -> String {
    match SchemaValidator::new(&Value::from_json(schema).unwrap(), None) {
        Err(e) => e.to_string(),
        Ok(v) => panic!("expected a schema error, built {}", v.title()),
    }
}

#[test]
fn item_schemas_must_be_dicts() {
    for schema in [
        r#"{"type": "list", "items_schema": 1}"#,
        r#"{"type": "tuple", "items_schema": [1]}"#,
    ] {
        assert!(
            build_error(schema).ends_with("TypeError: 'int' object is not an instance of 'dict'"),
            "{}",
            build_error(schema)
        );
    }
}

#[test]
fn variadic_index_out_of_range_is_a_schema_error() {
    // Upstream panics here (docs/DIVERGENCES.md #11).
    assert_eq!(
        build_error(
            r#"{"type": "tuple", "items_schema": [{"type": "int"}], "variadic_item_index": 1}"#
        ),
        "Error building \"tuple\" validator:\n  SchemaError: `variadic_item_index` 1 is out of range for 1 items"
    );
}
