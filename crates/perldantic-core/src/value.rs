//! The neutral data model that replaces Python objects throughout the core.
//!
//! Validators produce `Value`s and error records carry them as `input`. Host bindings (Perl)
//! convert their native data to and from `Value`.
//!
//! `repr()` and `type_name()` reproduce Python's `repr()` and `type(x).__name__`, because
//! pydantic embeds them in error output (`input_value=..., input_type=...`).

use std::fmt::Write as _;

use crate::core_error::{CoreError, CoreResult};

use jiter::JsonValue;
use num_bigint::BigInt;
use num_traits::FromPrimitive;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

/// A dynamically typed value.
///
/// `BigInt` only holds integers outside the `i64` range; constructors normalise, and equality
/// compares integers by value either way.
#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    BigInt(BigInt),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Dict(Dict),
    /// A Python `set`: items are unique and their order carries no meaning.
    Set(Vec<Value>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::BigInt(a), Self::BigInt(b)) => a == b,
            (Self::Int(a), Self::BigInt(b)) | (Self::BigInt(b), Self::Int(a)) => {
                BigInt::from(*a) == *b
            }
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::List(a), Self::List(b)) | (Self::Tuple(a), Self::Tuple(b)) => a == b,
            (Self::Dict(a), Self::Dict(b)) => a == b,
            (Self::Set(a), Self::Set(b)) => same_items(a, b, |x, y| x == y),
            _ => false,
        }
    }
}

/// Unordered comparison of set items.
fn same_items(a: &[Value], b: &[Value], eq: impl Fn(&Value, &Value) -> bool) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.iter().any(|y| eq(x, y)))
}

impl Value {
    /// Parse JSON text. Duplicate object keys keep their first position and last value,
    /// as in Python.
    pub fn from_json(json: &str) -> CoreResult<Self> {
        JsonValue::parse(json.as_bytes(), false)
            .map(|parsed| Self::from(&parsed))
            .map_err(|e| {
                CoreError::Value(format!("Invalid JSON: {}", e.description(json.as_bytes())))
            })
    }

    /// Python's `==`: numbers compare across `bool`, `int` and `float`, containers compare
    /// item by item. Plain `==` on `Value` keeps variants apart instead.
    pub fn py_eq(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (self.as_py_number(), other.as_py_number()) {
            return a.py_eq(&b);
        }
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::List(a), Self::List(b)) | (Self::Tuple(a), Self::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.py_eq(y))
            }
            (Self::Set(a), Self::Set(b)) => same_items(a, b, Self::py_eq),
            (Self::Dict(a), Self::Dict(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, v)| {
                        b.iter()
                            .find(|(other_k, _)| k.py_eq(other_k))
                            .is_some_and(|(_, other_v)| v.py_eq(other_v))
                    })
            }
            _ => false,
        }
    }

    fn as_py_number(&self) -> Option<PyNumber> {
        match self {
            Self::Bool(b) => Some(PyNumber::Int(BigInt::from(u8::from(*b)))),
            Self::Int(i) => Some(PyNumber::Int(BigInt::from(*i))),
            Self::BigInt(i) => Some(PyNumber::Int(i.clone())),
            Self::Float(f) => Some(PyNumber::Float(*f)),
            _ => None,
        }
    }

    /// Python's `type(value).__name__`.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::None => "NoneType",
            Self::Bool(_) => "bool",
            Self::Int(_) | Self::BigInt(_) => "int",
            Self::Float(_) => "float",
            Self::Str(_) => "str",
            Self::Bytes(_) => "bytes",
            Self::List(_) => "list",
            Self::Tuple(_) => "tuple",
            Self::Dict(_) => "dict",
            Self::Set(_) => "set",
        }
    }

    /// Python's `repr(value)`.
    pub fn repr(&self) -> String {
        let mut out = String::new();
        self.write_repr(&mut out);
        out
    }

    /// Python's `str(value)`: identical to `repr` except for strings, which are unquoted.
    pub fn py_str(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            other => other.repr(),
        }
    }

    fn write_repr(&self, out: &mut String) {
        match self {
            Self::None => out.push_str("None"),
            Self::Bool(true) => out.push_str("True"),
            Self::Bool(false) => out.push_str("False"),
            Self::Int(i) => write!(out, "{i}").unwrap(),
            Self::BigInt(i) => write!(out, "{i}").unwrap(),
            Self::Float(f) => out.push_str(&float_repr(*f)),
            Self::Str(s) => str_repr(s, out),
            Self::Bytes(b) => bytes_repr(b, out),
            Self::List(items) => {
                out.push('[');
                write_items(items, out);
                out.push(']');
            }
            Self::Set(items) if items.is_empty() => out.push_str("set()"),
            Self::Set(items) => {
                out.push('{');
                write_items(items, out);
                out.push('}');
            }
            Self::Tuple(items) => {
                out.push('(');
                write_items(items, out);
                if items.len() == 1 {
                    out.push(',');
                }
                out.push(')');
            }
            Self::Dict(dict) => {
                out.push('{');
                for (i, (k, v)) in dict.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    k.write_repr(out);
                    out.push_str(": ");
                    v.write_repr(out);
                }
                out.push('}');
            }
        }
    }

    /// The JSON object key pydantic produces for this value used as a dict key.
    fn json_key(&self) -> String {
        match self {
            Self::None => "None".to_owned(),
            Self::Bool(b) => b.to_string(),
            Self::Float(f) if !f.is_finite() => "None".to_owned(),
            Self::Str(s) => s.clone(),
            Self::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
            other => other.py_str(),
        }
    }
}

