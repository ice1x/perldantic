//! `Input` for parsed JSON and for bare strings (JSON object keys).
//! Port of the P0 parts of upstream `input/input_json.rs`.

use jiter::{JsonArray, JsonObject, JsonValue};

use crate::lookup_key::LookupPath;
use num_traits::cast::ToPrimitive;

use crate::core_error::CoreResult;
use speedate::{Date, DateTime, Duration, MicrosecondsPrecisionOverflowBehavior, Time};
use strum::EnumMessage;

use crate::errors::{ErrorType, ErrorTypeDefaults, ValError, ValResult};
use crate::validators::TemporalUnitMode;
use crate::validators::config::ValBytesMode;
use crate::value::Value;

use super::InputType;

use super::datetime::{
    bytes_as_date, bytes_as_datetime, bytes_as_time, bytes_as_timedelta, float_as_datetime,
    float_as_duration, float_as_time, int_as_datetime, int_as_duration, int_as_time,
};
use super::input_abstract::{
    BorrowInput, ConsumeIterator, Input, Never, ValMatch, ValidatedDict, ValidatedList,
    ValidatedTuple,
};
use super::return_enums::{EitherBytes, EitherFloat, EitherInt, EitherString, ValidationMatch};
use super::shared::{
    float_as_int, int_as_bool, str_as_bool, str_as_decimal, str_as_float, str_as_int,
    tuple_as_decimal,
};
use crate::core_error::CoreError;
use crate::decimal::Decimal;

