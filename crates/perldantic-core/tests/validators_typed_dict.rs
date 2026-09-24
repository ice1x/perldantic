//! `typed-dict` beyond the recorded cases; expected values come from pydantic-core 2.49.

use perldantic_core::{
    ErrorsOptions, InputType, JsonOptions, JsonSchemaOptions, PartialMode, SchemaSerializer,
    SchemaValidator, SerializeOptions, ValidateError, ValidateOptions, Value, generate_json_schema,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn validator(schema: &str) -> SchemaValidator {
    SchemaValidator::new(&j(schema), None).unwrap()
}

fn validate(schema: &str, input: &str) -> Result<Value, ValidateError> {
    validator(schema).validate_value(&j(input), &ValidateOptions::default())
}

/// `(type, loc)` of every error, the loc as JSON.
fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}");
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| (d.type_, serde_json::to_string(&d.loc).unwrap()))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(t, l)| ((*t).to_owned(), (*l).to_owned()))
        .collect()
}

fn schema_error(schema: &str) -> String {
    SchemaValidator::new(&j(schema), None)
        .unwrap_err()
        .to_string()
}

const MOVIE: &str = r#"{"type": "typed-dict", "fields": {
    "b": {"type": "typed-dict-field", "schema": {"type": "int"}},
    "a": {"type": "typed-dict-field", "schema": {"type": "str"}, "required": false}
}}"#;

fn with(schema: &str, extra: &str) -> String {
    let mut schema = schema.trim_end().to_owned();
    schema.pop();
    format!("{schema}, {extra}}}")
}

