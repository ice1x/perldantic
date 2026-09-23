//! `literal` schema. Port of upstream `validators/literal.rs`.
//!
//! Upstream keys its fallback lookups by Python hash and equality; here they are small tables
//! searched with [`Value::py_eq`], keeping the same results: `1`, `1.0` and `True` match each
//! other, and when several expected values are equal the last one wins, as in a Python dict.

use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ValError, ValResult};
use crate::input::{Input, ValidationMatch};
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug, Clone, Default)]
struct BoolLiteral {
    pub true_id: Option<usize>,
    pub false_id: Option<usize>,
}

/// Values compared by Python equality, in the order they were added.
#[derive(Debug, Clone, Default)]
struct EqTable(Vec<(Value, usize)>);

impl EqTable {
    /// Insert like a Python dict: an equal key keeps its place and takes the new id.
    fn set_item(&mut self, key: &Value, id: usize) {
        match self.0.iter_mut().find(|(k, _)| k.py_eq(key)) {
            Some(entry) => entry.1 = id,
            None => self.0.push((key.clone(), id)),
        }
    }

    /// Append without merging, like upstream's list of unhashable values.
    fn push(&mut self, key: &Value, id: usize) {
        self.0.push((key.clone(), id));
    }

    fn get_item(&self, key: &Value) -> Option<usize> {
        self.0.iter().find(|(k, _)| k.py_eq(key)).map(|(_, id)| *id)
    }

    fn into_option(self) -> Option<Self> {
        (!self.0.is_empty()).then_some(self)
    }
}

/// Whether Python could hash the value (lists, dicts and sets, also inside tuples, cannot be).
fn is_hashable(value: &Value) -> bool {
    match value {
        Value::List(_) | Value::Dict(_) | Value::Set(_) => false,
        Value::Tuple(items) => items.iter().all(is_hashable),
        _ => true,
    }
}

#[derive(Debug, Clone)]
pub struct LiteralLookup<T: Debug> {
    // Specialized lookups for ints, bools and strings because they
    // (1) are easy to convert between Rust and Python
    // (2) hashing them in Rust is very fast
    // (3) are the most commonly used things in Literal[...]
    expected_bool: Option<BoolLiteral>,
    expected_int: Option<HashMap<i64, usize>>,
    expected_str: Option<HashMap<String, usize>>,
    // Catch all for hashable types like bytes and floats
    expected_py_dict: Option<EqTable>,
    // Catch all for unhashable types like list
    expected_py_values: Option<EqTable>,
    // Fallback for ints, bools, and strings to use Python equality checks
    // which we can't mix with `expected_py_dict`, as there may be conflicts
    expected_py_primitives: Option<EqTable>,

    pub values: Vec<T>,
}

