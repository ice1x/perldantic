//! `Value` is the neutral data model that replaces Python objects in the core.
//!
//! `repr()` and `type_name()` must match Python's `repr()` and `type(x).__name__` exactly,
//! because pydantic embeds them in error messages (`input_value=..., input_type=...`).
//! Expected strings below were produced by CPython 3.12.

use num_bigint::BigInt;
use perldantic_core::{Dict, Model, Value};

fn s(v: &str) -> Value {
    Value::Str(v.to_owned())
}

fn dict(items: Vec<(Value, Value)>) -> Value {
    Value::Dict(items.into_iter().collect())
}

#[test]
fn repr_scalars() {
    let cases: Vec<(Value, &str, &str)> = vec![
        (Value::None, "None", "NoneType"),
        (Value::Bool(true), "True", "bool"),
        (Value::Bool(false), "False", "bool"),
        (Value::Int(0), "0", "int"),
        (Value::Int(-7), "-7", "int"),
        (
            Value::BigInt(BigInt::from(2).pow(70)),
            "1180591620717411303424",
            "int",
        ),
    ];
    for (value, repr, type_name) in cases {
        assert_eq!(value.repr(), repr, "repr of {value:?}");
        assert_eq!(value.type_name(), type_name, "type of {value:?}");
    }
}

#[test]
fn repr_floats_follow_python_shortest_round_trip() {
    let cases = [
        (1.0, "1.0"),
        (-0.0, "-0.0"),
        (1.5, "1.5"),
        (1e16, "1e+16"),
        (1e15, "1000000000000000.0"),
        (0.0001, "0.0001"),
        (0.00001, "1e-05"),
        (1e-7, "1e-07"),
        (1.23e100, "1.23e+100"),
        (123_456_789.123, "123456789.123"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
        (f64::NAN, "nan"),
    ];
    for (f, repr) in cases {
        assert_eq!(Value::Float(f).repr(), repr, "repr of {f:e}");
        assert_eq!(Value::Float(f).type_name(), "float");
    }
}

#[test]
fn repr_strings_pick_quotes_and_escape_like_python() {
    let cases = [
        ("", "''"),
        ("abc", "'abc'"),
        ("it's", "\"it's\""),
        ("say \"hi\"", "'say \"hi\"'"),
        ("both ' and \"", "'both \\' and \"'"),
        ("tab\tnl\nret\rback\\", "'tab\\tnl\\nret\\rback\\\\'"),
        ("\x00\x1f\x7f", "'\\x00\\x1f\\x7f'"),
        ("\u{a0}", "'\\xa0'"),
        ("caf\u{e9}", "'caf\u{e9}'"),
        ("\u{2028}", "'\\u2028'"),
        ("\u{200b}", "'\\u200b'"),
        ("\u{feff}", "'\\ufeff'"),
        ("emoji \u{1F600}", "'emoji \u{1F600}'"),
    ];
    for (input, repr) in cases {
        assert_eq!(s(input).repr(), repr, "repr of {input:?}");
        assert_eq!(s(input).type_name(), "str");
    }
}

#[test]
fn repr_bytes() {
    let cases: [(&[u8], &str); 4] = [
        (b"", "b''"),
        (b"abc", "b'abc'"),
        (b"it's", "b\"it's\""),
        (b"\x00\xff\t", "b'\\x00\\xff\\t'"),
    ];
    for (input, repr) in cases {
        assert_eq!(Value::Bytes(input.to_vec()).repr(), repr);
        assert_eq!(Value::Bytes(input.to_vec()).type_name(), "bytes");
    }
}

#[test]
fn repr_containers() {
    assert_eq!(Value::List(vec![]).repr(), "[]");
    assert_eq!(Value::List(vec![Value::Int(1), s("a")]).repr(), "[1, 'a']");
    assert_eq!(Value::Tuple(vec![]).repr(), "()");
    assert_eq!(Value::Tuple(vec![Value::Int(1)]).repr(), "(1,)");
    assert_eq!(
        Value::Tuple(vec![Value::Int(1), Value::Int(2)]).repr(),
        "(1, 2)"
    );
    assert_eq!(dict(vec![]).repr(), "{}");
    assert_eq!(
        dict(vec![
            (s("a"), Value::Int(1)),
            (s("b"), Value::List(vec![Value::None]))
        ])
        .repr(),
        "{'a': 1, 'b': [None]}"
    );
    assert_eq!(dict(vec![(Value::Int(1), s("x"))]).repr(), "{1: 'x'}");
    assert_eq!(Value::List(vec![]).type_name(), "list");
    assert_eq!(Value::Tuple(vec![]).type_name(), "tuple");
    assert_eq!(dict(vec![]).type_name(), "dict");
}

#[test]
fn str_differs_from_repr_only_for_strings() {
    assert_eq!(s("it's").py_str(), "it's");
    assert_eq!(Value::Int(42).py_str(), "42");
    assert_eq!(Value::Float(1.0).py_str(), "1.0");
    assert_eq!(Value::List(vec![s("a")]).py_str(), "['a']");
    assert_eq!(Value::Bytes(b"x".to_vec()).py_str(), "b'x'");
}

#[test]
fn json_serialization_matches_pydantic_defaults() {
    let value = dict(vec![
        (s("none"), Value::None),
        (s("bool"), Value::Bool(true)),
        (s("int"), Value::Int(-3)),
        (s("big"), Value::BigInt(BigInt::from(2).pow(70))),
        (s("float"), Value::Float(1.5)),
        (s("inf"), Value::Float(f64::INFINITY)),
        (s("str"), s("x\"y")),
        (s("bytes"), Value::Bytes(b"raw".to_vec())),
        (s("list"), Value::List(vec![Value::Int(1)])),
        (s("tuple"), Value::Tuple(vec![Value::Int(2)])),
        (
            s("keys"),
            dict(vec![
                (Value::Int(1), Value::Bool(false)),
                (Value::None, Value::Int(0)),
            ]),
        ),
    ]);
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        concat!(
            r#"{"none":null,"bool":true,"int":-3,"big":1180591620717411303424,"float":1.5,"#,
            r#""inf":null,"str":"x\"y","bytes":"raw","list":[1],"tuple":[2],"#,
            r#""keys":{"1":false,"None":0}}"#
        )
    );
}

