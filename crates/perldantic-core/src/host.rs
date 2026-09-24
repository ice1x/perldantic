//! Functions of the host language, called from inside the core: the Python callables of
//! upstream `function-*` schemas and serializer functions (default factories and hooks come
//! with the tasks that need them).
//!
//! A schema holds a function as a [`Value::Function`]. The core calls it with a [`HostCall`]
//! describing the arguments and gets back a value or a [`HostError`], which stands for what the
//! Python function would have raised.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::core_error::CoreError;
use crate::errors::{ErrorType, LocItem, ValError, ValidationError};
use crate::input::InputType;
use crate::value::{Dict, Value};

/// A function of the host language.
pub trait HostFunction: Send + Sync + fmt::Debug {
    /// The function's name, as upstream's `function_name()` gives it; validator names embed it.
    fn name(&self) -> &str;

    /// Call the function.
    fn call(&self, call: HostCall<'_>) -> Result<Value, HostError>;

    /// The host's own identifier for the function, if it keeps one (an FFI host refers to
    /// its functions by id).
    fn host_id(&self) -> Option<u64> {
        None
    }
}

/// A host function held in a schema. Two handles are equal when they are the same function.
#[derive(Clone)]
pub struct Function(Arc<dyn HostFunction>);

impl Function {
    pub fn new(function: impl HostFunction + 'static) -> Self {
        Self(Arc::new(function))
    }

    pub fn name(&self) -> &str {
        self.0.name()
    }

    pub fn call(&self, call: HostCall<'_>) -> Result<Value, HostError> {
        self.0.call(call)
    }

    pub fn host_id(&self) -> Option<u64> {
        self.0.host_id()
    }
}

impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Function({:?})", self.0)
    }
}

impl PartialEq for Function {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The arguments of a call, by what the function is for.
pub enum HostCall<'a> {
    /// A `function-before`, `-after` or `-plain` validator: `f(input[, info])`.
    Validate {
        input: Value,
        info: Option<ValidationInfo>,
    },
    /// A `function-wrap` validator: `f(input, handler[, info])`; the handler runs the wrapped
    /// schema.
    ValidateWrap {
        input: Value,
        handler: &'a mut dyn ValidatorHandler,
        info: Option<ValidationInfo>,
    },
    /// A plain serializer function: `f([model, ]value[, info])`; `model` is given to field
    /// serializers.
    Serialize {
        value: Value,
        model: Option<Value>,
        info: Option<SerializationInfo>,
    },
    /// A computed field: the value of property `name` of `model` (upstream reads the
    /// property of the Python object; see docs/DIVERGENCES.md #20).
    Property { model: Value, name: String },
    /// A wrap serializer function: `f([model, ]value, handler[, info])`; the handler runs the
    /// wrapped serializer.
    SerializeWrap {
        value: Value,
        model: Option<Value>,
        handler: &'a mut dyn SerializerHandler,
        info: Option<SerializationInfo>,
    },
}

/// What upstream passes to validator functions that take `info` (`ValidationInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationInfo {
    /// The schema's config.
    pub config: Option<Dict>,
    /// The `context` given to the validation call.
    pub context: Option<Value>,
    /// The fields validated so far, for field validators of models and typed dicts.
    pub data: Option<Dict>,
    /// The field being validated.
    pub field_name: Option<String>,
    /// `python` or `json`, as the input was given (upstream's `mode`).
    pub mode: InputType,
}

/// What upstream passes to serializer functions that take `info` (`SerializationInfo`).
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SerializationInfo {
    pub include: Option<Value>,
    pub exclude: Option<Value>,
    pub context: Option<Value>,
    /// `python`, `json` or another mode name given to `to_python`.
    pub mode: crate::serializers::SerMode,
    pub by_alias: Option<bool>,
    pub exclude_unset: bool,
    pub exclude_defaults: bool,
    pub exclude_none: bool,
    pub exclude_computed_fields: bool,
    pub round_trip: bool,
    pub serialize_as_any: bool,
    /// The field being serialized, for field serializers.
    pub field_name: Option<String>,
}

/// The `handler` of a wrap serializer: serializes a value with the wrapped schema (always to
/// Python values, as upstream's `SerializationCallable`).
pub trait SerializerHandler {
    /// Serialize `value`. With `index_key` (a list index or dict key), `include` / `exclude`
    /// apply to that position, and a value they filter out raises `HostError::Omit`.
    fn serialize(&mut self, value: Value, index_key: Option<Value>) -> Result<Value, HostError>;
}

/// The `handler` of a wrap validator: validates a value with the wrapped schema.
pub trait ValidatorHandler {
    /// Validate `input`; errors are located under `outer_location` when given.
    fn validate(
        &mut self,
        input: Value,
        outer_location: Option<LocItem>,
    ) -> Result<Value, HostError>;
}

