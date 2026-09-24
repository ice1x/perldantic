//! `decimal` schema. Port of upstream `validators/decimal.rs`; values are
//! `crate::decimal::Decimal`s rather than Python `decimal.Decimal` objects.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::build_tools::{SchemaDict, is_strict, schema_err, schema_or_config_same};
use crate::core_error::{CoreError, CoreResult};
use crate::decimal::Decimal;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, Number, ValError, ValResult};
use crate::input::Input;
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator};

fn validate_as_decimal(schema: &Dict, key: &str) -> CoreResult<Option<Decimal>> {
    match schema.get_str(key) {
        Some(value) => match value.validate_decimal(false) {
            Ok(v) => Ok(Some(v.into_inner())),
            Err(_) => schema_err!("'{key}' must be coercible to a Decimal instance"),
        },
        None => Ok(None),
    }
}

#[derive(Debug, Clone)]
pub struct DecimalValidator {
    strict: bool,
    allow_inf_nan: bool,
    check_digits: bool,
    multiple_of: Option<Decimal>,
    le: Option<Decimal>,
    lt: Option<Decimal>,
    ge: Option<Decimal>,
    gt: Option<Decimal>,
    max_digits: Option<u64>,
    decimal_places: Option<u64>,
}

impl BuildValidator for DecimalValidator {
    const EXPECTED_TYPE: &'static str = "decimal";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let allow_inf_nan =
            schema_or_config_same(schema, config, "allow_inf_nan")?.unwrap_or(false);
        let as_u64 =
            |key| -> CoreResult<Option<u64>> { Ok(schema.get_as::<usize>(key)?.map(|v| v as u64)) };
        let decimal_places = as_u64("decimal_places")?;
        let max_digits = as_u64("max_digits")?;
        if allow_inf_nan && (decimal_places.is_some() || max_digits.is_some()) {
            return Err(CoreError::Value(
                "allow_inf_nan=True cannot be used with max_digits or decimal_places".to_owned(),
            ));
        }
        Ok(Arc::new(
            Self {
                strict: is_strict(schema, config)?,
                allow_inf_nan,
                check_digits: decimal_places.is_some() || max_digits.is_some(),
                decimal_places,
                multiple_of: validate_as_decimal(schema, "multiple_of")?,
                le: validate_as_decimal(schema, "le")?,
                lt: validate_as_decimal(schema, "lt")?,
                ge: validate_as_decimal(schema, "ge")?,
                gt: validate_as_decimal(schema, "gt")?,
                max_digits,
            }
            .into(),
        ))
    }
}

fn count_digits(num_digits: u64, exponent: i64) -> (u64, u64) {
    if exponent >= 0 {
        // A positive exponent adds that many trailing zeros.
        (0, num_digits.saturating_add(exponent.unsigned_abs()))
    } else {
        // If the absolute value of the negative exponent is larger than the number of digits,
        // it consumes all the digits and adds leading zeros after the decimal point.
        let decimals = exponent.unsigned_abs();
        (decimals, num_digits.max(decimals))
    }
}

/// `(decimals, digits)` of the value as written and normalised (trailing zeros stripped).
fn extract_decimal_digits_info(decimal: &Decimal) -> Option<((u64, u64), (u64, u64))> {
    let (num_digits, trailing_zeros, exponent) = decimal.digits_info()?;
    // `Decimal.normalize()` canonicalises any zero to `0E0`, whatever the exponent is
    let is_zero = trailing_zeros == num_digits;
    let counts = count_digits(num_digits, exponent);
    let normalized = if is_zero {
        (0, 1)
    } else if exponent >= 0 {
        // stripping trailing zeros moves them into the exponent: the counts are unchanged
        counts
    } else {
        count_digits(
            num_digits - trailing_zeros,
            exponent.saturating_add(i64::try_from(trailing_zeros).ok()?),
        )
    };
    Some((counts, normalized))
}

impl Validator for DecimalValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let decimal = input
            .validate_decimal(state.strict_or(self.strict))?
            .unpack(state);

        if !self.allow_inf_nan || self.check_digits {
            if !decimal.is_finite() {
                return Err(ValError::new(ErrorTypeDefaults::FiniteNumber, input));
            }
            if self.check_digits
                && let Some(((decimals, digits), (normalized_decimals, normalized_digits))) =
                    extract_decimal_digits_info(&decimal)
            {
                self.check_digit_counts(
                    input,
                    (decimals, digits),
                    (normalized_decimals, normalized_digits),
                )?;
            }
        }

        if let Some(multiple_of) = &self.multiple_of {
            match decimal.is_multiple_of(multiple_of) {
                Some(true) => {}
                Some(false) => {
                    return Err(ValError::new(
                        ErrorType::MultipleOf {
                            multiple_of: Number::String(multiple_of.to_string()),
                            context: Some(decimal_ctx("multiple_of", multiple_of)),
                        },
                        input,
                    ));
                }
                None => {
                    // Python raises `decimal.InvalidOperation` or `DivisionByZero`
                    return Err(ValError::InternalErr(CoreError::Value(format!(
                        "{decimal} / {multiple_of} % 1 is not defined in the default decimal context"
                    ))));
                }
            }
        }

        // Decimal raises when comparing NaN, so a NaN fails every bound.
        macro_rules! check_bound {
            ($bound:ident, $error:ident, $($ok:pat_param)|+) => {
                if let Some($bound) = &self.$bound
                    && !matches!(decimal.py_cmp($bound), Some($($ok)|+))
                {
                    return Err(ValError::new(
                        ErrorType::$error {
                            $bound: Number::String($bound.to_string()),
                            context: Some(decimal_ctx(stringify!($bound), $bound)),
                        },
                        input,
                    ));
                }
            };
        }
        check_bound!(le, LessThanEqual, Ordering::Less | Ordering::Equal);
        check_bound!(lt, LessThan, Ordering::Less);
        check_bound!(ge, GreaterThanEqual, Ordering::Greater | Ordering::Equal);
        check_bound!(gt, GreaterThan, Ordering::Greater);

        Ok(Value::Decimal(Box::new(decimal)))
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

impl DecimalValidator {
    fn check_digit_counts(
        &self,
        input: &(impl Input + ?Sized),
        (decimals, digits): (u64, u64),
        (normalized_decimals, normalized_digits): (u64, u64),
    ) -> ValResult<()> {
        if let Some(max_digits) = self.max_digits
            && digits > max_digits
            && normalized_digits > max_digits
        {
            return Err(ValError::new(
                ErrorType::DecimalMaxDigits {
                    max_digits,
                    context: None,
                },
                input,
            ));
        }
        if let Some(decimal_places) = self.decimal_places {
            if decimals > decimal_places && normalized_decimals > decimal_places {
                return Err(ValError::new(
                    ErrorType::DecimalMaxPlaces {
                        decimal_places,
                        context: None,
                    },
                    input,
                ));
            }
            if let Some(max_digits) = self.max_digits {
                let whole_digits = digits.saturating_sub(decimals);
                let max_whole_digits = max_digits.saturating_sub(decimal_places);
                let normalized_whole_digits = normalized_digits.saturating_sub(normalized_decimals);
                if whole_digits > max_whole_digits && normalized_whole_digits > max_whole_digits {
                    return Err(ValError::new(
                        ErrorType::DecimalWholeDigits {
                            whole_digits: max_whole_digits,
                            context: None,
                        },
                        input,
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The error context upstream builds: the bound as a `Decimal`.
fn decimal_ctx(key: &str, value: &Decimal) -> Dict {
    [(Value::from(key), Value::Decimal(Box::new(value.clone())))]
        .into_iter()
        .collect()
}
