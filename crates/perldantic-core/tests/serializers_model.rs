//! Model serializers: `model` and `model-fields`, including aliases, `exclude_*` options,
//! extra values and root models. Expectations come from pydantic-core 2.49.0.

use perldantic_core::{
    Dict, JsonOptions, Model, SchemaSerializer, SerializeError, SerializeOptions, Serialized, Value,
};

fn j(json: &str) -> Value {
    Value::from_json(json).unwrap()
}

fn serializer(schema: &str) -> SchemaSerializer {
    SchemaSerializer::new(&j(schema), None).unwrap()
}

fn opts() -> SerializeOptions {
    SerializeOptions::default()
}

fn dict(json: &str) -> Dict {
    match j(json) {
        Value::Dict(d) => d,
        other => panic!("not a dict: {other:?}"),
    }
}

fn model(class: &str, fields: &str, fields_set: &[&str], extra: Option<&str>) -> Value {
    Value::Model(Box::new(Model {
        class: class.to_owned(),
        fields: dict(fields),
        fields_set: fields_set.iter().map(|f| Value::from(*f)).collect(),
        extra: extra.map(dict),
    }))
}

fn fallback(expected: &str, repr: &str, type_: &str) -> String {
    format!(
        "Pydantic serializer warnings:\n  PydanticSerializationUnexpectedValue(Expected `{expected}` - serialized value may not be as expected [input_value={repr}, input_type={type_}])"
    )
}

const FIELDS: &str = r#"{
    "type": "model-fields",
    "fields": {
        "a": {"type": "model-field", "schema": {"type": "int"}, "serialization_alias": "A"},
        "b": {"type": "model-field", "schema": {"type": "default", "schema": {"type": "nullable", "schema": {"type": "str"}}, "default": null}},
        "c": {"type": "model-field", "schema": {"type": "default", "schema": {"type": "int"}, "default": 3}},
        "secret": {"type": "model-field", "schema": {"type": "str"}, "serialization_exclude": true}
    }
}"#;

