//! `arguments` schema. Port of upstream `validators/arguments.rs`: the arguments of a call,
//! positional and keyword, each validated by its parameter; the output is the tuple
//! `(args, kwargs)`.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use crate::build_tools::{ExtraBehavior, SchemaDict, schema_err, schema_or_config_same};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorTypeDefaults, ValError, ValLineError, ValResult};
use crate::input::{BorrowInput, ConsumeIterator, Input, ValidatedDict, ValidatedTuple};
use crate::lookup_key::{LookupPath, LookupPathCollection, LookupType};
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, build_validator};

#[derive(Debug, PartialEq)]
enum VarKwargsMode {
    Uniform,
    UnpackedTypedDict,
}

impl FromStr for VarKwargsMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "uniform" => Ok(Self::Uniform),
            "unpacked-typed-dict" => Ok(Self::UnpackedTypedDict),
            s => schema_err!(
                "Invalid var_kwargs mode: `{s}`, expected `uniform` or `unpacked-typed-dict`"
            ),
        }
    }
}

#[derive(Debug)]
struct Parameter {
    positional: bool,
    name: String,
    /// The name as the validation state holds it (`info.field_name`).
    shared_name: Arc<str>,
    /// Whether the value goes to the keyword arguments of the output.
    keyword: bool,
    validator: Arc<CombinedValidator>,
    /// Lookup keys, only for keyword or positional-or-keyword parameters.
    lookup_path_collection: Option<LookupPathCollection>,
}

#[derive(Debug)]
pub struct ArgumentsValidator {
    parameters: Vec<Parameter>,
    positional_params_count: usize,
    var_args_validator: Option<Arc<CombinedValidator>>,
    var_kwargs_mode: VarKwargsMode,
    var_kwargs_validator: Option<Arc<CombinedValidator>>,
    loc_by_alias: bool,
    extra: ExtraBehavior,
    validate_by_alias: Option<bool>,
    validate_by_name: Option<bool>,
}

impl BuildValidator for ArgumentsValidator {
    const EXPECTED_TYPE: &'static str = "arguments";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let arguments_schema: Vec<Value> = schema.get_as_req("arguments_schema")?;
        let mut parameters: Vec<Parameter> = Vec::with_capacity(arguments_schema.len());

        let mut positional_params_count = 0;
        let mut had_default_arg = false;
        let mut had_keyword_only = false;

        for (arg_index, arg) in arguments_schema.iter().enumerate() {
            let Value::Dict(arg) = arg else {
                return Err(CoreError::Type(format!(
                    "'{}' object cannot be converted to 'PyDict'",
                    arg.type_name()
                )));
            };

            let name: String = arg.get_as_req("name")?;
            let mode: Option<String> = arg.get_as("mode")?;
            let mode = mode.as_deref().unwrap_or("positional_or_keyword");
            let positional = mode == "positional_only" || mode == "positional_or_keyword";
            if positional {
                positional_params_count = arg_index + 1;
            }
            if mode == "keyword_only" {
                had_keyword_only = true;
            }

            let keyword = matches!(mode, "keyword_only" | "positional_or_keyword");
            let lookup_path_collection = if keyword {
                Some(LookupPathCollection::new(arg.get_str("alias"), &name)?)
            } else {
                None
            };

            let schema: Dict = arg.get_as_req("schema")?;
            let validator = match build_validator(&schema, config, definitions) {
                Ok(v) => v,
                Err(err) => return schema_err!("Parameter '{name}':\n  {err}"),
            };

            let has_default = match validator.as_ref() {
                CombinedValidator::WithDefault(v) => {
                    if v.omit_on_error() {
                        return schema_err!(
                            "Parameter '{name}': omit_on_error cannot be used with arguments"
                        );
                    }
                    v.has_default()
                }
                _ => false,
            };

            if had_default_arg && !has_default && !had_keyword_only {
                return schema_err!("Non-default argument '{name}' follows default argument");
            } else if has_default {
                had_default_arg = true;
            }

            parameters.push(Parameter {
                positional,
                shared_name: Arc::from(name.as_str()),
                name,
                keyword,
                validator,
                lookup_path_collection,
            });
        }

        let var_kwargs_mode: Option<String> = schema.get_as("var_kwargs_mode")?;
        let var_kwargs_mode =
            VarKwargsMode::from_str(var_kwargs_mode.as_deref().unwrap_or("uniform"))?;
        let var_kwargs_validator = match schema.get_as::<Dict>("var_kwargs_schema")? {
            Some(v) => Some(build_validator(&v, config, definitions)?),
            None => None,
        };
        if var_kwargs_mode == VarKwargsMode::UnpackedTypedDict && var_kwargs_validator.is_none() {
            return schema_err!(
                "`var_kwargs_schema` must be specified when `var_kwargs_mode` is `'unpacked-typed-dict'`"
            );
        }
        let var_args_validator = match schema.get_as::<Dict>("var_args_schema")? {
            Some(v) => Some(build_validator(&v, config, definitions)?),
            None => None,
        };

