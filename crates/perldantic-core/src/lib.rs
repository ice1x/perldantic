//! Python-free port of the pydantic-core validation engine.
//!
//! The upstream sources this crate is ported from are vendored verbatim in
//! `upstream/pydantic-core/`; see `upstream/UPSTREAM.md` for the file map.

mod core_error;
pub mod errors;
pub mod input;
mod tools;
mod value;

pub use core_error::{CoreError, CoreErrorKind, CoreResult};
pub use errors::{
    ErrorDetails, ErrorType, ErrorTypeDefaults, ErrorsOptions, LocItem, Location, Number,
    ToErrorValue, ValError, ValLineError, ValResult, ValidationError,
};
pub use input::InputType;
pub use value::{Dict, Value};

/// Version of this crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Upstream `pydantic/pydantic` commit the port is based on.
pub const UPSTREAM_COMMIT: &str = "0384c970e37a59b344e75161eb106ea9996378ba";

/// `pydantic-core` version at [`UPSTREAM_COMMIT`].
pub const UPSTREAM_VERSION: &str = "2.49.0";
