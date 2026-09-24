//! Serializers taken from part of their schema. Port of upstream `type_serializers/other.rs`:
//! a `chain` serializes as its last step, `custom-error` as its inner schema and
//! `lax-or-strict` as its strict schema.

use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::serializers::shared::{BuildSerializer, CombinedSerializer};
use crate::validators::as_dict;
use crate::value::{Dict, Value};

pub struct ChainBuilder;

impl BuildSerializer for ChainBuilder {
    const EXPECTED_TYPE: &'static str = "chain";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let steps: Vec<Value> = schema.get_as_req("steps")?;
        let last = steps.last().ok_or_else(|| {
            crate::core_error::CoreError::Schema(
                "One or more steps are required for a chain validator".to_owned(),
            )
        })?;
        CombinedSerializer::build(as_dict(last)?, config, definitions)
    }
}

macro_rules! sub_schema_builder {
    ($Struct:ident, $expected_type:literal, $key:literal) => {
        pub struct $Struct;

        impl BuildSerializer for $Struct {
            const EXPECTED_TYPE: &'static str = $expected_type;

            fn build(
                schema: &Dict,
                config: Option<&Dict>,
                definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
            ) -> CoreResult<Arc<CombinedSerializer>> {
                let sub_schema: Dict = schema.get_as_req($key)?;
                CombinedSerializer::build(&sub_schema, config, definitions)
            }
        }
    };
}

sub_schema_builder!(CustomErrorBuilder, "custom-error", "schema");
sub_schema_builder!(LaxOrStrictBuilder, "lax-or-strict", "strict_schema");

/// Schemas whose values are serialized by inference.
macro_rules! any_build_serializer {
    ($Struct:ident, $expected_type:literal) => {
        pub struct $Struct;

        impl BuildSerializer for $Struct {
            const EXPECTED_TYPE: &'static str = $expected_type;

            fn build(
                schema: &Dict,
                config: Option<&Dict>,
                definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
            ) -> CoreResult<Arc<CombinedSerializer>> {
                super::any::AnySerializer::build(schema, config, definitions)
            }
        }
    };
}

any_build_serializer!(IsInstanceBuilder, "is-instance");
