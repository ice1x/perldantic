//! `Input` for host data read in place.
//!
//! A host converting all its data to [`Value`] before validation pays for building a copy that
//! validation reads once and drops. A host implementing [`HostData`] lets validators read its
//! arrays and hashes where they are instead. Everything else (scalars, objects, whatever the
//! host sends as a `Value`) is converted on first use and validated by the `Input` of `Value`,
//! so both ways give the same results.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::fmt;

use speedate::MicrosecondsPrecisionOverflowBehavior;

use crate::decimal::Decimal;
use crate::errors::{LocItem, ValResult};
use crate::lookup_key::LookupPath;
use crate::validators::TemporalUnitMode;
use crate::validators::config::ValBytesMode;
use crate::value::{Dict, Value};

use super::InputType;
use super::input_abstract::{
    Arguments, BorrowInput, ConsumeIterator, Input, ValMatch, ValidatedDict, ValidatedList,
    ValidatedTuple,
};
use super::return_enums::{EitherBytes, EitherFloat, EitherInt, EitherString, ValidationMatch};

/// How validation sees a node of host data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    /// A sequence, read in place: validated as a `list`.
    Array,
    /// A mapping with string keys, read in place: validated as a `dict`.
    Hash,
    /// Anything else, validated as the `Value` the host converts it to.
    Value,
}

/// Access to the data of a host, one node at a time.
pub trait HostData {
    /// A handle on one value of the host, valid for the whole validation call.
    type Node: Copy + fmt::Debug;

    fn kind(&self, node: Self::Node) -> HostKind;

    /// The node as a `Value` (for arrays and hashes: with everything they hold). Conversion
    /// cannot fail here: a host that fails records the error, returns `Value::None` and
    /// reports the error once validation returns.
    fn to_value(&self, node: Self::Node) -> Value;

    fn array_len(&self, node: Self::Node) -> usize;

    fn array_item(&self, node: Self::Node, index: usize) -> Self::Node;

    fn hash_get(&self, node: Self::Node, key: &str) -> Option<Self::Node>;

    /// The entries of a hash, in the order validation should visit them.
    fn hash_entries(&self, node: Self::Node) -> Vec<(String, Self::Node)>;

    /// What identifies the node for recursion guards: equal for the same host value.
    fn identity(&self, node: Self::Node) -> usize;
}

enum Repr<'a, H: HostData> {
    /// A node of the host, with its conversion once made.
    Node(H::Node, OnceCell<Value>),
    Owned(Value),
    Borrowed(&'a Value),
}

/// Host data to validate: a node of the host, or a `Value` found in or made from it.
pub struct HostInput<'a, H: HostData> {
    host: &'a H,
    repr: Repr<'a, H>,
}

impl<'a, H: HostData> HostInput<'a, H> {
    /// The root of the data to validate.
    pub fn new(host: &'a H, node: H::Node) -> Self {
        Self {
            host,
            repr: Repr::Node(node, OnceCell::new()),
        }
    }

    fn owned(host: &'a H, value: Value) -> Self {
        Self {
            host,
            repr: Repr::Owned(value),
        }
    }

    fn borrowed(host: &'a H, value: &'a Value) -> Self {
        Self {
            host,
            repr: Repr::Borrowed(value),
        }
    }

    /// The node and its kind, when this is an array or a hash of the host.
    fn container(&self) -> Option<(H::Node, HostKind)> {
        match &self.repr {
            Repr::Node(node, _) => match self.host.kind(*node) {
                HostKind::Value => None,
                kind => Some((*node, kind)),
            },
            _ => None,
        }
    }

    fn value(&self) -> &Value {
        match &self.repr {
            Repr::Node(node, value) => value.get_or_init(|| self.host.to_value(*node)),
            Repr::Owned(value) => value,
            Repr::Borrowed(value) => value,
        }
    }
}

impl<H: HostData> Clone for HostInput<'_, H> {
    fn clone(&self) -> Self {
        let repr = match &self.repr {
            Repr::Node(node, value) => Repr::Node(*node, value.clone()),
            Repr::Owned(value) => Repr::Owned(value.clone()),
            Repr::Borrowed(value) => Repr::Borrowed(value),
        };
        Self {
            host: self.host,
            repr,
        }
    }
}

