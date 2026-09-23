//! `str` schema. Port of upstream `validators/string.rs`.

use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::build_tools::{
    SchemaDict, is_strict, schema_err, schema_or_config, schema_or_config_same,
};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

#[derive(Debug)]
pub struct StrValidator {
    strict: bool,
    coerce_numbers_to_str: bool,
}

static STRICT_STR_VALIDATOR: LazyLock<Arc<CombinedValidator>> = LazyLock::new(|| {
    Arc::new(CombinedValidator::Str(StrValidator {
        strict: true,
        coerce_numbers_to_str: false,
    }))
});

static LAX_STR_VALIDATOR: LazyLock<Arc<CombinedValidator>> = LazyLock::new(|| {
    Arc::new(CombinedValidator::Str(StrValidator {
        strict: false,
        coerce_numbers_to_str: false,
    }))
});

impl BuildValidator for StrValidator {
    const EXPECTED_TYPE: &'static str = "str";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let con_str_validator = StrConstrainedValidator::build(schema, config)?;

        if con_str_validator.has_constraints_set() {
            Ok(Arc::new(CombinedValidator::StrConstrained(
                con_str_validator,
            )))
        } else if !con_str_validator.coerce_numbers_to_str {
            if is_strict(schema, config)? {
                Ok(STRICT_STR_VALIDATOR.clone())
            } else {
                Ok(LAX_STR_VALIDATOR.clone())
            }
        } else {
            Ok(Arc::new(CombinedValidator::Str(StrValidator {
                strict: con_str_validator.strict,
                coerce_numbers_to_str: con_str_validator.coerce_numbers_to_str,
            })))
        }
    }
}

impl Validator for StrValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        input
            .validate_str(state.strict_or(self.strict), self.coerce_numbers_to_str)
            .map(|val_match| val_match.unpack(state).into_value())
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

/// Any new property set here must be reflected in `has_constraints_set`.
#[derive(Debug, Clone, Default)]
pub struct StrConstrainedValidator {
    strict: bool,
    pattern: Option<Pattern>,
    max_length: Option<usize>,
    min_length: Option<usize>,
    strip_whitespace: bool,
    to_lower: bool,
    to_upper: bool,
    coerce_numbers_to_str: bool,
    ascii_only: bool,
}

impl Validator for StrConstrainedValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let either_str = input
            .validate_str(state.strict_or(self.strict), self.coerce_numbers_to_str)?
            .unpack(state);
        let cow = either_str.as_cow();
        let mut str = cow.as_ref();
        if self.strip_whitespace {
            str = str.trim();
        }
        if self.ascii_only && !str.is_ascii() {
            return Err(ValError::new(
                ErrorType::StringNotAscii { context: None },
                input,
            ));
        }

        let str_len: Option<usize> = if self.min_length.is_some() | self.max_length.is_some() {
            Some(str.chars().count())
        } else {
            None
        };
        if let Some(min_length) = self.min_length
            && str_len.unwrap() < min_length
        {
            return Err(ValError::new(
                ErrorType::StringTooShort {
                    min_length,
                    context: None,
                },
                input,
            ));
        }
        if let Some(max_length) = self.max_length
            && str_len.unwrap() > max_length
        {
            return Err(ValError::new(
                ErrorType::StringTooLong {
                    max_length,
                    context: None,
                },
                input,
            ));
        }

        if let Some(pattern) = &self.pattern
            && !pattern.is_match(str)
        {
            return Err(ValError::new(
                ErrorType::StringPatternMismatch {
                    pattern: pattern.pattern.clone(),
                    context: None,
                },
                input,
            ));
        }

        let output = if self.to_lower {
            str.to_lowercase()
        } else if self.to_upper {
            str.to_uppercase()
        } else {
            str.to_owned()
        };
        Ok(Value::Str(output))
    }

    fn get_name(&self) -> &'static str {
        "constrained-str"
    }
}

impl StrConstrainedValidator {
    fn build(schema: &Dict, config: Option<&Dict>) -> CoreResult<Self> {
        let pattern = schema
            .get_as::<Value>("pattern")?
            .map(|pattern| {
                let regex_engine =
                    schema_or_config::<String>(schema, config, "regex_engine", "regex_engine")?;
                Pattern::compile(
                    &pattern,
                    regex_engine.as_deref().unwrap_or(Pattern::RUST_REGEX),
                )
            })
            .transpose()?;
        let min_length: Option<usize> =
            schema_or_config(schema, config, "min_length", "str_min_length")?;
        let max_length: Option<usize> =
            schema_or_config(schema, config, "max_length", "str_max_length")?;
        let strip_whitespace: bool =
            schema_or_config(schema, config, "strip_whitespace", "str_strip_whitespace")?
                .unwrap_or(false);
        let to_lower: bool =
            schema_or_config(schema, config, "to_lower", "str_to_lower")?.unwrap_or(false);
        let to_upper: bool =
            schema_or_config(schema, config, "to_upper", "str_to_upper")?.unwrap_or(false);
        let coerce_numbers_to_str: bool =
            schema_or_config_same(schema, config, "coerce_numbers_to_str")?.unwrap_or(false);
        let ascii_only: bool =
            schema_or_config_same(schema, config, "ascii_only")?.unwrap_or(false);

        Ok(Self {
            strict: is_strict(schema, config)?,
            pattern,
            max_length,
            min_length,
            strip_whitespace,
            to_lower,
            to_upper,
            coerce_numbers_to_str,
            ascii_only,
        })
    }

    /// Whether any constraint or transformation is enabled (strict and
    /// coerce_numbers_to_str alone are handled by `StrValidator`).
    fn has_constraints_set(&self) -> bool {
        self.pattern.is_some()
            || self.max_length.is_some()
            || self.min_length.is_some()
            || self.strip_whitespace
            || self.to_lower
            || self.to_upper
            || self.ascii_only
    }
}

#[derive(Debug, Clone)]
struct Pattern {
    pattern: String,
    regex: Regex,
}

impl Pattern {
    const RUST_REGEX: &'static str = "rust-regex";

    /// Compile a pattern. Only the `rust-regex` engine exists without Python: `python-re` is
    /// reported as an invalid engine (docs/DIVERGENCES.md).
    fn compile(pattern: &Value, engine: &str) -> CoreResult<Self> {
        let Value::Str(pattern) = pattern else {
            return schema_err!(
                "Invalid pattern, must be str or re.Pattern: {}",
                pattern.py_str()
            );
        };
        if engine != Self::RUST_REGEX {
            return schema_err!("Invalid regex engine: {engine}");
        }
        match Regex::new(pattern) {
            Ok(regex) => Ok(Self {
                pattern: pattern.clone(),
                regex,
            }),
            Err(e) => schema_err!("{e}"),
        }
    }

    fn is_match(&self, target: &str) -> bool {
        self.regex.is_match(target)
    }
}
