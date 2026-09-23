//! Line errors collected during validation. Port of upstream `errors/line_error.rs`.

use crate::core_error::CoreError;
use crate::value::Value;

use super::location::{LocItem, Location};
use super::types::ErrorType;

pub type ValResult<T> = Result<T, ValError>;

/// Anything that can be recorded as the `input` of an error.
pub trait ToErrorValue {
    fn to_error_value(&self) -> Value;
}

impl ToErrorValue for Value {
    fn to_error_value(&self) -> Value {
        self.clone()
    }
}

impl<T: ToErrorValue + ?Sized> ToErrorValue for &T {
    fn to_error_value(&self) -> Value {
        (**self).to_error_value()
    }
}

/// The error half of a validation result.
#[derive(Debug)]
pub enum ValError {
    /// One or more validation failures.
    LineErrors(Vec<ValLineError>),
    /// A non-validation failure that aborts validation.
    InternalErr(CoreError),
    /// Omit this item (raised by `default` validators with `on_error='omit'`).
    Omit,
    /// Use the field default (raised from field validators).
    UseDefault,
}

impl From<CoreError> for ValError {
    fn from(err: CoreError) -> Self {
        Self::InternalErr(err)
    }
}

impl From<Vec<ValLineError>> for ValError {
    fn from(line_errors: Vec<ValLineError>) -> Self {
        Self::LineErrors(line_errors)
    }
}

impl ValError {
    pub fn new(error_type: ErrorType, input: impl ToErrorValue) -> Self {
        Self::LineErrors(vec![ValLineError::new(error_type, input)])
    }

    pub fn new_with_loc(
        error_type: ErrorType,
        input: impl ToErrorValue,
        loc: impl Into<LocItem>,
    ) -> Self {
        Self::LineErrors(vec![ValLineError::new_with_loc(error_type, input, loc)])
    }

    /// Prefix every line error's location with an outer item.
    #[must_use]
    pub fn with_outer_location(self, into_loc_item: impl Into<LocItem>) -> Self {
        let loc_item = into_loc_item.into();
        match self {
            Self::LineErrors(mut line_errors) => {
                for line_error in &mut line_errors {
                    line_error.location.with_outer(loc_item.clone());
                }
                Self::LineErrors(line_errors)
            }
            other => other,
        }
    }
}

/// A single validation failure: what went wrong, where, and the offending input.
#[derive(Debug, Clone)]
pub struct ValLineError {
    pub error_type: ErrorType,
    /// Reversed; see [`Location`].
    pub location: Location,
    pub input_value: Value,
}

impl ValLineError {
    pub fn new(error_type: ErrorType, input: impl ToErrorValue) -> Self {
        Self {
            error_type,
            input_value: input.to_error_value(),
            location: Location::default(),
        }
    }

    pub fn new_with_loc(
        error_type: ErrorType,
        input: impl ToErrorValue,
        loc: impl Into<LocItem>,
    ) -> Self {
        Self {
            error_type,
            input_value: input.to_error_value(),
            location: Location::new_some(loc.into()),
        }
    }

    pub fn new_with_full_loc(
        error_type: ErrorType,
        input: impl ToErrorValue,
        location: Location,
    ) -> Self {
        Self {
            error_type,
            input_value: input.to_error_value(),
            location,
        }
    }

    #[must_use]
    pub fn with_outer_location(mut self, into_loc_item: impl Into<LocItem>) -> Self {
        self.location.with_outer(into_loc_item.into());
        self
    }

    #[must_use]
    pub fn with_type(mut self, error_type: ErrorType) -> Self {
        self.error_type = error_type;
        self
    }

    /// The outermost location item, if any.
    pub fn first_loc_item(&self) -> Option<&LocItem> {
        match &self.location {
            Location::Empty => None,
            Location::List(loc_items) => loc_items.last(),
        }
    }
}
