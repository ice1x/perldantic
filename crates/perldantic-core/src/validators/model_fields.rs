//! `model-fields` schema. Port of upstream `validators/model_fields.rs`.
//!
//! The output is the tuple `(fields, extra, fields_set)`: a dict of field values, a dict of
//! extra values (or `None` unless `extra_behavior='allow'`), and the set of names that came
//! from the input. Assignment validation comes with the Perl model API.

use std::collections::HashSet;
use std::sync::Arc;

use jiter::{JsonObject, JsonValue};

use crate::build_tools::{ExtraBehavior, SchemaDict, is_strict, schema_err, schema_or_config_same};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, LocItem, ValError, ValLineError, ValResult};
use crate::input::{BorrowInput, ConsumeIterator, Input, ValidatedDict, ValidationMatch};
use crate::lookup_key::{LookupPathCollection, LookupType};
use crate::value::{Dict, Value};

use super::shared::lookup_tree::{LookupFieldInfo, LookupTree};
use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
struct Field {
    name: String,
    /// The name as the validation state holds it (`info.field_name`), shared, not copied.
    shared_name: Arc<str>,
    lookup_path_collection: LookupPathCollection,
    validator: Arc<CombinedValidator>,
}

#[derive(Debug)]
pub struct ModelFieldsValidator {
    fields: Vec<Field>,
    model_name: String,
    extra_behavior: ExtraBehavior,
    extras_validator: Option<Arc<CombinedValidator>>,
    extras_keys_validator: Option<Arc<CombinedValidator>>,
    strict: bool,
    from_attributes: bool,
    loc_by_alias: bool,
    lookup: LookupTree,
    validate_by_alias: Option<bool>,
    validate_by_name: Option<bool>,
}

impl BuildValidator for ModelFieldsValidator {
    const EXPECTED_TYPE: &'static str = "model-fields";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let strict = is_strict(schema, config)?;

        let from_attributes =
            schema_or_config_same(schema, config, "from_attributes")?.unwrap_or(false);

        let extra_behavior =
            ExtraBehavior::from_schema_or_config(schema, config, ExtraBehavior::Ignore)?;

        let extras_validator = match (schema.get_str("extras_schema"), &extra_behavior) {
            (Some(v), ExtraBehavior::Allow) => {
                Some(build_validator(as_dict(v)?, config, definitions)?)
            }
            (Some(_), _) => {
                return schema_err!("extras_schema can only be used if extra_behavior=allow");
            }
            (_, _) => None,
        };
        let extras_keys_validator = match (schema.get_str("extras_keys_schema"), &extra_behavior) {
            (Some(v), ExtraBehavior::Allow) => {
                Some(build_validator(as_dict(v)?, config, definitions)?)
            }
            (Some(_), _) => {
                return schema_err!("extras_keys_schema can only be used if extra_behavior=allow");
            }
            (_, _) => None,
        };
        let model_name: String = schema
            .get_as("model_name")?
            .unwrap_or_else(|| "Model".to_string());

        let fields_dict: Dict = schema.get_as_req("fields")?;
        let mut fields: Vec<Field> = Vec::with_capacity(fields_dict.len());

        for (key, value) in fields_dict.iter() {
            let field_info = as_dict(value)?;
            let Value::Str(name) = key else {
                return Err(crate::core_error::CoreError::Type(format!(
                    "'{}' object cannot be converted to 'PyString'",
                    key.type_name()
                )));
            };

            let schema: Dict = field_info.get_as_req("schema")?;

            let validator = match build_validator(&schema, config, definitions) {
                Ok(v) => v,
                Err(err) => {
                    return schema_err!("Field \"{name}\":\n  {}: {err}", err.kind().python_name());
                }
            };

            let lookup_path_collection =
                LookupPathCollection::new(field_info.get_str("validation_alias"), name)?;

            fields.push(Field {
                name: name.clone(),
                shared_name: Arc::from(name.as_str()),
                lookup_path_collection,
                validator,
            });
        }

        let lookup = LookupTree::from_fields(&fields, |field| &field.lookup_path_collection);

