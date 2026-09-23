//! `union` and `tagged-union` serializers. Port of upstream `type_serializers/union.rs`.
//!
//! A union tries its members in order, first requiring each value to match its member exactly
//! (strict check), then allowing subclasses (lax check), and otherwise falls back to inference
//! with warnings.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::lookup_key::{LookupPath, validation_alias_paths};
use crate::serializers::errors::{SerResult, SerializeError, UnexpectedValue};
use crate::serializers::extra::{IncludeExclude, SerCheck, SerializationState};
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::validators::as_dict;
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct UnionSerializer {
    choices: UnionChoices,
    name: String,
}

impl BuildSerializer for UnionSerializer {
    const EXPECTED_TYPE: &'static str = "union";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let choices = schema
            .get_as_req::<Vec<Value>>("choices")?
            .iter()
            .map(|choice| {
                // a `(schema, label)` choice
                let choice = match choice {
                    Value::Tuple(items) if !items.is_empty() => &items[0],
                    choice => choice,
                };
                CombinedSerializer::build(as_dict(choice)?, config, definitions)
            })
            .collect::<CoreResult<_>>()?;

        let choices = match UnionChoices::from_choices(choices)? {
            FromChoicesOutput::Single(s) => return Ok(s),
            FromChoicesOutput::Multiple(m) => m,
        };

        let descr = choices
            .choices
            .iter()
            .map(|v| v.get_name())
            .collect::<Vec<_>>()
            .join(", ");

        Ok(Arc::new(CombinedSerializer::Union(Self {
            choices,
            name: format!("Union[{descr}]"),
        })))
    }
}

impl TypeSerializer for UnionSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match self.choices.serialize(
            |comb_serializer, state| comb_serializer.to_python(value, state),
            state,
        )? {
            Some(v) => Ok(v),
            None => infer_to_python(value, state),
        }
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        match self.choices.serialize(
            |comb_serializer, state| comb_serializer.json_key(key, state).map(Cow::into_owned),
            state,
        )? {
            Some(v) => Ok(Cow::Owned(v)),
            None => infer_json_key(key, state),
        }
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        match self.choices.serialize(
            |comb_serializer, state| comb_serializer.to_python(value, state),
            state,
        ) {
            Ok(Some(v)) => {
                let state = &mut state.scoped_include_exclude(IncludeExclude::empty());
                infer_serialize(&v, serializer, state)
            }
            Ok(None) => infer_serialize(value, serializer, state),
            Err(err) => Err(serde::ser::Error::custom(err.py_display())),
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn retry_with_lax_check(&self) -> bool {
        self.choices.retry_with_lax_check()
    }
}

#[derive(Debug)]
pub struct TaggedUnionSerializer {
    discriminator: Vec<LookupPath>,
    lookup: Vec<(Value, usize)>,
    choices: UnionChoices,
    name: OnceLock<String>,
}

impl BuildSerializer for TaggedUnionSerializer {
    const EXPECTED_TYPE: &'static str = "tagged-union";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let discriminator = validation_alias_paths(&schema.get_as_req::<Value>("discriminator")?)?;
        let choices_map: Dict = schema.get_as_req("choices")?;

        let mut lookup = Vec::with_capacity(choices_map.len());
        let mut choices = Vec::with_capacity(choices_map.len());

        for (idx, (choice_key, choice_schema)) in choices_map.iter().enumerate() {
            let serializer =
                CombinedSerializer::build(as_dict(choice_schema)?, config, definitions)?;
            choices.push(serializer);
            // Keys should be unique, because they came from a dict
            lookup.push((choice_key.clone(), idx));
        }

        let choices = match UnionChoices::from_choices(choices)? {
            FromChoicesOutput::Single(s) => {
                return Ok(s);
            }
            FromChoicesOutput::Multiple(m) => m,
        };

        Ok(Arc::new(CombinedSerializer::TaggedUnion(Box::new(Self {
            discriminator,
            lookup,
            choices,
            name: OnceLock::new(),
        }))))
    }
}

