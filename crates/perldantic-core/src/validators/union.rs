//! `union` and `tagged-union` schemas. Port of upstream `validators/union.rs` and
//! `common/union.rs`.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::build_tools::{SchemaDict, schema_err, schema_or_config};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ToErrorValue, ValError, ValLineError, ValResult};
use crate::input::{BorrowInput, Input, ValidatedDict};
use crate::lookup_key::{LookupPath, validation_alias_paths};
use crate::value::{Dict, Value};

use super::custom_error::CustomError;
use super::literal::LiteralLookup;
use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
enum UnionMode {
    Smart,
    LeftToRight,
}

impl FromStr for UnionMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "smart" => Ok(Self::Smart),
            "left_to_right" => Ok(Self::LeftToRight),
            s => schema_err!("Invalid union mode: `{s}`, expected `smart` or `left_to_right`"),
        }
    }
}

#[derive(Debug)]
pub struct UnionValidator {
    mode: UnionMode,
    choices: Vec<(Arc<CombinedValidator>, Option<String>)>,
    custom_error: Option<CustomError>,
    name: String,
}

impl BuildValidator for UnionValidator {
    const EXPECTED_TYPE: &'static str = "union";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let choices: Vec<(Arc<CombinedValidator>, Option<String>)> = schema
            .get_as_req::<Vec<Value>>("choices")?
            .iter()
            .map(|choice| {
                // A choice is a schema, or a `(schema, label)` tuple.
                let (choice, label) = match choice {
                    Value::Tuple(items) => {
                        let choice = items
                            .first()
                            .ok_or_else(|| CoreError::Value("tuple index out of range".into()))?;
                        let label = items
                            .get(1)
                            .ok_or_else(|| CoreError::Value("tuple index out of range".into()))?;
                        (choice, Some(label.py_str()))
                    }
                    choice => (choice, None),
                };
                Ok((
                    build_validator(as_dict(choice)?, config, definitions)?,
                    label,
                ))
            })
            .collect::<CoreResult<_>>()?;

        let auto_collapse = || schema.get_as_req("auto_collapse").unwrap_or(true);
        let mode = schema
            .get_as::<String>("mode")?
            .map_or(Ok(UnionMode::Smart), |mode| UnionMode::from_str(&mode))?;
        match choices.len() {
            0 => schema_err!("One or more union choices required"),
            1 if auto_collapse() => Ok(choices.into_iter().next().unwrap().0),
            _ => {
                let descr = choices
                    .iter()
                    .map(|(choice, label)| label.as_deref().unwrap_or(choice.get_name()))
                    .collect::<Vec<_>>()
                    .join(",");

                Ok(Arc::new(CombinedValidator::Union(Self {
                    mode,
                    choices,
                    custom_error: CustomError::build(schema, config, definitions)?,
                    name: format!("{}[{descr}]", Self::EXPECTED_TYPE),
                })))
            }
        }
    }
}

