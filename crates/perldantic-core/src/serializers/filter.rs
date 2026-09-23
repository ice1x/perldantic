//! `include` / `exclude` filters. Port of upstream `serializers/filter.rs`.
//!
//! Filters are sets or (nested) dicts of keys and indices, as in pydantic. Python's `...` and
//! `True` both mean "the whole item"; hosts pass `true`. Membership follows Python equality.

use num_bigint::BigInt;
use num_traits::Signed;

use crate::build_tools::SchemaDict;
use crate::core_error::{CoreError, CoreResult};
use crate::value::{Dict, Value};

use super::errors::SerResult;
use super::extra::{IncludeExclude, SerializationState};

const ALL_KEY: &str = "__all__";

#[derive(Debug, Clone)]
pub(crate) struct SchemaFilter<T> {
    include: Option<Vec<T>>,
    exclude: Option<Vec<T>>,
}

// Manual impl: #[derive(Default)] would require T: Default.
impl<T> Default for SchemaFilter<T> {
    fn default() -> Self {
        Self {
            include: None,
            exclude: None,
        }
    }
}

/// Python's `index % len` for an index given in a filter, so `-1` means the last item.
fn map_negative_index(value: &Value, len: Option<usize>) -> CoreResult<Value> {
    match len {
        Some(len) => Ok(match value {
            Value::Int(i) => Value::from(BigInt::from(*i).mod_floor_py(len)),
            Value::BigInt(i) => Value::from(i.mod_floor_py(len)),
            Value::Bool(b) => Value::from(BigInt::from(u8::from(*b)).mod_floor_py(len)),
            other => other.clone(),
        }),
        None => {
            let negative = match value {
                Value::Int(i) => *i < 0,
                Value::BigInt(i) => i.is_negative(),
                _ => false,
            };
            if negative {
                Err(CoreError::Value(
                    "Negative indices cannot be used to exclude items on unsized iterables".into(),
                ))
            } else {
                Ok(value.clone())
            }
        }
    }
}

trait ModFloorPy {
    fn mod_floor_py(&self, len: usize) -> BigInt;
}

impl ModFloorPy for BigInt {
    fn mod_floor_py(&self, len: usize) -> BigInt {
        if len == 0 {
            // `x % 0` raises in Python and upstream then keeps the value as is
            return self.clone();
        }
        let len = BigInt::from(len);
        ((self % &len) + &len) % &len
    }
}

fn map_negative_indices(include_or_exclude: &Value, len: Option<usize>) -> CoreResult<Value> {
    match include_or_exclude {
        Value::Dict(exclude_dict) => {
            let mut out = Dict::new();
            for (k, v) in exclude_dict.iter() {
                out.insert(map_negative_index(k, len)?, v.clone());
            }
            Ok(Value::Dict(out))
        }
        Value::Set(exclude_set) => {
            let mut values = Vec::with_capacity(exclude_set.len());
            for v in exclude_set {
                let mapped = map_negative_index(v, len)?;
                if !values.iter().any(|x: &Value| x.py_eq(&mapped)) {
                    values.push(mapped);
                }
            }
            Ok(Value::Set(values))
        }
        // return as is and deal with the error later
        other => Ok(other.clone()),
    }
}

type NextFilters = Option<IncludeExclude>;

impl SchemaFilter<usize> {
    /// Filters given in a list, tuple or dict schema's `serialization`.
    pub fn from_schema(schema: &Dict) -> CoreResult<Self> {
        match schema.get_as::<Dict>("serialization")? {
            Some(ser) => {
                let include = Self::build_set_ints(ser.get_str("include"))?;
                let exclude = Self::build_set_ints(ser.get_str("exclude"))?;
                Ok(Self { include, exclude })
            }
            None => Ok(SchemaFilter::default()),
        }
    }

