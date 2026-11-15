//! Raw tale-document editing operations used by Forge.
//!
//! `TaleDocument` preserves the editable JSON tree and tracks unsaved changes. Rename
//! helpers rewrite explicit static references only; identifiers embedded in formulas or
//! generated dynamically cannot be discovered safely and remain author-managed.
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use crate::{parse_tale_text};
use formulas::has_formula_substitution;

/// Wraps parsed tale data while tracking its file path and dirty state.
#[derive(Debug, Clone)]
pub struct TaleDocument {
    pub path: PathBuf,
    pub root: Value,
    pub dirty: bool,
}

impl TaleDocument {
    /// Loads an existing JSON/JSON5 document, or creates a minimal dirty document when the path does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the existing file cannot be read or parsed.
    pub fn load_or_new(path: PathBuf) -> Result<Self> {
        if path.exists() {
            let text = fs::read_to_string(&path)?;
            let root: Value = parse_tale_text(&text)
                .with_context(|| format!("JSON ou JSON5 invalide dans {}", path.display()))?;
            Ok(Self {
                path,
                root,
                dirty: false,
            })
        } else {
            Ok(Self {
                path,
                root: minimal_tale(),
                dirty: true,
            })
        }
    }
    /// Writes the raw tale as normalized, pretty-printed strict JSON using a temporary file and rename.
    ///
    /// The method creates missing parent directories and clears `dirty` only after a successful write. JSON5 comments and original formatting are not preserved.
    ///
    /// # Errors
    ///
    /// Returns an error on serialization or filesystem failure.
    pub fn save(&mut self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let temp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&self.root)? + "\n";
        fs::write(&temp, text)?;
        if self.path.exists() {
            #[cfg(windows)]
            fs::remove_file(&self.path)?;
        }
        fs::rename(&temp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
    /// Returns the tale title, or `"Récit sans titre"` when the raw field is missing or not a string.
    pub fn title(&self) -> &str {
        self.root
            .get("titre")
            .and_then(Value::as_str)
            .unwrap_or("Récit sans titre")
    }
    /// Returns the raw page object, or `None` when `pages` is absent or not an object.
    pub fn pages(&self) -> Option<&Map<String, Value>> {
        self.root.get("pages")?.as_object()
    }

    /// Returns the mutable raw page map, creating it when necessary.
    pub fn pages_mut(&mut self) -> &mut Map<String, Value> {
        if !self.root.is_object() {
            self.root = json!({});
        }
        let root = self.root.as_object_mut().expect("root object");
        if !root.get("pages").is_some_and(Value::is_object) {
            root.insert("pages".into(), json!({}));
        }
        root.get_mut("pages")
            .and_then(Value::as_object_mut)
            .expect("pages object")
    }

    /// Returns the page IDs in their document order.
    pub fn page_ids(&self) -> Vec<String> {
        self.pages()
            .map(|pages| pages.keys().cloned().collect())
            .unwrap_or_default()
    }
    /// Returns the raw definition of page `id`, or `None` if the page map or ID is absent.
    pub fn page(&self, id: &str) -> Option<&Value> {
        self.pages()?.get(id)
    }
    /// Returns the mutable definition of page `id`, creating the top-level page map if necessary.
    ///
    /// Returns `None` when the requested ID does not exist.
    pub fn page_mut(&mut self, id: &str) -> Option<&mut Value> {
        self.pages_mut().get_mut(id)
    }
    /// Inserts a minimal page under `id`.
    ///
    /// Returns `false` without modifying the document when the ID already exists; otherwise marks the document dirty and returns `true`.
    pub fn add_page(&mut self, id: String) -> bool {
        if self.pages_mut().contains_key(&id) {
            return false;
        }
        self.pages_mut().insert(
            id,
            json!({
                "titre": "Nouvelle page",
                "tags": {},
                "cellules": [
                    {"id": "texte", "libelle": "Texte principal", "texte": ""}
                ],
                "connexions": []
            }),
        );
        self.dirty = true;
        true
    }
    /// Removes page `id`.
    ///
    /// Returns whether a page was removed and marks the document dirty only on success.
    pub fn delete_page(&mut self, id: &str) -> bool {
        let removed = self.pages_mut().remove(id).is_some();
        if removed {
            self.dirty = true;
        }
        removed
    }
    /// Copies `source` under `new_id` and marks the document dirty.
    ///
    /// # Errors
    ///
    /// Returns an error when the source is missing or the destination ID already exists.
    pub fn duplicate_page(&mut self, source: &str, new_id: String) -> Result<()> {
        if self.pages_mut().contains_key(&new_id) {
            anyhow::bail!("Une page nommée '{new_id}' existe déjà");
        }
        let Some(page) = self.page(source).cloned() else {
            anyhow::bail!("Page '{source}' introuvable");
        };
        self.pages_mut().insert(new_id, page);
        self.dirty = true;
        Ok(())
    }
    /// Renames a page and rewrites exact static page references throughout the document.
    ///
    /// Formula text and dynamically generated identifiers are deliberately left unchanged. A no-op rename succeeds without marking the document dirty.
    ///
    /// # Errors
    ///
    /// Returns an error when `old` is missing or `new` is already in use.
    pub fn rename_page(&mut self, old: &str, new: &str) -> Result<()> {
        if old == new {
            return Ok(());
        }
        if self.pages_mut().contains_key(new) {
            anyhow::bail!("Une page nommée '{new}' existe déjà");
        }
        let Some(page) = self.pages_mut().remove(old) else {
            anyhow::bail!("Page '{old}' introuvable");
        };
        self.pages_mut().insert(new.to_string(), page);
        update_page_references(&mut self.root, old, new);
        self.dirty = true;
        Ok(())
    }
    /// Returns human-readable paths for explicit static references to `target`.
    ///
    /// Formula-generated references cannot be enumerated and are omitted.
    pub fn incoming_references(&self, target: &str) -> Vec<String> {
        let mut refs = Vec::new();
        if self.root.get("depart").and_then(Value::as_str) == Some(target) {
            refs.push("depart".into());
        }
        if let Some(pages) = self.pages() {
            for (page_id, page) in pages {
                collect_connection_page_refs(
                    page.get("connexions"),
                    target,
                    &format!("pages.{page_id}.connexions"),
                    &mut refs,
                );
                collect_page_refs(
                    page.get("cellules"),
                    target,
                    &format!("pages.{page_id}.cellules"),
                    &mut refs,
                );
            }
        }
        collect_page_refs(self.root.get("avant"), target, "avant", &mut refs);
        collect_page_refs(self.root.get("apres"), target, "apres", &mut refs);
        if let Some(states) = self.root.get("etats").and_then(Value::as_object) {
            for (id, state) in states {
                if state.get("defaite").and_then(Value::as_str) == Some(target) {
                    refs.push(format!("etats.{id}.defaite"));
                }
                collect_page_refs(
                    state.get("avant"),
                    target,
                    &format!("etats.{id}.avant"),
                    &mut refs,
                );
                collect_page_refs(
                    state.get("apres"),
                    target,
                    &format!("etats.{id}.apres"),
                    &mut refs,
                );
            }
        }
        refs
    }

    /// Returns the IDs stored in the requested top-level entity collection.
    pub fn entity_ids(&self, key: &str) -> Vec<String> {
        self.root
            .get(key)
            .and_then(Value::as_object)
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    }
    /// Returns entity `id` from top-level object `key`, or `None` if the collection or entity is absent.
    pub fn entity(&self, key: &str, id: &str) -> Option<&Value> {
        self.root.get(key)?.as_object()?.get(id)
    }
    /// Returns mutable entity `id` from top-level collection `key`.
    ///
    /// The collection is created when missing; `None` is returned when the ID itself is absent.
    pub fn entity_mut(&mut self, key: &str, id: &str) -> Option<&mut Value> {
        ensure_object_key(&mut self.root, key).get_mut(id)
    }
    /// Inserts `value` into top-level collection `key` under `id`.
    ///
    /// Returns `false` if the ID already exists; otherwise marks the document dirty and returns `true`.
    pub fn add_entity(&mut self, key: &str, id: String, value: Value) -> bool {
        let map = ensure_object_key(&mut self.root, key);
        if map.contains_key(&id) {
            return false;
        }
        map.insert(id, value);
        self.dirty = true;
        true
    }
    /// Removes entity `id` from top-level collection `key`.
    ///
    /// Returns whether an entity was removed and marks the document dirty only on success.
    pub fn delete_entity(&mut self, key: &str, id: &str) -> bool {
        let removed = ensure_object_key(&mut self.root, key).remove(id).is_some();
        if removed {
            self.dirty = true;
        }
        removed
    }
    /// Copies an entity inside top-level collection `key`.
    ///
    /// # Errors
    ///
    /// Returns an error when `source` is missing or `new_id` already exists.
    pub fn duplicate_entity(&mut self, key: &str, source: &str, new_id: String) -> Result<()> {
        let map = ensure_object_key(&mut self.root, key);
        if map.contains_key(&new_id) {
            anyhow::bail!("L'identifiant '{new_id}' existe déjà");
        }
        let Some(value) = map.get(source).cloned() else {
            anyhow::bail!("Identifiant '{source}' introuvable");
        };
        map.insert(new_id, value);
        self.dirty = true;
        Ok(())
    }
    /// Renames an item, state, or image definition and rewrites supported exact static references.
    ///
    /// Formula text and dynamically generated identifiers are not rewritten.
    ///
    /// # Errors
    ///
    /// Returns an error when `old` is missing or `new` already exists.
    pub fn rename_entity(&mut self, key: &str, old: &str, new: &str) -> Result<()> {
        let map = ensure_object_key(&mut self.root, key);
        if map.contains_key(new) {
            anyhow::bail!("L'identifiant '{new}' existe déjà");
        }
        let Some(value) = map.remove(old) else {
            anyhow::bail!("Identifiant '{old}' introuvable");
        };
        map.insert(new.to_owned(), value);
        update_entity_references(&mut self.root, key, old, new);
        self.dirty = true;
        Ok(())
    }

    /// Returns the configured character attribute names.
    pub fn attribute_names(&self) -> Vec<String> {
        self.root
            .get("attributs")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_else(|| {
                [
                    "force",
                    "agilite",
                    "endurance",
                    "presence",
                    "chance",
                    "connaissance",
                    "perception",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect()
            })
    }

    /// Collects tag IDs that can be inferred from the raw tale.
    pub fn known_tags(&self) -> BTreeSet<String> {
        let mut tags = BTreeSet::new();
        collect_tag_ids(&self.root, &mut tags);
        tags
    }

    /// Marks the raw tale document as containing unsaved changes.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

/// Creates the smallest valid raw tale document used for a new file.
pub fn minimal_tale() -> Value {
    json!({
        "version": 2,
        "titre": "Nouveau récit",
        "auteur": "",
        "synopsis": "",
        "depart": "debut",
        "images": {},
        "objets": {},
        "etats": {},
        "global": {},
        "avant": [],
        "apres": [],
        "pages": {
            "debut": {
                "titre": "Début",
                "tags": {},
                "cellules": [{
                    "id": "texte",
                    "libelle": "Texte principal",
                    "texte": "Commencez votre récit ici."
                }],
                "connexions": []
            }
        }
    })
}

/// Ensures that a top-level field exists as a JSON object and returns its mutable map.
fn ensure_object_key<'a>(root: &'a mut Value, key: &str) -> &'a mut Map<String, Value> {
    if !root.is_object() {
        *root = json!({});
    }
    let object = root.as_object_mut().expect("root object");
    if !object.get(key).is_some_and(Value::is_object) {
        object.insert(key.to_owned(), json!({}));
    }
    object
        .get_mut(key)
        .and_then(Value::as_object_mut)
        .expect("object key")
}

/// Rewrites explicit static references to a renamed page.
///
/// Only exact static references are rewritten. Formula text and dynamically generated
/// identifiers are deliberately left unchanged.
fn update_page_references(root: &mut Value, old: &str, new: &str) {
    if let Some(value) = root.get_mut("depart") {
        replace_string(value, old, new);
    }
    if let Some(states) = root.get_mut("etats").and_then(Value::as_object_mut) {
        for state in states.values_mut() {
            if let Some(value) = state.get_mut("defaite") {
                replace_string(value, old, new);
            }
            update_effect_page_refs(state.get_mut("avant"), old, new);
            update_effect_page_refs(state.get_mut("apres"), old, new);
        }
    }
    update_effect_page_refs(root.get_mut("avant"), old, new);
    update_effect_page_refs(root.get_mut("apres"), old, new);
    if let Some(pages) = root.get_mut("pages").and_then(Value::as_object_mut) {
        for page in pages.values_mut() {
            update_effect_page_refs(page.get_mut("cellules"), old, new);
            update_connection_page_refs(page.get_mut("connexions"), old, new);
        }
    }
}

/// Rewrites static page references in a unified literal/formula connection container.
///
/// Formula strings are closures and are deliberately left untouched. A singleton literal
/// object and arrays mixing objects with formulas are both supported.
fn update_connection_page_refs(value: Option<&mut Value>, old: &str, new: &str) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Array(values) => {
            for value in values {
                update_connection_page_refs(Some(value), old, new);
            }
        }
        Value::Object(connection) => {
            if let Some(value) = connection.get_mut("destination") {
                replace_string(value, old, new);
            }
            if let Some(value) = connection
                .get_mut("test")
                .and_then(|test| test.get_mut("echec"))
            {
                replace_string(value, old, new);
            }
            update_effect_page_refs(connection.get_mut("cellules"), old, new);
        }
        _ => {}
    }
}

