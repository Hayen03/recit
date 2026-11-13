//! Shared library for the Récit player and Forge editor.
//!
//! Forge retains raw JSON/JSON5 for editing, while the player executes [`model::Tale`].
//! JSON5 comments and original formatting are not preserved when Forge serializes the
//! document again.
pub mod app;
pub mod builder;
pub mod character;
pub mod document;
pub mod effects;
pub mod engine;
pub mod error_report;
pub mod filters;
pub mod forge_ops;
pub mod formula;
pub mod graph;
pub mod json_util;
pub mod migration;
pub mod model;
pub mod polls;
pub mod prompts;
pub mod save;
pub mod schema;
pub mod terminal;
pub mod ui;
pub mod validation;
pub mod images;

use anyhow::{Context, Result};
use model::Tale;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::Path;
/// Parses JSON or JSON5 source into a `serde_json::Value` while preserving object insertion order.
///
/// # Errors
///
/// Returns the JSON5 parser error when the source is invalid.
pub fn parse_tale_text(text: &str) -> Result<JsonValue> {
    serde_json5::from_str(text).map_err(|error| anyhow::anyhow!("{error}"))
}
/// Reads and parses a UTF-8 JSON/JSON5 tale file into its raw value tree.
///
/// # Errors
///
/// Adds the file path to read and parse errors.
pub fn load_tale_json(path: &Path) -> Result<JsonValue> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("Impossible d'ouvrir le récit '{}'", path.display()))?;
    parse_tale_text(&text).with_context(|| {
        format!(
            "Le fichier '{}' n'est pas un document JSON ou JSON5 valide",
            path.display()
        )
    })
}
/// Loads a JSON/JSON5 file and converts it into the executable version-2 tale model.
///
/// # Errors
///
/// Propagates file, parse, and typed-model validation errors.
pub fn load_tale(path: &Path) -> Result<Tale> {
    let value = load_tale_json(path)?;
    Tale::from_json(&value)
        .with_context(|| format!("Impossible de construire le récit '{}'", path.display()))
}
