//! The binary exports, called the way the Perl host calls them: a binary wire value in, a result
//! buffer out.

use std::ffi::CString;
use std::ptr;

use perldantic_core::{Dict, Value};
use perldantic_ffi::binary;
use perldantic_ffi::{
    PdSerializer, PdValidator, pd_buffer_free, pd_serializer_free, pd_serializer_new,
    pd_serializer_to_data_binary, pd_serializer_to_json_binary, pd_validator_check_binary,
    pd_validator_free, pd_validator_new, pd_validator_validate_binary,
};

fn c(text: &str) -> CString {
    CString::new(text).unwrap()
}

fn validator(schema: &str) -> *mut PdValidator {
    let mut error = ptr::null_mut();
    // SAFETY: valid strings and error pointer.
    let handle = unsafe { pd_validator_new(c(schema).as_ptr(), ptr::null(), &raw mut error) };
    assert!(error.is_null());
    handle
}

fn serializer(schema: &str) -> *mut PdSerializer {
    let mut error = ptr::null_mut();
    // SAFETY: valid strings and error pointer.
    let handle = unsafe { pd_serializer_new(c(schema).as_ptr(), ptr::null(), &raw mut error) };
    assert!(error.is_null());
    handle
}

/// What a result buffer says, once released.
#[derive(Debug, PartialEq)]
enum Outcome {
    Value(Value, Option<String>),
    Envelope(String),
}

fn take(buffer: *mut u8, len: usize) -> Outcome {
    assert!(!buffer.is_null());
    // SAFETY: returned by the library with this length.
    let bytes = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
    // SAFETY: returned by the library and not freed yet.
    unsafe { pd_buffer_free(buffer, len) };
    match bytes[0] {
        b'B' => Outcome::Value(binary::decode(&bytes[1..]).unwrap(), None),
        b'W' => {
            let warning_len = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let warning = String::from_utf8(bytes[5..5 + warning_len].to_vec()).unwrap();
            Outcome::Value(
                binary::decode(&bytes[5 + warning_len..]).unwrap(),
                Some(warning),
            )
        }
        b'J' => Outcome::Envelope(String::from_utf8(bytes[1..].to_vec()).unwrap()),
        other => panic!("unknown result kind {other}"),
    }
}

