//! The public validation error. Port of upstream `errors/validation_exception.rs`.

use std::fmt::{self, Write as _};

use serde::Serialize;

use crate::core_error::{CoreError, CoreResult};
use crate::input::InputType;
use crate::tools::write_truncated_to_limited_bytes;
use crate::value::Value;

use super::line_error::{ValError, ValLineError};
use super::location::LocItem;
use super::types::ErrorType;

/// Error documentation URL prefix.
///
/// Upstream embeds the pydantic `major.minor` version; the core has no pydantic version, so it
/// uses upstream's own fallback, `latest`.
pub const ERROR_URL_PREFIX: &str = "https://errors.pydantic.dev/latest/v/";

/// Environment variable that disables URLs in the `Display` output when set to anything other
/// than `1` or `true` (mirrors `PYDANTIC_ERRORS_INCLUDE_URL`).
pub const INCLUDE_URL_ENV: &str = "PERLDANTIC_ERRORS_INCLUDE_URL";

/// Which optional fields `errors()` / `to_json()` include; all default to `true` as in pydantic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorsOptions {
    pub include_url: bool,
    pub include_context: bool,
    pub include_input: bool,
}

impl Default for ErrorsOptions {
    fn default() -> Self {
        Self {
            include_url: true,
            include_context: true,
            include_input: true,
        }
    }
}

/// One entry of `ValidationError::errors()`, serialized with pydantic's key order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorDetails {
    #[serde(rename = "type")]
    pub type_: String,
    pub loc: Vec<LocItem>,
    pub msg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// All validation failures of one validation call.
#[derive(Debug, Clone)]
pub struct ValidationError {
    line_errors: Vec<ValLineError>,
    title: String,
    input_type: InputType,
    hide_input: bool,
}

impl ValidationError {
    pub fn new(
        title: impl Into<String>,
        line_errors: Vec<ValLineError>,
        input_type: InputType,
        hide_input: bool,
    ) -> Self {
        Self {
            line_errors,
            title: title.into(),
            input_type,
            hide_input,
        }
    }

    /// Wrap the error half of a validation result; non-validation errors are passed through.
    pub fn from_val_error(
        title: impl Into<String>,
        error: ValError,
        input_type: InputType,
        hide_input: bool,
    ) -> CoreResult<Self> {
        match error {
            ValError::LineErrors(line_errors) => {
                Ok(Self::new(title, line_errors, input_type, hide_input))
            }
            ValError::InternalErr(err) => Err(err),
            ValError::Omit => Err(CoreError::Schema(
                "Uncaught Omit error, please check your usage of `default` validators.".into(),
            )),
            ValError::UseDefault => Err(CoreError::Schema(
                "Uncaught `PydanticUseDefault` exception: the error was raised in a field validator and no default value is available for that field.".into(),
            )),
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn error_count(&self) -> usize {
        self.line_errors.len()
    }

    pub fn line_errors(&self) -> &[ValLineError] {
        &self.line_errors
    }

    pub fn input_type(&self) -> InputType {
        self.input_type
    }

    /// Structured errors, like pydantic's `ValidationError.errors()`.
    ///
    /// A message that cannot be rendered (a malformed custom error context) is reported
    /// in place of the message rather than failing the whole call.
    pub fn errors(&self, options: &ErrorsOptions) -> Vec<ErrorDetails> {
        self.line_errors
            .iter()
            .map(|line| {
                let is_custom = matches!(line.error_type, ErrorType::CustomError { .. });
                ErrorDetails {
                    type_: line.error_type.type_string(),
                    loc: line.location.items(),
                    msg: render_or_explain(&line.error_type, self.input_type),
                    input: options.include_input.then(|| line.input_value.clone()),
                    ctx: if options.include_context {
                        line.error_type.context().map(Value::Dict)
                    } else {
                        None
                    },
                    url: (options.include_url && !is_custom).then(|| error_url(&line.error_type)),
                }
            })
            .collect()
    }

    /// JSON array of [`ErrorDetails`], like pydantic's `ValidationError.json()`.
    pub fn to_json(&self, options: &ErrorsOptions, indent: Option<usize>) -> String {
        let details = self.errors(options);
        match indent {
            None => serde_json::to_string(&details),
            Some(width) => {
                let indent = " ".repeat(width);
                let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
                let mut out = Vec::new();
                let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
                details
                    .serialize(&mut ser)
                    .map(|()| String::from_utf8(out).expect("serde_json writes UTF-8"))
            }
        }
        .expect("error details always serialize")
    }

    /// Human-readable report, like pydantic's `str(ValidationError)`.
    pub fn display(&self, include_url: bool, hide_input: bool) -> String {
        let count = self.line_errors.len();
        let plural = if count == 1 { "" } else { "s" };
        let lines: Vec<String> = self
            .line_errors
            .iter()
            .map(|line| self.pretty_line(line, include_url, hide_input))
            .collect();
        format!(
            "{count} validation error{plural} for {}\n{}",
            self.title,
            lines.join("\n")
        )
    }

    fn pretty_line(&self, line: &ValLineError, include_url: bool, hide_input: bool) -> String {
        let mut out = String::with_capacity(200);
        write!(out, "{}", line.location).unwrap();
        let message = render_or_explain(&line.error_type, self.input_type);
        write!(out, "  {message} [type={}", line.error_type.type_string()).unwrap();
        // There is no meaningful input for errors raised before a default factory runs.
        if !hide_input && !matches!(line.error_type, ErrorType::DefaultFactoryNotCalled { .. }) {
            out.push_str(", input_value=");
            write_truncated_to_limited_bytes(&mut out, &line.input_value.repr(), 50).unwrap();
            write!(out, ", input_type={}", line.input_value.type_name()).unwrap();
        }
        out.push(']');
        if include_url && !matches!(line.error_type, ErrorType::CustomError { .. }) {
            write!(
                out,
                "\n    For further information visit {}",
                error_url(&line.error_type)
            )
            .unwrap();
        }
        out
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display(include_url_env(), self.hide_input))
    }
}

impl std::error::Error for ValidationError {}

fn error_url(error_type: &ErrorType) -> String {
    format!("{ERROR_URL_PREFIX}{}", error_type.type_string())
}

fn render_or_explain(error_type: &ErrorType, input_type: InputType) -> String {
    error_type.render_message(input_type).unwrap_or_else(|err| {
        format!(
            "(error rendering message: {}: {err})",
            err.kind().python_name()
        )
    })
}

fn include_url_env() -> bool {
    std::env::var(INCLUDE_URL_ENV)
        .map_or(true, |val| val == "1" || val.eq_ignore_ascii_case("true"))
}
