//! `tuple` schema. Port of upstream `validators/tuple.rs`.

use std::collections::VecDeque;
use std::sync::Arc;

use jiter::PartialMode;

use crate::build_tools::{SchemaDict, is_strict};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, ValError, ValLineError, ValResult};
use crate::input::{BorrowInput, ConsumeIterator, Input, ValidatedTuple};
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
pub struct TupleValidator {
    strict: bool,
    validators: Vec<Arc<CombinedValidator>>,
    variadic_item_index: Option<usize>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    name: String,
    fail_fast: bool,
}

impl BuildValidator for TupleValidator {
    const EXPECTED_TYPE: &'static str = "tuple";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let items: Vec<Value> = schema.get_as_req("items_schema")?;
        let validators: Vec<Arc<CombinedValidator>> = items
            .iter()
            .map(|item| build_validator(as_dict(item)?, config, definitions))
            .collect::<CoreResult<_>>()?;

        let mut validator_names = validators.iter().map(|v| v.get_name()).collect::<Vec<_>>();
        let variadic_item_index: Option<usize> = schema.get_as("variadic_item_index")?;
        if let Some(variadic_item_index) = variadic_item_index {
            // Upstream panics on an out-of-range index (its FIXME); report it instead
            // (docs/DIVERGENCES.md #11).
            if variadic_item_index >= validators.len() {
                return crate::build_tools::schema_err!(
                    "`variadic_item_index` {variadic_item_index} is out of range for {} items",
                    validators.len()
                );
            }
            validator_names.insert(variadic_item_index + 1, "...");
        }
        let name = format!("tuple[{}]", validator_names.join(", "));

        Ok(Arc::new(CombinedValidator::Tuple(Self {
            strict: is_strict(schema, config)?,
            validators,
            variadic_item_index,
            min_length: schema.get_as("min_length")?,
            max_length: schema.get_as("max_length")?,
            name,
            fail_fast: schema.get_as("fail_fast")?.unwrap_or(false),
        })))
    }
}

impl TupleValidator {
    #[allow(clippy::too_many_arguments)]
    fn validate_tuple_items<I: BorrowInput>(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
        output: &mut Vec<Value>,
        errors: &mut Vec<ValLineError>,
        item_validators: &[Arc<CombinedValidator>],
        collection_iter: &mut NextCountingIterator<impl Iterator<Item = I>>,
        actual_length: Option<usize>,
        fail_fast: bool,
    ) -> ValResult<()> {
        // Validate the head:
        for validator in item_validators {
            match collection_iter.next() {
                Some((index, input_item)) => {
                    match validator.validate(input_item.borrow_input(), state) {
                        Ok(item) => self.push_output_item(input, output, item, actual_length)?,
                        Err(ValError::LineErrors(line_errors)) => {
                            errors.extend(
                                line_errors
                                    .into_iter()
                                    .map(|err| err.with_outer_location(index)),
                            );
                        }
                        Err(ValError::Omit) => (),
                        Err(err) => return Err(err),
                    }
                }
                None => {
                    let index = collection_iter.next_calls() - 1;
                    if let Some(value) = validator.default_value(Some(index), state)? {
                        output.push(value);
                    } else {
                        errors.push(ValLineError::new_with_loc(
                            ErrorTypeDefaults::Missing,
                            input,
                            index,
                        ));
                    }
                }
            }
            if fail_fast && !errors.is_empty() {
                return Ok(());
            }
        }

        Ok(())
    }

