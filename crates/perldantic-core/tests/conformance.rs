//! Replays the conformance cases recorded from pydantic-core (see tests/conformance/README.md).
//!
//! A case is active when every schema type it uses is registered, all its options are
//! supported and all its values can be represented; active cases must reproduce pydantic's
//! outcome exactly. Everything else is skipped and counted, so cases activate automatically as
//! validators are ported. Run with `--nocapture` to see the summary.

// The summary on stdout is the point of this runner.
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use perldantic_core::{
    ErrorDetails, ErrorsOptions, ExtraBehavior, LocItem, PartialMode, SchemaValidator,
    ValidateError, ValidateOptions, Value,
};
use serde_json::Value as Json;

/// Why a case cannot run yet.
#[derive(Debug, PartialEq)]
struct Skip(String);

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/upstream")
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

/// Decode a tagged conformance value (see the README's "Value encoding").
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
    let items = |p: &Json| -> Result<Vec<Value>, Skip> {
        p.as_array().unwrap().iter().map(decode).collect()
    };
    Ok(match tag {
        "tuple" => Value::Tuple(items(payload)?),
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
                .collect::<Result<_, Skip>>()?,
        ),
        other => return Err(Skip(format!("value ${other}"))),
    })
}

/// Every `type` string in a schema tree; nested non-schema dicts are included, which only
/// makes the support check more conservative.
fn schema_types(json: &Json, out: &mut Vec<String>) {
    match json {
        Json::Object(map) => {
            if let Some(Json::String(t)) = map.get("type") {
                out.push(t.clone());
            }
            map.values().for_each(|v| schema_types(v, out));
        }
        Json::Array(items) => items.iter().for_each(|v| schema_types(v, out)),
        _ => {}
    }
}

fn options(json: &Json) -> Result<ValidateOptions, Skip> {
    let bool_opt = |key: &str, value: &Json| -> Result<Option<bool>, Skip> {
        match value {
            Json::Null => Ok(None),
            Json::Bool(b) => Ok(Some(*b)),
            _ => Err(Skip(format!("option {key}={value}"))),
        }
    };
    let mut opts = ValidateOptions::default();
    for (key, value) in json.as_object().unwrap() {
        match key.as_str() {
            "strict" => opts.strict = bool_opt(key, value)?,
            "from_attributes" => opts.from_attributes = bool_opt(key, value)?,
            "by_alias" => opts.by_alias = bool_opt(key, value)?,
            "by_name" => opts.by_name = bool_opt(key, value)?,
            "context" => opts.context = (!value.is_null()).then(|| decode(value)).transpose()?,
            "extra" => {
                opts.extra_behavior = match value {
                    Json::Null => None,
                    Json::String(s) => {
                        Some(s.parse().map_err(|_| Skip(format!("option extra={s}")))?)
                    }
                    _ => return Err(Skip(format!("option extra={value}"))),
                }
            }
            "allow_partial" => {
                opts.allow_partial = match value {
                    Json::Bool(b) => (*b).into(),
                    Json::String(s) if s == "trailing-strings" => PartialMode::TrailingStrings,
                    _ => return Err(Skip(format!("option allow_partial={value}"))),
                }
            }
            _ => return Err(Skip(format!("option {key}"))),
        }
    }
    Ok(opts)
}

fn loc_json(loc: &[LocItem]) -> Json {
    Json::Array(
        loc.iter()
            .map(|item| match item {
                LocItem::S(s) => Json::from(s.as_str()),
                LocItem::I(i) => Json::from(*i),
            })
            .collect(),
    )
}

/// Values compare like Python `==`, except that NaN equals NaN (a repeat of the same input).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Value::List(x), Value::List(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_value(a, b))
        }
        (Value::Dict(x), Value::Dict(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_value(v, w)))
        }
        _ => a == b,
    }
}