#[test]
fn validation_reads_and_returns_binary_values() {
    let handle = validator(r#"{"type": "list", "items_schema": {"type": "int"}}"#);
    let mut len = 0;
    let input = binary::encode(&Value::List(vec![Value::from("1"), Value::Int(2)]));
    // SAFETY: live handle, readable input, writable length.
    let buffer = unsafe {
        pd_validator_validate_binary(
            handle,
            input.as_ptr(),
            input.len(),
            ptr::null(),
            &raw mut len,
        )
    };
    assert_eq!(
        take(buffer, len),
        Outcome::Value(Value::List(vec![Value::Int(1), Value::Int(2)]), None)
    );

    let bad = binary::encode(&Value::List(vec![Value::from("x")]));
    // SAFETY: as above.
    let buffer = unsafe {
        pd_validator_validate_binary(handle, bad.as_ptr(), bad.len(), ptr::null(), &raw mut len)
    };
    let Outcome::Envelope(envelope) = take(buffer, len) else {
        panic!("an error envelope")
    };
    assert!(
        envelope.starts_with(r#"{"validation_error":{"title":"list[int]""#),
        "{envelope}"
    );

    // SAFETY: a live handle, freed once.
    unsafe { pd_validator_free(handle) };
}

#[test]
fn malformed_input_is_an_error_envelope() {
    let handle = validator(r#"{"type": "any"}"#);
    let mut len = 0;
    for input in [&[42u8][..], &[5, 9, 0, 0, 0][..]] {
        // SAFETY: live handle, readable input, writable length.
        let buffer = unsafe {
            pd_validator_validate_binary(
                handle,
                input.as_ptr(),
                input.len(),
                ptr::null(),
                &raw mut len,
            )
        };
        let Outcome::Envelope(envelope) = take(buffer, len) else {
            panic!("an error envelope")
        };
        assert!(envelope.contains("Invalid binary wire value"), "{envelope}");
    }
    // SAFETY: a null input pointer is reported, not read.
    let buffer =
        unsafe { pd_validator_validate_binary(handle, ptr::null(), 0, ptr::null(), &raw mut len) };
    assert!(matches!(take(buffer, len), Outcome::Envelope(e) if e.contains("null pointer")));
    // SAFETY: a live handle, freed once.
    unsafe { pd_validator_free(handle) };
}

#[test]
fn serialization_returns_data_json_and_warnings() {
    let handle = serializer(r#"{"type": "list", "items_schema": {"type": "int"}}"#);
    let ints = binary::encode(&Value::List(vec![Value::Int(1)]));
    let to_data = |input: &[u8]| {
        let mut len = 0;
        // SAFETY: live handle, readable input, writable length.
        let buffer = unsafe {
            pd_serializer_to_data_binary(
                handle,
                input.as_ptr(),
                input.len(),
                ptr::null(),
                &raw mut len,
            )
        };
        take(buffer, len)
    };
    let to_json = |input: &[u8]| {
        let mut len = 0;
        // SAFETY: as above.
        let buffer = unsafe {
            pd_serializer_to_json_binary(
                handle,
                input.as_ptr(),
                input.len(),
                ptr::null(),
                &raw mut len,
            )
        };
        take(buffer, len)
    };
    assert_eq!(
        to_data(&ints),
        Outcome::Value(Value::List(vec![Value::Int(1)]), None)
    );
    assert_eq!(to_json(&ints), Outcome::Value(Value::from("[1]"), None));

    let mut keyed = Dict::new();
    keyed.insert(Value::from("a"), Value::from("x"));
    let Outcome::Value(_, Some(warning)) =
        to_data(&binary::encode(&Value::List(vec![Value::Dict(keyed)])))
    else {
        panic!("a warning")
    };
    assert!(warning.contains("Expected `Int`"), "{warning}");

    // SAFETY: a live handle, freed once.
    unsafe { pd_serializer_free(handle) };
}

#[test]
fn null_buffers_are_ignored() {
    // SAFETY: null is ignored.
    unsafe { pd_buffer_free(ptr::null_mut(), 0) };
}

#[test]
fn checking_answers_whether_input_is_valid() {
    let handle = validator(r#"{"type": "list", "items_schema": {"type": "int"}}"#);
    let check = |value: Value| {
        let input = binary::encode(&value);
        let mut len = 0;
        // SAFETY: live handle, readable input, writable length.
        let buffer = unsafe {
            pd_validator_check_binary(
                handle,
                input.as_ptr(),
                input.len(),
                ptr::null(),
                &raw mut len,
            )
        };
        take(buffer, len)
    };
    assert_eq!(
        check(Value::List(vec![Value::from("1")])),
        Outcome::Value(Value::Bool(true), None)
    );
    assert_eq!(
        check(Value::List(vec![Value::from("x")])),
        Outcome::Value(Value::Bool(false), None)
    );
    assert_eq!(check(Value::None), Outcome::Value(Value::Bool(false), None));

    let strict = |value: Value| {
        let input = binary::encode(&value);
        let options = c(r#"{"strict": true}"#);
        let mut len = 0;
        // SAFETY: live handle, readable input and options, writable length.
        let buffer = unsafe {
            pd_validator_check_binary(
                handle,
                input.as_ptr(),
                input.len(),
                options.as_ptr(),
                &raw mut len,
            )
        };
        take(buffer, len)
    };
    assert_eq!(
        strict(Value::List(vec![Value::from("1")])),
        Outcome::Value(Value::Bool(false), None),
        "options apply"
    );

    let bad = [42u8];
    let mut len = 0;
    // SAFETY: as above.
    let buffer = unsafe {
        pd_validator_check_binary(handle, bad.as_ptr(), bad.len(), ptr::null(), &raw mut len)
    };
    assert!(
        matches!(take(buffer, len), Outcome::Envelope(_)),
        "other errors are still errors"
    );
    // SAFETY: a live handle, freed once.
    unsafe { pd_validator_free(handle) };
}
