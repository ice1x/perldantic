//! Paths used to find values in input mappings: tagged-union discriminators now, field aliases
//! with models. Port of upstream `lookup_key.rs`.

use std::borrow::Cow;
use std::fmt;

use jiter::{JsonObject, JsonValue};

use crate::core_error::{CoreError, CoreResult};
use crate::errors::{LocItem, Location, ValLineError};
use crate::value::{Dict, Value};

/// The error pyo3 reports when a value is not of the expected Python type.
fn not_an_instance(value: &Value, expected: &str) -> CoreError {
    CoreError::Type(format!(
        "'{}' object is not an instance of '{expected}'",
        value.type_name()
    ))
}

/// `Kind: message`, as a Python exception is shown in a chained error.
fn describe(err: &CoreError) -> String {
    format!("{}: {err}", err.kind().python_name())
}

/// The possible choices for an alias value: `str`, `list[int | str]` or
/// `list[list[int | str]]`. Returns the paths it describes.
///
/// Mirrors pyo3's extraction of upstream's `ValidationAlias` enum, including its error text
/// when no form matches.
pub(crate) fn validation_alias_paths(value: &Value) -> CoreResult<Vec<LookupPath>> {
    let as_str = match value {
        Value::Str(s) => {
            return Ok(vec![LookupPath {
                first_item: PathItemString(s.clone()),
                rest: Vec::new(),
            }]);
        }
        other => not_an_instance(other, "str"),
    };
    let as_path = match LookupPath::from_list(value) {
        Ok(path) => return Ok(vec![path]),
        Err(e) => e,
    };
    let as_choices = match LookupPath::from_multiple_list(value) {
        Ok(paths) => return Ok(paths),
        Err(e) => e,
    };
    let variant = |name: &str, err: &CoreError| {
        format!(
            "- variant {name} ({name}): TypeError: failed to extract field ValidationAlias::{name}.0, caused by {}",
            describe(err)
        )
    };
    Err(CoreError::Type(format!(
        "failed to extract enum ValidationAlias ('Str | AliasPath | AliasChoices')\n{}\n{}\n{}",
        variant("Str", &as_str),
        variant("AliasPath", &as_path),
        variant("AliasChoices", &as_choices),
    )))
}

#[derive(Debug)]
pub struct LookupPath {
    /// All paths must start with a string key
    first_item: PathItemString,
    /// Most paths will have no extra items, though some do so we encode this here
    rest: Vec<PathItem>,
}

impl fmt::Display for LookupPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{first_key}", first_key = self.first_item)?;
        for item in &self.rest {
            write!(f, ".{item}")?;
        }
        Ok(())
    }
}

impl LookupPath {
    fn from_list(value: &Value) -> CoreResult<LookupPath> {
        let Value::List(items) = value else {
            return Err(not_an_instance(value, "list"));
        };
        let mut iter = items.iter();

        let Some(first_item) = iter.next() else {
            return Err(CoreError::Schema(
                "Each alias path should have at least one element".into(),
            ));
        };

        let Value::Str(first_item) = first_item else {
            return Err(CoreError::Type(
                "The first item in an alias path should be a string".into(),
            ));
        };

        let rest = iter.map(PathItem::from_value).collect::<CoreResult<_>>()?;

        Ok(Self {
            first_item: PathItemString(first_item.clone()),
            rest,
        })
    }

    fn from_multiple_list(value: &Value) -> CoreResult<Vec<LookupPath>> {
        let Value::List(items) = value else {
            return Err(not_an_instance(value, "list"));
        };
        if items.is_empty() {
            return Err(CoreError::Schema(
                "Lookup paths should have at least one element".into(),
            ));
        }
        items.iter().map(Self::from_list).collect()
    }

    /// Look the path up in host data: a dict key first, then items of nested values.
    pub fn value_get<'a>(&self, dict: &'a Dict) -> Option<Cow<'a, Value>> {
        let first = dict.get_str(&self.first_item.0)?;
        self.rest
            .iter()
            .try_fold(Cow::Borrowed(first), |value, item| item.value_get(value))
    }

    pub fn json_get<'a, 'data>(&self, dict: &'a JsonObject<'data>) -> Option<&'a JsonValue<'data>> {
        // first step is different as the first step is a key lookup; with duplicate keys the
        // last one wins, as in Python
        let first = dict
            .iter()
            .rev()
            .find_map(|(k, v)| (k == self.first_key()).then_some(v))?;
        // fold the rest of the path over the found value
        self.rest.iter().try_fold(first, |d, loc| loc.json_get(d))
    }

    /// get the `str` from the first item in the path, note paths always have length > 0, and
    /// the first item is always a string
    pub fn first_key(&self) -> &str {
        &self.first_item.0
    }

    /// get the first item in the path
    pub fn first_item(&self) -> &PathItemString {
        &self.first_item
    }

    pub fn rest(&self) -> &[PathItem] {
        &self.rest
    }

    pub fn loc(&self) -> Location {
        let mut location = Vec::with_capacity(1 + self.rest.len());
        for item in self.rest.iter().rev() {
            location.push(item.to_loc_item());
        }
        location.push(LocItem::from(self.first_item.0.clone()));
        Location::List(location)
    }

    pub fn apply_error_loc(
        &self,
        mut line_error: ValLineError,
        loc_by_alias: bool,
        field_name: &str,
    ) -> ValLineError {
        if loc_by_alias {
            for path_item in self.rest.iter().rev() {
                line_error = line_error.with_outer_location(path_item.to_loc_item());
            }
            line_error.with_outer_location(self.first_item.0.clone())
        } else {
            line_error.with_outer_location(field_name)
        }
    }
}

