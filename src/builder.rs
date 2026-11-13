//! Guided Forge editors for tale metadata, pages, cells, effects, filters, and polls.
//!
//! This module edits raw JSON rather than the typed runtime model so unknown fields can
//! survive round-trips. Each editor normalizes only the structure it owns; prompt
//! cancellation and invalid typed input are returned through `anyhow::Result`.
use anyhow::anyhow;
use anyhow::Result;
use serde_json::{json, Map, Number, Value};

use crate::validation::is_formula;
use crate::{
    document::TaleDocument,
    forge_ops,
    images::SUPPORTED_FORMATS,
    prompts,
    schema::{self, FilterFamily, CONDITIONAL_TYPES, CONDITION_EFFECT_TYPES, EFFECT_TYPES},
    terminal::AppTerminal,
};

/// Opens the guided editor for root tale metadata.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn edit_tale_metadata(terminal: &mut AppTerminal, document: &mut TaleDocument) -> Result<()> {
    let options = vec![
        ("titre".into(), "Titre".into()),
        ("auteur".into(), "Auteur".into()),
        ("synopsis".into(), "Synopsis".into()),
        ("images".into(), "Bibliothèque d'images".into()),
        ("image".into(), "Images affichées avant le synopsis".into()),
        ("depart".into(), "Page de départ".into()),
        ("attributs".into(), "Attributs personnalisés".into()),
        ("global".into(), "Constantes et fonctions globales".into()),
        ("avant".into(), "Cellules globales avant".into()),
        ("apres".into(), "Cellules globales après".into()),
        (
            "transfer".into(),
            "Déplacer ou copier entre les deux phases".into(),
        ),
        ("json".into(), "Éditer la racine en JSON".into()),
    ];
    let Some(choice) = prompts::select(terminal, "Modifier le récit", &options, true)? else {
        return Ok(());
    };
    match options[choice].0.as_str() {
        "titre" | "auteur" => {
            let key = options[choice].0.as_str();
            let current = document.root.get(key).and_then(Value::as_str).unwrap_or("");
            let value = prompts::prompt_line(terminal, &options[choice].1, Some(current))?;
            root_object_mut(document).insert(key.into(), Value::String(value));
            document.mark_dirty();
        }
        "synopsis" => {
            let current = document
                .root
                .get("synopsis")
                .and_then(Value::as_str)
                .unwrap_or("");
            let value = prompts::edit_multiline(terminal, "Synopsis", current)?;
            root_object_mut(document).insert("synopsis".into(), Value::String(value));
            document.mark_dirty();
        }
        "images" => manage_image_definitions(terminal, document)?,
        "image" => {
            let mut images = document
                .root
                .get("image")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if manage_image_references(
                terminal,
                document,
                &mut images,
                "Images affichées avant le synopsis",
                false,
            )? {
                root_object_mut(document).insert("image".into(), images);
                document.mark_dirty();
            }
        }
        "depart" => {
            if let Some(id) = choose_page(terminal, document, "Choisir la page de départ", false)?
            {
                root_object_mut(document).insert("depart".into(), Value::String(id));
                document.mark_dirty();
            }
        }
        "attributs" => {
            let current = document.attribute_names();
            let values = prompts::prompt_string_list(terminal, "Attributs", &current)?;
            root_object_mut(document).insert(
                "attributs".into(),
                Value::Array(values.into_iter().map(Value::String).collect()),
            );
            document.mark_dirty();
        }
        "global" => {
            let mut global = document
                .root
                .get("global")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if manage_json_object(
                terminal,
                "Constantes et fonctions globales",
                &mut global,
                false,
            )? {
                root_object_mut(document).insert("global".into(), global);
                document.mark_dirty();
            }
        }
        "avant" => {
            let mut cells = document
                .root
                .get("avant")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if manage_cells(terminal, document, &mut cells, "Cellules globales avant")? {
                root_object_mut(document).insert("avant".into(), cells);
                document.mark_dirty();
            }
        }
        "apres" => {
            let mut cells = document
                .root
                .get("apres")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if manage_cells(terminal, document, &mut cells, "Cellules globales après")? {
                root_object_mut(document).insert("apres".into(), cells);
                document.mark_dirty();
            }
        }
        "transfer" => {
            let mut pre = document
                .root
                .get("avant")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let mut post = document
                .root
                .get("apres")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if transfer_cell_phase(terminal, &mut pre, &mut post)? {
                let root = root_object_mut(document);
                root.insert("avant".into(), pre);
                root.insert("apres".into(), post);
                document.mark_dirty();
            }
        }
        "json" => {
            if let Some(value) = prompts::edit_json(terminal, "Racine du récit", &document.root)? {
                document.root = value;
                document.mark_dirty();
            }
        }
        _ => {}
    }
    Ok(())
}

