//! `default` schema. Port of upstream `validators/with_default.rs`.
//!
//! `default_factory` needs host callbacks, which do not exist yet; such schemas are rejected
//! while building. Defaults are plain values, so they are always copied, which is what upstream
//! does for unhashable (mutable) defaults.

use std::sync::Arc;

use crate::build_tools::{SchemaDict, schema_err, schema_or_config_same};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{LocItem, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, build_validator};

#[derive(Debug, Clone)]
pub enum DefaultType {
    None,
    Default(Value),
}

impl DefaultType {
    pub fn new(schema: &Dict) -> CoreResult<Self> {
        match (
            schema.get_as::<Value>("default")?,
            schema.get_as::<Value>("default_factory")?,
        ) {
            (Some(_), Some(_)) => {
                schema_err!("'default' and 'default_factory' cannot be used together")
            }
            (Some(default), None) => Ok(Self::Default(default)),
            (None, Some(_)) => schema_err!(
                "`default_factory` is not supported yet: host callbacks are not implemented"
            ),
            (None, None) => Ok(Self::None),
        }
    }

    pub fn default_value(&self) -> Option<Value> {
        match self {
            Self::Default(default) => Some(default.clone()),
            Self::None => None,
        }
    }
}

#[derive(Debug, Clone)]
enum OnError {
    Raise,
    Omit,
    Default,
}

/// The inner schema, with a default for missing input and optionally for invalid input.
#[derive(Debug)]
pub struct WithDefaultValidator {
    default: DefaultType,
    on_error: OnError,
    validator: Arc<CombinedValidator>,
    validate_default: bool,
    name: String,
}

impl BuildValidator for WithDefaultValidator {
    const EXPECTED_TYPE: &'static str = "default";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let default = DefaultType::new(schema)?;
        let on_error = match schema.get_as::<String>("on_error")?.as_deref() {
            Some("raise") | None => OnError::Raise,
            Some("omit") => OnError::Omit,
            Some("default") => {
                if matches!(default, DefaultType::None) {
                    return schema_err!(
                        "'on_error = default' requires a `default` or `default_factory`"
                    );
                }
                OnError::Default
            }
            // Upstream relies on its schema validation here and panics otherwise
            // (docs/DIVERGENCES.md #11).
            Some(other) => {
                return schema_err!(
                    "`on_error` should be 'raise', 'omit' or 'default', got '{other}'"
                );
            }
        };

        let sub_schema: Dict = schema.get_as_req("schema")?;
        let validator = build_validator(&sub_schema, config, definitions)?;

        let name = format!("{}[{}]", Self::EXPECTED_TYPE, validator.get_name());

        Ok(Arc::new(CombinedValidator::WithDefault(Self {
            default,
            on_error,
            validator,
            validate_default: schema_or_config_same(schema, config, "validate_default")?
                .unwrap_or(false),
            name,
        })))
    }
}

impl Validator for WithDefaultValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        match self.validator.validate(input, state) {
            Ok(v) => Ok(v),
            Err(e) => match e {
                ValError::UseDefault => Ok(self.default_value(None::<usize>, state)?.ok_or(e)?),
                e => match self.on_error {
                    OnError::Raise => Err(e),
                    OnError::Default => Ok(self.default_value(None::<usize>, state)?.ok_or(e)?),
                    OnError::Omit => Err(ValError::Omit),
                },
            },
        }
    }

    fn default_value(
        &self,
        outer_loc: Option<impl Into<LocItem>>,
        state: &mut ValidationState<'_>,
    ) -> ValResult<Option<Value>> {
        match self.default.default_value() {
            Some(dft) => {
                if self.validate_default {
                    match self.validate_default_value(&dft, state) {
                        Ok(v) => Ok(Some(v)),
                        Err(e) => {
                            if let Some(outer_loc) = outer_loc {
                                Err(e.with_outer_location(outer_loc))
                            } else {
                                Err(e)
                            }
                        }
                    }
                } else {
                    Ok(Some(dft))
                }
            }
            None => Ok(None),
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

impl WithDefaultValidator {
    pub fn has_default(&self) -> bool {
        !matches!(self.default, DefaultType::None)
    }

    pub fn omit_on_error(&self) -> bool {
        matches!(self.on_error, OnError::Omit)
    }

    /// Validate the default itself. Upstream runs the full `validate`, so an invalid default
    /// with `on_error='default'` recurses until the stack overflows; here the default's own
    /// error is reported instead (docs/DIVERGENCES.md #10).
    fn validate_default_value(
        &self,
        dft: &Value,
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        self.validator
            .validate(dft, state)
            .map_err(|e| match self.on_error {
                OnError::Omit if !matches!(e, ValError::UseDefault) => ValError::Omit,
                _ => e,
            })
    }
}
