//! Serialization errors. Port of upstream `serializers/errors.rs`.
//!
//! Upstream raises Python exceptions (`PydanticSerializationError`,
//! `PydanticSerializationUnexpectedValue`, `TypeError`, ...); here they are [`SerializeError`]
//! variants, which a host maps onto its own exception classes. Errors raised while writing JSON
//! travel through serde as strings with the same markers upstream uses.

use std::fmt::{self, Write as _};

use serde::ser;

use crate::core_error::CoreError;
use crate::tools::write_truncated_to_limited_bytes;
use crate::value::Value;

pub(super) static UNEXPECTED_TYPE_SER_MARKER: &str = "__PydanticSerializationUnexpectedValue__";
pub(super) static SERIALIZATION_ERR_MARKER: &str = "__PydanticSerializationError__";

/// Why serialization failed.
#[derive(Debug, Clone, PartialEq)]
pub enum SerializeError {
    /// Upstream `PydanticSerializationError`.
    Serialization(String),
    /// Upstream `PydanticSerializationUnexpectedValue`.
    UnexpectedValue(UnexpectedValue),
    /// Another upstream exception, e.g. `TypeError` for an invalid dict key.
    Core(CoreError),
}

pub type SerResult<T> = Result<T, SerializeError>;

impl SerializeError {
    /// Name of the exception class pydantic-core raises.
    pub fn python_name(&self) -> &'static str {
        match self {
            Self::Serialization(_) => "PydanticSerializationError",
            Self::UnexpectedValue(_) => "PydanticSerializationUnexpectedValue",
            Self::Core(e) => e.kind().python_name(),
        }
    }
}

impl SerializeError {
    /// The error as Python prints an exception: `Kind: message`.
    pub fn py_display(&self) -> String {
        format!("{}: {self}", self.python_name())
    }
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialization(msg) => f.write_str(msg),
            Self::UnexpectedValue(value) => f.write_str(&value.to_string()),
            Self::Core(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for SerializeError {}

impl From<CoreError> for SerializeError {
    fn from(err: CoreError) -> Self {
        Self::Core(err)
    }
}

/// An error raised inside serde, as Python shows it: `Kind: message`.
pub(super) fn py_err_se_err<T: ser::Error>(error: &SerializeError) -> T {
    T::custom(error.py_display())
}

/// The error type of the JSON writer.
#[derive(Debug, Clone)]
pub struct PythonSerializerError {
    pub message: String,
}

impl fmt::Display for PythonSerializerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PythonSerializerError {}

impl ser::Error for PythonSerializerError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        PythonSerializerError {
            message: format!("{msg}"),
        }
    }
}

/// Turn a JSON writer error back into the error upstream raises.
pub(super) fn se_err_py_err(error: &PythonSerializerError) -> SerializeError {
    let s = error.to_string();
    if let Some(msg) = s.strip_prefix(UNEXPECTED_TYPE_SER_MARKER) {
        SerializeError::UnexpectedValue(UnexpectedValue::new_from_msg(
            (!msg.is_empty()).then(|| msg.to_string()),
        ))
    } else if let Some(msg) = s.strip_prefix(SERIALIZATION_ERR_MARKER) {
        SerializeError::Serialization(msg.to_string())
    } else {
        SerializeError::Serialization(format!("Error serializing to JSON: {s}"))
    }
}

/// A value that does not match the serializer's type (upstream
/// `PydanticSerializationUnexpectedValue`), reported as a warning or, while checking union
/// members, as an error.
#[derive(Debug, Clone, PartialEq)]
pub struct UnexpectedValue {
    message: Option<String>,
    field_name: Option<String>,
    field_type: Option<String>,
    input_value: Option<Value>,
}

impl UnexpectedValue {
    pub fn new_from_msg(message: Option<String>) -> Self {
        Self {
            message,
            field_name: None,
            field_type: None,
            input_value: None,
        }
    }

    pub fn new_from_parts(
        field_name: Option<String>,
        field_type: Option<String>,
        input_value: Option<Value>,
    ) -> Self {
        Self {
            message: None,
            field_name,
            field_type,
            input_value,
        }
    }

    pub fn new(
        message: Option<String>,
        field_name: Option<String>,
        field_type: Option<String>,
        input_value: Option<Value>,
    ) -> Self {
        Self {
            message,
            field_name,
            field_type,
            input_value,
        }
    }

    /// The message, as `repr()` of the exception shows it.
    pub fn repr(&self) -> String {
        format!("PydanticSerializationUnexpectedValue({self})")
    }
}

impl fmt::Display for UnexpectedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut message = self.message.clone().unwrap_or_default();

        if let Some(field_type) = &self.field_type {
            if !message.is_empty() {
                message.push_str(": ");
            }
            write!(message, "Expected `{field_type}`")?;
            if self.input_value.is_some() {
                message.push_str(" - serialized value may not be as expected");
            }
        }

        if let Some(input_value) = &self.input_value {
            let input_type = input_value.type_name();
            let mut value_str = String::new();
            write_truncated_to_limited_bytes(&mut value_str, &input_value.repr(), 50)?;
            if let Some(field_name) = &self.field_name {
                write!(
                    message,
                    " [field_name='{field_name}', input_value={value_str}, input_type={input_type}]"
                )?;
            } else {
                write!(
                    message,
                    " [input_value={value_str}, input_type={input_type}]"
                )?;
            }
        }

        if message.is_empty() {
            message = "Unexpected Value".to_string();
        }
        f.write_str(&message)
    }
}
