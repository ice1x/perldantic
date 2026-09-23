//! `definitions` and `definition-ref` serializers. Port of upstream
//! `type_serializers/definitions.rs`.

use std::borrow::Cow;
use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::{DefinitionRef, DefinitionsBuilder, RecursionSafeCache};
use crate::serializers::errors::{SerResult, py_err_se_err};
use crate::serializers::extra::SerializationState;
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::validators::as_dict;
use crate::value::{Dict, Value};

/// Registers shared definitions, then builds the inner schema.
#[derive(Debug)]
pub struct DefinitionsSerializerBuilder;

impl BuildSerializer for DefinitionsSerializerBuilder {
    const EXPECTED_TYPE: &'static str = "definitions";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let schema_definitions: Vec<Value> = schema.get_as_req("definitions")?;
        for schema_definition in &schema_definitions {
            let schema = as_dict(schema_definition)?;
            let reference = schema.get_as_req::<String>("ref")?;
            let serializer = CombinedSerializer::build(schema, config, definitions)?;
            definitions.add_definition(reference, serializer)?;
        }

        let inner_schema: Dict = schema.get_as_req("schema")?;
        CombinedSerializer::build(&inner_schema, config, definitions)
    }
}

pub struct DefinitionRefSerializer {
    definition: DefinitionRef<Arc<CombinedSerializer>>,
    retry_with_lax_check: RecursionSafeCache<bool>,
}

impl std::fmt::Debug for DefinitionRefSerializer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefinitionRefSerializer")
            .field("definition", &self.definition)
            .field("retry_with_lax_check", &self.retry_with_lax_check())
            .finish()
    }
}

impl BuildSerializer for DefinitionRefSerializer {
    const EXPECTED_TYPE: &'static str = "definition-ref";

    fn build(
        schema: &Dict,
        _config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let schema_ref: String = schema.get_as_req("schema_ref")?;
        let definition = definitions.get_definition(&schema_ref);
        Ok(Arc::new(CombinedSerializer::Recursive(Self {
            definition,
            retry_with_lax_check: RecursionSafeCache::new(),
        })))
    }
}

const FILLED: &str = "definitions are filled before serialization";

impl TypeSerializer for DefinitionRefSerializer {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        self.definition.read(|comb_serializer| {
            let comb_serializer = comb_serializer.expect(FILLED);
            let mut guard = state.recursion_guard(value, self.definition.id())?;
            comb_serializer.to_python_no_infer(value, guard.state())
        })
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        self.definition
            .read(|s| s.expect(FILLED).json_key_no_infer(key, state))
    }

    fn serde_serialize<S: serde::ser::Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        self.definition.read(|comb_serializer| {
            let comb_serializer = comb_serializer.expect(FILLED);
            let mut guard = state
                .recursion_guard(value, self.definition.id())
                .map_err(|e| py_err_se_err(&e))?;
            comb_serializer.serde_serialize_no_infer(value, serializer, guard.state())
        })
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }

    fn retry_with_lax_check(&self) -> bool {
        *self.retry_with_lax_check.get_or_init(
            || {
                self.definition
                    .read(|s| s.expect(FILLED).retry_with_lax_check())
            },
            &false,
        )
    }
}