impl<'data> Input for JsonValue<'data> {
    fn as_json(&self) -> Option<&JsonValue<'_>> {
        Some(self)
    }

    fn as_error_value(&self) -> Value {
        Value::from(self)
    }

    fn is_none(&self) -> bool {
        matches!(self, JsonValue::Null)
    }

    fn validate_str(
        &self,
        strict: bool,
        coerce_numbers_to_str: bool,
    ) -> ValMatch<EitherString<'_>> {
        // `strict`, not `exact`: in JSON a string also stands for other types (UUID, date, ...),
        // so a JSON string is a converting input.
        match self {
            JsonValue::Str(s) => Ok(ValidationMatch::strict(s.as_ref().into())),
            JsonValue::Int(i) if !strict && coerce_numbers_to_str => {
                Ok(ValidationMatch::lax(i.to_string().into()))
            }
            JsonValue::BigInt(b) if !strict && coerce_numbers_to_str => {
                Ok(ValidationMatch::lax(b.to_string().into()))
            }
            JsonValue::Float(f) if !strict && coerce_numbers_to_str => {
                Ok(ValidationMatch::lax(f.to_string().into()))
            }
            _ => Err(ValError::new(ErrorTypeDefaults::StringType, self)),
        }
    }

    fn exact_str(&self) -> ValResult<EitherString<'_>> {
        match self {
            JsonValue::Str(s) => Ok(s.as_ref().into()),
            _ => Err(ValError::new(ErrorTypeDefaults::StringType, self)),
        }
    }

    fn validate_bytes(&self, _strict: bool, mode: ValBytesMode) -> ValMatch<EitherBytes<'_>> {
        match self {
            JsonValue::Str(s) => match mode.deserialize_string(s) {
                Ok(b) => Ok(ValidationMatch::strict(b)),
                Err(e) => Err(ValError::new(e, self)),
            },
            _ => Err(ValError::new(ErrorTypeDefaults::BytesType, self)),
        }
    }

    fn validate_bool(&self, strict: bool) -> ValMatch<bool> {
        match self {
            JsonValue::Bool(b) => Ok(ValidationMatch::exact(*b)),
            JsonValue::Str(s) if !strict => str_as_bool(self, s).map(ValidationMatch::lax),
            JsonValue::Int(int) if !strict => int_as_bool(self, *int).map(ValidationMatch::lax),
            JsonValue::Float(float) if !strict => match float_as_int(self, *float) {
                Ok(int) => int
                    .as_bool()
                    .ok_or_else(|| ValError::new(ErrorTypeDefaults::BoolParsing, self))
                    .map(ValidationMatch::lax),
                _ => Err(ValError::new(ErrorTypeDefaults::BoolType, self)),
            },
            _ => Err(ValError::new(ErrorTypeDefaults::BoolType, self)),
        }
    }

    fn validate_int(&self, strict: bool) -> ValMatch<EitherInt> {
        match self {
            JsonValue::Int(i) => Ok(ValidationMatch::exact(EitherInt::I64(*i))),
            JsonValue::BigInt(b) => Ok(ValidationMatch::exact(EitherInt::BigInt(b.clone()))),
            JsonValue::Bool(b) if !strict => Ok(ValidationMatch::lax(EitherInt::I64((*b).into()))),
            JsonValue::Float(f) if !strict => float_as_int(self, *f).map(ValidationMatch::lax),
            JsonValue::Str(str) if !strict => str_as_int(self, str).map(ValidationMatch::lax),
            _ => Err(ValError::new(ErrorTypeDefaults::IntType, self)),
        }
    }

    fn validate_date(&self, _strict: bool, mode: TemporalUnitMode) -> ValMatch<Date> {
        match self {
            JsonValue::Str(v) => {
                bytes_as_date(self, v.as_bytes(), mode).map(ValidationMatch::strict)
            }
            _ => Err(ValError::new(ErrorTypeDefaults::DateType, self)),
        }
    }

    fn validate_time(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<Time> {
        match self {
            JsonValue::Str(v) => bytes_as_time(self, v.as_bytes(), microseconds_overflow_behavior)
                .map(ValidationMatch::strict),
            JsonValue::Int(v) if !strict => int_as_time(self, *v, 0).map(ValidationMatch::lax),
            JsonValue::Float(v) if !strict => float_as_time(self, *v).map(ValidationMatch::lax),
            JsonValue::BigInt(_) if !strict => Err(ValError::new(
                ErrorType::TimeParsing {
                    error: speedate::ParseError::TimeTooLarge
                        .get_documentation()
                        .unwrap_or_default()
                        .to_owned(),
                    context: None,
                },
                self,
            )),
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
            JsonValue::Str(v) => {
                bytes_as_datetime(self, v.as_bytes(), microseconds_overflow_behavior, mode)
                    .map(ValidationMatch::strict)
            }
            JsonValue::Int(v) if !strict => {
                int_as_datetime(self, *v, 0, mode).map(ValidationMatch::lax)
            }
            JsonValue::Float(v) if !strict => {
                float_as_datetime(self, *v, mode).map(ValidationMatch::lax)
            }
            _ => Err(ValError::new(ErrorTypeDefaults::DatetimeType, self)),
        }
    }

    fn validate_timedelta(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<Duration> {
        match self {
            JsonValue::Str(v) => {
                bytes_as_timedelta(self, v.as_bytes(), microseconds_overflow_behavior)
                    .map(ValidationMatch::strict)
            }
            JsonValue::Int(v) if !strict => int_as_duration(self, *v).map(ValidationMatch::lax),
            JsonValue::Float(v) if !strict => float_as_duration(self, *v).map(ValidationMatch::lax),
            _ => Err(ValError::new(ErrorTypeDefaults::TimeDeltaType, self)),
        }
    }

    fn validate_float(&self, strict: bool) -> ValMatch<EitherFloat> {
        match self {
            JsonValue::Float(f) => Ok(ValidationMatch::exact(EitherFloat::F64(*f))),
            JsonValue::Int(i) => Ok(ValidationMatch::strict(EitherFloat::F64(*i as f64))),
            JsonValue::BigInt(b) => Ok(ValidationMatch::strict(EitherFloat::F64(
                b.to_f64().expect("BigInt should always return some value"),
            ))),
            JsonValue::Bool(b) if !strict => Ok(ValidationMatch::lax(EitherFloat::F64(if *b {
                1.0
            } else {
                0.0
            }))),
            JsonValue::Str(str) if !strict => str_as_float(self, str).map(ValidationMatch::lax),
            _ => Err(ValError::new(ErrorTypeDefaults::FloatType, self)),
        }
    }

    fn validate_decimal(&self, strict: bool) -> ValMatch<Decimal> {
        match self {
            // through Rust's formatting of the float, as upstream does
            JsonValue::Float(f) => {
                str_as_decimal(self, &f.to_string()).map(ValidationMatch::strict)
            }
            JsonValue::Str(s) => str_as_decimal(self, s).map(ValidationMatch::strict),
            JsonValue::Int(i) => Ok(ValidationMatch::strict(Decimal::from_bigint(&(*i).into()))),
            JsonValue::BigInt(i) => Ok(ValidationMatch::strict(Decimal::from_bigint(i))),
            JsonValue::Array(array) if !strict && array.as_slice().len() == 3 => {
                let items: Vec<Value> = array.iter().map(Value::from).collect();
                // upstream lets Python's ValueError escape
                tuple_as_decimal(&items)
                    .map(ValidationMatch::lax)
                    .map_err(|e| ValError::InternalErr(CoreError::Value(e.0.to_owned())))
            }
            _ => Err(ValError::new(ErrorTypeDefaults::DecimalType, self)),
        }
    }

    type Dict<'a>
        = &'a JsonObject<'data>
    where
        Self: 'a;

    fn validate_dict(&self, _strict: bool) -> ValResult<Self::Dict<'_>> {
        match self {
            JsonValue::Object(dict) => Ok(dict),
            _ => Err(ValError::new(ErrorTypeDefaults::DictType, self)),
        }
    }

    fn strict_dict(&self) -> ValResult<Self::Dict<'_>> {
        self.validate_dict(false)
    }

    type List<'a>
        = &'a JsonArray<'data>
    where
        Self: 'a;

    fn validate_list(&self, _strict: bool) -> ValMatch<&JsonArray<'data>> {
        match self {
            JsonValue::Array(a) => Ok(ValidationMatch::exact(a)),
            _ => Err(ValError::new(ErrorTypeDefaults::ListType, self)),
        }
    }

    // a list is allowed here, since otherwise a set could not be created from JSON
    fn validate_set(&self, _strict: bool, _input_type: InputType) -> ValMatch<&JsonArray<'data>> {
        match self {
            JsonValue::Array(a) => Ok(ValidationMatch::strict(a)),
            _ => Err(ValError::new(ErrorTypeDefaults::SetType, self)),
        }
    }

    fn validate_frozenset(
        &self,
        _strict: bool,
        _input_type: InputType,
    ) -> ValMatch<&JsonArray<'data>> {
        match self {
            JsonValue::Array(a) => Ok(ValidationMatch::strict(a)),
            _ => Err(ValError::new(ErrorTypeDefaults::FrozenSetType, self)),
        }
    }

    type Tuple<'a>
        = &'a JsonArray<'data>
    where
        Self: 'a;

    fn validate_tuple(&self, _strict: bool) -> ValMatch<&JsonArray<'data>> {
        // A JSON array is the only way to express a tuple, so it is accepted in strict mode.
        match self {
            JsonValue::Array(a) => Ok(ValidationMatch::strict(a)),
            _ => Err(ValError::new(ErrorTypeDefaults::TupleType, self)),
        }
    }
}

