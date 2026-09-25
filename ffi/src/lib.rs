//! C ABI over perldantic-core, consumed by the Perldantic Perl distribution.
//!
//! Phase 1 of the transport (docs/PLAN.md §5): schemas, configs, options and values cross the
//! boundary as JSON text in the wire format of [`wire`]. Every function that returns a `char *`
//! returns a JSON result envelope, which the caller must release with [`pd_string_free`]:
//!
//! - `{"ok": <value>}`: success; serializer results also carry `"warning"` (a string or `null`);
//! - `{"validation_error": {"title", "message", "errors": [...]}}`: the input is invalid; each
//!   error has pydantic's `type`, `loc`, `msg`, `input`, `ctx` (when present) and `url`;
//! - `{"error": {"type", "message"}}`: anything else, `type` being the name of the exception
//!   pydantic would raise (`SchemaError`, `TypeError`, ...), or `InternalError` when the call
//!   panicked or its arguments were unusable. `HostException` errors, raised by a host function
//!   (see [`host`]), also carry the host's `id` for the exception.
//!
//! The `*_binary` exports carry values in the binary form of the wire format ([`binary`]) instead:
//! the host writes and reads it without JSON text, and gets its result in a buffer it releases
//! with [`pd_buffer_free`].
//!
//! No panic crosses the boundary: every export runs under `catch_unwind`.

pub mod binary;
pub mod host;
pub mod host_input;
pub mod lazy;
pub mod options;
pub mod wire;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::any::Any;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use perldantic_core::{
    CoreError, Dict, ErrorsOptions, LocItem, SchemaSerializer, SchemaValidator, SerializeError,
    UrlHost, ValidateError, ValidationError, Value, generate_json_schema,
};

/// NUL-terminated copy of [`perldantic_core::VERSION`], built at compile time.
const VERSION_CSTR: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");

/// A compiled validator (opaque to C).
pub struct PdValidator(SchemaValidator);

/// A compiled serializer (opaque to C).
pub struct PdSerializer(SchemaSerializer);

/// A result envelope or the error one, as JSON text.
type Envelope = Result<String, String>;

pub(crate) fn error_envelope(type_: &str, message: &str) -> String {
    let mut out = String::from("{\"error\":{\"type\":");
    wire::write_str(type_, &mut out);
    out.push_str(",\"message\":");
    wire::write_str(message, &mut out);
    out.push_str("}}");
    out
}

pub(crate) fn core_error(error: &CoreError) -> String {
    if let CoreError::Host(exception) = error
        && let Some(id) = exception.payload().downcast_ref::<u64>()
    {
        let mut out = String::from("{\"error\":{\"type\":");
        wire::write_str(error.kind().python_name(), &mut out);
        out.push_str(",\"message\":");
        wire::write_str(&error.to_string(), &mut out);
        out.push_str(",\"id\":");
        out.push_str(&id.to_string());
        out.push_str("}}");
        return out;
    }
    error_envelope(error.kind().python_name(), &error.to_string())
}

pub(crate) fn internal_error(message: &str) -> String {
    error_envelope("InternalError", message)
}

pub(crate) fn ok_envelope(value_json: &str) -> String {
    format!("{{\"ok\":{value_json}}}")
}

fn ok_with_warning(value_json: &str, warning: Option<&str>) -> String {
    let mut out = format!("{{\"ok\":{value_json},\"warning\":");
    match warning {
        Some(warning) => wire::write_str(warning, &mut out),
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

pub(crate) fn validation_error(error: &ValidationError) -> String {
    let mut out = String::from("{\"validation_error\":{\"title\":");
    wire::write_str(error.title(), &mut out);
    out.push_str(",\"message\":");
    wire::write_str(&error.to_string(), &mut out);
    out.push_str(",\"errors\":[");
    for (i, details) in error.errors(&ErrorsOptions::default()).iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"type\":");
        wire::write_str(&details.type_, &mut out);
        out.push_str(",\"loc\":[");
        for (j, item) in details.loc.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            match item {
                LocItem::S(s) => wire::write_str(s, &mut out),
                LocItem::I(i) => out.push_str(&i.to_string()),
            }
        }
        out.push_str("],\"msg\":");
        wire::write_str(&details.msg, &mut out);
        if let Some(input) = &details.input {
            out.push_str(",\"input\":");
            out.push_str(&wire::encode(input));
        }
        if let Some(ctx) = &details.ctx {
            out.push_str(",\"ctx\":");
            out.push_str(&wire::encode(ctx));
        }
        if let Some(url) = &details.url {
            out.push_str(",\"url\":");
            wire::write_str(url, &mut out);
        }
        out.push('}');
    }
    out.push_str("]}}");
    out
}

