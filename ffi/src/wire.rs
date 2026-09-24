//! The wire format of values crossing the C ABI: JSON, with tagged objects for what JSON cannot
//! express. It is the conformance value encoding (tests/conformance/README.md), restricted to
//! what `Value` holds:
//!
//! - `null`, booleans, integers of any size, finite floats (written with a fraction or an
//!   exponent, so they stay floats), strings, arrays (lists) and objects with string keys (dicts);
//! - `{"$tuple": [...]}`, `{"$set": [...]}`, `{"$bytes": "<base64>"}`,
//!   `{"$float": "inf" | "-inf" | "nan"}`;
//! - `{"$dict": [[key, value], ...]}` for dicts with non-string keys or keys starting with `$`;
//! - `{"$date": "2022-06-08"}`, `{"$time": "12:13:14.000001+01:00"}`,
//!   `{"$datetime": "2022-06-08T12:13:14+01:00"}` (Python's `isoformat`) and
//!   `{"$timedelta": [days, seconds, microseconds]}` (Python's normalised fields);
//! - `{"$uuid": "12345678-1234-5678-1234-567812345678"}` (read in any form `uuid` parses);
//! - `{"$model": {"class", "fields", "fields_set", "extra"}}` for model instances.

use std::fmt::Write as _;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use perldantic_core::{CoreError, CoreResult, Dict, Model, Value, speedate, temporal, uuid};

/// Parse wire JSON into a value.
pub fn decode(json: &str) -> CoreResult<Value> {
    untag(Value::from_json(json)?)
}

fn untag(value: Value) -> CoreResult<Value> {
    Ok(match value {
        Value::List(items) => Value::List(untag_all(items)?),
        Value::Dict(dict) => {
            let tag = match dict.iter().next() {
                Some((Value::Str(key), _)) if dict.len() == 1 => {
                    key.strip_prefix('$').map(str::to_owned)
                }
                _ => None,
            };
            if let Some(tag) = tag {
                let (_, payload) = dict.into_iter().next().expect("one entry");
                return decode_tag(&tag, payload);
            }
            Value::Dict(
                dict.into_iter()
                    .map(|(k, v)| Ok((k, untag(v)?)))
                    .collect::<CoreResult<Dict>>()?,
            )
        }
        other => other,
    })
}

fn untag_all(items: Vec<Value>) -> CoreResult<Vec<Value>> {
    items.into_iter().map(untag).collect()
}

fn invalid(what: &str, payload: &Value) -> CoreError {
    CoreError::Value(format!(
        "Invalid wire value: {what}, got {}",
        payload.repr()
    ))
}

fn decode_tag(tag: &str, payload: Value) -> CoreResult<Value> {
    let list = |payload: Value, what: &str| match payload {
        Value::List(items) => untag_all(items),
        other => Err(invalid(what, &other)),
    };
    Ok(match tag {
        "tuple" => Value::Tuple(list(payload, "$tuple takes a list")?),
        "set" => Value::Set(list(payload, "$set takes a list")?),
        "bytes" => match &payload {
            Value::Str(text) => Value::Bytes(
                STANDARD
                    .decode(text)
                    .map_err(|_| invalid("$bytes takes base64", &payload))?,
            ),
            _ => return Err(invalid("$bytes takes base64", &payload)),
        },
        "float" => Value::Float(match &payload {
            Value::Str(s) if s == "inf" => f64::INFINITY,
            Value::Str(s) if s == "-inf" => f64::NEG_INFINITY,
            Value::Str(s) if s == "nan" => f64::NAN,
            _ => {
                return Err(invalid(
                    "$float takes \"inf\", \"-inf\" or \"nan\"",
                    &payload,
                ));
            }
        }),
        "dict" => {
            let pairs = list(payload, "$dict takes a list of pairs")?;
            Value::Dict(
                pairs
                    .into_iter()
                    .map(|pair| match pair {
                        Value::List(mut kv) if kv.len() == 2 => {
                            let value = kv.pop().expect("two items");
                            let key = kv.pop().expect("two items");
                            Ok((key, value))
                        }
                        other => Err(invalid("$dict takes a list of pairs", &other)),
                    })
                    .collect::<CoreResult<Dict>>()?,
            )
        }
        "model" => decode_model(payload)?,
        "date" => Value::Date(parse_temporal(
            &payload,
            "$date",
            speedate::Date::parse_str,
        )?),
        "time" => Value::Time(parse_temporal(
            &payload,
            "$time",
            speedate::Time::parse_str,
        )?),
        "datetime" => Value::DateTime(parse_temporal(
            &payload,
            "$datetime",
            speedate::DateTime::parse_str,
        )?),
        "timedelta" => decode_timedelta(&payload)?,
        "uuid" => match &payload {
            Value::Str(text) => Value::Uuid(
                uuid::Uuid::parse_str(text)
                    .map_err(|_| invalid("$uuid takes UUID text", &payload))?,
            ),
            _ => return Err(invalid("$uuid takes UUID text", &payload)),
        },
        other => {
            return Err(CoreError::Value(format!(
                "Invalid wire value: unknown tag `${other}`"
            )));
        }
    })
}

