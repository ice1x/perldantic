//! `Input` for host data converted to `Value`.
//!
//! Replaces upstream `input/input_python.rs`: the coercion rules are those pydantic applies to
//! Python objects of the corresponding types (`str`, `bytes`, `bool`, `int`, `float`, `list`,
//! `tuple`, `dict`).

use std::borrow::Cow;
use std::str::from_utf8;

use num_traits::cast::ToPrimitive;

use crate::core_error::CoreResult;
use crate::errors::{ErrorType, ErrorTypeDefaults, LocItem, ValError, ValResult};
use crate::lookup_key::LookupPath;
use crate::validators::config::ValBytesMode;
use crate::value::{Dict, Value};

use speedate::{Date, DateTime, Duration, MicrosecondsPrecisionOverflowBehavior, Time};

use super::InputType;
use super::datetime::{
    bytes_as_date, bytes_as_datetime, bytes_as_time, bytes_as_timedelta, date_as_datetime,
    float_as_datetime, float_as_duration, float_as_time, int_as_datetime, int_as_duration,
    int_as_time,
};
use super::input_abstract::{
    BorrowInput, ConsumeIterator, Input, ValMatch, ValidatedDict, ValidatedList, ValidatedTuple,
};
use super::return_enums::{EitherBytes, EitherFloat, EitherInt, EitherString, ValidationMatch};
use super::shared::{
    decimal_as_int, float_as_int, int_as_bool, str_as_bool, str_as_decimal, str_as_float,
    str_as_int, tuple_as_decimal,
};
use crate::decimal::Decimal;
use crate::validators::TemporalUnitMode;

/// Dict keys become location items like Python keys do: strings and ints as themselves
/// (bools are ints), anything else as its repr.
impl From<&Value> for LocItem {
    fn from(value: &Value) -> Self {
        match value {
            Value::Str(s) => s.as_str().into(),
            Value::Int(i) => (*i).into(),
            Value::Bool(b) => i64::from(*b).into(),
            Value::BigInt(b) => b.to_i64().map_or_else(|| b.to_string().into(), Into::into),
            other => other.repr().into(),
        }
    }
}

/// A string from `str` or UTF-8 `bytes` input; invalid UTF-8 is reported as `unicode_error`.
fn maybe_as_string(v: &Value, unicode_error: ErrorType) -> ValResult<Option<&str>> {
    match v {
        Value::Str(s) => Ok(Some(s)),
        Value::Bytes(b) => match from_utf8(b) {
            Ok(s) => Ok(Some(s)),
            Err(_) => Err(ValError::new(unicode_error, v)),
        },
        _ => Ok(None),
    }
}

impl Input for Value {
    fn as_error_value(&self) -> Value {
        self.clone()
    }

    fn as_value(&self) -> Option<&Value> {
        Some(self)
    }

    fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }

    fn validate_str(
        &self,
        strict: bool,
        coerce_numbers_to_str: bool,
    ) -> ValMatch<EitherString<'_>> {
        match self {
            Value::Str(s) => Ok(ValidationMatch::exact(s.as_str().into())),
            Value::Bytes(b) if !strict => match from_utf8(b) {
                Ok(s) => Ok(ValidationMatch::lax(s.into())),
                Err(_) => Err(ValError::new(ErrorTypeDefaults::StringUnicode, self)),
            },
            // Numbers only; bool is deliberately excluded.
            Value::Int(_) | Value::BigInt(_) | Value::Float(_) | Value::Decimal(_)
                if !strict && coerce_numbers_to_str =>
            {
                Ok(ValidationMatch::lax(self.py_str().into()))
            }
            _ => Err(ValError::new(ErrorTypeDefaults::StringType, self)),
        }
    }

    fn validate_bytes(&self, strict: bool, mode: ValBytesMode) -> ValMatch<EitherBytes<'_>> {
        match self {
            Value::Bytes(b) => Ok(ValidationMatch::exact(b.as_slice().into())),
            Value::Str(s) if !strict => match mode.deserialize_string(s) {
                Ok(b) => Ok(ValidationMatch::lax(b)),
                Err(e) => Err(ValError::new(e, self)),
            },
            _ => Err(ValError::new(ErrorTypeDefaults::BytesType, self)),
        }
    }

    fn validate_bool(&self, strict: bool) -> ValMatch<bool> {
        if let Value::Bool(b) = self {
            return Ok(ValidationMatch::exact(*b));
        }
        if !strict {
            if let Some(s) = maybe_as_string(self, ErrorTypeDefaults::BoolParsing)? {
                return str_as_bool(self, s).map(ValidationMatch::lax);
            }
            let as_i64 = match self {
                Value::Int(i) => Some(*i),
                Value::BigInt(b) => b.to_i64(),
                _ => None,
            };
            if let Some(int) = as_i64 {
                return int_as_bool(self, int).map(ValidationMatch::lax);
            }
            let as_f64 = match self {
                Value::Float(f) => Some(*f),
                Value::BigInt(b) => b.to_f64(),
                // `float(decimal)`; Python refuses signaling NaNs
                Value::Decimal(d) => (!d.is_signaling_nan()).then(|| d.to_f64()),
                _ => None,
            };
            if let Some(float) = as_f64
                && let Ok(int) = float_as_int(self, float)
            {
                return int
                    .as_bool()
                    .ok_or_else(|| ValError::new(ErrorTypeDefaults::BoolParsing, self))
                    .map(ValidationMatch::lax);
            }
        }
        Err(ValError::new(ErrorTypeDefaults::BoolType, self))
    }

    fn validate_int(&self, strict: bool) -> ValMatch<EitherInt> {
        match self {
            Value::Int(i) => return Ok(ValidationMatch::exact(EitherInt::I64(*i))),
            Value::BigInt(b) => return Ok(ValidationMatch::exact(EitherInt::BigInt(b.clone()))),
            // bool is a subclass of int in Python: accepted, but only in lax mode.
            Value::Bool(b) if !strict => {
                return Ok(ValidationMatch::lax(EitherInt::I64(i64::from(*b))));
            }
            _ => {}
        }
        if !strict {
            if let Some(s) = maybe_as_string(self, ErrorTypeDefaults::IntParsing)? {
                return str_as_int(self, s).map(ValidationMatch::lax);
            }
            if let Value::Float(f) = self {
                return float_as_int(self, *f).map(ValidationMatch::lax);
            }
            if let Value::Decimal(d) = self {
                return decimal_as_int(self, d).map(ValidationMatch::lax);
            }
        }
        Err(ValError::new(ErrorTypeDefaults::IntType, self))
    }

    fn validate_float(&self, strict: bool) -> ValMatch<EitherFloat> {
        match self {
            Value::Float(f) => Ok(ValidationMatch::exact(EitherFloat::F64(*f))),
            Value::Str(_) | Value::Bytes(_) if !strict => {
                let s = maybe_as_string(self, ErrorTypeDefaults::FloatParsing)?
                    .expect("str and bytes always yield a string");
                str_as_float(self, s).map(ValidationMatch::lax)
            }
            Value::Int(i) => Ok(ValidationMatch::strict(EitherFloat::F64(*i as f64))),
            // Python raises OverflowError for ints beyond the float range.
            Value::BigInt(b) => match b.to_f64() {
                Some(f) if f.is_finite() => Ok(ValidationMatch::strict(EitherFloat::F64(f))),
                _ => Err(ValError::new(ErrorTypeDefaults::FloatType, self)),
            },
            Value::Bool(b) if !strict => Ok(ValidationMatch::lax(EitherFloat::F64(if *b {
                1.0
            } else {
                0.0
            }))),
            // `float(decimal)` works even in strict mode, as upstream extracts an f64
            Value::Decimal(d) if !d.is_signaling_nan() => {
                Ok(ValidationMatch::strict(EitherFloat::F64(d.to_f64())))
            }
            _ => Err(ValError::new(ErrorTypeDefaults::FloatType, self)),
        }
    }

    fn validate_decimal(&self, strict: bool) -> ValMatch<Decimal> {
        match self {
            Value::Decimal(d) => return Ok(ValidationMatch::exact((**d).clone())),
            Value::Str(s) if !strict => return str_as_decimal(self, s).map(ValidationMatch::lax),
            Value::Int(i) if !strict => {
                return Ok(ValidationMatch::lax(Decimal::from_bigint(&(*i).into())));
            }
            Value::BigInt(i) if !strict => {
                return Ok(ValidationMatch::lax(Decimal::from_bigint(i)));
            }
            // through `str(float)`
            Value::Float(_) if !strict => {
                return str_as_decimal(self, &self.py_str()).map(ValidationMatch::lax);
            }
            Value::Tuple(items) if !strict && items.len() == 3 => {
                if let Ok(decimal) = tuple_as_decimal(items) {
                    return Ok(ValidationMatch::lax(decimal));
                }
            }
            _ => {}
        }
        Err(ValError::new(
            if strict {
                ErrorType::IsInstanceOf {
                    class: "Decimal".to_owned(),
                    context: None,
                }
            } else {
                ErrorTypeDefaults::DecimalType
            },
            self,
        ))
    }

    type Dict<'a> = &'a Dict;

    fn strict_dict(&self) -> ValResult<&Dict> {
        match self {
            Value::Dict(d) => Ok(d),
            _ => Err(ValError::new(ErrorTypeDefaults::DictType, self)),
        }
    }

    fn validate_model_fields(&self, strict: bool, from_attributes: bool) -> ValResult<&Dict> {
        if from_attributes {
            // Host data has no objects with attributes yet, so only a dict qualifies; the
            // error hints at `from_attributes` like upstream's.
            match self {
                Value::Dict(d) => Ok(d),
                _ => Err(ValError::new(ErrorTypeDefaults::ModelAttributesType, self)),
            }
        } else {
            self.validate_dict(strict)
        }
    }

    fn validate_date(&self, strict: bool, mode: TemporalUnitMode) -> ValMatch<Date> {
        match self {
            Value::Date(date) => Ok(ValidationMatch::exact(*date)),
            Value::Str(s) if !strict => {
                bytes_as_date(self, s.as_bytes(), mode).map(ValidationMatch::lax)
            }
            Value::Bytes(b) if !strict => bytes_as_date(self, b, mode).map(ValidationMatch::lax),
            // a datetime is a date in Python, but not a valid one here; the date validator
            // converts exact dates from datetimes itself
            _ => Err(ValError::new(ErrorTypeDefaults::DateType, self)),
        }
    }

    fn validate_time(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<Time> {
        match self {
            Value::Time(time) => Ok(ValidationMatch::exact(*time)),
            _ if strict => Err(ValError::new(ErrorTypeDefaults::TimeType, self)),
            Value::Str(s) => bytes_as_time(self, s.as_bytes(), microseconds_overflow_behavior)
                .map(ValidationMatch::lax),
            Value::Bytes(b) => {
                bytes_as_time(self, b, microseconds_overflow_behavior).map(ValidationMatch::lax)
            }
            Value::Int(i) => int_as_time(self, *i, 0).map(ValidationMatch::lax),
            Value::BigInt(i) => float_as_time(self, big_as_f64(i)).map(ValidationMatch::lax),
            Value::Float(f) => float_as_time(self, *f).map(ValidationMatch::lax),
            // `float(decimal)`, as upstream extracts an f64
            Value::Decimal(d) if !d.is_signaling_nan() => {
                float_as_time(self, d.to_f64()).map(ValidationMatch::lax)
            }
            _ => Err(ValError::new(ErrorTypeDefaults::TimeType, self)),
        }
    }

    fn validate_datetime(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
        mode: TemporalUnitMode,
    ) -> ValMatch<DateTime> {
        match self {
            Value::DateTime(dt) => Ok(ValidationMatch::exact(*dt)),
            _ if strict => Err(ValError::new(ErrorTypeDefaults::DatetimeType, self)),
            Value::Str(s) => {
                bytes_as_datetime(self, s.as_bytes(), microseconds_overflow_behavior, mode)
                    .map(ValidationMatch::lax)
            }
            Value::Bytes(b) => bytes_as_datetime(self, b, microseconds_overflow_behavior, mode)
                .map(ValidationMatch::lax),
            Value::Int(i) => int_as_datetime(self, *i, 0, mode).map(ValidationMatch::lax),
            Value::BigInt(i) => {
                float_as_datetime(self, big_as_f64(i), mode).map(ValidationMatch::lax)
            }
            Value::Float(f) => float_as_datetime(self, *f, mode).map(ValidationMatch::lax),
            Value::Decimal(d) if !d.is_signaling_nan() => {
                float_as_datetime(self, d.to_f64(), mode).map(ValidationMatch::lax)
            }
            Value::Date(date) => Ok(ValidationMatch::lax(date_as_datetime(*date))),
            _ => Err(ValError::new(ErrorTypeDefaults::DatetimeType, self)),
        }
    }

    fn validate_timedelta(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<Duration> {
        match self {
            Value::TimeDelta(duration) => Ok(ValidationMatch::exact(duration.clone())),
            _ if strict => Err(ValError::new(ErrorTypeDefaults::TimeDeltaType, self)),
            Value::Str(s) => bytes_as_timedelta(self, s.as_bytes(), microseconds_overflow_behavior)
                .map(ValidationMatch::lax),
            Value::Bytes(b) => bytes_as_timedelta(self, b, microseconds_overflow_behavior)
                .map(ValidationMatch::lax),
            // `bool` is an `int` in Python, and upstream reads it as one here
            Value::Bool(b) => int_as_duration(self, i64::from(*b)).map(ValidationMatch::lax),
            Value::Int(i) => int_as_duration(self, *i).map(ValidationMatch::lax),
            Value::BigInt(i) => float_as_duration(self, big_as_f64(i)).map(ValidationMatch::lax),
            Value::Float(f) => float_as_duration(self, *f).map(ValidationMatch::lax),
            Value::Decimal(d) if !d.is_signaling_nan() => {
                float_as_duration(self, d.to_f64()).map(ValidationMatch::lax)
            }
            _ => Err(ValError::new(ErrorTypeDefaults::TimeDeltaType, self)),
        }
    }

    type List<'a> = &'a [Value];

    fn validate_list(&self, strict: bool) -> ValMatch<&[Value]> {
        match self {
            Value::List(items) => Ok(ValidationMatch::exact(items)),
            Value::Tuple(items) | Value::Set(items) if !strict => Ok(ValidationMatch::lax(items)),
            _ => Err(ValError::new(ErrorTypeDefaults::ListType, self)),
        }
    }

    type Tuple<'a> = &'a [Value];

    fn validate_tuple(&self, strict: bool) -> ValMatch<&[Value]> {
        match self {
            Value::Tuple(items) => Ok(ValidationMatch::exact(items)),
            Value::List(items) | Value::Set(items) if !strict => Ok(ValidationMatch::lax(items)),
            _ => Err(ValError::new(ErrorTypeDefaults::TupleType, self)),
        }
    }

    fn validate_tuple_as(&self, strict: bool, input_type: InputType) -> ValMatch<&[Value]> {
        match self {
            Value::List(items) if input_type == InputType::Perl => {
                Ok(ValidationMatch::exact(items))
            }
            _ => self.validate_tuple(strict),
        }
    }
}

/// Upstream extracts an `int` too large for `i64` as a `float`.
fn big_as_f64(i: &num_bigint::BigInt) -> f64 {
    i.to_f64().unwrap_or(f64::INFINITY)
}

impl BorrowInput for Value {
    type Input = Value;
    fn borrow_input(&self) -> &Self::Input {
        self
    }
}

impl BorrowInput for Cow<'_, Value> {
    type Input = Value;
    fn borrow_input(&self) -> &Self::Input {
        self
    }
}