/// Opens the guided editor for one page.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
/// - `page_id` — Identifier of the page being read, edited, or visited.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn edit_page(
    terminal: &mut AppTerminal,
    document: &mut TaleDocument,
    page_id: &str,
) -> Result<()> {
    let options = vec![
        ("titre".into(), "Titre".into()),
        (
            "tags".into(),
            "Tags de page (formules évaluées une fois)".into(),
        ),
        ("cellules".into(), "Cellules".into()),
        (
            "connexions".into(),
            "Connexions (objets ou formules)".into(),
        ),
        ("json".into(), "Éditer la page en JSON".into()),
    ];
    let Some(choice) = prompts::select(
        terminal,
        &format!("Modifier la page `{page_id}`"),
        &options,
        true,
    )?
    else {
        return Ok(());
    };
    let page_snapshot = document.page(page_id).cloned().unwrap_or_else(|| json!({}));
    match options[choice].0.as_str() {
        "titre" => {
            let current = page_snapshot
                .get("titre")
                .and_then(Value::as_str)
                .unwrap_or("");
            let value = prompts::prompt_line(terminal, "Titre", Some(current))?;
            page_object_mut(document, page_id).insert("titre".into(), Value::String(value));
            document.mark_dirty();
        }
        "tags" => {
            let mut value = page_snapshot
                .get("tags")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if manage_json_object(terminal, "Tags immuables de page", &mut value, true)? {
                page_object_mut(document, page_id).insert("tags".into(), value);
                document.mark_dirty();
            }
        }
        "cellules" => {
            let mut value = page_snapshot
                .get("cellules")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if manage_cells(
                terminal,
                document,
                &mut value,
                &format!("Cellules de `{page_id}`"),
            )? {
                page_object_mut(document, page_id).insert("cellules".into(), value);
                document.mark_dirty();
            }
        }
        "connexions" => {
            let mut value = page_snapshot
                .get("connexions")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let had_legacy_connections = page_snapshot.get("connexions_dyn").is_some();
            if let Some(legacy) = page_snapshot.get("connexions_dyn") {
                merge_legacy_dynamic_connections(&mut value, legacy);
            }
            if manage_connections(terminal, document, &mut value, page_id)?
                || had_legacy_connections
            {
                let page = page_object_mut(document, page_id);
                page.insert("connexions".into(), value);
                page.remove("connexions_dyn");
                document.mark_dirty();
            }
        }
        "json" => {
            if let Some(value) =
                prompts::edit_json(terminal, &format!("Page `{page_id}`"), &page_snapshot)?
            {
                if let Some(slot) = document.page_mut(page_id) {
                    *slot = value;
                    document.mark_dirty();
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Opens the guided editor for one item definition.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
/// - `item_id` — Identifier of the item definition.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn edit_item(
    terminal: &mut AppTerminal,
    document: &mut TaleDocument,
    item_id: &str,
) -> Result<()> {
    let snapshot = document
        .entity("objets", item_id)
        .cloned()
        .unwrap_or_else(schema::new_item_definition);
    let options = vec![
        ("nom".into(), "Nom".into()),
        ("description".into(), "Description".into()),
        ("traits".into(), "Traits".into()),
        ("donnees".into(), "Métadonnées par défaut".into()),
        ("json".into(), "Éditer en JSON".into()),
    ];
    let Some(choice) = prompts::select(
        terminal,
        &format!("Modifier l'objet `{item_id}`"),
        &options,
        true,
    )?
    else {
        return Ok(());
    };
    if options[choice].0 == "json" {
        if let Some(value) = prompts::edit_json(terminal, "Objet", &snapshot)? {
            if let Some(slot) = document.entity_mut("objets", item_id) {
                *slot = value;
                document.mark_dirty();
            }
        }
        return Ok(());
    }
    let mut updated = snapshot.clone();
    let object = ensure_object(&mut updated);
    match options[choice].0.as_str() {
        "nom" => {
            let current = object.get("nom").and_then(Value::as_str).unwrap_or("");
            object.insert(
                "nom".into(),
                Value::String(prompts::prompt_line(terminal, "Nom", Some(current))?),
            );
        }
        "description" => {
            let current = object
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            object.insert(
                "description".into(),
                Value::String(prompts::edit_multiline(terminal, "Description", current)?),
            );
        }
        "traits" => {
            let current = string_array(object.get("traits"));
            let values = prompts::prompt_string_list(terminal, "Traits", &current)?;
            object.insert(
                "traits".into(),
                Value::Array(values.into_iter().map(Value::String).collect()),
            );
        }
        "donnees" => {
            let mut value = object.get("donnees").cloned().unwrap_or_else(|| json!({}));
            if manage_json_object(terminal, "Métadonnées", &mut value, false)? {
                object.insert("donnees".into(), value);
            }
        }
        _ => return Ok(()),
    }
    if let Some(slot) = document.entity_mut("objets", item_id) {
        *slot = updated;
        document.mark_dirty();
    }
    Ok(())
}

/// Opens the guided editor for one persistent-state definition.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
/// - `state_id` — Identifier of the persistent-state definition.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn edit_condition_definition(
    terminal: &mut AppTerminal,
    document: &mut TaleDocument,
    state_id: &str,
) -> Result<()> {
    let snapshot = document
        .entity("etats", state_id)
        .cloned()
        .unwrap_or_else(schema::new_condition_definition);
    let options = vec![
        ("nom".into(), "Nom".into()),
        ("description".into(), "Description".into()),
        ("traits".into(), "Traits".into()),
        ("donnees".into(), "Métadonnées par défaut".into()),
        ("effets".into(), "Modificateurs persistants".into()),
        ("avant".into(), "Cellules avant la page".into()),
        ("apres".into(), "Cellules après la page".into()),
        ("expiration".into(), "Expiration".into()),
        ("defaite".into(), "Page de défaite".into()),
        ("json".into(), "Éditer en JSON".into()),
    ];
    let Some(choice) = prompts::select(
        terminal,
        &format!("Modifier l'état `{state_id}`"),
        &options,
        true,
    )?
    else {
        return Ok(());
    };
    if options[choice].0 == "json" {
        if let Some(value) = prompts::edit_json(terminal, "État", &snapshot)? {
            if let Some(slot) = document.entity_mut("etats", state_id) {
                *slot = value;
                document.mark_dirty();
            }
        }
        return Ok(());
    }
    let mut updated = snapshot.clone();
    let object = ensure_object(&mut updated);
    match options[choice].0.as_str() {
        "nom" => {
            let current = object.get("nom").and_then(Value::as_str).unwrap_or("");
            object.insert(
                "nom".into(),
                Value::String(prompts::prompt_line(terminal, "Nom", Some(current))?),
            );
        }
        "description" => {
            let current = object
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("");
            object.insert(
                "description".into(),
                Value::String(prompts::edit_multiline(terminal, "Description", current)?),
            );
        }
        "traits" => {
            let current = string_array(object.get("traits"));
            let values = prompts::prompt_string_list(terminal, "Traits", &current)?;
            object.insert(
                "traits".into(),
                Value::Array(values.into_iter().map(Value::String).collect()),
            );
        }
        "donnees" => {
            let mut value = object.get("donnees").cloned().unwrap_or_else(|| json!({}));
            if manage_json_object(terminal, "Métadonnées", &mut value, false)? {
                object.insert("donnees".into(), value);
            }
        }
        "effets" => {
            let mut effects = object.get("effets").cloned().unwrap_or_else(|| json!([]));
            if manage_condition_effects(terminal, document, &mut effects)? {
                object.insert("effets".into(), effects);
            }
        }
        "avant" | "apres" => {
            let field = options[choice].0.as_str();
            let title = if field == "avant" {
                "Cellules d'état avant la page"
            } else {
                "Cellules d'état après la page"
            };
            let mut cells = object.get(field).cloned().unwrap_or_else(|| json!([]));
            if manage_cells(terminal, document, &mut cells, title)? {
                object.insert(field.into(), cells);
            }
        }
        "expiration" => {
            if prompts::confirm(
                terminal,
                "Ajouter ou remplacer le filtre d'expiration?",
                true,
            )? {
                object.insert(
                    "expiration".into(),
                    build_filter(terminal, document, FilterFamily::Condition, 0)?,
                );
            } else {
                object.insert("expiration".into(), Value::Null);
            }
        }
        "defaite" => {
            let page = choose_optional_page(terminal, document, "Page de défaite", true)?;
            object.insert("defaite".into(), page.map_or(Value::Null, Value::String));
        }
        _ => return Ok(()),
    }
    if let Some(slot) = document.entity_mut("etats", state_id) {
        *slot = updated;
        document.mark_dirty();
    }
    Ok(())
}

/// Normalizes a supported deferred container to the Forge's editable array form.
///
/// Connections, cells, and effects may legally be stored as one object or one formula.
/// Their guided managers use arrays internally so entries can be reordered without
/// discarding those compact forms.
fn normalize_deferred_container(container: &mut Value) {
    if container.is_array() {
        return;
    }
    let previous = container.take();
    *container = match previous {
        Value::Null => json!([]),
        other => Value::Array(vec![other]),
    };
}

/// Opens the guided manager for the tale's root image registry.
fn manage_image_definitions(
    terminal: &mut AppTerminal,
    document: &mut TaleDocument,
) -> Result<()> {
    if document
        .root
        .get("images")
        .is_some_and(|images| !images.is_object())
    {
        return Err(anyhow!(
            "La bibliothèque `images` doit être corrigée en JSON avant d'utiliser l'assistant"
        ));
    }
    loop {
        let ids = document.entity_ids("images");
        let mut options = ids
            .iter()
            .map(|id| {
                (
                    format!("image:{id}"),
                    image_definition_forge_label(id, document.entity("images", id)),
                )
            })
            .collect::<Vec<_>>();
        options.extend([
            ("add".into(), "＋ Ajouter une image".into()),
            ("json".into(), "Modifier le registre complet en JSON".into()),
            ("done".into(), "← Terminer".into()),
        ]);
        let choice = prompts::select(terminal, "Bibliothèque d'images", &options, false)?
            .expect("la liste contient toujours des actions");
        let selected = options[choice].0.clone();
        match selected.as_str() {
            "done" => break,
            "add" => {
                let id = prompts::prompt_line(terminal, "Identifiant de l'image", None)?;
                if id.is_empty() {
                    return Err(anyhow!("L'identifiant d'une image ne peut pas être vide"));
                }
                if document.entity("images", &id).is_some() {
                    return Err(anyhow!("L'identifiant d'image `{id}` existe déjà"));
                }
                let definition = build_image_definition(terminal)?;
                document.add_entity("images", id, definition);
            }
            "json" => {
                let current = document
                    .root
                    .get("images")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if let Some(updated) =
                    prompts::edit_json(terminal, "Bibliothèque d'images", &current)?
                {
                    if !updated.is_object() {
                        return Err(anyhow!("La bibliothèque d'images doit être un objet"));
                    }
                    root_object_mut(document).insert("images".into(), updated);
                    document.mark_dirty();
                }
            }
            _ => {
                let id = selected.trim_start_matches("image:").to_owned();
                let actions = vec![
                    ("edit".into(), "Modifier avec l'assistant".into()),
                    ("duplicate".into(), "Dupliquer".into()),
                    ("rename".into(), "Renommer".into()),
                    ("json".into(), "Modifier en JSON".into()),
                    ("delete".into(), "Supprimer".into()),
                ];
                let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
                    continue;
                };
                match actions[action].0.as_str() {
                    "edit" => {
                        let current = document
                            .entity("images", &id)
                            .cloned()
                            .unwrap_or_else(|| json!({}));
                        let updated = edit_image_definition_guided(terminal, current)?;
                        if let Some(slot) = document.entity_mut("images", &id) {
                            *slot = updated;
                            document.mark_dirty();
                        }
                    }
                    "duplicate" => {
                        let new_id = prompts::prompt_line(
                            terminal,
                            "Identifiant de la copie",
                            Some(&format!("{id}_copie")),
                        )?;
                        document.duplicate_entity("images", &id, new_id)?;
                    }
                    "rename" => {
                        let new_id =
                            prompts::prompt_line(terminal, "Nouvel identifiant", Some(&id))?;
                        if new_id.is_empty() {
                            return Err(anyhow!("L'identifiant d'une image ne peut pas être vide"));
                        }
                        if new_id != id {
                            document.rename_entity("images", &id, &new_id)?;
                        }
                    }
                    "json" => {
                        if let Some(current) = document.entity("images", &id).cloned() {
                            if let Some(updated) =
                                prompts::edit_json(terminal, &format!("Image `{id}`"), &current)?
                            {
                                if let Some(slot) = document.entity_mut("images", &id) {
                                    *slot = updated;
                                    document.mark_dirty();
                                }
                            }
                        }
                    }
                    "delete" => {
                        if prompts::confirm(
                            terminal,
                            &format!(
                                "Supprimer l'image `{id}`? Les références restantes seront signalées par la validation."
                            ),
                            false,
                        )? {
                            document.delete_entity("images", &id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// Builds one path, URL, or embedded base64 image definition.
fn build_image_definition(terminal: &mut AppTerminal) -> Result<Value> {
    let kind = prompt_image_type_current(terminal, None)?;
    let mut definition = json!({"type": kind});
    prompt_image_source(terminal, &mut definition)?;
    if let Some(alt) = prompts::prompt_optional_line(terminal, "Texte alternatif", None)? {
        ensure_object(&mut definition).insert("alt".into(), Value::String(alt));
    }
    Ok(definition)
}

/// Edits one image definition while preserving raw JSON access for advanced fields.
fn edit_image_definition_guided(
    terminal: &mut AppTerminal,
    mut definition: Value,
) -> Result<Value> {
    ensure_object(&mut definition);
    loop {
        let options = vec![
            ("type".into(), "Type et source".into()),
            ("source".into(), "Source".into()),
            ("alt".into(), "Texte alternatif".into()),
            ("json".into(), "Modifier toute la définition en JSON".into()),
            ("done".into(), "← Terminer".into()),
        ];
        let choice = prompts::select(terminal, "Modifier l'image", &options, false)?
            .expect("la liste contient toujours des actions");
        match options[choice].0.as_str() {
            "type" => {
                let current = definition.get("type").and_then(Value::as_str);
                let kind = prompt_image_type_current(terminal, current)?;
                let changed = current != Some(kind.as_str());
                let object = ensure_object(&mut definition);
                object.insert("type".into(), Value::String(kind));
                if changed {
                    for key in ["chemin", "url", "donnees", "format"] {
                        object.remove(key);
                    }
                    prompt_image_source(terminal, &mut definition)?;
                }
            }
            "source" => prompt_image_source(terminal, &mut definition)?,
            "alt" => {
                let current = definition.get("alt").and_then(Value::as_str);
                match prompts::prompt_optional_line(terminal, "Texte alternatif", current)? {
                    Some(alt) => {
                        ensure_object(&mut definition)
                            .insert("alt".into(), Value::String(alt));
                    }
                    None => {
                        ensure_object(&mut definition).remove("alt");
                    }
                }
            }
            "json" => {
                if let Some(updated) =
                    prompts::edit_json(terminal, "Définition d'image", &definition)?
                {
                    definition = updated;
                }
            }
            "done" => break,
            _ => {}
        }
    }
    Ok(definition)
}

/// Prompts for an image definition's source fields according to its selected type.
fn prompt_image_source(terminal: &mut AppTerminal, definition: &mut Value) -> Result<()> {
    let kind = definition
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Le type de l'image doit être choisi avant sa source"))?
        .to_owned();
    match kind.as_str() {
        "fichier" => {
            let current = definition.get("chemin").and_then(Value::as_str);
            let path = prompts::prompt_line(terminal, "Chemin absolu ou relatif", current)?;
            ensure_object(definition).insert("chemin".into(), Value::String(path));
        }
        "url" => {
            let current = definition.get("url").and_then(Value::as_str);
            let url = prompts::prompt_line(terminal, "URL HTTP ou HTTPS", current)?;
            ensure_object(definition).insert("url".into(), Value::String(url));
        }
        "base64" => {
            let current = definition
                .get("donnees")
                .and_then(Value::as_str)
                .unwrap_or("");
            let data = prompts::edit_multiline(terminal, "Données base64", current)?;
            let data = data
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            let format = prompt_image_format_current(
                terminal,
                definition.get("format").and_then(Value::as_str),
            )?;
            let object = ensure_object(definition);
            object.insert("donnees".into(), Value::String(data));
            object.insert("format".into(), Value::String(format));
        }
        _ => return Err(anyhow!("Type d'image inconnu: `{kind}`")),
    }
    Ok(())
}

/// Prompts for an image source type, retaining the current selection when possible.
fn prompt_image_type_current(
    terminal: &mut AppTerminal,
    current: Option<&str>,
) -> Result<String> {
    let options = vec![
        ("fichier".into(), "Fichier local (fichier)".into()),
        ("url".into(), "Adresse Internet (URL)".into()),
        ("base64".into(), "Image intégrée en base64".into()),
    ];
    let initial = options
        .iter()
        .position(|(id, _): &(String, String)| Some(id.as_str()) == current)
        .unwrap_or(0);
    let choice = prompts::select_with_initial(
        terminal,
        "Type d'image",
        &options,
        initial,
        false,
    )?
    .expect("la liste des types d'image n'est jamais vide");
    Ok(options[choice].0.clone())
}

/// Prompts for the declared format of an embedded image.
fn prompt_image_format_current(
    terminal: &mut AppTerminal,
    current: Option<&str>,
) -> Result<String> {
    let options = SUPPORTED_FORMATS
        .iter()
        .map(|format| ((*format).to_owned(), format.to_uppercase()))
        .collect::<Vec<_>>();
    let initial = options
        .iter()
        .position(|(id, _): &(String, String)| Some(id.as_str()) == current)
        .unwrap_or(0);
    let choice = prompts::select_with_initial(
        terminal,
        "Format de l'image intégrée",
        &options,
        initial,
        false,
    )?
    .expect("la liste des formats d'image n'est jamais vide");
    Ok(options[choice].0.clone())
}

/// Returns a compact Forge label without exposing large embedded image data.
fn image_definition_forge_label(id: &str, definition: Option<&Value>) -> String {
    let Some(definition) = definition.and_then(Value::as_object) else {
        return format!("{id} · définition invalide");
    };
    let kind = definition
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("type manquant");
    let source = match kind {
        "fichier" => definition.get("chemin").and_then(Value::as_str).unwrap_or("?"),
        "url" => definition.get("url").and_then(Value::as_str).unwrap_or("?"),
        "base64" => {
            let format = definition
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let size = definition
                .get("donnees")
                .and_then(Value::as_str)
                .map_or(0, str::len);
            return format!("{id} · base64 {format} · {size} caractères");
        }
        _ => "?",
    };
    format!("{id} · {kind} · {}", truncate_label(source, 48))
}

/// Opens an ordered editor for image IDs used by the title or by one cell.
fn manage_image_references(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    container: &mut Value,
    title: &str,
    allow_formula: bool,
) -> Result<bool> {
    normalize_deferred_container(container);
    let mut changed = false;
    loop {
        let mut options = container
            .as_array()
            .expect("image reference array")
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let label = match value {
                    Value::String(reference) if is_formula(value) => {
                        format!("ƒ {}", truncate_label(reference, 64))
                    }
                    Value::String(reference) => reference.clone(),
                    other => format!("Entrée invalide: {}", prompts::json_compact(other)),
                };
                (format!("image-ref:{index}"), label)
            })
            .collect::<Vec<_>>();
        options.push(("add".into(), "＋ Ajouter une image enregistrée".into()));
        if allow_formula {
            options.push(("add-formula".into(), "ƒ Ajouter une formule d'image".into()));
        }
        options.push(("done".into(), "← Terminer".into()));
        let choice = prompts::select(terminal, title, &options, false)?
            .expect("la liste contient toujours des actions");
        let selected = options[choice].0.clone();
        match selected.as_str() {
            "done" => break,
            "add" => {
                if let Some(id) = choose_entity(
                    terminal,
                    document,
                    "images",
                    "Image enregistrée",
                    false,
                )? {
                    container
                        .as_array_mut()
                        .expect("image reference array")
                        .push(Value::String(id));
                    changed = true;
                }
            }
            "add-formula" => {
                let formula = prompts::prompt_line(
                    terminal,
                    "Formule retournant un identifiant d'image",
                    Some("${\"image_id\"}"),
                )?;
                container
                    .as_array_mut()
                    .expect("image reference array")
                    .push(Value::String(formula));
                changed = true;
            }
            _ => {
                let index = selected.trim_start_matches("image-ref:").parse::<usize>()?;
                let mut actions = vec![(
                    "static".into(),
                    "Choisir une image enregistrée".into(),
                )];
                if allow_formula {
                    actions.push(("formula".into(), "Entrer une formule".into()));
                }
                actions.extend([
                    ("json".into(), "Modifier en JSON".into()),
                    ("top".into(), "Déplacer tout en haut".into()),
                    ("up".into(), "Monter".into()),
                    ("down".into(), "Descendre".into()),
                    ("bottom".into(), "Déplacer tout en bas".into()),
                    ("delete".into(), "Supprimer".into()),
                ]);
                let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
                    continue;
                };
                match actions[action].0.as_str() {
                    "static" => {
                        if let Some(id) = choose_entity(
                            terminal,
                            document,
                            "images",
                            "Image enregistrée",
                            false,
                        )? {
                            container.as_array_mut().unwrap()[index] = Value::String(id);
                            changed = true;
                        }
                    }
                    "formula" => {
                        let current = container.as_array().unwrap()[index]
                            .as_str()
                            .filter(|_| is_formula(&container.as_array().unwrap()[index]));
                        let formula = prompts::prompt_line(
                            terminal,
                            "Formule retournant un identifiant d'image",
                            current.or(Some("${\"image_id\"}")),
                        )?;
                        container.as_array_mut().unwrap()[index] = Value::String(formula);
                        changed = true;
                    }
                    "json" => {
                        let current = container.as_array().unwrap()[index].clone();
                        if let Some(updated) =
                            prompts::edit_json(terminal, "Référence d'image", &current)?
                        {
                            container.as_array_mut().unwrap()[index] = updated;
                            changed = true;
                        }
                    }
                    "top" => {
                        forge_ops::move_array_entry(container, index, 0)?;
                        changed = true;
                    }
                    "up" if index > 0 => {
                        container.as_array_mut().unwrap().swap(index, index - 1);
                        changed = true;
                    }
                    "down" if index + 1 < container.as_array().unwrap().len() => {
                        forge_ops::move_array_entry(container, index, index + 1)?;
                        changed = true;
                    }
                    "bottom" => {
                        let last = container.as_array().unwrap().len().saturating_sub(1);
                        forge_ops::move_array_entry(container, index, last)?;
                        changed = true;
                    }
                    "delete" => {
                        container.as_array_mut().unwrap().remove(index);
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(changed)
}

/// Migrates legacy `connexions_dyn` entries into the unified `connexions` list.
///
/// Old wrapper objects contributed only their `formule`; `id` and `libelle` were Forge
/// metadata and have no runtime meaning. Malformed legacy values are preserved so the
/// validator can report them instead of the editor silently deleting author data.
fn merge_legacy_dynamic_connections(connections: &mut Value, legacy: &Value) {
    normalize_deferred_container(connections);
    let output = connections.as_array_mut().expect("connection array");
    let legacy_entries: Vec<&Value> = match legacy {
        Value::Null => Vec::new(),
        Value::Array(values) => values.iter().collect(),
        value => vec![value],
    };
    for entry in legacy_entries {
        match entry {
            Value::Object(object) if object.contains_key("formule") => {
                output.push(object["formule"].clone());
            }
            other => output.push(other.clone()),
        }
    }
}

/// Produces the compact label used for one literal or formula connection entry.
fn connection_forge_label(connection: &Value) -> String {
    match connection {
        Value::Object(_) => {
            let destination = connection
                .get("destination")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let text = connection
                .get("texte")
                .and_then(Value::as_str)
                .unwrap_or("Connexion sans texte");
            format!("{text} → {destination}")
        }
        Value::String(formula) => format!(
            "ƒ {}",
            truncate_label(&formula.replace('\n', " ").replace('\r', " "), 68)
        ),
        other => format!("Entrée invalide: {}", prompts::json_compact(other)),
    }
}

/// Produces a compact label for a structured cell or a formula producing cells.
fn cell_forge_label(cell: &Value, index: usize) -> String {
    match cell {
        Value::Object(_) => schema::cell_summary(cell, index),
        Value::String(formula) => format!(
            "ƒ {}",
            truncate_label(&formula.replace('\n', " ").replace('\r', " "), 68)
        ),
        other => format!("Entrée invalide: {}", prompts::json_compact(other)),
    }
}

/// Produces a compact label for a structured effect or a formula producing effects.
fn effect_forge_label(effect: &Value) -> String {
    match effect {
        Value::Object(_) => schema::effect_summary(effect),
        Value::String(formula) => format!(
            "ƒ {}",
            truncate_label(&formula.replace('\n', " ").replace('\r', " "), 68)
        ),
        other => format!("Entrée invalide: {}", prompts::json_compact(other)),
    }
}

/// Opens an interactive manager for literal and formula connections.
///
/// Runs until the user closes the manager. The returned Boolean reports whether the
/// managed container changed.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
/// - `container` — Raw JSON array or object edited in place.
/// - `source_page` — Identifier of the page that owns the connection.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn manage_connections(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    container: &mut Value,
    source_page: &str,
) -> Result<bool> {
    normalize_deferred_container(container);
    let mut changed = false;
    loop {
        let connections = container
            .as_array()
            .expect("La valeur Json doit être un tableau");
        let mut options = connections
            .iter()
            .enumerate()
            .map(|(index, connection)| {
                (format!("conn-{index}"), connection_forge_label(connection))
            })
            .collect::<Vec<_>>();
        options.extend([
            ("add".into(), "＋ Ajouter une connexion structurée".into()),
            (
                "add-formula".into(),
                "ƒ Ajouter une formule de connexion".into(),
            ),
            ("done".into(), "← Terminer".into()),
        ]);
        let Some(choice) = prompts::select(
            terminal,
            &format!("Connexions de `{source_page}`"),
            &options,
            false,
        )?
        else {
            continue;
        };
        let key = &options[choice].0;
        if key == "done" {
            break;
        }
        if key == "add" {
            let connection = build_connection(terminal, document)?;
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(connection);
            changed = true;
            continue;
        }
        if key == "add-formula" {
            let formula = prompts::prompt_line(
                terminal,
                "Formule retournant une connexion ou une liste",
                Some("${[]}"),
            )?;
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(Value::String(formula));
            changed = true;
            continue;
        }
        let index = key.trim_start_matches("conn-").parse::<usize>()?;
        let is_structured = container.as_array().unwrap()[index].is_object();
        let primary_action: (String, String) = if is_structured {
            ("edit".into(), "Modifier avec l'assistant".into())
        } else {
            ("formula".into(), "Modifier la formule".into())
        };
        let mut actions = vec![
            primary_action,
            ("duplicate".into(), "Dupliquer".into()),
            ("json".into(), "Modifier en JSON".into()),
        ];
        if is_structured {
            actions.extend([
                ("cells".into(), "Gérer les cellules".into()),
                (
                    "tags".into(),
                    "Gérer les tags transmis à la destination".into(),
                ),
            ]);
        }
        actions.extend([
            ("top".into(), "Déplacer tout en haut".into()),
            ("up".into(), "Monter".into()),
            ("down".into(), "Descendre".into()),
            ("bottom".into(), "Déplacer tout en bas".into()),
            ("delete".into(), "Supprimer".into()),
        ]);
        let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
            continue;
        };
        match actions[action].0.as_str() {
            "edit" => {
                let current = container.as_array().unwrap()[index].clone();
                let updated = edit_connection_guided(terminal, document, current)?;
                container.as_array_mut().unwrap()[index] = updated;
                changed = true;
            }
            "formula" => {
                let current = container.as_array().unwrap()[index].as_str().unwrap_or("");
                let updated = prompts::prompt_line(
                    terminal,
                    "Formule retournant une connexion ou une liste",
                    Some(current),
                )?;
                container.as_array_mut().unwrap()[index] = Value::String(updated);
                changed = true;
            }
            "duplicate" => {
                forge_ops::duplicate_array_entry(container, index)?;
                changed = true;
            }
            "json" => {
                let current = container.as_array().unwrap()[index].clone();
                if let Some(updated) = prompts::edit_json(terminal, "Connexion", &current)? {
                    container.as_array_mut().unwrap()[index] = updated;
                    changed = true;
                }
            }
            "cells" => {
                let current = &mut container.as_array_mut().unwrap()[index];
                let object = ensure_object(current);
                let cells = object.entry("cellules").or_insert_with(|| json!([]));
                changed |= manage_cells(terminal, document, cells, "Cellules de connexion")?;
            }
            "tags" => {
                let current = &mut container.as_array_mut().unwrap()[index];
                let object = ensure_object(current);
                let tags = object.entry("tags").or_insert_with(|| json!({}));
                changed |= manage_json_object(
                    terminal,
                    "Tags immuables transmis dans `connexion`",
                    tags,
                    true,
                )?;
            }
            "top" => {
                forge_ops::move_array_entry(container, index, 0)?;
                changed = true;
            }
            "up" if index > 0 => {
                container.as_array_mut().unwrap().swap(index, index - 1);
                changed = true;
            }
            "down" if index + 1 < container.as_array().unwrap().len() => {
                forge_ops::move_array_entry(container, index, index + 1)?;
                changed = true;
            }
            "bottom" => {
                let last = container.as_array().unwrap().len().saturating_sub(1);
                forge_ops::move_array_entry(container, index, last)?;
                changed = true;
            }
            "delete" => {
                if prompts::confirm(terminal, "Supprimer cette connexion?", false)? {
                    container.as_array_mut().unwrap().remove(index);
                    changed = true;
                }
            }
            _ => {}
        }
    }
    Ok(changed)
}
/// Opens the normalized cell manager used by pages, connections, `avant`, and `apres`.
///
/// Runs until the user closes the manager. The returned Boolean reports whether the
/// managed container changed.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
/// - `container` — Raw JSON array or object edited in place.
/// - `title` — Prompt, window, or page title shown to the user.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn manage_cells(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    container: &mut Value,
    title: &str,
) -> Result<bool> {
    normalize_deferred_container(container);
    let mut changed = false;
    loop {
        let mut options = container
            .as_array()
            .expect("La valeur Json doit être un tableau")
            .iter()
            .enumerate()
            .map(|(index, cell)| (format!("cell-{index}"), cell_forge_label(cell, index)))
            .collect::<Vec<_>>();
        options.extend([
            ("add".into(), "＋ Ajouter une cellule structurée".into()),
            (
                "add-formula".into(),
                "ƒ Ajouter une formule de cellules".into(),
            ),
            ("done".into(), "← Terminer".into()),
        ]);
        let Some(choice) = prompts::select(terminal, title, &options, false)? else {
            continue;
        };
        let key = options[choice].0.as_str();
        if key == "done" {
            break;
        }
        if key == "add" {
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(build_cell(terminal, document)?);
            changed = true;
            continue;
        }
        if key == "add-formula" {
            let formula = prompts::prompt_line(
                terminal,
                "Formule retournant une cellule ou une liste de cellules",
                Some("${[]}"),
            )?;
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(Value::String(formula));
            changed = true;
            continue;
        }
        let index = key.trim_start_matches("cell-").parse::<usize>()?;
        let is_structured = container.as_array().unwrap()[index].is_object();
        let primary_action = if is_structured {
            ("edit".into(), "Modifier avec l'assistant".into())
        } else {
            ("formula".into(), "Modifier la formule".into())
        };
        let actions = vec![
            primary_action,
            ("duplicate".into(), "Dupliquer".into()),
            ("json".into(), "Modifier en JSON".into()),
            ("top".into(), "Déplacer tout en haut".into()),
            ("up".into(), "Monter".into()),
            ("down".into(), "Descendre".into()),
            ("bottom".into(), "Déplacer tout en bas".into()),
            ("delete".into(), "Supprimer".into()),
        ];
        let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
            continue;
        };
        match actions[action].0.as_str() {
            "edit" => {
                let current = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")[index]
                    .clone();
                container
                    .as_array_mut()
                    .expect("La valeur Json doit être un tableau")[index] =
                    edit_cell_guided(terminal, document, current)?;
                changed = true;
            }
            "formula" => {
                let current = container.as_array().unwrap()[index].as_str().unwrap_or("");
                let updated = prompts::prompt_line(
                    terminal,
                    "Formule retournant une cellule ou une liste de cellules",
                    Some(current),
                )?;
                container.as_array_mut().unwrap()[index] = Value::String(updated);
                changed = true;
            }
            "duplicate" => {
                forge_ops::duplicate_array_entry(container, index)?;
                changed = true;
            }
            "json" => {
                let current = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")[index]
                    .clone();
                if let Some(updated) = prompts::edit_json(terminal, "Cellule", &current)? {
                    container
                        .as_array_mut()
                        .expect("La valeur Json doit être un tableau")[index] = updated;
                    changed = true;
                }
            }
            "top" => {
                forge_ops::move_array_entry(container, index, 0)?;
                changed = true;
            }
            "up" if index > 0 => {
                forge_ops::move_array_entry(container, index, index - 1)?;
                changed = true;
            }
            "down"
                if index + 1
                    < container
                        .as_array()
                        .expect("La valeur Json doit être un tableau")
                        .len() =>
            {
                forge_ops::move_array_entry(container, index, index + 1)?;
                changed = true;
            }
            "bottom" => {
                let last = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")
                    .len()
                    .saturating_sub(1);
                forge_ops::move_array_entry(container, index, last)?;
                changed = true;
            }
            "delete" => {
                if prompts::confirm(terminal, "Supprimer cette cellule?", false)? {
                    container
                        .as_array_mut()
                        .expect("La valeur Json doit être un tableau")
                        .remove(index);
                    changed = true;
                }
            }
            _ => {}
        }
    }
    Ok(changed)
}

/// Opens a manager for the atomic effects nested inside one cell.
///
/// Runs until the user closes the manager. The returned Boolean reports whether the
/// managed container changed.
fn manage_atomic_effects(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    container: &mut Value,
    title: &str,
) -> Result<bool> {
    normalize_deferred_container(container);
    let mut changed = false;
    loop {
        let mut options = container
            .as_array()
            .expect("La valeur Json doit être un tableau")
            .iter()
            .enumerate()
            .map(|(index, effect)| (format!("effect-{index}"), effect_forge_label(effect)))
            .collect::<Vec<_>>();
        options.extend([
            ("add".into(), "＋ Ajouter un effet structuré".into()),
            (
                "add-formula".into(),
                "ƒ Ajouter une formule d'effets".into(),
            ),
            ("done".into(), "← Terminer".into()),
        ]);
        let Some(choice) = prompts::select(terminal, title, &options, false)? else {
            continue;
        };
        let key = options[choice].0.as_str();
        if key == "done" {
            break;
        }
        if key == "add" {
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(build_effect(terminal, document)?);
            changed = true;
            continue;
        }
        if key == "add-formula" {
            let formula = prompts::prompt_line(
                terminal,
                "Formule retournant un effet ou une liste d'effets",
                Some("${[]}"),
            )?;
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(Value::String(formula));
            changed = true;
            continue;
        }
        let index = key.trim_start_matches("effect-").parse::<usize>()?;
        let is_structured = container.as_array().unwrap()[index].is_object();
        let primary_action = if is_structured {
            ("edit".into(), "Modifier avec l'assistant".into())
        } else {
            ("formula".into(), "Modifier la formule".into())
        };
        let actions = vec![
            primary_action,
            ("duplicate".into(), "Dupliquer".into()),
            ("json".into(), "Modifier en JSON".into()),
            ("up".into(), "Monter".into()),
            ("down".into(), "Descendre".into()),
            ("delete".into(), "Supprimer".into()),
        ];
        let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
            continue;
        };
        match actions[action].0.as_str() {
            "edit" => {
                let current = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")[index]
                    .clone();
                container
                    .as_array_mut()
                    .expect("La valeur Json doit être un tableau")[index] =
                    edit_effect_guided(terminal, document, current)?;
                changed = true;
            }
            "formula" => {
                let current = container.as_array().unwrap()[index].as_str().unwrap_or("");
                let updated = prompts::prompt_line(
                    terminal,
                    "Formule retournant un effet ou une liste d'effets",
                    Some(current),
                )?;
                container.as_array_mut().unwrap()[index] = Value::String(updated);
                changed = true;
            }
            "duplicate" => {
                forge_ops::duplicate_array_entry(container, index)?;
                changed = true;
            }
            "json" => {
                let current = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")[index]
                    .clone();
                if let Some(updated) = prompts::edit_json(terminal, "Effet", &current)? {
                    container
                        .as_array_mut()
                        .expect("La valeur Json doit être un tableau")[index] = updated;
                    changed = true;
                }
            }
            "up" if index > 0 => {
                forge_ops::move_array_entry(container, index, index - 1)?;
                changed = true;
            }
            "down"
                if index + 1
                    < container
                        .as_array()
                        .expect("La valeur Json doit être un tableau")
                        .len() =>
            {
                forge_ops::move_array_entry(container, index, index + 1)?;
                changed = true;
            }
            "delete" => {
                container
                    .as_array_mut()
                    .expect("La valeur Json doit être un tableau")
                    .remove(index);
                changed = true;
            }
            _ => {}
        }
    }
    Ok(changed)
}

/// Builds a new cell with optional identity, condition, text, images, and effects.
///
/// Returns normalized raw JSON but does not insert it into the document; the caller
/// chooses where the new value belongs.
fn build_cell(terminal: &mut AppTerminal, document: &TaleDocument) -> Result<Value> {
    let id = prompts::prompt_optional_line(terminal, "ID stable de cellule", None)?;
    let label = prompts::prompt_optional_line(terminal, "Libellé Forge", None)?;
    let condition = if prompts::confirm(terminal, "Ajouter une condition?", false)? {
        Some(build_conditional(terminal, document, 0)?)
    } else {
        None
    };
    let text = prompt_optional_multiline(terminal, "Texte de la cellule", None)?;
    let mut images = json!([]);
    if prompts::confirm(terminal, "Ajouter des images?", false)? {
        manage_image_references(
            terminal,
            document,
            &mut images,
            "Images de la cellule",
            true,
        )?;
    }
    let mut effects = json!([]);
    if prompts::confirm(terminal, "Ajouter des effets?", false)? {
        manage_atomic_effects(terminal, document, &mut effects, "Effets de la cellule")?;
    }
    let mut object = Map::new();
    if let Some(id) = id {
        object.insert("id".into(), Value::String(id));
    }
    if let Some(label) = label {
        object.insert("libelle".into(), Value::String(label));
    }
    if let Some(condition) = condition {
        object.insert("conditions".into(), condition);
    }
    if let Some(text) = text {
        object.insert("texte".into(), Value::String(text));
    }
    if !images.as_array().is_some_and(Vec::is_empty) {
        object.insert("images".into(), images);
    }
    if !effects.as_array().is_some_and(Vec::is_empty) {
        object.insert("effets".into(), effects);
    }
    if !object.contains_key("conditions")
        && !object.contains_key("texte")
        && !object.contains_key("images")
        && !object.contains_key("effets")
    {
        object.insert("texte".into(), Value::String(String::new()));
    }
    Ok(Value::Object(object))
}

/// Edits every normalized cell field without exposing unrelated page structure.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
fn edit_cell_guided(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    mut cell: Value,
) -> Result<Value> {
    loop {
        let options = vec![
            ("id".into(), "ID stable".into()),
            ("libelle".into(), "Libellé Forge".into()),
            ("effects".into(), "Effets".into()),
            ("condition".into(), "Condition".into()),
            ("text".into(), "Texte".into()),
            ("images".into(), "Images affichées après le texte".into()),
            ("json".into(), "Modifier toute la cellule en JSON".into()),
            ("done".into(), "← Terminer".into()),
        ];
        let Some(choice) = prompts::select(terminal, "Modifier la cellule", &options, false)?
        else {
            continue;
        };
        match options[choice].0.as_str() {
            "id" | "libelle" => {
                let key = options[choice].0.as_str();
                let current = cell.get(key).and_then(Value::as_str);
                match prompts::prompt_optional_line(terminal, &options[choice].1, current)? {
                    Some(value) => {
                        ensure_object(&mut cell).insert(key.into(), Value::String(value));
                    }
                    None => {
                        ensure_object(&mut cell).remove(key);
                    }
                }
            }
            "effects" => {
                let effects = ensure_object(&mut cell)
                    .entry("effets")
                    .or_insert_with(|| json!([]));
                manage_atomic_effects(terminal, document, effects, "Effets de la cellule")?;
            }
            "condition" => {
                if prompts::confirm(terminal, "La cellule doit-elle avoir une condition?", true)? {
                    ensure_object(&mut cell).insert(
                        "conditions".into(),
                        build_conditional(terminal, document, 0)?,
                    );
                } else {
                    ensure_object(&mut cell).remove("conditions");
                }
            }
            "text" => {
                let current = cell.get("texte").and_then(Value::as_str);
                match prompt_optional_multiline(terminal, "Texte de la cellule", current)? {
                    Some(text) => {
                        ensure_object(&mut cell).insert("texte".into(), Value::String(text));
                    }
                    None => {
                        ensure_object(&mut cell).remove("texte");
                    }
                }
            }
            "images" => {
                let images = ensure_object(&mut cell)
                    .entry("images")
                    .or_insert_with(|| json!([]));
                manage_image_references(
                    terminal,
                    document,
                    images,
                    "Images de la cellule",
                    true,
                )?;
            }
            "json" => {
                if let Some(updated) = prompts::edit_json(terminal, "Cellule", &cell)? {
                    cell = updated;
                }
            }
            "done" => break,
            _ => {}
        }
    }
    Ok(cell)
}

/// Opens an interactive manager for condition effects.
///
/// Runs until the user closes the manager. The returned Boolean reports whether the
/// managed container changed.
fn manage_condition_effects(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    container: &mut Value,
) -> Result<bool> {
    if !container.is_array() {
        *container = json!([]);
    }
    let mut changed = false;
    loop {
        let mut options = container
            .as_array()
            .expect("La valeur Json doit être un tableau")
            .iter()
            .enumerate()
            .map(|(index, effect)| {
                let kind = effect.get("type").and_then(Value::as_str).unwrap_or("?");
                (format!("effect-{index}"), kind.replace('_', " "))
            })
            .collect::<Vec<_>>();
        options.extend([
            ("add".into(), "＋ Ajouter un modificateur".into()),
            ("done".into(), "← Terminer".into()),
        ]);
        let choice = prompts::select(terminal, "Modificateurs de l'état", &options, false)?
            .expect("selection");
        let key = &options[choice].0;
        if key == "done" {
            break;
        }
        if key == "add" {
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(build_condition_effect(terminal, document)?);
            changed = true;
            continue;
        }

        let index = key.trim_start_matches("effect-").parse::<usize>()?;
        let actions = vec![
            ("edit".into(), "Modifier avec l'assistant".into()),
            ("duplicate".into(), "Dupliquer".into()),
            ("json".into(), "Modifier en JSON".into()),
            ("top".into(), "Déplacer tout en haut".into()),
            ("up".into(), "Monter".into()),
            ("down".into(), "Descendre".into()),
            ("bottom".into(), "Déplacer tout en bas".into()),
            ("delete".into(), "Supprimer".into()),
        ];
        let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
            continue;
        };
        match actions[action].0.as_str() {
            "edit" => {
                let current = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")[index]
                    .clone();
                container
                    .as_array_mut()
                    .expect("La valeur Json doit être un tableau")[index] =
                    edit_condition_effect_guided(terminal, document, current)?;
                changed = true;
            }
            "duplicate" => {
                forge_ops::duplicate_array_entry(container, index)?;
                changed = true;
            }
            "json" => {
                let current = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")[index]
                    .clone();
                if let Some(updated) = prompts::edit_json(terminal, "Effet d'état", &current)? {
                    container
                        .as_array_mut()
                        .expect("La valeur Json doit être un tableau")[index] = updated;
                    changed = true;
                }
            }
            "top" => {
                forge_ops::move_array_entry(container, index, 0)?;
                changed = true;
            }
            "up" if index > 0 => {
                forge_ops::move_array_entry(container, index, index - 1)?;
                changed = true;
            }
            "down"
                if index + 1
                    < container
                        .as_array()
                        .expect("La valeur Json doit être un tableau")
                        .len() =>
            {
                forge_ops::move_array_entry(container, index, index + 1)?;
                changed = true;
            }
            "bottom" => {
                let last = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")
                    .len()
                    .saturating_sub(1);
                forge_ops::move_array_entry(container, index, last)?;
                changed = true;
            }
            "delete" => {
                if prompts::confirm(terminal, "Supprimer ce modificateur?", false)? {
                    container
                        .as_array_mut()
                        .expect("La valeur Json doit être un tableau")
                        .remove(index);
                    changed = true;
                }
            }
            _ => {}
        }
    }
    Ok(changed)
}

/// Chooses a static page or accepts a formula-backed destination string.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn choose_connection_destination(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    current: Option<&str>,
) -> Result<Option<String>> {
    let options = vec![
        ("page".into(), "Choisir une page existante".into()),
        (
            "formula".into(),
            "Saisir un identifiant ou une formule".into(),
        ),
    ];
    let Some(choice) = prompts::select(terminal, "Type de destination", &options, true)? else {
        return Ok(None);
    };
    if options[choice].0.as_str() == "page" {
        choose_page(terminal, document, "Destination", true)
    } else {
        let value = prompts::prompt_line(terminal, "Destination (ex. ${prochaine_page})", current)?;
        Ok(Some(value))
    }
}

/// Prompts for and constructs one connection object.
///
/// Returns normalized raw JSON but does not insert it into the document; the caller
/// chooses where the new value belongs.
fn build_connection(terminal: &mut AppTerminal, document: &TaleDocument) -> Result<Value> {
    let text = prompts::prompt_line(terminal, "Texte de la connexion", Some("Continuer"))?;
    let destination = choose_connection_destination(terminal, document, None)?.unwrap_or_default();
    let mut connection = json!({
        "texte": text,
        "destination": destination,
        "cellules": []
    });
    let object = connection.as_object_mut().unwrap();
    if prompts::confirm(terminal, "Ajouter des conditions d'accès?", false)? {
        object.insert(
            "conditions".into(),
            build_conditional(terminal, document, 0)?,
        );
    }
    if prompts::confirm(terminal, "Ajouter un test?", false)? {
        object.insert("test".into(), build_check(terminal, document)?);
    }
    if prompts::confirm(terminal, "Ajouter des cellules de connexion?", false)? {
        let mut cells = json!([]);
        manage_cells(terminal, document, &mut cells, "Cellules de connexion")?;
        object.insert("cellules".into(), cells);
    }
    if prompts::confirm(
        terminal,
        "Ajouter des tags à transmettre à la page de destination?",
        false,
    )? {
        let mut tags = json!({});
        manage_json_object(
            terminal,
            "Tags immuables transmis dans `connexion`",
            &mut tags,
            true,
        )?;
        object.insert("tags".into(), tags);
    }
    Ok(connection)
}

/// Edits one normalized connection object through guided prompts.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
fn edit_connection_guided(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    mut connection: Value,
) -> Result<Value> {
    let object = ensure_object(&mut connection);
    let options = vec![
        ("texte".into(), "Texte".into()),
        ("destination".into(), "Destination".into()),
        ("conditions".into(), "Conditions".into()),
        ("test".into(), "Test".into()),
        (
            "tags".into(),
            "Tags transmis à la page de destination".into(),
        ),
    ];
    let Some(choice) = prompts::select(terminal, "Modifier la connexion", &options, true)? else {
        return Ok(connection);
    };
    match options[choice].0.as_str() {
        "texte" => {
            let current = object.get("texte").and_then(Value::as_str).unwrap_or("");
            object.insert(
                "texte".into(),
                Value::String(prompts::prompt_line(terminal, "Texte", Some(current))?),
            );
        }
        "destination" => {
            let current = object.get("destination").and_then(Value::as_str);
            if let Some(id) = choose_connection_destination(terminal, document, current)? {
                object.insert("destination".into(), Value::String(id));
            }
        }
        "conditions" => {
            if prompts::confirm(
                terminal,
                "La connexion doit-elle avoir des conditions?",
                true,
            )? {
                object.insert(
                    "conditions".into(),
                    build_conditional(terminal, document, 0)?,
                );
            } else {
                object.remove("conditions");
            }
        }
        "test" => {
            if prompts::confirm(terminal, "La connexion doit-elle avoir un test?", true)? {
                object.insert("test".into(), build_check(terminal, document)?);
            } else {
                object.remove("test");
            }
        }
        "tags" => {
            let tags = object.entry("tags").or_insert_with(|| json!({}));
            manage_json_object(
                terminal,
                "Tags immuables transmis dans `connexion`",
                tags,
                true,
            )?;
        }
        _ => {}
    }
    Ok(connection)
}

/// Copies or moves one cell between the global `avant` and `apres` phases.
fn transfer_cell_phase(
    terminal: &mut AppTerminal,
    pre: &mut Value,
    post: &mut Value,
) -> Result<bool> {
    let phases = vec![
        ("avant".into(), "Cellules avant".into()),
        ("apres".into(), "Cellules après".into()),
    ];
    let Some(source_choice) = prompts::select(terminal, "Phase source", &phases, true)? else {
        return Ok(false);
    };
    let source_is_pre = phases[source_choice].0 == "avant";
    let source = if source_is_pre { &*pre } else { &*post };
    let Some(index) = select_cell_index(terminal, "Cellule à transférer", source)? else {
        return Ok(false);
    };
    let actions = vec![
        ("move".into(), "Déplacer".into()),
        ("copy".into(), "Copier".into()),
    ];
    let Some(action) = prompts::select(terminal, "Mode de transfert", &actions, true)? else {
        return Ok(false);
    };
    let copy = actions[action].0 == "copy";
    if source_is_pre {
        forge_ops::transfer_array_entry(pre, post, index, copy)?;
    } else {
        forge_ops::transfer_array_entry(post, pre, index, copy)?;
    }
    Ok(true)
}

/// Prompts for one cell in a raw cell array.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn select_cell_index(
    terminal: &mut AppTerminal,
    title: &str,
    container: &Value,
) -> Result<Option<usize>> {
    let Some(values) = container.as_array() else {
        return Ok(None);
    };
    if values.is_empty() {
        prompts::pause(terminal, "Cette phase ne contient aucune cellule.")?;
        return Ok(None);
    }
    let options = values
        .iter()
        .enumerate()
        .map(|(index, value)| (index.to_string(), schema::cell_summary(value, index)))
        .collect::<Vec<_>>();
    Ok(prompts::select(terminal, title, &options, true)?)
}

/// Edits one atomic page effect through guided prompts.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
fn edit_effect_guided(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    mut effect: Value,
) -> Result<Value> {
    loop {
        let kind = effect
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if kind.is_empty() {
            return Ok(prompts::edit_json(terminal, "Effet", &effect)?.unwrap_or(effect));
        }
        let mut options = effect_fields(&kind)
            .into_iter()
            .map(|(key, label)| (key.to_owned(), label.to_owned()))
            .collect::<Vec<_>>();
        options.extend([
            (
                "change_type".into(),
                "Changer le type et reconstruire".into(),
            ),
            ("json".into(), "Modifier tout l'effet en JSON".into()),
            ("done".into(), "← Terminer".into()),
        ]);
        let Some(choice) = prompts::select(
            terminal,
            &format!("Modifier: {}", schema::effect_summary(&effect)),
            &options,
            false,
        )?
        else {
            continue;
        };
        let field = options[choice].0.as_str();
        if field == "done" {
            break;
        }
        if field == "change_type" {
            if prompts::confirm(
                terminal,
                "Changer le type reconstruira les champs connus. Continuer?",
                false,
            )? {
                effect = build_effect(terminal, document)?;
            }
            continue;
        }
        if field == "json" {
            if let Some(updated) = prompts::edit_json(terminal, "Effet", &effect)? {
                effect = updated;
            }
            continue;
        }
        edit_effect_field(terminal, document, &mut effect, &kind, field)?;
    }
    Ok(effect)
}

/// Returns the editable fields exposed by the guided editor for an effect type.
fn effect_fields(kind: &str) -> Vec<(&'static str, &'static str)> {
    let mut fields = match kind {
        "ajouter_etat" => vec![("etat", "État"), ("donnees", "Métadonnées")],
        "retirer_etat" => vec![("filtre", "Filtre"), ("selection", "Sélection")],
        "modifier_etat" => vec![
            ("filtre", "Filtre"),
            ("donnees", "Métadonnées"),
            ("selection", "Sélection"),
        ],
        "definir_tag" => vec![
            ("filtre", "Filtre"),
            ("chemin", "Chemin imbriqué"),
            ("creer_chemin", "Créer les entrées intermédiaires"),
            ("valeur", "Valeur"),
            ("selection", "Sélection"),
        ],
        "incrementer_tag" => vec![
            ("filtre", "Filtre"),
            ("chemin", "Chemin imbriqué"),
            ("creer_chemin", "Créer les entrées intermédiaires"),
            ("valeur", "Montant"),
            ("selection", "Sélection"),
        ],
        "retirer_tag" => vec![("filtre", "Filtre"), ("selection", "Sélection")],
        "ajouter_objet" => vec![
            ("objet", "Objet"),
            ("quantite", "Quantité"),
            ("donnees", "Métadonnées"),
        ],
        "retirer_objet" => vec![
            ("filtre", "Filtre"),
            ("quantite", "Quantité"),
            ("selection", "Sélection"),
        ],
        "modifier_objet" => vec![
            ("filtre", "Filtre"),
            ("donnees", "Métadonnées"),
            ("selection", "Sélection"),
        ],
        "modifier_attribut" | "definir_attribut" => {
            vec![("attribut", "Attribut"), ("valeur", "Valeur")]
        }
        "modifier_nom" => vec![("valeur", "Nouveau nom")],
        "modifier_pv" => vec![("valeur", "Montant"), ("defaite", "Défaite")],
        "definir_pv" | "definir_pv_max" | "modifier_pv_max" | "definir_moral"
        | "definir_moral_max" | "modifier_moral" | "modifier_moral_max" => {
            vec![("valeur", "Valeur"), ("defaite", "Défaite")]
        }
        "demander" => vec![
            ("tag", "Tag de destination"),
            ("question", "Question"),
            ("donnees", "Type de données"),
            ("defaut", "Valeur par défaut"),
            ("choix", "Choix proposés"),
        ],
        "rediriger" => vec![("destination", "Page de destination")],
        _ => Vec::new(),
    };
    fields.push(("notification", "Notification"));
    fields
}

/// Edits one field of an atomic effect according to its schema.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
fn edit_effect_field(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    effect: &mut Value,
    kind: &str,
    field: &str,
) -> Result<()> {
    let current = effect.get(field).cloned();
    let object = ensure_object(effect);
    match field {
        "notification" => {
            let modes = vec![
                ("default".into(), "Notification par défaut".into()),
                ("custom".into(), "Notification personnalisée".into()),
                ("silent".into(), "Aucune notification".into()),
            ];
            let Some(choice) = prompts::select(terminal, "Mode de notification", &modes, true)?
            else {
                return Ok(());
            };
            match modes[choice].0.as_str() {
                "default" => {
                    object.remove("notification");
                }
                "silent" => {
                    object.insert("notification".into(), Value::String(String::new()));
                }
                "custom" => {
                    let template = prompts::edit_multiline(
                        terminal,
                        "Notification personnalisée",
                        current.as_ref().and_then(Value::as_str).unwrap_or(""),
                    )?;
                    object.insert("notification".into(), Value::String(template));
                }
                _ => {}
            }
        }
        "etat" => {
            if let Some(id) = choose_entity(terminal, document, "etats", "État", true)? {
                object.insert(field.into(), Value::String(id));
            }
        }
        "objet" => {
            if let Some(id) = choose_entity(terminal, document, "objets", "Objet", true)? {
                object.insert(field.into(), Value::String(id));
            }
        }
        "attribut" => {
            if let Some(attribute) = choose_attribute(terminal, document)? {
                object.insert(field.into(), Value::String(attribute));
            }
        }
        "filtre" => {
            let family = match kind {
                "retirer_etat" | "modifier_etat" => FilterFamily::Condition,
                "definir_tag" | "incrementer_tag" | "retirer_tag" => FilterFamily::Tag,
                _ => FilterFamily::Item,
            };
            object.insert(field.into(), build_filter(terminal, document, family, 0)?);
        }
        "donnees" if kind == "demander" => {
            object.insert(
                field.into(),
                Value::String(prompt_poll_data_type_current(
                    terminal,
                    current.as_ref().and_then(Value::as_str),
                )?),
            );
            /*
            // remove default and choices that are no longer valid for the new data type
            let dtype = object
                .get("donnees")
                .and_then(Value::as_str)
                .unwrap_or("null").to_string();
            if let Some(default) = object.get("defaut").cloned() {
                let new_default = ensure_type(default, &dtype);
                if let Ok(new_default) = new_default {
                    object.insert("defaut".into(), new_default);
                } else {
                    object.remove("defaut");
                }
            }
            if let Some(choices) = object.get("choix").cloned() {
                let new_choices = choices.as_array()
                    .ok_or_else(|| anyhow!("Les choix devraient être une liste"))?
                    .into_iter()
                    .filter_map(|val| ensure_type(val.clone(), &dtype).ok())
                    .collect::<Vec<_>>();
                if new_choices.is_empty() {
                    object.remove("choix");
                } else {
                    object.insert("choix".into(), Value::Array(new_choices));
                }
            }
            */
        }
        "donnees" => {
            let value = prompt_object_or_formula(terminal, "Métadonnées", current.as_ref())?;
            object.insert(field.into(), value);
        }
        "selection" => {
            object.insert(
                field.into(),
                Value::String(prompt_selection_current(
                    terminal,
                    current.as_ref().and_then(Value::as_str),
                )?),
            );
        }
        "defaite" => {
            if prompts::confirm(
                terminal,
                "Ajouter une redirection de défaite?",
                current.is_some(),
            )? {
                if let Some(page) = choose_page(terminal, document, "Page de défaite", true)? {
                    object.insert(field.into(), Value::String(page));
                }
            } else {
                object.remove(field);
            }
        }
        "tag" | "question" => {
            let value = prompts::prompt_line(
                terminal,
                if field == "tag" {
                    "Tag de destination"
                } else {
                    "Question"
                },
                current.as_ref().and_then(Value::as_str),
            )?;
            object.insert(field.into(), Value::String(value));
        }
        "valeur" if kind == "modifier_nom" => {
            let value = prompts::prompt_line(
                terminal,
                "Nouveau nom ou formule",
                current.as_ref().and_then(Value::as_str),
            )?;
            object.insert(field.into(), Value::String(value));
        }
        "defaut" => {
            if prompts::confirm(
                terminal,
                "Conserver une valeur par défaut?",
                current.is_some(),
            )? {
                let dtype = object
                    .get("donnees")
                    .and_then(Value::as_str)
                    .unwrap_or("null");
                object.insert(
                    field.into(),
                    ensure_type(
                        prompts::prompt_json_value(
                            terminal,
                            "Valeur par défaut",
                            current.as_ref(),
                        )?,
                        dtype,
                    )?,
                );
            } else {
                object.remove(field);
            }
        }
        "choix" => {
            let dtype = object
                .get("donnees")
                .and_then(Value::as_str)
                .unwrap_or("null");
            let mut modes = if current.as_ref().is_some_and(Value::is_string) {
                vec![
                    ("formula".into(), "Formule retournant une liste".into()),
                    ("list".into(), "Liste de choix structurés".into()),
                    ("none".into(), "Aucun choix prédéfini".into()),
                ]
            } else {
                vec![
                    ("list".into(), "Liste de choix structurés".into()),
                    ("formula".into(), "Formule retournant une liste".into()),
                    ("none".into(), "Aucun choix prédéfini".into()),
                ]
            };
            let Some(mode) = prompts::select(terminal, "Source des choix", &modes, true)? else {
                return Ok(());
            };
            match modes.swap_remove(mode).0.as_str() {
                "formula" => {
                    let formula = prompts::prompt_line(
                        terminal,
                        "Formule retournant les choix du sondage",
                        current.as_ref().and_then(Value::as_str).or(Some("${[]}")),
                    )?;
                    object.insert(field.into(), Value::String(formula));
                }
                "list" => {
                    let mut choices = current.filter(Value::is_array).unwrap_or_else(|| json!([]));
                    manage_poll_choices(terminal, &mut choices, dtype)?;
                    object.insert(field.into(), choices);
                }
                "none" => {
                    object.remove(field);
                }
                _ => unreachable!(),
            }
        }
        "chemin" => {
            if prompts::confirm(
                terminal,
                "Cibler une valeur imbriquée dans le tag?",
                current.is_some(),
            )? {
                object.insert(
                    field.into(),
                    prompts::prompt_json_value(
                        terminal,
                        "Chemin (liste ou formule retournant une liste)",
                        current.as_ref(),
                    )?,
                );
            } else {
                object.remove(field);
                object.remove("creer_chemin");
            }
        }
        "creer_chemin" => {
            if object.contains_key("chemin")
                && prompts::confirm(
                    terminal,
                    "Créer automatiquement les entrées intermédiaires manquantes?",
                    current.as_ref().and_then(Value::as_bool).unwrap_or(false),
                )?
            {
                object.insert(field.into(), Value::Bool(true));
            } else {
                object.remove(field);
            }
        }
        "valeur" if kind == "definir_tag" => {
            object.insert(
                field.into(),
                prompt_typed_tag_value(terminal, current.as_ref())?,
            );
        }
        "valeur" | "quantite" => {
            object.insert(
                field.into(),
                prompts::prompt_json_value(terminal, "Valeur ou formule", current.as_ref())?,
            );
        }
        "destination" => {
            let value = choose_page(terminal, document, "Page de destination", true)?;
            if let Some(value) = value {
                object.insert(field.into(), Value::String(value));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Edits condition effect guided.
///
/// Mutates the supplied document or value in place. Previously accepted prompt edits
/// may remain if a later prompt is cancelled.
fn edit_condition_effect_guided(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    mut effect: Value,
) -> Result<Value> {
    loop {
        let kind = effect
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let mut options = vec![
            ("valeur".into(), "Valeur".into()),
            ("categorie".into(), "Catégorie".into()),
        ];
        if kind == "modificateur_attribut" {
            options.push(("attribut".into(), "Attribut".into()));
        }
        if kind == "modificateur_test" {
            options.push(("filtre".into(), "Filtre de test".into()));
        }
        options.extend([
            (
                "change_type".into(),
                "Changer le type et reconstruire".into(),
            ),
            ("json".into(), "Modifier en JSON".into()),
            ("done".into(), "← Terminer".into()),
        ]);
        let Some(choice) = prompts::select(terminal, "Modifier le modificateur", &options, false)?
        else {
            continue;
        };
        match options[choice].0.as_str() {
            "done" => break,
            "change_type" => effect = build_condition_effect(terminal, document)?,
            "json" => {
                if let Some(updated) = prompts::edit_json(terminal, "Modificateur", &effect)? {
                    effect = updated;
                }
            }
            "valeur" => {
                let current = effect.get("valeur").cloned();
                ensure_object(&mut effect).insert(
                    "valeur".into(),
                    prompts::prompt_json_value(terminal, "Valeur ou formule", current.as_ref())?,
                );
            }
            "categorie" => {
                let current = effect.get("categorie").and_then(Value::as_str);
                match prompts::prompt_optional_line(terminal, "Catégorie", current)? {
                    Some(category) => {
                        ensure_object(&mut effect)
                            .insert("categorie".into(), Value::String(category));
                    }
                    None => {
                        ensure_object(&mut effect).remove("categorie");
                    }
                }
            }
            "attribut" => {
                if let Some(attribute) = choose_attribute(terminal, document)? {
                    ensure_object(&mut effect).insert("attribut".into(), Value::String(attribute));
                }
            }
            "filtre" => {
                if prompts::confirm(terminal, "Limiter par un filtre de test?", true)? {
                    ensure_object(&mut effect).insert(
                        "filtre".into(),
                        build_filter(terminal, document, FilterFamily::Check, 0)?,
                    );
                } else {
                    ensure_object(&mut effect).remove("filtre");
                }
            }
            _ => {}
        }
    }
    Ok(effect)
}

/// Opens an interactive manager for poll choices.
///
/// Runs until the user closes the manager. The returned Boolean reports whether the
/// managed container changed.
fn manage_poll_choices(
    terminal: &mut AppTerminal,
    container: &mut Value,
    dtype: &str,
) -> Result<bool> {
    if !container.is_array() {
        *container = json!([]);
    }
    let mut changed = false;
    loop {
        let mut options = container
            .as_array()
            .expect("La valeur Json doit être un tableau")
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let text = choice.get("texte").and_then(Value::as_str).unwrap_or("?");
                let value = choice
                    .get("valeur")
                    .map(prompts::json_compact)
                    .unwrap_or_else(|| "null".into());
                (
                    format!("choice-{index}"),
                    format!("{} → {}", truncate_label(text, 42), value),
                )
            })
            .collect::<Vec<_>>();
        options.extend([
            ("add".into(), "＋ Ajouter un choix".into()),
            ("done".into(), "← Terminer".into()),
        ]);
        let choice =
            prompts::select(terminal, "Choix du sondage", &options, false)?.expect("selection");
        let key = options[choice].0.as_str();
        if key == "done" {
            break;
        }
        if key == "add" {
            container
                .as_array_mut()
                .expect("La valeur Json doit être un tableau")
                .push(build_poll_choice(terminal, dtype)?);
            changed = true;
            continue;
        }
        let index = key.trim_start_matches("choice-").parse::<usize>()?;
        let actions = vec![
            ("edit".into(), "Modifier".into()),
            ("duplicate".into(), "Dupliquer".into()),
            ("up".into(), "Monter".into()),
            ("down".into(), "Descendre".into()),
            ("delete".into(), "Supprimer".into()),
        ];
        let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
            continue;
        };
        match actions[action].0.as_str() {
            "edit" => {
                let current = container
                    .as_array()
                    .expect("La valeur Json doit être un tableau")[index]
                    .clone();
                container
                    .as_array_mut()
                    .expect("La valeur Json doit être un tableau")[index] =
                    edit_poll_choice(terminal, current, dtype)?;
                changed = true;
            }
            "duplicate" => {
                forge_ops::duplicate_array_entry(container, index)?;
                changed = true;
            }
            "up" if index > 0 => {
                forge_ops::move_array_entry(container, index, index - 1)?;
                changed = true;
            }
            "down"
                if index + 1
                    < container
                        .as_array()
                        .expect("La valeur Json doit être un tableau")
                        .len() =>
            {
                forge_ops::move_array_entry(container, index, index + 1)?;
                changed = true;
            }
            "delete" => {
                container
                    .as_array_mut()
                    .expect("La valeur Json doit être un tableau")
                    .remove(index);
                changed = true;
            }
            _ => {}
        }
    }
    Ok(changed)
}
/// Prompts for one poll label/value pair whose value must match `dtype`.
///
/// # Errors
///
/// Propagates prompt or type-conversion errors.
fn build_poll_choice(terminal: &mut AppTerminal, dtype: &str) -> Result<Value> {
    Ok(json!({
        "texte": prompts::prompt_line(terminal, "Libellé", None)?,
        "valeur": ensure_type(prompts::prompt_json_value(terminal, "Valeur ou formule", None)?, dtype)?
    }))
}
/// Edits an existing poll choice and returns its normalized JSON representation.
///
/// # Errors
///
/// Propagates prompt or type-conversion errors.
fn edit_poll_choice(terminal: &mut AppTerminal, mut choice: Value, dtype: &str) -> Result<Value> {
    let text = choice
        .get("texte")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let value = choice.get("valeur").cloned();
    let object = ensure_object(&mut choice);
    object.insert(
        "texte".into(),
        Value::String(prompts::prompt_line(terminal, "Libellé", Some(&text))?),
    );
    object.insert(
        "valeur".into(),
        ensure_type(
            prompts::prompt_json_value(terminal, "Valeur ou formule", value.as_ref())?,
            dtype,
        )?,
    );
    Ok(choice)
}

/// Opens a JSON-object editor and optionally requests an explicit type for each value.
///
/// Runs until the user closes the manager. The returned Boolean reports whether the
/// managed container changed.
fn manage_json_object(
    terminal: &mut AppTerminal,
    title: &str,
    container: &mut Value,
    typed_values: bool,
) -> Result<bool> {
    if !container.is_object() {
        *container = json!({});
    }
    let mut changed = false;
    loop {
        let mut options = container
            .as_object()
            .expect("object")
            .iter()
            .map(|(key, value)| {
                (
                    format!("key:{key}"),
                    format!(
                        "{key}: {}",
                        truncate_label(&prompts::json_compact(value), 52)
                    ),
                )
            })
            .collect::<Vec<_>>();
        options.extend([
            ("add".into(), "＋ Ajouter une entrée".into()),
            ("json".into(), "Modifier l'objet complet en JSON".into()),
            ("done".into(), "← Terminer".into()),
        ]);
        let choice = prompts::select(terminal, title, &options, false)?.expect("selection");
        let selected = options[choice].0.clone();
        match selected.as_str() {
            "done" => break,
            "add" => {
                let key = prompts::prompt_line(terminal, "Clé", None)?;
                if !key.is_empty() {
                    let value = if typed_values {
                        prompt_typed_tag_value(terminal, None)?
                    } else {
                        prompts::prompt_json_value(terminal, "Valeur ou formule", None)?
                    };
                    container
                        .as_object_mut()
                        .expect("object")
                        .insert(key, value);
                    changed = true;
                }
            }
            "json" => {
                if let Some(updated) = prompts::edit_json(terminal, title, container)? {
                    if updated.is_object() {
                        *container = updated;
                        changed = true;
                    }
                }
            }
            _ => {
                let key = selected.trim_start_matches("key:").to_owned();
                let actions = vec![
                    ("edit".into(), "Modifier la valeur".into()),
                    ("rename".into(), "Renommer la clé".into()),
                    ("duplicate".into(), "Dupliquer sous une autre clé".into()),
                    ("delete".into(), "Supprimer".into()),
                ];
                let Some(action) = prompts::select(terminal, "Action", &actions, true)? else {
                    continue;
                };
                let current = container
                    .as_object()
                    .and_then(|object| object.get(&key))
                    .cloned();
                match actions[action].0.as_str() {
                    "edit" => {
                        if let Some(current) = current.as_ref() {
                            let value = if typed_values {
                                prompt_typed_tag_value(terminal, Some(current))?
                            } else {
                                prompts::prompt_json_value(
                                    terminal,
                                    "Valeur ou formule",
                                    Some(current),
                                )?
                            };
                            container
                                .as_object_mut()
                                .expect("object")
                                .insert(key, value);
                            changed = true;
                        }
                    }
                    "rename" => {
                        let new_key = prompts::prompt_line(terminal, "Nouvelle clé", Some(&key))?;
                        if new_key != key
                            && !new_key.is_empty()
                            && !container
                                .as_object()
                                .expect("object")
                                .contains_key(&new_key)
                        {
                            if let Some(value) =
                                container.as_object_mut().expect("object").remove(&key)
                            {
                                container
                                    .as_object_mut()
                                    .expect("object")
                                    .insert(new_key, value);
                                changed = true;
                            }
                        }
                    }
                    "duplicate" => {
                        let new_key = prompts::prompt_line(terminal, "Clé de la copie", None)?;
                        if let Some(value) = current {
                            if !new_key.is_empty()
                                && !container
                                    .as_object()
                                    .expect("object")
                                    .contains_key(&new_key)
                            {
                                container
                                    .as_object_mut()
                                    .expect("object")
                                    .insert(new_key, value);
                                changed = true;
                            }
                        }
                    }
                    "delete" => {
                        if prompts::confirm(terminal, &format!("Supprimer `{key}`?"), false)? {
                            container.as_object_mut().expect("object").remove(&key);
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(changed)
}

/// Prompts the user for selection current.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_selection_current(terminal: &mut AppTerminal, current: Option<&str>) -> Result<String> {
    let options = vec![
        ("tous".into(), "Tous les éléments correspondants".into()),
        ("premier".into(), "Seulement le premier".into()),
    ];
    let initial = usize::from(current == Some("premier"));
    let index =
        prompts::select_with_initial(terminal, "Mode de sélection", &options, initial, false)?
            .expect("selection");
    Ok(options[index].0.clone())
}

/// Prompts the user for poll data type current.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_poll_data_type_current(
    terminal: &mut AppTerminal,
    current: Option<&str>,
) -> Result<String> {
    let options = vec![
        ("str".into(), "Chaîne de caractères".into()),
        ("int".into(), "Entier".into()),
        ("float".into(), "Flottant".into()),
        ("bool".into(), "Booléen".into()),
    ];
    let initial = options
        .iter()
        .position(|(id, _): &(String, String)| Some(id.as_str()) == current)
        .unwrap_or(0);
    let index = prompts::select_with_initial(terminal, "Type de données", &options, initial, true)?
        .unwrap_or(initial);
    Ok(options[index].0.clone())
}

/// Prompts the user for optional multiline.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_optional_multiline(
    terminal: &mut AppTerminal,
    title: &str,
    current: Option<&str>,
) -> Result<Option<String>> {
    if current.is_none() && !prompts::confirm(terminal, "Ajouter un texte?", false)? {
        return Ok(None);
    }
    let value = prompts::edit_multiline(terminal, title, current.unwrap_or(""))?;
    Ok((!value.is_empty()).then_some(value))
}

/// Truncates a label to a maximum number of Unicode scalar values.
fn truncate_label(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let text = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{text}…")
    } else {
        text
    }
}

/// Prompts for a tag value after selecting its intended runtime data type.
///
/// Formula strings remain valid for every selected type because their result is
/// checked only when the tale runs. Literal lists and dictionaries are entered
/// as JSON, which prevents the Forge from silently turning them into strings.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_typed_tag_value(terminal: &mut AppTerminal, current: Option<&Value>) -> Result<Value> {
    let options = vec![
        ("str".into(), "Chaîne (str)".into()),
        ("int".into(), "Entier (int)".into()),
        ("float".into(), "Flottant (float)".into()),
        ("bool".into(), "Booléen (bool)".into()),
        ("list".into(), "Liste (list)".into()),
        ("dict".into(), "Objet (dict)".into()),
        ("null".into(), "Valeur nulle (null)".into()),
    ];
    let current_type = current.map(json_data_type).unwrap_or("str");
    let initial = options
        .iter()
        .position(|(kind, _)| kind == current_type)
        .unwrap_or(0);
    let choice =
        prompts::select_with_initial(terminal, "Type de données du tag", &options, initial, false)?
            .expect("la liste des types de tag n'est jamais vide");
    let expected = options[choice].0.as_str();
    if expected == "null" {
        return Ok(Value::Null);
    }
    let value = prompts::prompt_json_value(terminal, "Valeur ou formule", current)?;
    ensure_type(value, expected)
}

/// Returns the Forge data-type key that best describes one literal JSON value.
fn json_data_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(number) if number.is_i64() || number.is_u64() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}
/// Converts a literal Forge value to `expected` while leaving formula strings untouched for runtime evaluation.
///
/// # Errors
///
/// Returns an error when the requested type is unknown or the literal cannot be converted safely.
pub fn ensure_type(val: Value, expected: &str) -> Result<Value> {
    match (expected, &val) {
        ("str" | "chaine", Value::String(_)) => Ok(val),
        ("int" | "entier", Value::Number(n)) => Ok(Value::Number(
            n.as_i64()
                .ok_or_else(|| anyhow!("S'attendait à un entier, a reçu {val}"))?
                .into(),
        )),
        ("float" | "flottant", Value::Number(n)) => Ok(Value::Number(
            Number::from_f64(
                n.as_f64()
                    .ok_or_else(|| anyhow!("S'attendait à un flottant, a reçu {val}"))?,
            )
            .ok_or_else(|| anyhow!("S'attendait à un flottant, a reçu {val}"))?,
        )),
        ("bool" | "booleen", Value::Bool(_)) => Ok(val),
        ("list" | "liste", Value::Array(_)) => Ok(val),
        ("dict" | "objet", Value::Object(_)) => Ok(val),
        ("str" | "chaine", Value::Number(_n)) => Ok(Value::String(val.to_string())),
        ("int" | "entier", Value::String(s)) => {
            let n = s.parse::<i64>().ok().map(|n| Value::Number(n.into()));
            if let Some(n) = n {
                Ok(n)
            } else {
                if crate::validation::is_formula(&val) {
                    Ok(val)
                } else {
                    Err(anyhow!("S'il ne s'agit pas d'un entier, cette valeur devrait être une formule: {val}").into())
                }
            }
        }
        ("float" | "flottant", Value::String(s)) => {
            let n = s
                .parse::<f64>()
                .ok()
                .map(|n| {
                    Number::from_f64(n)
                        .ok_or_else(|| anyhow!("S'attendait à un flottant, a reçu {val}"))
                        .map(|n| Value::Number(n))
                })
                .transpose()?;
            if let Some(n) = n {
                Ok(n)
            } else {
                if crate::validation::is_formula(&val) {
                    Ok(val)
                } else {
                    Err(anyhow!("S'il ne s'agit pas d'un flottant, cette valeur devrait être une formule: {val}").into())
                }
            }
        }
        ("bool" | "booleen", Value::String(s)) => match s.to_lowercase().as_str() {
            "true" | "1" | "vrai" | "oui" | "yes" | "y" | "o" => Ok(Value::Bool(true)),
            "false" | "0" | "faux" | "non" | "no" | "n" => Ok(Value::Bool(false)),
            _ => {
                if is_formula(&val) {
                    Ok(val)
                } else {
                    Err(anyhow!("S'il ne s'agit pas d'un booléen, cette valeur devrait être une formule: {val}").into())
                }
            }
        },
        ("list" | "liste" | "dict" | "objet", Value::String(_)) if is_formula(&val) => Ok(val),
        ("null", Value::Null) => Ok(val),
        _ => Err(anyhow!("Type de donnée invalide: {expected}\nReçu {:?}", val).into()),
    }
}

/// Builds one atomic page effect through the Forge's guided prompts.
///
/// Returns normalized raw JSON but does not insert it into the document; the caller
/// chooses where the new value belongs.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn build_effect(terminal: &mut AppTerminal, document: &TaleDocument) -> Result<Value> {
    let options = EFFECT_TYPES
        .iter()
        .map(|(id, label)| ((*id).into(), (*label).into()))
        .collect::<Vec<_>>();
    let index = prompts::select(terminal, "Type d'effet", &options, false)?.expect("selection");
    let kind = options[index].0.as_str();
    let value = match kind {
        "ajouter_etat" => json!({
            "type": kind,
            "etat": choose_entity(terminal, document, "etats", "État à ajouter", true)?.unwrap_or_default(),
            "donnees": prompt_object_or_formula(terminal, "Métadonnées de l'instance", None)?
        }),
        "retirer_etat" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Condition, 0)?,
            "selection": prompt_selection(terminal)?
        }),
        "modifier_etat" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Condition, 0)?,
            "donnees": prompt_object_or_formula(terminal, "Métadonnées à remplacer", None)?,
            "selection": prompt_selection(terminal)?
        }),
        "definir_tag" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Tag, 0)?,
            "valeur": prompt_typed_tag_value(terminal, None)?,
            "selection": prompt_selection(terminal)?
        }),
        "incrementer_tag" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Tag, 0)?,
            "valeur": prompts::prompt_json_value(terminal, "Montant ou formule", Some(&json!(1)))?,
            "selection": prompt_selection(terminal)?
        }),
        "ajouter_objet" => json!({
            "type": kind,
            "objet": choose_entity(terminal, document, "objets", "Objet à ajouter", true)?.unwrap_or_default(),
            "quantite": prompts::prompt_json_value(terminal, "Quantité", Some(&json!(1)))?,
            "donnees": prompt_object_or_formula(terminal, "Métadonnées de la pile", None)?
        }),
        "retirer_objet" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Item, 0)?,
            "quantite": prompts::prompt_json_value(terminal, "Quantité", Some(&json!(1)))?,
            "selection": prompt_selection(terminal)?
        }),
        "modifier_objet" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Item, 0)?,
            "donnees": prompt_object_or_formula(terminal, "Métadonnées à remplacer", None)?,
            "selection": prompt_selection(terminal)?
        }),
        "modifier_pv" => resource_effect(terminal, document, kind, "valeur")?,
        "modifier_attribut" | "definir_attribut" => json!({
            "type": kind,
            "attribut": choose_attribute(terminal, document)?.unwrap_or_default(),
            "valeur": prompts::prompt_json_value(terminal, "Valeur ou formule", Some(&json!(0)))?
        }),
        "modifier_nom" => json!({
            "type": kind,
            "valeur": prompts::prompt_line(terminal, "Nouveau Nom", None)?
        }),
        "retirer_tag" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Tag, 0)?,
            "selection": prompt_selection(terminal)?
        }),
        "demander" => {
            let tag = prompts::prompt_line(terminal, "Tag de destination", None)?;
            let question = prompts::prompt_line(terminal, "Question à poser", None)?;
            let donnees = prompt_poll_data_type(terminal)?;
            let choix = prompt_poll_choices(terminal, &donnees)?;
            let defaut = if donnees != "bool"
                && prompts::confirm(terminal, "Ajouter une valeur par défaut?", false)?
            {
                let val = prompts::prompt_json_value(terminal, "Valeur par défaut", None).ok();
                if let Some(v) = val {
                    if Value::Null == v {
                        Some(Value::String(String::new()))
                    } else {
                        Some(ensure_type(v, &donnees)?)
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(defaut) = defaut {
                json!({
                    "type": kind,
                    "tag": tag,
                    "question": question,
                    "donnees": donnees,
                    "defaut": defaut,
                    "choix": choix,
                })
            } else {
                json!({
                    "type": kind,
                    "tag": tag,
                    "question": question,
                    "donnees": donnees,
                    "choix": choix,
                })
            }
        }
        "definir_pv" | "definir_pv_max" | "modifier_pv_max" | "definir_moral"
        | "definir_moral_max" | "modifier_moral" | "modifier_moral_max" => {
            resource_effect(terminal, document, kind, "valeur")?
        }
        _ => json!({"type": kind}),
    };
    Ok(value)
}

/// Builds a resource-adjustment effect with a selected effect type and numeric value.
fn resource_effect(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    kind: &str,
    value_key: &str,
) -> Result<Value> {
    let mut object = Map::new();
    object.insert("type".into(), Value::String(kind.into()));
    object.insert(
        value_key.into(),
        prompts::prompt_json_value(terminal, "Valeur ou formule", Some(&json!(0)))?,
    );
    if prompts::confirm(terminal, "Ajouter une redirection de défaite?", false)? {
        if let Some(page) = choose_page(terminal, document, "Page de défaite", true)? {
            object.insert("defaite".into(), Value::String(page));
        }
    }
    Ok(Value::Object(object))
}

/// Prompts for and constructs one conditional expression.
///
/// Returns normalized raw JSON but does not insert it into the document; the caller
/// chooses where the new value belongs.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
/// - `depth` — Current recursive nesting depth used to limit or indent editors.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn build_conditional(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    depth: usize,
) -> Result<Value> {
    if depth >= 8 {
        return Ok(json!({"type": "formule", "formule": "${True}"}));
    }
    let options = CONDITIONAL_TYPES
        .iter()
        .map(|(id, label)| ((*id).into(), (*label).into()))
        .collect::<Vec<_>>();
    let index =
        prompts::select(terminal, "Type de condition", &options, false)?.expect("selection");
    let kind = options[index].0.as_str();
    let value = match kind {
        "et" | "ou" => {
            let mut children = Vec::new();
            loop {
                children.push(build_conditional(terminal, document, depth + 1)?);
                if !prompts::confirm(terminal, "Ajouter une autre sous-condition?", false)? {
                    break;
                }
            }
            json!({"type": kind, "conditions": children})
        }
        "non" => json!({
            "type": kind,
            "condition": build_conditional(terminal, document, depth + 1)?
        }),
        "objet_requis" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Item, 0)?
        }),
        "tag_requis" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Tag, 0)?
        }),
        "etat_requis" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Condition, 0)?
        }),
        "attribut_egal" | "attribut_superieur" | "attribut_inferieur" => json!({
            "type": kind,
            "attribut": choose_attribute(terminal, document)?.unwrap_or_default(),
            "valeur": prompts::prompt_json_value(terminal, "Valeur ou formule", Some(&json!(0)))?
        }),
        "pv_egaux" | "pv_superieurs" | "pv_inferieurs" | "moral_egal" | "moral_superieur"
        | "moral_inferieur" => json!({
            "type": kind,
            "valeur": prompts::prompt_json_value(terminal, "Valeur ou formule", Some(&json!(0)))?
        }),
        "tag_page" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, FilterFamily::Tag, 0)?
        }),
        "aleatoire" => json!({
            "type": kind,
            "poids": prompts::prompt_json_value(terminal, "Poids en pourcentage", Some(&json!(50)))?
        }),
        "formule" => json!({
            "type": kind,
            "formule": prompts::prompt_line(terminal, "Formule", Some("${True}"))?
        }),
        _ => json!({"type": kind}),
    };
    Ok(value)
}

