//! Serializers of `function-*` schemas and serializer functions. Port of upstream
//! `type_serializers/function.rs`: the Python callables are host functions ([`crate::host`]).

use std::borrow::Cow;
use std::sync::Arc;

use crate::build_tools::SchemaDict;
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::host::{Function, HostCall, HostError, SerializationInfo, SerializerHandler};
use crate::serializers::errors::{SerResult, SerializeError, py_err_se_err};
use crate::serializers::extra::{IncludeExclude, SerializationState};
use crate::serializers::filter::AnyFilter;
use crate::serializers::infer::{infer_json_key, infer_serialize, infer_to_python};
use crate::serializers::shared::{BuildSerializer, CombinedSerializer, TypeSerializer};
use crate::value::{Dict, Value};

use super::any::AnySerializer;
use super::format::WhenUsed;

/// `function-before` schemas serialize as their schema: the function runs before validation.
pub struct FunctionBeforeSerializerBuilder;

impl BuildSerializer for FunctionBeforeSerializerBuilder {
    const EXPECTED_TYPE: &'static str = "function-before";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let schema: Dict = schema.get_as_req("schema")?;
        CombinedSerializer::build(&schema, config, definitions)
    }
}

/// `function-after` schemas: there is no way to know what the function returns, so the wrapped
/// schema is assumed; a `serialization` key overrides it.
pub struct FunctionAfterSerializerBuilder;

impl BuildSerializer for FunctionAfterSerializerBuilder {
    const EXPECTED_TYPE: &'static str = "function-after";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let schema: Dict = schema.get_as_req("schema")?;
        CombinedSerializer::build(&schema, config, definitions)
    }
}

/// `function-plain` validators say nothing about their output: it is serialized by inference.
pub struct FunctionPlainSerializerBuilder;

impl BuildSerializer for FunctionPlainSerializerBuilder {
    const EXPECTED_TYPE: &'static str = "function-plain";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        AnySerializer::build(schema, config, definitions)
    }
}

/// `function-wrap` schemas, like `function-after`, serialize as their schema.
pub struct FunctionWrapSerializerBuilder;

impl BuildSerializer for FunctionWrapSerializerBuilder {
    const EXPECTED_TYPE: &'static str = "function-wrap";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let schema: Dict = schema.get_as_req("schema")?;
        CombinedSerializer::build(&schema, config, definitions)
    }
}

/// The `serialization` entry of a schema with a serializer function:
/// `(is_field_serializer, info_arg, function)`.
fn destructure_function_schema(ser_schema: &Dict) -> CoreResult<(bool, bool, Function)> {
    let function = match ser_schema.get_str("function") {
        Some(Value::Function(function)) => function.clone(),
        Some(other) => {
            return Err(CoreError::Schema(format!(
                "`function` should be a host function, got {}",
                other.type_name()
            )));
        }
        None => return Err(CoreError::Schema("`function` is required".into())),
    };
    let is_field_serializer = ser_schema.get_as("is_field_serializer")?.unwrap_or(false);
    let info_arg = ser_schema.get_as("info_arg")?.unwrap_or(false);
    Ok((is_field_serializer, info_arg, function))
}

/// The schema without its `serialization` (and `ref`, already registered by the outer build).
fn copy_outer_schema(schema: &Dict) -> Dict {
    let mut copy = schema.clone();
    copy.remove_str("serialization");
    copy.remove_str("ref");
    copy
}

/// The arguments serializer functions share.
fn info(state: &SerializationState, is_field_serializer: bool) -> SerResult<SerializationInfo> {
    let field_name = if is_field_serializer {
        match state.field_name() {
            Some(name) => Some(name.to_owned()),
            None => {
                return Err(SerializeError::Core(CoreError::Internal(
                    "Model field context expected for field serialization info but no model field was found"
                        .into(),
                )));
            }
        }
    } else {
        None
    };
    let extra = &state.extra;
    Ok(SerializationInfo {
        include: state.include().cloned(),
        exclude: state.exclude().cloned(),
        context: extra.context.clone(),
        mode: extra.mode.clone(),
        by_alias: extra.by_alias,
        exclude_unset: extra.exclude_unset,
        exclude_defaults: extra.exclude_defaults,
        exclude_none: extra.exclude_none,
        exclude_computed_fields: false,
        round_trip: false,
        serialize_as_any: extra.serialize_as_any,
        field_name,
    })
}

/// The model a field serializer is given.
fn field_model(
    state: &SerializationState,
    is_field_serializer: bool,
    kind: &str,
) -> SerResult<Option<Value>> {
    if !is_field_serializer {
        return Ok(None);
    }
    match &state.model {
        Some(model) => Ok(Some(model.clone())),
        None => Err(SerializeError::Core(CoreError::Internal(format!(
            "Function {kind} serializer expected to be run inside the context of a model field but no model was found"
        )))),
    }
}

