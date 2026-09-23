//! State and options of one serialization call. Port of upstream `serializers/extra.rs`.

use std::fmt;
use std::ops::{Deref, DerefMut};

use serde::ser::Error;

use super::config::SerializationConfig;
use super::errors::{SerResult, SerializeError, UNEXPECTED_TYPE_SER_MARKER, UnexpectedValue};
use crate::recursion_guard::{
    ContainsRecursionState, RecursionError, RecursionGuard, RecursionState,
};
use crate::value::Value;

/// Mutable state threaded through serializers.
pub(crate) struct SerializationState {
    pub warnings: CollectWarnings,
    pub rec_guard: RecursionState,
    pub config: SerializationConfig,
    field_name: Option<String>,
    pub check: SerCheck,
    pub include_exclude: IncludeExclude,
    pub extra: Extra,
}

/// The `include` / `exclude` filters applying at the current position.
#[derive(Debug, Clone, Default)]
pub(crate) struct IncludeExclude {
    pub include: Option<Value>,
    pub exclude: Option<Value>,
}

impl IncludeExclude {
    pub fn new(include: Option<Value>, exclude: Option<Value>) -> Self {
        Self { include, exclude }
    }

    pub fn empty() -> Self {
        Self::default()
    }
}

impl SerializationState {
    pub fn new(
        config: SerializationConfig,
        warnings_mode: WarningsMode,
        include: Option<Value>,
        exclude: Option<Value>,
        extra: Extra,
    ) -> Self {
        Self {
            warnings: CollectWarnings::new(warnings_mode),
            rec_guard: RecursionState::default(),
            config,
            field_name: None,
            check: SerCheck::None,
            include_exclude: IncludeExclude { include, exclude },
            extra,
        }
    }

    /// Guard against serializing the same host value with the same serializer again, which
    /// upstream detects for cyclic Python objects.
    pub fn recursion_guard(
        &mut self,
        value: &Value,
        def_ref_id: usize,
    ) -> SerResult<RecursionGuard<'_, Self>> {
        RecursionGuard::new(self, std::ptr::from_ref(value) as usize, def_ref_id).map_err(|e| {
            match e {
                RecursionError::Depth => SerializeError::Core(crate::core_error::CoreError::Value(
                    "Circular reference detected (depth exceeded)".into(),
                )),
                RecursionError::Cyclic => {
                    SerializeError::Core(crate::core_error::CoreError::Value(
                        "Circular reference detected (id repeated)".into(),
                    ))
                }
            }
        })
    }

    pub fn warn_fallback_py(&mut self, field_type: &str, value: &Value) -> SerResult<()> {
        self.warnings
            .on_fallback_py(field_type, value, self.field_name.as_deref(), self.check)
    }

    pub fn warn_fallback_ser<S: serde::ser::Serializer>(
        &mut self,
        field_type: &str,
        value: &Value,
    ) -> Result<(), S::Error> {
        self.warnings.on_fallback_ser::<S>(
            field_type,
            value,
            self.field_name.as_deref(),
            self.check,
        )
    }

    pub fn scoped_set<'state, P, T>(
        &'state mut self,
        projector: P,
        new_value: T,
    ) -> ScopedSetState<'state, P, T>
    where
        P: for<'p> Fn(&'p mut Self) -> &'p mut T,
    {
        let value = std::mem::replace((projector)(self), new_value);
        ScopedSetState {
            state: self,
            projector,
            value,
        }
    }

    pub fn scoped_include_exclude(
        &mut self,
        next_include_exclude: IncludeExclude,
    ) -> ScopedIncludeExcludeState<'_> {
        self.scoped_set(Self::include_exclude_mut, next_include_exclude)
    }

    pub fn include(&self) -> Option<&Value> {
        self.include_exclude.include.as_ref()
    }

    pub fn exclude(&self) -> Option<&Value> {
        self.include_exclude.exclude.as_ref()
    }

    fn include_exclude_mut(&mut self) -> &mut IncludeExclude {
        &mut self.include_exclude
    }
}

impl ContainsRecursionState for SerializationState {
    fn access_recursion_state<R>(&mut self, f: impl FnOnce(&mut RecursionState) -> R) -> R {
        f(&mut self.rec_guard)
    }
}

/// Options of one serialization call that stay the same throughout it.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
// Temporary: the field options are read by the model serializers (task 00033).
#[allow(dead_code)]
pub(crate) struct Extra {
    pub mode: SerMode,
    pub by_alias: Option<bool>,
    pub exclude_unset: bool,
    pub exclude_defaults: bool,
    pub exclude_none: bool,
    pub exclude_computed_fields: bool,
    pub round_trip: bool,
    pub serialize_unknown: bool,
    pub serialize_as_any: bool,
    pub polymorphic_serialization: Option<bool>,
    pub context: Option<Value>,
}

impl Extra {
    // Temporary: used by the model serializers (task 00033).
    #[allow(dead_code)]
    pub fn serialize_by_alias_or(&self, serialize_by_alias: Option<bool>) -> bool {
        self.by_alias.or(serialize_by_alias).unwrap_or(false)
    }
}

/// Whether values are checked against the serializer's type, as union serializers do when
/// looking for the matching member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SerCheck {
    None,
    Strict,
    Lax,
}

