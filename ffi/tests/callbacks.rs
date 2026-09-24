//! Host functions through the C ABI, with a callback written the way a host writes it: it reads
//! the call JSON, answers with `pd_host_reply`, and calls wrap handlers back.

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;

use perldantic_ffi::host::{
    PdReply, pd_host_reply, pd_serializer_handler_call, pd_set_host_callback,
    pd_validator_handler_call,
};
use perldantic_ffi::{
    PdSerializer, PdValidator, pd_serializer_free, pd_serializer_new, pd_serializer_to_json,
    pd_string_free, pd_validator_free, pd_validator_new, pd_validator_validate,
};
use serde_json::{Value as Json, json};

fn c(text: &str) -> CString {
    CString::new(text).unwrap()
}

fn take(ptr: *mut c_char) -> Json {
    assert!(!ptr.is_null());
    // SAFETY: returned by the library, NUL-terminated.
    let text = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
    // SAFETY: returned by the library and not freed yet.
    unsafe { pd_string_free(ptr) };
    serde_json::from_str(&text).unwrap()
}

fn reply(slot: *mut PdReply, answer: &Json) {
    let text = c(&answer.to_string());
    // SAFETY: the slot of the current call and a NUL-terminated string.
    unsafe { pd_host_reply(slot, text.as_ptr()) };
}

fn handler_of(call: &Json) -> *mut c_void {
    call["handler"].as_u64().unwrap() as usize as *mut c_void
}

/// The host: what each function id does.
unsafe extern "C" fn host(id: u64, call: *const c_char, slot: *mut PdReply) {
    // SAFETY: the library passes a NUL-terminated string.
    let call: Json =
        serde_json::from_str(unsafe { CStr::from_ptr(call) }.to_str().unwrap()).unwrap();
    let answer = match id {
        // double
        1 => json!({"ok": call["input"].as_i64().unwrap() * 2}),
        // raise ValueError
        2 => json!({"error": {"kind": "value", "message": "boom"}}),
        // raise a host exception of its own
        3 => json!({"error": {"kind": "other", "message": "Died: kaput", "id": 99}}),
        // wrap: fall back to -1 when the handler fails
        4 => {
            let input = c(&call["input"].to_string());
            // SAFETY: the handler of this call, which is still running.
            let result = take(unsafe {
                pd_validator_handler_call(handler_of(&call), input.as_ptr(), ptr::null())
            });
            match result.get("ok") {
                Some(ok) => json!({"ok": ok}),
                None => json!({"ok": -1}),
            }
        }
        // wrap: locate errors and re-raise them
        5 => {
            let input = c(&call["input"].to_string());
            let location = c("\"here\"");
            // SAFETY: as above.
            let result = take(unsafe {
                pd_validator_handler_call(handler_of(&call), input.as_ptr(), location.as_ptr())
            });
            match result.get("validation_error") {
                Some(error) => json!({"error": {
                    "kind": "validation",
                    "title": error["title"],
                    "errors": error["errors"],
                }}),
                None => json!({"ok": result["ok"]}),
            }
        }
        // serializer: times ten
        6 => json!({"ok": call["value"].as_i64().unwrap() * 10}),
        // wrap serializer: handler + 1
        7 => {
            let value = c(&call["value"].to_string());
            // SAFETY: as above.
            let result = take(unsafe {
                pd_serializer_handler_call(handler_of(&call), value.as_ptr(), ptr::null())
            });
            json!({"ok": result["ok"].as_i64().unwrap() + 1})
        }
        // a function that forgets to reply
        8 => return,
        // echo the info argument
        9 => json!({"ok": call["info"]}),
        // a computed field: the model's name, upper-cased
        10 => {
            assert_eq!(call["call"], "property");
            let name = call["model"]["$model"]["fields"][call["name"].as_str().unwrap()]
                .as_str()
                .unwrap()
                .to_uppercase();
            json!({"ok": name})
        }
        other => panic!("unknown function {other}"),
    };
    reply(slot, &answer);
}

fn register() {
    pd_set_host_callback(Some(host));
}

fn function(id: u64, name: &str) -> Json {
    json!({"$function": {"id": id, "name": name}})
}

fn validator(schema: &Json) -> *mut PdValidator {
    let mut error = ptr::null_mut();
    // SAFETY: valid strings and error pointer.
    let handle =
        unsafe { pd_validator_new(c(&schema.to_string()).as_ptr(), ptr::null(), &raw mut error) };
    assert!(error.is_null(), "{}", take(error));
    handle
}

fn validate(handle: *const PdValidator, input: &Json) -> Json {
    // SAFETY: live handle and valid strings.
    take(unsafe { pd_validator_validate(handle, c(&input.to_string()).as_ptr(), ptr::null()) })
}

fn function_schema(kind: &str, id: u64, name: &str, info: bool) -> Json {
    let mut schema = json!({
        "type": kind,
        "function": {"type": if info { "with-info" } else { "no-info" }, "function": function(id, name)},
    });
    if kind != "function-plain" {
        schema["schema"] = json!({"type": "int"});
    }
    schema
}