fn validate_error(error: &ValidateError) -> String {
    match error {
        ValidateError::Validation(e) => validation_error(e),
        ValidateError::Core(e) => core_error(e),
    }
}

pub(crate) fn serialize_error(error: &SerializeError) -> String {
    match error {
        SerializeError::Core(error) => core_error(error),
        _ => error_envelope(error.python_name(), &error.to_string()),
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".to_owned());
    format!("Rust panic in perldantic-core: {detail}")
}

/// Run `body` so that no panic unwinds into C: a panic becomes an `InternalError` envelope.
pub(crate) fn guard(body: impl FnOnce() -> Envelope) -> String {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(envelope) | Err(envelope)) => envelope,
        Err(payload) => internal_error(&panic_message(payload.as_ref())),
    }
}

pub(crate) fn into_c(envelope: String) -> *mut c_char {
    // Envelopes are JSON, which escapes NUL characters.
    CString::new(envelope)
        .expect("JSON text has no NUL bytes")
        .into_raw()
}

/// Read a NUL-terminated UTF-8 argument; `None` for a null pointer.
///
/// # Safety
/// `ptr` is null or points to a NUL-terminated string that outlives the call.
unsafe fn text_arg<'a>(ptr: *const c_char, name: &str) -> Result<Option<&'a str>, String> {
    if ptr.is_null() {
        return Ok(None);
    }
    // SAFETY: guaranteed by the caller.
    let text = unsafe { CStr::from_ptr(ptr) };
    text.to_str()
        .map(Some)
        .map_err(|_| internal_error(&format!("`{name}` is not valid UTF-8")))
}

/// Read a required wire JSON argument.
///
/// # Safety
/// As for [`text_arg`].
pub(crate) unsafe fn value_arg(ptr: *const c_char, name: &str) -> Result<Value, String> {
    // SAFETY: guaranteed by the caller.
    match unsafe { text_arg(ptr, name) }? {
        Some(json) => wire::decode(json).map_err(|e| core_error(&e)),
        None => Err(internal_error(&format!("`{name}` is a null pointer"))),
    }
}

/// Read an optional wire JSON argument; null pointers and JSON `null` are `None`.
///
/// # Safety
/// As for [`text_arg`].
pub(crate) unsafe fn optional_value_arg(
    ptr: *const c_char,
    name: &str,
) -> Result<Option<Value>, String> {
    // SAFETY: guaranteed by the caller.
    match unsafe { text_arg(ptr, name) }? {
        None => Ok(None),
        Some(json) => match wire::decode(json).map_err(|e| core_error(&e))? {
            Value::None => Ok(None),
            value => Ok(Some(value)),
        },
    }
}

/// Read the keyword options object; a null pointer means no options.
///
/// # Safety
/// As for [`text_arg`].
unsafe fn options_arg(ptr: *const c_char) -> Result<Dict, String> {
    // SAFETY: guaranteed by the caller.
    match unsafe { optional_value_arg(ptr, "options") }? {
        None => Ok(Dict::new()),
        Some(Value::Dict(options)) => Ok(options),
        Some(other) => Err(core_error(&CoreError::Type(format!(
            "options should be an object, got {}",
            other.type_name()
        )))),
    }
}

/// Build a handle, reporting failure through `error`.
///
/// # Safety
/// `error` is null or points to writable storage for one pointer.
unsafe fn construct<T>(
    error: *mut *mut c_char,
    build: impl FnOnce() -> Result<T, String>,
) -> *mut T {
    let mut built = None;
    let envelope = guard(|| {
        built = Some(build()?);
        Ok(String::new())
    });
    match built {
        Some(handle) => Box::into_raw(Box::new(handle)),
        None => {
            if !error.is_null() {
                // SAFETY: guaranteed by the caller.
                unsafe { *error = into_c(envelope) };
            }
            ptr::null_mut()
        }
    }
}