impl SerCheck {
    pub fn enabled(self) -> bool {
        self != SerCheck::None
    }
}

/// `python`, `json`, or any other string (passed through to serializer functions).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SerMode {
    #[default]
    Python,
    Json,
    Other(String),
}

impl fmt::Display for SerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SerMode::Python => write!(f, "python"),
            SerMode::Json => write!(f, "json"),
            SerMode::Other(s) => write!(f, "{s}"),
        }
    }
}

impl SerMode {
    pub fn is_json(&self) -> bool {
        matches!(self, SerMode::Json)
    }
}

impl From<Option<&str>> for SerMode {
    fn from(s: Option<&str>) -> Self {
        match s {
            Some("json") => SerMode::Json,
            Some("python") | None => SerMode::Python,
            Some(other) => SerMode::Other(other.to_string()),
        }
    }
}

/// What to do with values that do not match their serializer.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum WarningsMode {
    None,
    /// Return a warning with the output (upstream emits a `UserWarning`).
    #[default]
    Warn,
    /// Fail with a serialization error.
    Error,
}

impl From<bool> for WarningsMode {
    fn from(mode: bool) -> Self {
        if mode { Self::Warn } else { Self::None }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CollectWarnings {
    mode: WarningsMode,
    warnings: Vec<UnexpectedValue>,
}

impl CollectWarnings {
    pub(crate) fn new(mode: WarningsMode) -> Self {
        Self {
            mode,
            warnings: Vec::new(),
        }
    }

    pub fn register_warning(&mut self, warning: UnexpectedValue) {
        if self.mode != WarningsMode::None {
            self.warnings.push(warning);
        }
    }

    fn on_fallback_py(
        &mut self,
        field_type: &str,
        value: &Value,
        field_name: Option<&str>,
        check: SerCheck,
    ) -> SerResult<()> {
        // special case for None as it's very common e.g. as a default value
        if matches!(value, Value::None) {
            Ok(())
        } else if check.enabled() {
            Err(SerializeError::UnexpectedValue(
                UnexpectedValue::new_from_parts(
                    field_name.map(str::to_owned),
                    Some(field_type.to_string()),
                    Some(value.clone()),
                ),
            ))
        } else {
            self.fallback_warning(field_name, field_type, value);
            Ok(())
        }
    }

    pub fn on_fallback_ser<S: serde::ser::Serializer>(
        &mut self,
        field_type: &str,
        value: &Value,
        field_name: Option<&str>,
        check: SerCheck,
    ) -> Result<(), S::Error> {
        // special case for None as it's very common e.g. as a default value
        if matches!(value, Value::None) {
            Ok(())
        } else if check.enabled() {
            // note: I think this should never actually happen since we use `to_python(..., mode='json')` during
            // JSON serialization to "try" union branches, but it's here for completeness/correctness
            // in particular, in future we could allow errors instead of warnings on fallback
            Err(S::Error::custom(UNEXPECTED_TYPE_SER_MARKER))
        } else {
            self.fallback_warning(field_name, field_type, value);
            Ok(())
        }
    }

    fn fallback_warning(&mut self, field_name: Option<&str>, field_type: &str, value: &Value) {
        if self.mode != WarningsMode::None {
            self.register_warning(UnexpectedValue::new_from_parts(
                field_name.map(str::to_owned),
                Some(field_type.to_string()),
                Some(value.clone()),
            ));
        }
    }

    /// The warning to report at the end of the call, or the error in `error` mode.
    pub fn final_check(&self) -> SerResult<Option<String>> {
        if self.mode == WarningsMode::None || self.warnings.is_empty() {
            return Ok(None);
        }
        let formatted_warnings: Vec<String> =
            self.warnings.iter().map(UnexpectedValue::repr).collect();
        let message = format!(
            "Pydantic serializer warnings:\n  {}",
            formatted_warnings.join("\n  ")
        );
        if self.mode == WarningsMode::Warn {
            Ok(Some(message))
        } else {
            Err(SerializeError::Serialization(message))
        }
    }
}

pub(crate) struct ScopedSetState<'scope, P, T>
where
    P: for<'p> Fn(&'p mut SerializationState) -> &'p mut T,
{
    state: &'scope mut SerializationState,
    projector: P,
    /// The previous value, restored on drop.
    value: T,
}

impl<P, T> Drop for ScopedSetState<'_, P, T>
where
    P: for<'p> Fn(&'p mut SerializationState) -> &'p mut T,
{
    fn drop(&mut self) {
        std::mem::swap((self.projector)(self.state), &mut self.value);
    }
}

impl<P, T> Deref for ScopedSetState<'_, P, T>
where
    P: for<'p> Fn(&'p mut SerializationState) -> &'p mut T,
{
    type Target = SerializationState;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}

impl<P, T> DerefMut for ScopedSetState<'_, P, T>
where
    P: for<'p> Fn(&'p mut SerializationState) -> &'p mut T,
{
    fn deref_mut(&mut self) -> &mut SerializationState {
        self.state
    }
}

type ScopedSetStateT<'scope, T> =
    ScopedSetState<'scope, for<'s> fn(&'s mut SerializationState) -> &'s mut T, T>;

pub(crate) type ScopedIncludeExcludeState<'scope> = ScopedSetStateT<'scope, IncludeExclude>;