#[derive(Debug, Clone)]
pub enum PathItem {
    S(PathItemString),
    /// integer key, used to get items from a list, tuple OR a dict with int keys
    Pos(usize),
    Neg(usize),
}

/// String type key, used to get or identify items from a dict
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PathItemString(pub String);

impl fmt::Display for PathItemString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{key}'", key = self.0)
    }
}

impl fmt::Display for PathItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::S(key) => key.fmt(f),
            Self::Pos(key) => write!(f, "{key}"),
            Self::Neg(key) => write!(f, "-{key}"),
        }
    }
}

impl PathItem {
    fn from_value(value: &Value) -> CoreResult<Self> {
        match value {
            Value::Str(s) => Ok(Self::S(PathItemString(s.clone()))),
            Value::Bool(b) => Ok(Self::Pos(usize::from(*b))),
            Value::Int(i) => Ok(match usize::try_from(*i) {
                Ok(index) => Self::Pos(index),
                Err(_) => Self::Neg(i.unsigned_abs() as usize),
            }),
            _ => Err(CoreError::Type(
                "Item in an alias path should be a string or int".into(),
            )),
        }
    }

    /// Python's `value[item]`, `None` where Python would raise. Strings are never indexed.
    fn value_get<'a>(&self, value: Cow<'a, Value>) -> Option<Cow<'a, Value>> {
        let index = |len: usize| match self {
            Self::Pos(i) => (*i < len).then_some(*i),
            Self::Neg(i) => len.checked_sub(*i).filter(|_| *i > 0),
            Self::S(_) => None,
        };
        match value {
            Cow::Borrowed(value) => match value {
                Value::List(items) | Value::Tuple(items) => {
                    index(items.len()).map(|i| Cow::Borrowed(&items[i]))
                }
                Value::Dict(dict) => self.dict_get(dict).map(Cow::Borrowed),
                Value::Bytes(bytes) => {
                    index(bytes.len()).map(|i| Cow::Owned(Value::Int(i64::from(bytes[i]))))
                }
                _ => None,
            },
            // Only bytes items are synthesized, and ints cannot be indexed further.
            Cow::Owned(_) => None,
        }
    }

    fn dict_get<'a>(&self, dict: &'a Dict) -> Option<&'a Value> {
        let key = match self {
            Self::S(key) => return dict.get_str(&key.0),
            #[allow(clippy::cast_possible_wrap)] // path indices never approach i64::MAX
            Self::Pos(i) => Value::Int(*i as i64),
            #[allow(clippy::cast_possible_wrap)]
            Self::Neg(i) => Value::Int(-(*i as i64)),
        };
        dict.iter().find(|(k, _)| k.py_eq(&key)).map(|(_, v)| v)
    }

    pub fn json_get<'a, 'data>(
        &self,
        any_json: &'a JsonValue<'data>,
    ) -> Option<&'a JsonValue<'data>> {
        match any_json {
            JsonValue::Object(v_obj) => self.json_obj_get(v_obj),
            JsonValue::Array(v_array) => match self {
                Self::Pos(index) => v_array.get(*index),
                Self::Neg(index) => {
                    if let Some(index) = v_array.len().checked_sub(*index) {
                        v_array.get(index)
                    } else {
                        None
                    }
                }
                Self::S(..) => None,
            },
            _ => None,
        }
    }

    fn to_loc_item(&self) -> LocItem {
        match self {
            Self::S(PathItemString(key)) => LocItem::from(key.clone()),
            Self::Pos(index) => LocItem::from(*index),
            #[allow(clippy::cast_possible_wrap)] // path indices never approach i64::MAX
            Self::Neg(index) => LocItem::from(-(*index as i64)),
        }
    }

    pub fn json_obj_get<'a, 'data>(
        &self,
        json_obj: &'a JsonObject<'data>,
    ) -> Option<&'a JsonValue<'data>> {
        match self {
            Self::S(PathItemString(key)) => json_obj
                .iter()
                .rev()
                .find_map(|(k, v)| (k == key.as_str()).then_some(v)),
            _ => None,
        }
    }
}

/// The paths a field is looked up by: its name, and its aliases if any.
#[derive(Debug)]
#[allow(clippy::struct_field_names)]
pub struct LookupPathCollection {
    pub by_name: LookupPath,
    pub by_alias: Vec<LookupPath>,
}