/// Borrow a handle, or fail for a null pointer.
///
/// # Safety
/// `handle` is null or was returned by the matching `*_new` and not freed.
unsafe fn handle_arg<'a, T>(handle: *const T, name: &str) -> Result<&'a T, String> {
    // SAFETY: guaranteed by the caller.
    unsafe { handle.as_ref() }.ok_or_else(|| internal_error(&format!("`{name}` is a null pointer")))
}

/// Returns the library version as a static NUL-terminated string.
///
/// The pointer is valid for the lifetime of the process and must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn pd_version() -> *const c_char {
    VERSION_CSTR.as_ptr().cast()
}

/// Compile a validator from a core schema and an optional core config (wire JSON; `config` may
/// be null). Returns null on failure and, when `error` is not null, stores an error envelope in
/// `*error`, to be released with [`pd_string_free`]. Release the validator with
/// [`pd_validator_free`].
///
/// # Safety
/// String arguments are null or NUL-terminated; `error` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_new(
    schema: *const c_char,
    config: *const c_char,
    error: *mut *mut c_char,
) -> *mut PdValidator {
    // SAFETY: guaranteed by the caller.
    unsafe {
        construct(error, || {
            let schema = value_arg(schema, "schema")?;
            let config = optional_value_arg(config, "config")?;
            SchemaValidator::new(&schema, config.as_ref())
                .map(PdValidator)
                .map_err(|e| core_error(&e))
        })
    }
}

/// Release a validator; null is ignored.
///
/// # Safety
/// `validator` is null or was returned by [`pd_validator_new`] and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_free(validator: *mut PdValidator) {
    if !validator.is_null() {
        // SAFETY: guaranteed by the caller; the handle came from `Box::into_raw`.
        drop(unsafe { Box::from_raw(validator) });
    }
}

/// Validate host data (upstream `validate_python`): `input` is wire JSON, `options` a wire JSON
/// object of keyword options or null. The extra option `input_type` is `"perl"` (the default:
/// Perl words in error messages, arrays accepted as strict tuples) or `"python"` (exactly
/// `validate_python`). Returns a result envelope.
///
/// # Safety
/// `validator` is null or a live handle; string arguments are null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_validate(
    validator: *const PdValidator,
    input: *const c_char,
    options: *const c_char,
) -> *mut c_char {
    into_c(guard(|| {
        // SAFETY: guaranteed by the caller.
        let (validator, input, options) = unsafe {
            (
                handle_arg(validator, "validator")?,
                value_arg(input, "input")?,
                options_arg(options)?,
            )
        };
        let (options, input_type) =
            options::host_validate_options(&options).map_err(|e| core_error(&e))?;
        let output = validator
            .0
            .validate_value_as(&input, input_type, &options)
            .map_err(|e| validate_error(&e))?;
        Ok(ok_envelope(&wire::encode(&output)))
    }))
}

/// Validate a JSON document (upstream `validate_json`): `json` points to `len` bytes of JSON
/// text, `options` is a wire JSON object of keyword options or null. Returns a result envelope.
///
/// # Safety
/// `validator` is null or a live handle; `json` points to `len` readable bytes (or is null with
/// `len` 0); `options` is null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_validate_json(
    validator: *const PdValidator,
    json: *const u8,
    len: usize,
    options: *const c_char,
) -> *mut c_char {
    into_c(guard(|| {
        // SAFETY: guaranteed by the caller.
        let (validator, options) =
            unsafe { (handle_arg(validator, "validator")?, options_arg(options)?) };
        // SAFETY: guaranteed by the caller.
        let text = unsafe { json_arg(json, len)? };
        let options = options::validate_options(&options).map_err(|e| core_error(&e))?;
        let output = validator
            .0
            .validate_json(text, &options)
            .map_err(|e| validate_error(&e))?;
        Ok(ok_envelope(&wire::encode(&output)))
    }))
}

/// A JSON document argument as text, or the error pydantic raises for bytes that are not UTF-8.
///
/// # Safety
/// `json` points to `len` readable bytes that outlive the call, or is null with `len` 0.
unsafe fn json_arg<'a>(json: *const u8, len: usize) -> Result<&'a str, String> {
    let bytes: &[u8] = if json.is_null() {
        if len != 0 {
            return Err(internal_error("`json` is a null pointer"));
        }
        &[]
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts(json, len) }
    };
    std::str::from_utf8(bytes).map_err(|e| {
        core_error(&CoreError::UnicodeDecode(format!(
            "'utf-8' codec can't decode byte 0x{:02x} in position {}: invalid utf-8",
            bytes[e.valid_up_to()],
            e.valid_up_to()
        )))
    })
}

