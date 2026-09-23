//! Per-call validation state. Port of upstream `validators/validation_state.rs`.

use std::ops::{Deref, DerefMut};

pub use jiter::PartialMode;

use crate::build_tools::ExtraBehavior;
use crate::recursion_guard::{ContainsRecursionState, RecursionState};
use crate::value::Dict;

use super::Extra;

/// How exactly an input matched; unions prefer the most exact member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd, Hash)]
pub enum Exactness {
    Lax,
    Strict,
    Exact,
}

pub struct ValidationState<'a> {
    pub recursion_guard: &'a mut RecursionState,
    pub exactness: Option<Exactness>,
    /// Tie-breaker for union validation: number of fields set on the validated value.
    ///
    /// Unlike `model_fields_set`, this never counts extra fields.
    pub fields_set_count: Option<usize>,
    /// Active when `allow_partial` is on and the last element of a sequence/mapping is validated.
    pub allow_partial: PartialMode,
    /// Whether at least one field had an error; default factories that take validated data
    /// must not run in that case.
    pub has_field_error: bool,
    /// The name of the field being validated, if applicable.
    field_name: Option<String>,
    /// The `data` argument passed to validator functions and default factories.
    pub data: Option<Dict>,
    // Read-only outside `rebind_extra`.
    extra: Extra<'a>,
}

impl<'a> ValidationState<'a> {
    pub fn new(
        extra: Extra<'a>,
        recursion_guard: &'a mut RecursionState,
        allow_partial: PartialMode,
    ) -> Self {
        Self {
            recursion_guard,
            // Exactness is only tracked when a union asks for it.
            exactness: None,
            fields_set_count: None,
            allow_partial,
            has_field_error: false,
            field_name: None,
            data: None,
            extra,
        }
    }

    /// Temporarily modify `extra`; the original is restored when the returned guard drops.
    pub fn rebind_extra<'state>(
        &'state mut self,
        f: impl FnOnce(&mut Extra<'a>),
    ) -> ValidationStateWithReboundExtra<'state, 'a> {
        let old_extra = self.extra.clone();
        f(&mut self.extra);
        ValidationStateWithReboundExtra {
            state: self,
            old_extra,
        }
    }

    /// Set one field for a scope; the previous value is restored when the guard drops.
    fn scoped_set<'state, P, T>(
        &'state mut self,
        projector: P,
        new_value: T,
    ) -> ScopedSetState<'state, 'a, P, T>
    where
        P: for<'p> Fn(&'p mut ValidationState<'a>) -> &'p mut T,
    {
        let value = std::mem::replace((projector)(self), new_value);
        ScopedSetState {
            state: self,
            projector,
            value,
        }
    }

    pub fn scoped_set_field_name(
        &mut self,
        new_value: Option<String>,
    ) -> ScopedFieldNameState<'_, 'a> {
        self.scoped_set(Self::field_name_mut, new_value)
    }

    pub fn scoped_set_data(&mut self, new_value: Option<Dict>) -> ScopedDataState<'_, 'a> {
        self.scoped_set(Self::data_mut, new_value)
    }

    pub fn scoped_clear_field_error(&mut self) -> ScopedHasFieldErrorState<'_, 'a> {
        self.scoped_set(Self::has_field_error_mut, false)
    }

    pub fn field_name(&self) -> Option<&str> {
        self.field_name.as_deref()
    }

    pub fn extra(&self) -> &Extra<'a> {
        &self.extra
    }

    pub fn enumerate_last_partial<I: Iterator>(&self, iter: I) -> EnumerateLastPartial<I> {
        EnumerateLastPartial::new(iter, self.allow_partial)
    }

    pub fn strict_or(&self, default: bool) -> bool {
        self.extra.strict.unwrap_or(default)
    }

    pub fn extra_behavior_or(&self, default: ExtraBehavior) -> ExtraBehavior {
        self.extra.extra_behavior.unwrap_or(default)
    }

    pub fn validate_by_alias_or(&self, default: Option<bool>) -> bool {
        self.extra.by_alias.or(default).unwrap_or(true)
    }

    pub fn validate_by_name_or(&self, default: Option<bool>) -> bool {
        self.extra.by_name.or(default).unwrap_or(false)
    }

    /// Lower the tracked exactness to `exactness` if that is less exact.
    ///
    /// Used by union validation, where the most exact member wins.
    pub fn floor_exactness(&mut self, exactness: Exactness) {
        match self.exactness {
            None | Some(Exactness::Lax) => {}
            Some(Exactness::Strict) => {
                if exactness == Exactness::Lax {
                    self.exactness = Some(Exactness::Lax);
                }
            }
            Some(Exactness::Exact) => self.exactness = Some(exactness),
        }
    }

    pub fn add_fields_set(&mut self, fields_set_count: usize) {
        *self.fields_set_count.get_or_insert(0) += fields_set_count;
    }

    fn field_name_mut(&mut self) -> &mut Option<String> {
        &mut self.field_name
    }

    fn data_mut(&mut self) -> &mut Option<Dict> {
        &mut self.data
    }

    fn has_field_error_mut(&mut self) -> &mut bool {
        &mut self.has_field_error
    }
}