    fn build_set_ints(v: Option<&Value>) -> CoreResult<Option<Vec<usize>>> {
        match v {
            None | Some(Value::None) => Ok(None),
            Some(Value::Set(items)) => {
                let mut set = Vec::with_capacity(items.len());
                for item in items {
                    let index = match item {
                        Value::Int(i) => usize::try_from(*i).map_err(|_| {
                            CoreError::Value("can't convert negative int to unsigned".into())
                        })?,
                        other => {
                            return Err(CoreError::Type(format!(
                                "'{}' object cannot be interpreted as an integer",
                                other.type_name()
                            )));
                        }
                    };
                    set.push(index);
                }
                Ok(Some(set))
            }
            Some(other) => Err(CoreError::Type(format!(
                "'{}' object is not an instance of 'set'",
                other.type_name()
            ))),
        }
    }

    pub fn index_filter(
        &self,
        index: usize,
        state: &SerializationState,
        len: Option<usize>,
    ) -> SerResult<NextFilters> {
        let include = state
            .include()
            .map(|v| map_negative_indices(v, len))
            .transpose()?;
        let exclude = state
            .exclude()
            .map(|v| map_negative_indices(v, len))
            .transpose()?;
        self.filter(
            &Value::from(index),
            &index,
            include.as_ref(),
            exclude.as_ref(),
        )
    }
}

impl SchemaFilter<Value> {
    /// Filters given as sets of keys, e.g. a dict schema's `serialization`.
    pub fn from_set_hash(include: Option<&Value>, exclude: Option<&Value>) -> CoreResult<Self> {
        Ok(Self {
            include: Self::build_set_hashes(include)?,
            exclude: Self::build_set_hashes(exclude)?,
        })
    }

    fn build_set_hashes(v: Option<&Value>) -> CoreResult<Option<Vec<Value>>> {
        match v {
            None | Some(Value::None) => Ok(None),
            Some(Value::Set(items)) => Ok(Some(items.clone())),
            Some(other) => Err(CoreError::Type(format!(
                "'{}' object is not an instance of 'set'",
                other.type_name()
            ))),
        }
    }

    pub fn key_filter(&self, key: &Value, state: &SerializationState) -> SerResult<NextFilters> {
        self.filter(key, key, state.include(), state.exclude())
    }
}

/// Membership test for the keys a filter holds.
trait FilterKey {
    fn same(&self, other: &Self) -> bool;
}

impl FilterKey for usize {
    fn same(&self, other: &Self) -> bool {
        self == other
    }
}

impl FilterKey for Value {
    fn same(&self, other: &Self) -> bool {
        self.py_eq(other)
    }
}

trait FilterLogic<T: ?Sized> {
    /// whether an `index`/`key` is explicitly included, this is combined with call-time `include` below
    fn explicit_include(&self, value: &T) -> bool;

    /// default decision on whether to include the item at a given `index`/`key`
    fn default_filter(&self, value: &T) -> bool;