fn compare_errors(expected: &[Json], actual: &[ErrorDetails]) -> Result<(), String> {
    if expected.len() != actual.len() {
        return Err(format!(
            "{} errors expected, got {actual:#?}",
            expected.len()
        ));
    }
    for (exp, act) in expected.iter().zip(actual) {
        let exp_input = exp.get("input").map(decode);
        let exp_ctx = exp.get("ctx").map(decode);
        let matches = exp["type"] == act.type_.as_str()
            && exp["loc"] == loc_json(&act.loc)
            && exp["msg"] == act.msg.as_str()
            && match (exp_input, &act.input) {
                (Some(Ok(e)), Some(a)) => same_value(&e, a),
                // An unrepresentable input cannot be compared.
                (Some(Err(_)), _) | (None, None) => true,
                _ => false,
            }
            && match (exp_ctx, &act.ctx) {
                (Some(Ok(e)), Some(a)) => same_value(&e, a),
                (Some(Err(_)), Some(_)) | (None, None) => true,
                _ => false,
            };
        if !matches {
            return Err(format!("expected error {exp}, got {act:#?}"));
        }
    }
    Ok(())
}

/// Cases that depend on a documented divergence (docs/DIVERGENCES.md) are skipped.
fn divergence(case: &Json) -> Option<Skip> {
    let uses_python_re = |j: &Json| j.to_string().contains(r#""regex_engine":"python-re""#);
    (uses_python_re(&case["schema"]) || uses_python_re(&case["config"]))
        .then(|| Skip("divergence #7: python-re regex engine".into()))
}

/// Run one case: `Ok(Ok(()))` passed, `Ok(Err(msg))` failed, `Err(skip)` skipped.
fn run_case(case: &Json, supported: &[&str]) -> Result<Result<(), String>, Skip> {
    if let Some(skip) = divergence(case) {
        return Err(skip);
    }
    let mut types = Vec::new();
    schema_types(&case["schema"], &mut types);
    if let Some(t) = types.iter().find(|t| !supported.contains(&t.as_str())) {
        return Err(Skip(format!("schema type {t}")));
    }
    let schema = decode(&case["schema"])?;
    let config = match &case["config"] {
        Json::Null => None,
        c => Some(decode(c)?),
    };
    let opts = options(&case["options"])?;
    let expected = &case["expected"];

    let validator = match SchemaValidator::new(&schema, config.as_ref()) {
        Ok(v) => v,
        Err(err) => {
            return Ok(Err(format!("building the validator failed: {err:?}")));
        }
    };
    let result = match case["mode"].as_str().unwrap() {
        "python" => validator.validate_value(&decode(&case["input"])?, &opts),
        "json" => match &case["input"] {
            Json::String(text) => validator.validate_json(text, &opts),
            _ => return Err(Skip("non-string JSON input".into())),
        },
        other => panic!("unknown mode {other}"),
    };

    Ok(match (result, expected) {
        (Ok(output), _) if expected.get("output").is_some() => {
            let want = decode(&expected["output"])?;
            if same_value(&want, &output) {
                Ok(())
            } else {
                Err(format!("expected output {want:?}, got {output:?}"))
            }
        }
        (Err(ValidateError::Validation(error)), _) if expected.get("errors").is_some() => {
            let details = error.errors(&ErrorsOptions {
                include_url: false,
                ..ErrorsOptions::default()
            });
            compare_errors(expected["errors"].as_array().unwrap(), &details).and_then(|()| {
                if expected["title"] == error.title() {
                    Ok(())
                } else {
                    Err(format!(
                        "expected title {}, got {}",
                        expected["title"],
                        error.title()
                    ))
                }
            })
        }
        (Err(ValidateError::Core(err)), _) if expected.get("exception").is_some() => {
            let want = expected["exception"]["type"].as_str().unwrap();
            if err.kind().python_name() == want {
                Ok(())
            } else {
                Err(format!("expected exception {want}, got {err:?}"))
            }
        }
        (actual, _) => Err(format!("expected {expected}, got {actual:?}")),
    })
}

#[test]
fn replay_upstream_cases() {
    let mut files = Vec::new();
    case_files(&cases_root(), &mut files);
    files.sort();
    let supported = SchemaValidator::supported_schema_types();

    let (mut total, mut passed) = (0, 0);
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for file in &files {
        let cases: Vec<Json> = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        for case in &cases {
            total += 1;
            match run_case(case, supported) {
                Ok(Ok(())) => passed += 1,
                Ok(Err(msg)) => failures.push(format!("{}: {msg}", case["id"])),
                Err(Skip(reason)) => *skipped.entry(reason).or_default() += 1,
            }
        }
    }

    let skipped_total: usize = skipped.values().sum();
    println!(
        "conformance: {total} cases, {} active, {passed} passed, {} failed, {skipped_total} skipped",
        total - skipped_total,
        failures.len()
    );
    let mut reasons: Vec<_> = skipped.iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(a.1));
    for (reason, count) in reasons.iter().take(15) {
        println!("  skipped {count:>5}: {reason}");
    }
    // Guard against silently losing the case files.
    assert!(total > 5000, "only {total} conformance cases found");
    assert!(
        failures.is_empty(),
        "{} conformance failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn decoder_handles_every_representable_tag() {
    let json: Json = serde_json::from_str(
        r#"[{"$tuple": [1]}, {"$bytes": "YQ=="}, {"$float": "-inf"}, {"$dict": [[1, "x"]]}, 18446744073709551616, 1.0]"#,
    )
    .unwrap();
    let Value::List(items) = decode(&json).unwrap() else {
        panic!("expected list")
    };
    assert_eq!(items[0], Value::Tuple(vec![Value::Int(1)]));
    assert_eq!(items[1], Value::Bytes(b"a".to_vec()));
    assert_eq!(items[2], Value::Float(f64::NEG_INFINITY));
    assert_eq!(
        items[3],
        Value::Dict([(Value::Int(1), Value::from("x"))].into_iter().collect())
    );
    assert_eq!(
        items[4],
        Value::BigInt("18446744073709551616".parse().unwrap())
    );
    assert_eq!(items[5], Value::Float(1.0));
    assert_eq!(
        decode(&serde_json::from_str::<Json>(r#"{"$function": "f"}"#).unwrap()),
        Err(Skip("value $function".into()))
    );
}

#[test]
fn unsupported_cases_are_skipped_with_a_reason() {
    let case: Json = serde_json::from_str(
        r#"{"schema": {"type": "int"}, "config": null, "mode": "python", "input": 1, "options": {}, "expected": {"output": 1}}"#,
    )
    .unwrap();
    assert_eq!(run_case(&case, &[]), Err(Skip("schema type int".into())));
    let case: Json = serde_json::from_str(
        r#"{"schema": {"type": "any"}, "config": null, "mode": "python", "input": 1, "options": {"self_instance": 1}, "expected": {"output": 1}}"#,
    )
    .unwrap();
    assert_eq!(
        run_case(&case, &["any"]),
        Err(Skip("option self_instance".into()))
    );
}

#[test]
fn options_map_onto_validate_options() {
    let json: Json = serde_json::from_str(
        r#"{"strict": null, "extra": "forbid", "context": {"a": 1}, "allow_partial": "trailing-strings", "by_alias": false}"#,
    )
    .unwrap();
    let opts = options(&json).unwrap();
    assert_eq!(opts.strict, None);
    assert_eq!(opts.extra_behavior, Some(ExtraBehavior::Forbid));
    assert_eq!(opts.context, Some(decode(&json["context"]).unwrap()));
    assert!(matches!(opts.allow_partial, PartialMode::TrailingStrings));
    assert_eq!(opts.by_alias, Some(false));
    assert_eq!(
        options(&serde_json::from_str::<Json>(r#"{"strict": 1}"#).unwrap()).unwrap_err(),
        Skip("option strict=1".into())
    );
}

#[test]
fn mismatches_are_reported_as_failures() {
    let case: Json = serde_json::from_str(
        r#"{"schema": {"type": "any"}, "config": null, "mode": "python", "input": 1, "options": {}, "expected": {"output": 2}}"#,
    )
    .unwrap();
    assert!(run_case(&case, &["any"]).unwrap().is_err());
}

#[test]
fn documented_divergences_are_skipped() {
    let case: Json = serde_json::from_str(
        r#"{"schema": {"type": "str", "pattern": "a"}, "config": {"regex_engine": "python-re"}, "mode": "python", "input": "a", "options": {}, "expected": {"output": "a"}}"#,
    )
    .unwrap();
    assert_eq!(
        run_case(&case, &["str"]),
        Err(Skip("divergence #7: python-re regex engine".into()))
    );
}