impl ContainsRecursionState for ValidationState<'_> {
    fn access_recursion_state<R>(&mut self, f: impl FnOnce(&mut RecursionState) -> R) -> R {
        f(self.recursion_guard)
    }
}

pub struct ValidationStateWithReboundExtra<'state, 'a> {
    state: &'state mut ValidationState<'a>,
    old_extra: Extra<'a>,
}

impl<'a> Deref for ValidationStateWithReboundExtra<'_, 'a> {
    type Target = ValidationState<'a>;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}

impl DerefMut for ValidationStateWithReboundExtra<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.state
    }
}

impl Drop for ValidationStateWithReboundExtra<'_, '_> {
    fn drop(&mut self) {
        std::mem::swap(&mut self.state.extra, &mut self.old_extra);
    }
}

/// Like `iter.enumerate()`, plus a flag marking the last element while partial mode is on.
pub struct EnumerateLastPartial<I: Iterator> {
    iter: I,
    index: usize,
    next_item: Option<I::Item>,
    allow_partial: PartialMode,
}

impl<I: Iterator> EnumerateLastPartial<I> {
    pub fn new(mut iter: I, allow_partial: PartialMode) -> Self {
        let next_item = iter.next();
        Self {
            iter,
            index: 0,
            next_item,
            allow_partial,
        }
    }
}

impl<I: Iterator> Iterator for EnumerateLastPartial<I> {
    type Item = (usize, bool, I::Item);

