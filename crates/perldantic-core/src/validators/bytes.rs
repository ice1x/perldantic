//! `bytes` schema. Port of upstream `validators/bytes.rs`.

use std::sync::Arc;

use crate::build_tools::{SchemaDict, is_strict};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::config::ValBytesMode;
use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug, Clone)]
pub struct BytesValidator {
    strict: bool,
    bytes_mode: ValBytesMode,
}

impl BuildValidator for BytesValidator {
    const EXPECTED_TYPE: &'static str = "bytes";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let use_constrained =
            schema.get_str("max_length").is_some() || schema.get_str("min_length").is_some();
        if use_constrained {
            BytesConstrainedValidator::build(schema, config)
        } else {
            Ok(Arc::new(CombinedValidator::Bytes(Self {
                strict: is_strict(schema, config)?,
                bytes_mode: ValBytesMode::from_config(config)?,
            })))
        }
    }
}

impl Validator for BytesValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        input
            .validate_bytes(state.strict_or(self.strict), self.bytes_mode)
            .map(|m| m.unpack(state).into_value())
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

#[derive(Debug, Clone)]
pub struct BytesConstrainedValidator {
    strict: bool,
    bytes_mode: ValBytesMode,
    max_length: Option<usize>,
    min_length: Option<usize>,
}

impl Validator for BytesConstrainedValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let either_bytes = input
            .validate_bytes(state.strict_or(self.strict), self.bytes_mode)?
            .unpack(state);
        let len = either_bytes.len();

        if let Some(min_length) = self.min_length
            && len < min_length
        {
            return Err(ValError::new(
                ErrorType::BytesTooShort {
                    min_length,
                    context: None,
                },
                input,
            ));
        }
        if let Some(max_length) = self.max_length
            && len > max_length
        {
            return Err(ValError::new(
                ErrorType::BytesTooLong {
                    max_length,
                    context: None,
                },
                input,
            ));
        }
        Ok(either_bytes.into_value())
    }

    fn get_name(&self) -> &'static str {
        "constrained-bytes"
    }
}

impl BytesConstrainedValidator {
    fn build(schema: &Dict, config: Option<&Dict>) -> CoreResult<Arc<CombinedValidator>> {
        Ok(Arc::new(CombinedValidator::ConstrainedBytes(Self {
            strict: is_strict(schema, config)?,
            bytes_mode: ValBytesMode::from_config(config)?,
            min_length: schema.get_as("min_length")?,
            max_length: schema.get_as("max_length")?,
        })))
    }
}