impl<H: HostData> fmt::Debug for HostInput<'_, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            Repr::Node(node, _) => write!(f, "HostInput({node:?})"),
            Repr::Owned(value) => write!(f, "HostInput({value:?})"),
            Repr::Borrowed(value) => write!(f, "HostInput({value:?})"),
        }
    }
}

impl<H: HostData> From<HostInput<'_, H>> for LocItem {
    fn from(input: HostInput<'_, H>) -> Self {
        LocItem::from(input.value())
    }
}

impl<H: HostData> BorrowInput for HostInput<'_, H> {
    type Input = Self;
    fn borrow_input(&self) -> &Self::Input {
        self
    }
}

impl<H: HostData> Input for HostInput<'_, H> {
    fn as_error_value(&self) -> Value {
        self.value().clone()
    }

    fn as_value(&self) -> Option<&Value> {
        // arrays and hashes stay in the host; validators that look at host values take them
        // for plain data, which is what they are
        if self.container().is_some() {
            None
        } else {
            Some(self.value())
        }
    }

    fn is_host_data(&self) -> bool {
        true
    }

    fn identity(&self) -> Option<usize> {
        Some(match &self.repr {
            Repr::Node(node, _) => self.host.identity(*node),
            Repr::Owned(value) => std::ptr::from_ref(value) as usize,
            Repr::Borrowed(value) => std::ptr::from_ref(*value) as usize,
        })
    }

    fn is_none(&self) -> bool {
        self.container().is_none() && self.value().is_none()
    }

    fn validate_str(
        &self,
        strict: bool,
        coerce_numbers_to_str: bool,
    ) -> ValMatch<EitherString<'_>> {
        self.value().validate_str(strict, coerce_numbers_to_str)
    }

    fn validate_bytes(&self, strict: bool, mode: ValBytesMode) -> ValMatch<EitherBytes<'_>> {
        self.value().validate_bytes(strict, mode)
    }

    fn validate_bool(&self, strict: bool) -> ValMatch<bool> {
        self.value().validate_bool(strict)
    }

    fn validate_int(&self, strict: bool) -> ValMatch<EitherInt> {
        self.value().validate_int(strict)
    }

    fn validate_float(&self, strict: bool) -> ValMatch<EitherFloat> {
        self.value().validate_float(strict)
    }

    fn validate_decimal(&self, strict: bool) -> ValMatch<Decimal> {
        self.value().validate_decimal(strict)
    }

    fn validate_date(&self, strict: bool, mode: TemporalUnitMode) -> ValMatch<speedate::Date> {
        self.value().validate_date(strict, mode)
    }

    fn validate_time(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<speedate::Time> {
        self.value()
            .validate_time(strict, microseconds_overflow_behavior)
    }

    fn validate_datetime(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
        mode: TemporalUnitMode,
    ) -> ValMatch<speedate::DateTime> {
        self.value()
            .validate_datetime(strict, microseconds_overflow_behavior, mode)
    }

    fn validate_timedelta(
        &self,
        strict: bool,
        microseconds_overflow_behavior: MicrosecondsPrecisionOverflowBehavior,
    ) -> ValMatch<speedate::Duration> {
        self.value()
            .validate_timedelta(strict, microseconds_overflow_behavior)
    }

    type Dict<'b>
        = HostDict<'b, H>
    where
        Self: 'b;

    fn strict_dict(&self) -> ValResult<HostDict<'_, H>> {
        match self.container() {
            Some((node, HostKind::Hash)) => Ok(HostDict::Host(self.host, node)),
            _ => self
                .value()
                .strict_dict()
                .map(|dict| HostDict::Value(self.host, dict)),
        }
    }

    fn lax_dict(&self) -> ValResult<HostDict<'_, H>> {
        match self.container() {
            Some((node, HostKind::Hash)) => Ok(HostDict::Host(self.host, node)),
            _ => self
                .value()
                .lax_dict()
                .map(|dict| HostDict::Value(self.host, dict)),
        }
    }

    fn validate_model_fields(
        &self,
        strict: bool,
        from_attributes: bool,
    ) -> ValResult<HostDict<'_, H>> {
        match self.container() {
            Some((node, HostKind::Hash)) => Ok(HostDict::Host(self.host, node)),
            _ => self
                .value()
                .validate_model_fields(strict, from_attributes)
                .map(|dict| HostDict::Value(self.host, dict)),
        }
    }

    type List<'b>
        = HostSeq<'b, H>
    where
        Self: 'b;

    fn validate_list(&self, strict: bool) -> ValMatch<HostSeq<'_, H>> {
        match self.container() {
            // a host array is a list
            Some((node, HostKind::Array)) => Ok(ValidationMatch::exact(self.host_seq(node))),
            _ => self
                .value()
                .validate_list(strict)
                .map(|m| m.map(|items| HostSeq::Value(self.host, items))),
        }
    }

    fn validate_set(&self, strict: bool, input_type: InputType) -> ValMatch<HostSeq<'_, H>> {
        match self.container() {
            // as a list is (see the `Input` of `Value`)
            Some((node, HostKind::Array)) if input_type == InputType::Perl => {
                Ok(ValidationMatch::strict(self.host_seq(node)))
            }
            Some((node, HostKind::Array)) if !strict => {
                Ok(ValidationMatch::lax(self.host_seq(node)))
            }
            _ => self
                .value()
                .validate_set(strict, input_type)
                .map(|m| m.map(|items| HostSeq::Value(self.host, items))),
        }
    }

    fn validate_frozenset(&self, strict: bool, input_type: InputType) -> ValMatch<HostSeq<'_, H>> {
        match self.container() {
            Some((node, HostKind::Array)) if input_type == InputType::Perl => {
                Ok(ValidationMatch::strict(self.host_seq(node)))
            }
            Some((node, HostKind::Array)) if !strict => {
                Ok(ValidationMatch::lax(self.host_seq(node)))
            }
            _ => self
                .value()
                .validate_frozenset(strict, input_type)
                .map(|m| m.map(|items| HostSeq::Value(self.host, items))),
        }
    }

    type Tuple<'b>
        = HostSeq<'b, H>
    where
        Self: 'b;

    fn validate_tuple(&self, strict: bool) -> ValMatch<HostSeq<'_, H>> {
        match self.container() {
            Some((node, HostKind::Array)) if !strict => {
                Ok(ValidationMatch::lax(self.host_seq(node)))
            }
            _ => self
                .value()
                .validate_tuple(strict)
                .map(|m| m.map(|items| HostSeq::Value(self.host, items))),
        }
    }

    fn validate_tuple_as(&self, strict: bool, input_type: InputType) -> ValMatch<HostSeq<'_, H>> {
        match self.container() {
            Some((node, HostKind::Array)) if input_type == InputType::Perl => {
                Ok(ValidationMatch::exact(self.host_seq(node)))
            }
            _ => self.validate_tuple(strict),
        }
    }

    fn validate_args(&self) -> ValResult<Arguments<HostSeq<'_, H>, HostDict<'_, H>>> {
        match self.container() {
            Some((node, HostKind::Array)) => Ok(Arguments {
                args: Some(self.host_seq(node)),
                kwargs: None,
            }),
            Some((node, HostKind::Hash)) => Ok(Arguments {
                args: None,
                kwargs: Some(HostDict::Host(self.host, node)),
            }),
            _ => self.value().validate_args().map(|arguments| Arguments {
                args: arguments.args.map(|args| HostSeq::Value(self.host, args)),
                kwargs: arguments
                    .kwargs
                    .map(|kwargs| HostDict::Value(self.host, kwargs)),
            }),
        }
    }
}

