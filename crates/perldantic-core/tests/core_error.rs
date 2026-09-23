//! `CoreError` replaces `PyErr`: each variant maps to the Python exception class pydantic
//! would raise, so the host layer can raise the matching exception object.

use perldantic_core::{CoreError, CoreErrorKind, ValError};

#[test]
fn display_is_the_bare_message() {
    assert_eq!(CoreError::Type("bad".into()).to_string(), "bad");
    assert_eq!(
        CoreError::Schema("bad schema".into()).to_string(),
        "bad schema"
    );
}

#[test]
fn kind_names_the_equivalent_python_exception() {
    let cases = [
        (
            CoreError::Type(String::new()),
            CoreErrorKind::Type,
            "TypeError",
        ),
        (
            CoreError::Value(String::new()),
            CoreErrorKind::Value,
            "ValueError",
        ),
        (
            CoreError::Key(String::new()),
            CoreErrorKind::Key,
            "KeyError",
        ),
        (
            CoreError::Schema(String::new()),
            CoreErrorKind::Schema,
            "SchemaError",
        ),
        (
            CoreError::Internal(String::new()),
            CoreErrorKind::Internal,
            "InternalError",
        ),
    ];
    for (err, kind, name) in cases {
        assert_eq!(err.kind(), kind);
        assert_eq!(kind.python_name(), name);
    }
}

#[test]
fn message_accessor_returns_the_text() {
    assert_eq!(CoreError::Value("v".into()).message(), "v");
}

#[test]
fn converts_into_internal_val_error() {
    let val_error: ValError = CoreError::Type("t".into()).into();
    assert!(matches!(val_error, ValError::InternalErr(CoreError::Type(ref m)) if m == "t"));
}