/// Prompts for and constructs one filter expression.
///
/// Returns normalized raw JSON but does not insert it into the document; the caller
/// chooses where the new value belongs.
///
/// # Arguments
///
/// - `terminal` — Active Forge terminal used to display prompts and read input.
/// - `document` — Raw tale document read or mutated by the operation.
/// - `family` — Filter family that determines the allowed filter variants.
/// - `depth` — Current recursive nesting depth used to limit or indent editors.
///
/// # Errors
///
/// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
/// invalid, or a delegated parser/evaluator fails.
pub fn build_filter(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    family: FilterFamily,
    depth: usize,
) -> Result<Value> {
    if depth >= 8 {
        return Ok(json!({"type": "formule", "formule": "${True}"}));
    }
    if family != FilterFamily::Check
        && prompts::confirm(
            terminal,
            &format!("Utiliser un {} réduit à un identifiant?", family.title()),
            true,
        )?
    {
        let id = match family {
            FilterFamily::Item => match choose_entity(terminal, document, "objets", "Objet", true)? {
                Some(id) => id,
                None => prompts::prompt_line(terminal, "Identifiant d'objet", None)?,
            },
            FilterFamily::Condition => match choose_entity(terminal, document, "etats", "État", true)? {
                Some(id) => id,
                None => prompts::prompt_line(terminal, "Identifiant d'état", None)?,
            },
            FilterFamily::Tag => prompts::prompt_line(terminal, "Identifiant du tag", None)?,
            FilterFamily::Check => unreachable!(),
        };
        return Ok(Value::String(id));
    }

    let options = family
        .types()
        .iter()
        .map(|(id, label)| ((*id).into(), (*label).into()))
        .collect::<Vec<_>>();
    let index = prompts::select(
        terminal,
        &format!("Type de {}", family.title()),
        &options,
        false,
    )?
    .expect("selection");
    let kind = options[index].0.as_str();
    let value = match kind {
        "et" | "ou" => {
            let mut children = Vec::new();
            loop {
                children.push(build_filter(terminal, document, family, depth + 1)?);
                if !prompts::confirm(terminal, "Ajouter un autre sous-filtre?", false)? {
                    break;
                }
            }
            json!({"type": kind, "filtres": children})
        }
        "non" => json!({
            "type": kind,
            "filtre": build_filter(terminal, document, family, depth + 1)?
        }),
        "id" => {
            let key = family.id_key().unwrap_or("id");
            let id = match family {
                FilterFamily::Item => {
                    choose_entity(terminal, document, "objets", "Objet", true)?.unwrap_or_default()
                }
                FilterFamily::Condition => {
                    choose_entity(terminal, document, "etats", "État", true)?.unwrap_or_default()
                }
                FilterFamily::Tag => prompts::prompt_line(terminal, "Identifiant du tag", None)?,
                FilterFamily::Check => String::new(),
            };
            let mut object = Map::new();
            object.insert("type".into(), Value::String(kind.into()));
            object.insert(key.into(), Value::String(id));
            Value::Object(object)
        }
        "donnee_egale" | "donnee_superieure" | "donnee_inferieure" => json!({
            "type": kind,
            "cle": prompts::prompt_line(terminal, "Clé de métadonnée", None)?,
            "valeur": prompts::prompt_json_value(terminal, "Valeur ou formule", None)?
        }),
        "quantite_egale" | "quantite_superieure" | "quantite_inferieure" => json!({
            "type": kind,
            "quantite": prompts::prompt_json_value(terminal, "Quantité ou formule", Some(&json!(1)))?
        }),
        "valeur_egale" | "valeur_superieure" | "valeur_inferieure" => json!({
            "type": kind,
            "valeur": prompts::prompt_json_value(terminal, "Valeur ou formule", None)?
        }),
        "attribut_parmi" => json!({
            "type": kind,
            "attribut": choose_attribute(terminal, document)?.unwrap_or_default()
        }),
        "tous_les_traits" | "un_des_traits" => json!({
            "type": kind,
            "traits": prompt_traits_value(terminal, "Traits")?
        }),
        "formule" => json!({
            "type": kind,
            "formule": prompts::prompt_line(terminal, "Formule", Some("${True}"))?
        }),
        _ => json!({"type": kind}),
    };
    Ok(value)
}