fn write_items(items: &[Value], out: &mut String) {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        item.write_repr(out);
    }
}

/// Python float repr: shortest round-trip digits, positional for exponents in [-4, 16),
/// scientific with a signed two-digit exponent otherwise.
fn float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_owned();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let sci = format!("{f:e}");
    let (mantissa, exp) = sci.split_once('e').expect("`{:e}` always has an exponent");
    let exp: i32 = exp.parse().expect("exponent is an integer");
    if (-4..16).contains(&exp) {
        let mut fixed = format!("{f}");
        if !fixed.contains('.') {
            fixed.push_str(".0");
        }
        fixed
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exp.abs())
    }
}

fn pick_quote(has_single: bool, has_double: bool) -> char {
    if has_single && !has_double { '"' } else { '\'' }
}

fn str_repr(s: &str, out: &mut String) {
    let quote = pick_quote(s.contains('\''), s.contains('"'));
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if is_printable(c) => out.push(c),
            c if u32::from(c) < 0x100 => write!(out, "\\x{:02x}", u32::from(c)).unwrap(),
            c if u32::from(c) < 0x1_0000 => write!(out, "\\u{:04x}", u32::from(c)).unwrap(),
            c => write!(out, "\\U{:08x}", u32::from(c)).unwrap(),
        }
    }
    out.push(quote);
}

fn bytes_repr(bytes: &[u8], out: &mut String) {
    let quote = pick_quote(bytes.contains(&b'\''), bytes.contains(&b'"'));
    out.push('b');
    out.push(quote);
    for &b in bytes {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b if char::from(b) == quote => {
                out.push('\\');
                out.push(char::from(b));
            }
            0x20..=0x7e => out.push(char::from(b)),
            b => write!(out, "\\x{b:02x}").unwrap(),
        }
    }
    out.push(quote);
}

/// Approximation of Python's `str.isprintable()` for one character.
///
/// Python treats categories Cc, Cf, Cs, Co, Cn, Zl, Zp and Zs (except the ASCII space) as
/// non-printable. Without Unicode tables we cover controls, separators, the common format
/// characters and private-use areas; unassigned code points are treated as printable.
fn is_printable(c: char) -> bool {
    let cp = u32::from(c);
    !matches!(
        cp,
        0x00..=0x1f
            | 0x7f..=0xa0
            | 0xad
            | 0x600..=0x605
            | 0x61c
            | 0x6dd
            | 0x70f
            | 0x1680
            | 0x180e
            | 0x2000..=0x200f
            | 0x2028..=0x202f
            | 0x205f..=0x2064
            | 0x2066..=0x206f
            | 0x3000
            | 0xe000..=0xf8ff
            | 0xfeff
            | 0xfff9..=0xfffb
            | 0x110bd
            | 0x1bca0..=0x1bca3
            | 0x1d173..=0x1d17a
            | 0xe0001
            | 0xe0020..=0xe007f
            | 0xf_0000..=0x10_ffff
    )
}