/// Rewrites static defeat and redirect destinations inside a cell container.
///
/// Formula destinations are deliberately left unchanged.
fn update_effect_page_refs(effects: Option<&mut Value>, old: &str, new: &str) {
    let Some(effects) = effects else { return };
    match effects {
        Value::Array(values) => {
            for value in values {
                update_effect_entry_refs(value, old, new);
            }
        }
        Value::Object(_) => update_effect_entry_refs(effects, old, new),
        _ => {}
    }
}

/// Rewrites static page references contained in one cell or effect object.
///
/// Formula destinations are deliberately left unchanged.
fn update_effect_entry_refs(entry: &mut Value, old: &str, new: &str) {
    if let Some(object) = entry.as_object_mut() {
        if let Some(defeat) = object.get_mut("defaite") {
            replace_string(defeat, old, new);
        }
        if object.get("type").and_then(Value::as_str) == Some("rediriger") {
            if let Some(destination) = object.get_mut("destination") {
                replace_string(destination, old, new);
            }
        }
        if let Some(inner) = object.get_mut("effets") {
            update_effect_page_refs(Some(inner), old, new);
        }
    }
}

/// Replaces a JSON string only when it exactly matches the old identifier.
///
/// Only exact static references are rewritten. Formula text and dynamically generated
/// identifiers are deliberately left unchanged.
fn replace_string(value: &mut Value, old: &str, new: &str) {
    if value.as_str() == Some(old) {
        *value = Value::String(new.to_owned());
    }
}

