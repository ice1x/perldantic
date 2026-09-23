//! Shared, possibly recursive definitions (`ref` / `definitions`). Port of upstream `definitions.rs`.
//!
//! Like JSON Schema, a schema names shared parts with `ref` strings and uses them elsewhere.
//! Unlike JSON Schema, definitions may appear inline, not only in one `definitions` block.
//! [`DefinitionsBuilder`] collects them while a validator tree is built; references are
//! weak so recursive definitions (A -> B -> A) do not leak.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use crate::build_tools::schema_err;
use crate::core_error::CoreResult;

/// Validators or serializers shared by reference, owned by the top-level validator.
pub struct Definitions<T>(HashMap<Arc<String>, Definition<T>>);

struct Definition<T> {
    value: Arc<OnceLock<T>>,
    name: Arc<LazyName>,
}

/// Reference to a definition.
pub struct DefinitionRef<T> {
    reference: Arc<String>,
    // Weak to avoid a reference cycle when definitions are recursive.
    value: Weak<OnceLock<T>>,
    name: Arc<LazyName>,
}

// Manual impl: #[derive(Clone)] would require T: Clone.
impl<T> Clone for DefinitionRef<T> {
    fn clone(&self) -> Self {
        Self {
            reference: self.reference.clone(),
            value: self.value.clone(),
            name: self.name.clone(),
        }
    }
}

impl<T> DefinitionRef<T> {
    /// Stable id of the referenced definition; used as the recursion guard node id.
    pub fn id(&self) -> usize {
        Weak::as_ptr(&self.value) as usize
    }

    pub fn reference(&self) -> &str {
        &self.reference
    }

    pub fn get_or_init_name(&self, init: impl FnOnce(&T) -> String) -> &str {
        let Some(definition) = self.value.upgrade() else {
            return "...";
        };
        match definition.get() {
            Some(value) => self.name.get_or_init(|| init(value)),
            None => "...",
        }
    }

    pub fn read<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
        f(self.value.upgrade().as_ref().and_then(|value| value.get()))
    }
}

impl<T: Debug> Debug for DefinitionRef<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only the name, to avoid infinite recursion through recursive definitions.
        self.name.fmt(f)
    }
}

impl<T: Debug> Debug for Definitions<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.values()).finish()
    }
}

impl<T: Debug> Debug for Definition<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.value.get() {
            Some(value) => value.fmt(f),
            None => "...".fmt(f),
        }
    }
}

#[derive(Debug)]
pub struct DefinitionsBuilder<T> {
    definitions: Definitions<T>,
}

impl<T: Debug> Default for DefinitionsBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Debug> DefinitionsBuilder<T> {
    pub fn new() -> Self {
        Self {
            definitions: Definitions(HashMap::new()),
        }
    }

    /// Reference a definition by name, whether or not it has been added yet.
    pub fn get_definition(&mut self, reference: &str) -> DefinitionRef<T> {
        let reference = Arc::new(reference.to_string());
        let value = match self.definitions.0.entry(reference.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(Definition {
                value: Arc::new(OnceLock::new()),
                name: Arc::new(LazyName::new()),
            }),
        };
        DefinitionRef {
            reference,
            value: Arc::downgrade(&value.value),
            name: value.name.clone(),
        }
    }

    /// Add a definition, returning a reference to it.
    pub fn add_definition(&mut self, reference: String, value: T) -> CoreResult<DefinitionRef<T>> {
        let reference = Arc::new(reference);
        let value = match self.definitions.0.entry(reference.clone()) {
            Entry::Occupied(entry) => {
                let definition = entry.into_mut();
                match definition.value.set(value) {
                    Ok(()) => definition,
                    Err(_) => return schema_err!("Duplicate ref: `{reference}`"),
                }
            }
            Entry::Vacant(entry) => entry.insert(Definition {
                value: Arc::new(OnceLock::from(value)),
                name: Arc::new(LazyName::new()),
            }),
        };
        Ok(DefinitionRef {
            reference,
            value: Arc::downgrade(&value.value),
            name: value.name.clone(),
        })
    }

