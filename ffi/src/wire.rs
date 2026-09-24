//! The wire format of values crossing the C ABI: JSON, with tagged objects for what JSON cannot
//! express. It is the conformance value encoding (tests/conformance/README.md), restricted to
//! what `Value` holds:
//!
//! - `null`, booleans, integers of any size (written as `{"$bigint": "<digits>"}` beyond 64
//!   bits, so hosts need not parse big numbers), finite floats (written with a fraction or an
//!   exponent, so they stay floats), strings, arrays (lists) and objects with string keys (dicts);
//! - `{"$tuple": [...]}`, `{"$set": [...]}`, `{"$frozenset": [...]}`, `{"$bytes": "<base64>"}`,
//!   `{"$float": "inf" | "-inf" | "nan"}`;
//! - `{"$dict": [[key, value], ...]}` for dicts with non-string keys or keys starting with `$`;
//! - `{"$date": "2022-06-08"}`, `{"$time": "12:13:14.000001+01:00"}`,
//!   `{"$datetime": "2022-06-08T12:13:14+01:00"}` (Python's `isoformat`) and
//!   `{"$timedelta": [days, seconds, microseconds]}` (Python's normalised fields);
//! - `{"$uuid": "12345678-1234-5678-1234-567812345678"}` (read in any form `uuid` parses);
//! - `{"$decimal": "1.50"}`, Python's `str` of the decimal;
//! - `{"$url": "https://example.com/"}` and `{"$multi_host_url": "redis://h1,h2/0"}`, by their
//!   text (read back keeping an empty path empty, so the text round-trips);
//! - `{"$model": {"class", "fields", "fields_set", "extra"}}` for model instances;
//! - `{"$host": {"id", "class", "isa", "repr"}}` for any other object of the host, which the
//!   core only checks for its class and hands back;
//! - `{"$function": {"id", "name"}}` for a function of the host, called through its callback
//!   ([`crate::host`]); a function the host did not give is written without `id`;
//! - `{"$enum": {"class", "name", "value", "mixin", "str_is_value"}}` for enum members
//!   (`mixin`, the builtin type an `IntEnum` or `StrEnum` member also is, and `str_is_value`
//!   may be left out).

use std::fmt::Write as _;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use perldantic_core::{
    CoreError, CoreResult, Decimal, Dict, EnumMember, EnumMixin, Function, HostObject, Model,
    MultiHostUrl, Url, Value, speedate, temporal, uuid,
};

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
        "frozenset" => Value::FrozenSet(list(payload, "$frozenset takes a list")?),
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
        "enum" => decode_enum(payload)?,
        "function" => decode_function(payload)?,
        "host" => decode_host(payload)?,
        "bigint" => match &payload {
            Value::Str(text) => Value::from(
                text.parse::<num_bigint::BigInt>()
                    .map_err(|_| invalid("$bigint takes integer digits", &payload))?,
            ),
            _ => return Err(invalid("$bigint takes integer digits", &payload)),
        },
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
        "decimal" => match &payload {
            Value::Str(text) => Value::Decimal(Box::new(
                Decimal::parse(text)
                    .ok_or_else(|| invalid("$decimal takes decimal text", &payload))?,
            )),
            _ => return Err(invalid("$decimal takes decimal text", &payload)),
        },
        "url" => Value::Url(Box::new(parse_url(&payload, "$url", Url::parse)?)),
        "multi_host_url" => Value::MultiHostUrl(Box::new(parse_url(
            &payload,
            "$multi_host_url",
            MultiHostUrl::parse,
        )?)),
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