/// Rewrites explicit static references to a renamed entity.
///
/// Only exact static references are rewritten. Formula text and dynamically generated
/// identifiers are deliberately left unchanged.
fn update_entity_references(root: &mut Value, key: &str, old: &str, new: &str) {
    if key == "images" {
        update_image_references(root, old, new);
        return;
    }
    visit_objects_mut(root, &mut |object| {
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let kind = kind.as_deref();
        match key {
            "objets" => {
                if kind == Some("ajouter_objet") {
                    if let Some(value) = object.get_mut("objet") {
                        replace_string(value, old, new);
                    }
                }
                if matches!(
                    kind,
                    Some("retirer_objet" | "modifier_objet" | "objet_requis")
                ) {
                    replace_id_filter(object.get_mut("filtre"), old, new, "id");
                }
            }
            "etats" => {
                if kind == Some("ajouter_etat") {
                    if let Some(value) = object.get_mut("etat") {
                        replace_string(value, old, new);
                    }
                }
                if matches!(kind, Some("retirer_etat" | "modifier_etat" | "etat_requis")) {
                    replace_id_filter(object.get_mut("filtre"), old, new, "id");
                }
            }
            _ => {}
        }
    });

    if key == "etats" {
        if let Some(states) = root.get_mut("etats").and_then(Value::as_object_mut) {
            for state in states.values_mut() {
                replace_id_filter(state.get_mut("expiration"), old, new, "id");
            }
        }
    }
}

