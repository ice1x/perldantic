//! Host functions over the C ABI: validator and serializer functions of the host language.
//!
//! The host registers one callback with [`pd_set_host_callback`] and refers to its functions
//! by id: a schema holds `{"$function": {"id": 7, "name": "check"}}`. When the core calls such a
//! function, the callback receives the id, the call as JSON and a reply slot, and answers with
//! [`pd_host_reply`] before returning (callbacks cannot return strings on every host).
//!
//! The call is one of:
//!
//! - `{"call": "validate", "input", "info"}` (before / after / plain validators);
//! - `{"call": "validate_wrap", "input", "handler", "info"}`, where `handler` is passed to
//!   [`pd_validator_handler_call`];
//! - `{"call": "serialize", "value", "model", "info"}`;
//! - `{"call": "serialize_wrap", "value", "model", "handler", "info"}`, with
//!   [`pd_serializer_handler_call`].
//!
//! `info` is `null` for functions that take none. The reply is `{"ok": value}` or
//! `{"error": {"kind", ...}}`, `kind` being what the function raised: `value` or `assertion`
//! (with `message`), `custom` (`error_type`, `message_template`, `context`), `known`
//! (`error_type`, `context`), `validation` (a validation error re-raised: `title` and pydantic's
//! `errors`), `omit`, `use_default`, `serialization` or `unexpected_value` (with `message`), or
//! `other` (`message` and the host's `id` for the exception, which comes back unchanged in the
//! `HostException` error of the outer call).

use std::ffi::{CStr, c_char, c_void};
use std::sync::RwLock;

use perldantic_core::{
    CoreError, Dict, ErrorType, HostCall, HostError, HostException, HostFunction, InputType,
    LocItem, SerializationInfo, SerializeError, SerializerHandler, UnexpectedValue, ValLineError,
    ValidationError, ValidationInfo, ValidatorHandler, Value,
};

use crate::wire;

/// The host's callback: `(function id, call JSON, reply slot)`; it must answer through
/// [`pd_host_reply`] before returning, and must not unwind. Null removes the callback.
pub type PdHostCallback = Option<unsafe extern "C" fn(u64, *const c_char, *mut PdReply)>;

/// Where the host puts its answer to one call (opaque to C).
pub struct PdReply(Option<String>);

static CALLBACK: RwLock<PdHostCallback> = RwLock::new(None);

/// Register the host's callback (or remove it with a null pointer). Functions in schemas can
/// only be called while a callback is registered.
#[unsafe(no_mangle)]
pub extern "C" fn pd_set_host_callback(callback: PdHostCallback) {
    *CALLBACK
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = callback;
}

/// Answer a call: `result` is the reply JSON, copied before this returns.
///
/// # Safety
/// `reply` is the slot the callback received, during that callback; `result` is a
/// NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_host_reply(reply: *mut PdReply, result: *const c_char) {
    // SAFETY: guaranteed by the caller.
    let (Some(reply), false) = (unsafe { reply.as_mut() }, result.is_null()) else {
        return;
    };
    // SAFETY: guaranteed by the caller.
    let text = unsafe { CStr::from_ptr(result) };
    reply.0 = Some(text.to_string_lossy().into_owned());
}

/// A function of the host, known by id.
#[derive(Debug)]
pub struct FfiFunction {
    pub id: u64,
    pub name: String,
}

impl FfiFunction {
    /// Run one call through the host's callback and read its reply.
    fn invoke(&self, call: &str) -> Result<Value, HostError> {
        let callback = CALLBACK
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ok_or_else(|| {
                HostError::Core(CoreError::Internal(format!(
                    "cannot call host function `{}`: no host callback is registered",
                    self.name
                )))
            })?;
        let call = std::ffi::CString::new(call).expect("JSON text has no NUL bytes");
        let mut reply = PdReply(None);
        // SAFETY: the callback is the host's, called as its registration promises.
        unsafe { callback(self.id, call.as_ptr(), &raw mut reply) };
        let text = reply.0.ok_or_else(|| {
            HostError::Core(CoreError::Internal(format!(
                "host function `{}` did not reply",
                self.name
            )))
        })?;
        parse_reply(&text)
    }
}

