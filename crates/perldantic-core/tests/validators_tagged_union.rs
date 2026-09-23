//! `tagged-union` validator and discriminator lookup paths (upstream validators/union.rs,
//! common/union.rs, lookup_key.rs). Expectations come from pydantic-core 2.49.0.

use perldantic_core::{
    CoreError, ErrorsOptions, SchemaValidator, ValidateError, ValidateOptions, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn s(v: &str) -> Value {
    Value::from(v)
}

/// The recorded upstream schema: tags `'a'`, `'b'` and `1`.
fn schema(discriminator: Value, extra: &[(&str, Value)]) -> Value {
    let choices: perldantic_core::Dict = [
        (s("a"), j(r#"{"type": "dict", "keys_schema": {"type": "str"}}"#)),
        (
            s("b"),
            j(r#"{"type": "dict", "keys_schema": {"type": "str"}, "values_schema": {"type": "int"}}"#),
        ),
        (Value::Int(1), j(r#"{"type": "int"}"#)),
    ]
    .into_iter()
    .collect();
    let mut schema: perldantic_core::Dict = [
        (s("type"), s("tagged-union")),
        (s("choices"), Value::Dict(choices)),
        (s("discriminator"), discriminator),
    ]
    .into_iter()
    .collect();
    for (k, v) in extra {
        schema.insert(s(k), v.clone());
    }
    Value::Dict(schema)
}

fn validator(discriminator: Value) -> SchemaValidator {
    SchemaValidator::new(&schema(discriminator, &[]), None).unwrap()
}

fn lax() -> ValidateOptions {
    ValidateOptions::default()
}

/// Errors as `(type, loc, msg, ctx)`, with `loc` and `ctx` rendered as JSON.
fn errors(result: Result<Value, ValidateError>) -> Vec<(String, String, String, String)> {
    let Err(ValidateError::Validation(e)) = result else {
        panic!("expected a validation error, got {result:?}")
    };
    e.errors(&ErrorsOptions::default())
        .into_iter()
        .map(|d| {
            (
                d.type_,
                serde_json::to_string(&d.loc).unwrap(),
                d.msg,
                serde_json::to_string(&d.ctx).unwrap(),
            )
        })
        .collect()
}

fn locs(result: Result<Value, ValidateError>) -> Vec<String> {
    errors(result).into_iter().map(|e| e.1).collect()
}

#[test]
fn the_tag_selects_the_choice() {
    let v = validator(s("kind"));
    assert_eq!(v.title(), "tagged-union[dict[str,any],dict[str,int],int]");
    let input = j(r#"{"kind": "a", "y": 1}"#);
    assert_eq!(v.validate_value(&input, &lax()).unwrap(), input);
    // Errors are located under the tag.
    assert_eq!(
        locs(v.validate_value(&j(r#"{"kind": "b", "y": "x"}"#), &lax())),
        vec![r#"["b","kind"]"#, r#"["b","y"]"#]
    );
    assert_eq!(
        locs(v.validate_json(r#"{"kind": "b", "y": "z"}"#, &lax())),
        vec![r#"["b","kind"]"#, r#"["b","y"]"#]
    );
    assert_eq!(
        errors(v.validate_value(&j(r#"{"kind": 1}"#), &lax())),
        vec![(
            "int_type".into(),
            "[1]".into(),
            "Input should be a valid integer".into(),
            "null".into()
        )]
    );
}

#[test]
fn unknown_and_missing_tags() {
    let v = validator(s("kind"));
    assert_eq!(
        errors(v.validate_value(&j(r#"{"kind": "c"}"#), &lax())),
        vec![(
            "union_tag_invalid".into(),
            "[]".into(),
            "Input tag 'c' found using 'kind' does not match any of the expected tags: 'a', 'b', 1"
                .into(),
            r#"{"discriminator":"'kind'","tag":"c","expected_tags":"'a', 'b', 1"}"#.into()
        )]
    );
    assert_eq!(
        errors(v.validate_value(&j(r#"{"other": "c"}"#), &lax())),
        vec![(
            "union_tag_not_found".into(),
            "[]".into(),
            "Unable to extract tag using discriminator 'kind'".into(),
            r#"{"discriminator":"'kind'"}"#.into()
        )]
    );
}

#[test]
fn non_mappings_are_rejected() {
    let v = validator(s("kind"));
    for input in [s("notadict"), j("[1]")] {
        assert_eq!(
            errors(v.validate_value(&input, &lax()))[0].0,
            "model_attributes_type"
        );
    }
    assert_eq!(errors(v.validate_json("[1]", &lax()))[0].0, "dict_type");
}

#[test]
fn nested_paths_and_path_choices() {
    let nested = validator(j(r#"["meta", "kind"]"#));
    assert_eq!(
        locs(nested.validate_value(&j(r#"{"meta": {"kind": "b"}, "y": 2}"#), &lax())),
        vec![r#"["b","meta"]"#]
    );

    let choices = validator(j(r#"[["meta", "kind"], ["kind"]]"#));
    assert_eq!(
        locs(choices.validate_value(&j(r#"{"kind": "b", "y": 2}"#), &lax())),
        vec![r#"["b","kind"]"#]
    );

    let index = validator(j(r#"["meta", 0]"#));
    let input = j(r#"{"meta": ["a"], "y": 2}"#);
    assert_eq!(index.validate_value(&input, &lax()).unwrap(), input);

    let negative = validator(j(r#"["meta", -1]"#));
    assert_eq!(
        locs(negative.validate_value(&j(r#"{"meta": ["x", "b"], "y": 2}"#), &lax())),
        vec![r#"["b","meta"]"#]
    );
    // The discriminator is shown as a path.
    assert_eq!(
        errors(negative.validate_value(&j(r#"{"y": 2}"#), &lax()))[0].2,
        "Unable to extract tag using discriminator 'meta'.-1"
    );
}

#[test]
fn custom_errors_replace_tag_errors() {
    let v = SchemaValidator::new(
        &schema(
            s("kind"),
            &[
                ("custom_error_type", s("bad")),
                ("custom_error_message", s("Bad!")),
            ],
        ),
        None,
    )
    .unwrap();
    for input in [j(r#"{"kind": "zz"}"#), j("{}")] {
        assert_eq!(
            errors(v.validate_value(&input, &lax())),
            vec![("bad".into(), "[]".into(), "Bad!".into(), "null".into())]
        );
    }
}

fn build_type_error(discriminator: Value) -> String {
    match SchemaValidator::new(&schema(discriminator, &[]), None) {
        Err(e @ CoreError::Schema(_)) => e.to_string(),
        other => panic!("expected a schema error, got {other:?}"),
    }
}

#[test]
fn invalid_discriminators_explain_each_alias_form() {
    let head = "Error building \"tagged-union\" validator:\n  TypeError: failed to extract enum ValidationAlias ('Str | AliasPath | AliasChoices')\n";
    assert_eq!(
        build_type_error(j("[]")),
        format!(
            "{head}- variant Str (Str): TypeError: failed to extract field ValidationAlias::Str.0, caused by TypeError: 'list' object is not an instance of 'str'\n\
             - variant AliasPath (AliasPath): TypeError: failed to extract field ValidationAlias::AliasPath.0, caused by SchemaError: Each alias path should have at least one element\n\
             - variant AliasChoices (AliasChoices): TypeError: failed to extract field ValidationAlias::AliasChoices.0, caused by SchemaError: Lookup paths should have at least one element"
        )
    );
    assert_eq!(
        build_type_error(j("[[]]")),
        format!(
            "{head}- variant Str (Str): TypeError: failed to extract field ValidationAlias::Str.0, caused by TypeError: 'list' object is not an instance of 'str'\n\
             - variant AliasPath (AliasPath): TypeError: failed to extract field ValidationAlias::AliasPath.0, caused by TypeError: The first item in an alias path should be a string\n\
             - variant AliasChoices (AliasChoices): TypeError: failed to extract field ValidationAlias::AliasChoices.0, caused by SchemaError: Each alias path should have at least one element"
        )
    );
    assert_eq!(
        build_type_error(j("[1]")),
        format!(
            "{head}- variant Str (Str): TypeError: failed to extract field ValidationAlias::Str.0, caused by TypeError: 'list' object is not an instance of 'str'\n\
             - variant AliasPath (AliasPath): TypeError: failed to extract field ValidationAlias::AliasPath.0, caused by TypeError: The first item in an alias path should be a string\n\
             - variant AliasChoices (AliasChoices): TypeError: failed to extract field ValidationAlias::AliasChoices.0, caused by TypeError: 'int' object is not an instance of 'list'"
        )
    );
}

#[test]
fn strict_mode_still_selects_by_tag() {
    let v = SchemaValidator::new(
        &j(r#"{"type": "tagged-union", "choices": {"a": {"type": "int"}}, "discriminator": "k", "strict": true}"#),
        None,
    )
    .unwrap();
    assert_eq!(v.title(), "tagged-union[int]");
    assert_eq!(
        locs(v.validate_value(&j(r#"{"k": "a"}"#), &lax())),
        vec![r#"["a"]"#]
    );
}
