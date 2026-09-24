//! Replays the JSON Schema cases recorded from pydantic (tests/json_schema/README.md):
//!
//! - `cases.json`, hand-picked core schemas: every case runs and must match;
//! - `upstream/`, every JSON Schema pydantic's own test suite generates: a case runs when all its
//!   values can be represented and every schema type in it is supported, and must then match.
//!   Everything else is skipped and counted, so cases activate as the port grows. Run with
//!   `--nocapture` to see the summary.
//!
//! Matching means pydantic's JSON Schema, including key order, or its error, and the same
//! warnings.

// The summary on stdout is the point of the upstream runner.
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use perldantic_core::{
    Dict, JsonSchemaError, JsonSchemaMode, JsonSchemaOptions, UnionFormat, Value,
    generate_json_schema,
};
use perldantic_core::{MultiHostUrl, Url, speedate, temporal, uuid};
use serde_json::Value as Json;

/// Why a case cannot run yet.
#[derive(Debug)]
struct Skip(String);

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/json_schema")
}

fn read_cases(path: &Path) -> Vec<Json> {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn case_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            case_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
}

/// Decode the conformance value encoding (tests/conformance/README.md); a class is its name.
fn decode(json: &Json) -> Result<Value, Skip> {
    Ok(match json {
        Json::Null => Value::None,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if n.is_f64() {
                Value::Float(n.as_f64().unwrap())
            } else {
                Value::BigInt(n.to_string().parse().unwrap())
            }
        }
        Json::String(s) => Value::Str(s.clone()),
        Json::Array(items) => Value::List(items.iter().map(decode).collect::<Result<_, _>>()?),
        Json::Object(map) => {
            if map.len() == 1 {
                let (tag, payload) = map.iter().next().unwrap();
                if let Some(tag) = tag.strip_prefix('$') {
                    return decode_tag(tag, payload);
                }
            }
            Value::Dict(
                map.iter()
                    .map(|(k, v)| Ok((Value::Str(k.clone()), decode(v)?)))
                    .collect::<Result<_, Skip>>()?,
            )
        }
    })
}

fn decode_tag(tag: &str, payload: &Json) -> Result<Value, Skip> {
    let items =
        || -> Result<Vec<Value>, Skip> { payload.as_array().unwrap().iter().map(decode).collect() };
    Ok(match tag {
        "tuple" => Value::Tuple(items()?),
        "set" => Value::Set(items()?),
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
                .map(|pair| Ok((decode(&pair[0])?, decode(&pair[1])?)))
                .collect::<Result<Dict, Skip>>()?,
        ),
        "class" => Value::Str(payload.as_str().unwrap().to_owned()),
        "date" => Value::Date(speedate::Date::parse_str(payload.as_str().unwrap()).unwrap()),
        "time" => Value::Time(speedate::Time::parse_str(payload.as_str().unwrap()).unwrap()),
        "datetime" => {
            Value::DateTime(speedate::DateTime::parse_str(payload.as_str().unwrap()).unwrap())
        }
        "timedelta" => {
            let part = |i: usize| payload[i].as_i64().unwrap();
            Value::TimeDelta(temporal::duration_from_parts(part(0), part(1), part(2)).unwrap())
        }
        "uuid" => Value::Uuid(uuid::Uuid::parse_str(payload.as_str().unwrap()).unwrap()),
        // `str(url)`: an empty path is kept empty so the text round-trips
        "url" => Value::Url(Box::new(
            Url::parse(payload.as_str().unwrap(), true).unwrap(),
        )),
        "multi_host_url" => Value::MultiHostUrl(Box::new(
            MultiHostUrl::parse(payload.as_str().unwrap(), true).unwrap(),
        )),
        other => return Err(Skip(format!("${other} value"))),
    })
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

