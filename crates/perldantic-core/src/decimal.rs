//! Python's `decimal.Decimal`, as far as pydantic uses it: construction from text, integers,
//! floats (through their `str`) and `(sign, digits, exponent)` tuples, `str` / `repr`, exact
//! comparisons, `as_tuple` digit counts, the `(value / multiple_of) % 1` check at the default
//! context precision, and conversions to `int` and `float`.
//!
//! A value keeps its coefficient and exponent exactly as Python does, so `Decimal('1.50')` stays
//! `1.50`; `==` on `Value`s compares that representation, while [`Decimal::py_eq`] follows
//! Python's numeric `==`.

use std::cmp::Ordering;
use std::fmt;

use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer;
use num_traits::{One, Pow, Zero};

/// Precision of Python's default decimal context.
const CONTEXT_PRECISION: usize = 28;

/// The largest exponent `to_integer` expands.
const MAX_INTEGER_EXPONENT: i64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Finite {
        coefficient: BigUint,
        exponent: i64,
    },
    Infinity,
    /// A quiet or signaling NaN; `payload` holds its diagnostic digits (`NaN12`).
    NaN {
        signaling: bool,
        payload: BigUint,
    },
}

/// A decimal number with Python's `decimal.Decimal` semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    negative: bool,
    kind: Kind,
}

/// Why a `(sign, digits, exponent)` tuple is not a decimal; Python raises `ValueError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TupleError(pub &'static str);

/// The parts of a decimal tuple, as Python's `Decimal(tuple)` reads them.
pub enum TupleExponent {
    Int(i64),
    /// `'F'`
    Infinity,
    /// `'n'`
    NaN,
    /// `'N'`
    SignalingNaN,
}

impl Decimal {
    fn finite(negative: bool, coefficient: BigUint, exponent: i64) -> Self {
        Self {
            negative,
            kind: Kind::Finite {
                coefficient,
                exponent,
            },
        }
    }

    pub fn infinity(negative: bool) -> Self {
        Self {
            negative,
            kind: Kind::Infinity,
        }
    }

    /// `Decimal(text)`: surrounding whitespace is ignored and underscores removed; the value is
    /// kept exactly (`1.50` has exponent -2). `None` for invalid text.
    pub fn parse(text: &str) -> Option<Self> {
        let cleaned: String = text.trim().chars().filter(|c| *c != '_').collect();
        let (negative, rest) = match cleaned.as_bytes().first() {
            Some(b'-') => (true, &cleaned[1..]),
            Some(b'+') => (false, &cleaned[1..]),
            _ => (false, cleaned.as_str()),
        };
        let lower = rest.to_ascii_lowercase();
        if lower == "inf" || lower == "infinity" {
            return Some(Self::infinity(negative));
        }
        for (prefix, signaling) in [("snan", true), ("nan", false)] {
            if let Some(digits) = lower.strip_prefix(prefix) {
                if !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let payload = if digits.is_empty() {
                    BigUint::zero()
                } else {
                    digits.parse().ok()?
                };
                return Some(Self {
                    negative,
                    kind: Kind::NaN { signaling, payload },
                });
            }
        }
        let (mantissa, exponent) = match lower.find('e') {
            Some(i) => (&lower[..i], Some(&lower[i + 1..])),
            None => (lower.as_str(), None),
        };
        let (int_part, frac_part) = match mantissa.find('.') {
            Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
            None => (mantissa, ""),
        };
        let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if (int_part.is_empty() && frac_part.is_empty())
            || !all_digits(int_part)
            || !all_digits(frac_part)
        {
            return None;
        }
        let exponent: i64 = match exponent {
            None => 0,
            Some(e) => {
                let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
                if digits.is_empty() || !all_digits(digits) {
                    return None;
                }
                e.parse().ok()?
            }
        };
        let frac_len = i64::try_from(frac_part.len()).ok()?;
        let coefficient: BigUint = format!("{int_part}{frac_part}").parse().ok()?;
        Some(Self::finite(
            negative,
            coefficient,
            exponent.checked_sub(frac_len)?,
        ))
    }

    /// `Decimal(int)`.
    pub fn from_bigint(value: &BigInt) -> Self {
        Self::finite(value.sign() == Sign::Minus, value.magnitude().clone(), 0)
    }