/// Prompts for and constructs one connection check object.
///
/// Returns normalized raw JSON but does not insert it into the document; the caller
/// chooses where the new value belongs.
fn build_check(terminal: &mut AppTerminal, document: &TaleDocument) -> Result<Value> {
    let attribute = choose_attribute(terminal, document)?.unwrap_or_default();
    let difficulty = prompts::prompt_json_value(terminal, "Difficulté", Some(&json!(10)))?;
    let traits = prompts::prompt_string_list(terminal, "Traits du test", &[])?;
    let failure = choose_optional_page(terminal, document, "Page d'échec", true)?;
    Ok(json!({
        "attribut": attribute,
        "difficulte": difficulty,
        "traits": traits,
        "echec": failure
    }))
}

/// Prompts for and constructs one persistent-state modifier.
///
/// Returns normalized raw JSON but does not insert it into the document; the caller
/// chooses where the new value belongs.
fn build_condition_effect(terminal: &mut AppTerminal, document: &TaleDocument) -> Result<Value> {
    let options = CONDITION_EFFECT_TYPES
        .iter()
        .map(|(id, label)| ((*id).into(), (*label).into()))
        .collect::<Vec<_>>();
    let index =
        prompts::select(terminal, "Type de modificateur", &options, false)?.expect("selection");
    let kind = options[index].0.as_str();
    let category = prompts::prompt_optional_line(terminal, "Catégorie", None)?;
    let value = prompts::prompt_json_value(terminal, "Valeur ou formule", Some(&json!(0)))?;
    let mut object = Map::new();
    object.insert("type".into(), Value::String(kind.into()));
    object.insert("valeur".into(), value);
    if let Some(category) = category {
        object.insert("categorie".into(), Value::String(category));
    }
    match kind {
        "modificateur_attribut" => {
            object.insert(
                "attribut".into(),
                Value::String(choose_attribute(terminal, document)?.unwrap_or_default()),
            );
        }
        "modificateur_test" => {
            if prompts::confirm(
                terminal,
                "Limiter ce modificateur par un filtre de test?",
                false,
            )? {
                object.insert(
                    "filtre".into(),
                    build_filter(terminal, document, FilterFamily::Check, 0)?,
                );
            }
        }
        _ => {}
    }
    Ok(Value::Object(object))
}

