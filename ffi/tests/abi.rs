//! The C ABI, called the way a C host calls it: NUL-terminated JSON in, result envelopes out.

use std::ffi::{CStr, CString, c_char};
use std::ptr;

use perldantic_ffi::{
    PdSerializer, PdValidator, pd_json_schema, pd_serializer_free, pd_serializer_new,
    pd_serializer_to_json, pd_serializer_to_python, pd_string_free, pd_url_parts,
    pd_validator_free, pd_validator_new, pd_validator_validate, pd_validator_validate_json,
    pd_version,
};

fn c(text: &str) -> CString {
    CString::new(text).unwrap()
}

/// Take ownership of a returned string and release it.
fn take(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null());
    // SAFETY: returned by the library, NUL-terminated.
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
    // SAFETY: returned by the library and not freed yet.
    unsafe { pd_string_free(ptr) };
    text
}

fn validator(schema: &str) -> *mut PdValidator {
    let mut error = ptr::null_mut();
    // SAFETY: valid strings and error pointer.
    let handle = unsafe { pd_validator_new(c(schema).as_ptr(), ptr::null(), &raw mut error) };
    assert!(error.is_null(), "{}", take(error));
    handle
}

fn validate(handle: *const PdValidator, input: &str, options: Option<&str>) -> String {
    let options = options.map(c);
    // SAFETY: live handle and valid strings.
    take(unsafe {
        pd_validator_validate(
            handle,
            c(input).as_ptr(),
            options.as_ref().map_or(ptr::null(), |o| o.as_ptr()),
        )
    })
}

fn validate_json(handle: *const PdValidator, json: &[u8]) -> String {
    // SAFETY: live handle and `json.len()` readable bytes.
    take(unsafe { pd_validator_validate_json(handle, json.as_ptr(), json.len(), ptr::null()) })
}

fn serializer(schema: &str, config: Option<&str>) -> *mut PdSerializer {
    let config = config.map(c);
    let mut error = ptr::null_mut();
    // SAFETY: valid strings and error pointer.
    let handle = unsafe {
        pd_serializer_new(
            c(schema).as_ptr(),
            config.as_ref().map_or(ptr::null(), |o| o.as_ptr()),
            &raw mut error,
        )
    };
    assert!(error.is_null(), "{}", take(error));
    handle
}

#[test]
fn pd_version_returns_core_version() {
    // SAFETY: pd_version returns a pointer to a static NUL-terminated string.
    let version = unsafe { CStr::from_ptr(pd_version()) };
    assert_eq!(version.to_str().unwrap(), perldantic_core::VERSION);
    assert_eq!(pd_version(), pd_version());
}

