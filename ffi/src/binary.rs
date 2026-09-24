//! A binary form of the wire format, for the bulk of the data crossing the C ABI: the host
//! writes it straight from its own values and reads the results straight into its own, with no
//! JSON text to write, escape or parse on either side. What it has no node for travels as a
//! JSON node holding the wire JSON of that value ([`crate::wire`]), so every value can be sent.
//!
//! A value is one tag byte followed by its payload; lengths and counts are `u32`, numbers little
//! endian:
//!
//! | tag | value | payload |
//! |---|---|---|
//! | 0 | `None` | |
//! | 1, 2 | `true`, `false` | |
//! | 3 | an integer that fits 64 bits | `i64` |
//! | 4 | a float | `f64` |
//! | 5 | a string | length, UTF-8 bytes |
//! | 6 | a list (tuples, sets and frozensets are written as lists: hosts read them as arrays) | count, values |
//! | 7 | a dict with string keys | count, then per entry: key length, key bytes, value |
//! | 8 | anything else, as wire JSON | length, JSON bytes |
//! | 9 | a model instance that set exactly the fields it holds (string keys), with no extra values | class length, class bytes, count, then entries as in a dict |
//! | 10 | any other model instance (written only) | class length, class bytes, fields as a dict (tag 7 or 8), fields set as a list, extra (tag 0 or a dict) |
//! | 11 | bytes | length, bytes |

use perldantic_core::{CoreError, CoreResult, Dict, Model, Value};

use crate::wire;

const NONE: u8 = 0;
const TRUE: u8 = 1;
const FALSE: u8 = 2;
const INT: u8 = 3;
const FLOAT: u8 = 4;
const STR: u8 = 5;
const LIST: u8 = 6;
const DICT: u8 = 7;
const JSON: u8 = 8;
const MODEL: u8 = 9;
const MODEL_FULL: u8 = 10;
const BYTES: u8 = 11;

/// Nesting deeper than this is refused rather than risking the stack; the host writes deeper
/// data as a JSON node.
const MAX_DEPTH: usize = 512;

/// Read a value from its binary form; the whole input must be one value.
pub fn decode(bytes: &[u8]) -> CoreResult<Value> {
    let mut reader = Reader { bytes, at: 0 };
    let value = reader.value(0)?;
    if reader.at != bytes.len() {
        return Err(malformed("trailing bytes"));
    }
    Ok(value)
}

fn malformed(what: &str) -> CoreError {
    CoreError::Value(format!("Invalid binary wire value: {what}"))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> CoreResult<&'a [u8]> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| malformed("unexpected end"))?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn byte(&mut self) -> CoreResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> CoreResult<usize> {
        let raw: [u8; 4] = self.take(4)?.try_into().expect("four bytes");
        Ok(u32::from_le_bytes(raw) as usize)
    }

    fn eight(&mut self) -> CoreResult<[u8; 8]> {
        Ok(self.take(8)?.try_into().expect("eight bytes"))
    }

    fn str(&mut self) -> CoreResult<&'a str> {
        let len = self.u32()?;
        std::str::from_utf8(self.take(len)?).map_err(|_| malformed("a string is not UTF-8"))
    }

    fn entries(&mut self, depth: usize) -> CoreResult<Dict> {
        let count = self.u32()?;
        let mut dict = Dict::new();
        for _ in 0..count {
            let key = Value::Str(self.str()?.to_owned());
            let value = self.value(depth + 1)?;
            dict.insert(key, value);
        }
        Ok(dict)
    }

    fn value(&mut self, depth: usize) -> CoreResult<Value> {
        if depth > MAX_DEPTH {
            return Err(malformed("nested too deeply"));
        }
        Ok(match self.byte()? {
            NONE => Value::None,
            TRUE => Value::Bool(true),
            FALSE => Value::Bool(false),
            INT => Value::Int(i64::from_le_bytes(self.eight()?)),
            FLOAT => Value::Float(f64::from_le_bytes(self.eight()?)),
            STR => Value::Str(self.str()?.to_owned()),
            BYTES => {
                let len = self.u32()?;
                Value::Bytes(self.take(len)?.to_vec())
            }
            LIST => {
                let count = self.u32()?;
                // a count is only a claim: never reserve more than the input could hold
                let mut items = Vec::with_capacity(count.min(self.bytes.len() - self.at));
                for _ in 0..count {
                    items.push(self.value(depth + 1)?);
                }
                Value::List(items)
            }
            DICT => Value::Dict(self.entries(depth)?),
            JSON => {
                let len = self.u32()?;
                let text = std::str::from_utf8(self.take(len)?)
                    .map_err(|_| malformed("a JSON node is not UTF-8"))?;
                wire::decode(text)?
            }
            MODEL => {
                let class = self.str()?.to_owned();
                let fields = self.entries(depth)?;
                let fields_set = fields.iter().map(|(k, _)| k.clone()).collect();
                Value::Model(Box::new(Model {
                    class,
                    fields,
                    fields_set,
                    extra: None,
                }))
            }
            tag => return Err(malformed(&format!("unknown tag {tag}"))),
        })
    }
}

