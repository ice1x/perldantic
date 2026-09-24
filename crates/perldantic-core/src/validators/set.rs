//! `set` and `frozenset` schemas. Port of upstream `validators/set.rs` and
//! `validators/frozenset.rs`; sets are `Value::Set` / `Value::FrozenSet` in input order.

use std::sync::Arc;

use crate::build_tools::{SchemaDict, is_strict};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::{BorrowInput, ConsumeIterator, Input, ValidatedList, validate_iter_to_set};
use crate::value::{Dict, Value};

use super::list::get_items_schema;
use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

macro_rules! build_set_validator {
    ($Struct:ident, $expected_type:literal, $field_type:literal, $validate:ident, $Variant:ident) => {
        #[derive(Debug)]
        pub struct $Struct {
            strict: bool,
            item_validator: Option<Arc<CombinedValidator>>,
            min_length: Option<usize>,
            max_length: Option<usize>,
            name: String,
            fail_fast: bool,
        }

        impl BuildValidator for $Struct {
            const EXPECTED_TYPE: &'static str = $expected_type;

            fn build(
                schema: &Dict,
                config: Option<&Dict>,
                definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
            ) -> CoreResult<Arc<CombinedValidator>> {
                let item_validator = get_items_schema(schema, config, definitions)?;
                let inner_name = item_validator.as_ref().map_or("any", |v| v.get_name());
                let name = format!("{}[{inner_name}]", Self::EXPECTED_TYPE);
                Ok(Arc::new(
                    Self {
                        strict: is_strict(schema, config)?,
                        item_validator,
                        min_length: schema.get_as("min_length")?,
                        max_length: schema.get_as("max_length")?,
                        name,
                        fail_fast: schema.get_as("fail_fast")?.unwrap_or(false),
                    }
                    .into(),
                ))
            }
        }

        impl Validator for $Struct {
            fn validate(
                &self,
                input: &(impl Input + ?Sized),
                state: &mut ValidationState<'_>,
            ) -> ValResult<Value> {
                let input_type = state.extra().input_type;
                let collection = input
                    .$validate(state.strict_or(self.strict), input_type)?
                    .unpack(state);
                let set = collection.iterate(ValidateToSet {
                    input,
                    field_type: $field_type,
                    max_length: self.max_length,
                    item_validator: self.item_validator.as_deref(),
                    state,
                    fail_fast: self.fail_fast,
                })??;
                super::list::min_length_check!(input, $field_type, self.min_length, set);
                Ok(Value::$Variant(set))
            }

            fn get_name(&self) -> &str {
                &self.name
            }
        }
    };
}

build_set_validator!(SetValidator, "set", "Set", validate_set, Set);
build_set_validator!(
    FrozenSetValidator,
    "frozenset",
    "Frozenset",
    validate_frozenset,
    FrozenSet
);

struct ValidateToSet<'a, 's, I: Input + ?Sized> {
    input: &'a I,
    field_type: &'static str,
    max_length: Option<usize>,
    item_validator: Option<&'a CombinedValidator>,
    state: &'a mut ValidationState<'s>,
    fail_fast: bool,
}

impl<T: BorrowInput, I: Input + ?Sized> ConsumeIterator<T> for ValidateToSet<'_, '_, I> {
    type Output = ValResult<Vec<Value>>;
    fn consume_iterator(self, iterator: impl Iterator<Item = T>) -> ValResult<Vec<Value>> {
        validate_iter_to_set(
            iterator,
            self.input,
            self.field_type,
            self.max_length,
            self.item_validator,
            self.state,
            self.fail_fast,
        )
    }
}