#[test]
fn dict_preserves_insertion_order_and_looks_up_string_keys() {
    let mut d = Dict::new();
    d.insert(s("b"), Value::Int(1));
    d.insert(s("a"), Value::Int(2));
    d.insert(s("b"), Value::Int(3));
    assert_eq!(d.len(), 2);
    assert_eq!(d.get_str("b"), Some(&Value::Int(3)));
    assert_eq!(d.get_str("missing"), None);
    let keys: Vec<String> = d.iter().map(|(k, _)| k.py_str()).collect();
    assert_eq!(keys, ["b", "a"]);
}

#[test]
fn conversions_from_rust_types() {
    assert_eq!(Value::from("a"), s("a"));
    assert_eq!(Value::from(String::from("a")), s("a"));
    assert_eq!(Value::from(5_i64), Value::Int(5));
    assert_eq!(Value::from(5_usize), Value::Int(5));
    assert_eq!(Value::from(true), Value::Bool(true));
    assert_eq!(Value::from(0.5), Value::Float(0.5));
    assert_eq!(Value::from(None::<i64>), Value::None);
    assert_eq!(Value::from(Some(3_i64)), Value::Int(3));
}

#[test]
fn big_ints_that_fit_i64_are_canonicalised() {
    // Python has one int type; `BigInt` is only used beyond the i64 range.
    assert_eq!(
        Value::from_json("9223372036854775807").unwrap(),
        Value::Int(i64::MAX)
    );
    assert!(matches!(
        Value::from_json("-9223372036854775808").unwrap(),
        Value::Int(i64::MIN)
    ));
    assert!(matches!(Value::from(BigInt::from(7)), Value::Int(7)));
    assert!(matches!(
        Value::from(BigInt::from(2).pow(70)),
        Value::BigInt(_)
    ));
}

#[test]
fn int_and_big_int_compare_by_value() {
    assert_eq!(Value::Int(5), Value::BigInt(BigInt::from(5)));
    assert_ne!(Value::Int(5), Value::BigInt(BigInt::from(6)));
    assert_ne!(Value::Int(1), Value::Float(1.0));
}

#[test]
fn python_equality_crosses_numeric_types() {
    // Python: True == 1 == 1.0, and hash-equal keys collide in dicts and literals.
    assert!(Value::Bool(true).py_eq(&Value::Int(1)));
    assert!(Value::Int(1).py_eq(&Value::Float(1.0)));
    assert!(Value::Bool(false).py_eq(&Value::Float(0.0)));
    assert!(Value::from(BigInt::from(2).pow(70)).py_eq(&Value::Float(2f64.powi(70))));
    assert!(!Value::Int(1).py_eq(&Value::Float(1.5)));
    assert!(!Value::Float(f64::NAN).py_eq(&Value::Float(f64::NAN)));
    assert!(!Value::Int(1).py_eq(&s("1")));
    assert!(!Value::None.py_eq(&Value::Bool(false)));
    assert!(!s("a").py_eq(&Value::Bytes(b"a".to_vec())));
}