impl TypeSerializer for TaggedUnionSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        match self.tagged_union_serialize(
            value,
            |comb_serializer, state| comb_serializer.to_python(value, state),
            state,
        )? {
            Some(v) => Ok(v),
            None => infer_to_python(value, state),
        }
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        match self.tagged_union_serialize(
            key,
            |comb_serializer, state| comb_serializer.json_key(key, state).map(Cow::into_owned),
            state,
        )? {
            Some(v) => Ok(Cow::Owned(v)),
            None => infer_json_key(key, state),
        }
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        match self.tagged_union_serialize(
            value,
            |comb_serializer, state| comb_serializer.to_python(value, state),
            state,
        ) {
            Ok(Some(v)) => {
                let state = &mut state.scoped_include_exclude(IncludeExclude::empty());
                infer_serialize(&v, serializer, state)
            }
            Ok(None) => infer_serialize(value, serializer, state),
            Err(err) => Err(serde::ser::Error::custom(err.py_display())),
        }
    }

    fn get_name(&self) -> &str {
        self.name.get_or_init(|| {
            let names: Vec<&str> = self.choices.choices.iter().map(|s| s.get_name()).collect();
            format!("TaggedUnion[{}]", names.join(", "))
        })
    }

    fn retry_with_lax_check(&self) -> bool {
        self.choices.retry_with_lax_check()
    }
}

impl TaggedUnionSerializer {
    fn get_discriminator_value<'a>(&self, value: &'a Value) -> Option<Cow<'a, Value>> {
        // we're pretty lax here, we allow either dict[key] or model.field
        let dict = match value {
            Value::Dict(dict) => dict,
            Value::Model(model) => &model.fields,
            _ => return None,
        };
        self.discriminator
            .iter()
            .find_map(|path| path.value_get(dict))
    }

    fn tagged_union_serialize<S>(
        &self,
        value: &Value,
        // if this returns `Ok(v)`, we picked a union variant to serialize, where
        // `S` is intermediate state which can be passed on to the finalizer
        mut selector: impl FnMut(&CombinedSerializer, &mut SerializationState) -> SerResult<S>,
        state: &mut SerializationState,
    ) -> SerResult<Option<S>> {
        let choice = if let Some(tag) = self.get_discriminator_value(value)
            && let Some((_, serializer_index)) = self.lookup.iter().find(|(k, _)| k.py_eq(&tag))
        {
            &self.choices.choices[*serializer_index]
        } else {
            // No tag found, fall back to left-to-right inference
            if in_top_level_union(state) {
                register_tagged_union_fallback_warning(value, state);
            }
            return self.choices.serialize(selector, state);
        };

        // Try a first pass with the appropriate checking level
        let err = match selector(
            choice,
            &mut scoped_check_level(state, initial_check_level(state)),
        ) {
            Ok(v) => return Ok(Some(v)),
            Err(err) => err,
        };

        // If not in a nested union, try lax check
        if in_top_level_union(state)
            && self.retry_with_lax_check()
            && let Ok(v) = selector(choice, &mut scoped_check_level(state, SerCheck::Lax))
        {
            return Ok(Some(v));
        }

        // The discriminator matched a specific variant but serialization failed.
        if in_top_level_union(state) {
            // Register a warning for the matched variant only, then
            // fall through to inference instead of trying all other variants.
            register_error_as_warning(state, err);
            return Ok(None);
        }

        // In a nested union, propagate the error so the parent can retry
        Err(err)
    }
}

/// Whether currently in a top-level union serialization
fn in_top_level_union(state: &SerializationState) -> bool {
    state.check == SerCheck::None
}

/// Check level to use for the first pass of union serialization
/// - If we're in a nested union (state.check != None), we use the current check level
/// - If we're in a top-level union (state.check == None), we use strict checking
fn initial_check_level(state: &SerializationState) -> SerCheck {
    if in_top_level_union(state) {
        SerCheck::Strict
    } else {
        state.check
    }
}

