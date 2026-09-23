//! `model` schema. Port of upstream `validators/model.rs`.
//!
//! A model class is identified by its name (for Perl, the package the host blesses instances
//! into) and an instance is a `Value::Model`. "Is an instance of the class" means an instance
//! with the same class name; class hierarchies need the host (docs/DIVERGENCES.md #13).
//! `__init__` validation (`self_instance`), `post_init` and `custom_init` need host callbacks;
//! assignment validation comes with the Perl model API.

use std::sync::Arc;

use crate::build_tools::{SchemaDict, schema_err, schema_or_config_same};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::ValResult;
use crate::input::Input;
use crate::value::{Dict, Model, Value};

use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Validator, as_dict, build_validator};

const ROOT_FIELD: &str = "root";

#[derive(Debug, Clone)]
pub(super) enum Revalidate {
    Always,
    Never,
    SubclassInstances,
}

impl Revalidate {
    pub fn from_str(s: Option<&str>) -> CoreResult<Self> {
        match s {
            Some("always") => Ok(Self::Always),
            Some("never") | None => Ok(Self::Never),
            Some("subclass-instances") => Ok(Self::SubclassInstances),
            Some(s) => schema_err!("Invalid revalidate_instances value: {s}"),
        }
    }

    /// Instances are matched by exact class name, so none is a subclass instance.
    pub fn should_revalidate(&self) -> bool {
        match self {
            Revalidate::Always => true,
            Revalidate::Never | Revalidate::SubclassInstances => false,
        }
    }
}

#[derive(Debug)]
pub struct ModelValidator {
    revalidate: Revalidate,
    validator: Arc<CombinedValidator>,
    class: String,
    generic_origin: Option<String>,
    root_model: bool,
    name: String,
}

impl BuildValidator for ModelValidator {
    const EXPECTED_TYPE: &'static str = "model";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        // models ignore the parent config and always use the config from this model
        let config: Option<Dict> = schema.get_as("config")?;
        let class: String = schema.get_as_req("cls")?;
        let generic_origin: Option<String> = schema.get_as("generic_origin")?;

        for hook in ["post_init", "custom_init"] {
            if schema
                .get_str(hook)
                .is_some_and(|v| !matches!(v, Value::None | Value::Bool(false)))
            {
                return schema_err!(
                    "`{hook}` is not supported yet: host callbacks are not implemented"
                );
            }
        }

        let sub_schema = as_dict(
            schema
                .get_str("schema")
                .ok_or_else(|| crate::core_error::CoreError::Key("schema".into()))?,
        )?;
        let validator = build_validator(sub_schema, config.as_ref(), definitions)?;
        let revalidate = Revalidate::from_str(
            schema_or_config_same::<String>(schema, config.as_ref(), "revalidate_instances")?
                .as_deref(),
        )?;

        Ok(Arc::new(CombinedValidator::Model(Self {
            revalidate,
            validator,
            name: class.clone(),
            class,
            generic_origin,
            root_model: schema.get_as("root_model")?.unwrap_or(false),
        })))
    }
}

impl Validator for ModelValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        // if the input is an instance of the class, we "revalidate" it - e.g. we extract and
        // reuse `fields_set` but validate its data to create a new instance
        let instance = match input.as_value() {
            Some(Value::Model(model)) if model.class == self.class => Some((model, false)),
            // if the model has a generic origin, instances of the origin are accepted but
            // always revalidated
            Some(Value::Model(model)) if self.generic_origin.as_ref() == Some(&model.class) => {
                Some((model, true))
            }
            _ => None,
        };

        if let Some((model, force_revalidate)) = instance {
            if self.revalidate.should_revalidate() || force_revalidate {
                let fields_set = Some(model.fields_set.clone());
                if self.root_model {
                    let inner_input = model.fields.get_str(ROOT_FIELD).ok_or_else(|| {
                        crate::core_error::CoreError::Internal(format!(
                            "'{}' object has no attribute '{ROOT_FIELD}'",
                            model.class
                        ))
                    })?;
                    self.validate_construct(inner_input, fields_set, state)
                } else {
                    let mut inner_input = model.fields.clone();
                    for (k, v) in model.extra.iter().flat_map(Dict::iter) {
                        inner_input.insert(k.clone(), v.clone());
                    }
                    self.validate_construct(&Value::Dict(inner_input), fields_set, state)
                }
            } else {
                Ok(input.to_value())
            }
        } else {
            // Having to construct a new model is not an exact match
            state.floor_exactness(Exactness::Strict);
            self.validate_construct(input, None, state)
        }
    }

    fn get_name(&self) -> &str {
        &self.name
    }
}

impl ModelValidator {
    fn validate_construct(
        &self,
        input: &(impl Input + ?Sized),
        existing_fields_set: Option<Vec<Value>>,
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let model = if self.root_model {
            let state = &mut state.scoped_set_field_name(Some(ROOT_FIELD.to_owned()));
            let output = self.validator.validate(input, state)?;
            let mut fields = Dict::new();
            fields.insert(Value::from(ROOT_FIELD), output);
            Model {
                class: self.class.clone(),
                fields,
                fields_set: vec![Value::from(ROOT_FIELD)],
                extra: None,
            }
        } else {
            let output = self.validator.validate(input, state)?;
            let (fields, extra, val_fields_set) = split_model_fields(output)?;
            Model {
                class: self.class.clone(),
                fields,
                fields_set: existing_fields_set.unwrap_or(val_fields_set),
                extra,
            }
        };
        Ok(Value::Model(Box::new(model)))
    }
}

/// Unpack the `(fields, extra, fields_set)` tuple a `model-fields` schema returns.
fn split_model_fields(output: Value) -> CoreResult<(Dict, Option<Dict>, Vec<Value>)> {
    let Value::Tuple(items) = output else {
        return Err(crate::core_error::CoreError::Type(format!(
            "'{}' object is not an instance of 'tuple'",
            output.type_name()
        )));
    };
    match <[Value; 3]>::try_from(items) {
        Ok([Value::Dict(fields), extra, Value::Set(fields_set)]) => {
            let extra = match extra {
                Value::Dict(extra) => Some(extra),
                _ => None,
            };
            Ok((fields, extra, fields_set))
        }
        _ => Err(crate::core_error::CoreError::Type(
            "a model schema must produce (dict, extra, set)".into(),
        )),
    }
}