    fn next(&mut self) -> Option<Self::Item> {
        let a = std::mem::replace(&mut self.next_item, self.iter.next())?;
        let i = self.index;
        self.index += 1;
        Some((
            i,
            self.allow_partial.is_active() && self.next_item.is_none(),
            a,
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

pub struct ScopedSetState<'scope, 'a, P, T>
where
    P: for<'p> Fn(&'p mut ValidationState<'a>) -> &'p mut T,
{
    state: &'scope mut ValidationState<'a>,
    projector: P,
    /// The previous value, restored on drop.
    value: T,
}

impl<'a, P, T> Drop for ScopedSetState<'_, 'a, P, T>
where
    P: for<'p> Fn(&'p mut ValidationState<'a>) -> &'p mut T,
{
    fn drop(&mut self) {
        std::mem::swap((self.projector)(self.state), &mut self.value);
    }
}

impl<'a, P, T> Deref for ScopedSetState<'_, 'a, P, T>
where
    P: for<'p> Fn(&'p mut ValidationState<'a>) -> &'p mut T,
{
    type Target = ValidationState<'a>;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}

impl<'a, P, T> DerefMut for ScopedSetState<'_, 'a, P, T>
where
    P: for<'p> Fn(&'p mut ValidationState<'a>) -> &'p mut T,
{
    fn deref_mut(&mut self) -> &mut ValidationState<'a> {
        self.state
    }
}

type ScopedSetStateT<'scope, 'a, T> =
    ScopedSetState<'scope, 'a, for<'s> fn(&'s mut ValidationState<'a>) -> &'s mut T, T>;

pub type ScopedFieldNameState<'scope, 'a> = ScopedSetStateT<'scope, 'a, Option<String>>;
pub type ScopedDataState<'scope, 'a> = ScopedSetStateT<'scope, 'a, Option<Dict>>;
pub type ScopedHasFieldErrorState<'scope, 'a> = ScopedSetStateT<'scope, 'a, bool>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputType;
    use crate::build_tools::ExtraBehavior;
    use crate::recursion_guard::RecursionState;

    fn extra() -> Extra<'static> {
        Extra::new(None, None, None, None, InputType::Python, None, None)
    }

    #[test]
    fn floor_exactness_only_ever_lowers() {
        let mut guard = RecursionState::default();
        let mut state = ValidationState::new(extra(), &mut guard, PartialMode::Off);
        state.floor_exactness(Exactness::Lax);
        assert_eq!(
            state.exactness, None,
            "no tracking unless a union asked for it"
        );

        state.exactness = Some(Exactness::Exact);
        state.floor_exactness(Exactness::Strict);
        assert_eq!(state.exactness, Some(Exactness::Strict));
        state.floor_exactness(Exactness::Exact);
        assert_eq!(state.exactness, Some(Exactness::Strict));
        state.floor_exactness(Exactness::Lax);
        assert_eq!(state.exactness, Some(Exactness::Lax));
        state.floor_exactness(Exactness::Exact);
        assert_eq!(state.exactness, Some(Exactness::Lax));
    }

    #[test]
    fn fields_set_count_accumulates() {
        let mut guard = RecursionState::default();
        let mut state = ValidationState::new(extra(), &mut guard, PartialMode::Off);
        assert_eq!(state.fields_set_count, None);
        state.add_fields_set(2);
        state.add_fields_set(3);
        assert_eq!(state.fields_set_count, Some(5));
    }

    #[test]
    fn extra_settings_fall_back_to_defaults() {
        let mut guard = RecursionState::default();
        let state = ValidationState::new(extra(), &mut guard, PartialMode::Off);
        assert!(state.strict_or(true));
        assert!(!state.strict_or(false));
        assert_eq!(
            state.extra_behavior_or(ExtraBehavior::Ignore),
            ExtraBehavior::Ignore
        );
        assert!(state.validate_by_alias_or(None));
        assert!(!state.validate_by_name_or(None));
        assert!(state.validate_by_name_or(Some(true)));

        let mut guard = RecursionState::default();
        let strict_extra = Extra::new(
            Some(false),
            Some(ExtraBehavior::Forbid),
            None,
            None,
            InputType::Json,
            Some(false),
            Some(true),
        );
        let state = ValidationState::new(strict_extra, &mut guard, PartialMode::Off);
        assert!(!state.strict_or(true));
        assert_eq!(
            state.extra_behavior_or(ExtraBehavior::Ignore),
            ExtraBehavior::Forbid
        );
        assert!(!state.validate_by_alias_or(Some(true)));
        assert!(state.validate_by_name_or(Some(false)));
        assert_eq!(state.extra().input_type, InputType::Json);
    }

    #[test]
    fn scoped_values_are_restored_on_drop() {
        let mut guard = RecursionState::default();
        let mut state = ValidationState::new(extra(), &mut guard, PartialMode::Off);
        {
            let mut scoped = state.scoped_set_field_name(Some("name".into()));
            assert_eq!(scoped.field_name(), Some("name"));
            scoped.has_field_error = true;
        }
        assert_eq!(state.field_name(), None);
        assert!(state.has_field_error);
        {
            let scoped = state.scoped_clear_field_error();
            assert!(!scoped.has_field_error);
        }
        assert!(state.has_field_error);
    }

    #[test]
    fn rebound_extra_is_restored_on_drop() {
        let mut guard = RecursionState::default();
        let mut state = ValidationState::new(extra(), &mut guard, PartialMode::Off);
        {
            let rebound = state.rebind_extra(|e| e.strict = Some(true));
            assert!(rebound.strict_or(false));
        }
        assert!(!state.strict_or(false));
    }

    #[test]
    fn context_is_exposed_to_validators() {
        let context = crate::Value::from("ctx");
        let mut guard = RecursionState::default();
        let extra = Extra::new(
            None,
            None,
            None,
            Some(&context),
            InputType::Python,
            None,
            None,
        );
        let state = ValidationState::new(extra, &mut guard, PartialMode::Off);
        assert_eq!(state.extra().context, Some(&context));
    }

    #[test]
    fn enumerate_last_partial_flags_only_the_last_item_when_active() {
        let mut guard = RecursionState::default();
        let state = ValidationState::new(extra(), &mut guard, PartialMode::On);
        let items: Vec<_> = state
            .enumerate_last_partial(["a", "b", "c"].into_iter())
            .collect();
        assert_eq!(items, [(0, false, "a"), (1, false, "b"), (2, true, "c")]);

        let mut guard = RecursionState::default();
        let state = ValidationState::new(extra(), &mut guard, PartialMode::Off);
        let items: Vec<_> = state
            .enumerate_last_partial(["a", "b"].into_iter())
            .collect();
        assert_eq!(items, [(0, false, "a"), (1, false, "b")]);
    }
}