#[derive(Debug)]
struct UnionChoices {
    choices: Vec<Arc<CombinedSerializer>>,
}

enum FromChoicesOutput {
    Single(Arc<CombinedSerializer>),
    Multiple(UnionChoices),
}

impl UnionChoices {
    fn from_choices(choices: Vec<Arc<CombinedSerializer>>) -> CoreResult<FromChoicesOutput> {
        match choices.len() {
            0 => schema_err!("One or more union choices required"),
            1 => Ok(FromChoicesOutput::Single(
                choices.into_iter().next().expect("one choice"),
            )),
            _ => Ok(FromChoicesOutput::Multiple(Self { choices })),
        }
    }

    fn retry_with_lax_check(&self) -> bool {
        self.choices.iter().any(|c| c.retry_with_lax_check())
    }

    /// Try to serialize using the union choices from left to right
    fn serialize<S>(
        &self,
        mut selector: impl FnMut(&CombinedSerializer, &mut SerializationState) -> SerResult<S>,
        state: &mut SerializationState,
    ) -> SerResult<Option<S>> {
        // try the serializers in left to right order with strict checking
        let mut errors: Vec<SerializeError> = Vec::new();

        // First try left-to-right with checks enabled, collecting errors
        // - at strict level if we're in a top-level union (state.check == None)
        // - otherwise, use the current check level
        {
            let state = &mut scoped_check_level(state, initial_check_level(state));
            for comb_serializer in &self.choices {
                match selector(comb_serializer, state) {
                    Ok(v) => return Ok(Some(v)),
                    Err(err) => errors.push(err),
                }
            }
        }

        // in a nested union, we immediately bail out with the collected errors
        if !in_top_level_union(state) {
            debug_assert_eq!(errors.len(), self.choices.len());
            return Err(union_serialization_unexpected_value(&errors));
        }

        // otherwise, in a top level union, we retry with lax checking if any choice supports it
        if self.retry_with_lax_check() {
            let state = &mut scoped_check_level(state, SerCheck::Lax);
            for comb_serializer in &self.choices {
                if let Ok(v) = selector(comb_serializer, state) {
                    return Ok(Some(v));
                }
            }
        }

        // ... and if that still didn't work, we register all collected errors as warnings
        for err in errors {
            register_error_as_warning(state, err);
        }

        // ... before falling back to inference
        Ok(None)
    }
}

/// Set the serialization check level for the duration of the scoped state, helper just to
/// reduce boilerplate
fn scoped_check_level(
    state: &mut SerializationState,
    check_level: SerCheck,
) -> crate::serializers::extra::ScopedSetState<
    '_,
    impl for<'s> Fn(&'s mut SerializationState) -> &'s mut SerCheck,
    SerCheck,
> {
    state.scoped_set(|s| &mut s.check, check_level)
}

/// Produce an unexpected value error from errors encountered during union serialization
#[cold]
fn union_serialization_unexpected_value(errors: &[SerializeError]) -> SerializeError {
    let message = errors
        .iter()
        .map(SerializeError::py_display)
        .collect::<Vec<_>>()
        .join("\n");
    SerializeError::UnexpectedValue(UnexpectedValue::new_from_msg(Some(message)))
}

#[cold]
fn register_error_as_warning(state: &mut SerializationState, err: SerializeError) {
    match err {
        SerializeError::UnexpectedValue(unexpected_value) => {
            state.warnings.register_warning(unexpected_value);
        }
        other => state
            .warnings
            .register_warning(UnexpectedValue::new_from_msg(Some(other.py_display()))),
    }
}

#[cold]
fn register_tagged_union_fallback_warning(value: &Value, state: &mut SerializationState) {
    state.warnings.register_warning(UnexpectedValue::new(
        Some(
            "Defaulting to left to right union serialization - failed to get discriminator value for tagged union serialization"
                .to_string(),
        ),
        None,
        None,
        Some(value.clone()),
    ));
}
