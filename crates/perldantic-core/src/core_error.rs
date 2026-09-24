//! `CoreError` replaces `PyErr` in the Python-free core.
//!
//! Each variant corresponds to the Python exception pydantic-core would raise, so a host
//! binding can map it onto its own exception class.

/// Result type used wherever upstream returns `PyResult`.
pub type CoreResult<T> = Result<T, CoreError>;

/// A non-validation failure: bad schema, bad arguments or an internal fault.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// Upstream `TypeError`: a value of the wrong type was passed to the core.
    #[error("{0}")]
    Type(String),
    /// Upstream `ValueError`: a value of the right type but invalid content.
    #[error("{0}")]
    Value(String),
    /// Upstream `KeyError`: an unknown name was looked up. Holds the key; like Python,
    /// `Display` shows its repr.
    #[error("{}", crate::value::Value::Str(.0.clone()).repr())]
    Key(String),
    /// Upstream `UnicodeDecodeError` (a `ValueError`): bytes that are not valid UTF-8.
    #[error("{0}")]
    UnicodeDecode(String),
    /// Upstream `SchemaError`: the core schema is invalid or misused.
    #[error("{0}")]
    Schema(String),
    /// A fault inside the core or at the host boundary.
    #[error("{0}")]
    Internal(String),
    /// An exception raised by a host function (e.g. a validator) that is not a validation
    /// failure; the host raises it again unchanged, as Python propagates it.
    #[error("{0}")]
    Host(crate::host::HostException),
}

/// The kind of a [`CoreError`], without its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreErrorKind {
    Type,
    Value,
    Key,
    UnicodeDecode,
    Schema,
    Internal,
    Host,
}

impl CoreErrorKind {
    /// Name of the exception class pydantic-core raises for this kind.
    pub fn python_name(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            Self::Value => "ValueError",
            Self::Key => "KeyError",
            Self::UnicodeDecode => "UnicodeDecodeError",
            Self::Schema => "SchemaError",
            Self::Internal => "InternalError",
            // the host's own exception; it has no pydantic-core class
            Self::Host => "HostException",
        }
    }
}

impl CoreError {
    pub fn kind(&self) -> CoreErrorKind {
        match self {
            Self::Type(_) => CoreErrorKind::Type,
            Self::Value(_) => CoreErrorKind::Value,
            Self::Key(_) => CoreErrorKind::Key,
            Self::UnicodeDecode(_) => CoreErrorKind::UnicodeDecode,
            Self::Schema(_) => CoreErrorKind::Schema,
            Self::Internal(_) => CoreErrorKind::Internal,
            Self::Host(_) => CoreErrorKind::Host,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Type(m)
            | Self::Value(m)
            | Self::Key(m)
            | Self::UnicodeDecode(m)
            | Self::Schema(m)
            | Self::Internal(m) => m,
            Self::Host(exception) => exception.message(),
        }
    }
}
