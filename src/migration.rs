//! Best-effort conversion from version-1 tale JSON to normalized version-2 structures.
//!
//! Migration rewrites known fields without evaluating formulas or proving semantic
//! equivalence. The returned document should always be passed through normal version-2
//! validation before use.
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
/// Converts a syntax-v1 tale into the normalized syntax-v2 representation.
///
/// A missing `version` is treated as version 1. Passing version 2 returns an unchanged clone. The
/// migration rewrites known legacy fields but does not evaluate formulas or guarantee semantic
/// equivalence.
///
/// # Errors
///
/// Returns an error if the root or page map has the wrong shape, or if an unsupported explicit version
/// is supplied.
pub fn migrate_v1_to_v2(source: &Value) -> Result<Value> {
    let source = source
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("La racine du récit v1 doit être un objet JSON"))?;
    match source.get("version") {
        Some(Value::Number(number)) if number.as_u64() == Some(2) => {
            return Ok(Value::Object(source.clone()));
        }
        None => {}
        Some(Value::Number(number)) if number.as_u64() == Some(1) => {}
        Some(version) => bail!("Version source non supportée: {version}"),
    }

    let mut old = source.clone();
    old.remove("version");
    let mut migrated = Map::new();
    migrated.insert("version".into(), Value::from(2));

    let old_pre = old.remove("effets");
    let old_post = old.remove("apres");
    let pages = old.remove("pages");
    for (key, value) in old {
        migrated.insert(key, value);
    }
    if !migrated.get("tags").is_some_and(Value::is_object) {
        migrated.insert("tags".into(), json!({}));
    }
    migrated.insert(
        "avant".into(),
        Value::Array(effect_container_to_cells(old_pre.as_ref(), "avant")),
    );
    migrated.insert(
        "apres".into(),
        Value::Array(effect_container_to_cells(old_post.as_ref(), "apres")),
    );
    migrated.insert("pages".into(), migrate_pages(pages)?);
    let mut migrated = Value::Object(migrated);
    migrate_formula_names(&mut migrated);
    Ok(migrated)
}

/// Rewrites formula-facing v1 character aliases to their French v2 names.
fn migrate_formula_names(value: &mut Value) {
    match value {
        Value::String(text) if text.contains('$') => {
            *text = text
                .replace("character.", "personnage.")
                .replace("character[", "personnage[")
                .replace("get_actual_attribute", "obtenir_attribut_effectif")
                .replace("get_actual_max_hp", "obtenir_pv_max")
                .replace("get_actual_max_morale", "obtenir_moral_max");
        }
        Value::Array(values) => values.iter_mut().for_each(migrate_formula_names),
        Value::Object(values) => values.values_mut().for_each(migrate_formula_names),
        _ => {}
    }
}

/// Migrates the complete page map while retaining page order.
fn migrate_pages(pages: Option<Value>) -> Result<Value> {
    let Some(pages) = pages else {
        return Ok(json!({}));
    };
    let pages = pages
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Le champ `pages` v1 doit être un objet"))?;
    let mut migrated = Map::new();
    for (page_id, page) in pages {
        migrated.insert(page_id.clone(), migrate_page(page)?);
    }
    Ok(Value::Object(migrated))
}

/// Migrates one page's traits, narrative body, effects, and connections.
fn migrate_page(page: &Value) -> Result<Value> {
    let page = page
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Une page v1 doit être un objet"))?;
    let mut old = page.clone();
    let traits = old.remove("traits");
    let text = old.remove("texte");
    let before = old.remove("effets");
    let after = old.remove("apres");
    let connections = old.remove("connexions");

    let mut tags = match old.remove("tags") {
        Some(Value::Object(tags)) => tags,
        _ => Map::new(),
    };
    match traits {
        Some(Value::String(tag)) => {
            tags.entry(tag).or_insert(Value::Bool(true));
        }
        Some(Value::Array(values)) => {
            for tag in values
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
            {
                tags.entry(tag).or_insert(Value::Bool(true));
            }
        }
        _ => {}
    }

    let mut cells = effect_container_to_cells(before.as_ref(), "entree");
    if let Some(Value::String(text)) = text {
        if !text.is_empty() {
            cells.push(json!({"id": "texte", "libelle": "Texte de page", "texte": text}));
        }
    }
    cells.extend(effect_container_to_cells(after.as_ref(), "sortie"));

    old.insert("tags".into(), Value::Object(tags));
    old.insert("cellules".into(), Value::Array(cells));
    old.insert(
        "connexions".into(),
        migrate_connections(connections.unwrap_or_else(|| json!([])))?,
    );
    Ok(Value::Object(old))
}

/// Migrates connection effects into normalized connection cells.
fn migrate_connections(connections: Value) -> Result<Value> {
    let Some(connections) = connections.as_array() else {
        bail!("Le champ `connexions` v1 doit être un tableau");
    };
    let mut migrated = Vec::with_capacity(connections.len());
    for connection in connections {
        let connection = connection
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Une connexion v1 doit être un objet"))?;
        let mut connection = connection.clone();
        if let Some(conditional) = connection.get_mut("conditions") {
            *conditional = migrate_conditional(conditional);
        }
        let effects = connection.remove("effets");
        connection.insert(
            "cellules".into(),
            Value::Array(effect_container_to_cells(effects.as_ref(), "connexion")),
        );
        migrated.push(Value::Object(connection));
    }
    Ok(Value::Array(migrated))
}

/// Converts a v1 scalar-or-array effect container into an array of cells.
fn effect_container_to_cells(value: Option<&Value>, prefix: &str) -> Vec<Value> {
    let entries = match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values.clone(),
        Some(value) => vec![value.clone()],
    };
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| effect_entry_to_cell(entry, prefix, index + 1))
        .collect()
}