#[test]
fn validation_returns_wire_values() {
    let handle =
        validator(r#"{"type": "tuple", "items_schema": [{"type": "int"}, {"type": "bytes"}]}"#);
    assert_eq!(
        validate(handle, r#"["1", {"$bytes": "aGk="}]"#, None),
        r#"{"ok":{"$tuple":[1,{"$bytes":"aGk="}]}}"#
    );
    assert_eq!(
        validate(
            handle,
            r#"{"$tuple": ["1", "x"]}"#,
            Some(r#"{"strict": true}"#)
        ),
        r#"{"validation_error":{"title":"tuple[int, bytes]","message":"2 validation errors for tuple[int, bytes]\n0\n  Input should be a valid integer [type=int_type, input_value='1', input_type=str]\n    For further information visit https://errors.pydantic.dev/latest/v/int_type\n1\n  Input should be a valid bytes [type=bytes_type, input_value='x', input_type=str]\n    For further information visit https://errors.pydantic.dev/latest/v/bytes_type","errors":[{"type":"int_type","loc":[0],"msg":"Input should be a valid integer","input":"1","url":"https://errors.pydantic.dev/latest/v/int_type"},{"type":"bytes_type","loc":[1],"msg":"Input should be a valid bytes","input":"x","url":"https://errors.pydantic.dev/latest/v/bytes_type"}]}}"#
    );
    // SAFETY: returned by pd_validator_new.
    unsafe { pd_validator_free(handle) };
}

#[test]
fn host_data_is_perl_input_unless_told_otherwise() {
    let list = validator(r#"{"type": "list"}"#);
    assert!(
        validate(list, "1", None).contains(r#""msg":"Input should be an array reference""#),
        "Perl words by default"
    );
    assert!(
        validate(list, "1", Some(r#"{"input_type": "python"}"#))
            .contains(r#""msg":"Input should be a valid list""#),
        "pydantic's words on request"
    );
    assert_eq!(
        validate(list, "1", Some(r#"{"input_type": "yaml"}"#)),
        r#"{"error":{"type":"ValueError","message":"invalid value for 'input_type': 'yaml'"}}"#
    );
    let pair = validator(r#"{"type": "tuple", "items_schema": [{"type": "int"}]}"#);
    assert_eq!(
        validate(pair, "[1]", Some(r#"{"strict": true}"#)),
        r#"{"ok":{"$tuple":[1]}}"#,
        "Perl arrays are strict tuples"
    );
    // SAFETY: returned by pd_validator_new.
    unsafe {
        pd_validator_free(list);
        pd_validator_free(pair);
    }
}

#[test]
fn validation_errors_carry_context() {
    let handle = validator(r#"{"type": "int", "gt": 3}"#);
    let envelope = validate(handle, "2", None);
    assert!(envelope.contains(r#""ctx":{"gt":3}"#), "{envelope}");
    // SAFETY: returned by pd_validator_new.
    unsafe { pd_validator_free(handle) };
}

#[test]
fn json_validation_takes_raw_bytes() {
    let handle = validator(
        r#"{"type": "model", "cls": "My::Point", "schema": {"type": "model-fields",
            "fields": {"x": {"type": "model-field", "schema": {"type": "int"}}}}}"#,
    );
    assert_eq!(
        validate_json(handle, br#"{"x": 1}"#),
        r#"{"ok":{"$model":{"class":"My::Point","fields":{"x":1},"fields_set":["x"],"extra":null}}}"#
    );
    assert!(validate_json(handle, b"{").contains(r#""type":"json_invalid""#));
    assert_eq!(
        validate_json(handle, b"\xff"),
        r#"{"error":{"type":"UnicodeDecodeError","message":"'utf-8' codec can't decode byte 0xff in position 0: invalid utf-8"}}"#
    );
    // SAFETY: returned by pd_validator_new.
    unsafe { pd_validator_free(handle) };
}

#[test]
fn bad_arguments_are_error_envelopes() {
    let handle = validator(r#"{"type": "int"}"#);
    assert_eq!(
        validate(handle, "1", Some(r#"{"stric": true}"#)),
        r#"{"error":{"type":"TypeError","message":"got an unexpected keyword argument 'stric'"}}"#
    );
    assert_eq!(
        validate(handle, r#"{"$nope": 1}"#, None),
        r#"{"error":{"type":"ValueError","message":"Invalid wire value: unknown tag `$nope`"}}"#
    );
    // SAFETY: null handles and pointers are reported, not dereferenced.
    let envelope = take(unsafe { pd_validator_validate(ptr::null(), ptr::null(), ptr::null()) });
    assert_eq!(
        envelope,
        r#"{"error":{"type":"InternalError","message":"`validator` is a null pointer"}}"#
    );
    // SAFETY: as above.
    let envelope = take(unsafe { pd_validator_validate(handle, ptr::null(), ptr::null()) });
    assert!(envelope.contains("`input` is a null pointer"));
    // SAFETY: returned by pd_validator_new; freeing null is a no-op.
    unsafe {
        pd_validator_free(handle);
        pd_validator_free(ptr::null_mut());
        pd_string_free(ptr::null_mut());
    }
}

#[test]
fn invalid_schemas_fail_to_construct() {
    let mut error = ptr::null_mut();
    // SAFETY: valid strings and error pointer.
    let handle = unsafe {
        pd_validator_new(
            c(r#"{"type": "bogus"}"#).as_ptr(),
            ptr::null(),
            &raw mut error,
        )
    };
    assert!(handle.is_null());
    assert_eq!(
        take(error),
        r#"{"error":{"type":"SchemaError","message":"Unknown schema type: \"bogus\""}}"#
    );
    // SAFETY: a null error pointer is allowed.
    let handle = unsafe { pd_serializer_new(c("[").as_ptr(), ptr::null(), ptr::null_mut()) };
    assert!(handle.is_null());
}

#[test]
fn serialization_returns_values_json_and_warnings() {
    let handle = serializer(
        r#"{"type": "list", "items_schema": {"type": "bytes"}}"#,
        Some(r#"{"ser_json_bytes": "base64"}"#),
    );
    let value = c(r#"[{"$bytes": "+/8="}, 1]"#);
    // SAFETY: live handle and valid strings.
    let python = take(unsafe {
        pd_serializer_to_python(handle, value.as_ptr(), c(r#"{"mode": "json"}"#).as_ptr())
    });
    assert_eq!(
        python,
        r#"{"ok":["-_8=",1],"warning":"Pydantic serializer warnings:\n  PydanticSerializationUnexpectedValue(Expected `bytes` - serialized value may not be as expected [input_value=1, input_type=int])"}"#
    );
    // SAFETY: live handle and valid strings.
    let json = take(unsafe {
        pd_serializer_to_json(
            handle,
            value.as_ptr(),
            c(r#"{"warnings": false, "indent": 1}"#).as_ptr(),
        )
    });
    assert_eq!(json, r#"{"ok":"[\n \"-_8=\",\n 1\n]","warning":null}"#);
    // SAFETY: live handle and valid strings.
    let error = take(unsafe {
        pd_serializer_to_python(
            handle,
            value.as_ptr(),
            c(r#"{"warnings": "error"}"#).as_ptr(),
        )
    });
    assert!(
        error.starts_with(r#"{"error":{"type":"PydanticSerializationError","#),
        "{error}"
    );
    // SAFETY: returned by pd_serializer_new.
    unsafe { pd_serializer_free(handle) };
}

#[test]
fn json_schemas_come_with_warnings() {
    let schema = c(
        r#"{"type": "model", "cls": "M", "schema": {"type": "model-fields", "fields": {
            "b": {"type": "model-field", "schema": {"type": "default", "schema": {"type": "bytes"},
                  "default": {"$bytes": "+w=="}}}}}}"#,
    );
    // SAFETY: valid strings.
    let envelope = take(unsafe {
        pd_json_schema(
            schema.as_ptr(),
            ptr::null(),
            c(r#"{"mode": "serialization"}"#).as_ptr(),
        )
    });
    assert_eq!(
        envelope,
        r#"{"ok":{"properties":{"b":{"format":"binary","title":"B","type":"string"}},"title":"M","type":"object"},"warnings":["Default value b'\\xfb' is not JSON serializable; excluding default from JSON schema [non-serializable-default]"]}"#
    );
    // SAFETY: valid strings.
    let envelope = take(unsafe {
        pd_json_schema(
            c(r#"{"type": "decimal"}"#).as_ptr(),
            ptr::null(),
            ptr::null(),
        )
    });
    assert_eq!(
        envelope,
        r#"{"error":{"type":"SchemaError","message":"JSON Schema generation for `decimal` schemas is not supported yet"}}"#
    );
}

#[test]
fn url_parts_are_pydantics_accessors() {
    let parts = |wire: &str| take(unsafe { pd_url_parts(c(wire).as_ptr()) });
    assert_eq!(
        parts(r#"{"$url": "https://u:p@xn--mnchen-3ya.de/a?x=1&y=2#f"}"#),
        concat!(
            r#"{"ok":{"scheme":"https","username":"u","password":"p","host":"xn--mnchen-3ya.de","#,
            r#""unicode_host":"münchen.de","port":443,"path":"/a","query":"x=1&y=2","#,
            r#""query_params":[["x","1"],["y","2"]],"fragment":"f","#,
            r#""unicode_string":"https://münchen.dea.de/a?x=1&y=2#f"}}"#
        )
    );
    assert_eq!(
        parts(r#"{"$multi_host_url": "redis://h1:1,h2"}"#),
        concat!(
            r#"{"ok":{"scheme":"redis","hosts":[{"username":null,"password":null,"host":"h1","port":1},"#,
            r#"{"username":null,"password":null,"host":"h2","port":null}],"path":null,"query":null,"#,
            r#""query_params":[],"fragment":null,"unicode_string":"redis://h1:1,h2"}}"#
        )
    );
    assert_eq!(
        parts("1"),
        r#"{"error":{"type":"TypeError","message":"expected a $url or $multi_host_url value, got 1"}}"#
    );
}
