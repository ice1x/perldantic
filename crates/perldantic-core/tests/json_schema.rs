//! Replays the JSON Schema cases recorded from pydantic (tests/json_schema/cases.json, written by
//! tools/json_schema/record.sh): every case must produce pydantic's JSON Schema, including key
//! order, or its error, and the same warnings.

use std::fs;
use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use perldantic_core::{
    Dict, JsonSchemaMode, JsonSchemaOptions, UnionFormat, Value, generate_json_schema,
};
use serde_json::Value as Json;

fn cases() -> Vec<Json> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/json_schema/cases.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Decode the conformance value encoding (tests/conformance/README.md); a class is its name.
fn decode(json: &Json) -> Value {
    match json {
        Json::Null => Value::None,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Float(n.as_f64().unwrap()), Value::Int),
        Json::String(s) => Value::Str(s.clone()),
        Json::Array(items) => Value::List(items.iter().map(decode).collect()),
        Json::Object(map) => {
            if map.len() == 1 {
                let (tag, payload) = map.iter().next().unwrap();
                if let Some(tag) = tag.strip_prefix('$') {
                    return decode_tag(tag, payload);
                }
            }
            Value::Dict(
                map.iter()
                    .map(|(k, v)| (Value::Str(k.clone()), decode(v)))
                    .collect(),
            )
        }
    }
}

fn decode_tag(tag: &str, payload: &Json) -> Value {
    let items = || payload.as_array().unwrap().iter().map(decode).collect();
    match tag {
        "tuple" => Value::Tuple(items()),
        "set" => Value::Set(items()),
        "bytes" => Value::Bytes(STANDARD.decode(payload.as_str().unwrap()).unwrap()),
        "float" => Value::Float(match payload.as_str().unwrap() {
            "inf" => f64::INFINITY,
            "-inf" => f64::NEG_INFINITY,
            _ => f64::NAN,
        }),
        "dict" => Value::Dict(
            payload
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| (decode(&pair[0]), decode(&pair[1])))
                .collect::<Dict>(),
        ),
        "class" => Value::Str(payload.as_str().unwrap().to_owned()),
        other => panic!("unexpected tag ${other}"),
    }
}

fn options(case: &Json) -> JsonSchemaOptions {
    let recorded = &case["options"];
    let mut options = JsonSchemaOptions {
        mode: match case["mode"].as_str().unwrap() {
            "validation" => JsonSchemaMode::Validation,
            "serialization" => JsonSchemaMode::Serialization,
            other => panic!("unexpected mode {other}"),
        },
        ..JsonSchemaOptions::default()
    };
    for (key, value) in recorded.as_object().unwrap() {
        match key.as_str() {
            "by_alias" => options.by_alias = value.as_bool().unwrap(),
            "ref_template" => value
                .as_str()
                .unwrap()
                .clone_into(&mut options.ref_template),
            "union_format" => {
                options.union_format = match value.as_str().unwrap() {
                    "any_of" => UnionFormat::AnyOf,
                    "primitive_type_array" => UnionFormat::PrimitiveTypeArray,
                    other => panic!("unexpected union format {other}"),
                }
            }
            other => panic!("unexpected option {other}"),
        }
    }
    options
}

#[test]
fn json_schemas_match_pydantic() {
    let cases = cases();
    assert!(cases.len() > 30);
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        let result = generate_json_schema(&decode(&case["schema"]), &options(case));
        let expected = &case["expected"];
        if let Some(schema) = expected.get("json_schema") {
            let generated = result.unwrap_or_else(|e| panic!("{id}: unexpected error {e}"));
            let schema = decode(schema);
            assert_eq!(generated.schema, schema, "{id}");
            // Value equality ignores dict order; pydantic's key order is part of the output.
            assert_eq!(
                format!("{:?}", generated.schema),
                format!("{schema:?}"),
                "{id}: key order"
            );
            let warnings: Vec<String> = case["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| w.as_str().unwrap().to_owned())
                .collect();
            assert_eq!(generated.warnings, warnings, "{id}: warnings");
        } else {
            let error = result.expect_err(id);
            assert_eq!(
                [error.python_name(), &error.to_string()],
                [
                    expected["error"][0].as_str().unwrap(),
                    expected["error"][1].as_str().unwrap()
                ],
                "{id}"
            );
        }
    }
}

fn generate(schema: &str) -> Result<Value, (String, String)> {
    generate_json_schema(
        &Value::from_json(schema).unwrap(),
        &JsonSchemaOptions::default(),
    )
    .map(|g| g.schema)
    .map_err(|e| (e.python_name().to_owned(), e.to_string()))
}

#[test]
fn default_options_match_pydantic() {
    let options = JsonSchemaOptions::default();
    assert_eq!(options.mode, JsonSchemaMode::Validation);
    assert!(options.by_alias);
    assert_eq!(options.ref_template, "#/$defs/{model}");
    assert_eq!(options.union_format, UnionFormat::AnyOf);
}

#[test]
fn schemas_that_cannot_be_described_yet_are_errors() {
    assert_eq!(
        generate(r#"{"type": "date"}"#).unwrap_err(),
        (
            "SchemaError".to_owned(),
            "JSON Schema generation for `date` schemas is not supported yet".to_owned()
        )
    );
    // Upstream's callables need host callbacks.
    assert_eq!(
        generate(r#"{"type": "int", "metadata": {"pydantic_js_functions": ["f"]}}"#)
            .unwrap_err()
            .1,
        "`pydantic_js_functions` are not supported yet: host callbacks are not implemented"
    );
    assert_eq!(
        generate(r#"{"type": "int", "metadata": {"pydantic_js_extra": "f"}}"#)
            .unwrap_err()
            .0,
        "SchemaError"
    );
}

#[test]
fn malformed_schemas_are_errors() {
    assert_eq!(
        generate(r#"{"type": "list", "items_schema": 1}"#).unwrap_err(),
        (
            "TypeError".to_owned(),
            "'int' object is not an instance of 'dict'".to_owned()
        )
    );
    assert_eq!(
        generate(
            r#"{"type": "model", "cls": "M", "config": {"json_schema_extra": 1},
                "schema": {"type": "model-fields", "fields": {}}}"#
        )
        .unwrap_err(),
        (
            "ValueError".to_owned(),
            "model_config['json_schema_extra']=1 should be a dict, callable, or None".to_owned()
        )
    );
}

#[test]
fn http_references_are_left_alone() {
    let schema = generate(r#"{"type": "list", "metadata": {"pydantic_js_updates": {"items": {"$ref": "https://example.com/s"}}}}"#)
        .unwrap();
    assert_eq!(
        schema,
        Value::from_json(r#"{"items": {"$ref": "https://example.com/s"}, "type": "array"}"#)
            .unwrap()
    );
}