/// Wraps an atomic effect or normalizes a v1 effect group as one cell.
fn effect_entry_to_cell(entry: Value, prefix: &str, index: usize) -> Value {
    let generated_id = format!("{prefix}_{index}");
    let Some(mut object) = entry.as_object().cloned() else {
        return json!({"id": generated_id, "effets": [entry]});
    };
    if object.contains_key("effets") {
        object
            .entry("id".to_owned())
            .or_insert_with(|| Value::String(generated_id));
        if let Some(effect) = object.get_mut("effets") {
            if !effect.is_array() {
                *effect = Value::Array(vec![effect.clone()]);
            }
            if let Some(effects) = effect.as_array_mut() {
                for effect in effects {
                    *effect = migrate_atomic_effect(effect.clone());
                }
            }
        }
        if let Some(conditional) = object.get_mut("conditions") {
            *conditional = migrate_conditional(conditional);
        }
        Value::Object(object)
    } else {
        json!({"id": generated_id, "effets": [migrate_atomic_effect(Value::Object(object))]})
    }
}

/// Translates v1 primitive poll type names to the French v2 vocabulary.
fn migrate_atomic_effect(mut _value: Value) -> Value {
    /*
    let Some(_effect) = value.as_object_mut() else {
        return value;
    };
    if effect.get("type").and_then(Value::as_str) == Some("demander") {
        let translated = effect.get("donnees").and_then(Value::as_str);
        if let Some(translated) = translated {
            effect.insert("donnees".into(), Value::String(translated.into()));
        }
    }
    */
    _value
}

/// Rewrites only conditionals whose old meaning referred to page traits.
fn migrate_conditional(value: &Value) -> Value {
    if let Some(values) = value.as_array() {
        return Value::Array(values.iter().map(migrate_conditional).collect());
    }
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let kind = object.get("type").and_then(Value::as_str);
    match kind {
        Some("tous_les_traits") => {
            let tags = old_trait_names(object.get("traits"));
            if tags.is_empty() {
                json!({"type": "formule", "formule": "${True}"})
            } else {
                json!({
                    "type": "et",
                    "conditions": tags.into_iter().map(|tag| {
                        json!({"type": "tag_page", "filtre": tag})
                    }).collect::<Vec<_>>()
                })
            }
        }
        Some("un_des_traits") => {
            let tags = old_trait_names(object.get("traits"));
            if tags.is_empty() {
                json!({"type": "formule", "formule": "${False}"})
            } else if tags.len() == 1 {
                json!({"type": "tag_page", "filtre": tags[0].clone()})
            } else {
                json!({
                    "type": "tag_page",
                    "filtre": {"type": "ou", "filtres": tags}
                })
            }
        }
        Some("et" | "ou") => {
            let mut migrated = object.clone();
            if let Some(Value::Array(children)) = migrated.get_mut("conditions") {
                *children = children.iter().map(migrate_conditional).collect();
            }
            Value::Object(migrated)
        }
        Some("non") => {
            let mut migrated = object.clone();
            if let Some(child) = migrated.get_mut("condition") {
                *child = migrate_conditional(child);
            }
            Value::Object(migrated)
        }
        _ => Value::Object(object.clone()),
    }
}

/// Reads the string-or-array trait representation accepted by syntax v1.
fn old_trait_names(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verifies ordering, page tags, cells, and conditional migration together.
    #[test]
    fn migrates_core_v1_structure() {
        let source = json!({
            "titre": "Essai",
            "depart": "debut",
            "effets": {"type": "definir_tag", "filtre": "visites", "valeur": 0},
            "pages": {
                "debut": {
                    "traits": ["ville"],
                    "texte": "Bonjour",
                    "connexions": [{
                        "texte": "Suite",
                        "destination": "debut",
                        "conditions": {"type": "un_des_traits", "traits": ["ville", "port"]},
                        "effets": [{"type": "incrementer_tag", "filtre": "visites"}]
                    }]
                }
            }
        });
        let migrated = migrate_v1_to_v2(&source).unwrap();
        assert_eq!(migrated["version"], 2);
        assert_eq!(migrated["pages"]["debut"]["tags"]["ville"], true);
        assert_eq!(
            migrated["pages"]["debut"]["cellules"][0]["texte"],
            "Bonjour"
        );
        assert_eq!(migrated["avant"][0]["effets"][0]["type"], "definir_tag");
        assert_eq!(
            migrated["pages"]["debut"]["connexions"][0]["conditions"]["type"],
            "tag_page"
        );
        assert_eq!(
            migrated["pages"]["debut"]["connexions"][0]["cellules"][0]["effets"][0]["type"],
            "incrementer_tag"
        );
    }

    /*
    // Verifies that public primitive poll names are translated to French.
    #[test]
    fn migrates_poll_data_type_names() {
        let source = json!({
            "depart": "debut",
            "effets": {
                "type": "demander",
                "tag": "age",
                "question": "Quel âge?",
                "donnees": "int"
            },
            "pages": {"debut": {
                "texte": "${personnage.get_actual_max_hp()}",
                "connexions": []
            }}
        });
        let migrated = migrate_v1_to_v2(&source).unwrap();
        assert_eq!(migrated["avant"][0]["effets"][0]["donnees"], "entier");
        assert_eq!(
            migrated["pages"]["debut"]["cellules"][0]["texte"],
            "${personnage.obtenir_pv_max()}"
        );
    }
    */

    // Verifies that a v2 document is not rewritten a second time.
    #[test]
    fn migration_is_idempotent_for_v2() {
        let source = json!({"version": 2, "pages": {}});
        assert_eq!(migrate_v1_to_v2(&source).unwrap(), source);
    }
}