impl ValidatedDict for &'_ Dict {
    type Key<'a>
        = &'a Value
    where
        Self: 'a;

    type Item<'a>
        = &'a Value
    where
        Self: 'a;

    type PathItem<'a>
        = Cow<'a, Value>
    where
        Self: 'a;

    fn get_item<'a>(&'a self, key: &LookupPath) -> ValResult<Option<Self::PathItem<'a>>> {
        Ok(key.value_get(self))
    }

    fn iterate<'a, R>(
        &'a self,
        consumer: impl ConsumeIterator<ValResult<(Self::Key<'a>, Self::Item<'a>)>, Output = R>,
    ) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.iter().map(Ok)))
    }

    fn last_key(&self) -> Option<Self::Key<'_>> {
        self.iter().last().map(|(k, _)| k)
    }
}

impl<'a> ValidatedList for &'a [Value] {
    type Item = &'a Value;

    fn len(&self) -> Option<usize> {
        Some(<[Value]>::len(self))
    }

    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.iter()))
    }
}

impl<'a> ValidatedTuple for &'a [Value] {
    type Item = &'a Value;

    fn len(&self) -> Option<usize> {
        Some(<[Value]>::len(self))
    }

    fn try_for_each(
        self,
        mut f: impl FnMut(CoreResult<Self::Item>) -> ValResult<()>,
    ) -> ValResult<()> {
        for item in self {
            f(Ok(item))?;
        }
        Ok(())
    }

    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.iter()))
    }
}
