//! The serializer trait, the serializer registry and JSON output. Port of upstream
//! `serializers/shared.rs`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Debug;
use std::io::{self, Write};
use std::sync::Arc;

use enum_dispatch::enum_dispatch;
use serde::{Serialize, Serializer};
use serde_json::ser::{Formatter, PrettyFormatter};

use crate::build_tools::{SchemaDict, schema_err};
use crate::core_error::{CoreError, CoreResult};
use crate::definitions::DefinitionsBuilder;
use crate::value::{Dict, Value};

use super::errors::{SerResult, se_err_py_err};
use super::extra::SerializationState;
use super::infer::{infer_json_key, infer_serialize, infer_to_python};
use super::ob_type::{IsType, ObType, is_type};
use super::ser::PythonSerializer;
use super::type_serializers::any::AnySerializer;

pub(crate) trait BuildSerializer: Sized {
    const EXPECTED_TYPE: &'static str;

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>>;
}

/// Registers serializers built from a schema `type`: generates the lookup and the list of
/// supported types. One serializer per line; braces keep rustfmt from reflowing the list.
macro_rules! serializers {
    ($($serializer:path,)+) => {
        const SUPPORTED_SERIALIZER_TYPES: &[&str] = &[$(<$serializer>::EXPECTED_TYPE,)+];

        fn find_serializer(
            lookup_type: &str,
            schema: &Dict,
            config: Option<&Dict>,
            definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
        ) -> CoreResult<Arc<CombinedSerializer>> {
            match lookup_type {
                $(<$serializer>::EXPECTED_TYPE => <$serializer>::build(schema, config, definitions)
                    .map_err(|err| failed_to_build_serializer(lookup_type, &err)),)+
                _ => schema_err!("Unknown serialization schema type: `{lookup_type}`"),
            }
        }
    };
}

serializers! {
    super::type_serializers::any::AnySerializer,
    super::type_serializers::bytes::BytesSerializer,
    super::type_serializers::definitions::DefinitionRefSerializer,
    super::type_serializers::definitions::DefinitionsSerializerBuilder,
    super::type_serializers::dict::DictSerializer,
    super::type_serializers::float::FloatSerializer,
    super::type_serializers::list::ListSerializer,
    super::type_serializers::literal::LiteralSerializer,
    super::type_serializers::model::ModelFieldsBuilder,
    super::type_serializers::model::ModelSerializer,
    super::type_serializers::nullable::NullableSerializer,
    super::type_serializers::set::SetSerializer,
    super::type_serializers::simple::BoolSerializer,
    super::type_serializers::simple::IntSerializer,
    super::type_serializers::simple::NoneSerializer,
    super::type_serializers::string::StrSerializer,
    super::type_serializers::tuple::TupleSerializer,
    super::type_serializers::union::TaggedUnionSerializer,
    super::type_serializers::union::UnionSerializer,
    super::type_serializers::with_default::WithDefaultSerializer,
}

#[cold]
fn failed_to_build_serializer(lookup_type: &str, err: &CoreError) -> CoreError {
    CoreError::Schema(format!(
        "Error building `{lookup_type}` serializer:\n  {}: {err}",
        err.kind().python_name()
    ))
}

/// Schema types this build can serialize.
pub(crate) fn supported_serializer_types() -> &'static [&'static str] {
    SUPPORTED_SERIALIZER_TYPES
}

/// Every serializer, dispatched statically.
#[derive(Debug)]
#[enum_dispatch]
pub enum CombinedSerializer {
    Any(super::type_serializers::any::AnySerializer),
    Bool(super::type_serializers::simple::BoolSerializer),
    Bytes(super::type_serializers::bytes::BytesSerializer),
    Dict(super::type_serializers::dict::DictSerializer),
    Float(super::type_serializers::float::FloatSerializer),
    Int(super::type_serializers::simple::IntSerializer),
    List(super::type_serializers::list::ListSerializer),
    Fields(super::fields::GeneralFieldsSerializer),
    Literal(super::type_serializers::literal::LiteralSerializer),
    Model(super::type_serializers::model::ModelSerializer),
    None(super::type_serializers::simple::NoneSerializer),
    Nullable(super::type_serializers::nullable::NullableSerializer),
    Recursive(super::type_serializers::definitions::DefinitionRefSerializer),
    Set(super::type_serializers::set::SetSerializer),
    Str(super::type_serializers::string::StrSerializer),
    Tuple(super::type_serializers::tuple::TupleSerializer),
    Union(super::type_serializers::union::UnionSerializer),
    // Boxed: much larger than most serializers.
    TaggedUnion(Box<super::type_serializers::union::TaggedUnionSerializer>),
    WithDefault(super::type_serializers::with_default::WithDefaultSerializer),
}