    /// `Decimal((sign, digits, exponent))`.
    pub fn from_tuple(
        sign: i64,
        digits: &[i64],
        exponent: TupleExponent,
    ) -> Result<Self, TupleError> {
        let negative = match sign {
            0 => false,
            1 => true,
            _ => return Err(TupleError("sign must be an integer with the value 0 or 1")),
        };
        let digits_value = || -> Result<BigUint, TupleError> {
            let mut value = BigUint::zero();
            for &digit in digits {
                let digit = u8::try_from(digit)
                    .ok()
                    .filter(|d| *d <= 9)
                    .ok_or(TupleError("coefficient must be a tuple of digits"))?;
                value = value * 10u8 + digit;
            }
            Ok(value)
        };
        Ok(match exponent {
            TupleExponent::Int(exponent) => Self::finite(negative, digits_value()?, exponent),
            TupleExponent::Infinity => Self::infinity(negative),
            TupleExponent::NaN | TupleExponent::SignalingNaN => Self {
                negative,
                kind: Kind::NaN {
                    signaling: matches!(exponent, TupleExponent::SignalingNaN),
                    payload: digits_value()?,
                },
            },
        })
    }

    /// The value of a finite float, exactly (`Decimal(float)`); Python validates floats through
    /// their `str` instead, see [`Decimal::parse`].
    pub fn from_f64_exact(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let negative = value.is_sign_negative();
        let bits = value.abs().to_bits();
        let raw_exponent = i64::try_from((bits >> 52) & 0x7ff).expect("11 bits");
        let fraction = bits & ((1 << 52) - 1);
        let (mantissa, exponent) = if raw_exponent == 0 {
            (fraction, -1074)
        } else {
            (fraction | (1 << 52), raw_exponent - 1075)
        };
        let mantissa = BigUint::from(mantissa);
        Some(if exponent >= 0 {
            Self::finite(negative, mantissa << usize::try_from(exponent).ok()?, 0)
        } else {
            // m * 2^e == m * 5^-e / 10^-e
            let five = BigUint::from(5u8).pow(u32::try_from(-exponent).ok()?);
            Self::finite(negative, mantissa * five, exponent)
        })
    }

    pub fn is_finite(&self) -> bool {
        matches!(self.kind, Kind::Finite { .. })
    }

    pub fn is_nan(&self) -> bool {
        matches!(self.kind, Kind::NaN { .. })
    }

    pub fn is_signaling_nan(&self) -> bool {
        matches!(
            self.kind,
            Kind::NaN {
                signaling: true,
                ..
            }
        )
    }

    pub fn is_negative(&self) -> bool {
        self.negative
    }

    /// The number of coefficient digits, the number of its trailing zeros and the exponent of a
    /// finite value (`as_tuple()`); `None` otherwise.
    pub fn digits_info(&self) -> Option<(u64, u64, i64)> {
        let Kind::Finite {
            coefficient,
            exponent,
        } = &self.kind
        else {
            return None;
        };
        let digits = coefficient.to_string();
        let trailing_zeros = digits.bytes().rev().take_while(|b| *b == b'0').count();
        Some((
            u64::try_from(digits.len()).ok()?,
            u64::try_from(trailing_zeros).ok()?,
            *exponent,
        ))
    }