/// Prompts the user for selection.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_selection(terminal: &mut AppTerminal) -> Result<String> {
    let options = vec![
        ("tous".into(), "Tous les éléments correspondants".into()),
        ("premier".into(), "Seulement le premier".into()),
    ];
    let index =
        prompts::select(terminal, "Mode de sélection", &options, false)?.expect("selection");
    Ok(options[index].0.clone())
}

/// Prompts the user for poll data type.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_poll_data_type(terminal: &mut AppTerminal) -> Result<String> {
    let options = vec![
        ("str".into(), "Chaîne de caractères".into()),
        ("int".into(), "Entier".into()),
        ("float".into(), "Flottant".into()),
        ("bool".into(), "Booléen".into()),
    ];
    let index = prompts::select(terminal, "Type de données", &options, true)?.unwrap_or(0);
    Ok(options[index].0.clone())
}

/// Prompts the user for poll choices.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_poll_choices(
    terminal: &mut AppTerminal,
    dtype: &str,
) -> Result<Value> {
    let modes = vec![
        ("list".into(), "Liste de choix structurés".into()),
        ("formula".into(), "Formule retournant une liste".into()),
        ("none".into(), "Aucun choix prédéfini".into()),
    ];
    let mode = prompts::select(terminal, "Source des choix", &modes, false)?.expect("selection");
    if modes[mode].0 == "formula" {
        return Ok(Value::String(prompts::prompt_line(
            terminal,
            "Formule retournant les choix du sondage",
            Some("${[]}"),
        )?));
    }
    if modes[mode].0 == "none" {
        return Ok(Value::Null);
    }

    let mut choices = Vec::new();
    loop {
        let choice = prompts::prompt_line(
            terminal,
            "Choix de réponse (laisser vide pour terminer)",
            None,
        )?;
        if choice.trim().is_empty() {
            break;
        }
        let choice = ensure_type(Value::String(choice), dtype)?;
        let label = prompts::prompt_line(terminal, &format!("Libellé du choix: {choice}"), None)?;
        let mut map = Map::new();
        map.insert("valeur".into(), choice);
        map.insert("texte".into(), Value::String(label));
        choices.push(map);
    }
    Ok(Value::Array(choices.into_iter().map(Value::Object).collect()))
}

