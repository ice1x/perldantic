//! Cycle and depth protection during validation. Port of upstream `recursion_guard.rs`.

use std::collections::HashSet;

type RecursionKey = (
    // Identifier for the input object, e.g. the address of a host container.
    usize,
    // Identifier for the node being traversed, e.g. a definition reference id.
    usize,
);

/// Guards against cyclic input data causing infinite recursion through definition references.
///
/// Releases its (object, node) pair and depth when dropped.
pub(crate) struct RecursionGuard<'a, S: ContainsRecursionState> {
    state: &'a mut S,
    obj_id: usize,
    node_id: usize,
}

pub(crate) enum RecursionError {
    /// Cyclic reference detected
    Cyclic,
    /// Recursion limit exceeded
    Depth,
}

impl<S: ContainsRecursionState> RecursionGuard<'_, S> {
    pub fn new(
        state: &'_ mut S,
        obj_id: usize,
        node_id: usize,
    ) -> Result<RecursionGuard<'_, S>, RecursionError> {
        state.access_recursion_state(|state| {
            if !state.insert(obj_id, node_id) {
                return Err(RecursionError::Cyclic);
            }
            if state.incr_depth() {
                return Err(RecursionError::Depth);
            }
            Ok(())
        })?;
        Ok(RecursionGuard {
            state,
            obj_id,
            node_id,
        })
    }

    pub fn state(&mut self) -> &mut S {
        self.state
    }
}

impl<S: ContainsRecursionState> Drop for RecursionGuard<'_, S> {
    fn drop(&mut self) {
        self.state.access_recursion_state(|state| {
            state.decr_depth();
            state.remove(self.obj_id, self.node_id);
        });
    }
}

/// Access to the recursion state held by some other type.
pub(crate) trait ContainsRecursionState {
    fn access_recursion_state<R>(&mut self, f: impl FnOnce(&mut RecursionState) -> R) -> R;
}

/// State for the [`RecursionGuard`]; can also be used directly to track depth.
#[derive(Debug, Clone, Default)]
pub struct RecursionState {
    ids: RecursionStack,
    // One depth for all validators keeps this simple and fast.
    depth: u8,
}

/// Hard limit that prevents stack overflows from runaway recursion.
///
/// Upstream lowers the limit where stacks are small (wasm, Windows, debug builds);
/// debug builds keep a margin here as well.
pub const RECURSION_GUARD_LIMIT: u8 = if cfg!(debug_assertions) { 235 } else { 255 };

impl RecursionState {
    /// Insert a pair; `false` if it was already present.
    fn insert(&mut self, obj_id: usize, node_id: usize) -> bool {
        self.ids.insert((obj_id, node_id))
    }

    /// Increase depth; `true` if the limit would be exceeded.
    #[must_use]
    pub fn incr_depth(&mut self) -> bool {
        match self.depth.checked_add(1) {
            Some(depth) if depth <= RECURSION_GUARD_LIMIT => {
                self.depth = depth;
                false
            }
            _ => true,
        }
    }

    pub fn decr_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn remove(&mut self, obj_id: usize, node_id: usize) {
        self.ids.remove(&(obj_id, node_id));
    }
}

/// Upstream's measured size beyond which a hash set beats a linear scan.
const ARRAY_SIZE: usize = 16;

/// A small inline stack that spills into a hash set when it grows.
// The large inline variant is intentional: shallow recursion never allocates.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum RecursionStack {
    Array {
        data: [RecursionKey; ARRAY_SIZE],
        len: usize,
    },
    Set(HashSet<RecursionKey>),
}

impl Default for RecursionStack {
    fn default() -> Self {
        Self::Array {
            data: [(0, 0); ARRAY_SIZE],
            len: 0,
        }
    }
}

impl RecursionStack {
    /// Insert a key; `false` if it was already present.
    fn insert(&mut self, v: RecursionKey) -> bool {
        match self {
            Self::Array { data, len } => {
                if data[..*len].contains(&v) {
                    return false;
                }
                if *len < ARRAY_SIZE {
                    data[*len] = v;
                    *len += 1;
                    true
                } else {
                    let mut set: HashSet<RecursionKey> = data.iter().copied().collect();
                    let inserted = set.insert(v);
                    *self = Self::Set(set);
                    inserted
                }
            }
            Self::Set(set) => set.insert(v),
        }
    }

    /// Remove a key. The inline array is strictly LIFO, matching guard drop order.
    fn remove(&mut self, v: &RecursionKey) {
        match self {
            Self::Array { data, len } => {
                *len = len
                    .checked_sub(1)
                    .expect("remove from empty recursion guard");
                assert!(data[*len] == *v, "remove did not match insert");
            }
            Self::Set(set) => {
                set.remove(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Holder(RecursionState);

    impl ContainsRecursionState for Holder {
        fn access_recursion_state<R>(&mut self, f: impl FnOnce(&mut RecursionState) -> R) -> R {
            f(&mut self.0)
        }
    }

    #[test]
    fn same_object_and_node_is_cyclic_until_released() {
        let mut holder = Holder(RecursionState::default());
        {
            let Ok(mut guard) = RecursionGuard::new(&mut holder, 1, 7) else {
                panic!("first entry must succeed")
            };
            assert!(matches!(
                RecursionGuard::new(guard.state(), 1, 7),
                Err(RecursionError::Cyclic)
            ));
            // A different node or object is fine.
            assert!(RecursionGuard::new(guard.state(), 1, 8).is_ok());
            assert!(RecursionGuard::new(guard.state(), 2, 7).is_ok());
        }
        assert!(RecursionGuard::new(&mut holder, 1, 7).is_ok());
        assert_eq!(holder.0.depth, 0);
    }

    #[test]
    fn nesting_beyond_the_limit_is_a_depth_error() {
        let mut state = RecursionState::default();
        for _ in 0..RECURSION_GUARD_LIMIT {
            assert!(!state.incr_depth());
        }
        assert!(state.incr_depth());
        assert_eq!(state.depth, RECURSION_GUARD_LIMIT);
        state.decr_depth();
        assert_eq!(state.depth, RECURSION_GUARD_LIMIT - 1);
    }

    #[test]
    fn stack_spills_into_a_set_after_the_inline_array() {
        let mut stack = RecursionStack::default();
        for i in 0..40 {
            assert!(stack.insert((i, 0)));
        }
        assert!(matches!(stack, RecursionStack::Set(_)));
        assert!(!stack.insert((3, 0)));
        stack.remove(&(3, 0));
        assert!(stack.insert((3, 0)));
    }

    #[test]
    fn inline_array_removes_in_lifo_order() {
        let mut stack = RecursionStack::default();
        assert!(stack.insert((1, 1)));
        assert!(stack.insert((2, 2)));
        assert!(!stack.insert((1, 1)));
        stack.remove(&(2, 2));
        stack.remove(&(1, 1));
        assert!(stack.insert((1, 1)));
    }
}