    /// Python's comparison; `None` when either value is a NaN (Python raises).
    pub fn py_cmp(&self, other: &Self) -> Option<Ordering> {
        let sign = |d: &Self| -> i8 {
            match &d.kind {
                Kind::Finite { coefficient, .. } if coefficient.is_zero() => 0,
                _ if d.negative => -1,
                _ => 1,
            }
        };
        match (&self.kind, &other.kind) {
            (Kind::NaN { .. }, _) | (_, Kind::NaN { .. }) => None,
            (Kind::Infinity, Kind::Infinity) => Some(sign(self).cmp(&sign(other))),
            (Kind::Infinity, _) => Some(if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            }),
            (_, Kind::Infinity) => Some(if other.negative {
                Ordering::Greater
            } else {
                Ordering::Less
            }),
            (
                Kind::Finite {
                    coefficient: a,
                    exponent: ea,
                },
                Kind::Finite {
                    coefficient: b,
                    exponent: eb,
                },
            ) => {
                let (sa, sb) = (sign(self), sign(other));
                if sa != sb || sa == 0 {
                    return Some(sa.cmp(&sb));
                }
                // same sign: the adjusted exponents (of the leading digits) decide, unless equal;
                // then the exponents differ by less than the digit counts
                let magnitude = match adjusted(a, *ea).cmp(&adjusted(b, *eb)) {
                    Ordering::Equal => {
                        let common = (*ea).min(*eb);
                        let scale = |c: &BigUint, e: i64| c * pow10(e - common);
                        scale(a, *ea).cmp(&scale(b, *eb))
                    }
                    unequal => unequal,
                };
                Some(if sa < 0 {
                    magnitude.reverse()
                } else {
                    magnitude
                })
            }
        }
    }

    /// Python's numeric `==` between decimals.
    pub fn py_eq(&self, other: &Self) -> bool {
        self.py_cmp(other) == Some(Ordering::Equal)
    }

    /// Whether `(self / divisor) % 1 == 0` in Python's default context: the quotient is rounded
    /// to 28 significant digits first. `None` when Python raises (a quotient too large for
    /// `% 1`, a division by zero, an infinity) and `Some(false)` for a NaN, which is never 0.
    pub fn is_multiple_of(&self, divisor: &Self) -> Option<bool> {
        let (
            Kind::Finite {
                coefficient: a,
                exponent: ea,
            },
            Kind::Finite {
                coefficient: b,
                exponent: eb,
            },
        ) = (&self.kind, &divisor.kind)
        else {
            return if self.is_nan() || divisor.is_nan() {
                Some(false)
            } else {
                None
            };
        };
        if b.is_zero() {
            return None;
        }
        if a.is_zero() {
            return Some(true);
        }
        let (coefficient, exponent) =
            divide_rounded(a, b, ea.saturating_sub(*eb), CONTEXT_PRECISION);
        let digits = digit_count(&coefficient);
        if exponent >= 0 {
            // an integer quotient: `% 1` works only while it fits in the context precision
            let precision = i64::try_from(CONTEXT_PRECISION).ok()?;
            return (digits.saturating_add(exponent) <= precision).then_some(true);
        }
        if -exponent > digits {
            // all the (non-zero) digits are fractional
            return Some(false);
        }
        Some((&coefficient % pow10(-exponent)).is_zero())
    }

    /// `int(value)` when the value is a finite integer (`as_integer_ratio()` with denominator
    /// 1); `None` otherwise.
    pub fn to_integer(&self) -> Option<BigInt> {
        let Kind::Finite {
            coefficient,
            exponent,
        } = &self.kind
        else {
            return None;
        };
        let magnitude = if *exponent >= 0 {
            // a guard against allocating absurdly large integers (Python would try)
            if *exponent > MAX_INTEGER_EXPONENT {
                return None;
            }
            coefficient * pow10(*exponent)
        } else if coefficient.is_zero() {
            BigUint::zero()
        } else if -exponent > digit_count(coefficient) {
            return None;
        } else {
            let (quotient, remainder) = coefficient.div_rem(&pow10(-exponent));
            if !remainder.is_zero() {
                return None;
            }
            quotient
        };
        let sign = if self.negative {
            Sign::Minus
        } else {
            Sign::Plus
        };
        Some(BigInt::from_biguint(sign, magnitude))
    }

    /// `float(value)`: correctly rounded; NaNs are NaN (Python refuses signaling NaNs).
    pub fn to_f64(&self) -> f64 {
        match &self.kind {
            Kind::Infinity if self.negative => f64::NEG_INFINITY,
            Kind::Infinity => f64::INFINITY,
            Kind::NaN { .. } => f64::NAN,
            Kind::Finite { .. } => self.to_string().parse().unwrap_or(f64::NAN),
        }
    }

    /// Python's `repr`: `Decimal('1.50')`.
    pub fn repr(&self) -> String {
        format!("Decimal('{self}')")
    }
}