/// Write a value in its binary form.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_value(value, &mut out);
    out
}

fn write_len(len: usize, out: &mut Vec<u8>) {
    let len = u32::try_from(len).expect("values crossing the ABI are smaller than 4 GiB");
    out.extend_from_slice(&len.to_le_bytes());
}

fn write_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    write_len(bytes.len(), out);
    out.extend_from_slice(bytes);
}

fn write_json(value: &Value, out: &mut Vec<u8>) {
    out.push(JSON);
    write_bytes(wire::encode(value).as_bytes(), out);
}

fn write_items(items: &[Value], out: &mut Vec<u8>) {
    out.push(LIST);
    write_len(items.len(), out);
    for item in items {
        write_value(item, out);
    }
}

/// A dict with string keys as a dict node; any other as wire JSON (which has `$dict` pairs).
fn write_dict(dict: &Dict, out: &mut Vec<u8>) {
    if !dict.iter().all(|(k, _)| matches!(k, Value::Str(_))) {
        write_json(&Value::Dict(dict.clone()), out);
        return;
    }
    out.push(DICT);
    write_len(dict.len(), out);
    for (key, value) in dict.iter() {
        let Value::Str(key) = key else {
            unreachable!("checked above")
        };
        write_bytes(key.as_bytes(), out);
        write_value(value, out);
    }
}

/// Whether a model set exactly the fields it holds, all with string names, and has no extra
/// values: then the short model node says all there is.
fn sets_exactly_its_fields(model: &Model) -> bool {
    model.extra.is_none()
        && model.fields_set.len() == model.fields.len()
        && model.fields.iter().all(|(k, _)| matches!(k, Value::Str(_)))
        && model
            .fields_set
            .iter()
            .all(|name| model.fields.get(name).is_some())
}