/// Prompts the user for traits value.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_traits_value(terminal: &mut AppTerminal, label: &str) -> Result<Value> {
    let values = prompts::prompt_string_list(terminal, label, &[])?;
    if values.len() == 1 {
        Ok(Value::String(values[0].clone()))
    } else {
        Ok(Value::Array(
            values.into_iter().map(Value::String).collect(),
        ))
    }
}

/// Prompts for either literal metadata or a deferred formula producing metadata.
fn prompt_object_or_formula(
    terminal: &mut AppTerminal,
    label: &str,
    current: Option<&Value>,
) -> Result<Value> {
    let options = if current.is_some_and(Value::is_string) {
        vec![
            ("formula".into(), "Formule retournant un dictionnaire".into()),
            ("object".into(), "Dictionnaire JSON".into()),
        ]
    } else {
        vec![
            ("object".into(), "Dictionnaire JSON".into()),
            ("formula".into(), "Formule retournant un dictionnaire".into()),
        ]
    };
    let mode = prompts::select(terminal, label, &options, false)?.expect("selection");
    match options[mode].0.as_str() {
        "formula" => Ok(Value::String(prompts::prompt_line(
            terminal,
            "Formule retournant un dictionnaire",
            current.and_then(Value::as_str).or(Some("${{}}")),
        )?)),
        "object" => loop {
            let fallback = json!({});
            let initial = current.filter(|value| value.is_object()).unwrap_or(&fallback);
            let value = prompts::prompt_json_value(terminal, label, Some(initial))?;
            if value.is_object() {
                break Ok(value);
            }
            prompts::pause(
                terminal,
                "Cette valeur doit être un objet JSON, par exemple {\"intensite\": 2}.",
            )?;
        },
        _ => unreachable!(),
    }
}