fn warnings(case: &Json) -> Vec<String> {
    case["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_owned())
        .collect()
}

/// Whether the port cannot describe the schema yet (rather than got it wrong).
fn unsupported(error: &JsonSchemaError) -> Option<String> {
    let message = error.to_string();
    (message.contains("is not supported yet") || message.contains("are not supported yet"))
        .then_some(message)
}

/// Run one case: `Ok(Err(skip))` when it cannot run, `Ok(Ok(()))` when it matches pydantic.
fn run_case(case: &Json) -> Result<Result<(), Skip>, String> {
    let schema = match decode(&case["schema"]) {
        Ok(schema) => schema,
        Err(skip) => return Ok(Err(skip)),
    };
    let config = match decode(&case["config"]) {
        Ok(config) => config,
        Err(skip) => return Ok(Err(skip)),
    };
    let result = generate_json_schema(&schema, Some(&config), &options(case));
    if let Err(error) = &result
        && let Some(message) = unsupported(error)
    {
        return Ok(Err(Skip(message)));
    }
    let expected = &case["expected"];
    if let Some(expected_schema) = expected.get("json_schema") {
        let expected_schema = match decode(expected_schema) {
            Ok(schema) => schema,
            Err(skip) => return Ok(Err(skip)),
        };
        let generated = result.map_err(|e| format!("unexpected error {}: {e}", e.python_name()))?;
        // Value equality ignores dict order; pydantic's key order is part of the output.
        if format!("{:?}", generated.schema) != format!("{expected_schema:?}") {
            return Err(format!(
                "schema differs\n  got:      {:?}\n  expected: {expected_schema:?}",
                generated.schema
            ));
        }
        if generated.warnings != warnings(case) {
            return Err(format!(
                "warnings differ: got {:?}, expected {:?}",
                generated.warnings,
                warnings(case)
            ));
        }
    } else {
        let expected = [
            expected["error"][0].as_str().unwrap(),
            expected["error"][1].as_str().unwrap(),
        ];
        match result {
            Ok(generated) => {
                return Err(format!("expected {expected:?}, got {:?}", generated.schema));
            }
            Err(error) => {
                let got = [error.python_name(), &error.to_string()];
                if got != expected {
                    return Err(format!("expected {expected:?}, got {got:?}"));
                }
            }
        }
    }
    Ok(Ok(()))
}

#[test]
fn hand_picked_cases_match_pydantic() {
    let cases = read_cases(&cases_dir().join("cases.json"));
    assert!(cases.len() > 30);
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        match run_case(case) {
            Ok(Ok(())) => {}
            Ok(Err(Skip(reason))) => panic!("{id}: skipped: {reason}"),
            Err(failure) => panic!("{id}: {failure}"),
        }
    }
}

#[test]
fn upstream_cases_match_pydantic() {
    let mut files = Vec::new();
    case_files(&cases_dir().join("upstream"), &mut files);
    files.sort();
    let (mut total, mut active) = (0, 0);
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for file in &files {
        for case in read_cases(file) {
            total += 1;
            match run_case(&case) {
                Ok(Ok(())) => active += 1,
                Ok(Err(Skip(reason))) => {
                    // group by reason, without case-specific details
                    let reason = reason.split(':').next().unwrap_or(&reason).to_owned();
                    *skipped.entry(reason).or_default() += 1;
                }
                Err(failure) => {
                    failures.push(format!("{}: {failure}", case["id"].as_str().unwrap()));
                }
            }
        }
    }
    println!(
        "JSON Schema upstream cases: {total} total, {active} active, {} failing",
        failures.len()
    );
    for (reason, count) in &skipped {
        println!("  skipped {count:>4}: {reason}");
    }
    assert!(active > 0, "no upstream case ran");
    assert!(
        failures.is_empty(),
        "{} upstream cases differ from pydantic:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn generate(schema: &str) -> Result<Value, (String, String)> {
    generate_json_schema(
        &Value::from_json(schema).unwrap(),
        None,
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
        generate(r#"{"type": "decimal"}"#).unwrap_err(),
        (
            "SchemaError".to_owned(),
            "JSON Schema generation for `decimal` schemas is not supported yet".to_owned()
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
