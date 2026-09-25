//! `enum` schema, so named because `enum` is a Rust keyword. Port of upstream
//! `validators/enum_.rs`.
//!
//! Members are `Value::Enum`s and the class is a name (docs/DIVERGENCES.md #13). When no member
//! matches, upstream calls the enum class, which looks the value up among the members by Python
//! equality and then runs the class's `_missing_` hook; the lookup is done here, and a schema
//! that names a `missing` hook needs host callbacks (task 00054).

use std::sync::Arc;

use crate::build_tools::{SchemaDict, is_strict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ValError, ValResult};
use crate::input::{Input, InputType};
use crate::value::{Dict, EnumMixin, Value};

use super::literal::{LiteralLookup, expected_repr};
use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Validator};

/// The builtin type an enum class mixes in: how input is matched against the member values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubType {
    Plain,
    Int,
    Str,
    Float,
}

#[derive(Debug, Clone)]
pub struct EnumValidator {
    sub_type: SubType,
    class: String,
    lookup: Box<LiteralLookup<Value>>,
    /// Members in declaration order, for the fallback lookup by value.
    members: Vec<Value>,
    expected_repr: String,
    strict: bool,
    class_repr: String,
    name: String,
}

impl BuildValidator for EnumValidator {
    const EXPECTED_TYPE: &'static str = "enum";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let members: Vec<Value> = schema.get_as_req("members")?;
        if members.is_empty() {
            return schema_err!("`members` should have length > 0");
        }
        let class: String = schema.get_as_req("cls")?;
        let mut values = Vec::with_capacity(members.len());
        for member in &members {
            let Value::Enum(member) = member else {
                return schema_err!("`members` should be members of the enum class");
            };
            values.push(member.value.clone());
        }
        if !matches!(schema.get_str("missing"), None | Some(Value::None)) {
            return schema_err!(
                "`missing` is not supported yet: host callbacks are not implemented"
            );
        }

        let repr_args: Vec<String> = values.iter().map(Value::repr).collect();
        let expected_repr = expected_repr(&repr_args);
        let class_repr: String = schema.get_as("cls_repr")?.unwrap_or_else(|| class.clone());
        let lookup = Box::new(LiteralLookup::new(
            values.iter().zip(members.iter().cloned()),
        )?);

        let sub_type_name: Option<String> = schema.get_as("sub_type")?;
        let (sub_type, prefix) = match sub_type_name.as_deref() {
            Some("int") => (SubType::Int, "int-enum"),
            Some("str") => (SubType::Str, "str-enum"),
            Some("float") => (SubType::Float, "float-enum"),
            Some(_) => {
                return schema_err!("`sub_type` must be one of: 'int', 'str', 'float' or None");
            }
            None => (SubType::Plain, "enum"),
        };
        Ok(Arc::new(CombinedValidator::Enum(Box::new(Self {
            sub_type,
            class,
            lookup,
            members,
            expected_repr,
            strict: is_strict(schema, config)?,
            name: format!("{prefix}[{class_repr}]"),
            class_repr,
        }))))
    }
}

impl EnumValidator {
    /// The member matching the input, by the rules of the enum's sub type.
    fn lookup(&self, input: &(impl Input + ?Sized), strict: bool) -> ValResult<Option<&Value>> {
        match self.sub_type {
            SubType::Int => self.lookup.validate_int(input, strict),
            SubType::Str => self.lookup.validate_str(input, strict),
            SubType::Float => self.lookup.validate_float(input, strict),
            SubType::Plain => {
                if let Some((_, member)) = self.lookup.validate(input)? {
                    return Ok(Some(member));
                }
                if strict {
                    return Ok(None);
                }
                // compatibility with pydantic 2.6: str and int values (and their subclasses) are
                // matched laxly, as are floats against int members
                let Some(value) = input.as_value() else {
                    return Ok(None);
                };
                let mixin = match value {
                    Value::Enum(member) => member.mixin,
                    _ => None,
                };
                if matches!(value, Value::Str(_)) || mixin == Some(EnumMixin::Str) {
                    self.lookup.validate_str(input, false)
                } else if matches!(
                    value,
                    Value::Int(_) | Value::BigInt(_) | Value::Bool(_) | Value::Float(_)
                ) || matches!(mixin, Some(EnumMixin::Int | EnumMixin::Float))
                {
                    self.lookup.validate_int(input, false)
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// What calling the enum class does before `_missing_`: the member whose value equals the
    /// input.
    fn member_by_value(&self, input: &(impl Input + ?Sized)) -> Option<&Value> {
        let value = input.to_value();
        self.members.iter().find(|member| match member {
            Value::Enum(member) => member.value.py_eq(&value),
            _ => false,
        })
    }
}

impl Validator for EnumValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        if let Some(Value::Enum(member)) = input.as_value()
            && member.class == self.class
        {
            return Ok(Value::Enum(member.clone()));
        }
        let strict = state.strict_or(self.strict);
        // host input (Python's or Perl's) must be a member; JSON has only values
        if strict
            && matches!(
                state.extra().input_type,
                InputType::Python | InputType::Perl
            )
        {
            return Err(ValError::new(
                ErrorType::IsInstanceOf {
                    class: self.class_repr.clone(),
                    context: None,
                },
                input,
            ));
        }

        state.floor_exactness(Exactness::Lax);

        if let Some(member) = self.lookup(input, strict)? {
            return Ok(member.clone());
        }
        if let Some(member) = self.member_by_value(input) {
            return Ok(member.clone());
        }
        Err(ValError::new(
            ErrorType::Enum {
                expected: self.expected_repr.clone(),
                context: None,
            },
            input,
        ))
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}
