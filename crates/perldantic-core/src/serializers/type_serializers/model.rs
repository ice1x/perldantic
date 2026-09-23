//! `model-fields` and `model` serializers. Port of upstream `type_serializers/model.rs`.
//!
//! A model class is a name, so "an instance of the class" is a `Value::Model` of that class
//! (docs/DIVERGENCES.md #13).

use std::borrow::Cow;
use std::sync::Arc;

use crate::build_tools::{ExtraBehavior, SchemaDict, schema_err};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::{SerCheck, SerializationState};
use crate::serializers::fields::{FieldsMode, GeneralFieldsSerializer, SerField};
use crate::serializers::infer::{
    infer_json_key, infer_json_key_known, infer_serialize, infer_to_python,
};
use crate::serializers::ob_type::ObType;
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::validators::as_dict;
use crate::value::{Dict, Model, Value};

use super::any::AnySerializer;

const ROOT_FIELD: &str = "root";

pub struct ModelFieldsBuilder;

impl BuildSerializer for ModelFieldsBuilder {
    const EXPECTED_TYPE: &'static str = "model-fields";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let fields_mode = if has_extra(schema, config)? {
            FieldsMode::ModelExtra
        } else {
            FieldsMode::SimpleDict
        };

        let extra_serializer = match (schema.get_str("extras_schema"), &fields_mode) {
            (Some(v), FieldsMode::ModelExtra) => {
                Some(CombinedSerializer::build(as_dict(v)?, config, definitions)?)
            }
            (Some(_), _) => {
                return schema_err!("extras_schema can only be used if extra_behavior=allow");
            }
            (_, _) => None,
        };
        if schema
            .get_str("computed_fields")
            .is_some_and(|c| !matches!(c, Value::None) && c != &Value::List(vec![]))
        {
            return schema_err!(
                "`computed_fields` are not supported yet: host callbacks are not implemented"
            );
        }
        let serialize_by_alias: Option<bool> = config.get_as("serialize_by_alias")?;

        let fields_dict: Dict = schema.get_as_req("fields")?;
        let mut fields = Vec::with_capacity(fields_dict.len());
        for (key, value) in fields_dict.iter() {
            let Value::Str(key) = key else {
                return Err(CoreError::Type(format!(
                    "'{}' object cannot be converted to 'PyString'",
                    key.type_name()
                )));
            };
            let field_info = as_dict(value)?;
            if field_info.get_as::<bool>("serialization_exclude")? == Some(true) {
                fields.push(SerField::new(
                    key.clone(),
                    None,
                    None,
                    true,
                    serialize_by_alias,
                ));
            } else {
                if field_info
                    .get_str("serialization_exclude_if")
                    .is_some_and(|v| !matches!(v, Value::None))
                {
                    return schema_err!(
                        "`serialization_exclude_if` is not supported yet: host callbacks are not implemented"
                    );
                }
                let alias: Option<String> = field_info.get_as("serialization_alias")?;
                let schema: Dict = field_info.get_as_req("schema")?;
                let serializer =
                    CombinedSerializer::build(&schema, config, definitions).map_err(|e| {
                        CoreError::Schema(format!(
                            "Field `{key}`:\n  {}: {e}",
                            e.kind().python_name()
                        ))
                    })?;
                fields.push(SerField::new(
                    key.clone(),
                    alias,
                    Some(serializer),
                    true,
                    serialize_by_alias,
                ));
            }
        }

        Ok(Arc::new(
            GeneralFieldsSerializer::new(fields, fields_mode, extra_serializer).into(),
        ))
    }
}

#[derive(Debug)]
pub struct ModelSerializer {
    class: String,
    serializer: Arc<CombinedSerializer>,
    has_extra: bool,
    root_model: bool,
    name: String,
}

impl BuildSerializer for ModelSerializer {
    const EXPECTED_TYPE: &'static str = "model";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        // models ignore the parent config and always use the config from this model
        let config: Option<Dict> = schema.get_as("config")?;
        let class: String = schema.get_as_req("cls")?;
        let sub_schema: Dict = schema.get_as_req("schema")?;
        let serializer = CombinedSerializer::build(&sub_schema, config.as_ref(), definitions)?;
        let root_model = schema.get_as("root_model")?.unwrap_or(false);

        Ok(Arc::new(CombinedSerializer::Model(Self {
            name: class.clone(),
            class,
            serializer,
            has_extra: has_extra(schema, config.as_ref())?,
            root_model,
        })))
    }
}