#[test]
fn python_equality_of_containers() {
    let list = Value::List(vec![Value::Int(1), Value::Bool(true)]);
    assert!(list.py_eq(&Value::List(vec![Value::Float(1.0), Value::Int(1)])));
    // list and tuple are never equal, even with equal items.
    assert!(!list.py_eq(&Value::Tuple(vec![Value::Int(1), Value::Int(1)])));
    assert!(dict(vec![(Value::Int(1), s("x"))]).py_eq(&dict(vec![(Value::Bool(true), s("x"))])));
    assert!(!dict(vec![(s("a"), Value::Int(1))]).py_eq(&dict(vec![(s("a"), Value::Int(2))])));
}

#[test]
fn sets_are_unordered_and_print_like_python() {
    let set = Value::Set(vec![s("a"), s("b")]);
    assert_eq!(set.type_name(), "set");
    assert_eq!(set.repr(), "{'a', 'b'}");
    assert_eq!(Value::Set(vec![]).repr(), "set()");
    assert_eq!(set, Value::Set(vec![s("b"), s("a")]));
    assert_ne!(set, Value::Set(vec![s("a")]));
    assert!(set.py_eq(&Value::Set(vec![s("b"), s("a")])));
    assert!(Value::Set(vec![Value::Int(1)]).py_eq(&Value::Set(vec![Value::Float(1.0)])));
    assert!(!set.py_eq(&Value::List(vec![s("a"), s("b")])));
    // JSON has no sets: they serialize as arrays, like pydantic's to_json.
    assert_eq!(serde_json::to_string(&set).unwrap(), r#"["a","b"]"#);
}

fn model(class: &str, fields: Value, fields_set: &[&str], extra: Option<Value>) -> Value {
    let Value::Dict(fields) = fields else {
        panic!()
    };
    let extra = extra.map(|e| match e {
        Value::Dict(d) => d,
        _ => panic!(),
    });
    Value::Model(Box::new(Model {
        class: class.into(),
        fields,
        fields_set: fields_set.iter().map(|f| s(f)).collect(),
        extra,
    }))
}

#[test]
fn models_print_like_pydantic_models() {
    let m = model(
        "User",
        dict(vec![(s("a"), Value::Int(1)), (s("b"), s("x"))]),
        &["a"],
        None,
    );
    assert_eq!(m.type_name(), "User");
    assert_eq!(m.repr(), "User(a=1, b='x')");
    // Serialized like model_dump: fields, then extra.
    let with_extra = model(
        "User",
        dict(vec![(s("a"), Value::Int(1))]),
        &["a", "z"],
        Some(dict(vec![(s("z"), Value::Int(2))])),
    );
    assert_eq!(with_extra.repr(), "User(a=1, z=2)");
    assert_eq!(
        serde_json::to_string(&with_extra).unwrap(),
        r#"{"a":1,"z":2}"#
    );
}

#[test]
fn model_equality() {
    let a = model("User", dict(vec![(s("a"), Value::Int(1))]), &["a"], None);
    assert_eq!(
        a,
        model("User", dict(vec![(s("a"), Value::Int(1))]), &["a"], None)
    );
    assert_ne!(
        a,
        model("Other", dict(vec![(s("a"), Value::Int(1))]), &["a"], None)
    );
    assert_ne!(
        a,
        model("User", dict(vec![(s("a"), Value::Int(1))]), &[], None)
    );
    // Like BaseModel.__eq__: same class, fields and extra; fields_set is ignored.
    assert!(a.py_eq(&model(
        "User",
        dict(vec![(s("a"), Value::Float(1.0))]),
        &[],
        None
    )));
    assert!(!a.py_eq(&dict(vec![(s("a"), Value::Int(1))])));
}

#[test]
fn dicts_convert_iterate_and_compare_like_python() {
    let mut dict: Dict = [
        (Value::from("a"), Value::Int(1)),
        (Value::from("b"), Value::Float(2.0)),
    ]
    .into_iter()
    .collect();
    for (_, value) in dict.iter_mut() {
        if let Value::Int(i) = value {
            *i += 1;
        }
    }
    let same: Dict = [
        (Value::from("b"), Value::Int(2)),
        (Value::from("a"), Value::Float(2.0)),
    ]
    .into_iter()
    .collect();
    assert!(dict.py_eq(&same));
    assert_ne!(dict, same);
    let pairs: Vec<(Value, Value)> = dict.clone().into_iter().collect();
    assert_eq!(pairs[0], (Value::from("a"), Value::Int(2)));
    assert_eq!(Value::from(dict.clone()), Value::Dict(dict));
}