#[test]
fn keys_are_validated_in_field_order_and_unknown_keys_dropped() {
    let out = validate(MOVIE, r#"{"a": "x", "b": "1", "c": 3}"#).unwrap();
    assert_eq!(out, j(r#"{"b": 1, "a": "x"}"#));
    let Value::Dict(dict) = out else { panic!() };
    let keys: Vec<_> = dict.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(keys, [Value::from("b"), Value::from("a")]);

    assert_eq!(
        errors(validate(MOVIE, r#"{"a": 1}"#)),
        pairs(&[("missing", r#"["b"]"#), ("string_type", r#"["a"]"#)])
    );
    assert_eq!(
        errors(validate(MOVIE, "[1]")),
        pairs(&[("dict_type", "[]")])
    );
}

#[test]
fn total_false_makes_fields_optional() {
    assert_eq!(
        validate(&with(MOVIE, r#""total": false"#), "{}").unwrap(),
        j("{}")
    );
    // the config's `typed_dict_total` does the same
    let config = with(MOVIE, r#""config": {"typed_dict_total": false}"#);
    assert_eq!(validate(&config, "{}").unwrap(), j("{}"));
}

#[test]
fn extra_behavior_forbid_and_allow() {
    let forbid = with(MOVIE, r#""extra_behavior": "forbid""#);
    assert_eq!(
        errors(validate(&forbid, r#"{"b": 1, "c": 3}"#)),
        pairs(&[("extra_forbidden", r#"["c"]"#)])
    );
    let allow = with(MOVIE, r#""extra_behavior": "allow""#);
    assert_eq!(
        validate(&allow, r#"{"b": 1, "c": [1]}"#).unwrap(),
        j(r#"{"b": 1, "c": [1]}"#)
    );
    let typed = with(
        MOVIE,
        r#""extra_behavior": "allow", "extras_schema": {"type": "int"}"#,
    );
    assert_eq!(
        validate(&typed, r#"{"b": 1, "c": "3"}"#).unwrap(),
        j(r#"{"b": 1, "c": 3}"#)
    );
    assert_eq!(
        errors(validate(&typed, r#"{"b": 1, "c": "3", "d": "x"}"#)),
        pairs(&[("int_parsing", r#"["d"]"#)])
    );
    // the config's `extra_fields_behavior` applies when the schema sets none
    let config = with(MOVIE, r#""config": {"extra_fields_behavior": "forbid"}"#);
    assert_eq!(
        errors(validate(&config, r#"{"b": 1, "c": 3}"#)),
        pairs(&[("extra_forbidden", r#"["c"]"#)])
    );
}

#[test]
fn aliases_and_lookup_paths() {
    let aliased = r#"{"type": "typed-dict", "fields": {
        "name": {"type": "typed-dict-field", "schema": {"type": "str"}, "validation_alias": "Name"}
    }}"#;
    assert_eq!(
        validate(aliased, r#"{"Name": "x"}"#).unwrap(),
        j(r#"{"name": "x"}"#)
    );
    assert_eq!(
        errors(validate(aliased, r#"{"name": "x"}"#)),
        pairs(&[("missing", r#"["Name"]"#)])
    );
    let by_name = with(aliased, r#""config": {"loc_by_alias": false}"#);
    assert_eq!(
        errors(validate(&by_name, "{}")),
        pairs(&[("missing", r#"["name"]"#)])
    );
    let deep = r#"{"type": "typed-dict", "fields": {
        "name": {"type": "typed-dict-field", "schema": {"type": "str"}, "validation_alias": [["a", "b"]]}
    }}"#;
    assert_eq!(
        validate(deep, r#"{"a": {"b": "deep"}}"#).unwrap(),
        j(r#"{"name": "deep"}"#)
    );
}

#[test]
fn defaults_and_omitted_errors() {
    let default = r#"{"type": "typed-dict", "fields": {
        "x": {"type": "typed-dict-field", "schema": {"type": "default", "schema": {"type": "int"}, "default": 7}}
    }}"#;
    assert_eq!(validate(default, "{}").unwrap(), j(r#"{"x": 7}"#));
    let omit = r#"{"type": "typed-dict", "fields": {
        "x": {"type": "typed-dict-field", "required": false,
              "schema": {"type": "default", "schema": {"type": "int"}, "on_error": "omit"}}
    }}"#;
    assert_eq!(validate(omit, r#"{"x": "bad"}"#).unwrap(), j("{}"));
}

#[test]
fn invalid_schemas_are_rejected() {
    assert!(
        schema_error(&with(MOVIE, r#""extras_schema": {"type": "int"}"#))
            .contains("extras_schema can only be used if extra_behavior=allow")
    );
    let required_default = r#"{"type": "typed-dict", "fields": {
        "x": {"type": "typed-dict-field", "required": true,
              "schema": {"type": "default", "schema": {"type": "int"}, "default": 1}}
    }}"#;
    assert!(
        schema_error(required_default)
            .contains("Field 'x': a required field cannot have a default value")
    );
    let required_omit = r#"{"type": "typed-dict", "fields": {
        "x": {"type": "typed-dict-field", "required": true,
              "schema": {"type": "default", "schema": {"type": "int"}, "on_error": "omit"}}
    }}"#;
    assert!(
        schema_error(required_omit)
            .contains("Field 'x': 'on_error = omit' cannot be set for required fields")
    );
}

#[test]
fn titles_name_the_class() {
    assert_eq!(validator(MOVIE).title(), "typed-dict");
    assert_eq!(
        validator(&with(MOVIE, r#""cls_name": "Movie""#)).title(),
        "Movie"
    );
}

#[test]
fn partial_json_keeps_the_complete_keys() {
    let schema = r#"{"type": "typed-dict", "fields": {
        "a": {"type": "typed-dict-field", "schema": {"type": "int"}},
        "b": {"type": "typed-dict-field", "schema": {"type": "list", "items_schema": {"type": "int"}}}
    }}"#;
    let options = ValidateOptions {
        allow_partial: PartialMode::On,
        ..ValidateOptions::default()
    };
    assert_eq!(
        validator(schema)
            .validate_json(r#"{"a": 1, "b": [1, 2"#, &options)
            .unwrap(),
        j(r#"{"a": 1, "b": [1, 2]}"#)
    );
}

#[test]
fn perl_hashes_are_typed_dicts() {
    let result =
        validator(MOVIE).validate_value_as(&j("[1]"), InputType::Perl, &ValidateOptions::default());
    let Err(ValidateError::Validation(e)) = result else {
        panic!()
    };
    assert_eq!(
        e.errors(&ErrorsOptions::default())[0].msg,
        "Input should be a hash reference"
    );
}

#[test]
fn serializers_emit_fields_and_allowed_extras() {
    let json = |schema: &str, value: &str, options: &SerializeOptions| {
        SchemaSerializer::new(&j(schema), None)
            .unwrap()
            .to_json(&j(value), options, &JsonOptions::default())
            .unwrap()
            .output
    };
    let defaults = SerializeOptions::default();
    assert_eq!(
        json(MOVIE, r#"{"b": 1, "a": "x", "c": 2}"#, &defaults),
        r#"{"b":1,"a":"x"}"#
    );
    let allow = with(MOVIE, r#""extra_behavior": "allow""#);
    let exclude_a = SerializeOptions {
        exclude: Some(j(r#"{"a": true}"#)),
        ..SerializeOptions::default()
    };
    assert_eq!(
        json(&allow, r#"{"b": 1, "a": "x", "c": 2}"#, &exclude_a),
        r#"{"b":1,"c":2}"#
    );
    let exclude_none = SerializeOptions {
        exclude_none: true,
        ..SerializeOptions::default()
    };
    assert_eq!(
        json(&allow, r#"{"b": 1, "a": "x", "c": null}"#, &exclude_none),
        r#"{"b":1,"a":"x"}"#
    );
    // computed fields need host callbacks
    assert!(
        SchemaSerializer::new(
            &j(&with(
                MOVIE,
                r#""computed_fields": [{"type": "computed-field", "property_name": "p", "return_schema": {"type": "int"}}]"#
            )),
            None
        )
        .is_err()
    );
}

#[test]
fn json_schema_lists_required_keys_and_extras() {
    let generate = |schema: &str| {
        generate_json_schema(&j(schema), None, &JsonSchemaOptions::default())
            .unwrap()
            .schema
    };
    assert_eq!(
        generate(MOVIE),
        j(r#"{"type": "object", "properties": {
            "b": {"type": "integer", "title": "B"},
            "a": {"type": "string", "title": "A"}
        }, "required": ["b"]}"#)
    );
    let forbid = generate(&with(MOVIE, r#""extra_behavior": "forbid""#));
    let Value::Dict(forbid) = forbid else {
        panic!()
    };
    assert_eq!(
        forbid.get_str("additionalProperties"),
        Some(&Value::Bool(false))
    );
    let extras = generate(&with(
        MOVIE,
        r#""extra_behavior": "allow", "extras_schema": {"type": "int"}"#,
    ));
    let Value::Dict(extras) = extras else {
        panic!()
    };
    assert_eq!(
        extras.get_str("additionalProperties"),
        Some(&j(r#"{"type": "integer"}"#))
    );
    let optional = generate(&with(MOVIE, r#""total": false"#));
    let Value::Dict(optional) = optional else {
        panic!()
    };
    assert_eq!(optional.get_str("required"), None);
}
