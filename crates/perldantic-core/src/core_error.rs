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
    /// Upstream `KeyError`: an unknown name was looked up.
    #[error("{0}")]
    Key(String),
    /// Upstream `SchemaError`: the core schema is invalid or misused.
    #[error("{0}")]
    Schema(String),
    /// A fault inside the core or at the host boundary.
    #[error("{0}")]
    Internal(String),
}

/// The kind of a [`CoreError`], without its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreErrorKind {
    Type,
    Value,
    Key,
    Schema,
    Internal,
}

impl CoreErrorKind {
    /// Name of the exception class pydantic-core raises for this kind.
    pub fn python_name(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            Self::Value => "ValueError",
            Self::Key => "KeyError",
            Self::Schema => "SchemaError",
            Self::Internal => "InternalError",
        }
    }
}

impl CoreError {
    pub fn kind(&self) -> CoreErrorKind {
        match self {
            Self::Type(_) => CoreErrorKind::Type,
            Self::Value(_) => CoreErrorKind::Value,
            Self::Key(_) => CoreErrorKind::Key,
            Self::Schema(_) => CoreErrorKind::Schema,
            Self::Internal(_) => CoreErrorKind::Internal,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Type(m) | Self::Value(m) | Self::Key(m) | Self::Schema(m) | Self::Internal(m) => {
                m
            }
        }
    }
}
