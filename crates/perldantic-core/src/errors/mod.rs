//! Validation errors. Port of upstream `errors/`.

mod line_error;
mod location;
mod types;
mod validation_error;

pub use self::line_error::{ToErrorValue, ValError, ValLineError, ValResult};
pub use self::location::{LocItem, Location};
pub use self::types::{ErrorType, ErrorTypeDefaults, Number};
pub use self::validation_error::{
    ERROR_URL_PREFIX, ErrorDetails, ErrorsOptions, INCLUDE_URL_ENV, ValidationError,
};