/// Port of upstream `on_error`: how an exception raised by the function ends serialization.
/// `Ok` means "serialize the value by inference instead" (after recording a warning).
fn on_error(
    error: HostError,
    function_name: &str,
    state: &mut SerializationState,
) -> SerResult<()> {
    match error {
        HostError::Serialization(SerializeError::UnexpectedValue(unexpected)) => {
            if state.check.enabled() {
                Err(SerializeError::UnexpectedValue(unexpected))
            } else {
                state.warnings.register_warning(unexpected);
                Ok(())
            }
        }
        HostError::Serialization(SerializeError::Serialization(message)) => {
            Err(SerializeError::Serialization(message))
        }
        other => Err(SerializeError::Serialization(format!(
            "Error calling function `{function_name}`: {other}"
        ))),
    }
}

#[derive(Debug)]
pub struct FunctionPlainSerializer {
    func: Function,
    name: String,
    return_serializer: Arc<CombinedSerializer>,
    /// Used when `when_used` says the function does not apply.
    fallback_serializer: Option<Arc<CombinedSerializer>>,
    when_used: WhenUsed,
    pub(crate) is_field_serializer: bool,
    info_arg: bool,
}

impl FunctionPlainSerializer {
    /// `schema` is the whole core schema, not its `serialization` entry, as upstream.
    pub fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let ser_schema: Dict = schema.get_as_req("serialization")?;
        let (is_field_serializer, info_arg, func) = destructure_function_schema(&ser_schema)?;
        let return_serializer = match ser_schema.get_as::<Dict>("return_schema")? {
            Some(s) => CombinedSerializer::build(&s, config, definitions)?,
            None => AnySerializer::build(schema, config, definitions)?,
        };
        let when_used = WhenUsed::new(&ser_schema, WhenUsed::Always)?;
        let fallback_serializer = match when_used {
            WhenUsed::Always => None,
            _ => Some(CombinedSerializer::build(
                &copy_outer_schema(schema),
                config,
                definitions,
            )?),
        };
        Ok(Arc::new(CombinedSerializer::Function(Box::new(Self {
            name: format!("plain_function[{}]", func.name()),
            func,
            return_serializer,
            fallback_serializer,
            when_used,
            is_field_serializer,
            info_arg,
        }))))
    }

    /// Call the function when it applies: `(true, output)`, or `(false, value)` otherwise.
    fn call(
        &self,
        value: &Value,
        state: &mut SerializationState,
    ) -> SerResult<Result<(bool, Value), HostError>> {
        if !self.when_used.should_use(value, &state.extra) {
            return Ok(Ok((false, value.clone())));
        }
        let model = field_model(state, self.is_field_serializer, "plain")?;
        let info = if self.info_arg {
            Some(info(state, self.is_field_serializer)?)
        } else {
            None
        };
        Ok(self
            .func
            .call(HostCall::Serialize {
                value: value.clone(),
                model,
                info,
            })
            .map(|v| (true, v)))
    }

    fn fallback(&self) -> &CombinedSerializer {
        self.fallback_serializer
            .as_deref()
            .expect("a fallback serializer exists unless when_used is always")
    }
}

#[derive(Debug)]
pub struct FunctionWrapSerializer {
    serializer: Arc<CombinedSerializer>,
    func: Function,
    name: String,
    return_serializer: Arc<CombinedSerializer>,
    when_used: WhenUsed,
    pub(crate) is_field_serializer: bool,
    info_arg: bool,
}

impl FunctionWrapSerializer {
    /// `schema` is the whole core schema, not its `serialization` entry, as upstream.
    pub fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        let ser_schema: Dict = schema.get_as_req("serialization")?;
        let (is_field_serializer, info_arg, func) = destructure_function_schema(&ser_schema)?;
        // `serialization.schema`, else the schema itself without its `serialization`
        let inner_schema = match ser_schema.get_as::<Dict>("schema")? {
            Some(s) => s,
            None => copy_outer_schema(schema),
        };
        let serializer = CombinedSerializer::build(&inner_schema, config, definitions)?;
        let return_serializer = match ser_schema.get_as::<Dict>("return_schema")? {
            Some(s) => CombinedSerializer::build(&s, config, definitions)?,
            None => AnySerializer::build(schema, config, definitions)?,
        };
        Ok(Arc::new(CombinedSerializer::FunctionWrap(Box::new(Self {
            name: format!("wrap_function[{}, {}]", func.name(), serializer.get_name()),
            serializer,
            func,
            return_serializer,
            when_used: WhenUsed::new(&ser_schema, WhenUsed::Always)?,
            is_field_serializer,
            info_arg,
        }))))
    }

    fn call(
        &self,
        value: &Value,
        state: &mut SerializationState,
    ) -> SerResult<Result<(bool, Value), HostError>> {
        if !self.when_used.should_use(value, &state.extra) {
            return Ok(Ok((false, value.clone())));
        }
        let model = field_model(state, self.is_field_serializer, "wrap")?;
        let info = if self.info_arg {
            Some(info(state, self.is_field_serializer)?)
        } else {
            None
        };
        let mut handler = SerializationCallable {
            serializer: &self.serializer,
            state: state.fork(),
            filter: AnyFilter::new(),
        };
        let result = self.func.call(HostCall::SerializeWrap {
            value: value.clone(),
            model,
            handler: &mut handler,
            info,
        });
        state.warnings.absorb(handler.state.warnings);
        Ok(result.map(|v| (true, v)))
    }

    fn fallback(&self) -> &CombinedSerializer {
        &self.serializer
    }
}