/// Compile a serializer from a core schema and an optional core config; see
/// [`pd_validator_new`] for the conventions. Release it with [`pd_serializer_free`].
///
/// # Safety
/// String arguments are null or NUL-terminated; `error` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_serializer_new(
    schema: *const c_char,
    config: *const c_char,
    error: *mut *mut c_char,
) -> *mut PdSerializer {
    // SAFETY: guaranteed by the caller.
    unsafe {
        construct(error, || {
            let schema = value_arg(schema, "schema")?;
            let config = optional_value_arg(config, "config")?;
            SchemaSerializer::new(&schema, config.as_ref())
                .map(PdSerializer)
                .map_err(|e| core_error(&e))
        })
    }
}

/// Release a serializer; null is ignored.
///
/// # Safety
/// `serializer` is null or was returned by [`pd_serializer_new`] and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_serializer_free(serializer: *mut PdSerializer) {
    if !serializer.is_null() {
        // SAFETY: guaranteed by the caller; the handle came from `Box::into_raw`.
        drop(unsafe { Box::from_raw(serializer) });
    }
}

/// Serialize to host data (upstream `to_python`): `value` is wire JSON, `options` a wire JSON
/// object of keyword options or null. The envelope's `ok` is the wire value and `warning` the
/// serializer warning, if any.
///
/// # Safety
/// `serializer` is null or a live handle; string arguments are null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_serializer_to_data(
    serializer: *const PdSerializer,
    value: *const c_char,
    options: *const c_char,
) -> *mut c_char {
    into_c(guard(|| {
        // SAFETY: guaranteed by the caller.
        let (serializer, value, options) = unsafe {
            (
                handle_arg(serializer, "serializer")?,
                value_arg(value, "value")?,
                options_arg(options)?,
            )
        };
        let (options, _) = options::serialize_options(&options).map_err(|e| core_error(&e))?;
        let result = serializer
            .0
            .to_python(&value, &options)
            .map_err(|e| serialize_error(&e))?;
        Ok(ok_with_warning(
            &wire::encode(&result.output),
            result.warning.as_deref(),
        ))
    }))
}

/// Serialize to JSON text (upstream `to_json`); `options` may also hold `indent` and
/// `ensure_ascii`. The envelope's `ok` is the JSON text as a string.
///
/// # Safety
/// `serializer` is null or a live handle; string arguments are null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_serializer_to_json(
    serializer: *const PdSerializer,
    value: *const c_char,
    options: *const c_char,
) -> *mut c_char {
    into_c(guard(|| {
        // SAFETY: guaranteed by the caller.
        let (serializer, value, options) = unsafe {
            (
                handle_arg(serializer, "serializer")?,
                value_arg(value, "value")?,
                options_arg(options)?,
            )
        };
        let (options, json) = options::serialize_options(&options).map_err(|e| core_error(&e))?;
        let result = serializer
            .0
            .to_json(&value, &options, &json)
            .map_err(|e| serialize_error(&e))?;
        let mut text = String::new();
        wire::write_str(&result.output, &mut text);
        Ok(ok_with_warning(&text, result.warning.as_deref()))
    }))
}

// ---- binary transport -----------------------------------------------------------------------

/// A binary call's outcome: the result value, with a serializer's warning, or an error envelope.
type BinaryResult = Result<(Value, Option<String>), String>;

/// The buffer a binary call returns: `B` and the value in the binary wire format ([`binary`]);
/// `W`, the warning's length (`u32`, little endian) and UTF-8 bytes, then the value; or `J` and a
/// JSON error envelope, as the other exports return. No panic unwinds into C.
fn into_buffer(body: impl FnOnce() -> BinaryResult, len: *mut usize) -> *mut u8 {
    into_buffer_with(body, len, binary::encode)
}