impl<'data> BorrowInput for JsonValue<'data> {
    type Input = JsonValue<'data>;
    fn borrow_input(&self) -> &Self::Input {
        self
    }
}

/// A bare string: a JSON object key, or string-only data such as environment variables.
impl Input for str {
    fn as_error_value(&self) -> Value {
        Value::Str(self.to_owned())
    }

    fn validate_str(
        &self,
        _strict: bool,
        _coerce_numbers_to_str: bool,
    ) -> ValMatch<EitherString<'_>> {
        Ok(ValidationMatch::strict(self.into()))
    }

    fn validate_bytes(&self, _strict: bool, mode: ValBytesMode) -> ValMatch<EitherBytes<'_>> {
        match mode.deserialize_string(self) {
            Ok(b) => Ok(ValidationMatch::strict(b)),
            Err(e) => Err(ValError::new(e, self)),
        }
    }

    fn validate_bool(&self, _strict: bool) -> ValMatch<bool> {
        str_as_bool(self, self).map(ValidationMatch::lax)
    }

    fn validate_int(&self, _strict: bool) -> ValMatch<EitherInt> {
        str_as_int(self, self).map(ValidationMatch::lax)
    }

    fn validate_date(&self, _strict: bool, mode: TemporalUnitMode) -> ValMatch<Date> {
        bytes_as_date(self, self.as_bytes(), mode).map(ValidationMatch::lax)
    }

    fn validate_time(
        &self,
        _strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<Time> {
        bytes_as_time(self, self.as_bytes(), microseconds_overflow_behavior)
            .map(ValidationMatch::lax)
    }

    fn validate_datetime(
        &self,
        _strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
        mode: TemporalUnitMode,
    ) -> ValMatch<DateTime> {
        bytes_as_datetime(self, self.as_bytes(), microseconds_overflow_behavior, mode)
            .map(ValidationMatch::lax)
    }

    fn validate_timedelta(
        &self,
        _strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<Duration> {
        bytes_as_timedelta(self, self.as_bytes(), microseconds_overflow_behavior)
            .map(ValidationMatch::lax)
    }

    fn validate_float(&self, _strict: bool) -> ValMatch<EitherFloat> {
        str_as_float(self, self).map(ValidationMatch::lax)
    }

    fn validate_decimal(&self, _strict: bool) -> ValMatch<Decimal> {
        str_as_decimal(self, self).map(ValidationMatch::strict)
    }

    type Dict<'a> = Never;

    fn strict_dict(&self) -> ValResult<Never> {
        Err(ValError::new(ErrorTypeDefaults::DictType, self))
    }

    type List<'a> = Never;

    fn validate_list(&self, _strict: bool) -> ValMatch<Never> {
        Err(ValError::new(ErrorTypeDefaults::ListType, self))
    }

    fn validate_set(&self, _strict: bool, _input_type: InputType) -> ValMatch<Never> {
        Err(ValError::new(ErrorTypeDefaults::SetType, self))
    }

    // upstream reports a set error here too
    fn validate_frozenset(&self, _strict: bool, _input_type: InputType) -> ValMatch<Never> {
        Err(ValError::new(ErrorTypeDefaults::SetType, self))
    }

    type Tuple<'a> = Never;

    fn validate_tuple(&self, _strict: bool) -> ValMatch<Never> {
        Err(ValError::new(ErrorTypeDefaults::TupleType, self))
    }
}

impl BorrowInput for String {
    type Input = str;
    fn borrow_input(&self) -> &Self::Input {
        self
    }
}

impl<'data> ValidatedDict for &'_ JsonObject<'data> {
    type Key<'a>
        = &'a str
    where
        Self: 'a;

    type Item<'a>
        = &'a JsonValue<'data>
    where
        Self: 'a;

    type PathItem<'a>
        = &'a JsonValue<'data>
    where
        Self: 'a;

    fn get_item<'a>(&'a self, key: &LookupPath) -> ValResult<Option<Self::PathItem<'a>>> {
        Ok(key.json_get(self))
    }

    fn iterate<'a, R>(
        &'a self,
        consumer: impl ConsumeIterator<ValResult<(Self::Key<'a>, Self::Item<'a>)>, Output = R>,
    ) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.as_slice().iter().map(|(k, v)| Ok((k.as_ref(), v)))))
    }

    fn last_key(&self) -> Option<Self::Key<'_>> {
        self.last().map(|(k, _)| k.as_ref())
    }
}

impl<'a, 'data> ValidatedList for &'a JsonArray<'data> {
    type Item = &'a JsonValue<'data>;

    fn len(&self) -> Option<usize> {
        Some(Vec::len(self))
    }

    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.iter()))
    }
}

impl<'a, 'data> ValidatedTuple for &'a JsonArray<'data> {
    type Item = &'a JsonValue<'data>;

    fn len(&self) -> Option<usize> {
        Some(Vec::len(self))
    }

    fn try_for_each(
        self,
        mut f: impl FnMut(CoreResult<Self::Item>) -> ValResult<()>,
    ) -> ValResult<()> {
        for item in self.iter() {
            f(Ok(item))?;
        }
        Ok(())
    }

    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.iter()))
    }
}