/// Rewrites static cover and cell image references in supported tale structures.
fn update_image_references(root: &mut Value, old: &str, new: &str) {
    if let Some(value) = root.get_mut("image") {
        replace_static_image_reference(value, old, new);
    }
    update_cell_image_references(root.get_mut("avant"), old, new);
    update_cell_image_references(root.get_mut("apres"), old, new);
    if let Some(states) = root.get_mut("etats").and_then(Value::as_object_mut) {
        for state in states.values_mut() {
            update_cell_image_references(state.get_mut("avant"), old, new);
            update_cell_image_references(state.get_mut("apres"), old, new);
        }
    }
    if let Some(pages) = root.get_mut("pages").and_then(Value::as_object_mut) {
        for page in pages.values_mut() {
            update_cell_image_references(page.get_mut("cellules"), old, new);
            update_connection_image_references(page.get_mut("connexions"), old, new);
        }
    }
}

/// Rewrites static references in a literal cell or cell list, ignoring formula cells.
fn update_cell_image_references(value: Option<&mut Value>, old: &str, new: &str) {
    let Some(value) = value else { return };
    match value {
        Value::Array(cells) => {
            for cell in cells {
                update_cell_image_references(Some(cell), old, new);
            }
        }
        Value::Object(cell) => {
            if let Some(images) = cell.get_mut("images") {
                replace_static_image_reference(images, old, new);
            }
        }
        _ => {}
    }
}

