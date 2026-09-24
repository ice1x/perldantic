//! Python-free port of the pydantic-core validation engine.
//!
//! The upstream sources this crate is ported from are vendored verbatim in
//! `upstream/pydantic-core/`; see `upstream/UPSTREAM.md` for the file map.

pub(crate) mod build_tools;
mod core_error;
pub(crate) mod definitions;
pub mod errors;
pub mod input;
pub mod json_schema;
mod lookup_key;
pub(crate) mod recursion_guard;
pub mod serializers;
pub mod temporal;
mod tools;
pub(crate) mod validators;
mod value;

pub use build_tools::ExtraBehavior;
pub use core_error::{CoreError, CoreErrorKind, CoreResult};
pub use errors::{
    ErrorDetails, ErrorType, ErrorTypeDefaults, ErrorsOptions, LocItem, Location, Number,
    ToErrorValue, ValError, ValLineError, ValResult, ValidationError,
};
pub use input::InputType;
pub use jiter::PartialMode;
pub use json_schema::{
    GeneratedJsonSchema, JsonSchemaError, JsonSchemaMode, JsonSchemaOptions, UnionFormat,
    generate_json_schema,
};
pub use serializers::{
    JsonOptions, SchemaSerializer, SerMode, SerializeError, SerializeOptions, Serialized,
    UnexpectedValue, WarningsMode,
};
pub use speedate;
pub use uuid;
pub use validators::{SchemaValidator, ValidateError, ValidateOptions};
pub use value::{Dict, Model, Value};

/// Version of this crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Upstream `pydantic/pydantic` commit the port is based on.
pub const UPSTREAM_COMMIT: &str = "0384c970e37a59b344e75161eb106ea9996378ba";

/// `pydantic-core` version at [`UPSTREAM_COMMIT`].
pub const UPSTREAM_VERSION: &str = "2.49.0";