/// What a host function raised.
#[derive(Debug, Clone)]
pub enum HostError {
    /// A `ValueError`: reported as a `value_error` with the message.
    Value(String),
    /// An `AssertionError`: reported as an `assertion_error` with the message.
    Assertion(String),
    /// A `PydanticCustomError`: an error with its own type and message template.
    Custom {
        error_type: String,
        message_template: String,
        context: Option<Dict>,
    },
    /// A `PydanticKnownError`: one of the built-in error types.
    Known(ErrorType),
    /// A `ValidationError`, e.g. from a wrap validator's handler, re-raised.
    Validation(ValidationError),
    /// `PydanticOmit`: leave the item out.
    Omit,
    /// `PydanticUseDefault`: use the field's default.
    UseDefault,
    /// A failure of the core itself inside the call (e.g. a schema error in the handler).
    Core(CoreError),
    /// A serialization error (`PydanticSerializationError`,
    /// `PydanticSerializationUnexpectedValue`), e.g. from a wrap serializer's handler.
    Serialization(crate::serializers::SerializeError),
    /// Any other exception: it aborts validation and reaches the caller unchanged.
    Other(HostException),
}

/// An exception of the host language that passes through the core unchanged, e.g. a Perl
/// exception object raised by a validator function.
#[derive(Clone)]
pub struct HostException {
    message: String,
    payload: Arc<dyn Any + Send + Sync>,
}

impl HostException {
    /// `message` is what the core reports when it has to show the exception as text.
    pub fn new(message: impl Into<String>, payload: impl Any + Send + Sync) -> Self {
        Self {
            message: message.into(),
            payload: Arc::new(payload),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// The host's own exception, for the host to raise again.
    pub fn payload(&self) -> &(dyn Any + Send + Sync) {
        &*self.payload
    }
}

impl fmt::Debug for HostException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HostException({:?})", self.message)
    }
}

impl fmt::Display for HostException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl PartialEq for HostException {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.payload, &other.payload)
    }
}

impl Eq for HostException {}

impl HostError {
    /// Port of upstream `convert_err`: what the raised exception means for validation of
    /// `input`.
    pub(crate) fn into_val_error(self, input: &Value) -> ValError {
        use crate::errors::ValLineError;
        let line = |error_type| ValError::LineErrors(vec![ValLineError::new(error_type, input)]);
        match self {
            Self::Value(message) => line(ErrorType::ValueError {
                error: Some(message),
                context: None,
            }),
            Self::Assertion(message) => line(ErrorType::AssertionError {
                error: Some(message),
                context: None,
            }),
            Self::Custom {
                error_type,
                message_template,
                context,
            } => line(ErrorType::new_custom_error(
                error_type,
                message_template,
                context,
            )),
            Self::Known(error_type) => line(error_type),
            Self::Validation(error) => ValError::LineErrors(error.into_line_errors()),
            Self::Omit => ValError::Omit,
            Self::UseDefault => ValError::UseDefault,
            Self::Core(error) => ValError::InternalErr(error),
            Self::Other(exception) => ValError::InternalErr(CoreError::Host(exception)),
            Self::Serialization(error) => ValError::InternalErr(match error {
                crate::serializers::SerializeError::Core(error) => error,
                other => CoreError::Value(other.py_display()),
            }),
        }
    }
}

/// Python's `str()` of the raised exception with its class, as `{err}` shows a `PyErr`.
impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(message) => write!(f, "ValueError: {message}"),
            Self::Assertion(message) => write!(f, "AssertionError: {message}"),
            Self::Custom {
                error_type,
                message_template,
                context,
            } => {
                let error =
                    ErrorType::new_custom_error(error_type, message_template, context.clone());
                write!(
                    f,
                    "PydanticCustomError: {}",
                    error
                        .render_message(InputType::Python)
                        .unwrap_or_else(|_| message_template.clone())
                )
            }
            Self::Known(error_type) => write!(
                f,
                "PydanticKnownError: {}",
                error_type
                    .render_message(InputType::Python)
                    .unwrap_or_default()
            ),
            Self::Validation(error) => write!(f, "ValidationError: {error}"),
            Self::Omit => f.write_str("PydanticOmit: PydanticOmit()"),
            Self::UseDefault => f.write_str("PydanticUseDefault: PydanticUseDefault()"),
            Self::Core(error) => write!(f, "{}: {error}", error.kind().python_name()),
            Self::Serialization(error) => f.write_str(&error.py_display()),
            Self::Other(exception) => f.write_str(exception.message()),
        }
    }
}