/// [`into_buffer`] writing the value with `encode`.
fn into_buffer_with(
    body: impl FnOnce() -> BinaryResult,
    len: *mut usize,
    encode: fn(&Value) -> Vec<u8>,
) -> *mut u8 {
    let buffer = match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok((value, None))) => {
            let mut out = vec![b'B'];
            out.extend(encode(&value));
            out
        }
        Ok(Ok((value, Some(warning)))) => {
            let mut out = vec![b'W'];
            let warning_len = u32::try_from(warning.len()).expect("warnings are short");
            out.extend_from_slice(&warning_len.to_le_bytes());
            out.extend_from_slice(warning.as_bytes());
            out.extend(encode(&value));
            out
        }
        Ok(Err(envelope)) => [b"J".as_slice(), envelope.as_bytes()].concat(),
        Err(payload) => [
            b"J".as_slice(),
            internal_error(&panic_message(payload.as_ref())).as_bytes(),
        ]
        .concat(),
    };
    let buffer = buffer.into_boxed_slice();
    if !len.is_null() {
        // SAFETY: guaranteed by the caller of the export.
        unsafe { *len = buffer.len() };
    }
    Box::into_raw(buffer).cast::<u8>()
}

/// Read a binary wire value argument.
///
/// # Safety
/// `ptr` is null or points to `len` readable bytes that outlive the call.
unsafe fn binary_arg(ptr: *const u8, len: usize, name: &str) -> Result<Value, String> {
    if ptr.is_null() {
        return Err(internal_error(&format!("`{name}` is a null pointer")));
    }
    // SAFETY: guaranteed by the caller.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    binary::decode(bytes).map_err(|e| core_error(&e))
}

/// [`pd_validator_validate`] with the input and the result in the binary wire format: `input`
/// points to `input_len` bytes. The returned buffer is `*len` bytes long and must be released
/// with [`pd_buffer_free`]: `B` and the result value; `W`, the warning's length (`u32`, little
/// endian) and UTF-8 bytes, then the value; or `J` and a JSON error envelope.
///
/// # Safety
/// `validator` is null or a live handle; `input` is null or points to `input_len` bytes;
/// `options` is null or NUL-terminated; `len` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_validate_binary(
    validator: *const PdValidator,
    input: *const u8,
    input_len: usize,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    into_buffer(
        || {
            // SAFETY: guaranteed by the caller.
            let (validator, input, options) = unsafe {
                (
                    handle_arg(validator, "validator")?,
                    binary_arg(input, input_len, "input")?,
                    options_arg(options)?,
                )
            };
            let (options, input_type) =
                options::host_validate_options(&options).map_err(|e| core_error(&e))?;
            let output = validator
                .0
                .validate_value_as(&input, input_type, &options)
                .map_err(|e| validate_error(&e))?;
            Ok((output, None))
        },
        len,
    )
}

/// Whether the input is valid: [`pd_validator_validate_binary`] without the validated value,
/// which the caller does not need. The buffer holds `B` and a boolean; invalid input is `false`,
/// not an error. Anything else that fails (malformed input, an exception of a host function)
/// is a `J` error envelope, as for validation.
///
/// # Safety
/// As for [`pd_validator_validate_binary`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_check_binary(
    validator: *const PdValidator,
    input: *const u8,
    input_len: usize,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    into_buffer(
        || {
            // SAFETY: guaranteed by the caller.
            let (validator, input, options) = unsafe {
                (
                    handle_arg(validator, "validator")?,
                    binary_arg(input, input_len, "input")?,
                    options_arg(options)?,
                )
            };
            let (options, input_type) =
                options::host_validate_options(&options).map_err(|e| core_error(&e))?;
            match validator.0.validate_value_as(&input, input_type, &options) {
                Ok(_) => Ok((Value::Bool(true), None)),
                Err(ValidateError::Validation(_)) => Ok((Value::Bool(false), None)),
                Err(e) => Err(validate_error(&e)),
            }
        },
        len,
    )
}

/// Validate host data read in place: `root` is a node of `host` (see [`host_input::PdHost`]).
/// Returns the result as [`pd_validator_validate_binary`] does; when the host fails to convert a
/// value, the result is an error envelope and the host raises its own error.
///
/// # Safety
/// `validator` is null or a live handle; `host` points to a table whose functions accept
/// `root` and every node they return, for the whole call; `options` is null or NUL-terminated;
/// `len` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_validate_host(
    validator: *const PdValidator,
    host: *const host_input::PdHost,
    root: *mut std::ffi::c_void,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    // SAFETY: guaranteed by the caller.
    unsafe { validate_host(validator, host, root, options, len, Mode::Validate) }
}