/// Prompts the user to choose page.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn choose_page(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    title: &str,
    allow_formula: bool,
) -> Result<Option<String>> {
    let ids = document.page_ids();
    if ids.is_empty() {
        return Ok(Some(prompts::prompt_line(
            terminal,
            "Identifiant de page",
            None,
        )?));
    }
    if allow_formula {
        match prompts::select_strings_or_formula(terminal, title, &ids, true)? {
            None => Ok(None),
            Some(prompts::IndexOrFormula::Index(index)) => Ok(Some(ids[index].clone())),
            Some(prompts::IndexOrFormula::Formula(formula)) => Ok(Some(formula)),
        }
    } else {
        Ok(prompts::select_strings(terminal, title, &ids, true)?.map(|index| ids[index].clone()))
    }
}

/// Prompts the user to choose optional page.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn choose_optional_page(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    title: &str,
    allow_formula: bool,
) -> Result<Option<String>> {
    choose_page(
        terminal,
        document,
        &format!("{title} (0 = aucune)"),
        allow_formula,
    )
}

/// Prompts the user to choose entity.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn choose_entity(
    terminal: &mut AppTerminal,
    document: &TaleDocument,
    key: &str,
    title: &str,
    allow_formula: bool,
) -> Result<Option<String>> {
    let ids = document.entity_ids(key);
    if ids.is_empty() {
        return Ok(Some(prompts::prompt_line(terminal, "Identifiant", None)?));
    }
    if allow_formula {
        match prompts::select_strings_or_formula(terminal, title, &ids, true)? {
            None => Ok(None),
            Some(prompts::IndexOrFormula::Index(index)) => Ok(Some(ids[index].clone())),
            Some(prompts::IndexOrFormula::Formula(formula)) => Ok(Some(formula)),
        }
    } else {
        Ok(prompts::select_strings(terminal, title, &ids, true)?.map(|index| ids[index].clone()))
    }
}