impl fmt::Display for Decimal {
    /// Python's `str`: the scientific string of the General Decimal Arithmetic specification.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        match &self.kind {
            Kind::Infinity => f.write_str("Infinity"),
            Kind::NaN { signaling, payload } => {
                f.write_str(if *signaling { "sNaN" } else { "NaN" })?;
                if payload.is_zero() {
                    Ok(())
                } else {
                    write!(f, "{payload}")
                }
            }
            Kind::Finite {
                coefficient,
                exponent,
            } => {
                let digits = coefficient.to_string();
                let len = i64::try_from(digits.len()).map_err(|_| fmt::Error)?;
                let left_digits = exponent + len;
                let dot_place = if *exponent <= 0 && left_digits > -6 {
                    left_digits
                } else {
                    1
                };
                if dot_place <= 0 {
                    let zeros = usize::try_from(-dot_place).map_err(|_| fmt::Error)?;
                    write!(f, "0.{}{digits}", "0".repeat(zeros))?;
                } else if dot_place >= len {
                    let zeros = usize::try_from(dot_place - len).map_err(|_| fmt::Error)?;
                    write!(f, "{digits}{}", "0".repeat(zeros))?;
                } else {
                    let split = usize::try_from(dot_place).map_err(|_| fmt::Error)?;
                    write!(f, "{}.{}", &digits[..split], &digits[split..])?;
                }
                if left_digits != dot_place {
                    write!(f, "E{:+}", left_digits - dot_place)?;
                }
                Ok(())
            }
        }
    }
}

fn digit_count(coefficient: &BigUint) -> i64 {
    i64::try_from(coefficient.to_string().len()).expect("digit count fits in i64")
}

/// The exponent of the leading digit of a non-zero coefficient.
fn adjusted(coefficient: &BigUint, exponent: i64) -> i64 {
    exponent.saturating_add(digit_count(coefficient) - 1)
}

fn pow10(exponent: i64) -> BigUint {
    BigUint::from(10u8).pow(u32::try_from(exponent).expect("exponent difference fits in u32"))
}

