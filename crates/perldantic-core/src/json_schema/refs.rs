//! Names of JSON Schema definitions: `$defs` keys derived from core schema refs, and the final
//! pass that replaces them with the simplest unambiguous names. Port of pydantic's
//! `GenerateJsonSchema.get_defs_ref` and `_DefinitionsRemapping`.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::core_error::CoreError;
use crate::value::{Dict, Value};

use super::{JsResult, JsonSchemaError, JsonSchemaMode};

static MODULE_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:[^.\[\]]+\.)+([^.\[\]]+)").expect("valid regex"));
static NOT_NAME_CHARACTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9.\-_]").expect("valid regex"));

/// Normalizes a name to be used as a key in a dictionary.
fn normalize_name(name: &str) -> String {
    NOT_NAME_CHARACTER.replace_all(name, "_").replace('.', "__")
}

/// Split a core ref into components: generic origins and arguments are each separate
/// components, and the `[`, `]` and `,` separators are kept (Python's `re.split(r'([\][,])')`).
fn split_components(core_ref: &str) -> Vec<&str> {
    let mut components = Vec::new();
    let mut start = 0;
    for (i, c) in core_ref.char_indices() {
        if matches!(c, '[' | ']' | ',') {
            components.push(&core_ref[start..i]);
            components.push(&core_ref[i..=i]);
            start = i + 1;
        }
    }
    components.push(&core_ref[start..]);
    components
}

/// The candidate `$defs` keys of every definition, from the most to the least preferred.
#[derive(Debug, Default)]
pub(super) struct DefsRefs {
    /// A fully unique defs ref → its preferred alternatives, which are generally simpler, such
    /// as only the class name.
    pub prioritized_choices: HashMap<String, Vec<String>>,
    collision_counter: HashMap<String, usize>,
    collision_index: HashMap<String, usize>,
}

impl DefsRefs {
    /// The definitions key for a core ref (upstream `get_defs_ref`).
    pub fn get_defs_ref(&mut self, core_ref: &str, mode: JsonSchemaMode) -> String {
        // Remove IDs from each component
        let components: Vec<&str> = split_components(core_ref)
            .into_iter()
            .map(|x| x.rsplit_once(':').map_or(x, |(before, _)| before))
            .collect();
        let core_ref_no_id = components.concat();
        // Remove everything before the last period from each "component"
        let short_ref: String = components
            .iter()
            .map(|x| MODULE_PREFIX.replace_all(x, "$1"))
            .collect();

        let mode_title = mode.title();

        // It is important that the generated defs_ref values be such that at least one choice
        // will not be generated for any other core_ref. Currently, this should be the case
        // because we include the id of the source type in the core_ref
        let name = normalize_name(&short_ref);
        let name_mode = format!("{name}-{mode_title}");
        let module_qualname = normalize_name(&core_ref_no_id);
        let module_qualname_mode = format!("{module_qualname}-{mode_title}");
        let module_qualname_id = normalize_name(core_ref);
        let occurrence_index = match self.collision_index.get(&module_qualname_id) {
            Some(index) => *index,
            None => {
                let counter = self
                    .collision_counter
                    .entry(module_qualname.clone())
                    .or_insert(0);
                *counter += 1;
                self.collision_index.insert(module_qualname_id, *counter);
                *counter
            }
        };

        let module_qualname_occurrence = format!("{module_qualname}__{occurrence_index}");
        let module_qualname_occurrence_mode = format!("{module_qualname_mode}__{occurrence_index}");

        self.prioritized_choices.insert(
            module_qualname_occurrence_mode.clone(),
            vec![
                name,
                name_mode,
                module_qualname,
                module_qualname_mode,
                module_qualname_occurrence,
                module_qualname_occurrence_mode.clone(),
            ],
        );

        module_qualname_occurrence_mode
    }
}

/// Replaces complex defs refs with simpler ones where that introduces no ambiguity.
#[derive(Debug)]
pub(super) struct DefinitionsRemapping {
    defs_remapping: HashMap<String, String>,
    json_remapping: HashMap<String, String>,
}