/// Prompts the user to choose attribute.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn choose_attribute(terminal: &mut AppTerminal, document: &TaleDocument) -> Result<Option<String>> {
    let attributes = document.attribute_names();
    Ok(
        prompts::select_strings(terminal, "Attribut", &attributes, true)?
            .map(|index| attributes[index].clone()),
    )
}

/// Returns the mutable root object of the tale document.
fn root_object_mut(document: &mut TaleDocument) -> &mut Map<String, Value> {
    if !document.root.is_object() {
        document.root = json!({});
    }
    document.root.as_object_mut().expect("root object")
}

/// Returns the mutable JSON object for the requested page.
fn page_object_mut<'a>(document: &'a mut TaleDocument, id: &str) -> &'a mut Map<String, Value> {
    let page = document.page_mut(id).expect("selected page exists");
    ensure_object(page)
}

/// Ensures a JSON value is an object and returns its mutable map.
fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("object")
}

/// Extracts a list of strings from an optional JSON array.
fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        cell_forge_label, connection_forge_label, effect_forge_label, ensure_type,
        image_definition_forge_label, merge_legacy_dynamic_connections,
        normalize_deferred_container,
    };
    use serde_json::json;

    // Verifies that the Forge preserves literal list and dictionary tag values.
    #[test]
    fn structured_tag_values_keep_their_json_types() {
        assert_eq!(ensure_type(json!([1, 2]), "list").unwrap(), json!([1, 2]));
        assert_eq!(
            ensure_type(json!({"cle": 3}), "dict").unwrap(),
            json!({"cle": 3})
        );
    }

    // Verifies that collection-producing formulas remain deferred strings in the Forge.
    #[test]
    fn structured_tag_formulas_remain_deferred() {
        let list_formula = json!("${[i + 1 for i in nums]}");
        let dict_formula = json!("${{key: val + 1 for key, val in nums}}");
        assert_eq!(
            ensure_type(list_formula.clone(), "list").unwrap(),
            list_formula
        );
        assert_eq!(
            ensure_type(dict_formula.clone(), "dict").unwrap(),
            dict_formula
        );
    }

    /// Verifies that opening the connection manager never discards compact legal forms.
    #[test]
    fn connection_manager_normalizes_without_losing_formula_or_object_entries() {
        let mut formula = json!("${recit.sorties()}");
        normalize_deferred_container(&mut formula);
        assert_eq!(formula, json!(["${recit.sorties()}"]));

        let mut object = json!({"texte": "Continuer", "destination": "fin"});
        normalize_deferred_container(&mut object);
        assert_eq!(
            object,
            json!([{"texte": "Continuer", "destination": "fin"}])
        );
        assert!(connection_forge_label(&formula[0]).starts_with('ƒ'));
        assert!(cell_forge_label(&formula[0], 0).starts_with('ƒ'));
        assert!(effect_forge_label(&formula[0]).starts_with('ƒ'));
    }

    /// Verifies the lossless Forge migration from the legacy generator field.
    #[test]
    fn legacy_dynamic_connections_merge_into_unified_connections() {
        let mut connections = json!([{"texte": "Rester", "destination": "ici"}]);
        let legacy = json!([
            {"id": "portes", "libelle": "Portes", "formule": "${recit.portes()}"},
            "${recit.retour()}"
        ]);
        merge_legacy_dynamic_connections(&mut connections, &legacy);
        assert_eq!(
            connections,
            json!([
                {"texte": "Rester", "destination": "ici"},
                "${recit.portes()}",
                "${recit.retour()}"
            ])
        );
    }

    /// Verifies that Forge summaries never copy a potentially huge base64 payload.
    #[test]
    fn embedded_image_summary_reports_size_without_payload() {
        let definition = json!({
            "type": "base64",
            "format": "webp",
            "donnees": "abcdefghijklmnopqrstuvwxyz"
        });
        let label = image_definition_forge_label("portrait", Some(&definition));
        assert_eq!(label, "portrait · base64 webp · 26 caractères");
        assert!(!label.contains("abcdefghijklmnopqrstuvwxyz"));
    }
}
