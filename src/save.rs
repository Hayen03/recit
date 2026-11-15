//! Save-file serialization and restoration of runtime and rendered page state.
//!
//! Saves include the character, pending redirect, page counter, resolved title, and
//! semantic [`Message`] values. Loading rebinds saved item/state IDs to definitions from
//! the currently loaded tale and does not replay page effects.
use crate::character::{Character, SavedCharacter};
use crate::engine::{Message, PageProgress};
use formulas::{Env, EvalValue};
use crate::model::Tale;
use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::Arc;

/// Serializable game state containing the character and page transition state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedState {
    #[serde(rename = "personnage")]
    pub character: SavedCharacter,
    pub page: SavedPageProgress,
    #[serde(rename = "prochaine_page", default)]
    pub pending_page: Option<String>,
    pub title: String,
    pub ancien_env: IndexMap<String, EvalValue>,
}
/// Writes the complete current save state as pretty JSON.
///
/// `progress` contains already rendered semantic messages, preserving `$[…]` highlighting without storing terminal escape codes. The function does not replay or reevaluate page content.
///
/// # Errors
///
/// Returns an error when character values cannot be serialized or the save file cannot be created/written.
pub fn save_state(
    character: &Character,
    progress: &PageProgress,
    title: String,
    pending_page: Option<&str>,
    filename: &Path,
    previous_env: &Env,
) -> Result<()> {
    let ancien_env = previous_env.solidified();
    let ancien_env = ancien_env.vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect::<IndexMap<String, EvalValue>>();
    let state = SavedState {
        character: character.save()?,
        page: progress.to_save_data(),
        pending_page: pending_page.map(str::to_owned),
        title,
        ancien_env,
    };
    std::fs::create_dir_all(filename.parent().unwrap_or(Path::new("")))
        .with_context(|| format!("Impossible de créer le répertoire pour la sauvegarde '{}'", filename.display()))?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(filename)
        .with_context(|| format!("Impossible d'ouvrir la sauvegarde '{}'", filename.display()))?;
    serde_json::to_writer_pretty(BufWriter::new(file), &state)
        .context("Impossible d'écrire la sauvegarde")?;
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
/// Serializable snapshot of the current page visit.
///
/// Messages retain semantic `$[…]` highlighting and message categories rather than terminal escape codes.
pub struct SavedPageProgress {
    pub page_id: String,
    pub page_number: usize,
    pub content: Vec<Message>,
}
/// Loads a save, rebinds its character to `tale`, and returns the rendered page snapshot and transition state.
///
/// The tuple contains character, page ID, resolved title, page number, rendered messages, and pending redirect. No page effects or formulas are replayed.
///
/// # Errors
///
/// Returns an error for file/JSON failures, missing tale definitions, or invalid saved values.
pub fn load_state(tale: &Tale, filename: &Path) -> Result<(Character, String, String, usize, Vec<Message>, Option<String>, Env<'static>)> {
    let file = File::open(filename)
        .with_context(|| format!("Impossible d'ouvrir la sauvegarde '{}'", filename.display()))?;
    let state: SavedState = serde_json::from_reader(BufReader::new(file))
        .context("Le fichier de sauvegarde est invalide")?;
    let character = Character::from_saved(state.character, tale)?;
    Ok((
        character,
        state.page.page_id,
        state.title,
        state.page.page_number,
        state.page.content,
        state.pending_page,
        Env::from_index_map(&state.ancien_env
            .into_iter()
            .map(|(k, v)| (Arc::from(k), v))
            .collect::<IndexMap<Arc<str>, EvalValue>>()
        )
    ))
}