fn write_value(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::None => out.push(NONE),
        Value::Bool(true) => out.push(TRUE),
        Value::Bool(false) => out.push(FALSE),
        Value::Int(i) => {
            out.push(INT);
            out.extend_from_slice(&i.to_le_bytes());
        }
        Value::Float(f) => {
            out.push(FLOAT);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Value::Str(s) => {
            out.push(STR);
            write_bytes(s.as_bytes(), out);
        }
        Value::Bytes(b) => {
            out.push(BYTES);
            write_bytes(b, out);
        }
        Value::List(items) | Value::Tuple(items) | Value::Set(items) | Value::FrozenSet(items) => {
            write_items(items, out);
        }
        Value::Dict(dict) => write_dict(dict, out),
        Value::Model(model) if sets_exactly_its_fields(model) => {
            out.push(MODEL);
            write_bytes(model.class.as_bytes(), out);
            write_len(model.fields.len(), out);
            for (key, value) in model.fields.iter() {
                let Value::Str(key) = key else {
                    unreachable!("checked")
                };
                write_bytes(key.as_bytes(), out);
                write_value(value, out);
            }
        }
        Value::Model(model) => {
            out.push(MODEL_FULL);
            write_bytes(model.class.as_bytes(), out);
            write_dict(&model.fields, out);
            write_items(&model.fields_set, out);
            match &model.extra {
                Some(extra) => write_dict(extra, out),
                None => out.push(NONE),
            }
        }
        other => write_json(other, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        out.extend_from_slice(payload);
        out
    }

    fn text(s: &str) -> Vec<u8> {
        let mut out = (s.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(s.as_bytes());
        out
    }

    #[test]
    fn scalars_read_and_write() {
        let cases = [
            (Value::None, vec![NONE]),
            (Value::Bool(true), vec![TRUE]),
            (Value::Bool(false), vec![FALSE]),
            (Value::Int(-2), node(INT, &(-2i64).to_le_bytes())),
            (Value::Float(1.5), node(FLOAT, &1.5f64.to_le_bytes())),
            (Value::from("caf\u{e9}"), node(STR, &text("caf\u{e9}"))),
            (
                Value::Bytes(vec![0, 255]),
                node(BYTES, &[2, 0, 0, 0, 0, 255]),
            ),
        ];
        for (value, bytes) in cases {
            assert_eq!(encode(&value), bytes, "{value:?}");
            assert_eq!(decode(&bytes).unwrap(), value, "{value:?}");
        }
        let nan = decode(&node(FLOAT, &f64::NAN.to_le_bytes())).unwrap();
        assert!(matches!(nan, Value::Float(f) if f.is_nan()));
    }

    #[test]
    fn containers_read_and_write() {
        let mut dict = Dict::new();
        dict.insert(
            Value::from("$a"),
            Value::List(vec![Value::Int(1), Value::None]),
        );
        let value = Value::Dict(dict);
        let mut bytes = vec![DICT, 1, 0, 0, 0];
        bytes.extend(text("$a"));
        bytes.extend([LIST, 2, 0, 0, 0, INT, 1, 0, 0, 0, 0, 0, 0, 0, NONE]);
        assert_eq!(
            encode(&value),
            bytes,
            "keys starting with $ need no tagging"
        );
        assert_eq!(decode(&bytes).unwrap(), value);
    }

    #[test]
    fn sequences_are_written_as_lists() {
        for value in [
            Value::Tuple(vec![Value::Int(1)]),
            Value::Set(vec![Value::Int(1)]),
            Value::FrozenSet(vec![Value::Int(1)]),
        ] {
            assert_eq!(encode(&value), encode(&Value::List(vec![Value::Int(1)])));
        }
    }

    #[test]
    fn other_values_travel_as_wire_json() {
        let big = Value::from(
            "18446744073709551616"
                .parse::<num_bigint::BigInt>()
                .unwrap(),
        );
        let bytes = encode(&big);
        assert_eq!(
            bytes,
            node(JSON, &text(r#"{"$bigint":"18446744073709551616"}"#))
        );
        assert_eq!(decode(&bytes).unwrap(), big);

        let mut keyed = Dict::new();
        keyed.insert(Value::Int(1), Value::from("one"));
        let bytes = encode(&Value::Dict(keyed.clone()));
        assert_eq!(bytes[0], JSON, "dicts with other keys");
        assert_eq!(decode(&bytes).unwrap(), Value::Dict(keyed));

        let tuple = decode(&node(JSON, &text(r#"{"$tuple":[1]}"#))).unwrap();
        assert_eq!(
            tuple,
            Value::Tuple(vec![Value::Int(1)]),
            "hosts send tagged values as JSON"
        );
    }

    #[test]
    fn models_read_and_write_short_when_they_set_exactly_their_fields() {
        let mut bytes = vec![MODEL];
        bytes.extend(text("My::Point"));
        bytes.extend([1, 0, 0, 0]);
        bytes.extend(text("x"));
        bytes.extend(node(INT, &1i64.to_le_bytes()));
        let Value::Model(model) = decode(&bytes).unwrap() else {
            panic!("a model")
        };
        assert_eq!(model.class, "My::Point");
        assert_eq!(model.fields.get_str("x"), Some(&Value::Int(1)));
        assert_eq!(model.fields_set, vec![Value::from("x")]);
        assert_eq!(model.extra, None);

        assert_eq!(
            encode(&Value::Model(model.clone())),
            bytes,
            "written back as it came"
        );

        let mut model = model;
        model.fields_set.clear();
        let written = encode(&Value::Model(model));
        let mut expected = vec![MODEL_FULL];
        expected.extend(text("My::Point"));
        expected.extend([DICT, 1, 0, 0, 0]);
        expected.extend(text("x"));
        expected.extend(node(INT, &1i64.to_le_bytes()));
        expected.extend([LIST, 0, 0, 0, 0, NONE]);
        assert_eq!(written, expected, "in full when a field was not set");
    }

    #[test]
    fn malformed_input_is_an_error() {
        for bytes in [
            vec![],
            vec![INT, 1],
            vec![STR, 9, 0, 0, 0, b'a'],
            vec![STR, 1, 0, 0, 0, 0xff],
            vec![LIST, 255, 255, 255, 255],
            vec![42],
            vec![NONE, NONE],
            vec![MODEL_FULL],
        ] {
            assert!(decode(&bytes).is_err(), "{bytes:?}");
        }
        // unoptimised builds have large frames: give the reader the room a release build needs
        let deep: Vec<u8> = std::iter::repeat_n([LIST, 1, 0, 0, 0], 5000)
            .flatten()
            .collect();
        let result = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || decode(&deep).is_err())
            .unwrap()
            .join()
            .unwrap();
        assert!(result, "too deep");
    }
}