fn parse_temporal<T>(
    payload: &Value,
    tag: &str,
    parse: impl Fn(&str) -> Result<T, speedate::ParseError>,
) -> CoreResult<T> {
    match payload {
        Value::Str(text) => {
            parse(text).map_err(|_| invalid(&format!("{tag} takes ISO 8601 text"), payload))
        }
        _ => Err(invalid(&format!("{tag} takes ISO 8601 text"), payload)),
    }
}

fn decode_timedelta(payload: &Value) -> CoreResult<Value> {
    const WHAT: &str = "$timedelta takes [days, seconds, microseconds]";
    let Value::List(parts) = payload else {
        return Err(invalid(WHAT, payload));
    };
    match parts.as_slice() {
        [Value::Int(days), Value::Int(seconds), Value::Int(micros)] => {
            temporal::duration_from_parts(*days, *seconds, *micros)
                .map(Value::TimeDelta)
                .map_err(|_| invalid(WHAT, payload))
        }
        _ => Err(invalid(WHAT, payload)),
    }
}

fn decode_model(payload: Value) -> CoreResult<Value> {
    let Value::Dict(mut model) = payload else {
        return Err(invalid("$model takes an object", &payload));
    };
    let class = match model.remove_str("class") {
        Some(Value::Str(class)) => class,
        other => {
            return Err(invalid(
                "$model needs a `class` string",
                &other.unwrap_or(Value::None),
            ));
        }
    };
    let fields = match model.remove_str("fields").map(untag).transpose()? {
        Some(Value::Dict(fields)) => fields,
        other => {
            return Err(invalid(
                "$model needs `fields`",
                &other.unwrap_or(Value::None),
            ));
        }
    };
    let fields_set = match model.remove_str("fields_set").map(untag).transpose()? {
        Some(Value::List(names) | Value::Set(names)) => names,
        None => fields.iter().map(|(k, _)| k.clone()).collect(),
        Some(other) => return Err(invalid("$model `fields_set` takes a list", &other)),
    };
    let extra = match model.remove_str("extra").map(untag).transpose()? {
        None | Some(Value::None) => None,
        Some(Value::Dict(extra)) => Some(extra),
        Some(other) => return Err(invalid("$model `extra` takes an object or null", &other)),
    };
    Ok(Value::Model(Box::new(Model {
        class,
        fields,
        fields_set,
        extra,
    })))
}

/// Write a value as wire JSON.
pub fn encode(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut out);
    out
}

pub(crate) fn write_str(s: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(s).expect("strings serialize"));
}

fn write_tagged(tag: &str, out: &mut String, payload: impl FnOnce(&mut String)) {
    out.push_str("{\"$");
    out.push_str(tag);
    out.push_str("\":");
    payload(out);
    out.push('}');
}

fn write_items(items: &[Value], out: &mut String) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_value(item, out);
    }
    out.push(']');
}