/// [`pd_validator_validate_host`] for lazy host objects: models in the result are lazy nodes
/// (tag 12 of [`binary`]), each a handle the host releases with [`pd_lazy_release`].
///
/// # Safety
/// As for [`pd_validator_validate_host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_validate_host_lazy(
    validator: *const PdValidator,
    host: *const host_input::PdHost,
    root: *mut std::ffi::c_void,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    // SAFETY: guaranteed by the caller.
    unsafe { validate_host(validator, host, root, options, len, Mode::Lazy) }
}

/// [`pd_validator_validate_host`] answering only whether the input is valid, as
/// [`pd_validator_check_binary`] does.
///
/// # Safety
/// As for [`pd_validator_validate_host`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_check_host(
    validator: *const PdValidator,
    host: *const host_input::PdHost,
    root: *mut std::ffi::c_void,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    // SAFETY: guaranteed by the caller.
    unsafe { validate_host(validator, host, root, options, len, Mode::Check) }
}

/// What a validation of host data returns.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Validate,
    Check,
    Lazy,
}

/// # Safety
/// As for [`pd_validator_validate_host`].
unsafe fn validate_host(
    validator: *const PdValidator,
    host: *const host_input::PdHost,
    root: *mut std::ffi::c_void,
    options: *const c_char,
    len: *mut usize,
    mode: Mode,
) -> *mut u8 {
    let check = mode == Mode::Check;
    into_buffer_with(
        || {
            // SAFETY: guaranteed by the caller.
            let (validator, host, options) = unsafe {
                (
                    handle_arg(validator, "validator")?,
                    handle_arg(host, "host")?,
                    options_arg(options)?,
                )
            };
            let (options, input_type) =
                options::host_validate_options(&options).map_err(|e| core_error(&e))?;
            // SAFETY: guaranteed by the caller.
            let host = unsafe { host_input::PerlHost::new(host) };
            let input = perldantic_core::HostInput::new(&host, host_input::HostNode(root));
            let result = validator.0.validate_host_as(&input, input_type, &options);
            if host.failed() {
                return Err(internal_error("The host could not convert a value"));
            }
            match (result, check) {
                (Ok(output), false) => Ok((output, None)),
                (Ok(_), true) => Ok((Value::Bool(true), None)),
                (Err(ValidateError::Validation(_)), true) => Ok((Value::Bool(false), None)),
                (Err(e), _) => Err(validate_error(&e)),
            }
        },
        len,
        if mode == Mode::Lazy {
            binary::encode_lazy
        } else {
            binary::encode
        },
    )
}

/// [`pd_validator_validate_json`] for lazy host objects, returning a buffer as
/// [`pd_validator_validate_host_lazy`] does.
///
/// # Safety
/// As for [`pd_validator_validate_json`]; `len` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_validate_json_lazy(
    validator: *const PdValidator,
    json: *const u8,
    json_len: usize,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    into_buffer_with(
        || {
            // SAFETY: guaranteed by the caller.
            let (validator, options) =
                unsafe { (handle_arg(validator, "validator")?, options_arg(options)?) };
            // SAFETY: guaranteed by the caller.
            let text = unsafe { json_arg(json, json_len)? };
            let options = options::validate_options(&options).map_err(|e| core_error(&e))?;
            let output = validator
                .0
                .validate_json(text, &options)
                .map_err(|e| validate_error(&e))?;
            Ok((output, None))
        },
        len,
        binary::encode_lazy,
    )
}

/// The contents of a lazy model for its host object: a buffer as
/// [`pd_validator_validate_binary`] returns, holding the model in full (tag 10), with the
/// models in its fields as new lazy nodes; an error envelope when the handle is not live.
///
/// # Safety
/// `len` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_lazy_contents(handle: u64, len: *mut usize) -> *mut u8 {
    let model = usize::try_from(handle).ok().and_then(lazy::get);
    let buffer = match model {
        Some(model) => [b"B".as_slice(), &binary::encode_lazy_contents(&model)].concat(),
        None => [
            b"J".as_slice(),
            internal_error("A lazy model handle is not live").as_bytes(),
        ]
        .concat(),
    };
    let buffer = buffer.into_boxed_slice();
    if !len.is_null() {
        // SAFETY: guaranteed by the caller.
        unsafe { *len = buffer.len() };
    }
    Box::into_raw(buffer).cast::<u8>()
}