/// Port of upstream `SerializationCallable`, the wrap serializer's `handler`.
struct SerializationCallable<'s> {
    serializer: &'s CombinedSerializer,
    state: SerializationState,
    filter: AnyFilter,
}

impl SerializerHandler for SerializationCallable<'_> {
    fn serialize(&mut self, value: Value, index_key: Option<Value>) -> Result<Value, HostError> {
        // wrap serializers are tied to their inner type: no inference at this level
        let state = &mut self.state;
        let serialized = match index_key {
            None => self.serializer.to_python_no_infer(&value, state),
            Some(key) => {
                let filter = match &key {
                    Value::Int(i) if *i >= 0 =>
                    {
                        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                        self.filter.index_filter(*i as usize, state, None)
                    }
                    Value::Str(_) | Value::Int(_) | Value::BigInt(_) => {
                        self.filter.key_filter(&key, state)
                    }
                    other => {
                        return Err(HostError::Core(CoreError::Type(format!(
                            "'index_key' is expected to be an integer or a string, got '{}'",
                            other.repr()
                        ))));
                    }
                };
                match filter.map_err(HostError::Serialization)? {
                    Some(next) => {
                        let state = &mut state.scoped_include_exclude(next);
                        self.serializer.to_python_no_infer(&value, state)
                    }
                    None => return Err(HostError::Omit),
                }
            }
        };
        serialized.map_err(HostError::Serialization)
    }
}

macro_rules! function_type_serializer {
    ($name:ident) => {
        impl TypeSerializer for $name {
            fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
                let (ret_serializer, v, include_exclude) = match self.call(value, state)? {
                    // the function did the filtering: none for the return serializer
                    Ok((true, v)) => (&*self.return_serializer, v, IncludeExclude::empty()),
                    Ok((false, v)) => (self.fallback(), v, state.include_exclude.clone()),
                    Err(err) => {
                        on_error(err, self.func.name(), state)?;
                        return infer_to_python(value, state);
                    }
                };
                let state = &mut state.scoped_include_exclude(include_exclude);
                ret_serializer.to_python(&v, state)
            }

            fn json_key<'a>(
                &self,
                key: &'a Value,
                state: &mut SerializationState,
            ) -> SerResult<Cow<'a, str>> {
                let (ret_serializer, v, include_exclude) = match self.call(key, state)? {
                    Ok((true, v)) => (&*self.return_serializer, v, IncludeExclude::empty()),
                    Ok((false, v)) => (self.fallback(), v, state.include_exclude.clone()),
                    Err(err) => {
                        on_error(err, self.func.name(), state)?;
                        return infer_json_key(key, state);
                    }
                };
                let state = &mut state.scoped_include_exclude(include_exclude);
                ret_serializer
                    .json_key(&v, state)
                    .map(|cow| Cow::Owned(cow.into_owned()))
            }

            fn serde_serialize<S: serde::ser::Serializer>(
                &self,
                value: &Value,
                serializer: S,
                state: &mut SerializationState,
            ) -> Result<S::Ok, S::Error> {
                let called = self.call(value, state).map_err(|e| py_err_se_err(&e))?;
                let (ret_serializer, v, include_exclude) = match called {
                    Ok((true, v)) => (&*self.return_serializer, v, IncludeExclude::empty()),
                    Ok((false, v)) => (self.fallback(), v, state.include_exclude.clone()),
                    Err(err) => {
                        on_error(err, self.func.name(), state).map_err(|e| py_err_se_err(&e))?;
                        return infer_serialize(value, serializer, state);
                    }
                };
                let state = &mut state.scoped_include_exclude(include_exclude);
                ret_serializer.serde_serialize(&v, serializer, state)
            }

            fn get_name(&self) -> &str {
                &self.name
            }

            fn retry_with_lax_check(&self) -> bool {
                self.return_serializer.retry_with_lax_check()
                    || self.fallback_retry_with_lax_check()
            }
        }
    };
}

impl FunctionPlainSerializer {
    fn fallback_retry_with_lax_check(&self) -> bool {
        self.fallback_serializer
            .as_ref()
            .is_some_and(|f| f.retry_with_lax_check())
    }
}

impl FunctionWrapSerializer {
    fn fallback_retry_with_lax_check(&self) -> bool {
        self.serializer.retry_with_lax_check()
    }
}

function_type_serializer!(FunctionPlainSerializer);
function_type_serializer!(FunctionWrapSerializer);