fn model_schema(config: &str) -> String {
    format!(r#"{{"type": "model", "cls": "M", "schema": {FIELDS}, "config": {config}}}"#)
}

/// `M.model_validate({'a': 1, 'secret': 'x'})`
fn m() -> Value {
    model(
        "M",
        r#"{"a": 1, "b": null, "c": 3, "secret": "x"}"#,
        &["a", "secret"],
        None,
    )
}

fn out(s: &SchemaSerializer, value: &Value, options: &SerializeOptions) -> Value {
    let result = s.to_python(value, options).unwrap();
    assert_eq!(result.warning, None);
    result.output
}

#[test]
fn models_serialize_their_fields() {
    let s = serializer(&model_schema("{}"));
    assert_eq!(out(&s, &m(), &opts()), j(r#"{"a": 1, "b": null, "c": 3}"#));
    assert_eq!(
        s.to_json(&m(), &opts(), &JsonOptions::default()).unwrap(),
        Serialized {
            output: r#"{"a":1,"b":null,"c":3}"#.to_owned(),
            warning: None
        }
    );
}

#[test]
fn aliases_come_from_the_call_or_the_model_config() {
    let s = serializer(&model_schema("{}"));
    let by_alias = SerializeOptions {
        by_alias: Some(true),
        ..opts()
    };
    assert_eq!(
        out(&s, &m(), &by_alias),
        j(r#"{"A": 1, "b": null, "c": 3}"#)
    );
    let configured = serializer(&model_schema(r#"{"serialize_by_alias": true}"#));
    assert_eq!(
        out(&configured, &m(), &opts()),
        j(r#"{"A": 1, "b": null, "c": 3}"#)
    );
}

#[test]
fn exclude_options_drop_fields() {
    let s = serializer(&model_schema("{}"));
    for options in [
        SerializeOptions {
            exclude_unset: true,
            ..opts()
        },
        SerializeOptions {
            exclude_defaults: true,
            ..opts()
        },
    ] {
        assert_eq!(out(&s, &m(), &options), j(r#"{"a": 1}"#));
    }
    let exclude_none = SerializeOptions {
        exclude_none: true,
        ..opts()
    };
    assert_eq!(out(&s, &m(), &exclude_none), j(r#"{"a": 1, "c": 3}"#));
}

#[test]
fn include_and_exclude_filter_fields() {
    let s = serializer(&model_schema("{}"));
    let include = SerializeOptions {
        include: Some(Value::Set(vec![Value::from("a")])),
        ..opts()
    };
    assert_eq!(out(&s, &m(), &include), j(r#"{"a": 1}"#));
    let exclude = SerializeOptions {
        exclude: Some(j(r#"{"a": true}"#)),
        ..opts()
    };
    assert_eq!(out(&s, &m(), &exclude), j(r#"{"b": null, "c": 3}"#));
}

#[test]
fn values_that_are_not_instances_are_inferred_with_a_warning() {
    let s = serializer(&model_schema("{}"));
    let input = j(r#"{"a": 1}"#);
    assert_eq!(
        s.to_python(&input, &opts()).unwrap(),
        Serialized {
            output: input.clone(),
            warning: Some(fallback("M", "{'a': 1}", "dict")),
        }
    );
    assert_eq!(
        s.to_json(&input, &opts(), &JsonOptions::default()).unwrap(),
        Serialized {
            output: r#"{"a":1}"#.to_owned(),
            warning: Some(fallback("M", "{'a': 1}", "dict")),
        }
    );
}

#[test]
fn extra_values_follow_the_fields() {
    let schema = r#"{
        "type": "model", "cls": "M", "config": {"extra_fields_behavior": "allow"},
        "schema": {
            "type": "model-fields", "extra_behavior": "allow", "extras_schema": {"type": "str"},
            "fields": {"a": {"type": "model-field", "schema": {"type": "int"}}}
        }
    }"#;
    let s = serializer(schema);
    let with_extra = model("M", r#"{"a": 1}"#, &["a", "z"], Some(r#"{"z": "q"}"#));
    assert_eq!(out(&s, &with_extra, &opts()), j(r#"{"a": 1, "z": "q"}"#));
    assert_eq!(
        s.to_json(&with_extra, &opts(), &JsonOptions::default())
            .unwrap()
            .output,
        r#"{"a":1,"z":"q"}"#
    );
    let exclude = SerializeOptions {
        exclude: Some(Value::Set(vec![Value::from("z")])),
        ..opts()
    };
    assert_eq!(out(&s, &with_extra, &exclude), j(r#"{"a": 1}"#));

    // extra values are checked against `extras_schema`
    let wrong = model("M", r#"{"a": 1}"#, &["a"], Some(r#"{"z": "q", "y": 5}"#));
    assert_eq!(
        s.to_python(&wrong, &opts()).unwrap(),
        Serialized {
            output: j(r#"{"a": 1, "z": "q", "y": 5}"#),
            warning: Some(fallback("str", "5", "int")),
        }
    );
}

#[test]
fn extra_allowed_only_on_the_fields_schema_falls_back() {
    // The model does not know about the extra values, so the fields serializer gets a dict where
    // it expects a tuple; upstream behaves the same way.
    let schema = r#"{
        "type": "model", "cls": "M",
        "schema": {
            "type": "model-fields", "extra_behavior": "allow",
            "fields": {"a": {"type": "model-field", "schema": {"type": "int"}}}
        }
    }"#;
    let s = serializer(schema);
    let value = model("M", r#"{"a": 1}"#, &["a", "z"], Some(r#"{"z": "q"}"#));
    assert_eq!(
        s.to_python(&value, &opts()).unwrap(),
        Serialized {
            output: j(r#"{"a": 1}"#),
            warning: Some(fallback("general-fields", "{'a': 1}", "dict")),
        }
    );
}

#[test]
fn root_models_serialize_their_root() {
    let s = serializer(
        r#"{"type": "model", "cls": "R", "root_model": true,
            "schema": {"type": "list", "items_schema": {"type": "int"}}}"#,
    );
    let r = model("R", r#"{"root": [1, 2]}"#, &["root"], None);
    assert_eq!(out(&s, &r, &opts()), j("[1, 2]"));
    assert_eq!(
        s.to_json(&r, &opts(), &JsonOptions::default())
            .unwrap()
            .output,
        "[1,2]"
    );
    assert_eq!(
        s.to_python(&j("[1]"), &opts()).unwrap(),
        Serialized {
            output: j("[1]"),
            warning: Some(fallback("R", "[1]", "list")),
        }
    );
}

#[test]
fn unions_pick_the_model_member() {
    let s = serializer(&format!(
        r#"{{"type": "union", "choices": [{}, {{"type": "int"}}]}}"#,
        model_schema("{}")
    ));
    assert_eq!(out(&s, &m(), &opts()), j(r#"{"a": 1, "b": null, "c": 3}"#));
    assert_eq!(out(&s, &Value::Int(5), &opts()), Value::Int(5));
}

/// Models built outside the validator (the Perl objects) may leave out fields that have a
/// default; a union still picks their model instead of warning about the field count.
#[test]
fn unions_accept_models_without_defaulted_fields() {
    let s = serializer(&format!(
        r#"{{"type": "union", "choices": [{}, {{"type": "int"}}]}}"#,
        model_schema("{}")
    ));
    let partial = model(
        "M",
        r#"{"a": 1, "b": null, "secret": "x"}"#,
        &["a", "secret"],
        None,
    );
    assert_eq!(out(&s, &partial, &opts()), j(r#"{"a": 1, "b": null}"#));
}

#[test]
fn fields_serializer_needs_a_model() {
    let s = serializer(FIELDS);
    let err = s
        .to_python(&j(r#"{"a": 1, "b": null, "c": 3, "secret": "s"}"#), &opts())
        .unwrap_err();
    assert!(matches!(err, SerializeError::UnexpectedValue(_)));
    assert_eq!(
        err.py_display(),
        "PydanticSerializationUnexpectedValue: No model found for fields serialization"
    );
}

#[test]
fn schema_errors() {
    let err = SchemaSerializer::new(
        &j(
            r#"{"type": "model-fields", "extras_schema": {"type": "str"},
               "fields": {"a": {"type": "model-field", "schema": {"type": "int"}}}}"#,
        ),
        None,
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Error building `model-fields` serializer:\n  SchemaError: extras_schema can only be used if extra_behavior=allow"
    );
    let err = SchemaSerializer::new(
        &j(
            r#"{"type": "model-fields", "fields": {}, "computed_fields": [
                {"type": "computed-field", "property_name": "area", "return_schema": {"type": "int"}}
            ]}"#,
        ),
        None,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Computed field `area` needs the host `function` computing it"),
        "{err}"
    );
}