impl LookupPathCollection {
    pub fn new(validation_alias: Option<&Value>, field_name: &str) -> CoreResult<Self> {
        let by_name = LookupPath {
            first_item: PathItemString(field_name.to_owned()),
            rest: Vec::new(),
        };
        let by_alias = validation_alias
            .map(validation_alias_paths)
            .transpose()?
            .unwrap_or_default();
        Ok(Self { by_name, by_alias })
    }

    /// Returns the lookup paths to use based on the provided `lookup_type`. At least one path
    /// will always be returned.
    pub fn lookup_paths(&self, lookup_type: LookupType) -> impl Iterator<Item = &LookupPath> {
        let by_alias = lookup_type
            .matches(LookupType::Alias)
            .then_some(&self.by_alias)
            .into_iter()
            .flatten();

        // always use the name if no alias is defined
        let by_name = (self.by_alias.is_empty() || lookup_type.matches(LookupType::Name))
            .then_some(&self.by_name);

        by_alias.chain(by_name)
    }

    /// Returns the error location to use based on the provided `lookup_type` and
    /// `loc_by_alias`.
    pub fn error_loc(&self, lookup_type: LookupType, loc_by_alias: bool) -> Location {
        if loc_by_alias
            && lookup_type.matches(LookupType::Alias)
            && let Some(first_alias) = &self.by_alias.first()
        {
            return first_alias.loc();
        }
        self.by_name.loc()
    }
}

/// Whether this lookup represents a name or an alias
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum LookupType {
    Name = 1,
    Alias = 2,
    Both = 3,
}

impl LookupType {
    pub fn from_bools(validate_by_alias: bool, validate_by_name: bool) -> CoreResult<LookupType> {
        match (validate_by_alias, validate_by_name) {
            (true, true) => Ok(LookupType::Both),
            (true, false) => Ok(LookupType::Alias),
            (false, true) => Ok(LookupType::Name),
            (false, false) => Err(CoreError::Value(
                "`validate_by_name` and `validate_by_alias` cannot both be set to `False`.".into(),
            )),
        }
    }

    pub fn matches(self, other: LookupType) -> bool {
        (self as u8 & other as u8) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(json: &str) -> Value {
        Value::from_json(json).unwrap()
    }

    fn get(path: &str, data: &Value) -> Option<Value> {
        let Value::Dict(dict) = data else { panic!() };
        let paths = validation_alias_paths(&j(path)).unwrap();
        paths[0].value_get(dict).map(Cow::into_owned)
    }

    #[test]
    fn paths_display_like_upstream() {
        let paths = validation_alias_paths(&j(r#"["meta", 0, -1, "x"]"#)).unwrap();
        assert_eq!(paths[0].to_string(), "'meta'.0.-1.'x'");
        assert_eq!(
            validation_alias_paths(&j(r#""k""#)).unwrap()[0].to_string(),
            "'k'"
        );
        assert_eq!(
            validation_alias_paths(&j(r#"[["a"], ["b"]]"#))
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn host_data_lookups_follow_python_indexing() {
        let data = j(r#"{"meta": ["x", "b"], "s": "ab", "d": {"k": 1}}"#);
        assert_eq!(get(r#"["meta", 0]"#, &data), Some(j(r#""x""#)));
        assert_eq!(get(r#"["meta", -1]"#, &data), Some(j(r#""b""#)));
        assert_eq!(get(r#"["meta", 2]"#, &data), None);
        assert_eq!(get(r#"["meta", -3]"#, &data), None);
        assert_eq!(get(r#"["meta", "k"]"#, &data), None);
        // strings are never indexed
        assert_eq!(get(r#"["s", 0]"#, &data), None);
        assert_eq!(get(r#"["d", "k"]"#, &data), Some(Value::Int(1)));
        assert_eq!(get(r#"["missing"]"#, &data), None);

        let mut with_bytes = Dict::new();
        with_bytes.insert(Value::from("b"), Value::Bytes(b"az".to_vec()));
        let mut int_keys = Dict::new();
        int_keys.insert(Value::Bool(false), Value::from("zero"));
        with_bytes.insert(Value::from("d"), Value::Dict(int_keys));
        let data = Value::Dict(with_bytes);
        assert_eq!(get(r#"["b", -1]"#, &data), Some(Value::Int(122)));
        assert_eq!(get(r#"["b", 0, 0]"#, &data), None);
        // dict keys compare like Python: False == 0
        assert_eq!(get(r#"["d", 0]"#, &data), Some(Value::from("zero")));
    }

    #[test]
    fn path_items_must_be_strings_or_ints() {
        assert_eq!(
            validation_alias_paths(&j(r#"["a", 1.5]"#))
                .unwrap_err()
                .to_string()
                .lines()
                .nth(2)
                .unwrap(),
            "- variant AliasPath (AliasPath): TypeError: failed to extract field ValidationAlias::AliasPath.0, caused by TypeError: Item in an alias path should be a string or int"
        );
    }
}
