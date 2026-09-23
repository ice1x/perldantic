//! Coercion rules of the `Input` implementations.
//!
//! JSON rules follow upstream `input_json.rs`; `Value` rules follow upstream `input_python.rs`
//! (host-native data); `str` rules follow upstream's `Input for str` (JSON object keys).

use jiter::JsonValue;
use num_bigint::BigInt;

use super::{
    BorrowInput, ConsumeIterator, EitherInt, Input, ValidatedDict, ValidatedList, ValidatedTuple,
};
use crate::errors::{ValError, ValResult};
use crate::validators::config::{BytesMode, ValBytesMode};
use crate::validators::validation_state::Exactness;
use crate::{Dict, LocItem, Value};

fn json(s: &str) -> JsonValue<'static> {
    JsonValue::parse(s.as_bytes(), false).unwrap().into_static()
}

fn s(v: &str) -> Value {
    Value::from(v)
}

/// The error type code of a single-error result.
fn err_type<T>(result: ValResult<T>) -> String {
    match result {
        Err(ValError::LineErrors(lines)) => {
            assert_eq!(lines.len(), 1);
            lines[0].error_type.type_string()
        }
        Err(other) => panic!("expected a line error, got {other:?}"),
        Ok(_) => panic!("expected an error"),
    }
}

fn str_ok(input: &(impl Input + ?Sized), strict: bool, coerce: bool) -> (String, Exactness) {
    let m = input.validate_str(strict, coerce).unwrap();
    let e = m.exactness();
    (m.into_inner().as_cow().into_owned(), e)
}

fn int_ok(input: &(impl Input + ?Sized), strict: bool) -> (EitherInt, Exactness) {
    let m = input.validate_int(strict).unwrap();
    let e = m.exactness();
    (m.into_inner(), e)
}

fn i(v: i64) -> EitherInt {
    EitherInt::I64(v)
}

// ---- str -------------------------------------------------------------------------------------