    /// Finish building; every referenced definition must have been added.
    pub fn finish(self) -> CoreResult<Definitions<T>> {
        for (reference, def) in &self.definitions.0 {
            if def.value.get().is_none() {
                return schema_err!("Definitions error: definition `{reference}` was never filled");
            }
        }
        Ok(self.definitions)
    }
}

/// A lazily initialised value that returns a default instead of recursing forever when its
/// initializer (indirectly) asks for the value itself.
pub(crate) struct RecursionSafeCache<T> {
    cache: OnceLock<T>,
    in_recursion: AtomicBool,
}

impl<T: Clone> Clone for RecursionSafeCache<T> {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            in_recursion: AtomicBool::new(false),
        }
    }
}

impl<T> RecursionSafeCache<T> {
    pub(crate) fn new() -> Self {
        Self {
            cache: OnceLock::new(),
            in_recursion: AtomicBool::new(false),
        }
    }

    pub(crate) fn get_or_init<D: ?Sized>(
        &self,
        init: impl FnOnce() -> T,
        recursive_default: &'static D,
    ) -> &D
    where
        T: Borrow<D>,
    {
        if let Some(cached) = self.cache.get() {
            return cached.borrow();
        }

        if self
            .in_recursion
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return recursive_default;
        }
        let result = self.cache.get_or_init(init).borrow();
        self.in_recursion.store(false, Ordering::SeqCst);
        result
    }

    fn get(&self) -> Option<&T> {
        self.cache.get()
    }
}

#[derive(Clone)]
struct LazyName(RecursionSafeCache<String>);

impl LazyName {
    fn new() -> Self {
        Self(RecursionSafeCache::new())
    }

    fn get_or_init(&self, init: impl FnOnce() -> String) -> &str {
        self.0.get_or_init(init, "...")
    }
}

impl Debug for LazyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().map_or("...", String::as_str).fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoreError;

    #[test]
    fn reference_before_definition_resolves_after_add() {
        let mut builder = DefinitionsBuilder::<String>::new();
        let early = builder.get_definition("Node");
        early.read(|v| assert_eq!(v, None));
        let added = builder
            .add_definition("Node".into(), "node-validator".into())
            .unwrap();
        assert_eq!(early.id(), added.id());
        let defs = builder.finish().unwrap();
        early.read(|v| assert_eq!(v.map(String::as_str), Some("node-validator")));
        assert_eq!(early.reference(), "Node");
        drop(defs);
        // Weak reference: once definitions are gone the value is unreachable.
        early.read(|v| assert_eq!(v, None));
    }

    #[test]
    fn duplicate_definition_is_a_schema_error() {
        let mut builder = DefinitionsBuilder::<i32>::new();
        builder.add_definition("A".into(), 1).unwrap();
        assert_eq!(
            builder.add_definition("A".into(), 2).unwrap_err(),
            CoreError::Schema("Duplicate ref: `A`".into())
        );
    }

    #[test]
    fn unfilled_definition_is_a_schema_error() {
        let mut builder = DefinitionsBuilder::<i32>::new();
        let _ = builder.get_definition("Missing");
        assert_eq!(
            builder.finish().unwrap_err(),
            CoreError::Schema("Definitions error: definition `Missing` was never filled".into())
        );
    }

    #[test]
    fn names_are_computed_once_and_recursion_safe() {
        let mut builder = DefinitionsBuilder::<i32>::new();
        let reference = builder.add_definition("A".into(), 5).unwrap();
        let _defs = builder.finish().unwrap();
        assert_eq!(
            reference.get_or_init_name(|v| format!("value-{v}")),
            "value-5"
        );
        // The cached name wins over a new initializer.
        assert_eq!(reference.get_or_init_name(|_| "other".into()), "value-5");
    }

    #[test]
    fn recursion_safe_cache_returns_default_inside_its_own_initializer() {
        let cache: RecursionSafeCache<String> = RecursionSafeCache::new();
        let outer = cache.get_or_init(
            || {
                let inner = cache.get_or_init(|| unreachable!(), "...");
                format!("outer({inner})")
            },
            "...",
        );
        assert_eq!(outer, "outer(...)");
    }
}