fn has_extra(schema: &Dict, config: Option<&Dict>) -> CoreResult<bool> {
    let extra_behaviour =
        ExtraBehavior::from_schema_or_config(schema, config, ExtraBehavior::Ignore)?;
    Ok(matches!(extra_behaviour, ExtraBehavior::Allow))
}

impl ModelSerializer {
    fn is_instance<'a>(&self, value: &'a Value) -> Option<&'a Model> {
        match value {
            Value::Model(model) if model.class == self.class => Some(model),
            _ => None,
        }
    }

    fn allow_value(&self, value: &Value, check: SerCheck) -> bool {
        match check {
            SerCheck::Strict | SerCheck::Lax => self.is_instance(value).is_some(),
            // any object with fields
            SerCheck::None => matches!(value, Value::Model(_)),
        }
    }

    /// The value the fields serializer receives: the fields, plus the extra values when the
    /// model allows them.
    fn get_inner_value(&self, model: &Model) -> Value {
        let fields = Value::Dict(model.fields.clone());
        if self.has_extra {
            Value::Tuple(vec![
                fields,
                model.extra.clone().map_or(Value::None, Value::Dict),
            ])
        } else {
            fields
        }
    }

    /// Fields not set from input, dropped by `exclude_unset`.
    fn unset_fields(model: &Model) -> Vec<String> {
        model
            .fields
            .iter()
            .filter(|(k, _)| !model.fields_set.iter().any(|f| f == *k))
            .filter_map(|(k, _)| match k {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// The root value and serializer of a root model; `None` when the value is not an instance.
    fn root<'a>(
        &'a self,
        value: &'a Value,
        state: &SerializationState,
    ) -> Option<(&'a Value, &'a CombinedSerializer)> {
        let model = self.is_instance(value)?;
        let root = model.fields.get_str(ROOT_FIELD)?;
        // for root models, `serialize_as_any` may apply
        let serializer: &CombinedSerializer = if state.extra.serialize_as_any {
            AnySerializer::get()
        } else {
            &self.serializer
        };
        Some((root, serializer))
    }
}

impl TypeSerializer for ModelSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        if self.root_model {
            let Some((root, serializer)) = self.root(value, state) else {
                state.warn_fallback_py(self.get_name(), value)?;
                return infer_to_python(value, state);
            };
            let state = &mut state.scoped_set_field_name(Some(ROOT_FIELD.to_owned()));
            let state = &mut state.scoped_set(|s| &mut s.model, Some(value.clone()));
            return serializer.to_python_no_infer(root, state);
        }
        if !self.allow_value(value, state.check) {
            state.warn_fallback_py(self.get_name(), value)?;
            return infer_to_python(value, state);
        }
        let Value::Model(model) = value else {
            unreachable!("allowed values are models")
        };
        let inner_value = self.get_inner_value(model);
        let unset = state.extra.exclude_unset.then(|| Self::unset_fields(model));
        let state = &mut state.scoped_set(|s| &mut s.model, Some(value.clone()));
        let state = &mut state.scoped_set(|s| &mut s.unset_fields, unset);
        self.serializer.to_python_no_infer(&inner_value, state)
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        if self.allow_value(key, state.check) {
            infer_json_key_known(ObType::PydanticSerializable, key, state)
        } else {
            state.warn_fallback_py(&self.name, key)?;
            infer_json_key(key, state)
        }
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        if self.root_model {
            let Some((root, root_serializer)) = self.root(value, state) else {
                state.warn_fallback_ser::<S>(self.get_name(), value)?;
                return infer_serialize(value, serializer, state);
            };
            let state = &mut state.scoped_set_field_name(Some(ROOT_FIELD.to_owned()));
            let state = &mut state.scoped_set(|s| &mut s.model, Some(value.clone()));
            return root_serializer.serde_serialize_no_infer(root, serializer, state);
        }
        if !self.allow_value(value, state.check) {
            state.warn_fallback_ser::<S>(self.get_name(), value)?;
            return infer_serialize(value, serializer, state);
        }
        let Value::Model(model) = value else {
            unreachable!("allowed values are models")
        };
        let inner_value = self.get_inner_value(model);
        let unset = state.extra.exclude_unset.then(|| Self::unset_fields(model));
        let state = &mut state.scoped_set(|s| &mut s.model, Some(value.clone()));
        let state = &mut state.scoped_set(|s| &mut s.unset_fields, unset);
        self.serializer
            .serde_serialize_no_infer(&inner_value, serializer, state)
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn retry_with_lax_check(&self) -> bool {
        true
    }
}
