//! Port of upstream `validators/shared/lookup_tree.rs`.
//!
//! Nested index lookups are kept in a `BTreeMap` instead of upstream's hash map so that the
//! iteration order, and thus which of several equal-priority matches wins, is deterministic.

use std::borrow::Cow;
use std::collections::btree_map;
use std::collections::{BTreeMap, HashMap};

use jiter::{JsonArray, JsonValue};

use crate::lookup_key::{LookupPath, LookupPathCollection, LookupType, PathItem, PathItemString};

/// A tree of paths for lookups when trying to find fields from input.
///
/// The structure is nested maps, typically there is only one level unless there are
/// `AliasPath` aliases which require deeper lookups.
#[derive(Debug)]
pub struct LookupTree {
    inner: HashMap<PathItemString, LookupTreeNode>,
}

impl LookupTree {
    /// Construct a `LookupTree` from a slice of fields and a function to get the
    /// `LookupPathCollection` for each field.
    pub fn from_fields<T>(
        fields: &[T],
        get_field_collection: impl Fn(&T) -> &LookupPathCollection,
    ) -> Self {
        let mut tree = Self {
            inner: HashMap::with_capacity(fields.len()),
        };

        for (field_index, field) in fields.iter().enumerate() {
            let collection = get_field_collection(field);

            add_path_to_map(
                &mut tree.inner,
                &collection.by_name,
                LookupFieldInfo {
                    field_index,
                    lookup_priority: LookupFieldPriority {
                        lookup_type: if collection.by_alias.is_empty() {
                            LookupType::Both
                        } else {
                            LookupType::Name
                        },
                        alias_index: 0,
                    },
                },
            );

            for (alias_index, alias) in collection.by_alias.iter().enumerate() {
                add_path_to_map(
                    &mut tree.inner,
                    alias,
                    LookupFieldInfo {
                        field_index,
                        lookup_priority: LookupFieldPriority {
                            lookup_type: LookupType::Alias,
                            alias_index,
                        },
                    },
                );
            }
        }
        tree
    }

    /// Given a root key and JSON object representing the structure at that key, iterates
    /// through all paths in the lookup tree where there is a field which matches that path.
    pub fn iter_matches<'a, 'j>(
        &'a self,
        root_key: &'a str,
        json_value: &'a JsonValue<'j>,
    ) -> LookupMatchesIter<'a, 'j> {
        let node = self.inner.get(&PathItemString(root_key.to_owned()));
        LookupMatchesIter::new(node, json_value)
    }
}

/// When resolving data for a field, aliases are preferred over names, and earlier aliases are
/// preferred over later ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookupFieldPriority {
    /// The type of lookups that will match this lookup
    lookup_type: LookupType,
    /// The index of this alias within the `AliasChoices` for the field
    alias_index: usize,
}

impl LookupFieldPriority {
    /// Returns `true` if `self` has higher priority than `other`, i.e. data from this lookup
    /// should be used over data from `other`.
    pub fn is_higher_priority_than(&self, other: &Self) -> bool {
        if self.lookup_type == LookupType::Name {
            // name lookups are never higher priority than other lookups
            return false;
        } else if other.lookup_type == LookupType::Name {
            // other is a name lookup, so self is higher priority
            return true;
        }

        // lower alias indices are higher priority
        self.alias_index < other.alias_index
    }
}

/// Represents a location in the lookup tree which corresponds to data for a specific field.
#[derive(Debug, Clone, Copy)]
pub struct LookupFieldInfo {
    /// The field which this lookup will populate.
    pub field_index: usize,
    /// Information about whether this data should be preferred over other possible matches
    /// for the same field.
    pub lookup_priority: LookupFieldPriority,
}

impl LookupFieldInfo {
    /// Whether this lookup should be used for the given lookup type (i.e. when validating
    /// by_name / by_alias)
    pub fn matches_lookup(&self, lookup_type: LookupType) -> bool {
        self.lookup_priority.lookup_type.matches(lookup_type)
    }

    /// The alias index for this lookup, if it is an alias lookup, or `None` if it is a name
    /// lookup.
    pub fn alias_index(&self) -> Option<usize> {
        if self.lookup_priority.lookup_type == LookupType::Alias {
            Some(self.lookup_priority.alias_index)
        } else {
            None
        }
    }
}

/// Represents a point in the lookup tree, containing exact matches plus possible nested
/// lookups.
#[derive(Debug, Default)]
pub struct LookupTreeNode {
    /// All fields which wanted _exactly_ this key, typically this is just a single entry
    fields: Vec<LookupFieldInfo>,
    /// For nested lookups by name, e.g. `['foo', 'bar']`, typically empty
    pub map: HashMap<PathItemString, LookupTreeNode>,
    /// For nested lookups by integer index, e.g. `['foo', 0]`, typically empty
    pub list: BTreeMap<i64, LookupTreeNode>,
}

fn add_field_to_node(node: &mut LookupTreeNode, info: LookupFieldInfo) {
    node.fields.push(info);
}