impl HostFunction for FfiFunction {
    fn name(&self) -> &str {
        &self.name
    }

    fn host_id(&self) -> Option<u64> {
        Some(self.id)
    }

    fn call(&self, call: HostCall<'_>) -> Result<Value, HostError> {
        let mut out = String::new();
        match call {
            HostCall::Validate { input, info } => {
                out.push_str("{\"call\":\"validate\",\"input\":");
                out.push_str(&wire::encode(&input));
                push_validation_info(info.as_ref(), &mut out);
                out.push('}');
                self.invoke(&out)
            }
            HostCall::ValidateWrap {
                input,
                handler,
                info,
            } => {
                // the host gets the address of this reference, alive until `invoke` returns
                let mut handler: &mut dyn ValidatorHandler = handler;
                out.push_str("{\"call\":\"validate_wrap\",\"input\":");
                out.push_str(&wire::encode(&input));
                push_handler((&raw mut handler).cast(), &mut out);
                push_validation_info(info.as_ref(), &mut out);
                out.push('}');
                self.invoke(&out)
            }
            HostCall::Serialize { value, model, info } => {
                out.push_str("{\"call\":\"serialize\",\"value\":");
                out.push_str(&wire::encode(&value));
                push_model(model.as_ref(), &mut out);
                push_serialization_info(info.as_ref(), &mut out);
                out.push('}');
                self.invoke(&out)
            }
            HostCall::SerializeWrap {
                value,
                model,
                handler,
                info,
            } => {
                let mut handler: &mut dyn SerializerHandler = handler;
                out.push_str("{\"call\":\"serialize_wrap\",\"value\":");
                out.push_str(&wire::encode(&value));
                push_model(model.as_ref(), &mut out);
                push_handler((&raw mut handler).cast(), &mut out);
                push_serialization_info(info.as_ref(), &mut out);
                out.push('}');
                self.invoke(&out)
            }
        }
    }
}

fn push_handler(handler: *mut c_void, out: &mut String) {
    out.push_str(",\"handler\":");
    out.push_str(&(handler as usize).to_string());
}

fn push_optional(key: &str, value: Option<&Value>, out: &mut String) {
    out.push_str(",\"");
    out.push_str(key);
    out.push_str("\":");
    match value {
        Some(value) => out.push_str(&wire::encode(value)),
        None => out.push_str("null"),
    }
}

fn push_model(model: Option<&Value>, out: &mut String) {
    push_optional("model", model, out);
}

fn push_validation_info(info: Option<&ValidationInfo>, out: &mut String) {
    out.push_str(",\"info\":");
    let Some(info) = info else {
        out.push_str("null");
        return;
    };
    out.push_str("{\"mode\":");
    wire::write_str(info.mode.as_str(), out);
    push_optional("config", info.config.clone().map(Value::Dict).as_ref(), out);
    push_optional("context", info.context.as_ref(), out);
    push_optional("data", info.data.clone().map(Value::Dict).as_ref(), out);
    out.push_str(",\"field_name\":");
    match &info.field_name {
        Some(name) => wire::write_str(name, out),
        None => out.push_str("null"),
    }
    out.push('}');
}

fn push_serialization_info(info: Option<&SerializationInfo>, out: &mut String) {
    out.push_str(",\"info\":");
    let Some(info) = info else {
        out.push_str("null");
        return;
    };
    out.push_str("{\"mode\":");
    wire::write_str(&info.mode.to_string(), out);
    push_optional("include", info.include.as_ref(), out);
    push_optional("exclude", info.exclude.as_ref(), out);
    push_optional("context", info.context.as_ref(), out);
    out.push_str(",\"by_alias\":");
    out.push_str(match info.by_alias {
        Some(true) => "true",
        Some(false) => "false",
        None => "null",
    });
    for (key, flag) in [
        ("exclude_unset", info.exclude_unset),
        ("exclude_defaults", info.exclude_defaults),
        ("exclude_none", info.exclude_none),
        ("exclude_computed_fields", info.exclude_computed_fields),
        ("round_trip", info.round_trip),
        ("serialize_as_any", info.serialize_as_any),
    ] {
        out.push_str(",\"");
        out.push_str(key);
        out.push_str("\":");
        out.push_str(if flag { "true" } else { "false" });
    }
    out.push_str(",\"field_name\":");
    match &info.field_name {
        Some(name) => wire::write_str(name, out),
        None => out.push_str("null"),
    }
    out.push('}');
}

