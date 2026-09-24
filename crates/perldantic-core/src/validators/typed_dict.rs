//! `typed-dict` schema. Port of upstream `validators/typed_dict.rs`: a dict with named,
//! individually validated keys; the output is a dict.

use std::collections::HashSet;
use std::sync::Arc;

use jiter::PartialMode;

use crate::build_tools::{ExtraBehavior, SchemaDict, is_strict, schema_err, schema_or_config};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorTypeDefaults, LocItem, ValError, ValLineError, ValResult};
use crate::input::{BorrowInput, ConsumeIterator, Input, ValidatedDict, ValidationMatch};
use crate::lookup_key::{LookupPathCollection, LookupType};
use crate::value::{Dict, Value};

use super::validation_state::ValidationState;
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

#[derive(Debug)]
struct TypedDictField {
    name: String,
    lookup_path_collection: LookupPathCollection,
    required: bool,
    validator: Arc<CombinedValidator>,
}

#[derive(Debug)]
pub struct TypedDictValidator {
    fields: Vec<TypedDictField>,
    extra_behavior: ExtraBehavior,
    extras_validator: Option<Arc<CombinedValidator>>,
    strict: bool,
    loc_by_alias: bool,
    validate_by_alias: Option<bool>,
    validate_by_name: Option<bool>,
    cls_name: Option<String>,
}

impl BuildValidator for TypedDictValidator {
    const EXPECTED_TYPE: &'static str = "typed-dict";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        // typed dicts ignore the parent config and always use their own
        let config: Option<Dict> = schema.get_as("config")?;
        let config = config.as_ref();
        let strict = is_strict(schema, config)?;
        let total = schema_or_config(schema, config, "total", "typed_dict_total")?.unwrap_or(true);
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
        // `cls` is a class name here (docs/DIVERGENCES.md #13)
        let cls_name: Option<String> = match schema.get_as("cls_name")? {
            Some(name) => Some(name),
            None => schema.get_as("cls")?,
        };

        let fields_dict: Dict = schema.get_as_req("fields")?;
        let mut fields = Vec::with_capacity(fields_dict.len());
        for (key, value) in fields_dict.iter() {
            let field_info = as_dict(value)?;
            let Value::Str(name) = key else {
                return Err(CoreError::Type(format!(
                    "'{}' object cannot be converted to 'PyString'",
                    key.type_name()
                )));
            };
            let field_schema: Dict = field_info.get_as_req("schema")?;
            let validator = match build_validator(&field_schema, config, definitions) {
                Ok(v) => v,
                Err(err) => return schema_err!("Field \"{name}\":\n  {err}"),
            };
            let has_default = |v: &CombinedValidator| match v {
                CombinedValidator::WithDefault(v) => v.has_default(),
                _ => false,
            };
            let omit_on_error = |v: &CombinedValidator| match v {
                CombinedValidator::WithDefault(v) => v.omit_on_error(),
                _ => false,
            };
            let required = match field_info.get_as::<bool>("required")? {
                Some(true) if has_default(&validator) => {
                    return schema_err!(
                        "Field '{name}': a required field cannot have a default value"
                    );
                }
                Some(required) => required,
                None => total,
            };
            if required && omit_on_error(&validator) {
                return schema_err!(
                    "Field '{name}': 'on_error = omit' cannot be set for required fields"
                );
            }
            let lookup_path_collection =
                LookupPathCollection::new(field_info.get_str("validation_alias"), name)?;
            fields.push(TypedDictField {
                name: name.clone(),
                lookup_path_collection,
                required,
                validator,
            });
        }

        Ok(Arc::new(CombinedValidator::TypedDict(Box::new(Self {
            fields,
            extra_behavior,
            extras_validator,
            strict,
            loc_by_alias: config.get_as("loc_by_alias")?.unwrap_or(true),
            validate_by_alias: config.get_as("validate_by_alias")?,
            validate_by_name: config.get_as("validate_by_name")?,
            cls_name,
        }))))
    }
}

/// Record a validated item in the output dict, which is also the `data` validators see.
fn set_item(state: &mut ValidationState<'_>, name: &str, value: Value) {
    if let Some(data) = state.data.as_mut() {
        data.insert(Value::from(name), value);
    }
}