        Ok(Arc::new(CombinedValidator::Arguments(Box::new(Self {
            parameters,
            positional_params_count,
            var_args_validator,
            var_kwargs_mode,
            var_kwargs_validator,
            loc_by_alias: config.get_as("loc_by_alias")?.unwrap_or(true),
            extra: ExtraBehavior::from_schema_or_config(schema, config, ExtraBehavior::Forbid)?,
            validate_by_alias: schema_or_config_same(schema, config, "validate_by_alias")?,
            validate_by_name: schema_or_config_same(schema, config, "validate_by_name")?,
        }))))
    }
}

/// Collects what a collection yields.
struct Collect;

impl<T> ConsumeIterator<T> for Collect {
    type Output = Vec<T>;
    fn consume_iterator(self, iterator: impl Iterator<Item = T>) -> Vec<T> {
        iterator.collect()
    }
}

/// The value of the first lookup path of `paths` found in `kwargs`.
fn lookup<'a, K: ValidatedDict>(
    paths: &'a LookupPathCollection,
    lookup_type: LookupType,
    kwargs: &'a K,
) -> ValResult<Option<(&'a LookupPath, K::PathItem<'a>)>> {
    for path in paths.lookup_paths(lookup_type) {
        if let Some(value) = kwargs.get_item(path)? {
            return Ok(Some((path, value)));
        }
    }
    Ok(None)
}

