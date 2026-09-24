//! `uuid` schema. Port of upstream `validators/uuid.rs`: values are `uuid::Uuid`s rather than
//! Python `uuid.UUID` objects.

use std::str::from_utf8;
use std::sync::Arc;

use uuid::{Uuid, Variant};

use crate::build_tools::{SchemaDict, is_strict, schema_err};
use crate::core_error::CoreResult;
use crate::definitions::DefinitionsBuilder;
use crate::errors::{ErrorType, ErrorTypeDefaults, ValError, ValResult};
use crate::input::{Input, InputType};
use crate::value::{Dict, Value};

use super::config::{BytesMode, ValBytesMode};
use super::validation_state::{Exactness, ValidationState};
use super::{BuildValidator, CombinedValidator, Validator};

/// The UUID versions upstream accepts in a schema.
const VERSIONS: [usize; 7] = [1, 3, 4, 5, 6, 7, 8];

#[derive(Debug, Clone)]
pub struct UuidValidator {
    strict: bool,
    version: Option<usize>,
}

impl BuildValidator for UuidValidator {
    const EXPECTED_TYPE: &'static str = "uuid";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        _definitions: &mut DefinitionsBuilder<Arc<CombinedValidator>>,
    ) -> CoreResult<Arc<CombinedValidator>> {
        let version: Option<usize> = schema.get_as("version")?;
        if let Some(version) = version
            && !VERSIONS.contains(&version)
        {
            return schema_err!("'version' must be one of {VERSIONS:?}, got {version}");
        }
        Ok(Arc::new(
            Self {
                strict: is_strict(schema, config)?,
                version,
            }
            .into(),
        ))
    }
}

/// Python's `UUID.version`: only set for RFC 4122 (RFC 9562) UUIDs.
fn py_version(uuid: &Uuid) -> Option<usize> {
    (uuid.get_variant() == Variant::RFC4122).then(|| uuid.get_version_num())
}

impl Validator for UuidValidator {
    fn validate(
        &self,
        input: &(impl Input + ?Sized),
        state: &mut ValidationState<'_>,
    ) -> ValResult<Value> {
        let host_input = matches!(
            state.extra().input_type,
            InputType::Python | InputType::Perl
        );
        if let Some(Value::Uuid(uuid)) = input.as_value() {
            if let Some(expected_version) = self.version
                && py_version(uuid) != Some(expected_version)
            {
                return Err(version_error(expected_version, input));
            }
            Ok(Value::Uuid(*uuid))
        } else if state.strict_or(self.strict) && host_input {
            Err(ValError::new(
                ErrorType::IsInstanceOf {
                    class: "UUID".to_owned(),
                    context: None,
                },
                input,
            ))
        } else {
            // For host data this is a coercion; in JSON a UUID string is an exact match.
            if host_input {
                state.floor_exactness(Exactness::Lax);
            }
            let uuid = self.get_uuid(input)?;
            // The version must match and the variant conform to RFC 9562 (superseding
            // RFC 4122).
            if let Some(expected_version) = self.version
                && py_version(&uuid) != Some(expected_version)
            {
                return Err(version_error(expected_version, input));
            }
            Ok(Value::Uuid(uuid))
        }
    }

    fn get_name(&self) -> &str {
        Self::EXPECTED_TYPE
    }
}

fn version_error(expected_version: usize, input: &(impl Input + ?Sized)) -> ValError {
    ValError::new(
        ErrorType::UuidVersion {
            expected_version,
            context: None,
        },
        input,
    )
}

fn parsing_error(err: &uuid::Error, input: &(impl Input + ?Sized)) -> ValError {
    ValError::new(
        ErrorType::UuidParsing {
            error: err.to_string(),
            context: None,
        },
        input,
    )
}

impl UuidValidator {
    fn get_uuid(&self, input: &(impl Input + ?Sized)) -> ValResult<Uuid> {
        let uuid = if let Ok(string) = input.validate_str(true, false) {
            let string = string.into_inner();
            Uuid::parse_str(&string.as_cow()).map_err(|err| parsing_error(&err, input))?
        } else {
            let bytes = input
                .validate_bytes(
                    true,
                    ValBytesMode {
                        ser: BytesMode::Utf8,
                    },
                )
                .map_err(|_| ValError::new(ErrorTypeDefaults::UuidType, input))?
                .into_inner();
            let bytes = bytes.as_slice();
            // UUID text as UTF-8, otherwise the 16 bytes of the UUID
            match from_utf8(bytes).ok().and_then(|s| Uuid::parse_str(s).ok()) {
                Some(uuid) => uuid,
                None => Uuid::from_slice(bytes).map_err(|err| parsing_error(&err, input))?,
            }
        };
        if let Some(expected_version) = self.version
            && uuid.get_version_num() != expected_version
        {
            return Err(version_error(expected_version, input));
        }
        Ok(uuid)
    }
}