impl Validator for TypedDictValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let strict = state.strict_or(self.strict);
        let extra_behavior = state.extra_behavior_or(self.extra_behavior);
        let dict = input.validate_dict(strict)?;

        let mut errors: Vec<ValLineError> = Vec::with_capacity(self.fields.len());
        let partial_last_key: Option<LocItem> = if state.allow_partial.is_active() {
            dict.last_key().map(Into::into)
        } else {
            None
        };
        let allow_partial = state.allow_partial;

        let validate_by_alias = state.validate_by_alias_or(self.validate_by_alias);
        let validate_by_name = state.validate_by_name_or(self.validate_by_name);
        let lookup_type = LookupType::from_bools(validate_by_alias, validate_by_name)?;

        // which keys were used only matters when the input is iterated for extras afterwards
        let mut used_keys: Option<HashSet<&str>> = if extra_behavior == ExtraBehavior::Ignore {
            None
        } else {
            Some(HashSet::with_capacity(self.fields.len()))
        };

        let mut output = {
            let state = &mut state.scoped_set_data(Some(Dict::new()));
            let state = &mut state.scoped_clear_field_error();
            let mut fields_set_count: usize = 0;

            for field in &self.fields {
                if let Some((lookup_path, lookup_result)) = field
                    .lookup_path_collection
                    .lookup_paths(lookup_type)
                    .find_map(|path| Some((path, dict.get_item(path).transpose()?)))
                {
                    let value = match lookup_result {
                        Ok(v) => v,
                        Err(ValError::LineErrors(line_errors)) => {
                            let field_loc = LocItem::from(field.name.clone());
                            if partial_last_key.as_ref() == Some(&field_loc) {
                                for err in line_errors {
                                    errors.push(err.with_outer_location(field_loc.clone()));
                                }
                            }
                            continue;
                        }
                        Err(err) => return Err(err),
                    };
                    if let Some(ref mut used_keys) = used_keys {
                        // a key is "used" whether or not validation passes
                        used_keys.insert(lookup_path.first_key());
                    }
                    let is_last_partial = partial_last_key
                        .as_ref()
                        .is_some_and(|last| *last == LocItem::from(lookup_path.first_key()));
                    state.allow_partial = if is_last_partial {
                        allow_partial
                    } else {
                        PartialMode::Off
                    };
                    let state = &mut state.scoped_set_field_name(Some(field.name.clone()));
                    match field.validator.validate(value.borrow_input(), state) {
                        Ok(value) => {
                            set_item(state, &field.name, value);
                            fields_set_count += 1;
                        }
                        Err(e) => {
                            state.has_field_error = true;
                            match e {
                                ValError::Omit => {}
                                ValError::LineErrors(line_errors) => {
                                    if !is_last_partial || field.required {
                                        for err in line_errors {
                                            errors.push(lookup_path.apply_error_loc(
                                                err,
                                                self.loc_by_alias,
                                                &field.name,
                                            ));
                                        }
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
                    Ok(Some(value)) => set_item(state, &field.name, value),
                    Ok(None) => {
                        state.has_field_error = true;
                        if field.required {
                            let error_loc = field
                                .lookup_path_collection
                                .error_loc(lookup_type, self.loc_by_alias);
                            errors.push(ValLineError::new_with_full_loc(
                                ErrorTypeDefaults::Missing,
                                input,
                                error_loc,
                            ));
                        }
                    }
                    Err(ValError::Omit) => {}
                    Err(ValError::LineErrors(line_errors)) => {
                        state.has_field_error = true;
                        // the default's own errors, located by field name
                        errors.extend(line_errors);
                    }
                    Err(err) => return Err(err),
                }
            }
            state.add_fields_set(fields_set_count);
            state.data.take().unwrap_or_default()
        };

        if let Some(used_keys) = used_keys {
            dict.iterate(ValidateExtras {
                used_keys,
                errors: &mut errors,
                extras_validator: self.extras_validator.as_deref(),
                output: &mut output,
                state,
                extra_behavior,
                partial_last_key,
                allow_partial,
            })??;
        }

        if errors.is_empty() {
            Ok(Value::Dict(output))
        } else {
            Err(ValError::LineErrors(errors))
        }
    }

    fn get_name(&self) -> &str {
        self.cls_name.as_deref().unwrap_or(Self::EXPECTED_TYPE)
    }
}

struct ValidateExtras<'a, 's> {
    used_keys: HashSet<&'a str>,
    errors: &'a mut Vec<ValLineError>,
    extras_validator: Option<&'a CombinedValidator>,
    output: &'a mut Dict,
    state: &'a mut ValidationState<'s>,
    extra_behavior: ExtraBehavior,
    partial_last_key: Option<LocItem>,
    allow_partial: PartialMode,
}

impl<Key, Item> ConsumeIterator<ValResult<(Key, Item)>> for ValidateExtras<'_, '_>
where
    Key: BorrowInput + Clone + Into<LocItem>,
    Item: BorrowInput,
{
    type Output = ValResult<()>;
    fn consume_iterator(
        self,
        iterator: impl Iterator<Item = ValResult<(Key, Item)>>,
    ) -> ValResult<()> {
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
            let key = either_str.as_cow();
            if self.used_keys.contains(key.as_ref()) {
                continue;
            }
            let value = value.borrow_input();
            // an unknown key
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
                    if let Some(validator) = self.extras_validator {
                        let last_partial =
                            self.partial_last_key.as_ref() == Some(&raw_key.clone().into());
                        self.state.allow_partial = if last_partial {
                            self.allow_partial
                        } else {
                            PartialMode::Off
                        };
                        match validator.validate(value, self.state) {
                            Ok(value) => {
                                self.output.insert(Value::from(key.as_ref()), value);
                            }
                            Err(ValError::LineErrors(line_errors)) => {
                                if !last_partial {
                                    for err in line_errors {
                                        self.errors.push(err.with_outer_location(raw_key.clone()));
                                    }
                                }
                            }
                            Err(err) => return Err(err),
                        }
                    } else {
                        self.output
                            .insert(Value::from(key.as_ref()), value.to_value());
                    }
                }
            }
        }
        Ok(())
    }
}