        Ok(Arc::new(CombinedValidator::ModelFields(Self {
            fields,
            model_name,
            extra_behavior,
            extras_validator,
            extras_keys_validator,
            strict,
            from_attributes,
            loc_by_alias: config.get_as("loc_by_alias")?.unwrap_or(true),
            lookup,
            validate_by_alias: config.get_as("validate_by_alias")?,
            validate_by_name: config.get_as("validate_by_name")?,
        })))
    }
}

impl Validator for ModelFieldsValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        // this validator does not yet support partial validation, disable it to avoid incorrect
        // results
        state.allow_partial = false.into();

        let strict = state.strict_or(self.strict);
        let extra_behavior = state.extra_behavior_or(self.extra_behavior);
        let from_attributes = state
            .extra()
            .from_attributes
            .unwrap_or(self.from_attributes);

        let (model_dict, mut model_extra_dict_op, fields_set) =
            if let Some(json_input) = input.as_json() {
                let JsonValue::Object(json_object) = json_input else {
                    return Err(ValError::new(
                        ErrorType::ModelType {
                            context: None,
                            class_name: self.model_name.clone(),
                        },
                        input,
                    ));
                };
                self.validate_json_by_iteration(json_input, json_object, state)?
            } else {
                // we convert the DictType error to a ModelType error
                let dict = match input.validate_model_fields(strict, from_attributes) {
                    Ok(d) => d,
                    Err(ValError::LineErrors(errors)) => {
                        let errors: Vec<ValLineError> = errors
                            .into_iter()
                            .map(|e| match e.error_type {
                                ErrorType::DictType { .. } => e.with_type(ErrorType::ModelType {
                                    class_name: self.model_name.clone(),
                                    context: None,
                                }),
                                _ => e,
                            })
                            .collect();
                        return Err(ValError::LineErrors(errors));
                    }
                    Err(err) => return Err(err),
                };
                self.validate_by_get_item(input, dict, state)?
            };

        // if we have extra=allow, but we didn't create a dict because we were validating
        // from attributes, set it now so the extra dict is always a dict if extra=allow
        if matches!(extra_behavior, ExtraBehavior::Allow) && model_extra_dict_op.is_none() {
            model_extra_dict_op = Some(Dict::new());
        }

        Ok(Value::Tuple(vec![
            Value::Dict(model_dict),
            model_extra_dict_op.map_or(Value::None, Value::Dict),
            Value::Set(fields_set),
        ]))
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

type ValidatedModelFields = (Dict, Option<Dict>, Vec<Value>);

/// Record a validated field in the model dict, which is also the `data` validators see.
fn set_field(state: &mut ValidationState<'_>, name: &str, value: Value) {
    if let Some(data) = state.data.as_mut() {
        // each field is set once
        data.push_new(Value::from(name), value);
    }
}