/// Rewrites image references in cells belonging to literal connections.
fn update_connection_image_references(value: Option<&mut Value>, old: &str, new: &str) {
    let Some(value) = value else { return };
    match value {
        Value::Array(connections) => {
            for connection in connections {
                update_connection_image_references(Some(connection), old, new);
            }
        }
        Value::Object(connection) => {
            update_cell_image_references(connection.get_mut("cellules"), old, new);
        }
        _ => {}
    }
}

/// Rewrites exact image IDs in a literal string or list while preserving formulas.
fn replace_static_image_reference(value: &mut Value, old: &str, new: &str) {
    match value {
        Value::String(reference) if reference == old => *reference = new.to_owned(),
        Value::Array(references) => {
            for reference in references {
                replace_static_image_reference(reference, old, new);
            }
        }
        _ => {}
    }
}

/// Visits every nested JSON object in the document mutably.
///
/// Traverses supported static structures and appends matches to the supplied collection;
/// references generated only by formulas cannot be enumerated.
fn visit_objects_mut(value: &mut Value, f: &mut impl FnMut(&mut Map<String, Value>)) {
    match value {
        Value::Object(object) => {
            f(object);
            for child in object.values_mut() {
                visit_objects_mut(child, f);
            }
        }
        Value::Array(array) => {
            for child in array {
                visit_objects_mut(child, f);
            }
        }
        _ => {}
    }
}
/// Rewrites an exact ID in a string shorthand or simple ID-filter object.
///
/// Compound filters and formula strings are left unchanged.
fn replace_id_filter(filter: Option<&mut Value>, old: &str, new: &str, key: &str) {
    let Some(filter) = filter else { return };
    match filter {
        Value::String(value) if value == old => *value = new.to_owned(),
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("id") {
                if let Some(value) = object.get_mut(key) {
                    replace_string(value, old, new);
                }
            }
            for nested_key in ["filtre", "filtres"] {
                if let Some(nested) = object.get_mut(nested_key) {
                    replace_id_filter(Some(nested), old, new, key);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                replace_id_filter(Some(value), old, new, key);
            }
        }
        _ => {}
    }
}

/// Traverses a cell container and appends paths whose static defeat or redirect destination equals `target`.
///
/// Formula destinations cannot be enumerated.
fn collect_page_refs(effects: Option<&Value>, target: &str, path: &str, output: &mut Vec<String>) {
    let Some(effects) = effects else { return };
    match effects {
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_effect_page_ref(value, target, &format!("{path}[{index}]"), output);
            }
        }
        Value::Object(_) => collect_effect_page_ref(effects, target, path, output),
        _ => {}
    }
}

/// Collects static references from literal connections while ignoring formula entries.
fn collect_connection_page_refs(
    value: Option<&Value>,
    target: &str,
    path: &str,
    output: &mut Vec<String>,
) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_connection_page_refs(
                    Some(value),
                    target,
                    &format!("{path}[{index}]"),
                    output,
                );
            }
        }
        Value::Object(connection) => {
            if connection.get("destination").and_then(Value::as_str) == Some(target) {
                output.push(format!("{path}.destination"));
            }
            if connection
                .get("test")
                .and_then(|test| test.get("echec"))
                .and_then(Value::as_str)
                == Some(target)
            {
                output.push(format!("{path}.test.echec"));
            }
            collect_page_refs(
                connection.get("cellules"),
                target,
                &format!("{path}.cellules"),
                output,
            );
        }
        _ => {}
    }
}