/// An invalid reply is a fault at the host boundary.
fn bad_reply(what: &str) -> HostError {
    HostError::Core(CoreError::Internal(format!(
        "Invalid reply from a host function: {what}"
    )))
}

fn text(dict: &Dict, key: &str) -> Result<String, HostError> {
    match dict.get_str(key) {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(bad_reply(&format!("`{key}` should be a string"))),
    }
}

fn context(dict: &Dict) -> Result<Option<Dict>, HostError> {
    match dict.get_str("context") {
        None | Some(Value::None) => Ok(None),
        Some(Value::Dict(context)) => Ok(Some(context.clone())),
        Some(_) => Err(bad_reply("`context` should be an object")),
    }
}

/// The reply of the host: the function's result or what it raised.
fn parse_reply(reply: &str) -> Result<Value, HostError> {
    let Value::Dict(mut reply) = wire::decode(reply).map_err(|e| bad_reply(&e.to_string()))? else {
        return Err(bad_reply("the reply should be an object"));
    };
    if let Some(value) = reply.remove_str("ok") {
        return Ok(value);
    }
    let Some(Value::Dict(error)) = reply.remove_str("error") else {
        return Err(bad_reply("the reply needs `ok` or `error`"));
    };
    Err(match text(&error, "kind")?.as_str() {
        "value" => HostError::Value(text(&error, "message")?),
        "assertion" => HostError::Assertion(text(&error, "message")?),
        "custom" => HostError::Custom {
            error_type: text(&error, "error_type")?,
            message_template: text(&error, "message_template")?,
            context: context(&error)?,
        },
        "known" => HostError::Known(
            ErrorType::new(&text(&error, "error_type")?, context(&error)?.as_ref())
                .map_err(HostError::Core)?,
        ),
        "validation" => HostError::Validation(validation_error(&error)?),
        "omit" => HostError::Omit,
        "use_default" => HostError::UseDefault,
        "serialization" => {
            HostError::Serialization(SerializeError::Serialization(text(&error, "message")?))
        }
        "unexpected_value" => HostError::Serialization(SerializeError::UnexpectedValue(
            UnexpectedValue::new_from_msg(Some(text(&error, "message")?)),
        )),
        "other" => {
            let id = match error.get_str("id") {
                Some(Value::Int(id)) => u64::try_from(*id).map_err(|_| bad_reply("bad `id`"))?,
                _ => return Err(bad_reply("`other` errors need the host's `id`")),
            };
            HostError::Other(HostException::new(text(&error, "message")?, id))
        }
        other => return Err(bad_reply(&format!("unknown error kind `{other}`"))),
    })
}

/// A validation error raised in the host (e.g. by a handler, or by validation the function
/// ran itself), rebuilt from pydantic's error details: built-in types by name, others as
/// custom errors with their message.
fn validation_error(error: &Dict) -> Result<ValidationError, HostError> {
    let title = text(error, "title")?;
    let Some(Value::List(details)) = error.get_str("errors") else {
        return Err(bad_reply("`errors` should be a list"));
    };
    let mut line_errors = Vec::with_capacity(details.len());
    for detail in details {
        let Value::Dict(detail) = detail else {
            return Err(bad_reply("each error should be an object"));
        };
        let type_ = text(detail, "type")?;
        let ctx = match detail.get_str("ctx") {
            Some(Value::Dict(ctx)) => Some(ctx.clone()),
            _ => None,
        };
        let error_type = if ErrorType::valid_type(&type_) {
            ErrorType::new(&type_, ctx.as_ref()).map_err(HostError::Core)?
        } else {
            ErrorType::new_custom_error(type_, text(detail, "msg")?, ctx)
        };
        let input = detail.get_str("input").cloned().unwrap_or(Value::None);
        let mut line = ValLineError::new(error_type, input);
        if let Some(Value::List(loc)) = detail.get_str("loc") {
            for item in loc.iter().rev() {
                line = line.with_outer_location(match item {
                    Value::Int(i) => LocItem::I(*i),
                    other => LocItem::S(other.py_str()),
                });
            }
        }
        line_errors.push(line);
    }
    Ok(ValidationError::new(
        title,
        line_errors,
        InputType::Python,
        false,
    ))
}

