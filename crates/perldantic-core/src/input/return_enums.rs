//! Values returned by `Input` validation methods. Port of the P0 parts of upstream
//! `input/return_enums.rs`.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::ops::Rem;

use num_bigint::BigInt;

use crate::errors::{ErrorTypeDefaults, ValError, ValResult};
use crate::validators::validation_state::{Exactness, ValidationState};
use crate::value::Value;

/// A validated value plus how exactly the input matched.
#[derive(Debug)]
pub struct ValidationMatch<T>(T, Exactness);

impl<T> ValidationMatch<T> {
    pub const fn new(value: T, exactness: Exactness) -> Self {
        Self(value, exactness)
    }

    pub const fn exact(value: T) -> Self {
        Self(value, Exactness::Exact)
    }

    pub const fn strict(value: T) -> Self {
        Self(value, Exactness::Strict)
    }

    pub const fn lax(value: T) -> Self {
        Self(value, Exactness::Lax)
    }

    pub fn exactness(&self) -> Exactness {
        self.1
    }

    pub fn require_exact(self) -> Option<T> {
        (self.1 == Exactness::Exact).then_some(self.0)
    }

    /// Record the exactness in the state and return the value.
    pub fn unpack(self, state: &mut ValidationState) -> T {
        state.floor_exactness(self.1);
        self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

/// A string borrowed from the input or produced by coercion.
#[derive(Debug, Clone, PartialEq)]
pub struct EitherString<'a>(Cow<'a, str>);

impl<'a> EitherString<'a> {
    pub fn as_cow(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.0.as_ref())
    }

    pub fn into_cow(self) -> Cow<'a, str> {
        self.0
    }

    pub fn into_value(self) -> Value {
        Value::Str(self.0.into_owned())
    }
}

impl<'a> From<&'a str> for EitherString<'a> {
    fn from(s: &'a str) -> Self {
        Self(Cow::Borrowed(s))
    }
}

impl From<String> for EitherString<'_> {
    fn from(s: String) -> Self {
        Self(Cow::Owned(s))
    }
}

/// Bytes borrowed from the input or produced by decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct EitherBytes<'a>(Cow<'a, [u8]>);

impl EitherBytes<'_> {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_value(self) -> Value {
        Value::Bytes(self.0.into_owned())
    }
}

impl<'a> From<&'a [u8]> for EitherBytes<'a> {
    fn from(b: &'a [u8]) -> Self {
        Self(Cow::Borrowed(b))
    }
}

impl From<Vec<u8>> for EitherBytes<'_> {
    fn from(b: Vec<u8>) -> Self {
        Self(Cow::Owned(b))
    }
}

/// An integer that may exceed `i64`.
#[derive(Debug, Clone, PartialEq)]
pub enum EitherInt {
    I64(i64),
    BigInt(BigInt),
}

impl EitherInt {
    pub fn into_i64(self) -> ValResult<i64> {
        match self {
            Self::I64(i) => Ok(i),
            Self::BigInt(b) => i64::try_from(&b)
                .map_err(|_| ValError::new(ErrorTypeDefaults::IntParsingSize, Value::BigInt(b))),
        }
    }

    pub fn as_int(&self) -> Int {
        match self {
            Self::I64(i) => Int::I64(*i),
            Self::BigInt(b) => Int::Big(b.clone()),
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::I64(0) => Some(false),
            Self::I64(1) => Some(true),
            Self::I64(_) => None,
            Self::BigInt(i) => match u8::try_from(i) {
                Ok(0) => Some(false),
                Ok(1) => Some(true),
                _ => None,
            },
        }
    }

    pub fn into_value(self) -> Value {
        match self {
            Self::I64(i) => Value::Int(i),
            Self::BigInt(b) => Value::from(b),
        }
    }
}

/// A float produced by validation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EitherFloat {
    F64(f64),
}

impl EitherFloat {
    pub fn as_f64(&self) -> f64 {
        match self {
            Self::F64(f) => *f,
        }
    }
}

/// An integer for constraint arithmetic (`gt`, `multiple_of`, ...).
#[derive(Debug, Clone)]
pub enum Int {
    I64(i64),
    Big(BigInt),
}

impl PartialOrd for Int {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Int::I64(i1), Int::I64(i2)) => Some(i1.cmp(i2)),
            (Int::Big(b1), Int::Big(b2)) => Some(b1.cmp(b2)),
            (Int::I64(i), Int::Big(b)) => Some(BigInt::from(*i).cmp(b)),
            (Int::Big(b), Int::I64(i)) => Some(b.cmp(&BigInt::from(*i))),
        }
    }
}

impl PartialEq for Int {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl Rem for &Int {
    type Output = Int;

    fn rem(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Int::I64(i1), Int::I64(i2)) => Int::I64(i1 % i2),
            (Int::Big(b1), Int::Big(b2)) => Int::Big(b1 % b2),
            (Int::I64(i), Int::Big(b)) => Int::Big(BigInt::from(*i) % b),
            (Int::Big(b), Int::I64(i)) => Int::Big(b % BigInt::from(*i)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_compares_across_representations() {
        let big = Int::Big(BigInt::from(5));
        assert!(Int::I64(4) < big);
        assert_eq!(Int::I64(5), big);
        assert_eq!(&Int::I64(7) % &Int::Big(BigInt::from(4)), Int::I64(3));
    }

    #[test]
    fn either_int_conversions() {
        assert_eq!(EitherInt::BigInt(BigInt::from(9)).into_i64().unwrap(), 9);
        assert!(
            EitherInt::BigInt(BigInt::from(2).pow(70))
                .into_i64()
                .is_err()
        );
        assert_eq!(EitherInt::BigInt(BigInt::from(1)).as_bool(), Some(true));
        assert_eq!(EitherInt::I64(2).as_bool(), None);
        assert_eq!(EitherInt::I64(4).into_value(), Value::Int(4));
        assert!(matches!(
            EitherInt::BigInt(BigInt::from(i64::MAX)).into_value(),
            Value::Int(i64::MAX)
        ));
    }

    #[test]
    fn exactness_is_recorded_on_unpack() {
        use crate::InputType;
        use crate::recursion_guard::RecursionState;
        use crate::validators::Extra;
        use crate::validators::validation_state::PartialMode;

        let mut guard = RecursionState::default();
        let extra = Extra::new(None, None, None, None, InputType::Python, None, None);
        let mut state = ValidationState::new(extra, &mut guard, PartialMode::Off);
        state.exactness = Some(Exactness::Exact);
        assert_eq!(ValidationMatch::lax(1).unpack(&mut state), 1);
        assert_eq!(state.exactness, Some(Exactness::Lax));
        assert_eq!(ValidationMatch::strict(2).require_exact(), None);
        assert_eq!(ValidationMatch::exact(3).require_exact(), Some(3));
    }
}