/// URL text as the core writes it; an empty path stays empty so the text round-trips.
fn parse_url<T>(
    payload: &Value,
    tag: &str,
    parse: impl Fn(&str, bool) -> CoreResult<T>,
) -> CoreResult<T> {
    match payload {
        Value::Str(text) => {
            parse(text, true).map_err(|_| invalid(&format!("{tag} takes URL text"), payload))
        }
        _ => Err(invalid(&format!("{tag} takes URL text"), payload)),
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

/// An object of the host: `{"id", "class", "isa", "repr"}` (`isa` and `repr` may be left out).
fn decode_host(payload: Value) -> CoreResult<Value> {
    let Value::Dict(object) = &payload else {
        return Err(invalid("$host takes an object", &payload));
    };
    let (Some(Value::Int(id)), Some(Value::Str(class))) =
        (object.get_str("id"), object.get_str("class"))
    else {
        return Err(invalid("$host needs an `id` and a `class`", &payload));
    };
    let isa = match object.get_str("isa") {
        None => Vec::new(),
        Some(Value::List(classes)) => classes
            .iter()
            .map(|c| match c {
                Value::Str(c) => Ok(c.clone()),
                other => Err(invalid("$host `isa` takes class names", other)),
            })
            .collect::<CoreResult<_>>()?,
        Some(other) => return Err(invalid("$host `isa` takes a list", other)),
    };
    let repr = match object.get_str("repr") {
        Some(Value::Str(repr)) => repr.clone(),
        _ => format!("<{class} object>"),
    };
    Ok(Value::Host(Box::new(HostObject {
        id: id.unsigned_abs(),
        class: class.clone(),
        isa,
        repr,
    })))
}

/// A host function: `{"id", "name"}`, called through the host's callback ([`crate::host`]).
fn decode_function(payload: Value) -> CoreResult<Value> {
    let Value::Dict(function) = &payload else {
        return Err(invalid("$function takes an object", &payload));
    };
    match (function.get_str("id"), function.get_str("name")) {
        (Some(Value::Int(id)), Some(Value::Str(name))) if *id >= 0 => {
            Ok(Value::Function(Function::new(crate::host::FfiFunction {
                id: id.unsigned_abs(),
                name: name.clone(),
            })))
        }
        _ => Err(invalid(
            "$function needs an `id` (a non-negative integer) and a `name`",
            &payload,
        )),
    }
}

fn decode_enum(payload: Value) -> CoreResult<Value> {
    let Value::Dict(mut member) = payload else {
        return Err(invalid("$enum takes an object", &payload));
    };
    let given = member.clone();
    let (Some(Value::Str(class)), Some(Value::Str(name)), Some(value)) = (
        member.remove_str("class"),
        member.remove_str("name"),
        member.remove_str("value"),
    ) else {
        return Err(invalid(
            "$enum needs `class`, `name` and `value`",
            &Value::Dict(given),
        ));
    };
    let mixin = match member.remove_str("mixin") {
        None | Some(Value::None) => None,
        Some(Value::Str(name)) if EnumMixin::from_name(&name).is_some() => {
            EnumMixin::from_name(&name)
        }
        Some(other) => {
            return Err(invalid(
                "$enum `mixin` takes \"int\", \"str\", \"float\" or \"bytes\"",
                &other,
            ));
        }
    };
    let str_is_value = match member.remove_str("str_is_value") {
        None | Some(Value::None) => false,
        Some(Value::Bool(b)) => b,
        Some(other) => return Err(invalid("$enum `str_is_value` takes a boolean", &other)),
    };
    Ok(Value::Enum(Box::new(EnumMember {
        class,
        name,
        value: untag(value)?,
        mixin,
        str_is_value,
    })))
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
        // beyond 64 bits, tagged: hosts need not parse big numbers
        Value::BigInt(i) => write_tagged("bigint", out, |out| write_str(&i.to_string(), out)),
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
        Value::FrozenSet(items) => {
            write_tagged("frozenset", out, |out| write_items(items, out));
        }
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
        Value::Decimal(d) => write_tagged("decimal", out, |out| write_str(&d.to_string(), out)),
        Value::Url(u) => write_tagged("url", out, |out| write_str(&u.as_str(), out)),
        Value::MultiHostUrl(u) => {
            write_tagged("multi_host_url", out, |out| write_str(&u.as_str(), out));
        }
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
        Value::Function(function) => write_tagged("function", out, |out| {
            out.push_str("{\"name\":");
            write_str(function.name(), out);
            if let Some(id) = function.host_id() {
                write!(out, ",\"id\":{id}").expect("writing to a String");
            }
            out.push('}');
        }),
        Value::Host(object) => write_tagged("host", out, |out| {
            write!(out, "{{\"id\":{},\"class\":", object.id).expect("writing to a String");
            write_str(&object.class, out);
            out.push_str(",\"isa\":[");
            for (i, class) in object.isa.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(class, out);
            }
            out.push_str("],\"repr\":");
            write_str(&object.repr, out);
            out.push('}');
        }),
        Value::Enum(member) => write_tagged("enum", out, |out| {
            out.push_str("{\"class\":");
            write_str(&member.class, out);
            out.push_str(",\"name\":");
            write_str(&member.name, out);
            out.push_str(",\"value\":");
            write_value(&member.value, out);
            if let Some(mixin) = member.mixin {
                out.push_str(",\"mixin\":");
                write_str(mixin.name(), out);
            }
            if member.str_is_value {
                out.push_str(",\"str_is_value\":true");
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
        // beyond 64 bits, integers are tagged; plain digits are still read
        assert_eq!(
            encode(&big),
            r#"{"$bigint":"123456789012345678901234567890"}"#
        );
        assert_eq!(round_trip(&big), big);
        assert_eq!(decode("123456789012345678901234567890").unwrap(), big);
        assert_eq!(
            decode(r#"{"$bigint": "12"}"#).unwrap(),
            Value::Int(12),
            "normalised"
        );
        assert_eq!(
            decode(r#"{"$bigint": "x"}"#).unwrap_err().to_string(),
            "Invalid wire value: $bigint takes integer digits, got 'x'"
        );
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

    #[derive(Debug)]
    struct Named;

    impl perldantic_core::HostFunction for Named {
        fn name(&self) -> &'static str {
            "check"
        }

        fn call(
            &self,
            _: perldantic_core::HostCall<'_>,
        ) -> Result<Value, perldantic_core::HostError> {
            Ok(Value::None)
        }
    }

    #[test]
    fn host_objects_travel_by_id() {
        let json = r#"{"$host":{"id":5,"class":"My::Point","isa":["My::Point","My::Base"],"repr":"My::Point=HASH(0x1)"}}"#;
        let value = decode(json).unwrap();
        let Value::Host(object) = &value else {
            panic!("{value:?}")
        };
        assert!(object.is_instance("My::Base") && !object.is_instance("Other"));
        assert_eq!(value.repr(), "My::Point=HASH(0x1)");
        assert_eq!(encode(&value), json);
        assert_eq!(
            decode(r#"{"$host": {"id": 1}}"#).unwrap_err().to_string(),
            "Invalid wire value: $host needs an `id` and a `class`, got {'id': 1}"
        );
    }

    #[test]
    fn host_functions_carry_their_id() {
        let value = Value::Function(perldantic_core::Function::new(Named));
        assert_eq!(encode(&value), r#"{"$function":{"name":"check"}}"#);
        let host = decode(r#"{"$function": {"id": 7, "name": "check"}}"#).unwrap();
        assert_eq!(encode(&host), r#"{"$function":{"name":"check","id":7}}"#);
        assert_eq!(
            decode(r#"{"$function": "check"}"#).unwrap_err().to_string(),
            "Invalid wire value: $function takes an object, got 'check'"
        );
        assert_eq!(
            decode(r#"{"$function": {"id": -1, "name": "check"}}"#)
                .unwrap_err()
                .to_string(),
            "Invalid wire value: $function needs an `id` (a non-negative integer) and a `name`, got {'id': -1, 'name': 'check'}"
        );
    }

    #[test]
    fn enum_members_carry_class_name_and_value() {
        let member = |value: Value, mixin, str_is_value| {
            Value::Enum(Box::new(EnumMember {
                class: "Color".into(),
                name: "RED".into(),
                value,
                mixin,
                str_is_value,
            }))
        };
        let plain = member(Value::Int(1), None, false);
        let json = r#"{"$enum":{"class":"Color","name":"RED","value":1}}"#;
        assert_eq!(encode(&plain), json);
        assert_eq!(decode(json).unwrap(), plain);
        let int_enum = member(Value::Int(1), Some(EnumMixin::Int), true);
        let json = r#"{"$enum":{"class":"Color","name":"RED","value":1,"mixin":"int","str_is_value":true}}"#;
        assert_eq!(encode(&int_enum), json);
        assert_eq!(decode(json).unwrap(), int_enum);
        // values are wire values themselves
        let tuple = member(Value::Tuple(vec![Value::Int(1)]), None, false);
        assert_eq!(decode(&encode(&tuple)).unwrap(), tuple);
        assert_eq!(
            decode(r#"{"$enum": {"class": "Color"}}"#)
                .unwrap_err()
                .to_string(),
            "Invalid wire value: $enum needs `class`, `name` and `value`, got {'class': 'Color'}"
        );
        assert_eq!(
            decode(r#"{"$enum": {"class": "C", "name": "A", "value": 1, "mixin": "list"}}"#)
                .unwrap_err()
                .to_string(),
            "Invalid wire value: $enum `mixin` takes \"int\", \"str\", \"float\" or \"bytes\", got 'list'"
        );
    }

    #[test]
    fn decimals_keep_their_exponent() {
        for json in [
            r#"{"$decimal":"1.50"}"#,
            r#"{"$decimal":"-1E+2"}"#,
            r#"{"$decimal":"-Infinity"}"#,
            r#"{"$decimal":"sNaN12"}"#,
        ] {
            assert_eq!(encode(&decode(json).unwrap()), json, "{json}");
        }
        assert_eq!(
            decode(r#"{"$decimal": "one"}"#).unwrap_err().to_string(),
            "Invalid wire value: $decimal takes decimal text, got 'one'"
        );
    }

    #[test]
    fn urls_are_their_text() {
        for json in [
            r#"{"$url":"https://example.com/"}"#,
            r#"{"$url":"https://example.com"}"#,
            r#"{"$url":"https://example.com?a=1"}"#,
            r#"{"$multi_host_url":"redis://u:p@h1:1,h2:2/0"}"#,
            r#"{"$multi_host_url":"postgres://h1,h2"}"#,
        ] {
            assert_eq!(encode(&decode(json).unwrap()), json, "{json}");
        }
        assert_eq!(
            decode(r#"{"$url": "nope"}"#).unwrap_err().to_string(),
            "Invalid wire value: $url takes URL text, got 'nope'"
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