/// The result of a handler call as an envelope (see the crate documentation); what the
/// handler raised keeps its kind so the host can raise it again.
fn handler_envelope(result: Result<Value, HostError>) -> String {
    match result {
        Ok(value) => crate::ok_envelope(&wire::encode(&value)),
        Err(HostError::Validation(error)) => crate::validation_error(&error),
        Err(HostError::Core(error)) => crate::core_error(&error),
        Err(HostError::Other(exception)) => crate::core_error(&CoreError::Host(exception)),
        Err(HostError::Serialization(error)) => crate::serialize_error(&error),
        Err(HostError::Omit) => crate::error_envelope("PydanticOmit", "PydanticOmit()"),
        Err(HostError::UseDefault) => {
            crate::error_envelope("PydanticUseDefault", "PydanticUseDefault()")
        }
        Err(other) => crate::internal_error(&format!("unexpected handler failure: {other}")),
    }
}

/// Validate `input` with a wrap validator's `handler` (from a `validate_wrap` call), errors
/// located under `outer_location` (JSON: a string, an integer or `null`; may be null).
///
/// # Safety
/// `handler` is the value of a `validate_wrap` call that has not returned yet; the strings
/// are NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_validator_handler_call(
    handler: *mut c_void,
    input: *const c_char,
    outer_location: *const c_char,
) -> *mut c_char {
    crate::into_c(crate::guard(|| {
        // SAFETY: guaranteed by the caller.
        let input = unsafe { crate::value_arg(input, "input") }?;
        // SAFETY: guaranteed by the caller.
        let outer_location =
            match unsafe { crate::optional_value_arg(outer_location, "outer_location") }? {
                None => None,
                Some(Value::Int(i)) => Some(LocItem::I(i)),
                Some(Value::Str(s)) => Some(LocItem::S(s)),
                Some(other) => {
                    return Err(crate::core_error(&CoreError::Type(format!(
                        "outer_location should be a string or an integer, got {}",
                        other.type_name()
                    ))));
                }
            };
        let handler = handler.cast::<&mut dyn ValidatorHandler>();
        // SAFETY: guaranteed by the caller: the reference `call` handed out is still alive.
        let handler = unsafe { handler.as_mut() }
            .ok_or_else(|| crate::internal_error("`handler` is a null pointer"))?;
        Ok(handler_envelope(handler.validate(input, outer_location)))
    }))
}

/// Serialize `value` with a wrap serializer's `handler` (from a `serialize_wrap` call);
/// `index_key` (JSON: a list index or a dict key, or `null`; may be null) applies `include` /
/// `exclude` at that position, and a filtered-out value is a `PydanticOmit` error.
///
/// # Safety
/// As for [`pd_validator_handler_call`], with a `serialize_wrap` call's handler.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pd_serializer_handler_call(
    handler: *mut c_void,
    value: *const c_char,
    index_key: *const c_char,
) -> *mut c_char {
    crate::into_c(crate::guard(|| {
        // SAFETY: guaranteed by the caller.
        let value = unsafe { crate::value_arg(value, "value") }?;
        // SAFETY: guaranteed by the caller.
        let index_key = unsafe { crate::optional_value_arg(index_key, "index_key") }?;
        let handler = handler.cast::<&mut dyn SerializerHandler>();
        // SAFETY: guaranteed by the caller: the reference `call` handed out is still alive.
        let handler = unsafe { handler.as_mut() }
            .ok_or_else(|| crate::internal_error("`handler` is a null pointer"))?;
        Ok(handler_envelope(handler.serialize(value, index_key)))
    }))
}
