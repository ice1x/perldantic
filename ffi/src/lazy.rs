//! Validated models kept in the core for lazy host objects. A lazy object holds a handle, a
//! number the host cannot forge into memory access: the core looks it up here to read the
//! model's fields when the object is first used, or to serialize the model as it is.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use perldantic_core::Model;

struct Registry {
    models: HashMap<usize, Arc<Model>>,
    next: usize,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| {
    Mutex::new(Registry {
        models: HashMap::new(),
        next: 1,
    })
});

fn registry() -> MutexGuard<'static, Registry> {
    // a panic while holding the lock leaves the map consistent: every operation is one call
    REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Keep a model for a lazy object; returns its handle (never 0).
pub fn register(model: Arc<Model>) -> usize {
    let mut registry = registry();
    loop {
        let handle = registry.next;
        registry.next = registry.next.wrapping_add(1).max(1);
        if let std::collections::hash_map::Entry::Vacant(slot) = registry.models.entry(handle) {
            slot.insert(model);
            return handle;
        }
    }
}

/// The model behind a handle, if it is live.
pub fn get(handle: usize) -> Option<Arc<Model>> {
    registry().models.get(&handle).cloned()
}

/// Drop a handle; unknown handles are ignored.
pub fn release(handle: usize) {
    let model = registry().models.remove(&handle);
    // the model is dropped outside the lock
    drop(model);
}

/// How many handles are live.
pub fn live() -> usize {
    registry().models.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use perldantic_core::Dict;

    fn model(class: &str) -> Arc<Model> {
        Arc::new(Model {
            class: class.to_owned(),
            fields: Dict::new(),
            fields_set: vec![],
            extra: None,
        })
    }

    #[test]
    fn handles_find_their_model_until_released() {
        let point = model("My::Point");
        let handle = register(point.clone());
        assert_ne!(handle, 0);
        assert!(Arc::ptr_eq(&get(handle).unwrap(), &point));
        release(handle);
        assert!(get(handle).is_none());
        release(handle);
    }

    #[test]
    fn every_handle_is_new() {
        let first = register(model("A"));
        let second = register(model("B"));
        assert_ne!(first, second);
        assert_eq!(get(first).unwrap().class, "A");
        assert_eq!(get(second).unwrap().class, "B");
        release(first);
        release(second);
    }

    #[test]
    fn a_released_model_is_dropped() {
        let point = model("My::Point");
        let handle = register(point.clone());
        assert_eq!(Arc::strong_count(&point), 2);
        release(handle);
        assert_eq!(Arc::strong_count(&point), 1);
    }
}
