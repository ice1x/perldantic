//! `dict` schema. Port of upstream `validators/dict.rs`.

use std::sync::Arc;

use jiter::PartialMode;

use crate::build_tools::{SchemaDict, is_strict};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{LocItem, ValError, ValLineError, ValResult};
use crate::input::{BorrowInput, ConsumeIterator, Input, ValidatedDict};
use crate::value::{Dict, Value};

use super::any::AnyValidator;
use super::list::length_check;
use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
pub struct DictValidator {
    strict: bool,
    key_validator: Arc<CombinedValidator>,
    value_validator: Arc<CombinedValidator>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    fail_fast: bool,
    name: String,
}

impl BuildValidator for DictValidator {
    const EXPECTED_TYPE: &'static str = "dict";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let key_validator = match schema.get_str("keys_schema") {
            Some(schema) => build_validator(as_dict(schema)?, config, definitions)?,
            None => AnyValidator::build(schema, config, definitions)?,
        };
        let value_validator = match schema.get_str("values_schema") {
            Some(d) => build_validator(as_dict(d)?, config, definitions)?,
            None => AnyValidator::build(schema, config, definitions)?,
        };
        let name = format!(
            "{}[{},{}]",
            Self::EXPECTED_TYPE,
            key_validator.get_name(),
            value_validator.get_name()
        );
        Ok(Arc::new(CombinedValidator::Dict(Self {
            strict: is_strict(schema, config)?,
            key_validator,
            value_validator,
            min_length: schema.get_as("min_length")?,
            max_length: schema.get_as("max_length")?,
            fail_fast: schema.get_as("fail_fast")?.unwrap_or(false),
            name,
        })))
    }
}

impl Validator for DictValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let strict = state.strict_or(self.strict);
        let dict = input.validate_dict(strict)?;
        dict.iterate(ValidateToDict {
            input,
            min_length: self.min_length,
            max_length: self.max_length,
            fail_fast: self.fail_fast,
            key_validator: &self.key_validator,
            value_validator: &self.value_validator,
            state,
        })?
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

struct ValidateToDict<'a, 's, I: Input + ?Sized> {
    input: &'a I,
    min_length: Option<usize>,
    max_length: Option<usize>,
    fail_fast: bool,
    key_validator: &'a CombinedValidator,
    value_validator: &'a CombinedValidator,
    state: &'a mut ValidationState<'s>,
}

impl<Key, Item, I: Input + ?Sized> ConsumeIterator<ValResult<(Key, Item)>>
    for ValidateToDict<'_, '_, I>
where
    Key: BorrowInput + Clone + Into<LocItem>,
    Item: BorrowInput,
{
    type Output = ValResult<Value>;
    fn consume_iterator(
        self,
        iterator: impl Iterator<Item = ValResult<(Key, Item)>>,
    ) -> ValResult<Value> {
        let mut output = Dict::new();
        let mut errors: Vec<ValLineError> = Vec::new();
        let allow_partial = self.state.allow_partial;

        macro_rules! should_fail_fast {
            () => {
                self.fail_fast && !errors.is_empty()
            };
        }

        for (_, is_last_partial, item_result) in self.state.enumerate_last_partial(iterator) {
            self.state.allow_partial = PartialMode::Off;
            let (key, value) = item_result?;
            let output_key = match self.key_validator.validate(key.borrow_input(), self.state) {
                Ok(value) => Some(value),
                Err(ValError::LineErrors(line_errors)) => {
                    for err in line_errors {
                        // these are added in reverse order so [key] is shunted along by the
                        // second call
                        errors.push(
                            err.with_outer_location("[key]")
                                .with_outer_location(key.clone()),
                        );
                    }
                    None
                }
                Err(ValError::Omit) => continue,
                Err(err) => return Err(err),
            };
            self.state.allow_partial = if is_last_partial {
                allow_partial
            } else {
                PartialMode::Off
            };

            if should_fail_fast!() {
                break;
            }

            let output_value = match self
                .value_validator
                .validate(value.borrow_input(), self.state)
            {
                Ok(value) => value,
                Err(ValError::LineErrors(line_errors)) => {
                    if !is_last_partial {
                        errors.extend(
                            line_errors
                                .into_iter()
                                .map(|err| err.with_outer_location(key.clone())),
                        );
                    }
                    continue;
                }
                Err(ValError::Omit) => continue,
                Err(err) => return Err(err),
            };

            if should_fail_fast!() {
                break;
            }

            if let Some(key) = output_key {
                output.insert(key, output_value);
            }
        }

        if errors.is_empty() {
            let input = self.input;
            length_check!(
                input,
                "Dictionary",
                self.min_length,
                self.max_length,
                output
            );
            Ok(Value::Dict(output))
        } else {
            Err(ValError::LineErrors(errors))
        }
    }
}