#[test]
fn validator_functions_run_in_the_host() {
    register();
    let before = validator(&function_schema("function-before", 1, "double", false));
    assert_eq!(validate(before, &json!(21)), json!({"ok": 42}));

    let after = validator(&function_schema("function-after", 2, "boom", false));
    let result = validate(after, &json!("3"));
    let error = &result["validation_error"]["errors"][0];
    assert_eq!(error["type"], "value_error");
    assert_eq!(error["msg"], "Value error, boom");
    assert_eq!(error["input"], "3");

    // other exceptions come back with the host's id, to be raised again
    let plain = validator(&function_schema("function-plain", 3, "die", false));
    assert_eq!(
        validate(plain, &json!(1)),
        json!({"error": {"type": "HostException", "message": "Died: kaput", "id": 99}})
    );

    let silent = validator(&function_schema("function-plain", 8, "silent", false));
    assert_eq!(
        validate(silent, &json!(1))["error"]["message"],
        "host function `silent` did not reply"
    );
    for handle in [before, after, plain, silent] {
        // SAFETY: handles from pd_validator_new, freed once.
        unsafe { pd_validator_free(handle) };
    }
}

#[test]
fn wrap_handlers_are_called_back() {
    register();
    let fallback = validator(&function_schema("function-wrap", 4, "fallback", false));
    assert_eq!(validate(fallback, &json!("5")), json!({"ok": 5}));
    assert_eq!(validate(fallback, &json!("x")), json!({"ok": -1}));

    let located = validator(&function_schema("function-wrap", 5, "located", false));
    let result = validate(located, &json!("x"));
    let error = &result["validation_error"]["errors"][0];
    assert_eq!(error["type"], "int_parsing");
    assert_eq!(error["loc"], json!(["here"]));
    assert_eq!(error["input"], "x");
    for handle in [fallback, located] {
        // SAFETY: handles from pd_validator_new, freed once.
        unsafe { pd_validator_free(handle) };
    }
}

#[test]
fn info_is_passed_as_json() {
    register();
    let schema = json!({
        "type": "typed-dict",
        "fields": {
            "a": {"type": "typed-dict-field", "schema": {"type": "int"}},
            "b": {"type": "typed-dict-field", "schema": function_schema("function-plain", 9, "echo", true)},
        },
    });
    let v = validator(&schema);
    let options = c(r#"{"context": {"c": 1}}"#);
    // SAFETY: live handle and valid strings.
    let result = take(unsafe {
        pd_validator_validate(v, c(r#"{"a": 1, "b": 2}"#).as_ptr(), options.as_ptr())
    });
    assert_eq!(
        result["ok"]["b"],
        json!({"mode": "perl", "config": null, "context": {"c": 1}, "data": {"a": 1}, "field_name": "b"})
    );
    // SAFETY: handle from pd_validator_new.
    unsafe { pd_validator_free(v) };
}

fn serializer(schema: &Json) -> *mut PdSerializer {
    let mut error = ptr::null_mut();
    // SAFETY: valid strings and error pointer.
    let handle =
        unsafe { pd_serializer_new(c(&schema.to_string()).as_ptr(), ptr::null(), &raw mut error) };
    assert!(error.is_null(), "{}", take(error));
    handle
}

#[test]
fn serializer_functions_run_in_the_host() {
    register();
    let plain = serializer(&json!({
        "type": "int",
        "serialization": {"type": "function-plain", "function": function(6, "times_ten")},
    }));
    let wrap = serializer(&json!({
        "type": "int",
        "serialization": {"type": "function-wrap", "function": function(7, "plus_one")},
    }));
    for (handle, expected) in [(plain, "10"), (wrap, "2")] {
        // SAFETY: live handle and valid strings.
        let result = take(unsafe { pd_serializer_to_json(handle, c("1").as_ptr(), ptr::null()) });
        assert_eq!(result["ok"], expected);
        // SAFETY: handle from pd_serializer_new, freed once.
        unsafe { pd_serializer_free(handle) };
    }
}

#[test]
fn computed_fields_ask_the_host() {
    register();
    let s = serializer(&json!({
        "type": "model",
        "cls": "Person",
        "schema": {
            "type": "model-fields",
            "fields": {"name": {"type": "model-field", "schema": {"type": "str"}}},
            "computed_fields": [{
                "type": "computed-field",
                "property_name": "name",
                "alias": "shout",
                "return_schema": {"type": "str"},
                "function": function(10, "shout"),
            }],
        },
    }));
    let model = c(r#"{"$model": {"class": "Person", "fields": {"name": "ada"}}}"#);
    let options = c(r#"{"by_alias": true}"#);
    // SAFETY: live handle and valid strings.
    let result = take(unsafe { pd_serializer_to_json(s, model.as_ptr(), options.as_ptr()) });
    assert_eq!(result["ok"], r#"{"name":"ada","shout":"ADA"}"#);
    // SAFETY: handle from pd_serializer_new.
    unsafe { pd_serializer_free(s) };
}