impl CombinedSerializer {
    fn build_inner(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        if let Some(ser_schema) = schema.get_as::<Dict>("serialization")? {
            let op_ser_type: Option<String> = ser_schema.get_as("type")?;
            match op_ser_type.as_deref() {
                Some("function-plain" | "function-wrap") => {
                    return schema_err!(
                        "`{}` serializers are not supported yet: host callbacks are not implemented",
                        op_ser_type.unwrap_or_default()
                    );
                }
                Some(
                    // applies to lists tuples and dicts, does not override the main schema `type`
                    "include-exclude-sequence" | "include-exclude-dict"
                    // applies specifically to bytes, does not override the main schema `type`
                    | "base64",
                )
                // if `schema.serialization.type` is None, fall back to `schema.type`
                | None => (),
                Some(_) => {
                    // otherwise, `schema.serialization` is an arbitrary core schema, so build a
                    // serializer from it as if it was the main schema
                    return Self::build(&ser_schema, config, definitions);
                }
            }
        }

        let type_: String = schema.get_as_req("type")?;
        find_serializer(&type_, schema, config, definitions)
    }

    /// Main recursive way to call serializers, supports possible recursive type inference by
    /// switching to type inference mode eagerly.
    pub fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        if state.extra.serialize_as_any {
            infer_to_python(value, state)
        } else {
            self.to_python_no_infer(value, state)
        }
    }

    /// Variant of the above which does not fall back to inference mode immediately
    #[inline]
    pub fn to_python_no_infer(
        &self,
        value: &Value,
        state: &mut SerializationState,
    ) -> SerResult<Value> {
        TypeSerializer::to_python(self, value, state)
    }

    pub fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        if state.extra.serialize_as_any {
            infer_json_key(key, state)
        } else {
            self.json_key_no_infer(key, state)
        }
    }

    #[inline]
    pub fn json_key_no_infer<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        TypeSerializer::json_key(self, key, state)
    }

    pub fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        if state.extra.serialize_as_any {
            infer_serialize(value, serializer, state)
        } else {
            self.serde_serialize_no_infer(value, serializer, state)
        }
    }

    #[inline]
    pub fn serde_serialize_no_infer<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        TypeSerializer::serde_serialize(self, value, serializer, state)
    }
}

impl BuildSerializer for CombinedSerializer {
    // this value is never used, it's just here to satisfy the trait
    const EXPECTED_TYPE: &'static str = "";

    fn build(
        schema: &Dict,
        config: Option<&Dict>,
        definitions: &mut DefinitionsBuilder<Arc<CombinedSerializer>>,
    ) -> CoreResult<Arc<CombinedSerializer>> {
        Self::build_inner(schema, config, definitions)
    }
}

/// Rarely used, large serializers are boxed in `CombinedSerializer`; delegate to the inner one.
impl<T: TypeSerializer> TypeSerializer for Box<T> {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value> {
        (**self).to_python(value, state)
    }

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>> {
        (**self).json_key(key, state)
    }

    fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error> {
        (**self).serde_serialize(value, serializer, state)
    }

    fn get_name(&self) -> &str {
        (**self).get_name()
    }

    fn retry_with_lax_check(&self) -> bool {
        (**self).retry_with_lax_check()
    }

    fn get_default(&self) -> CoreResult<Option<Value>> {
        (**self).get_default()
    }
}

#[enum_dispatch(CombinedSerializer)]
pub(crate) trait TypeSerializer: Send + Sync + Debug {
    fn to_python(&self, value: &Value, state: &mut SerializationState) -> SerResult<Value>;

    fn json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
    ) -> SerResult<Cow<'a, str>>;

    fn invalid_as_json_key<'a>(
        &self,
        key: &'a Value,
        state: &mut SerializationState,
        expected_type: &'static str,
    ) -> SerResult<Cow<'a, str>> {
        match is_type(key, ObType::None) {
            IsType::Exact | IsType::Subclass => {
                Err(CoreError::Type(format!("`{expected_type}` not valid as object key")).into())
            }
            IsType::False => {
                state.warn_fallback_py(self.get_name(), key)?;
                infer_json_key(key, state)
            }
        }
    }

    fn serde_serialize<S: Serializer>(
        &self,
        value: &Value,
        serializer: S,
        state: &mut SerializationState,
    ) -> Result<S::Ok, S::Error>;

    fn get_name(&self) -> &str;

    /// Used by union serializers to decide if it's worth trying again while allowing subclasses
    fn retry_with_lax_check(&self) -> bool {
        false
    }

    fn get_default(&self) -> CoreResult<Option<Value>> {
        Ok(None)
    }
}

/// Makes a value and its serializer usable as a serde `Serialize`.
pub(crate) struct PydanticSerializer<'slf> {
    value: &'slf Value,
    serializer: &'slf CombinedSerializer,
    /// RefCell to allow mutable access to the state during serialization, we expect it
    /// to only ever be borrowed mutably once at a time.
    state: RefCell<&'slf mut SerializationState>,
}