impl ModelFieldsValidator {
    fn validate_by_get_item<D: ValidatedDict>(
        &self,
        input: &(impl Input + ?Sized),
        dict: D,
        state: &mut ValidationState<'_>,
    ) -> ValResult<ValidatedModelFields> {
        let extra_behavior = state.extra_behavior_or(self.extra_behavior);
        let mut errors: Vec<ValLineError> = Vec::with_capacity(self.fields.len());
        let mut fields_set_vec: Vec<Value> = Vec::with_capacity(self.fields.len());
        let mut fields_set_count: usize = 0;

        let validate_by_alias = state.validate_by_alias_or(self.validate_by_alias);
        let validate_by_name = state.validate_by_name_or(self.validate_by_name);
        let lookup_type = LookupType::from_bools(validate_by_alias, validate_by_name)?;

        // we only care about which keys have been used if we're iterating over the object for
        // extra after the first pass
        let mut used_keys: Option<HashSet<&str>> = if extra_behavior == ExtraBehavior::Ignore {
            None
        } else {
            Some(HashSet::with_capacity(self.fields.len()))
        };

        let model_dict = {
            let state = &mut state.scoped_set_data(Some(Dict::new()));
            let state = &mut state.scoped_clear_field_error();

            for field in &self.fields {
                let state = &mut state.scoped_set_field_name(Some(field.shared_name.clone()));

                if let Some((lookup_path, lookup_result)) = field
                    .lookup_path_collection
                    .lookup_paths(lookup_type)
                    .find_map(|path| Some((path, dict.get_item(path).transpose()?)))
                {
                    let value = match lookup_result {
                        Ok(value) => value,
                        Err(ValError::LineErrors(line_errors)) => {
                            for err in line_errors {
                                errors.push(err.with_outer_location(field.name.clone()));
                            }
                            continue;
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    };

                    if let Some(ref mut used_keys) = used_keys {
                        // key is "used" whether or not validation passes, since we want to skip
                        // this key in extra logic either way
                        used_keys.insert(lookup_path.first_key());
                    }

                    match field.validator.validate(value.borrow_input(), state) {
                        Ok(value) => {
                            set_field(state, &field.name, value);
                            fields_set_vec.push(Value::from(field.name.as_str()));
                            fields_set_count += 1;
                        }
                        Err(e) => {
                            state.has_field_error = true;
                            match e {
                                ValError::Omit => {}
                                ValError::LineErrors(line_errors) => {
                                    for err in line_errors {
                                        errors.push(lookup_path.apply_error_loc(
                                            err,
                                            self.loc_by_alias,
                                            &field.name,
                                        ));
                                    }
                                }
                                err => return Err(err),
                            }
                        }
                    }
                    continue;
                }

                match field
                    .validator
                    .default_value(Some(field.name.as_str()), state)
                {
                    Ok(Some(value)) => {
                        // Default value exists, and passed validation if required
                        set_field(state, &field.name, value);
                    }
                    Ok(None) => {
                        // There was no default value
                        state.has_field_error = true;
                        let error_type = ErrorTypeDefaults::Missing;
                        let error_loc = field
                            .lookup_path_collection
                            .error_loc(lookup_type, self.loc_by_alias);
                        errors.push(ValLineError::new_with_full_loc(
                            error_type, input, error_loc,
                        ));
                    }
                    Err(ValError::Omit) => {}
                    Err(ValError::LineErrors(line_errors)) => {
                        state.has_field_error = true;
                        // Note: this will always use the field name even if there is an alias
                        // However, we don't mind so much because this error can only happen if
                        // the default value fails validation, which is arguably a developer
                        // error.
                        errors.extend(line_errors);
                    }
                    Err(err) => return Err(err),
                }
            }
            state.data.take().unwrap_or_default()
        };

        let mut model_extra_dict_op = None;
        if let Some(used_keys) = used_keys {
            let model_extra_dict = dict.iterate(ValidateToModelExtra {
                used_keys,
                errors: &mut errors,
                fields_set_vec: &mut fields_set_vec,
                extra_behavior,
                extras_validator: self.extras_validator.as_deref(),
                extras_keys_validator: self.extras_keys_validator.as_deref(),
                state,
            })??;

            if matches!(extra_behavior, ExtraBehavior::Allow) {
                model_extra_dict_op = Some(model_extra_dict);
            }
        }

        if errors.is_empty() {
            state.add_fields_set(fields_set_count);
            Ok((model_dict, model_extra_dict_op, fields_set_vec))
        } else {
            Err(ValError::LineErrors(errors))
        }
    }

    fn validate_json_by_iteration(
        &self,
        json_input: &JsonValue<'_>,
        json_object: &JsonObject<'_>,
        state: &mut ValidationState<'_>,
    ) -> ValResult<ValidatedModelFields> {
        let extra_behavior = state.extra_behavior_or(self.extra_behavior);
        let validate_by_alias = state.validate_by_alias_or(self.validate_by_alias);
        let validate_by_name = state.validate_by_name_or(self.validate_by_name);
        let lookup_type = LookupType::from_bools(validate_by_alias, validate_by_name)?;

        let mut field_results: Vec<Option<(LookupFieldInfo, &JsonValue)>> =
            (0..self.fields.len()).map(|_| None).collect();
        let mut errors: Vec<ValLineError> = Vec::new();
        let mut fields_set: Vec<Value> = Vec::new();
        let mut fields_set_count: usize = 0;

        let state = &mut state.scoped_set_data(Some(Dict::new()));
        let state = &mut state.scoped_clear_field_error();

        let mut model_extra_dict = Dict::new();
        for (key, value) in json_object.iter() {
            let mut handled = false;
            let key = key.as_ref();
            for (field_info, field_value) in self.lookup.iter_matches(key, value) {
                handled = true;

                if !field_info.matches_lookup(lookup_type) {
                    continue;
                }

                let field_result = &mut field_results[field_info.field_index];

                // later results are preferred unless the existing result has come from a higher
                // priority alias
                if let Some((existing_field_info, _)) = &field_result
                    && existing_field_info
                        .lookup_priority
                        .is_higher_priority_than(&field_info.lookup_priority)
                {
                    continue;
                }

                *field_result = Some((*field_info, field_value));
            }

            if handled {
                continue;
            }

            // Unknown / extra field - we currently only care about these at the top level
            match extra_behavior {
                ExtraBehavior::Forbid => {
                    errors.push(ValLineError::new_with_loc(
                        ErrorTypeDefaults::ExtraForbidden,
                        value,
                        key,
                    ));
                }
                ExtraBehavior::Ignore => {}
                ExtraBehavior::Allow => {
                    if let Some(validator) = &self.extras_validator {
                        match validator.validate(value, state) {
                            Ok(value) => {
                                model_extra_dict.insert(Value::from(key), value);
                                add_to_set(&mut fields_set, key);
                            }
                            Err(ValError::LineErrors(line_errors)) => {
                                for err in line_errors {
                                    errors.push(err.with_outer_location(key));
                                }
                            }
                            Err(err) => return Err(err),
                        }
                    } else {
                        model_extra_dict.insert(Value::from(key), value.to_value());
                        add_to_set(&mut fields_set, key);
                    }
                }
            }
        }

        // now that we've iterated over all the keys, we can set the values in the model
        // dict, and try to set defaults for any missing fields

        for (field, field_result) in std::iter::zip(&self.fields, field_results) {
            let state = &mut state.scoped_set_field_name(Some(field.shared_name.clone()));

            let field_value =
                if let Some((field_info, field_json_value)) = field_result {
                    match field.validator.validate(field_json_value, state) {
                        Ok(value) => {
                            add_to_set(&mut fields_set, &field.name);
                            fields_set_count += 1;
                            value
                        }
                        Err(ValError::Omit) => continue,
                        Err(ValError::LineErrors(line_errors)) => {
                            state.has_field_error = true;
                            // for line errors, apply the actual lookup path used
                            errors.extend(line_errors.into_iter().map(|err| {
                                if self.loc_by_alias
                                    && let Some(alias_index) = field_info.alias_index()
                                {
                                    field.lookup_path_collection.by_alias[alias_index]
                                        .apply_error_loc(err, self.loc_by_alias, &field.name)
                                } else {
                                    err.with_outer_location(field.name.clone())
                                }
                            }));
                            continue;
                        }
                        Err(err) => return Err(err),
                    }
                } else {
                    match field
                        .validator
                        .default_value(Some(field.name.as_str()), state)
                    {
                        Ok(Some(default_value)) => default_value,
                        Ok(None) => {
                            // There was no default value
                            let error_type = ErrorTypeDefaults::Missing;
                            let error_loc = field
                                .lookup_path_collection
                                .error_loc(lookup_type, self.loc_by_alias);
                            errors.push(ValLineError::new_with_full_loc(
                                error_type, json_input, error_loc,
                            ));
                            continue;
                        }
                        Err(ValError::Omit) => continue,
                        Err(ValError::LineErrors(line_errors)) => {
                            state.has_field_error = true;
                            // Note: this will always use the field name even if there is an alias
                            errors.extend(line_errors);
                            continue;
                        }
                        Err(err) => return Err(err),
                    }
                };

            set_field(state, &field.name, field_value);
        }

        let model_extra_dict_op =
            matches!(extra_behavior, ExtraBehavior::Allow).then_some(model_extra_dict);

        if !errors.is_empty() {
            return Err(ValError::LineErrors(errors));
        }

        state.add_fields_set(fields_set_count);
        let model_dict = state.data.take().unwrap_or_default();
        Ok((model_dict, model_extra_dict_op, fields_set))
    }
}

/// Add a name to the fields set, keeping it free of duplicates like a Python set.
fn add_to_set(fields_set: &mut Vec<Value>, name: &str) {
    if !fields_set
        .iter()
        .any(|v| matches!(v, Value::Str(s) if s == name))
    {
        fields_set.push(Value::from(name));
    }
}

struct ValidateToModelExtra<'a, 's> {
    used_keys: HashSet<&'a str>,
    errors: &'a mut Vec<ValLineError>,
    fields_set_vec: &'a mut Vec<Value>,
    extra_behavior: ExtraBehavior,
    extras_validator: Option<&'a CombinedValidator>,
    extras_keys_validator: Option<&'a CombinedValidator>,
    state: &'a mut ValidationState<'s>,
}

impl<Key, Item> ConsumeIterator<ValResult<(Key, Item)>> for ValidateToModelExtra<'_, '_>
where
    Key: BorrowInput + Clone + Into<LocItem>,
    Item: BorrowInput,
{
    type Output = ValResult<Dict>;
    fn consume_iterator(
        self,
        iterator: impl Iterator<Item = ValResult<(Key, Item)>>,
    ) -> Self::Output {
        let mut model_extra_dict = Dict::new();
        for item_result in iterator {
            let (raw_key, value) = item_result?;
            let either_str = match raw_key
                .borrow_input()
                .validate_str(true, false)
                .map(ValidationMatch::into_inner)
            {
                Ok(k) => k,
                Err(ValError::LineErrors(line_errors)) => {
                    for err in line_errors {
                        self.errors.push(
                            err.with_outer_location(raw_key.clone())
                                .with_type(ErrorTypeDefaults::InvalidKey),
                        );
                    }
                    continue;
                }
                Err(err) => return Err(err),
            };
            let cow = either_str.as_cow();
            if self.used_keys.contains(cow.as_ref()) {
                continue;
            }

            let value = value.borrow_input();
            // Unknown / extra field
            match self.extra_behavior {
                ExtraBehavior::Forbid => {
                    self.errors.push(ValLineError::new_with_loc(
                        ErrorTypeDefaults::ExtraForbidden,
                        value,
                        raw_key.clone(),
                    ));
                }
                ExtraBehavior::Ignore => {}
                ExtraBehavior::Allow => {
                    let key = match self.extras_keys_validator {
                        Some(validator) => {
                            match validator.validate(raw_key.borrow_input(), self.state) {
                                Ok(Value::Str(key)) => key,
                                Ok(other) => {
                                    return Err(crate::core_error::CoreError::Type(format!(
                                        "'{}' object cannot be converted to 'PyString'",
                                        other.type_name()
                                    ))
                                    .into());
                                }
                                Err(ValError::LineErrors(line_errors)) => {
                                    for err in line_errors {
                                        self.errors.push(err.with_outer_location(raw_key.clone()));
                                    }
                                    continue;
                                }
                                Err(err) => return Err(err),
                            }
                        }
                        None => cow.into_owned(),
                    };

                    if let Some(validator) = self.extras_validator {
                        match validator.validate(value, self.state) {
                            Ok(value) => {
                                model_extra_dict.insert(Value::from(key.as_str()), value);
                                add_to_set(self.fields_set_vec, &key);
                            }
                            Err(ValError::LineErrors(line_errors)) => {
                                for err in line_errors {
                                    self.errors.push(err.with_outer_location(raw_key.clone()));
                                }
                            }
                            Err(err) => return Err(err),
                        }
                    } else {
                        model_extra_dict.insert(Value::from(key.as_str()), value.to_value());
                        add_to_set(self.fields_set_vec, &key);
                    }
                }
            }
        }
        Ok(model_extra_dict)
    }
}