/// A new handle to the model behind `handle` (for a copy of the host object), or 0 when the
/// handle is not live.
#[unsafe(no_mangle)]
pub extern "C" fn pd_lazy_clone(handle: u64) -> u64 {
    usize::try_from(handle)
        .ok()
        .and_then(lazy::get)
        .map_or(0, |model| lazy::register(model) as u64)
}

/// Release a handle of a lazy model; unknown handles are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn pd_lazy_release(handle: u64) {
    if let Ok(handle) = usize::try_from(handle) {
        lazy::release(handle);
    }
}

/// How many handles of lazy models are live (for leak checks).
#[unsafe(no_mangle)]
pub extern "C" fn pd_lazy_live() -> usize {
    lazy::live()
}

/// [`pd_serializer_to_data`] with the value and the result in the binary wire format, returned
/// as [`pd_validator_validate_binary`] returns its result.
///
/// # Safety
/// As for [`pd_validator_validate_binary`], with a live serializer handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_serializer_to_data_binary(
    serializer: *const PdSerializer,
    value: *const u8,
    value_len: usize,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    into_buffer(
        || {
            // SAFETY: guaranteed by the caller.
            let (serializer, value, options) = unsafe {
                (
                    handle_arg(serializer, "serializer")?,
                    binary_arg(value, value_len, "value")?,
                    options_arg(options)?,
                )
            };
            let (options, _) = options::serialize_options(&options).map_err(|e| core_error(&e))?;
            let result = serializer
                .0
                .to_python(&value, &options)
                .map_err(|e| serialize_error(&e))?;
            Ok((result.output, result.warning))
        },
        len,
    )
}

/// [`pd_serializer_to_json`] with the value in the binary wire format; the JSON text comes back
/// as a string value in the buffer, as [`pd_validator_validate_binary`] returns its result.
///
/// # Safety
/// As for [`pd_serializer_to_data_binary`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_serializer_to_json_binary(
    serializer: *const PdSerializer,
    value: *const u8,
    value_len: usize,
    options: *const c_char,
    len: *mut usize,
) -> *mut u8 {
    into_buffer(
        || {
            // SAFETY: guaranteed by the caller.
            let (serializer, value, options) = unsafe {
                (
                    handle_arg(serializer, "serializer")?,
                    binary_arg(value, value_len, "value")?,
                    options_arg(options)?,
                )
            };
            let (options, json) =
                options::serialize_options(&options).map_err(|e| core_error(&e))?;
            let result = serializer
                .0
                .to_json(&value, &options, &json)
                .map_err(|e| serialize_error(&e))?;
            Ok((Value::Str(result.output), result.warning))
        },
        len,
    )
}

/// Release a buffer returned by a binary export; null is ignored.
///
/// # Safety
/// `buffer` is null or was returned by a binary export with length `len`, and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_buffer_free(buffer: *mut u8, len: usize) {
    if !buffer.is_null() {
        // SAFETY: guaranteed by the caller; the buffer came from a boxed slice of `len` bytes.
        drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(buffer, len)) });
    }
}

/// Generate the JSON Schema of a core schema. `config` (wire JSON or null) applies to the whole
/// schema like a `TypeAdapter`'s config; `options` holds `mode`, `by_alias`, `ref_template` and
/// `union_format`. The envelope's `ok` is the JSON Schema and `warnings` a list of messages.
///
/// # Safety
/// String arguments are null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_json_schema(
    schema: *const c_char,
    config: *const c_char,
    options: *const c_char,
) -> *mut c_char {
    into_c(guard(|| {
        // SAFETY: guaranteed by the caller.
        let (schema, config, options) = unsafe {
            (
                value_arg(schema, "schema")?,
                optional_value_arg(config, "config")?,
                options_arg(options)?,
            )
        };
        let options = options::json_schema_options(&options).map_err(|e| core_error(&e))?;
        let generated = generate_json_schema(&schema, config.as_ref(), &options)
            .map_err(|e| error_envelope(e.python_name(), &e.to_string()))?;
        let warnings = Value::List(generated.warnings.into_iter().map(Value::Str).collect());
        Ok(format!(
            "{{\"ok\":{},\"warnings\":{}}}",
            wire::encode(&generated.schema),
            wire::encode(&warnings)
        ))
    }))
}