impl Validator for ArgumentsValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        // this validator does not support partial validation yet
        state.allow_partial = false.into();

        let args = input.validate_args()?;

        let mut output_args: Vec<Value> = Vec::with_capacity(self.positional_params_count);
        let mut output_kwargs = Dict::new();
        let mut errors: Vec<ValLineError> = Vec::new();
        let mut used_kwargs: HashSet<&str> = HashSet::with_capacity(self.parameters.len());

        let validate_by_alias = state.validate_by_alias_or(self.validate_by_alias);
        let validate_by_name = state.validate_by_name_or(self.validate_by_name);
        let lookup_type = LookupType::from_bools(validate_by_alias, validate_by_name)?;

        let mut pos_args = Vec::new();
        let have_pos_args = args.args.is_some();
        if let Some(positional) = args.args {
            positional.try_for_each(|item| {
                pos_args.push(item?);
                Ok(())
            })?;
        }
        let kw_args = args.kwargs.as_ref();

        // go through the parameters, getting each value from args or kwargs and validating it
        for (index, parameter) in self.parameters.iter().enumerate() {
            let pos_value = if parameter.positional {
                pos_args.get(index)
            } else {
                None
            };
            let mut kw_value = None;
            if let (Some(kwargs), Some(paths)) = (kw_args, &parameter.lookup_path_collection)
                && let Some((lookup_path, value)) = lookup(paths, lookup_type, kwargs)?
            {
                used_kwargs.insert(lookup_path.first_key());
                kw_value = Some((lookup_path, value));
            }

            let state = &mut state.scoped_set_field_name(Some(parameter.shared_name.clone()));

            match (pos_value, kw_value) {
                (Some(_), Some((_, kw_value))) => {
                    errors.push(ValLineError::new_with_loc(
                        ErrorTypeDefaults::MultipleArgumentValues,
                        kw_value.borrow_input(),
                        parameter.name.clone(),
                    ));
                }
                (Some(pos_value), None) => {
                    match parameter
                        .validator
                        .validate(pos_value.borrow_input(), state)
                    {
                        Ok(value) => output_args.push(value),
                        Err(ValError::LineErrors(line_errors)) => {
                            errors.extend(
                                line_errors
                                    .into_iter()
                                    .map(|err| err.with_outer_location(index)),
                            );
                        }
                        Err(err) => return Err(err),
                    }
                }
                (None, Some((lookup_path, kw_value))) => {
                    match parameter.validator.validate(kw_value.borrow_input(), state) {
                        Ok(value) => {
                            output_kwargs.insert(Value::from(parameter.name.as_str()), value);
                        }
                        Err(ValError::LineErrors(line_errors)) => {
                            errors.extend(line_errors.into_iter().map(|err| {
                                lookup_path.apply_error_loc(err, self.loc_by_alias, &parameter.name)
                            }));
                        }
                        Err(err) => return Err(err),
                    }
                }
                (None, None) => {
                    if let Some(value) = parameter
                        .validator
                        .default_value(Some(parameter.name.as_str()), state)?
                    {
                        if parameter.keyword {
                            output_kwargs.insert(Value::from(parameter.name.as_str()), value);
                        } else {
                            output_args.push(value);
                        }
                    } else if let Some(paths) = &parameter.lookup_path_collection {
                        let error_type = if parameter.positional {
                            ErrorTypeDefaults::MissingArgument
                        } else {
                            ErrorTypeDefaults::MissingKeywordOnlyArgument
                        };
                        let error_loc = paths.error_loc(lookup_type, self.loc_by_alias);
                        errors.push(ValLineError::new_with_full_loc(
                            error_type, input, error_loc,
                        ));
                    } else {
                        errors.push(ValLineError::new_with_loc(
                            ErrorTypeDefaults::MissingPositionalOnlyArgument,
                            input,
                            index,
                        ));
                    }
                }
            }
        }

        // arguments beyond the positional parameters have not been looked at yet
        if have_pos_args && pos_args.len() > self.positional_params_count {
            if let Some(ref validator) = self.var_args_validator {
                for (index, item) in pos_args
                    .iter()
                    .enumerate()
                    .skip(self.positional_params_count)
                {
                    match validator.validate(item.borrow_input(), state) {
                        Ok(value) => output_args.push(value),
                        Err(ValError::LineErrors(line_errors)) => {
                            errors.extend(
                                line_errors
                                    .into_iter()
                                    .map(|err| err.with_outer_location(index)),
                            );
                        }
                        Err(err) => return Err(err),
                    }
                }
            } else {
                for (index, item) in pos_args
                    .iter()
                    .enumerate()
                    .skip(self.positional_params_count)
                {
                    errors.push(ValLineError::new_with_loc(
                        ErrorTypeDefaults::UnexpectedPositionalArgument,
                        item.borrow_input(),
                        index,
                    ));
                }
            }
        }

        let mut remaining_kwargs = Dict::new();

        // keyword arguments no parameter took
        if let Some(kwargs) = kw_args {
            for entry in kwargs.iterate(Collect)? {
                let (raw_key, value) = entry?;
                let key = match raw_key
                    .borrow_input()
                    .validate_str(true, false)
                    .map(crate::input::ValidationMatch::into_inner)
                {
                    Ok(key) => key.as_cow().into_owned(),
                    Err(ValError::LineErrors(line_errors)) => {
                        for err in line_errors {
                            errors.push(
                                err.with_outer_location(raw_key.clone())
                                    .with_type(ErrorTypeDefaults::InvalidKey),
                            );
                        }
                        continue;
                    }
                    Err(err) => return Err(err),
                };
                if used_kwargs.contains(key.as_str()) {
                    continue;
                }
                match self.var_kwargs_mode {
                    VarKwargsMode::Uniform => match &self.var_kwargs_validator {
                        Some(validator) => match validator.validate(value.borrow_input(), state) {
                            Ok(value) => {
                                output_kwargs.insert(Value::Str(key), value);
                            }
                            Err(ValError::LineErrors(line_errors)) => {
                                for err in line_errors {
                                    errors.push(err.with_outer_location(raw_key.clone()));
                                }
                            }
                            Err(err) => return Err(err),
                        },
                        None => {
                            if self.extra == ExtraBehavior::Forbid {
                                errors.push(ValLineError::new_with_loc(
                                    ErrorTypeDefaults::UnexpectedKeywordArgument,
                                    value.borrow_input(),
                                    raw_key.clone(),
                                ));
                            }
                        }
                    },
                    VarKwargsMode::UnpackedTypedDict => {
                        // validated below, as a single dict
                        remaining_kwargs.insert(Value::Str(key), value.borrow_input().to_value());
                    }
                }
            }
        }

        if self.var_kwargs_mode == VarKwargsMode::UnpackedTypedDict {
            let validator = self
                .var_kwargs_validator
                .as_ref()
                .expect("checked when built");
            match validator.validate(&Value::Dict(remaining_kwargs), state) {
                Ok(Value::Dict(validated)) => {
                    for (key, value) in validated {
                        output_kwargs.insert(key, value);
                    }
                }
                Ok(other) => {
                    return Err(ValError::InternalErr(CoreError::Type(format!(
                        "'{}' object is not a mapping",
                        other.type_name()
                    ))));
                }
                Err(ValError::LineErrors(line_errors)) => errors.extend(line_errors),
                Err(err) => return Err(err),
            }
        }

        if errors.is_empty() {
            Ok(Value::Tuple(vec![
                Value::Tuple(output_args),
                Value::Dict(output_kwargs),
            ]))
        } else {
            Err(ValError::LineErrors(errors))
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