/// Appends matching defeat and redirect destinations from one cell/effect object.
///
/// Formula-generated references cannot be enumerated.
fn collect_effect_page_ref(value: &Value, target: &str, path: &str, output: &mut Vec<String>) {
    if let Some(object) = value.as_object() {
        if object.get("defaite").and_then(Value::as_str) == Some(target) {
            output.push(format!("{path}.defaite"));
        }
        if object.get("type").and_then(Value::as_str) == Some("rediriger")
            && object.get("destination").and_then(Value::as_str) == Some(target)
        {
            output.push(format!("{path}.destination"));
        }
        collect_page_refs(
            object.get("effets"),
            target,
            &format!("{path}.effets"),
            output,
        );
    }
}
/// Recursively collects tag IDs referenced by supported tag effects and required-tag conditionals.
///
/// Only statically recognizable single-ID filters are included.
fn collect_tag_ids(value: &Value, output: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            let effect_type = object.get("type").and_then(Value::as_str);
            if matches!(effect_type, Some("definir_tag" | "incrementer_tag")) {
                if let Some(filter) = object.get("filtre") {
                    if let Some(id) = single_id_filter(filter, "tag") {
                        output.insert(id.to_owned());
                    }
                }
            }
            if effect_type == Some("tag_requis") {
                if let Some(filter) = object.get("filtre") {
                    if let Some(id) = single_id_filter(filter, "tag") {
                        output.insert(id.to_owned());
                    }
                }
            }
            for child in object.values() {
                collect_tag_ids(child, output);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_tag_ids(value, output);
            }
        }
        _ => {}
    }
}
/// Returns the ID encoded by a string shorthand or a simple `{type: "id"}` filter.
///
/// Compound filters and malformed values return `None`.
pub fn single_id_filter<'a>(filter: &'a Value, id_key: &str) -> Option<&'a str> {
    if let Some(value) = filter.as_str() {
        return Some(value);
    }
    let object = filter.as_object()?;
    if object.get("type").and_then(Value::as_str) != Some("id") {
        return None;
    }
    object.get(id_key)?.as_str()
}
/// Collects explicit static page IDs referenced by the start page, transitions, failures, defeat effects, and redirects.
///
/// Formula destinations are omitted because their values are unknown until runtime.
pub fn all_page_references(root: &Value) -> HashSet<String> {
    let mut refs = HashSet::new();
    if let Some(start) = root.get("depart").and_then(Value::as_str) {
        refs.insert(start.to_owned());
    }
    collect_static_page_references(root, &mut refs);
    refs
}

/// Recursively collects literal page destinations while ignoring formula strings.
fn collect_static_page_references(value: &Value, refs: &mut HashSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(kind) = object.get("type").and_then(Value::as_str) {
                if kind == "rediriger" {
                    insert_static_page_ref(object.get("destination"), refs);
                }
            }
            if object.contains_key("destination") && object.contains_key("texte") {
                insert_static_page_ref(object.get("destination"), refs);
            }
            insert_static_page_ref(object.get("test").and_then(|test| test.get("echec")), refs);
            insert_static_page_ref(object.get("defaite"), refs);
            for child in object.values() {
                collect_static_page_references(child, refs);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_static_page_references(child, refs);
            }
        }
        _ => {}
    }
}

/// Inserts a literal page ID, excluding formulas and non-string values.
fn insert_static_page_ref(value: Option<&Value>, refs: &mut HashSet<String>) {
    if let Some(id) = value.and_then(Value::as_str) {
        if !has_formula_substitution(id) {
            refs.insert(id.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TaleDocument;
    use serde_json::json;
    use std::path::PathBuf;

    /// Verifies that image renames update static cover and cell references only.
    #[test]
    fn image_rename_updates_static_references_and_preserves_formulas() {
        let mut document = TaleDocument {
            path: PathBuf::from("test.rct"),
            root: json!({
                "images": {"ancienne": {"type": "chemin", "chemin": "vue.png"}},
                "image": ["ancienne"],
                "global": {"images": ["ancienne"]},
                "pages": {
                    "debut": {
                        "cellules": [{"images": ["ancienne", "${image_choisie}"]}]
                    }
                }
            }),
            dirty: false,
        };

        document
            .rename_entity("images", "ancienne", "nouvelle")
            .unwrap();

        assert!(document.entity("images", "ancienne").is_none());
        assert!(document.entity("images", "nouvelle").is_some());
        assert_eq!(document.root["image"], json!(["nouvelle"]));
        assert_eq!(
            document.root["pages"]["debut"]["cellules"][0]["images"],
            json!(["nouvelle", "${image_choisie}"])
        );
        assert_eq!(document.root["global"]["images"], json!(["ancienne"]));
        assert!(document.dirty);
    }
}