impl DefinitionsRemapping {
    /// A remapping that replaces complex defs refs with the simpler ones from the prioritized
    /// choices such that applying the name remapping results in an equivalent JSON schema.
    pub fn from_prioritized_choices(
        prioritized_choices: &HashMap<String, Vec<String>>,
        defs_to_json: &HashMap<String, String>,
        definitions: &Dict,
    ) -> JsResult<Self> {
        // We need to iteratively simplify the definitions until we reach a fixed point.
        // The reason for this is that outer definitions may reference inner definitions that get
        // simplified into an equivalent reference, and the outer definitions won't be equivalent
        // until we've simplified the inner definitions.
        let mut copied: Vec<(String, Value)> = definitions
            .iter()
            .map(|(k, v)| (k.py_str(), v.clone()))
            .collect();
        // Upstream compares its previous `$defs` with the remapped one; both share the (mutated
        // in place) schemas, so only the keys differ between the two.
        let mut previous_keys: Vec<String> = copied.iter().map(|(k, _)| k.clone()).collect();
        let choices_of = |defs_ref: &str| {
            prioritized_choices
                .get(defs_ref)
                .ok_or_else(|| JsonSchemaError::Core(CoreError::Key(defs_ref.to_owned())))
        };
        for _ in 0..100 {
            // For every possible remapped defs ref, collect all distinct schemas that defs ref
            // might be used for
            let mut schemas_for_alternatives: HashMap<&str, Vec<&Value>> = HashMap::new();
            for (defs_ref, schema) in &copied {
                for alternative in choices_of(defs_ref)? {
                    let schemas = schemas_for_alternatives.entry(alternative).or_default();
                    // only remap to a new defs ref if it introduces no ambiguity
                    if !schemas.iter().any(|s| s.py_eq(schema)) {
                        schemas.push(schema);
                    }
                }
            }

            let mut defs_remapping = HashMap::new();
            let mut json_remapping = HashMap::new();
            for (original_defs_ref, _) in &copied {
                let alternatives = choices_of(original_defs_ref)?;
                // Pick the first alternative that has only one schema, since that means there is
                // no collision
                let remapped_index = alternatives
                    .iter()
                    .position(|x| schemas_for_alternatives[x.as_str()].len() == 1)
                    .expect("the fully unique choice has a single schema");
                let remapped_defs_ref = &alternatives[remapped_index];
                defs_remapping.insert(original_defs_ref.clone(), remapped_defs_ref.clone());
                // Map all alternatives after the remapped one to the remapped one; this ensures
                // that intermediate simplifications are also remapped
                for alternative in &alternatives[remapped_index..] {
                    json_remapping.insert(
                        defs_to_json[alternative].clone(),
                        defs_to_json[remapped_defs_ref].clone(),
                    );
                }
            }
            let remapping = Self {
                defs_remapping,
                json_remapping,
            };
            for (_, schema) in &mut copied {
                remapping.remap_json_schema(schema);
            }
            let new_keys: Vec<String> = copied
                .iter()
                .map(|(k, _)| remapping.remap_defs_ref(k).to_owned())
                .collect();
            let as_defs = |keys: &[String]| -> Dict {
                keys.iter()
                    .zip(&copied)
                    .map(|(k, (_, schema))| (Value::Str(k.clone()), schema.clone()))
                    .collect()
            };
            if as_defs(&previous_keys).py_eq(&as_defs(&new_keys)) {
                // We've reached the fixed point
                return Ok(remapping);
            }
            previous_keys = new_keys;
        }

        Err(JsonSchemaError::InvalidForJsonSchema(
            "Failed to simplify the JSON schema definitions".to_owned(),
        ))
    }

    fn remap_defs_ref<'a>(&'a self, defs_ref: &'a str) -> &'a str {
        self.defs_remapping
            .get(defs_ref)
            .map_or(defs_ref, String::as_str)
    }

    fn remap_json_ref(&self, json_ref: &mut String) {
        if let Some(remapped) = self.json_remapping.get(json_ref.as_str()) {
            json_ref.clone_from(remapped);
        }
    }

    /// Recursively update the JSON schema replacing all `$ref`s.
    pub fn remap_json_schema(&self, schema: &mut Value) {
        match schema {
            // Note: this may not really be a JSON ref; we rely on having no collisions between
            // JSON refs and other strings
            Value::Str(s) => self.remap_json_ref(s),
            Value::List(items) => items
                .iter_mut()
                .for_each(|item| self.remap_json_schema(item)),
            Value::Dict(dict) => {
                for (key, value) in dict.iter_mut() {
                    match (key, value) {
                        (Value::Str(k), Value::Str(json_ref)) if k == "$ref" => {
                            self.remap_json_ref(json_ref);
                        }
                        (Value::Str(k), Value::Dict(defs)) if k == "$defs" => {
                            *defs = std::mem::take(defs)
                                .into_iter()
                                .map(|(defs_ref, mut value)| {
                                    self.remap_json_schema(&mut value);
                                    let defs_ref = match defs_ref {
                                        Value::Str(r) => {
                                            Value::Str(self.remap_defs_ref(&r).to_owned())
                                        }
                                        other => other,
                                    };
                                    (defs_ref, value)
                                })
                                .collect();
                        }
                        (_, value) => self.remap_json_schema(value),
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_drop_modules_and_ids() {
        let mut refs = DefsRefs::default();
        assert_eq!(
            refs.get_defs_ref("pkg.mod.Model:123", JsonSchemaMode::Validation),
            "pkg__mod__Model-Input__1"
        );
        assert_eq!(
            refs.prioritized_choices["pkg__mod__Model-Input__1"],
            [
                "Model",
                "Model-Input",
                "pkg__mod__Model",
                "pkg__mod__Model-Input",
                "pkg__mod__Model__1",
                "pkg__mod__Model-Input__1"
            ]
        );
        // generic arguments are components of their own
        refs.get_defs_ref("a.Box[b.Item:1, int]:2", JsonSchemaMode::Serialization);
        assert_eq!(
            refs.prioritized_choices["a__Box_b__Item__int_-Output__1"][0],
            "Box_Item__int_"
        );
    }

    #[test]
    fn the_same_qualified_name_with_another_id_is_another_occurrence() {
        let mut refs = DefsRefs::default();
        let mode = JsonSchemaMode::Validation;
        assert_eq!(refs.get_defs_ref("m.A:1", mode), "m__A-Input__1");
        assert_eq!(refs.get_defs_ref("m.A:2", mode), "m__A-Input__2");
        assert_eq!(refs.get_defs_ref("m.A:1", mode), "m__A-Input__1");
    }

    #[test]
    fn components_keep_their_separators() {
        assert_eq!(
            split_components("a[b,c]"),
            ["a", "[", "b", ",", "c", "]", ""]
        );
        assert_eq!(normalize_name("a.b c<d>"), "a__b_c_d_");
    }
}