impl UnionValidator {
    fn validate_smart(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let old_exactness = state.exactness;
        let old_fields_set_count = state.fields_set_count;

        let mut errors = MaybeErrors::new(self.custom_error.as_ref());
        let mut should_omit = false;

        let mut best_match: Option<(Value, Exactness, Option<usize>)> = None;

        for (choice, label) in &self.choices {
            state.exactness = Some(Exactness::Exact);
            state.fields_set_count = None;
            let result = choice.validate(input, state);
            match result {
                Ok(new_success) => match (state.exactness, state.fields_set_count) {
                    (Some(Exactness::Exact), None) => {
                        // exact match with no fields set data, return immediately, restoring
                        // any previous exactness
                        state.exactness = old_exactness;
                        state.fields_set_count = old_fields_set_count;
                        return Ok(new_success);
                    }
                    _ => {
                        // success should always have an exactness
                        debug_assert_ne!(state.exactness, None);

                        let new_exactness = state.exactness.unwrap_or(Exactness::Lax);
                        let new_fields_set_count = state.fields_set_count;

                        // we use both the exactness and the fields_set_count to determine the
                        // best union member match: if fields_set_count is available for the
                        // current best match and the new candidate, it is the primary metric
                        // (more fields set wins) and exactness breaks ties; otherwise exactness
                        // alone decides.
                        let new_success_is_best_match: bool = best_match.as_ref().is_none_or(
                            |(_, cur_exactness, cur_fields_set_count)| match (
                                *cur_fields_set_count,
                                new_fields_set_count,
                            ) {
                                (Some(cur), Some(new)) if cur != new => cur < new,
                                _ => *cur_exactness < new_exactness,
                            },
                        );

                        if new_success_is_best_match {
                            best_match = Some((new_success, new_exactness, new_fields_set_count));
                        }
                    }
                },
                Err(ValError::Omit) => {
                    if best_match.is_none() {
                        should_omit = true;
                    }
                }
                Err(ValError::LineErrors(lines)) => {
                    // if we don't yet know this validation will succeed, record the error
                    if best_match.is_none() {
                        errors.push(choice, label.as_deref(), lines);
                    }
                }
                otherwise => return otherwise,
            }
        }

        // restore previous validation state to prepare for any future validations
        state.exactness = old_exactness;
        state.fields_set_count = old_fields_set_count;

        if let Some((best_match, exactness, fields_set_count)) = best_match {
            state.floor_exactness(exactness);
            if let Some(count) = fields_set_count {
                state.add_fields_set(count);
            }
            return Ok(best_match);
        }
        // if there were no successful matches, but there was at least one omit, return omit
        // instead of errors
        if should_omit {
            return Err(ValError::Omit);
        }
        // no matches, build errors
        Err(errors.into_val_error(input))
    }

    fn validate_left_to_right(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let mut errors = MaybeErrors::new(self.custom_error.as_ref());

        for (validator, label) in &self.choices {
            match validator.validate(input, state) {
                Err(ValError::LineErrors(lines)) => {
                    errors.push(validator, label.as_deref(), lines);
                }
                otherwise => return otherwise,
            }
        }

        Err(errors.into_val_error(input))
    }
}

impl Validator for UnionValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        match self.mode {
            UnionMode::Smart => self.validate_smart(input, state),
            UnionMode::LeftToRight => self.validate_left_to_right(input, state),
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

struct ChoiceLineErrors<'a> {
    choice: &'a CombinedValidator,
    label: Option<&'a str>,
    line_errors: Vec<ValLineError>,
}

enum MaybeErrors<'a> {
    Custom(&'a CustomError),
    Errors(Vec<ChoiceLineErrors<'a>>),
}

impl<'a> MaybeErrors<'a> {
    fn new(custom_error: Option<&'a CustomError>) -> Self {
        match custom_error {
            Some(custom_error) => Self::Custom(custom_error),
            None => Self::Errors(Vec::new()),
        }
    }

    fn push(
        &mut self,
        choice: &'a CombinedValidator,
        label: Option<&'a str>,
        line_errors: Vec<ValLineError>,
    ) {
        match self {
            Self::Custom(_) => {}
            Self::Errors(errors) => errors.push(ChoiceLineErrors {
                choice,
                label,
                line_errors,
            }),
        }
    }

    fn into_val_error(self, input: impl ToErrorValue) -> ValError {
        match self {
            Self::Custom(custom_error) => custom_error.as_val_error(input),
            Self::Errors(errors) => ValError::LineErrors(
                errors
                    .into_iter()
                    .flat_map(
                        |ChoiceLineErrors {
                             choice,
                             label,
                             line_errors,
                         }| {
                            line_errors.into_iter().map(move |err| {
                                let case_label = label.unwrap_or(choice.get_name());
                                err.with_outer_location(case_label)
                            })
                        },
                    )
                    .collect(),
            ),
        }
    }
}

/// How a tagged union finds the tag. Upstream also accepts a function, which needs host
/// callbacks.
#[derive(Debug)]
struct Discriminator(Vec<LookupPath>);

impl Discriminator {
    fn new(raw: &Value) -> CoreResult<Self> {
        Ok(Self(validation_alias_paths(raw)?))
    }
}

impl fmt::Display for Discriminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let paths: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        f.write_str(&paths.join(" | "))
    }
}