    fn validate_tuple_variable<I: BorrowInput>(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
        errors: &mut Vec<ValLineError>,
        collection_iter: &mut NextCountingIterator<impl Iterator<Item = I>>,
        actual_length: Option<usize>,
    ) -> ValResult<Vec<Value>> {
        let expected_length = if self.variadic_item_index.is_some() {
            actual_length.unwrap_or(self.validators.len())
        } else {
            self.validators.len()
        };
        let mut output = Vec::with_capacity(expected_length);
        if let Some(variable_validator_index) = self.variadic_item_index {
            let (head_validators, [variable_validator, tail_validators @ ..]) =
                self.validators.split_at(variable_validator_index)
            else {
                unreachable!("validators will always contain variable validator")
            };

            // Validate the "head" items
            self.validate_tuple_items(
                input,
                state,
                &mut output,
                errors,
                head_validators,
                collection_iter,
                actual_length,
                self.fail_fast,
            )?;

            if self.fail_fast && !errors.is_empty() {
                return Ok(output);
            }

            let n_tail_validators = tail_validators.len();
            if n_tail_validators == 0 {
                for (index, input_item) in collection_iter {
                    match variable_validator.validate(input_item.borrow_input(), state) {
                        Ok(item) => {
                            self.push_output_item(input, &mut output, item, actual_length)?;
                        }
                        Err(ValError::LineErrors(line_errors)) => {
                            errors.extend(
                                line_errors
                                    .into_iter()
                                    .map(|err| err.with_outer_location(index)),
                            );
                        }
                        Err(ValError::Omit) => (),
                        Err(err) => return Err(err),
                    }

                    if self.fail_fast && !errors.is_empty() {
                        return Ok(output);
                    }
                }
            } else {
                // Populate a buffer with the first n_tail_validators items
                // NB: We take from collection_iter.inner to avoid increasing the next calls count
                // while populating the buffer. This means the index in the following loop is the
                // right one for user errors.
                let mut tail_buffer: VecDeque<I> = collection_iter
                    .inner
                    .by_ref()
                    .take(n_tail_validators)
                    .collect();

                // Save the current index for the tail validation below when we recreate a new
                // NextCountingIterator
                let mut index = collection_iter.next_calls();

                // Iterate over all remaining collection items, validating as items "leave" the
                // buffer
                for (buffer_item_index, input_item) in collection_iter {
                    index = buffer_item_index;
                    // This `unwrap` is safe because you can only get here
                    // if there were at least `n_tail_validators` (> 0) items in the iterator
                    let buffered_item = tail_buffer.pop_front().unwrap();
                    tail_buffer.push_back(input_item);

                    match variable_validator.validate(buffered_item.borrow_input(), state) {
                        Ok(item) => {
                            self.push_output_item(input, &mut output, item, actual_length)?;
                        }
                        Err(ValError::LineErrors(line_errors)) => {
                            errors.extend(
                                line_errors
                                    .into_iter()
                                    .map(|err| err.with_outer_location(buffer_item_index)),
                            );
                        }
                        Err(ValError::Omit) => (),
                        Err(err) => return Err(err),
                    }

                    if self.fail_fast && !errors.is_empty() {
                        return Ok(output);
                    }
                }

                // Validate the buffered items using the tail validators
                self.validate_tuple_items(
                    input,
                    state,
                    &mut output,
                    errors,
                    tail_validators,
                    &mut NextCountingIterator::new(tail_buffer.into_iter(), index),
                    actual_length,
                    self.fail_fast,
                )?;
            }
        } else {
            // Validate all items as positional
            self.validate_tuple_items(
                input,
                state,
                &mut output,
                errors,
                &self.validators,
                collection_iter,
                actual_length,
                self.fail_fast,
            )?;

            if self.fail_fast && !errors.is_empty() {
                return Ok(output);
            }

            // Generate an error if there are any extra items:
            if collection_iter.next().is_some() {
                return Err(ValError::new(
                    ErrorType::TooLong {
                        field_type: "Tuple".to_string(),
                        max_length: self.validators.len(),
                        actual_length,
                        context: None,
                    },
                    input,
                ));
            }
        }
        Ok(output)
    }

    fn push_output_item(
        &self,
        input: &(impl Input + ?Sized),
        output: &mut Vec<Value>,
        item: Value,
        actual_length: Option<usize>,
    ) -> ValResult<()> {
        output.push(item);
        if let Some(max_length) = self.max_length
            && output.len() > max_length
        {
            return Err(ValError::new(
                ErrorType::TooLong {
                    field_type: "Tuple".to_string(),
                    max_length,
                    actual_length,
                    context: None,
                },
                input,
            ));
        }
        Ok(())
    }
}

impl Validator for TupleValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        // this validator does not yet support partial validation, disable it to avoid incorrect
        // results
        state.allow_partial = PartialMode::Off;

        let collection = input
            .validate_tuple(state.strict_or(self.strict))?
            .unpack(state);
        let actual_length = collection.len();

        let mut errors: Vec<ValLineError> = Vec::new();

        let output = collection.iterate(ValidateToTuple {
            input,
            actual_length,
            validator: self,
            errors: &mut errors,
            state,
        })??;

        if let Some(min_length) = self.min_length {
            let actual_length = output.len();
            if actual_length < min_length {
                errors.push(ValLineError::new(
                    ErrorType::TooShort {
                        field_type: "Tuple".to_string(),
                        min_length,
                        actual_length,
                        context: None,
                    },
                    input,
                ));
            }
        }

        if errors.is_empty() {
            Ok(Value::Tuple(output))
        } else {
            Err(ValError::LineErrors(errors))
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

struct ValidateToTuple<'a, 's, I: Input + ?Sized> {
    input: &'a I,
    actual_length: Option<usize>,
    validator: &'a TupleValidator,
    errors: &'a mut Vec<ValLineError>,
    state: &'a mut ValidationState<'s>,
}

impl<T: BorrowInput, I: Input + ?Sized> ConsumeIterator<T> for ValidateToTuple<'_, '_, I> {
    type Output = ValResult<Vec<Value>>;
    fn consume_iterator(self, iterator: impl Iterator<Item = T>) -> ValResult<Vec<Value>> {
        self.validator.validate_tuple_variable(
            self.input,
            self.state,
            self.errors,
            &mut NextCountingIterator::new(iterator, 0),
            self.actual_length,
        )
    }
}

struct NextCountingIterator<I: Iterator> {
    inner: I,
    count: usize,
}

impl<I: Iterator> NextCountingIterator<I> {
    fn new(inner: I, count: usize) -> Self {
        Self { inner, count }
    }

    fn next_calls(&self) -> usize {
        self.count
    }
}

impl<I: Iterator> Iterator for NextCountingIterator<I> {
    type Item = (usize, I::Item);

    fn next(&mut self) -> Option<Self::Item> {
        let count = self.count;
        self.count += 1;
        self.inner.next().map(|item| (count, item))
    }
}
