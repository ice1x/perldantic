//! `list` schema. Port of upstream `validators/list.rs`.

use std::sync::{Arc, OnceLock};

use crate::build_tools::{SchemaDict, is_strict};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::{
    BorrowInput, ConsumeIterator, Input, MaxLengthCheck, ValidatedList, no_validator_iter_to_vec,
    validate_iter_to_vec,
};
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
pub struct ListValidator {
    strict: bool,
    item_validator: Option<Arc<CombinedValidator>>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    name: OnceLock<String>,
    fail_fast: bool,
}

/// The `items_schema` validator, or `None` when items are accepted as they are.
pub fn get_items_schema(
    schema: &Dict,
    config: Option<&Dict>,
    definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
) -> CoreResult<Option<Arc<CombinedValidator>>> {
    match schema.get_str("items_schema") {
        Some(d) => {
            let validator = build_validator(as_dict(d)?, config, definitions)?;
            match validator.as_ref() {
                CombinedValidator::Any(_) => Ok(None),
                _ => Ok(Some(validator)),
            }
        }
        None => Ok(None),
    }
}

macro_rules! length_check {
    ($input:ident, $field_type:literal, $min_length:expr, $max_length:expr, $obj:ident) => {{
        let mut op_actual_length: Option<usize> = None;
        if let Some(min_length) = $min_length {
            let actual_length = $obj.len();
            if actual_length < min_length {
                return Err(crate::errors::ValError::new(
                    crate::errors::ErrorType::TooShort {
                        field_type: $field_type.to_string(),
                        min_length,
                        actual_length,
                        context: None,
                    },
                    $input,
                ));
            }
            op_actual_length = Some(actual_length);
        }
        if let Some(max_length) = $max_length {
            let actual_length = op_actual_length.unwrap_or_else(|| $obj.len());
            if actual_length > max_length {
                return Err(crate::errors::ValError::new(
                    crate::errors::ErrorType::TooLong {
                        field_type: $field_type.to_string(),
                        max_length,
                        actual_length: Some(actual_length),
                        context: None,
                    },
                    $input,
                ));
            }
        }
    }};
}
pub(crate) use length_check;

macro_rules! min_length_check {
    ($input:ident, $field_type:literal, $min_length:expr, $obj:ident) => {{
        if let Some(min_length) = $min_length {
            let actual_length = $obj.len();
            if actual_length < min_length {
                return Err(crate::errors::ValError::new(
                    crate::errors::ErrorType::TooShort {
                        field_type: $field_type.to_string(),
                        min_length,
                        actual_length,
                        context: None,
                    },
                    $input,
                ));
            }
        }
    }};
}

impl BuildValidator for ListValidator {
    const EXPECTED_TYPE: &'static str = "list";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let item_validator = get_items_schema(schema, config, definitions)?;
        Ok(Arc::new(CombinedValidator::List(Self {
            strict: is_strict(schema, config)?,
            item_validator,
            min_length: schema.get_as("min_length")?,
            max_length: schema.get_as("max_length")?,
            name: OnceLock::new(),
            fail_fast: schema.get_as("fail_fast")?.unwrap_or(false),
        })))
    }
}

impl Validator for ListValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let seq = input
            .validate_list(state.strict_or(self.strict))?
            .unpack(state);

        let actual_length = seq.len();
        let output = match self.item_validator {
            Some(ref v) => seq.iterate(ValidateToVec {
                input,
                actual_length,
                max_length: self.max_length,
                field_type: "List",
                item_validator: v,
                state,
                fail_fast: self.fail_fast,
            })??,
            None => seq.iterate(ToVec {
                input,
                actual_length,
                max_length: self.max_length,
                field_type: "List",
            })??,
        };
        min_length_check!(input, "List", self.min_length, output);
        Ok(Value::List(output))
    }

    fn get_name(&self) -> &str {
        // The logic here is a little janky, it's done to try to cache the formatted name
        // while also trying to render definitions correctly when possible.
        match self.name.get() {
            Some(s) => s.as_str(),
            None => {
                let name = self.item_validator.as_ref().map_or("any", |v| v.get_name());
                if name == "..." {
                    // when inner name is not initialized yet, don't cache it here
                    "list[...]"
                } else {
                    self.name.get_or_init(|| format!("list[{name}]")).as_str()
                }
            }
        }
    }
}

pub(super) struct ValidateToVec<'a, 's, I: Input + ?Sized> {
    pub(super) input: &'a I,
    pub(super) actual_length: Option<usize>,
    pub(super) max_length: Option<usize>,
    pub(super) field_type: &'static str,
    pub(super) item_validator: &'a CombinedValidator,
    pub(super) state: &'a mut ValidationState<'s>,
    pub(super) fail_fast: bool,
}

// pretty arbitrary default capacity when creating vecs from iteration
const DEFAULT_CAPACITY: usize = 10;

impl<T: BorrowInput, I: Input + ?Sized> ConsumeIterator<T> for ValidateToVec<'_, '_, I> {
    type Output = ValResult<Vec<Value>>;
    fn consume_iterator(self, iterator: impl Iterator<Item = T>) -> ValResult<Vec<Value>> {
        let capacity = self.actual_length.unwrap_or(DEFAULT_CAPACITY);
        let max_length_check = MaxLengthCheck::new(
            self.max_length,
            self.field_type,
            self.input,
            self.actual_length,
        );
        validate_iter_to_vec(
            iterator,
            capacity,
            max_length_check,
            self.item_validator,
            self.state,
            self.fail_fast,
        )
    }
}

pub(super) struct ToVec<'a, I: Input + ?Sized> {
    pub(super) input: &'a I,
    pub(super) actual_length: Option<usize>,
    pub(super) max_length: Option<usize>,
    pub(super) field_type: &'static str,
}

impl<T: BorrowInput, I: Input + ?Sized> ConsumeIterator<T> for ToVec<'_, I> {
    type Output = ValResult<Vec<Value>>;
    fn consume_iterator(self, iterator: impl Iterator<Item = T>) -> ValResult<Vec<Value>> {
        let max_length_check = MaxLengthCheck::new(
            self.max_length,
            self.field_type,
            self.input,
            self.actual_length,
        );
        no_validator_iter_to_vec(iterator, max_length_check)
    }
}