#[test]
fn json_str() {
    assert_eq!(
        str_ok(&json(r#""x""#), true, false),
        ("x".into(), Exactness::Strict)
    );
    assert_eq!(
        err_type(json("5").validate_str(false, false)),
        "string_type"
    );
    assert_eq!(
        str_ok(&json("5"), false, true),
        ("5".into(), Exactness::Lax)
    );
    assert_eq!(
        str_ok(&json("1.0"), false, true),
        ("1".into(), Exactness::Lax)
    );
    assert_eq!(
        str_ok(&json("1180591620717411303424"), false, true).0,
        "1180591620717411303424"
    );
    assert_eq!(err_type(json("5").validate_str(true, true)), "string_type");
    assert_eq!(
        err_type(json("true").validate_str(false, true)),
        "string_type"
    );
    assert_eq!(json(r#""x""#).exact_str().unwrap().as_cow(), "x");
    assert_eq!(err_type(json("1").exact_str()), "string_type");
}

#[test]
fn value_str() {
    assert_eq!(str_ok(&s("x"), true, false), ("x".into(), Exactness::Exact));
    assert_eq!(
        str_ok(&Value::Bytes(b"ab".to_vec()), false, false),
        ("ab".into(), Exactness::Lax)
    );
    assert_eq!(
        err_type(Value::Bytes(b"ab".to_vec()).validate_str(true, false)),
        "string_type"
    );
    assert_eq!(
        err_type(Value::Bytes(vec![0xff]).validate_str(false, false)),
        "string_unicode"
    );
    assert_eq!(
        str_ok(&Value::Int(5), false, true),
        ("5".into(), Exactness::Lax)
    );
    // Python str(1.0) keeps the fractional part.
    assert_eq!(str_ok(&Value::Float(1.0), false, true).0, "1.0");
    assert_eq!(
        err_type(Value::Bool(true).validate_str(false, true)),
        "string_type"
    );
    assert_eq!(
        err_type(Value::Int(5).validate_str(false, false)),
        "string_type"
    );
    assert_eq!(s("x").exact_str().unwrap().as_cow(), "x");
}

// ---- bool ------------------------------------------------------------------------------------

#[test]
fn json_bool() {
    let ok = |j: &str, strict: bool| {
        let m = json(j).validate_bool(strict).unwrap();
        (m.exactness(), m.into_inner())
    };
    assert_eq!(ok("true", true), (Exactness::Exact, true));
    assert_eq!(ok(r#""yes""#, false), (Exactness::Lax, true));
    assert_eq!(ok(r#""OFF""#, false), (Exactness::Lax, false));
    assert_eq!(ok("1", false), (Exactness::Lax, true));
    assert_eq!(ok("0.0", false), (Exactness::Lax, false));
    assert_eq!(err_type(json(r#""yes""#).validate_bool(true)), "bool_type");
    assert_eq!(err_type(json("2").validate_bool(false)), "bool_parsing");
    assert_eq!(err_type(json("2.0").validate_bool(false)), "bool_parsing");
    assert_eq!(err_type(json("0.5").validate_bool(false)), "bool_type");
    assert_eq!(
        err_type(json(r#""maybe""#).validate_bool(false)),
        "bool_parsing"
    );
    assert_eq!(err_type(json("null").validate_bool(false)), "bool_type");
}

#[test]
fn value_bool() {
    let ok = |v: Value| {
        let m = v.validate_bool(false).unwrap();
        (m.exactness(), m.into_inner())
    };
    assert_eq!(ok(Value::Bool(false)), (Exactness::Exact, false));
    assert_eq!(ok(s("on")), (Exactness::Lax, true));
    assert_eq!(ok(Value::Bytes(b"n".to_vec())), (Exactness::Lax, false));
    assert_eq!(ok(Value::Int(1)), (Exactness::Lax, true));
    assert_eq!(ok(Value::Float(1.0)), (Exactness::Lax, true));
    assert_eq!(
        err_type(Value::Bytes(vec![0xff]).validate_bool(false)),
        "bool_parsing"
    );
    assert_eq!(err_type(Value::Int(5).validate_bool(false)), "bool_parsing");
    assert_eq!(
        err_type(Value::BigInt(BigInt::from(2).pow(70)).validate_bool(false)),
        "bool_type"
    );
    assert_eq!(
        err_type(Value::Float(1.5).validate_bool(false)),
        "bool_type"
    );
    assert_eq!(err_type(s("on").validate_bool(true)), "bool_type");
    assert_eq!(
        err_type(Value::List(vec![]).validate_bool(false)),
        "bool_type"
    );
}

// ---- int -------------------------------------------------------------------------------------

#[test]
fn json_int() {
    assert_eq!(int_ok(&json("42"), true), (i(42), Exactness::Exact));
    assert_eq!(
        int_ok(&json("1180591620717411303424"), true),
        (EitherInt::BigInt(BigInt::from(2).pow(70)), Exactness::Exact)
    );
    assert_eq!(int_ok(&json("true"), false), (i(1), Exactness::Lax));
    assert_eq!(err_type(json("true").validate_int(true)), "int_type");
    assert_eq!(int_ok(&json("3.0"), false), (i(3), Exactness::Lax));
    assert_eq!(err_type(json("3.5").validate_int(false)), "int_from_float");
    assert_eq!(
        int_ok(&json(r#"" 1_000 ""#), false),
        (i(1000), Exactness::Lax)
    );
    assert_eq!(err_type(json(r#""12""#).validate_int(true)), "int_type");
    assert_eq!(err_type(json("null").validate_int(false)), "int_type");
    assert_eq!(err_type(json("[1]").validate_int(false)), "int_type");
}

#[test]
fn value_int() {
    assert_eq!(int_ok(&Value::Int(7), true), (i(7), Exactness::Exact));
    assert_eq!(int_ok(&Value::Bool(true), false), (i(1), Exactness::Lax));
    assert_eq!(err_type(Value::Bool(true).validate_int(true)), "int_type");
    assert_eq!(int_ok(&s("0012"), false), (i(12), Exactness::Lax));
    assert_eq!(
        int_ok(&Value::Bytes(b"7".to_vec()), false),
        (i(7), Exactness::Lax)
    );
    assert_eq!(
        err_type(Value::Bytes(vec![0xff]).validate_int(false)),
        "int_parsing"
    );
    assert_eq!(int_ok(&Value::Float(-2.0), false), (i(-2), Exactness::Lax));
    assert_eq!(
        err_type(Value::Float(f64::NAN).validate_int(false)),
        "finite_number"
    );
    assert_eq!(
        err_type(Value::Float(1e20).validate_int(false)),
        "int_parsing_size"
    );
    assert_eq!(
        err_type(Value::List(vec![]).validate_int(false)),
        "int_type"
    );
    assert_eq!(
        err_type(s(&"9".repeat(5000)).validate_int(false)),
        "int_parsing_size"
    );
    assert_eq!(Value::Int(3).exact_int().unwrap(), i(3));
    assert_eq!(err_type(Value::Bool(true).exact_int()), "int_type");
}

/// String cases from upstream tests/validators/test_int.py (`test_int_py_and_json`).
#[test]
fn str_as_int_matches_upstream_table() {
    let ok = [
        ("0", 0),
        ("00", 0),
        ("000", 0),
        ("0_000", 0),
        ("+0", 0),
        ("+00", 0),
        ("+000", 0),
        ("+0_000", 0),
        ("  1  ", 1),
        ("-1", -1),
        ("-1.0", -1),
        ("42", 42),
        ("0.0", 0),
        ("00.0", 0),
        ("00.00", 0),
        ("42.0", 42),
        ("42.00", 42),
        ("042", 42),
        ("01", 1),
        ("09", 9),
        ("+4_2", 42),
        ("+0_42", 42),
        ("+4_2.0", 42),
        ("+04_2.0", 42),
        ("-00001", -1),
        ("-00042_000", -42000),
        ("4_2", 42),
        ("0_42", 42),
        ("4_2.0", 42),
        ("04_2.0", 42),
        ("  04_2.0 ", 42),
        ("  0_42.0 ", 42),
        ("000001", 1),
        ("123456789.0", 123_456_789),
    ];
    for (input, expected) in ok {
        assert_eq!(int_ok(input, false).0, i(expected), "input {input:?}");
        assert_eq!(int_ok(&s(input), false).0, i(expected), "input {input:?}");
    }
    let bad = [
        "00_",
        "0:",
        "++4_2",
        "-+1",
        "+-1",
        "--0001",
        "-+0001",
        "-0-001",
        "-0+001",
        "  _042.0 ",
        "42_",
        "42_.0",
        " ",
        "1.",
        "42.",
        "123456789123456.00001",
    ];
    for input in bad {
        assert_eq!(
            err_type(input.validate_int(false)),
            "int_parsing",
            "input {input:?}"
        );
    }
}

// ---- float -----------------------------------------------------------------------------------

#[test]
fn json_float() {
    let ok = |j: &str, strict: bool| {
        let m = json(j).validate_float(strict).unwrap();
        (m.exactness(), m.into_inner().as_f64())
    };
    assert_eq!(ok("1.5", true), (Exactness::Exact, 1.5));
    assert_eq!(ok("3", true), (Exactness::Strict, 3.0));
    assert_eq!(ok("true", false), (Exactness::Lax, 1.0));
    assert_eq!(ok(r#"" 2.5 ""#, false), (Exactness::Lax, 2.5));
    assert_eq!(ok(r#""1_0.5""#, false), (Exactness::Lax, 10.5));
    assert_eq!(err_type(json("true").validate_float(true)), "float_type");
    assert_eq!(
        err_type(json(r#""x""#).validate_float(false)),
        "float_parsing"
    );
    assert_eq!(err_type(json("null").validate_float(false)), "float_type");
}

#[test]
fn value_float() {
    let ok = |v: Value, strict: bool| {
        let m = v.validate_float(strict).unwrap();
        (m.exactness(), m.into_inner().as_f64())
    };
    assert_eq!(ok(Value::Float(0.5), true), (Exactness::Exact, 0.5));
    assert_eq!(ok(Value::Int(2), true), (Exactness::Strict, 2.0));
    assert_eq!(
        ok(Value::BigInt(BigInt::from(2).pow(70)), true),
        (Exactness::Strict, 2f64.powi(70))
    );
    assert_eq!(ok(Value::Bool(false), false), (Exactness::Lax, 0.0));
    assert_eq!(ok(s("inf"), false).1, f64::INFINITY);
    assert_eq!(
        ok(Value::Bytes(b"2".to_vec()), false),
        (Exactness::Lax, 2.0)
    );
    assert_eq!(
        err_type(Value::Bool(true).validate_float(true)),
        "float_type"
    );
    assert_eq!(err_type(s("1.5").validate_float(true)), "float_type");
    assert_eq!(
        err_type(Value::Bytes(vec![0xff]).validate_float(false)),
        "float_parsing"
    );
    assert_eq!(err_type(Value::None.validate_float(false)), "float_type");
}

// ---- bytes -----------------------------------------------------------------------------------

fn json_bytes(j: &str, mode: ValBytesMode) -> Vec<u8> {
    json(j)
        .validate_bytes(false, mode)
        .unwrap()
        .into_inner()
        .as_slice()
        .to_vec()
}

#[test]
fn bytes_modes() {
    let utf8 = ValBytesMode::default();
    let b64 = ValBytesMode {
        ser: BytesMode::Base64,
    };
    let hex = ValBytesMode {
        ser: BytesMode::Hex,
    };

    assert_eq!(json_bytes(r#""ab""#, utf8), b"ab");
    assert_eq!(json_bytes(r#""YWI=""#, b64), b"ab");
    // Standard alphabet and missing padding are accepted too.
    assert_eq!(json_bytes(r#""+/8""#, b64), [0xfb, 0xff]);
    assert_eq!(json_bytes(r#""6162""#, hex), b"ab");
    assert_eq!(
        err_type(json("1").validate_bytes(false, utf8)),
        "bytes_type"
    );
    let Err(ValError::LineErrors(lines)) = json(r#""6""#).validate_bytes(false, hex) else {
        panic!("expected an encoding error")
    };
    assert_eq!(
        lines[0]
            .error_type
            .render_message(crate::InputType::Json)
            .unwrap(),
        "Data should be valid hex: Odd number of digits"
    );

    let raw = Value::Bytes(b"x".to_vec());
    let m = raw.validate_bytes(true, utf8).unwrap();
    assert_eq!(m.exactness(), Exactness::Exact);
    let text = s("x");
    let m = text.validate_bytes(false, utf8).unwrap();
    assert_eq!(m.exactness(), Exactness::Lax);
    assert_eq!(m.into_inner().as_slice(), b"x");
    assert_eq!(err_type(s("x").validate_bytes(true, utf8)), "bytes_type");
    assert_eq!(
        err_type(Value::Int(1).validate_bytes(false, utf8)),
        "bytes_type"
    );
}

// ---- collections -----------------------------------------------------------------------------

struct Collect;

impl<T: BorrowInput> ConsumeIterator<T> for Collect {
    type Output = Vec<Value>;
    fn consume_iterator(self, iterator: impl Iterator<Item = T>) -> Vec<Value> {
        iterator
            .map(|item| item.borrow_input().as_error_value())
            .collect()
    }
}

struct CollectPairs;

impl<K: BorrowInput + Into<LocItem>, V: BorrowInput> ConsumeIterator<ValResult<(K, V)>>
    for CollectPairs
{
    type Output = Vec<(LocItem, Value)>;
    fn consume_iterator(self, iterator: impl Iterator<Item = ValResult<(K, V)>>) -> Self::Output {
        iterator
            .map(|pair| {
                let (k, v) = pair.unwrap();
                let value = v.borrow_input().as_error_value();
                (k.into(), value)
            })
            .collect()
    }
}

#[test]
fn json_dict() {
    let input = json(r#"{"a": 1, "b": [true]}"#);
    let dict = input.validate_dict(false).unwrap();
    assert_eq!(
        dict.iterate(CollectPairs).unwrap(),
        vec![
            (LocItem::from("a"), Value::Int(1)),
            (LocItem::from("b"), Value::List(vec![Value::Bool(true)]))
        ]
    );
    assert_eq!(dict.last_key(), Some("b"));
    assert_eq!(err_type(json("[1]").validate_dict(false)), "dict_type");
    assert_eq!(err_type(json("1").validate_dict(true)), "dict_type");
}

#[test]
fn value_dict() {
    let input = Value::Dict(Dict::from_iter([
        (s("x"), Value::Int(1)),
        (Value::Int(2), Value::None),
    ]));
    let dict = input.validate_dict(true).unwrap();
    assert_eq!(
        dict.iterate(CollectPairs).unwrap(),
        vec![
            (LocItem::from("x"), Value::Int(1)),
            (LocItem::from(2_i64), Value::None)
        ]
    );
    assert_eq!(dict.last_key(), Some(&Value::Int(2)));
    assert_eq!(
        err_type(Value::List(vec![]).validate_dict(false)),
        "dict_type"
    );
}

#[test]
fn json_list_and_tuple() {
    let input = json("[1, 2]");
    let list = input.validate_list(true).unwrap();
    assert_eq!(list.exactness(), Exactness::Exact);
    let list = list.into_inner();
    assert_eq!(ValidatedList::len(&list), Some(2));
    assert_eq!(
        ValidatedList::iterate(list, Collect).unwrap(),
        vec![Value::Int(1), Value::Int(2)]
    );
    assert_eq!(err_type(json("{}").validate_list(false)), "list_type");

    let tuple = input.validate_tuple(true).unwrap();
    assert_eq!(tuple.exactness(), Exactness::Strict);
    let mut seen = Vec::new();
    ValidatedTuple::try_for_each(tuple.into_inner(), |item| {
        seen.push(item?.borrow_input().as_error_value());
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, vec![Value::Int(1), Value::Int(2)]);
    assert_eq!(
        err_type(json(r#""ab""#).validate_tuple(false)),
        "tuple_type"
    );
}

#[test]
fn value_list_and_tuple() {
    let list = Value::List(vec![Value::Int(1)]);
    let tuple = Value::Tuple(vec![Value::Int(2)]);

    let m = list.validate_list(true).unwrap();
    assert_eq!(m.exactness(), Exactness::Exact);
    assert_eq!(
        ValidatedList::iterate(m.into_inner(), Collect).unwrap(),
        vec![Value::Int(1)]
    );
    let m = tuple.validate_list(false).unwrap();
    assert_eq!(m.exactness(), Exactness::Lax);
    assert_eq!(
        ValidatedList::iterate(m.into_inner(), Collect).unwrap(),
        vec![Value::Int(2)]
    );
    assert_eq!(err_type(tuple.validate_list(true)), "list_type");

    let m = tuple.validate_tuple(true).unwrap();
    assert_eq!(m.exactness(), Exactness::Exact);
    assert_eq!(ValidatedTuple::len(&m.into_inner()), Some(1));
    let m = list.validate_tuple(false).unwrap();
    assert_eq!(m.exactness(), Exactness::Lax);
    assert_eq!(err_type(list.validate_tuple(true)), "tuple_type");

    // Strings, bytes and dicts are never treated as sequences.
    for not_seq in [
        s("ab"),
        Value::Bytes(b"ab".to_vec()),
        Value::Dict(Dict::new()),
        Value::Int(1),
    ] {
        assert_eq!(err_type(not_seq.validate_list(false)), "list_type");
        assert_eq!(err_type(not_seq.validate_tuple(false)), "tuple_type");
    }
}

// ---- misc ------------------------------------------------------------------------------------

#[test]
fn none_and_error_values() {
    assert!(json("null").is_none());
    assert!(!json("0").is_none());
    assert!(Value::None.is_none());
    assert!(!Value::Int(0).is_none());
    assert_eq!(
        json(r#"{"a": [1.5, null]}"#).as_error_value(),
        Value::from_json(r#"{"a": [1.5, null]}"#).unwrap()
    );
    assert_eq!("k".as_error_value(), s("k"));
}

#[test]
fn str_input_is_lax_string_data() {
    assert_eq!(str_ok("x", true, false), ("x".into(), Exactness::Strict));
    let m = "yes".validate_bool(true).unwrap();
    assert_eq!((m.exactness(), m.into_inner()), (Exactness::Lax, true));
    assert_eq!(int_ok("12", true), (i(12), Exactness::Lax));
    assert_eq!(
        "2.5".validate_float(true).unwrap().into_inner().as_f64(),
        2.5
    );
    assert_eq!(err_type("x".validate_dict(false)), "dict_type");
    assert_eq!(err_type("x".validate_list(false)), "list_type");
    assert_eq!(err_type("x".validate_tuple(false)), "tuple_type");
}

#[test]
fn loc_items_from_dict_keys() {
    assert_eq!(LocItem::from(&s("k")), LocItem::from("k"));
    assert_eq!(LocItem::from(&Value::Int(3)), LocItem::from(3_i64));
    assert_eq!(LocItem::from(&Value::Bool(true)), LocItem::from(1_i64));
    assert_eq!(LocItem::from(&Value::Float(1.5)), LocItem::from("1.5"));
}

#[test]
fn only_host_data_exposes_itself_as_a_value() {
    let v = Value::Int(1);
    assert_eq!(v.as_value(), Some(&Value::Int(1)));
    assert_eq!(json("1").as_value(), None);
    assert_eq!("1".as_value(), None);
}

#[test]
fn sets_are_lax_lists_and_tuples() {
    let set = Value::Set(vec![Value::Int(1), Value::Int(2)]);
    assert_eq!(
        set.validate_list(false).unwrap().exactness(),
        Exactness::Lax
    );
    assert_eq!(err_type(set.validate_list(true)), "list_type");
    assert_eq!(
        set.validate_tuple(false).unwrap().exactness(),
        Exactness::Lax
    );
    assert_eq!(err_type(set.validate_tuple(true)), "tuple_type");
    assert_eq!(err_type(set.validate_dict(false)), "dict_type");
}

#[test]
fn only_json_exposes_itself_as_json() {
    assert!(json("1").as_json().is_some());
    assert!(Value::Int(1).as_json().is_none());
    assert!("1".as_json().is_none());
}
