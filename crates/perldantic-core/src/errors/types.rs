//! Error types, message templates and contexts. Port of upstream `errors/types.rs`.
//!
//! The `error_types!` table and the message templates are copied verbatim from upstream so
//! codes and messages stay identical to pydantic; only the Python plumbing is replaced.

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use num_bigint::BigInt;
use strum::IntoEnumIterator;

use crate::core_error::{CoreError, CoreResult};
use crate::input::InputType;
use crate::value::{Dict, Value};

/// Extraction of a typed context field from a context value.
trait FromCtx: Sized {
    fn from_ctx(value: &Value) -> Option<Self>;
}

impl FromCtx for String {
    fn from_ctx(value: &Value) -> Option<Self> {
        match value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl FromCtx for usize {
    fn from_ctx(value: &Value) -> Option<Self> {
        match value {
            Value::Int(i) => usize::try_from(*i).ok(),
            _ => None,
        }
    }
}

impl FromCtx for Option<usize> {
    fn from_ctx(value: &Value) -> Option<Self> {
        match value {
            Value::None => Some(None),
            other => usize::from_ctx(other).map(Some),
        }
    }
}

impl FromCtx for i32 {
    fn from_ctx(value: &Value) -> Option<Self> {
        match value {
            Value::Int(i) => i32::try_from(*i).ok(),
            _ => None,
        }
    }
}

impl FromCtx for u64 {
    fn from_ctx(value: &Value) -> Option<Self> {
        match value {
            Value::Int(i) => u64::try_from(*i).ok(),
            _ => None,
        }
    }
}

impl FromCtx for Number {
    fn from_ctx(value: &Value) -> Option<Self> {
        match value {
            Value::Int(i) => Some(Self::Int(*i)),
            Value::BigInt(i) => Some(Self::BigInt(i.clone())),
            Value::Float(f) => Some(Self::Float(*f)),
            Value::Str(s) => Some(Self::String(s.clone())),
            _ => None,
        }
    }
}

/// `ValueError` / `AssertionError` contexts hold the host exception; the core keeps its text.
impl FromCtx for Option<String> {
    fn from_ctx(value: &Value) -> Option<Self> {
        match value {
            Value::None => Some(None),
            other => Some(Some(other.py_str())),
        }
    }
}

fn field_from_context<T: FromCtx>(
    context: Option<&Dict>,
    field_name: &str,
    enum_name: &str,
    type_name: &str,
) -> CoreResult<T> {
    let value = context
        .and_then(|ctx| ctx.get_str(field_name))
        .ok_or_else(|| {
            CoreError::Type(format!("{enum_name}: '{field_name}' required in context"))
        })?;
    T::from_ctx(value).ok_or_else(|| {
        CoreError::Type(format!(
            "{enum_name}: '{field_name}' context value must be a {type_name}"
        ))
    })
}

/// Conversion of a typed context field back into a context value.
trait ToCtx {
    fn to_ctx(&self) -> Value;
}

impl ToCtx for String {
    fn to_ctx(&self) -> Value {
        Value::Str(self.clone())
    }
}

impl ToCtx for usize {
    fn to_ctx(&self) -> Value {
        Value::from(*self)
    }
}

impl ToCtx for Option<usize> {
    fn to_ctx(&self) -> Value {
        self.map_or(Value::None, Value::from)
    }
}

impl ToCtx for i32 {
    fn to_ctx(&self) -> Value {
        Value::Int(i64::from(*self))
    }
}

impl ToCtx for u64 {
    fn to_ctx(&self) -> Value {
        i64::try_from(*self).map_or_else(|_| Value::BigInt(BigInt::from(*self)), Value::Int)
    }
}

impl ToCtx for Number {
    fn to_ctx(&self) -> Value {
        match self {
            Self::Int(i) => Value::Int(*i),
            Self::BigInt(i) => Value::BigInt(i.clone()),
            Self::Float(f) => Value::Float(*f),
            Self::String(s) => Value::Str(s.clone()),
        }
    }
}

impl ToCtx for Option<String> {
    fn to_ctx(&self) -> Value {
        self.clone().map_or(Value::None, Value::Str)
    }
}

macro_rules! basic_error_default {
    (
        $item:ident $(,)?
    ) => {
        pub const $item: ErrorType = ErrorType::$item { context: None };
    };
    (
        $item:ident, $($key:ident),* $(,)?
    ) => {}; // With more parameters enum item must be explicitly created
}

macro_rules! error_types {
    (
        $(
            $item:ident {
                $($key:ident: {ctx_type: $ctx_type:ty, ctx_fn: $ctx_fn:path}),* $(,)?
            },
        )+
    ) => {
        #[derive(Clone, Debug, strum::Display, strum::EnumIter, strum::IntoStaticStr)]
        #[strum(serialize_all = "snake_case")]
        pub enum ErrorType {
            $(
                $item {
                    context: Option<Dict>,
                    $($key: $ctx_type,)*
                }
            ),+,
        }
        impl ErrorType {
            /// Build an error type from its code and a context providing its fields.
            pub fn new(value: &str, context: Option<&Dict>) -> CoreResult<Self> {
                let error_type = error_type_lookup()
                    .get(value)
                    .cloned()
                    .ok_or_else(|| CoreError::Key(format!("Invalid error type: '{value}'")))?;
                match error_type {
                    $(
                        Self::$item { .. } => {
                            Ok(Self::$item {
                                $(
                                    $key: $ctx_fn(context, stringify!($key), stringify!($item), stringify!($ctx_type))?,
                                )*
                                context: context.cloned(),
                            })
                        },
                    )+
                }
            }

            /// Add typed fields, then the custom context; returns whether a custom context exists.
            fn update_ctx(&self, dict: &mut Dict) -> bool {
                match self {
                    $(
                        Self::$item { context, $($key,)* } => {
                            $(
                                dict.insert(Value::from(stringify!($key)), $key.to_ctx());
                            )*
                            if let Some(ctx) = context {
                                for (k, v) in ctx.iter() {
                                    dict.insert(k.clone(), v.clone());
                                }
                                true
                            } else {
                                false
                            }
                        },
                    )+
                }
            }
        }

        /// Context-free instances of every error type without fields.
        pub struct ErrorTypeDefaults {}
        // Constants keep the variant names so they are easy to find.
        #[allow(dead_code, non_upper_case_globals)]
        impl ErrorTypeDefaults {
            $(
                basic_error_default!($item, $($key),*);
            )+
        }
    };
}

// Definite each validation error.
// NOTE: if an error has parameters:
// * the variables in the message need to match the enum struct
// * you need to add an entry to the `render` enum to render the error message as a template
error_types! {
    // ---------------------
    // Assignment errors
    NoSuchAttribute {
        attribute: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // JSON errors
    JsonInvalid {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    JsonType {},
    NeedsPythonObject { method_name: {ctx_type: String, ctx_fn: field_from_context} },
    // ---------------------
    // recursion error
    RecursionLoop {},
    // ---------------------
    // typed dict specific errors
    Missing {},
    FrozenField {},
    FrozenInstance {},
    ExtraForbidden {},
    InvalidKey {},
    GetAttributeError {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // model class specific errors
    ModelType {
        class_name: {ctx_type: String, ctx_fn: field_from_context},
    },
    ModelAttributesType {},
    // ---------------------
    // dataclass errors (we don't talk about ArgsKwargs here for simplicity)
    DataclassType {
        class_name: {ctx_type: String, ctx_fn: field_from_context},
    },
    DataclassExactType {
        class_name: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // namedtuple errors
    NamedTupleType {
        class_name: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // Default factory not called (happens when there's already an error and the factory takes data)
    DefaultFactoryNotCalled {},
    // ---------------------
    // None errors
    NoneRequired {},
    // ---------------------
    // generic comparison errors
    GreaterThan {
        gt: {ctx_type: Number, ctx_fn: field_from_context},
    },
    GreaterThanEqual {
        ge: {ctx_type: Number, ctx_fn: field_from_context},
    },
    LessThan {
        lt: {ctx_type: Number, ctx_fn: field_from_context},
    },
    LessThanEqual {
        le: {ctx_type: Number, ctx_fn: field_from_context},
    },
    MultipleOf {
        multiple_of: {ctx_type: Number, ctx_fn: field_from_context},
    },
    FiniteNumber {},
    // ---------------------
    // generic length errors - used for everything with a length except strings and bytes which need custom messages
    TooShort {
        field_type: {ctx_type: String, ctx_fn: field_from_context},
        min_length: {ctx_type: usize, ctx_fn: field_from_context},
        actual_length: {ctx_type: usize, ctx_fn: field_from_context},
    },
    TooLong {
        field_type: {ctx_type: String, ctx_fn: field_from_context},
        max_length: {ctx_type: usize, ctx_fn: field_from_context},
        actual_length: {ctx_type: Option<usize>, ctx_fn: field_from_context},
    },
    // ---------------------
    // generic collection and iteration errors
    IterableType {},
    IterationError {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // string errors
    StringType {},
    StringUnicode {},
    StringTooShort {
        min_length: {ctx_type: usize, ctx_fn: field_from_context},
    },
    StringTooLong {
        max_length: {ctx_type: usize, ctx_fn: field_from_context},
    },
    StringPatternMismatch {
        pattern: {ctx_type: String, ctx_fn: field_from_context},
    },
    StringNotAscii {},
    // ---------------------
    // enum errors
    Enum {
        expected: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // dict errors
    DictType {},
    FrozenDictType {},
    OrderedDictType {},
    CounterType {},
    MappingType {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // list errors
    ListType {},
    // ---------------------
    // deque errors
    DequeType {},
    // ---------------------
    // tuple errors
    TupleType {},
    // ---------------------
    // set errors
    SetType {},
    SetItemNotHashable {},
    // ---------------------
    // bool errors
    BoolType {},
    BoolParsing {},
    // ---------------------
    // int errors
    IntType {},
    IntParsing {},
    IntParsingSize {},
    IntFromFloat {},
    // ---------------------
    // float errors
    FloatType {},
    FloatParsing {},
    // ---------------------
    // bytes errors
    BytesType {},
    BytesTooShort {
        min_length: {ctx_type: usize, ctx_fn: field_from_context},
    },
    BytesTooLong {
        max_length: {ctx_type: usize, ctx_fn: field_from_context},
    },
    BytesInvalidEncoding {
        encoding: {ctx_type: String, ctx_fn: field_from_context},
        encoding_error: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // python errors from functions
    ValueError {
        error: {ctx_type: Option<String>, ctx_fn: field_from_context}, // the host exception message; Option for Default (EnumIter)
    },
    AssertionError {
        error: {ctx_type: Option<String>, ctx_fn: field_from_context}, // the host exception message; Option for Default (EnumIter)
    },
    // Note: strum message and serialize are not used here
    CustomError {
        // context is a common field in all enums
        error_type: {ctx_type: String, ctx_fn: field_from_context},
        message_template: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // literals
    LiteralError {
        expected: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // missing sentinel
    MissingSentinelError {},
    // ---------------------
    // ellipsis
    EllipsisError {},
    // date errors
    DateType {},
    DateParsing {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    DateFromDatetimeParsing {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    DateFromDatetimeInexact {},
    DatePast {},
    DateFuture {},
    // ---------------------
    // date errors
    TimeType {},
    TimeParsing {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // datetime errors
    DatetimeType {},
    DatetimeParsing {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    DatetimeObjectInvalid {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    DatetimeFromDateParsing {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    DatetimePast {},
    DatetimeFuture {},
    // ---------------------
    // timezone errors
    TimezoneNaive {},
    TimezoneAware {},
    TimezoneOffset {
        tz_expected: {ctx_type: i32, ctx_fn: field_from_context},
        tz_actual: {ctx_type: i32, ctx_fn: field_from_context},
    },
    // ---------------------
    // timedelta errors
    TimeDeltaType {},
    TimeDeltaParsing {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // frozenset errors
    FrozenSetType {},
    // ---------------------
    // introspection types - e.g. isinstance, callable
    IsInstanceOf {
        class: {ctx_type: String, ctx_fn: field_from_context},
    },
    IsSubclassOf {
        class: {ctx_type: String, ctx_fn: field_from_context},
    },
    CallableType {},
    // ---------------------
    // union errors
    UnionTagInvalid {
        discriminator: {ctx_type: String, ctx_fn: field_from_context},
        tag: {ctx_type: String, ctx_fn: field_from_context},
        expected_tags: {ctx_type: String, ctx_fn: field_from_context},
    },
    UnionTagNotFound {
        discriminator: {ctx_type: String, ctx_fn: field_from_context},
    },
    // ---------------------
    // argument errors
    ArgumentsType {},
    MissingArgument {},
    UnexpectedKeywordArgument {},
    MissingKeywordOnlyArgument {},
    UnexpectedPositionalArgument {},
    MissingPositionalOnlyArgument {},
    MultipleArgumentValues {},
    // ---------------------
    // URL errors
    UrlType {},
    UrlParsing {
        // would be great if this could be a static cow, waiting for https://github.com/servo/rust-url/issues/801
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    UrlSyntaxViolation {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    UrlTooLong {
        max_length: {ctx_type: usize, ctx_fn: field_from_context},
    },
    UrlScheme {
        expected_schemes: {ctx_type: String, ctx_fn: field_from_context},
    },
    // UUID errors,
    UuidType {},
    UuidParsing {
        error: {ctx_type: String, ctx_fn: field_from_context},
    },
    UuidVersion {
        expected_version: {ctx_type: usize, ctx_fn: field_from_context},
    },
    // Decimal errors
    DecimalType {},
    DecimalParsing {},
    DecimalMaxDigits {
        max_digits: {ctx_type: u64, ctx_fn: field_from_context},
    },
    DecimalMaxPlaces {
        decimal_places: {ctx_type: u64, ctx_fn: field_from_context},
    },
    DecimalWholeDigits {
        whole_digits: {ctx_type: u64, ctx_fn: field_from_context},
    },
    // Fraction errors
    FractionType {},
    FractionParsing {},
    // Complex errors
    ComplexType {},
    ComplexStrParsing {},
}

macro_rules! render {
    ($template:ident, $($value:ident),* $(,)?) => {
        Ok(
            $template
            $(
                .replace(concat!("{", stringify!($value), "}"), $value)
            )*
        )
    };
}

macro_rules! to_string_render {
    ($template:ident, $($value:ident),* $(,)?) => {
        Ok(
            $template
            $(
                .replace(concat!("{", stringify!($value), "}"), &$value.to_string())
            )*
        )
    };
}

fn plural_s<T: From<u8> + PartialEq>(value: T) -> &'static str {
    if value == 1.into() { "" } else { "s" }
}

fn error_type_lookup() -> &'static HashMap<&'static str, ErrorType> {
    static LOOKUP: OnceLock<HashMap<&'static str, ErrorType>> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        ErrorType::iter()
            .filter(|e| !matches!(e, ErrorType::CustomError { .. }))
            .map(|e| (<&'static str>::from(&e), e))
            .collect()
    })
}

impl ErrorType {
    /// A user-defined error with its own code and message template.
    pub fn new_custom_error(
        error_type: impl Into<String>,
        message_template: impl Into<String>,
        context: Option<Dict>,
    ) -> Self {
        Self::CustomError {
            error_type: error_type.into(),
            message_template: message_template.into(),
            context,
        }
    }

    /// Codes of every built-in error type (custom errors excluded).
    pub fn all_type_names() -> Vec<&'static str> {
        let mut names: Vec<&'static str> = error_type_lookup().keys().copied().collect();
        names.sort_unstable();
        names
    }

    pub fn valid_type(error_type: &str) -> bool {
        error_type_lookup().contains_key(error_type)
    }

    pub fn message_template_python(&self) -> &'static str {
        #[allow(clippy::match_same_arms)] // much nicer to have the messages explicitly listed
        match self {
            Self::NoSuchAttribute { .. } => "Object has no attribute '{attribute}'",
            Self::JsonInvalid { .. } => "Invalid JSON: {error}",
            Self::JsonType { .. } => "JSON input should be string, bytes or bytearray",
            Self::NeedsPythonObject { .. } => {
                "Cannot check `{method_name}` when validating from json, use a JsonOrPython validator instead"
            }
            Self::RecursionLoop { .. } => "Recursion error - cyclic reference detected",
            Self::Missing { .. } => "Field required",
            Self::FrozenField { .. } => "Field is frozen",
            Self::FrozenInstance { .. } => "Instance is frozen",
            Self::ExtraForbidden { .. } => "Extra inputs are not permitted",
            Self::InvalidKey { .. } => "Keys should be strings",
            Self::GetAttributeError { .. } => "Error extracting attribute: {error}",
            Self::ModelType { .. } => {
                "Input should be a valid dictionary or instance of {class_name}"
            }
            Self::ModelAttributesType { .. } => {
                "Input should be a valid dictionary or object to extract fields from"
            }
            Self::DataclassType { .. } => {
                "Input should be a dictionary or an instance of {class_name}"
            }
            Self::DataclassExactType { .. } => "Input should be an instance of {class_name}",
            Self::NamedTupleType { .. } => {
                "Input should be a tuple, list, dictionary or an instance of {class_name}"
            }
            Self::DefaultFactoryNotCalled { .. } => {
                "The default factory uses validated data, but at least one validation error occurred"
            }
            Self::NoneRequired { .. } => "Input should be None",
            Self::GreaterThan { .. } => "Input should be greater than {gt}",
            Self::GreaterThanEqual { .. } => "Input should be greater than or equal to {ge}",
            Self::LessThan { .. } => "Input should be less than {lt}",
            Self::LessThanEqual { .. } => "Input should be less than or equal to {le}",
            Self::MultipleOf { .. } => "Input should be a multiple of {multiple_of}",
            Self::FiniteNumber { .. } => "Input should be a finite number",
            Self::TooShort { .. } => {
                "{field_type} should have at least {min_length} item{expected_plural} after validation, not {actual_length}"
            }
            Self::TooLong { .. } => {
                "{field_type} should have at most {max_length} item{expected_plural} after validation, not {actual_length}"
            }
            Self::IterableType { .. } => "Input should be iterable",
            Self::IterationError { .. } => "Error iterating over object, error: {error}",
            Self::StringType { .. } => "Input should be a valid string",
            Self::StringUnicode { .. } => {
                "Input should be a valid string, unable to parse raw data as a unicode string"
            }
            Self::StringTooShort { .. } => {
                "String should have at least {min_length} character{expected_plural}"
            }
            Self::StringTooLong { .. } => {
                "String should have at most {max_length} character{expected_plural}"
            }
            Self::StringPatternMismatch { .. } => "String should match pattern '{pattern}'",
            Self::StringNotAscii { .. } => "String should contain only ASCII characters",
            Self::Enum { .. } => "Input should be {expected}",
            Self::DictType { .. } => "Input should be a valid dictionary",
            Self::FrozenDictType { .. } => "Input should be a valid frozendict",
            Self::OrderedDictType { .. } => "Input should be a valid OrderedDict",
            Self::CounterType { .. } => "Input should be a valid Counter",
            Self::MappingType { .. } => "Input should be a valid mapping, error: {error}",
            Self::ListType { .. } => "Input should be a valid list",
            Self::DequeType { .. } => "Input should be a valid deque",
            Self::TupleType { .. } => "Input should be a valid tuple",
            Self::SetType { .. } => "Input should be a valid set",
            Self::SetItemNotHashable { .. } => "Set items should be hashable",
            Self::BoolType { .. } => "Input should be a valid boolean",
            Self::BoolParsing { .. } => {
                "Input should be a valid boolean, unable to interpret input"
            }
            Self::IntType { .. } => "Input should be a valid integer",
            Self::IntParsing { .. } => {
                "Input should be a valid integer, unable to parse string as an integer"
            }
            Self::IntFromFloat { .. } => {
                "Input should be a valid integer, got a number with a fractional part"
            }
            Self::IntParsingSize { .. } => {
                "Unable to parse input string as an integer, exceeded maximum size"
            }
            Self::FloatType { .. } => "Input should be a valid number",
            Self::FloatParsing { .. } => {
                "Input should be a valid number, unable to parse string as a number"
            }
            Self::BytesType { .. } => "Input should be a valid bytes",
            Self::BytesTooShort { .. } => {
                "Data should have at least {min_length} byte{expected_plural}"
            }
            Self::BytesTooLong { .. } => {
                "Data should have at most {max_length} byte{expected_plural}"
            }
            Self::BytesInvalidEncoding { .. } => {
                "Data should be valid {encoding}: {encoding_error}"
            }
            Self::ValueError { .. } => "Value error, {error}",
            Self::AssertionError { .. } => "Assertion failed, {error}",
            Self::CustomError { .. } => "", // custom errors are handled separately
            Self::LiteralError { .. } => "Input should be {expected}",
            Self::MissingSentinelError { .. } => "Input should be the 'MISSING' sentinel",
            Self::EllipsisError { .. } => "Input should be the 'Ellipsis' literal",
            Self::DateType { .. } => "Input should be a valid date",
            Self::DateParsing { .. } => {
                "Input should be a valid date in the format YYYY-MM-DD, {error}"
            }
            Self::DateFromDatetimeParsing { .. } => {
                "Input should be a valid date or datetime, {error}"
            }
            Self::DateFromDatetimeInexact { .. } => {
                "Datetimes provided to dates should have zero time - e.g. be exact dates"
            }
            Self::DatePast { .. } => "Date should be in the past",
            Self::DateFuture { .. } => "Date should be in the future",
            Self::TimeType { .. } => "Input should be a valid time",
            Self::TimeParsing { .. } => "Input should be in a valid time format, {error}",
            Self::DatetimeType { .. } => "Input should be a valid datetime",
            Self::DatetimeParsing { .. } => "Input should be a valid datetime, {error}",
            Self::DatetimeObjectInvalid { .. } => "Invalid datetime object, got {error}",
            Self::DatetimeFromDateParsing { .. } => {
                "Input should be a valid datetime or date, {error}"
            }
            Self::DatetimePast { .. } => "Input should be in the past",
            Self::DatetimeFuture { .. } => "Input should be in the future",
            Self::TimezoneNaive { .. } => "Input should not have timezone info",
            Self::TimezoneAware { .. } => "Input should have timezone info",
            Self::TimezoneOffset { .. } => {
                "Timezone offset of {tz_expected} required, got {tz_actual}"
            }
            Self::TimeDeltaType { .. } => "Input should be a valid timedelta",
            Self::TimeDeltaParsing { .. } => "Input should be a valid timedelta, {error}",
            Self::FrozenSetType { .. } => "Input should be a valid frozenset",
            Self::IsInstanceOf { .. } => "Input should be an instance of {class}",
            Self::IsSubclassOf { .. } => "Input should be a subclass of {class}",
            Self::CallableType { .. } => "Input should be callable",
            Self::UnionTagInvalid { .. } => {
                "Input tag '{tag}' found using {discriminator} does not match any of the expected tags: {expected_tags}"
            }
            Self::UnionTagNotFound { .. } => {
                "Unable to extract tag using discriminator {discriminator}"
            }
            Self::ArgumentsType { .. } => "Arguments must be a tuple, list or a dictionary",
            Self::MissingArgument { .. } => "Missing required argument",
            Self::UnexpectedKeywordArgument { .. } => "Unexpected keyword argument",
            Self::MissingKeywordOnlyArgument { .. } => "Missing required keyword only argument",
            Self::UnexpectedPositionalArgument { .. } => "Unexpected positional argument",
            Self::MissingPositionalOnlyArgument { .. } => {
                "Missing required positional only argument"
            }
            Self::MultipleArgumentValues { .. } => "Got multiple values for argument",
            Self::UrlType { .. } => "URL input should be a string or URL",
            Self::UrlParsing { .. } => "Input should be a valid URL, {error}",
            Self::UrlSyntaxViolation { .. } => "Input violated strict URL syntax rules, {error}",
            Self::UrlTooLong { .. } => {
                "URL should have at most {max_length} character{expected_plural}"
            }
            Self::UrlScheme { .. } => "URL scheme should be {expected_schemes}",
            Self::UuidType { .. } => "UUID input should be a string, bytes or UUID object",
            Self::UuidParsing { .. } => "Input should be a valid UUID, {error}",
            Self::UuidVersion { .. } => "UUID version {expected_version} expected",
            Self::DecimalType { .. } => {
                "Decimal input should be an integer, float, string or Decimal object"
            }
            Self::DecimalParsing { .. } => "Input should be a valid decimal",
            Self::DecimalMaxDigits { .. } => {
                "Decimal input should have no more than {max_digits} digit{expected_plural} in total"
            }
            Self::DecimalMaxPlaces { .. } => {
                "Decimal input should have no more than {decimal_places} decimal place{expected_plural}"
            }
            Self::DecimalWholeDigits { .. } => {
                "Decimal input should have no more than {whole_digits} digit{expected_plural} before the decimal point"
            }
            Self::FractionParsing { .. } => "Input is not a valid fraction",
            Self::FractionType { .. } => {
                "Fraction input should be an integer, float, string or Fraction object"
            }
            Self::ComplexType { .. } => {
                "Input should be a valid python complex object, a number, or a valid complex string following the rules at https://docs.python.org/3/library/functions.html#complex"
            }
            Self::ComplexStrParsing { .. } => {
                "Input should be a valid complex string following the rules at https://docs.python.org/3/library/functions.html#complex"
            }
        }
    }

    pub fn message_template_json(&self) -> &'static str {
        match self {
            Self::NoneRequired { .. } => "Input should be null",
            Self::ListType { .. }
            | Self::DequeType { .. }
            | Self::TupleType { .. }
            | Self::IterableType { .. }
            | Self::SetType { .. }
            | Self::FrozenSetType { .. } => "Input should be a valid array",
            Self::ModelType { .. }
            | Self::ModelAttributesType { .. }
            | Self::DictType { .. }
            | Self::FrozenDictType { .. }
            | Self::OrderedDictType { .. }
            | Self::CounterType { .. }
            | Self::DataclassType { .. } => "Input should be an object",
            Self::NamedTupleType { .. } => "Input should be an array or an object",
            Self::TimeDeltaType { .. } => "Input should be a valid duration",
            Self::TimeDeltaParsing { .. } => "Input should be a valid duration, {error}",
            Self::ArgumentsType { .. } => "Arguments must be an array or an object",
            _ => self.message_template_python(),
        }
    }

    /// Perl input (docs/DIVERGENCES.md #8): Perl's words for the kinds of data, pydantic's
    /// templates otherwise.
    pub fn message_template_perl(&self) -> &'static str {
        match self {
            Self::NoneRequired { .. } => "Input should be undef",
            Self::ListType { .. }
            | Self::DequeType { .. }
            | Self::TupleType { .. }
            | Self::SetType { .. }
            | Self::FrozenSetType { .. } => "Input should be an array reference",
            Self::DictType { .. }
            | Self::FrozenDictType { .. }
            | Self::OrderedDictType { .. }
            | Self::CounterType { .. } => "Input should be a hash reference",
            Self::ModelType { .. } => {
                "Input should be a hash reference or an instance of {class_name}"
            }
            Self::ModelAttributesType { .. } => {
                "Input should be a hash reference or an object to extract fields from"
            }
            _ => self.message_template_python(),
        }
    }

    pub fn type_string(&self) -> String {
        match self {
            Self::CustomError { error_type, .. } => error_type.clone(),
            _ => self.to_string(),
        }
    }

    pub fn render_message(&self, input_type: InputType) -> CoreResult<String> {
        let tmpl = match input_type {
            InputType::Python => self.message_template_python(),
            InputType::Perl => self.message_template_perl(),
            InputType::Json | InputType::String => self.message_template_json(),
        };
        // Perl calls the containers arrays and hashes; the context keeps pydantic's names.
        let perl_field_type = |field_type: &str| -> String {
            match (input_type, field_type) {
                (InputType::Perl, "List" | "Tuple" | "Set" | "Frozenset" | "Deque") => {
                    "Array".into()
                }
                (InputType::Perl, "Dictionary") => "Hash".into(),
                _ => field_type.to_owned(),
            }
        };
        match self {
            Self::NoSuchAttribute { attribute, .. } => render!(tmpl, attribute),
            Self::JsonInvalid { error, .. }
            | Self::GetAttributeError { error, .. }
            | Self::IterationError { error, .. }
            | Self::DatetimeObjectInvalid { error, .. }
            | Self::UrlParsing { error, .. }
            | Self::UuidParsing { error, .. }
            | Self::MappingType { error, .. }
            | Self::DateParsing { error, .. }
            | Self::DateFromDatetimeParsing { error, .. }
            | Self::TimeParsing { error, .. }
            | Self::DatetimeParsing { error, .. }
            | Self::DatetimeFromDateParsing { error, .. }
            | Self::TimeDeltaParsing { error, .. }
            | Self::UrlSyntaxViolation { error, .. } => render!(tmpl, error),
            Self::NeedsPythonObject { method_name, .. } => render!(tmpl, method_name),
            Self::ModelType { class_name, .. }
            | Self::DataclassType { class_name, .. }
            | Self::DataclassExactType { class_name, .. }
            | Self::NamedTupleType { class_name, .. } => render!(tmpl, class_name),
            Self::GreaterThan { gt, .. } => to_string_render!(tmpl, gt),
            Self::GreaterThanEqual { ge, .. } => to_string_render!(tmpl, ge),
            Self::LessThan { lt, .. } => to_string_render!(tmpl, lt),
            Self::LessThanEqual { le, .. } => to_string_render!(tmpl, le),
            Self::MultipleOf { multiple_of, .. } => to_string_render!(tmpl, multiple_of),
            Self::TooShort {
                field_type,
                min_length,
                actual_length,
                ..
            } => {
                let expected_plural = plural_s(*min_length);
                let field_type = perl_field_type(field_type);
                to_string_render!(tmpl, field_type, min_length, actual_length, expected_plural,)
            }
            Self::TooLong {
                field_type,
                max_length,
                actual_length,
                ..
            } => {
                let expected_plural = plural_s(*max_length);
                let actual_length =
                    actual_length.map_or_else(|| "more".to_owned(), |v| v.to_string());
                let field_type = perl_field_type(field_type);
                to_string_render!(tmpl, field_type, max_length, actual_length, expected_plural,)
            }
            Self::StringTooShort { min_length, .. } | Self::BytesTooShort { min_length, .. } => {
                let expected_plural = plural_s(*min_length);
                to_string_render!(tmpl, min_length, expected_plural)
            }
            Self::StringTooLong { max_length, .. }
            | Self::BytesTooLong { max_length, .. }
            | Self::UrlTooLong { max_length, .. } => {
                let expected_plural = plural_s(*max_length);
                to_string_render!(tmpl, max_length, expected_plural)
            }
            Self::StringPatternMismatch { pattern, .. } => render!(tmpl, pattern),
            Self::Enum { expected, .. } => to_string_render!(tmpl, expected),
            Self::BytesInvalidEncoding {
                encoding,
                encoding_error,
                ..
            } => render!(tmpl, encoding, encoding_error),
            Self::ValueError { error, .. } | Self::AssertionError { error, .. } => {
                let error = error.as_deref().unwrap_or("None");
                render!(tmpl, error)
            }
            Self::CustomError {
                message_template,
                context,
                ..
            } => format_custom_message(message_template, context.as_ref()),
            Self::LiteralError { expected, .. } => render!(tmpl, expected),
            Self::TimezoneOffset {
                tz_expected,
                tz_actual,
                ..
            } => to_string_render!(tmpl, tz_expected, tz_actual),
            Self::IsInstanceOf { class, .. } | Self::IsSubclassOf { class, .. } => {
                render!(tmpl, class)
            }
            Self::UnionTagInvalid {
                discriminator,
                tag,
                expected_tags,
                ..
            } => render!(tmpl, discriminator, tag, expected_tags),
            Self::UnionTagNotFound { discriminator, .. } => render!(tmpl, discriminator),
            Self::UrlScheme {
                expected_schemes, ..
            } => render!(tmpl, expected_schemes),
            Self::UuidVersion {
                expected_version, ..
            } => to_string_render!(tmpl, expected_version),
            Self::DecimalMaxDigits { max_digits, .. } => {
                let expected_plural = plural_s(*max_digits);
                to_string_render!(tmpl, max_digits, expected_plural)
            }
            Self::DecimalMaxPlaces { decimal_places, .. } => {
                let expected_plural = plural_s(*decimal_places);
                to_string_render!(tmpl, decimal_places, expected_plural)
            }
            Self::DecimalWholeDigits { whole_digits, .. } => {
                let expected_plural = plural_s(*whole_digits);
                to_string_render!(tmpl, whole_digits, expected_plural)
            }
            _ => Ok(tmpl.to_string()),
        }
    }

    /// The `ctx` of this error as pydantic reports it, or `None` when there is nothing to report.
    pub fn context(&self) -> Option<Dict> {
        let mut dict = Dict::new();
        let custom_ctx_used = self.update_ctx(&mut dict);

        if let Self::CustomError { .. } = self {
            if custom_ctx_used {
                // The custom error type and template are reported at the top level instead.
                dict.remove_str("error_type");
                dict.remove_str("message_template");
                Some(dict)
            } else {
                None
            }
        } else if custom_ctx_used || !dict.is_empty() {
            Some(dict)
        } else {
            None
        }
    }
}

/// Port of upstream `PydanticCustomError::format_message`.
fn format_custom_message(message_template: &str, context: Option<&Dict>) -> CoreResult<String> {
    let mut message = message_template.to_string();
    if let Some(ctx) = context {
        for (key, value) in ctx.iter() {
            let Value::Str(key) = key else {
                return Err(CoreError::Type(format!(
                    "'{}' object is not an instance of 'str'",
                    key.type_name()
                )));
            };
            let placeholder = format!("{{{key}}}");
            let replacement = match value {
                Value::Str(s) => s.clone(),
                Value::Int(i) => i.to_string(),
                other => other.py_str(),
            };
            message = message.replace(&placeholder, &replacement);
        }
    }
    Ok(message)
}

/// A numeric (or pre-formatted) constraint value such as `gt` or `multiple_of`.
#[derive(Clone, Debug, PartialEq)]
pub enum Number {
    Int(i64),
    BigInt(BigInt),
    Float(f64),
    String(String),
}

impl Default for Number {
    fn default() -> Self {
        Self::Int(0)
    }
}

impl From<i64> for Number {
    fn from(i: i64) -> Self {
        Self::Int(i)
    }
}

impl From<f64> for Number {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<String> for Number {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<BigInt> for Number {
    fn from(i: BigInt) -> Self {
        Self::BigInt(i)
    }
}

impl From<crate::input::Int> for Number {
    fn from(i: crate::input::Int) -> Self {
        match i {
            crate::input::Int::I64(i) => Self::Int(i),
            crate::input::Int::Big(b) => Self::BigInt(b),
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Float(s) => write!(f, "{s}"),
            Self::Int(i) => write!(f, "{i}"),
            Self::BigInt(i) => write!(f, "{i}"),
            Self::String(s) => write!(f, "{s}"),
        }
    }
}