impl<T: Debug> LiteralLookup<T> {
    pub fn new<'a>(expected: impl Iterator<Item = (&'a Value, T)>) -> CoreResult<Self> {
        let mut expected_bool = BoolLiteral::default();
        let mut expected_int = HashMap::new();
        let mut expected_str: HashMap<String, usize> = HashMap::new();
        let mut expected_py_dict = EqTable::default();
        let mut expected_py_values = EqTable::default();
        let mut expected_py_primitives = EqTable::default();
        let mut values = Vec::new();
        for (k, v) in expected {
            let id = values.len();
            values.push(v);

            if let Ok(bool_value) = k.validate_bool(true) {
                if bool_value.into_inner() {
                    expected_bool.true_id = Some(id);
                } else {
                    expected_bool.false_id = Some(id);
                }
                expected_py_primitives.set_item(k, id);
            }
            match k {
                Value::Int(int_64) => {
                    expected_int.insert(*int_64, id);
                    expected_py_primitives.set_item(k, id);
                }
                // cover the case of an int that's > i64::MAX etc.
                Value::BigInt(_) => expected_py_dict.set_item(k, id),
                Value::Str(s) => {
                    expected_str.insert(s.clone(), id);
                    expected_py_primitives.set_item(k, id);
                }
                _ if is_hashable(k) => expected_py_dict.set_item(k, id),
                _ => expected_py_values.push(k, id),
            }
        }

        Ok(Self {
            expected_bool: (expected_bool.true_id.is_some() || expected_bool.false_id.is_some())
                .then_some(expected_bool),
            expected_int: (!expected_int.is_empty()).then_some(expected_int),
            expected_str: (!expected_str.is_empty()).then_some(expected_str),
            expected_py_dict: expected_py_dict.into_option(),
            expected_py_values: expected_py_values.into_option(),
            expected_py_primitives: expected_py_primitives.into_option(),
            values,
        })
    }

    pub fn validate<'a, I: Input + ?Sized>(&self, input: &'a I) -> ValResult<Option<(&'a I, &T)>> {
        if let Some(expected_bool) = &self.expected_bool
            && let Ok(bool_value) = input.validate_bool(true)
        {
            if bool_value.into_inner() {
                if let Some(true_value) = &expected_bool.true_id {
                    return Ok(Some((input, &self.values[*true_value])));
                }
            } else if let Some(false_value) = &expected_bool.false_id {
                return Ok(Some((input, &self.values[*false_value])));
            }
        }
        if let Some(expected_ints) = &self.expected_int
            && let Ok(either_int) = input.exact_int()
        {
            let int = either_int.into_i64()?;
            if let Some(id) = expected_ints.get(&int) {
                return Ok(Some((input, &self.values[*id])));
            }
        }
        if let Some(expected_strings) = &self.expected_str {
            let validation_result = if input.as_value().is_some() {
                input.exact_str()
            } else {
                // Strings coming from JSON are treated as "strict" but not "exact" for reasons
                // of parsing types like UUID; see the implementation of `validate_str` for Json
                // inputs for justification. We might change that eventually, but for now we need
                // to work around this when loading from JSON
                // V3 TODO: revisit making this "exact" for JSON inputs
                input
                    .validate_str(true, false)
                    .map(ValidationMatch::into_inner)
            };

            if let Ok(either_str) = validation_result
                && let Some(id) = expected_strings.get(either_str.as_cow().as_ref())
            {
                return Ok(Some((input, &self.values[*id])));
            }
        }
        // Convert the input once, only if a fallback lookup needs it.
        let mut value_input = None;
        let mut get_value_input = || value_input.get_or_insert_with(|| input.to_value()).clone();

        if let Some(expected_py_dict) = &self.expected_py_dict
            && let Some(id) = expected_py_dict.get_item(&get_value_input())
        {
            return Ok(Some((input, &self.values[id])));
        }
        if let Some(expected_py_values) = &self.expected_py_values
            && let Some(id) = expected_py_values.get_item(&get_value_input())
        {
            return Ok(Some((input, &self.values[id])));
        }

        // this one must be last to avoid conflicts with the other lookups, think of this
        // almost as a lax fallback
        if let Some(expected_py_primitives) = &self.expected_py_primitives
            && let Some(id) = expected_py_primitives.get_item(&get_value_input())
        {
            return Ok(Some((input, &self.values[id])));
        }
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct LiteralValidator {
    lookup: Box<LiteralLookup<Value>>,
    expected_repr: String,
    name: String,
}

impl BuildValidator for LiteralValidator {
    const EXPECTED_TYPE: &'static str = "literal";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let expected: Vec<Value> = schema.get_as_req("expected")?;
        if expected.is_empty() {
            return schema_err!("`expected` should have length > 0");
        }
        let repr_args: Vec<String> = expected.iter().map(Value::repr).collect();
        let expected_repr = expected_repr(&repr_args);
        let name = expected_name(&repr_args, Self::EXPECTED_TYPE);
        let lookup = Box::new(LiteralLookup::new(expected.iter().map(|v| (v, v.clone())))?);
        Ok(Arc::new(CombinedValidator::Literal(Self {
            lookup,
            expected_repr,
            name,
        })))
    }
}

impl Validator for LiteralValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        _state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        match self.lookup.validate(input)? {
            Some((_, v)) => Ok(v.clone()),
            None => Err(ValError::new(
                ErrorType::LiteralError {
                    expected: self.expected_repr.clone(),
                    context: None,
                },
                input,
            )),
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

/// Formats strings according to (`base_name[arg1,arg2,...,argN]`)
pub fn expected_name(repr_args: &[String], base_name: &'static str) -> String {
    format!("{base_name}[{}]", repr_args.join(","))
}

/// Formats strings according to (`arg1, arg2, ... or argN`)
pub fn expected_repr(repr_args: &[String]) -> String {
    match repr_args.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}