fn write_dict(dict: &Dict, out: &mut String) {
    let plain = dict
        .iter()
        .all(|(k, _)| matches!(k, Value::Str(s) if !s.starts_with('$')));
    if plain {
        out.push('{');
        for (i, (k, v)) in dict.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_value(k, out);
            out.push(':');
            write_value(v, out);
        }
        out.push('}');
    } else {
        write_tagged("dict", out, |out| {
            out.push('[');
            for (i, (k, v)) in dict.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('[');
                write_value(k, out);
                out.push(',');
                write_value(v, out);
                out.push(']');
            }
            out.push(']');
        });
    }
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::None => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => write!(out, "{i}").expect("writing to a String"),
        Value::BigInt(i) => write!(out, "{i}").expect("writing to a String"),
        Value::Float(f) if f.is_nan() => write_tagged("float", out, |out| out.push_str("\"nan\"")),
        Value::Float(f) if f.is_infinite() => write_tagged("float", out, |out| {
            out.push_str(if *f > 0.0 { "\"inf\"" } else { "\"-inf\"" });
        }),
        // serde_json writes floats with a fraction or an exponent (`1.0`, `1e20`)
        Value::Float(f) => out.push_str(&serde_json::to_string(f).expect("finite floats")),
        Value::Str(s) => write_str(s, out),
        Value::Bytes(b) => write_tagged("bytes", out, |out| write_str(&STANDARD.encode(b), out)),
        Value::List(items) => write_items(items, out),
        Value::Tuple(items) => write_tagged("tuple", out, |out| write_items(items, out)),
        Value::Set(items) => write_tagged("set", out, |out| write_items(items, out)),
        Value::Dict(dict) => write_dict(dict, out),
        Value::Date(d) => write_tagged("date", out, |out| write_str(&temporal::date_str(d), out)),
        Value::Time(t) => write_tagged("time", out, |out| write_str(&temporal::time_str(t), out)),
        Value::DateTime(dt) => write_tagged("datetime", out, |out| {
            let iso = format!(
                "{}T{}",
                temporal::date_str(&dt.date),
                temporal::time_str(&dt.time)
            );
            write_str(&iso, out);
        }),
        Value::TimeDelta(d) => write_tagged("timedelta", out, |out| {
            let (days, seconds, micros) = temporal::timedelta_parts(d);
            write!(out, "[{days},{seconds},{micros}]").expect("writing to a String");
        }),
        Value::Uuid(u) => write_tagged("uuid", out, |out| write_str(&u.to_string(), out)),
        Value::Model(model) => write_tagged("model", out, |out| {
            out.push_str("{\"class\":");
            write_str(&model.class, out);
            out.push_str(",\"fields\":");
            write_dict(&model.fields, out);
            out.push_str(",\"fields_set\":");
            write_items(&model.fields_set, out);
            out.push_str(",\"extra\":");
            match &model.extra {
                Some(extra) => write_dict(extra, out),
                None => out.push_str("null"),
            }
            out.push('}');
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: &Value) -> Value {
        decode(&encode(value)).unwrap()
    }

    #[test]
    fn plain_json_types_are_written_as_is() {
        let value = Value::from_json(r#"{"a": [1, 2.5, "x", true, null], "b": {}}"#).unwrap();
        assert_eq!(encode(&value), r#"{"a":[1,2.5,"x",true,null],"b":{}}"#);
        assert_eq!(encode(&Value::Float(1.0)), "1.0");
        assert_eq!(decode("1.0").unwrap(), Value::Float(1.0));
        let big = Value::from_json("123456789012345678901234567890").unwrap();
        assert_eq!(encode(&big), "123456789012345678901234567890");
        assert_eq!(round_trip(&big), big);
    }

    #[test]
    fn other_values_are_tagged_and_round_trip() {
        let mut odd_keys = Dict::new();
        odd_keys.insert(Value::Int(1), Value::from("one"));
        odd_keys.insert(Value::from("$x"), Value::None);
        let values = [
            Value::Tuple(vec![Value::Int(1), Value::from("a")]),
            Value::Set(vec![Value::Int(1)]),
            Value::Bytes(vec![0, 255, 10]),
            Value::Dict(odd_keys),
            Value::Model(Box::new(Model {
                class: "My::Model".to_owned(),
                fields: [(Value::from("a"), Value::Bytes(b"x".to_vec()))]
                    .into_iter()
                    .collect(),
                fields_set: vec![Value::from("a")],
                extra: Some(Dict::new()),
            })),
        ];
        for value in &values {
            assert_eq!(&round_trip(value), value, "{}", encode(value));
        }
        assert_eq!(
            encode(&Value::Tuple(vec![Value::Bytes(b"hi".to_vec())])),
            r#"{"$tuple":[{"$bytes":"aGk="}]}"#
        );
        assert_eq!(
            encode(&Value::Float(f64::NEG_INFINITY)),
            r#"{"$float":"-inf"}"#
        );
        assert!(matches!(decode(r#"{"$float": "nan"}"#).unwrap(), Value::Float(f) if f.is_nan()));
    }

    #[test]
    fn temporal_values_use_python_forms() {
        for (json, value) in [
            (
                r#"{"$date":"2022-06-08"}"#,
                Value::Date(speedate::Date::parse_str("2022-06-08").unwrap()),
            ),
            (
                r#"{"$time":"12:13:14.000001+01:00"}"#,
                Value::Time(speedate::Time::parse_str("12:13:14.000001+01:00").unwrap()),
            ),
            (
                r#"{"$datetime":"2022-06-08T00:00:00"}"#,
                Value::DateTime(speedate::DateTime::parse_str("2022-06-08T00:00").unwrap()),
            ),
            (
                r#"{"$timedelta":[-1,86399,877000]}"#,
                Value::TimeDelta(temporal::duration_from_parts(0, 0, -123_000).unwrap()),
            ),
        ] {
            assert_eq!(encode(&value), json);
            assert_eq!(decode(json).unwrap(), value, "{json}");
        }
        assert_eq!(
            decode(r#"{"$date": "nope"}"#).unwrap_err().to_string(),
            "Invalid wire value: $date takes ISO 8601 text, got 'nope'"
        );
        assert_eq!(
            decode(r#"{"$timedelta": [1]}"#).unwrap_err().to_string(),
            "Invalid wire value: $timedelta takes [days, seconds, microseconds], got [1]"
        );
    }

    #[test]
    fn uuids_are_hyphenated_text() {
        let text = "12345678-1234-5678-1234-567812345678";
        let value = Value::Uuid(uuid::Uuid::parse_str(text).unwrap());
        assert_eq!(encode(&value), format!(r#"{{"$uuid":"{text}"}}"#));
        assert_eq!(
            decode(r#"{"$uuid": "12345678123456781234567812345678"}"#).unwrap(),
            value
        );
        assert_eq!(
            decode(r#"{"$uuid": "nope"}"#).unwrap_err().to_string(),
            "Invalid wire value: $uuid takes UUID text, got 'nope'"
        );
    }

    #[test]
    fn models_default_their_fields_set_and_extra() {
        let Value::Model(model) =
            decode(r#"{"$model": {"class": "M", "fields": {"a": 1}}}"#).unwrap()
        else {
            panic!("not a model");
        };
        assert_eq!(model.fields_set, vec![Value::from("a")]);
        assert_eq!(model.extra, None);
    }

    #[test]
    fn invalid_wire_values_are_errors() {
        for (json, message) in [
            (r#"{"$nope": 1}"#, "Invalid wire value: unknown tag `$nope`"),
            (
                r#"{"$bytes": "!!"}"#,
                "Invalid wire value: $bytes takes base64, got '!!'",
            ),
            (
                r#"{"$tuple": 1}"#,
                "Invalid wire value: $tuple takes a list, got 1",
            ),
            (
                r#"{"$dict": [[1]]}"#,
                "Invalid wire value: $dict takes a list of pairs, got [1]",
            ),
            (
                r#"{"$model": {"fields": {}}}"#,
                "Invalid wire value: $model needs a `class` string, got None",
            ),
        ] {
            assert_eq!(decode(json).unwrap_err().to_string(), message);
        }
        assert!(
            decode("{")
                .unwrap_err()
                .to_string()
                .starts_with("Invalid JSON")
        );
    }
}