impl<'slf> PydanticSerializer<'slf> {
    pub(crate) fn new(
        value: &'slf Value,
        serializer: &'slf CombinedSerializer,
        state: &'slf mut SerializationState,
    ) -> Self {
        Self {
            value,
            serializer: if state.extra.serialize_as_any {
                AnySerializer::get()
            } else {
                serializer
            },
            state: RefCell::new(state),
        }
    }

    /// Same as above but will not fall back to type inference when `serialize_as_any` is set
    pub(crate) fn new_no_infer(
        value: &'slf Value,
        serializer: &'slf CombinedSerializer,
        state: &'slf mut SerializationState,
    ) -> Self {
        Self {
            value,
            serializer,
            state: RefCell::new(state),
        }
    }
}

impl Serialize for PydanticSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // inference is handled in the constructor
        self.serializer.serde_serialize_no_infer(
            self.value,
            serializer,
            &mut self.state.borrow_mut(),
        )
    }
}

struct EscapeNonAsciiFormatter;

impl Formatter for EscapeNonAsciiFormatter {
    fn write_string_fragment<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut input = fragment;

        while let Some((idx, non_ascii_char)) = input.char_indices().find(|(_, c)| !c.is_ascii()) {
            if idx > 0 {
                // write all ascii characters before the non-ascii one
                writer.write_all(&input.as_bytes()[..idx])?;
            }

            let codepoint = non_ascii_char as u32;
            if codepoint < 0xFFFF {
                // write basic codepoint as single escape
                write!(writer, "\\u{codepoint:04x}")?;
            } else {
                // encode extended plane character as utf16 pair
                for escape in non_ascii_char.encode_utf16(&mut [0; 2]) {
                    write!(writer, "\\u{escape:04x}")?;
                }
            }

            input = &input[idx + non_ascii_char.len_utf8()..];
        }

        // write any ascii trailer
        writer.write_all(input.as_bytes())?;
        Ok(())
    }
}

struct EscapeNonAsciiPrettyFormatter<'a> {
    pretty: PrettyFormatter<'a>,
    escape_non_ascii: EscapeNonAsciiFormatter,
}

impl<'a> EscapeNonAsciiPrettyFormatter<'a> {
    pub fn with_indent(indent: &'a [u8]) -> Self {
        Self {
            pretty: PrettyFormatter::with_indent(indent),
            escape_non_ascii: EscapeNonAsciiFormatter,
        }
    }
}

macro_rules! defer {
    ($formatter:ident, $fun:ident) => {
        fn $fun<W>(&mut self, writer: &mut W) -> io::Result<()>
        where
            W: ?Sized + io::Write,
        {
            self.$formatter.$fun(writer)
        }
    };
    ($formatter:ident, $fun:ident, $val:ty) => {
        fn $fun<W>(&mut self, writer: &mut W, val: $val) -> io::Result<()>
        where
            W: ?Sized + io::Write,
        {
            self.$formatter.$fun(writer, val)
        }
    };
}

impl Formatter for EscapeNonAsciiPrettyFormatter<'_> {
    defer!(escape_non_ascii, write_string_fragment, &str);
    defer!(pretty, begin_array);
    defer!(pretty, end_array);
    defer!(pretty, begin_array_value, bool);
    defer!(pretty, end_array_value);
    defer!(pretty, begin_object);
    defer!(pretty, end_object);
    defer!(pretty, begin_object_key, bool);
    defer!(pretty, end_object_key);
    defer!(pretty, begin_object_value);
    defer!(pretty, end_object_value);
}

/// Serialize to JSON text, optionally indented and with non-ASCII characters escaped.
pub(crate) fn to_json_bytes(
    value: &Value,
    serializer: &CombinedSerializer,
    state: &mut SerializationState,
    indent: Option<usize>,
    ensure_ascii: bool,
) -> SerResult<Vec<u8>> {
    let serializer = PydanticSerializer::new(value, serializer, state);
    let writer: Vec<u8> = Vec::with_capacity(1024);
    let result = match (indent, ensure_ascii) {
        (Some(indent), true) => {
            let indent = vec![b' '; indent];
            let formatter = EscapeNonAsciiPrettyFormatter::with_indent(&indent);
            let mut ser = PythonSerializer::with_formatter(writer, formatter);
            serializer.serialize(&mut ser).map(|()| ser.into_inner())
        }
        (Some(indent), false) => {
            let indent = vec![b' '; indent];
            let formatter = PrettyFormatter::with_indent(&indent);
            let mut ser = PythonSerializer::with_formatter(writer, formatter);
            serializer.serialize(&mut ser).map(|()| ser.into_inner())
        }
        (None, true) => {
            let mut ser = PythonSerializer::with_formatter(writer, EscapeNonAsciiFormatter);
            serializer.serialize(&mut ser).map(|()| ser.into_inner())
        }
        (None, false) => {
            let mut ser = PythonSerializer::new(writer);
            serializer.serialize(&mut ser).map(|()| ser.into_inner())
        }
    };
    result.map_err(|e| se_err_py_err(&e))
}
