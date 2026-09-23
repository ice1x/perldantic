//! Validators. Port of upstream `validators/`.

pub(crate) mod config;
pub(crate) mod validation_state;

use crate::build_tools::ExtraBehavior;
use crate::input::InputType;
use crate::value::Value;

/// Settings that stay constant for one validation call.
#[derive(Debug, Clone)]
pub struct Extra<'a> {
    /// Validation mode
    pub input_type: InputType,
    /// Whether we're in strict or lax mode
    pub strict: Option<bool>,
    /// Whether to ignore, allow, or forbid extra data during model validation
    #[allow(clippy::struct_field_names)]
    pub extra_behavior: Option<ExtraBehavior>,
    /// Validation-time setting of `from_attributes`
    pub from_attributes: Option<bool>,
    /// Context passed to validator functions
    pub context: Option<&'a Value>,
    /// Whether to use the field's alias to match input data to an attribute
    by_alias: Option<bool>,
    /// Whether to use the field's name to match input data to an attribute
    by_name: Option<bool>,
}

impl<'a> Extra<'a> {
    pub fn new(
        strict: Option<bool>,
        extra_behavior: Option<ExtraBehavior>,
        from_attributes: Option<bool>,
        context: Option<&'a Value>,
        input_type: InputType,
        by_alias: Option<bool>,
        by_name: Option<bool>,
    ) -> Self {
        Extra {
            input_type,
            strict,
            extra_behavior,
            from_attributes,
            context,
            by_alias,
            by_name,
        }
    }
}
