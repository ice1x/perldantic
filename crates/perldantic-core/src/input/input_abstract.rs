//! The `Input` abstraction validators are written against. Port of upstream
//! `input/input_abstract.rs`, reduced to the methods the ported validators use; further methods
//! are added as their validators are ported.

use std::fmt;

use crate::core_error::CoreResult;
use crate::errors::{ErrorTypeDefaults, LocItem, ValError, ValResult};
use crate::validators::config::ValBytesMode;
use crate::value::Value;

use super::return_enums::{EitherBytes, EitherFloat, EitherInt, EitherString, ValidationMatch};

pub type ValMatch<T> = ValResult<ValidationMatch<T>>;

/// Something that can be validated: JSON, host data (`Value`) or a string key.
///
/// Every type has `validate_*`, `strict_*` and `lax_*` methods by convention: implement either
/// `strict_*` and `lax_*` when they differ, or `validate_*` alone when they don't.
pub trait Input: fmt::Debug {
    /// The input as it is reported in errors.
    fn as_error_value(&self) -> Value;

    /// The input converted to a `Value` unchanged, e.g. for `any` (upstream `to_object`).
    fn to_value(&self) -> Value {
        self.as_error_value()
    }

    fn is_none(&self) -> bool {
        false
    }

    fn validate_str(&self, strict: bool, coerce_numbers_to_str: bool)
    -> ValMatch<EitherString<'_>>;

    /// A string, only accepting exact string input.
    fn exact_str(&self) -> ValResult<EitherString<'_>> {
        self.validate_str(true, false).and_then(|val_match| {
            val_match
                .require_exact()
                .ok_or_else(|| ValError::new(ErrorTypeDefaults::StringType, self))
        })
    }

    fn validate_bytes(&self, strict: bool, mode: ValBytesMode) -> ValMatch<EitherBytes<'_>>;

    fn validate_bool(&self, strict: bool) -> ValMatch<bool>;

    fn validate_int(&self, strict: bool) -> ValMatch<EitherInt>;

    /// An integer, only accepting exact integer input.
    fn exact_int(&self) -> ValResult<EitherInt> {
        self.validate_int(true).and_then(|val_match| {
            val_match
                .require_exact()
                .ok_or_else(|| ValError::new(ErrorTypeDefaults::IntType, self))
        })
    }

    fn validate_float(&self, strict: bool) -> ValMatch<EitherFloat>;

    type Dict<'a>: ValidatedDict
    where
        Self: 'a;

    fn validate_dict(&self, strict: bool) -> ValResult<Self::Dict<'_>> {
        if strict {
            self.strict_dict()
        } else {
            self.lax_dict()
        }
    }

    fn strict_dict(&self) -> ValResult<Self::Dict<'_>>;

    fn lax_dict(&self) -> ValResult<Self::Dict<'_>> {
        self.strict_dict()
    }

    type List<'a>: ValidatedList
    where
        Self: 'a;

    fn validate_list(&self, strict: bool) -> ValMatch<Self::List<'_>>;

    type Tuple<'a>: ValidatedTuple
    where
        Self: 'a;

    fn validate_tuple(&self, strict: bool) -> ValMatch<Self::Tuple<'_>>;
}

/// Abstracts over owned and borrowed items yielded while iterating collections.
pub trait BorrowInput {
    type Input: Input + ?Sized;
    fn borrow_input(&self) -> &Self::Input;
}

impl<T: Input + ?Sized> BorrowInput for &'_ T {
    type Input = T;
    fn borrow_input(&self) -> &Self::Input {
        self
    }
}

/// A generic consumer of the different iterator types collections can produce.
pub trait ConsumeIterator<T> {
    type Output;
    fn consume_iterator(self, iterator: impl Iterator<Item = T>) -> Self::Output;
}

/// A mapping accepted by dict-like validation.
pub trait ValidatedDict {
    type Key<'a>: BorrowInput + Clone + Into<LocItem>
    where
        Self: 'a;
    type Item<'a>: BorrowInput
    where
        Self: 'a;
    fn iterate<'a, R>(
        &'a self,
        consumer: impl ConsumeIterator<ValResult<(Self::Key<'a>, Self::Item<'a>)>, Output = R>,
    ) -> ValResult<R>;
    /// Used in partial mode to check whether errors occurred in the last value.
    fn last_key(&self) -> Option<Self::Key<'_>>;
}

/// A sequence accepted by list validation.
// `len` is `None` for inputs of unknown length, so an `is_empty` would be misleading.
#[allow(clippy::len_without_is_empty)]
pub trait ValidatedList {
    type Item: BorrowInput;
    fn len(&self) -> Option<usize>;
    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R>;
}

/// A sequence accepted by tuple validation.
#[allow(clippy::len_without_is_empty)]
pub trait ValidatedTuple {
    type Item: BorrowInput;
    fn len(&self) -> Option<usize>;
    fn try_for_each(self, f: impl FnMut(CoreResult<Self::Item>) -> ValResult<()>) -> ValResult<()>;
    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R>;
}

/// Collection type for inputs that never produce that kind of collection.
#[derive(Debug)]
pub enum Never {}

impl ValidatedDict for Never {
    type Key<'a> = &'a Value;
    type Item<'a> = &'a Value;
    fn iterate<'a, R>(
        &'a self,
        _consumer: impl ConsumeIterator<ValResult<(Self::Key<'a>, Self::Item<'a>)>, Output = R>,
    ) -> ValResult<R> {
        match *self {}
    }
    fn last_key(&self) -> Option<Self::Key<'_>> {
        match *self {}
    }
}

impl ValidatedList for Never {
    type Item = &'static Value;
    fn len(&self) -> Option<usize> {
        match *self {}
    }
    fn iterate<R>(self, _consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        match self {}
    }
}

impl ValidatedTuple for Never {
    type Item = &'static Value;
    fn len(&self) -> Option<usize> {
        match *self {}
    }
    fn try_for_each(
        self,
        _f: impl FnMut(CoreResult<Self::Item>) -> ValResult<()>,
    ) -> ValResult<()> {
        match self {}
    }
    fn iterate<R>(self, _consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        match self {}
    }
}