    /// this is the somewhat hellish logic for deciding:
    /// 1. whether we should omit a value at a particular index/key - returning `Ok(None)` here
    /// 2. or include it, in which case, what values of `include` and `exclude` should be passed to it
    fn filter(
        &self,
        py_key: &Value,
        int_key: &T,
        include: Option<&Value>,
        exclude: Option<&Value>,
    ) -> SerResult<NextFilters> {
        let mut next_exclude = None;
        if let Some(exclude) = exclude {
            match exclude {
                // Do nothing; place this check at the top for performance in the common case
                Value::None => {}
                Value::Dict(exclude_dict) => {
                    if let Some(exc_value) = merge_all_value(exclude_dict, py_key)? {
                        if is_ellipsis_like(&exc_value) {
                            // if the index is in exclude, and the exclude value is `None`, we want to omit this index/item
                            return Ok(None);
                        }
                        // if the index is in exclude, and the exclude-value is not `None`,
                        // we want to return `Some((..., Some(next_exclude))`
                        next_exclude = Some(exc_value);
                    }
                }
                Value::Set(exclude_set) => {
                    if set_contains(exclude_set, py_key) || set_contains_all(exclude_set) {
                        // index is in the exclude set, we return Ok(None) to omit this index
                        return Ok(None);
                    }
                }
                other => match check_contains(other, py_key) {
                    Some(true) => return Ok(None),
                    Some(false) => {}
                    None => {
                        return Err(CoreError::Type(
                            "`exclude` argument must be a set or dict.".into(),
                        )
                        .into());
                    }
                },
            }
        }

        if let Some(include) = include {
            match include {
                // Do nothing; place this check at the top for performance in the common case
                Value::None => {}
                Value::Dict(include_dict) => {
                    if let Some(inc_value) = merge_all_value(include_dict, py_key)? {
                        // if the index is in include, we definitely want to include this index
                        return if is_ellipsis_like(&inc_value) {
                            Ok(Some(IncludeExclude::new(None, next_exclude)))
                        } else {
                            Ok(Some(IncludeExclude::new(Some(inc_value), next_exclude)))
                        };
                    } else if !self.explicit_include(int_key) {
                        // if the index is not in include, include exists, AND it's not in schema include,
                        // this index should be omitted
                        return Ok(None);
                    }
                }
                Value::Set(include_set) => {
                    if set_contains(include_set, py_key) || set_contains_all(include_set) {
                        return Ok(Some(IncludeExclude::new(None, next_exclude)));
                    } else if !self.explicit_include(int_key) {
                        // if the index is not in include, include exists, AND it's not in schema include,
                        // this index should be omitted
                        return Ok(None);
                    }
                }
                other => match check_contains(other, py_key) {
                    Some(true) => return Ok(Some(IncludeExclude::new(None, next_exclude))),
                    Some(false) if !self.explicit_include(int_key) => return Ok(None),
                    Some(false) => {}
                    None => {
                        return Err(CoreError::Type(
                            "`include` argument must be a set or dict.".into(),
                        )
                        .into());
                    }
                },
            }
        }

        if next_exclude.is_some() {
            Ok(Some(IncludeExclude::new(None, next_exclude)))
        } else if self.default_filter(int_key) {
            Ok(Some(IncludeExclude::empty()))
        } else {
            Ok(None)
        }
    }
}

impl<T: FilterKey> FilterLogic<T> for SchemaFilter<T> {
    fn explicit_include(&self, value: &T) -> bool {
        match self.include {
            Some(ref include) => include.iter().any(|v| v.same(value)),
            None => false,
        }
    }

    fn default_filter(&self, value: &T) -> bool {
        let contains = |set: &Vec<T>| set.iter().any(|v| v.same(value));
        match (&self.include, &self.exclude) {
            (Some(include), Some(exclude)) => contains(include) && !contains(exclude),
            (Some(include), None) => contains(include),
            (None, Some(exclude)) => !contains(exclude),
            (None, None) => true,
        }
    }
}

/// A filter with no schema-level include or exclude, used when serializing by inference.
#[derive(Debug, Clone)]
pub(crate) struct AnyFilter;

impl AnyFilter {
    pub fn new() -> Self {
        AnyFilter {}
    }

    pub fn key_filter(&self, key: &Value, state: &SerializationState) -> SerResult<NextFilters> {
        // just use 0 for the int_key, it's always ignored in the implementation here
        self.filter(key, &0, state.include(), state.exclude())
    }

    pub fn index_filter(
        &self,
        index: usize,
        state: &SerializationState,
        len: Option<usize>,
    ) -> SerResult<NextFilters> {
        let include = state
            .include()
            .map(|v| map_negative_indices(v, len))
            .transpose()?;
        let exclude = state
            .exclude()
            .map(|v| map_negative_indices(v, len))
            .transpose()?;
        self.filter(
            &Value::from(index),
            &index,
            include.as_ref(),
            exclude.as_ref(),
        )
    }
}

impl FilterLogic<usize> for AnyFilter {
    fn explicit_include(&self, _value: &usize) -> bool {
        false
    }

    fn default_filter(&self, _value: &usize) -> bool {
        true
    }
}