#[allow(clippy::cast_possible_wrap)] // path indices never approach i64::MAX
fn index_key(item: &PathItem) -> Option<i64> {
    match item {
        PathItem::S(_) => None,
        PathItem::Pos(i) => Some(*i as i64),
        PathItem::Neg(i) => Some(-(*i as i64)),
    }
}

fn child<'n>(node: &'n mut LookupTreeNode, item: &PathItem) -> &'n mut LookupTreeNode {
    match (item, index_key(item)) {
        (PathItem::S(s), _) => node.map.entry(s.clone()).or_default(),
        (_, Some(i)) => node.list.entry(i).or_default(),
        (_, None) => unreachable!("only string items have no index"),
    }
}

fn add_path_to_map(
    map: &mut HashMap<PathItemString, LookupTreeNode>,
    path: &LookupPath,
    info: LookupFieldInfo,
) {
    // traverse the tree structure to find the final node to insert the field info into
    let mut tree_node = map.entry(path.first_item().clone()).or_default();
    for item in path.rest() {
        tree_node = child(tree_node, item);
    }
    add_field_to_node(tree_node, info);
}

/// Iterator for matching fields in a lookup tree against JSON values, return value of
/// `iter_matches`.
pub struct LookupMatchesIter<'a, 'j> {
    stack: Vec<NestedFrame<'a, 'j>>,
}

/// State of the iterator at a given depth in the lookup tree
struct NestedFrame<'a, 'j> {
    json_value: &'a JsonValue<'j>,
    node: &'a LookupTreeNode,
    state: FrameState<'a, 'j>,
}

enum FrameState<'a, 'j> {
    /// Iterating through all fields that match exactly this path
    Fields {
        fields: std::slice::Iter<'a, LookupFieldInfo>,
    },
    /// Iterating through a JSON object at this path which might have matches on its keys
    NestedObject {
        iter: std::slice::Iter<'a, (Cow<'j, str>, JsonValue<'j>)>,
    },
    /// Iterating through a JSON array at this path which might have matches on its indices
    NestedArray {
        // NB we iterate the interesting entries in the lookup map, not the JSON array itself
        iter: btree_map::Iter<'a, i64, LookupTreeNode>,
        json_array: &'a JsonArray<'j>,
    },
}

impl<'a, 'j> LookupMatchesIter<'a, 'j> {
    fn new(node: Option<&'a LookupTreeNode>, json_value: &'a JsonValue<'j>) -> Self {
        let stack = node
            .map(|node| NestedFrame {
                json_value,
                node,
                state: FrameState::Fields {
                    fields: node.fields.iter(),
                },
            })
            .into_iter()
            .collect();
        Self { stack }
    }
}

impl<'a, 'j> Iterator for LookupMatchesIter<'a, 'j> {
    type Item = (&'a LookupFieldInfo, &'a JsonValue<'j>);

    fn next(&mut self) -> Option<Self::Item> {
        'top_level: while let Some(frame) = self.stack.last_mut() {
            match &mut frame.state {
                FrameState::Fields { fields } => {
                    if let Some(field_info) = fields.next() {
                        return Some((field_info, frame.json_value));
                    }

                    // no more fields, possibly explore nested structures if there are complex
                    // aliases
                    match frame.json_value {
                        JsonValue::Object(obj) if !frame.node.map.is_empty() => {
                            frame.state = FrameState::NestedObject { iter: obj.iter() };
                        }
                        JsonValue::Array(arr) if !frame.node.list.is_empty() => {
                            frame.state = FrameState::NestedArray {
                                json_array: arr,
                                iter: frame.node.list.iter(),
                            };
                        }
                        _ => {
                            self.stack.pop();
                        }
                    }
                }
                FrameState::NestedObject { iter } => {
                    if let Some(next_frame) = iter.by_ref().find_map(|(key, value)| {
                        let nested_node = frame
                            .node
                            .map
                            .get(&PathItemString(key.as_ref().to_owned()))?;
                        Some(NestedFrame {
                            json_value: value,
                            node: nested_node,
                            state: FrameState::Fields {
                                fields: nested_node.fields.iter(),
                            },
                        })
                    }) {
                        self.stack.push(next_frame);
                        continue 'top_level;
                    }

                    self.stack.pop();
                }
                FrameState::NestedArray { json_array, iter } => {
                    if let Some(next_frame) = iter.by_ref().find_map(|(list_item, nested_node)| {
                        let index = if *list_item < 0 {
                            usize::try_from(*list_item + i64::try_from(json_array.len()).ok()?)
                                .ok()?
                        } else {
                            usize::try_from(*list_item).ok()?
                        };

                        let value = json_array.get(index)?;
                        Some(NestedFrame {
                            json_value: value,
                            node: nested_node,
                            state: FrameState::Fields {
                                fields: nested_node.fields.iter(),
                            },
                        })
                    }) {
                        self.stack.push(next_frame);
                        continue 'top_level;
                    }

                    self.stack.pop();
                }
            }
        }
        None
    }
}