#[derive(Debug)]
pub struct TaggedUnionValidator {
    discriminator: Discriminator,
    lookup: LiteralLookup<Arc<CombinedValidator>>,
    from_attributes: bool,
    custom_error: Option<CustomError>,
    tags_repr: String,
    discriminator_repr: String,
    name: String,
}

impl BuildValidator for TaggedUnionValidator {
    const EXPECTED_TYPE: &'static str = "tagged-union";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let discriminator = Discriminator::new(&schema.get_as_req::<Value>("discriminator")?)?;
        let discriminator_repr = discriminator.to_string();

        let schema_choices: Dict = schema.get_as_req("choices")?;
        let mut tags_repr = Vec::with_capacity(schema_choices.len());
        let mut descr = Vec::with_capacity(schema_choices.len());
        let mut lookup_map = Vec::with_capacity(schema_choices.len());
        for (choice_key, choice_schema) in schema_choices.iter() {
            let validator = build_validator(as_dict(choice_schema)?, config, definitions)?;
            tags_repr.push(choice_key.repr());
            // no spaces in get_name() output to make loc easy to read
            descr.push(validator.get_name().to_owned());
            lookup_map.push((choice_key, validator));
        }

        let lookup = LiteralLookup::new(lookup_map.into_iter())?;

        let from_attributes =
            schema_or_config(schema, config, "from_attributes", "from_attributes")?.unwrap_or(true);

        Ok(Arc::new(CombinedValidator::TaggedUnion(Box::new(Self {
            discriminator,
            lookup,
            from_attributes,
            custom_error: CustomError::build(schema, config, definitions)?,
            tags_repr: tags_repr.join(", "),
            discriminator_repr,
            name: format!("{}[{}]", Self::EXPECTED_TYPE, descr.join(",")),
        }))))
    }
}

impl Validator for TaggedUnionValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let from_attributes = state
            .extra()
            .from_attributes
            .unwrap_or(self.from_attributes);
        let dict = input.validate_model_fields(state.strict_or(false), from_attributes)?;
        let Some(tag_result) = self
            .discriminator
            .0
            .iter()
            .find_map(|path| dict.get_item(path).transpose())
        else {
            return Err(self.tag_not_found(input));
        };
        let tag = tag_result?;
        self.find_call_validator(&tag.borrow_input().to_value(), input, state)
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

impl TaggedUnionValidator {
    fn find_call_validator(
        &self,
        tag: &Value,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        if let Ok(Some((tag, validator))) = self.lookup.validate(tag) {
            return match validator.validate(input, state) {
                Ok(res) => Ok(res),
                Err(err) => Err(err.with_outer_location(tag)),
            };
        }
        match self.custom_error {
            Some(ref custom_error) => Err(custom_error.as_val_error(input)),
            None => Err(ValError::new(
                ErrorType::UnionTagInvalid {
                    discriminator: self.discriminator_repr.clone(),
                    tag: tag.py_str(),
                    expected_tags: self.tags_repr.clone(),
                    context: None,
                },
                input,
            )),
        }
    }

    fn tag_not_found(&self, input: &(impl Input + ?Sized)) -> ValError {
        match self.custom_error {
            Some(ref custom_error) => custom_error.as_val_error(input),
            None => ValError::new(
                ErrorType::UnionTagNotFound {
                    discriminator: self.discriminator_repr.clone(),
                    context: None,
                },
                input,
            ),
        }
    }
}