/// A number as Python compares it: integers exactly, floats against integers only when integral.
enum PyNumber {
    Int(BigInt),
    Float(f64),
}

impl PyNumber {
    fn py_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Int(i), Self::Float(f)) | (Self::Float(f), Self::Int(i)) => {
                f.fract() == 0.0 && BigInt::from_f64(*f).is_some_and(|f| f == *i)
            }
        }
    }
}

impl Serialize for Value {
    /// JSON output with pydantic's defaults: bytes as UTF-8 text, inf/nan as `null`,
    /// tuples as arrays, non-string dict keys stringified the way pydantic does.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::None => serializer.serialize_none(),
            Self::Bool(b) => serializer.serialize_bool(*b),
            Self::Int(i) => serializer.serialize_i64(*i),
            Self::BigInt(i) => match i64::try_from(i) {
                Ok(small) => serializer.serialize_i64(small),
                Err(_) => {
                    let number: serde_json::Number =
                        i.to_string().parse().map_err(serde::ser::Error::custom)?;
                    number.serialize(serializer)
                }
            },
            Self::Float(f) if !f.is_finite() => serializer.serialize_none(),
            Self::Float(f) => serializer.serialize_f64(*f),
            Self::Str(s) => serializer.serialize_str(s),
            Self::Bytes(b) => serializer.serialize_str(&String::from_utf8_lossy(b)),
            Self::List(items) | Self::Tuple(items) | Self::Set(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Dict(dict) => {
                let mut map = serializer.serialize_map(Some(dict.len()))?;
                for (k, v) in dict.iter() {
                    map.serialize_entry(&k.json_key(), v)?;
                }
                map.end()
            }
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::Str(s.to_owned())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

impl From<usize> for Value {
    fn from(u: usize) -> Self {
        i64::try_from(u).map_or_else(|_| Self::BigInt(BigInt::from(u)), Self::Int)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<BigInt> for Value {
    /// Integers that fit `i64` become `Int`.
    fn from(i: BigInt) -> Self {
        i64::try_from(&i).map_or(Self::BigInt(i), Self::Int)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(opt: Option<T>) -> Self {
        opt.map_or(Self::None, Into::into)
    }
}

/// An insertion-ordered mapping with arbitrary `Value` keys, like a Python `dict`.
///
/// Equality ignores order, as in Python.
#[derive(Debug, Clone, Default)]
pub struct Dict(Vec<(Value, Value)>);

impl Dict {
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Insert or replace; a replaced key keeps its original position.
    pub fn insert(&mut self, key: Value, value: Value) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key, value)),
        }
    }

    pub fn get(&self, key: &Value) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_str(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(k, _)| matches!(k, Value::Str(s) if s == key))
            .map(|(_, v)| v)
    }

    pub fn remove_str(&mut self, key: &str) -> Option<Value> {
        let index = self
            .0
            .iter()
            .position(|(k, _)| matches!(k, Value::Str(s) if s == key))?;
        Some(self.0.remove(index).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.0.iter().map(|(k, v)| (k, v))
    }
}

impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl FromIterator<(Value, Value)> for Dict {
    fn from_iter<I: IntoIterator<Item = (Value, Value)>>(iter: I) -> Self {
        let mut dict = Self::new();
        for (k, v) in iter {
            dict.insert(k, v);
        }
        dict
    }
}

impl From<&JsonValue<'_>> for Value {
    fn from(json: &JsonValue<'_>) -> Self {
        match json {
            JsonValue::Null => Self::None,
            JsonValue::Bool(b) => Self::Bool(*b),
            JsonValue::Int(i) => Self::Int(*i),
            JsonValue::BigInt(i) => Self::from(i.clone()),
            JsonValue::Float(f) => Self::Float(*f),
            JsonValue::Str(s) => Self::Str(s.to_string()),
            JsonValue::Array(items) => Self::List(items.iter().map(Self::from).collect()),
            JsonValue::Object(entries) => Self::Dict(
                entries
                    .iter()
                    .map(|(k, v)| (Self::Str(k.to_string()), Self::from(v)))
                    .collect(),
            ),
        }
    }
}
