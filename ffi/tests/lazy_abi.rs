//! Lazy results: validated models kept in the core behind handles, read and released by the
//! host (Perl's lazy objects).

use std::ffi::CString;
use std::ptr;
use std::sync::{Mutex, MutexGuard, PoisonError};

use perldantic_core::{Dict, Value};
use perldantic_ffi::binary;
use perldantic_ffi::{
    PdSerializer, PdValidator, pd_buffer_free, pd_lazy_clone, pd_lazy_contents, pd_lazy_live,
    pd_lazy_release, pd_serializer_free, pd_serializer_new, pd_serializer_to_data_binary,
    pd_validator_free, pd_validator_new, pd_validator_validate_json_lazy,
};

const POINTS: &str = r#"{"type": "list", "items_schema": {"type": "model", "cls": "My::Point",
    "schema": {"type": "model-fields", "fields": {
        "x": {"type": "model-field", "schema": {"type": "int"}},
        "y": {"type": "model-field", "schema": {"type": "default", "schema": {"type": "int"},
              "default": 0}}}}}}"#;

/// The tests count live handles, which the others change: they run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

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

/// The bytes of a result buffer, released.
fn take(buffer: *mut u8, len: usize) -> Vec<u8> {
    assert!(!buffer.is_null());
    // SAFETY: returned by the library with this length.
    let bytes = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
    // SAFETY: returned by the library and not freed yet.
    unsafe { pd_buffer_free(buffer, len) };
    bytes
}

fn validate_lazy(validator: *const PdValidator, json: &str) -> Vec<u8> {
    let mut len = 0;
    // SAFETY: live handle, readable JSON, writable length.
    let buffer = unsafe {
        pd_validator_validate_json_lazy(
            validator,
            json.as_ptr(),
            json.len(),
            ptr::null(),
            &raw mut len,
        )
    };
    take(buffer, len)
}

/// The handles of the lazy nodes (tag 12) of a list of models.
fn handles(bytes: &[u8]) -> Vec<u64> {
    assert_eq!(bytes[0], b'B');
    assert_eq!(bytes[1], 6, "a list");
    let count = u32::from_le_bytes(bytes[2..6].try_into().unwrap()) as usize;
    let mut at = 6;
    (0..count)
        .map(|_| {
            assert_eq!(bytes[at], 12, "a lazy node");
            let class_len = u32::from_le_bytes(bytes[at + 1..at + 5].try_into().unwrap()) as usize;
            assert_eq!(&bytes[at + 5..at + 5 + class_len], b"My::Point");
            at += 5 + class_len + 8;
            let handle = u64::from_le_bytes(bytes[at - 8..at].try_into().unwrap());
            assert_eq!(&bytes[at..at + 4], &[0, 0, 0, 0], "no names from the core");
            at += 4;
            handle
        })
        .collect()
}

fn contents(handle: u64) -> Vec<u8> {
    let mut len = 0;
    // SAFETY: writable length; any handle is accepted.
    take(unsafe { pd_lazy_contents(handle, &raw mut len) }, len)
}

#[test]
fn validated_models_stay_in_the_core_until_released() {
    let _serial = serial();
    let validator = validator(POINTS);
    let before = pd_lazy_live();
    let bytes = validate_lazy(validator, r#"[{"x": 1}, {"x": 2, "y": 3}]"#);
    let handles = handles(&bytes);
    assert_eq!(handles.len(), 2);
    assert!(pd_lazy_live() >= before + 2);

    let first = contents(handles[0]);
    assert_eq!(first[0], b'B');
    let Value::Model(model) = binary::decode(&first[1..]).unwrap() else {
        panic!("a model")
    };
    assert_eq!(model.class, "My::Point");
    assert_eq!(model.fields.get_str("x"), Some(&Value::Int(1)));
    assert_eq!(model.fields.get_str("y"), Some(&Value::Int(0)));
    assert_eq!(model.fields_set, vec![Value::from("x")], "y was not given");

    let copy = pd_lazy_clone(handles[1]);
    assert_ne!(copy, 0);
    assert_ne!(copy, handles[1]);
    for handle in &handles {
        pd_lazy_release(*handle);
    }
    let second = contents(copy);
    let Value::Model(model) = binary::decode(&second[1..]).unwrap() else {
        panic!("a model")
    };
    assert_eq!(
        model.fields.get_str("y"),
        Some(&Value::Int(3)),
        "a clone outlives the original"
    );
    pd_lazy_release(copy);

    let gone = contents(handles[0]);
    assert_eq!(gone[0], b'J', "a released handle is an error");
    assert!(String::from_utf8_lossy(&gone).contains("not live"));
    assert_eq!(pd_lazy_clone(handles[0]), 0);
    // SAFETY: live handle.
    unsafe { pd_validator_free(validator) };
}

#[test]
fn lazy_models_serialize_as_they_are() {
    let _serial = serial();
    let validator = validator(POINTS);
    let serializer = serializer(POINTS);
    let bytes = validate_lazy(validator, r#"[{"x": 1}]"#);
    // the host sends the lazy node back as it got it
    let input = &bytes[1..];
    let mut len = 0;
    let options = c(r#"{"exclude_unset": true}"#);
    // SAFETY: live handle, readable input and options, writable length.
    let result = take(
        unsafe {
            pd_serializer_to_data_binary(
                serializer,
                input.as_ptr(),
                input.len(),
                options.as_ptr(),
                &raw mut len,
            )
        },
        len,
    );
    let mut point = Dict::new();
    point.insert(Value::from("x"), Value::Int(1));
    assert_eq!(
        binary::decode(&result[1..]).unwrap(),
        Value::List(vec![Value::Dict(point)]),
        "fields set come from validation"
    );
    for handle in handles(&bytes) {
        pd_lazy_release(handle);
    }
    // SAFETY: live handles.
    unsafe {
        pd_serializer_free(serializer);
        pd_validator_free(validator);
    }
}

#[test]
fn invalid_input_is_a_validation_error() {
    let _serial = serial();
    let validator = validator(POINTS);
    let bytes = validate_lazy(validator, r#"[{"x": "a"}]"#);
    assert_eq!(bytes[0], b'J');
    assert!(String::from_utf8_lossy(&bytes).contains("validation_error"));
    // SAFETY: live handle.
    unsafe { pd_validator_free(validator) };
}