fn set_contains(set: &[Value], key: &Value) -> bool {
    set.iter().any(|v| v.py_eq(key))
}

fn set_contains_all(set: &[Value]) -> bool {
    set.iter()
        .any(|v| matches!(v, Value::Str(s) if s == ALL_KEY))
}

/// Python's `key in obj or '__all__' in obj` for other containers, `None` where Python's
/// `__contains__` would fail or not exist.
fn check_contains(obj: &Value, key: &Value) -> Option<bool> {
    let all = Value::from(ALL_KEY);
    match obj {
        Value::List(items) | Value::Tuple(items) => {
            Some(set_contains(items, key) || set_contains(items, &all))
        }
        // `in` on a string is a substring test, and fails for non-string keys
        Value::Str(s) => match key {
            Value::Str(k) => Some(s.contains(k.as_str()) || s.contains(ALL_KEY)),
            _ => None,
        },
        _ => None,
    }
}

/// detect both ellipsis and `True` to be compatible with pydantic V1
fn is_ellipsis_like(v: &Value) -> bool {
    matches!(v, Value::Bool(true))
}

fn dict_get<'a>(dict: &'a Dict, key: &Value) -> Option<&'a Value> {
    dict.iter().find(|(k, _)| k.py_eq(key)).map(|(_, v)| v)
}

/// lookup the dict, for the key and "__all__" key, and merge them following the same rules as pydantic V1
fn merge_all_value(dict: &Dict, py_key: &Value) -> CoreResult<Option<Value>> {
    let op_item_value = dict_get(dict, py_key);
    let op_all_value = dict.get_str(ALL_KEY);

    match (op_item_value, op_all_value) {
        (Some(item_value), Some(all_value)) => {
            if is_ellipsis_like(item_value) || is_ellipsis_like(all_value) {
                Ok(Some(item_value.clone()))
            } else {
                let item_dict = as_dict(item_value)?;
                let item_dict_merged = merge_dicts(&item_dict, all_value)?;
                Ok(Some(Value::Dict(item_dict_merged)))
            }
        }
        (Some(item_value), None) => Ok(Some(item_value.clone())),
        (None, Some(all_value)) => Ok(Some(all_value.clone())),
        (None, None) => Ok(None),
    }
}

fn as_dict(value: &Value) -> CoreResult<Dict> {
    match value {
        Value::Dict(dict) => Ok(dict.clone()),
        Value::Set(items) => Ok(items
            .iter()
            .map(|item| (item.clone(), Value::Bool(true)))
            .collect()),
        _ => Err(CoreError::Type(
            "`include` and `exclude` must be of type `dict[str | int, <recursive> | ...] | set[str | int | ...]`".into(),
        )),
    }
}

fn merge_dicts(item_dict: &Dict, all_value: &Value) -> CoreResult<Dict> {
    let mut item_dict = item_dict.clone();
    match all_value {
        Value::Dict(all_dict) => {
            for (all_key, all_value) in all_dict.iter() {
                if let Some(item_value) = dict_get(&item_dict, all_key).cloned() {
                    if is_ellipsis_like(&item_value) {
                        continue;
                    }
                    let item_value_dict = as_dict(&item_value)?;
                    // if the all value is an ellipsis, we don't overwrite the item value
                    if !is_ellipsis_like(all_value) {
                        item_dict.insert(
                            all_key.clone(),
                            Value::Dict(merge_dicts(&item_value_dict, all_value)?),
                        );
                    }
                } else {
                    item_dict.insert(all_key.clone(), all_value.clone());
                }
            }
        }
        Value::Set(set) => {
            for item in set {
                if dict_get(&item_dict, item).is_none() {
                    item_dict.insert(item.clone(), Value::Bool(true));
                }
            }
        }
        _ => {
            return Err(CoreError::Type(
                "'__all__' key of `include` and `exclude` must be of type `dict[str | int, <recursive> | ...] | set[str | int | ...]`".into(),
            ));
        }
    }
    Ok(item_dict)
}
