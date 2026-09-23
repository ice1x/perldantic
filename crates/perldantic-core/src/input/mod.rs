//! Validation inputs. Port of upstream `input/`.

mod input_abstract;
mod input_json;
mod input_value;
mod return_enums;
mod shared;

pub use input_abstract::{
    BorrowInput, ConsumeIterator, Input, Never, ValMatch, ValidatedDict, ValidatedList,
    ValidatedTuple,
};
pub use return_enums::{EitherBytes, EitherFloat, EitherInt, EitherString, Int, ValidationMatch};
pub(crate) use return_enums::{MaxLengthCheck, no_validator_iter_to_vec, validate_iter_to_vec};

/// The kind of input being validated; selects message wording (e.g. "None" vs "null").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputType {
    /// Host-native data (upstream: Python objects).
    Python,
    /// JSON text.
    Json,
    /// String-only data such as environment variables.
    String,
}

impl InputType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Json => "json",
            Self::String => "string",
        }
    }
}

impl TryFrom<&str> for InputType {
    type Error = crate::CoreError;

    fn try_from(error_mode: &str) -> Result<Self, Self::Error> {
        match error_mode {
            "python" => Ok(Self::Python),
            "json" => Ok(Self::Json),
            "string" => Ok(Self::String),
            s => Err(crate::CoreError::Value(format!("Invalid error mode: {s}"))),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod input_type_tests {
    use super::InputType;
    use crate::CoreError;

    #[test]
    fn round_trips_names() {
        for t in [InputType::Python, InputType::Json, InputType::String] {
            assert_eq!(InputType::try_from(t.as_str()), Ok(t));
        }
    }

    #[test]
    fn rejects_unknown_mode() {
        assert_eq!(
            InputType::try_from("yaml"),
            Err(CoreError::Value("Invalid error mode: yaml".into()))
        );
    }
}