/// The accessors of a URL (`{"$url": ...}` or `{"$multi_host_url": ...}` wire JSON), as
/// pydantic's `Url` and `MultiHostUrl` give them: `scheme`, `username`, `password`, `host`,
/// `unicode_host`, `port` (or `hosts` for a multi-host URL), `path`, `query`, `query_params`
/// (a list of `[key, value]` pairs), `fragment` and `unicode_string`.
///
/// # Safety
/// `url` is null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_url_parts(url: *const c_char) -> *mut c_char {
    into_c(guard(|| {
        // SAFETY: guaranteed by the caller.
        let url = unsafe { value_arg(url, "url") }?;
        let opt = |s: Option<&str>| s.map_or(Value::None, Value::from);
        let query_params = |params: Vec<(String, String)>| {
            Value::List(
                params
                    .into_iter()
                    .map(|(k, v)| Value::List(vec![Value::Str(k), Value::Str(v)]))
                    .collect(),
            )
        };
        let port = |port: Option<u16>| port.map_or(Value::None, |p| Value::Int(i64::from(p)));
        let mut parts = Dict::new();
        let mut set = |key: &str, value: Value| {
            parts.insert(Value::from(key), value);
        };
        match &url {
            Value::Url(u) => {
                set("scheme", u.scheme().into());
                set("username", opt(u.username()));
                set("password", opt(u.password()));
                set("host", opt(u.host()));
                set("unicode_host", opt(u.unicode_host().as_deref()));
                set("port", port(u.port()));
                set("path", opt(u.path()));
                set("query", opt(u.query()));
                set("query_params", query_params(u.query_params()));
                set("fragment", opt(u.fragment()));
                set("unicode_string", u.unicode_string().into());
            }
            Value::MultiHostUrl(u) => {
                let host = |h: &UrlHost| {
                    let mut dict = Dict::new();
                    dict.insert("username".into(), opt(h.username.as_deref()));
                    dict.insert("password".into(), opt(h.password.as_deref()));
                    dict.insert("host".into(), opt(h.host.as_deref()));
                    dict.insert("port".into(), port(h.port));
                    Value::Dict(dict)
                };
                set("scheme", u.scheme().into());
                set("hosts", Value::List(u.hosts().iter().map(host).collect()));
                set("path", opt(u.path()));
                set("query", opt(u.query()));
                set("query_params", query_params(u.query_params()));
                set("fragment", opt(u.fragment()));
                set("unicode_string", u.unicode_string().into());
            }
            other => {
                return Err(core_error(&CoreError::Type(format!(
                    "expected a $url or $multi_host_url value, got {}",
                    wire::encode(other)
                ))));
            }
        }
        Ok(ok_envelope(&wire::encode(&Value::Dict(parts))))
    }))
}

/// Release a string returned by this library; null is ignored.
///
/// # Safety
/// `string` is null or was returned by a `pd_*` function and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_string_free(string: *mut c_char) {
    if !string.is_null() {
        // SAFETY: guaranteed by the caller; the string came from `CString::into_raw`.
        drop(unsafe { CString::from_raw(string) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panics_become_internal_errors() {
        let envelope = guard(|| panic!("boom"));
        assert_eq!(
            envelope,
            r#"{"error":{"type":"InternalError","message":"Rust panic in perldantic-core: boom"}}"#
        );
        let envelope = guard(|| std::panic::panic_any(7_u8));
        assert!(envelope.contains("unknown panic payload"));
    }

    #[test]
    fn a_panicking_constructor_reports_through_the_error_pointer() {
        let mut error: *mut c_char = ptr::null_mut();
        // SAFETY: `error` is writable.
        let handle: *mut PdValidator = unsafe { construct(&raw mut error, || panic!("in build")) };
        assert!(handle.is_null());
        // SAFETY: `construct` stored an envelope from `into_c`.
        let message = unsafe { CStr::from_ptr(error) }
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            message.contains("Rust panic in perldantic-core: in build"),
            "{message}"
        );
        // SAFETY: returned by this library.
        unsafe { pd_string_free(error) };
    }
}
