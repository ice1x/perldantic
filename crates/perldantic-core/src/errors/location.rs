//! Error locations. Port of upstream `errors/location.rs`.

use std::fmt;

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

/// One step of an error location: a field/key name or a sequence index.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LocItem {
    /// String key: a field name or a dict key.
    S(String),
    /// Integer key: a list/tuple index, or an int dict key / union tag.
    I(i64),
}

impl fmt::Display for LocItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::S(s) if s.contains('.') => write!(f, "`{s}`"),
            Self::S(s) => write!(f, "{s}"),
            Self::I(i) => write!(f, "{i}"),
        }
    }
}

impl From<String> for LocItem {
    fn from(s: String) -> Self {
        Self::S(s)
    }
}

impl From<&str> for LocItem {
    fn from(s: &str) -> Self {
        Self::S(s.to_owned())
    }
}

impl From<i64> for LocItem {
    fn from(i: i64) -> Self {
        Self::I(i)
    }
}

impl From<usize> for LocItem {
    #[allow(clippy::cast_possible_wrap)] // indices never approach i64::MAX
    fn from(u: usize) -> Self {
        Self::I(u as i64)
    }
}

impl Serialize for LocItem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::S(s) => serializer.serialize_str(s),
            Self::I(i) => serializer.serialize_i64(*i),
        }
    }
}

/// Where an error occurred, e.g. `["foo", 2]` for the third item of field `foo`.
///
/// Stored in REVERSE order, so prefixing an outer item is a cheap `push`; it is reversed
/// again whenever it is shown or serialized.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Location {
    #[default]
    Empty,
    /// Reversed: the innermost item comes first.
    List(Vec<LocItem>),
}

impl Location {
    pub fn new_some(item: LocItem) -> Self {
        let mut loc = Vec::with_capacity(3);
        loc.push(item);
        Self::List(loc)
    }

    /// Prefix the location with an outer item.
    pub fn with_outer(&mut self, loc_item: LocItem) {
        match self {
            Self::List(loc) => loc.push(loc_item),
            Self::Empty => *self = Self::new_some(loc_item),
        }
    }

    /// The location from outermost to innermost item.
    pub fn items(&self) -> Vec<LocItem> {
        match self {
            Self::Empty => Vec::new(),
            Self::List(loc) => loc.iter().rev().cloned().collect(),
        }
    }
}

impl fmt::Display for Location {
    /// Dotted path followed by a newline, or nothing for an empty location.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List(loc) => {
                let parts: Vec<String> = loc.iter().rev().map(ToString::to_string).collect();
                writeln!(f, "{}", parts.join("."))
            }
            Self::Empty => Ok(()),
        }
    }
}

impl Serialize for Location {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Empty => serializer.serialize_seq(Some(0))?.end(),
            Self::List(loc) => {
                let mut seq = serializer.serialize_seq(Some(loc.len()))?;
                for item in loc.iter().rev() {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
        }
    }
}
