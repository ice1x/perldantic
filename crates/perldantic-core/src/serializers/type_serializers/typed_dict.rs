//! `typed-dict` serializer. Port of upstream `type_serializers/typed_dict.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use crate::build_tools::{ExtraBehavior, SchemaDict, schema_err, schema_or_config};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::serializers::errors::SerResult;
use crate::serializers::extra::SerializationState;
use crate::serializers::fields::{FieldsMode, GeneralFieldsSerializer, SerField};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::validators::as_dict;
use crate::value::{Dict, Value};

#[derive(Debug)]
pub struct TypedDictSerializer {
    serializer: GeneralFieldsSerializer,
}

impl BuildSerializer for TypedDictSerializer {
    const EXPECTED_TYPE: &'static str = "typed-dict";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let total = schema_or_config(schema, config, "total", "typed_dict_total")?.unwrap_or(true);
        let fields_mode =
            match ExtraBehavior::from_schema_or_config(schema, config, ExtraBehavior::Ignore)? {
                ExtraBehavior::Allow => FieldsMode::TypedDictAllow,
                _ => FieldsMode::SimpleDict,
            };
        let serialize_by_alias: Option<bool> = config.get_as("serialize_by_alias")?;
        let extra_serializer = match (schema.get_str("extras_schema"), &fields_mode) {
            (Some(v), FieldsMode::TypedDictAllow) => {
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
            let required = field_info.get_as("required")?.unwrap_or(total);
            if field_info.get_as::<bool>("serialization_exclude")? == Some(true) {
                fields.push(SerField::new(
                    key.clone(),
                    None,
                    None,
                    required,
                    serialize_by_alias,
                ));
                continue;
            }
            if field_info
                .get_str("serialization_exclude_if")
                .is_some_and(|v| !matches!(v, Value::None))
            {
                return schema_err!(
                    "`serialization_exclude_if` is not supported yet: host callbacks are not implemented"
                );
            }
            let alias: Option<String> = field_info.get_as("serialization_alias")?;
            let field_schema: Dict = field_info.get_as_req("schema")?;
            let serializer = CombinedSerializer::build(&field_schema, config, definitions)
                .map_err(|e| {
                    CoreError::Schema(format!("Field `{key}`:\n  {}: {e}", e.kind().python_name()))
                })?;
            fields.push(SerField::new(
                key.clone(),
                alias,
                Some(serializer),
                required,
                serialize_by_alias,
            ));
        }

        Ok(Arc::new(CombinedSerializer::TypedDict(Box::new(Self {
            serializer: GeneralFieldsSerializer::new(fields, fields_mode, extra_serializer),
        }))))
    }
}

impl TypeSerializer for TypedDictSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        let state = &mut state.scoped_set(|s| &mut s.model, Some(value.clone()));
        self.serializer.to_python(value, state)
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        self.invalid_as_json_key(key, state, Self::EXPECTED_TYPE)
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        let state = &mut state.scoped_set(|s| &mut s.model, Some(value.clone()));
        self.serializer.serde_serialize(value, serializer, state)
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}