impl<'a, H: HostData> HostInput<'a, H> {
    fn host_seq(&self, node: H::Node) -> HostSeq<'a, H> {
        HostSeq::Host(self.host, node, self.host.array_len(node))
    }
}

/// A hash to validate as a dict: in the host, or a `Dict` found in or made from host data.
pub enum HostDict<'a, H: HostData> {
    Host(&'a H, H::Node),
    Value(&'a H, &'a Dict),
}

impl<H: HostData> ValidatedDict for HostDict<'_, H> {
    type Key<'b>
        = HostInput<'b, H>
    where
        Self: 'b;

    type Item<'b>
        = HostInput<'b, H>
    where
        Self: 'b;

    type PathItem<'b>
        = HostInput<'b, H>
    where
        Self: 'b;

    fn get_item<'b>(&'b self, key: &LookupPath) -> ValResult<Option<HostInput<'b, H>>> {
        Ok(match *self {
            HostDict::Host(host, node) => {
                let Some(first) = host.hash_get(node, key.first_key()) else {
                    return Ok(None);
                };
                if key.rest().is_empty() {
                    Some(HostInput::new(host, first))
                } else {
                    // paths into nested data are rare: follow them in the converted value
                    key.rest_get(&host.to_value(first))
                        .map(|value| HostInput::owned(host, value))
                }
            }
            HostDict::Value(host, dict) => key.value_get(dict).map(|value| match value {
                Cow::Borrowed(value) => HostInput::borrowed(host, value),
                Cow::Owned(value) => HostInput::owned(host, value),
            }),
        })
    }

    fn iterate<'b, R>(
        &'b self,
        consumer: impl ConsumeIterator<ValResult<(HostInput<'b, H>, HostInput<'b, H>)>, Output = R>,
    ) -> ValResult<R> {
        Ok(match *self {
            HostDict::Host(host, node) => {
                consumer.consume_iterator(host.hash_entries(node).into_iter().map(|(key, item)| {
                    Ok((
                        HostInput::owned(host, Value::Str(key)),
                        HostInput::new(host, item),
                    ))
                }))
            }
            HostDict::Value(host, dict) => consumer
                .consume_iterator(dict.iter().map(|(k, v)| {
                    Ok((HostInput::borrowed(host, k), HostInput::borrowed(host, v)))
                })),
        })
    }

    fn last_key(&self) -> Option<HostInput<'_, H>> {
        match *self {
            HostDict::Host(host, node) => host
                .hash_entries(node)
                .pop()
                .map(|(key, _)| HostInput::owned(host, Value::Str(key))),
            HostDict::Value(host, dict) => dict
                .iter()
                .last()
                .map(|(k, _)| HostInput::borrowed(host, k)),
        }
    }
}

