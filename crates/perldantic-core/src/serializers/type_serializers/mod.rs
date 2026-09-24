//! Serializers for each schema type. Port of upstream `serializers/type_serializers/`.

pub(crate) mod any;
pub(crate) mod bytes;
pub(crate) mod datetime_etc;
pub(crate) mod decimal;
pub(crate) mod definitions;
pub(crate) mod dict;
pub(crate) mod enum_;
pub(crate) mod float;
pub(crate) mod format;
pub(crate) mod function;
pub(crate) mod json;
pub(crate) mod list;
pub(crate) mod literal;
pub(crate) mod model;
pub(crate) mod nullable;
pub(crate) mod other;
pub(crate) mod set;
pub(crate) mod simple;
pub(crate) mod string;
pub(crate) mod timedelta;
pub(crate) mod tuple;
pub(crate) mod typed_dict;
pub(crate) mod union;
pub(crate) mod url;
pub(crate) mod uuid;
pub(crate) mod with_default;