/// `a / b * 10^exponent` rounded half-even to `precision` significant digits, as a coefficient
/// and exponent.
fn divide_rounded(a: &BigUint, b: &BigUint, exponent: i64, precision: usize) -> (BigUint, i64) {
    let digits = |n: &BigUint| n.to_string().len();
    // scale the dividend so that the integer quotient has more than `precision` digits
    let shift = (precision + 1 + digits(b)).saturating_sub(digits(a));
    let shift_i64 = i64::try_from(shift).expect("small shift");
    let scaled = a * pow10(shift_i64);
    let (quotient, remainder) = scaled.div_rem(b);
    let excess = digits(&quotient).saturating_sub(precision);
    let excess_i64 = i64::try_from(excess).expect("small excess");
    let unit = pow10(excess_i64);
    let (mut rounded, dropped) = quotient.div_rem(&unit);
    let twice = &dropped * 2u8;
    let round_up = match twice.cmp(&unit) {
        Ordering::Greater => true,
        Ordering::Equal => !remainder.is_zero() || rounded.is_odd(),
        Ordering::Less => false,
    };
    if round_up {
        rounded += BigUint::one();
    }
    (rounded, exponent - shift_i64 + excess_i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap_or_else(|| panic!("invalid decimal {text}"))
    }

    #[test]
    fn text_round_trips_like_python() {
        for (input, output) in [
            ("1_000", "1000"),
            ("1__0", "10"),
            ("_1", "1"),
            ("1_0.0_1", "10.01"),
            ("  1.5  ", "1.5"),
            ("1e+5", "1E+5"),
            ("1E-7", "1E-7"),
            (".5", "0.5"),
            ("5.", "5"),
            ("+.5e1", "5"),
            ("inf", "Infinity"),
            ("-Infinity", "-Infinity"),
            ("nan", "NaN"),
            ("NaN0012", "NaN12"),
            ("sNaN", "sNaN"),
            ("snan9", "sNaN9"),
            ("0e5", "0E+5"),
            ("-0", "-0"),
            ("1.50", "1.50"),
            ("0.000001", "0.000001"),
            ("0.0000001", "1E-7"),
            ("123e25", "1.23E+27"),
            ("1E+2", "1E+2"),
        ] {
            assert_eq!(d(input).to_string(), output, "{input}");
        }
        for invalid in [
            "", ".", "e5", "1e", "1e+", "abc", "1.2.3", "nan1x", "- 1", "0x10",
        ] {
            assert!(Decimal::parse(invalid).is_none(), "{invalid}");
        }
        assert_eq!(d("1.5").repr(), "Decimal('1.5')");
        assert_eq!(d("-inf").repr(), "Decimal('-Infinity')");
    }

    #[test]
    fn tuples_build_decimals() {
        let t = |sign, digits: &[i64], exponent| Decimal::from_tuple(sign, digits, exponent);
        assert_eq!(
            t(0, &[1, 2], TupleExponent::Int(-1)).unwrap().to_string(),
            "1.2"
        );
        assert_eq!(
            t(1, &[0, 0, 1], TupleExponent::Int(2)).unwrap().to_string(),
            "-1E+2"
        );
        assert_eq!(
            t(0, &[], TupleExponent::Infinity).unwrap().to_string(),
            "Infinity"
        );
        assert_eq!(t(0, &[1], TupleExponent::NaN).unwrap().to_string(), "NaN1");
        assert!(t(2, &[1], TupleExponent::Int(0)).is_err());
        assert!(t(0, &[10], TupleExponent::Int(0)).is_err());
    }

    #[test]
    fn comparisons_are_exact() {
        assert_eq!(d("1E+2").py_cmp(&d("101")), Some(Ordering::Less));
        assert_eq!(d("Infinity").py_cmp(&d("1e999")), Some(Ordering::Greater));
        assert_eq!(d("-Infinity").py_cmp(&d("-1e999")), Some(Ordering::Less));
        assert!(d("1.0").py_eq(&d("1")));
        assert!(d("0").py_eq(&d("-0.00")));
        assert!(!d("NaN").py_eq(&d("NaN")));
        assert_eq!(d("-2").py_cmp(&d("-1")), Some(Ordering::Less));
        // huge exponents compare without scaling
        assert_eq!(
            d("1e999999999999").py_cmp(&d("9e-999999999999")),
            Some(Ordering::Greater)
        );
        assert_eq!(
            d("-1e999999999999").py_cmp(&d("-1e999999999998")),
            Some(Ordering::Less)
        );
        assert_eq!(d("1e-999999999999").is_multiple_of(&d("1")), Some(false));
        assert_eq!(d("1e-999999999999").to_integer(), None);
        assert!(Decimal::from_f64_exact(0.5).unwrap().py_eq(&d("0.5")));
        assert!(!Decimal::from_f64_exact(0.1).unwrap().py_eq(&d("0.1")));
        assert_ne!(
            d("1.50"),
            d("1.5"),
            "`==` on values compares the representation"
        );
    }

    #[test]
    fn multiples_follow_the_default_context() {
        assert_eq!(d("0.3").is_multiple_of(&d("0.1")), Some(true));
        assert_eq!(d("-0.015").is_multiple_of(&d("0.01")), Some(false));
        assert_eq!(d("1e27").is_multiple_of(&d("3")), Some(false));
        // rounded to 28 digits the quotient is an integer
        assert_eq!(
            d("10000000000000000000000000000.5").is_multiple_of(&d("1")),
            None
        );
        // half-even rounding to 28 digits drops the .5
        assert_eq!(
            d("1000000000000000000000000000.5").is_multiple_of(&d("1")),
            Some(true)
        );
        assert_eq!(
            d("100000000000000000000000000.5").is_multiple_of(&d("1")),
            Some(false)
        );
        assert_eq!(d("7").is_multiple_of(&d("0")), None, "Python raises");
        assert_eq!(d("1e30").is_multiple_of(&d("1")), None, "Python raises");
        assert_eq!(d("NaN").is_multiple_of(&d("1")), Some(false));
        assert_eq!(d("0").is_multiple_of(&d("7")), Some(true));
    }

    #[test]
    fn conversions() {
        assert_eq!(d("1E+2").to_integer(), Some(BigInt::from(100)));
        assert_eq!(d("-12.000").to_integer(), Some(BigInt::from(-12)));
        assert_eq!(d("1.5").to_integer(), None);
        assert_eq!(d("0.1").to_f64(), 0.1);
        assert!(d("-inf").to_f64().is_infinite());
        assert_eq!(d("1.50").digits_info(), Some((3, 1, -2)));
        assert_eq!(d("0E+5").digits_info(), Some((1, 1, 5)));
    }
}