/// An array to validate as a list or tuple: in the host (with its length), or items found in
/// or made from host data.
pub enum HostSeq<'a, H: HostData> {
    Host(&'a H, H::Node, usize),
    Value(&'a H, &'a [Value]),
}

impl<'a, H: HostData> HostSeq<'a, H> {
    fn length(&self) -> usize {
        match self {
            HostSeq::Host(_, _, len) => *len,
            HostSeq::Value(_, items) => <[Value]>::len(items),
        }
    }

    fn items(self) -> impl Iterator<Item = HostInput<'a, H>> {
        let (host, node, len, values) = match self {
            HostSeq::Host(host, node, len) => (host, Some(node), len, [].as_slice()),
            HostSeq::Value(host, items) => (host, None, 0, items),
        };
        (0..len)
            .map(move |i| HostInput::new(host, host.array_item(node.expect("a host array"), i)))
            .chain(values.iter().map(move |v| HostInput::borrowed(host, v)))
    }
}

impl<'a, H: HostData> ValidatedList for HostSeq<'a, H> {
    type Item = HostInput<'a, H>;

    fn len(&self) -> Option<usize> {
        Some(self.length())
    }

    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.items()))
    }
}

impl<'a, H: HostData> ValidatedTuple for HostSeq<'a, H> {
    type Item = HostInput<'a, H>;

    fn len(&self) -> Option<usize> {
        Some(self.length())
    }

    fn try_for_each(
        self,
        mut f: impl FnMut(crate::core_error::CoreResult<Self::Item>) -> ValResult<()>,
    ) -> ValResult<()> {
        for item in self.items() {
            f(Ok(item))?;
        }
        Ok(())
    }

    fn iterate<R>(self, consumer: impl ConsumeIterator<Self::Item, Output = R>) -> ValResult<R> {
        Ok(consumer.consume_iterator(self.items()))
    }
}
