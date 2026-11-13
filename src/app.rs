//! Coordinates Forge state, view navigation, keyboard dispatch, and validation refreshes.
//!
//! `App` owns the editable [`TaleDocument`] and every UI selection index. Structural
//! edits must be followed by `after_edit`, which marks the document dirty, recomputes
//! validation, and clamps indices before the next render.
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::{
    builder,
    document::TaleDocument,
    graph, prompts, schema,
    terminal::AppTerminal,
    ui,
    validation::{self, Problem},
};

/// Identifies the active top-level view in the Forge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Tale,
    Pages,
    Objects,
    States,
    Graph,
    Problems,
}

impl View {
    /// Top-level views in keyboard navigation order.
    pub const ALL: [View; 6] = [
        View::Tale,
        View::Pages,
        View::Objects,
        View::States,
        View::Graph,
        View::Problems,
    ];
    /// Returns the label displayed for this Forge top-level view.
    pub fn title(self) -> &'static str {
        match self {
            Self::Tale => "Récit",
            Self::Pages => "Pages",
            Self::Objects => "Objets",
            Self::States => "États",
            Self::Graph => "Graphe",
            Self::Problems => "Problèmes",
        }
    }

    /// Returns the next value in the configured navigation order.
    fn next(self) -> Self {
        let index = Self::ALL.iter().position(|view| *view == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Returns the previous value in the configured navigation order.
    fn previous(self) -> Self {
        let index = Self::ALL.iter().position(|view| *view == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Owns the Forge document, selection state, validation results, and event loop state.
pub struct App {
    pub document: TaleDocument,
    pub view: View,
    pub page_index: usize,
    pub object_index: usize,
    pub state_index: usize,
    pub graph_index: usize,
    pub problem_index: usize,
    pub problems: Vec<Problem>,
    pub status: String,
    should_quit: bool,
}

impl App {
    /// Creates Forge application state for `document` with the tale view selected and validation scheduled.
    pub fn new(document: TaleDocument) -> Self {
        let problems = validation::validate_tale(&document.root);
        Self {
            document,
            view: View::Pages,
            page_index: 0,
            object_index: 0,
            state_index: 0,
            graph_index: 0,
            problem_index: 0,
            problems,
            status: "F1: aide · Ctrl+S: enregistrer · Tab: changer de vue".into(),
            should_quit: false,
        }
    }

    /// Runs the interactive loop until the user exits or the current mode completes.
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
    /// invalid, or a delegated parser/evaluator fails.
    pub fn run(&mut self, terminal: &mut AppTerminal) -> Result<()> {
        while !self.should_quit {
            self.clamp_selections();
            let backdrop = {
                let completed = terminal.draw(|frame| ui::draw(frame, self))?;
                (*completed.buffer).clone()
            };
            prompts::set_backdrop(backdrop);
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
            if let Event::Key(key) = event::read()? {
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    continue;
                }
                if let Err(error) = self.handle_key(terminal, key) {
                    if prompts::is_cancelled(&error) {
                        self.status = "Action annulée".into();
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }

    /// Handles one keyboard event for the current application state.
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    fn handle_key(&mut self, terminal: &mut AppTerminal, key: KeyEvent) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            self.save()?;
            return Ok(());
        }
        match key.code {
            KeyCode::Tab => self.view = self.view.next(),
            KeyCode::BackTab => self.view = self.view.previous(),
            KeyCode::F(1) | KeyCode::Char('?') => self.help(terminal)?,
            KeyCode::Esc | KeyCode::Char('q') => self.request_quit(terminal)?,
            KeyCode::Char('v') => {
                self.refresh_problems();
                self.view = View::Problems;
            }
            KeyCode::Char('j') => self.edit_raw_document(terminal)?,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            _ => self.handle_view_key(terminal, key)?,
        }
        Ok(())
    }

    /// Handles keyboard navigation shared by all Forge views.
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    fn handle_view_key(&mut self, terminal: &mut AppTerminal, key: KeyEvent) -> Result<()> {
        match self.view {
            View::Tale => match key.code {
                KeyCode::Enter | KeyCode::Char('e') => {
                    builder::edit_tale_metadata(terminal, &mut self.document)?;
                    self.after_edit();
                }
                KeyCode::Char('f') => {
                    builder::edit_tale_metadata(terminal, &mut self.document)?;
                    self.after_edit();
                }
                _ => {}
            },
            View::Pages => self.handle_pages_key(terminal, key)?,
            View::Objects => self.handle_entities_key(terminal, key, "objets")?,
            View::States => self.handle_entities_key(terminal, key, "etats")?,
            View::Graph => {
                if key.code == KeyCode::Enter {
                    let graph = graph::analyze(&self.document.root);
                    if let Some(node) = graph.nodes.get(self.graph_index) {
                        if let Some(index) = self
                            .document
                            .page_ids()
                            .iter()
                            .position(|id| id == &node.id)
                        {
                            self.page_index = index;
                            self.view = View::Pages;
                        }
                    }
                }
            }
            View::Problems => {}
        }
        Ok(())
    }

    /// Handles keyboard commands specific to the page editor view.
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    fn handle_pages_key(&mut self, terminal: &mut AppTerminal, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(id) = self.selected_page_id() {
                    builder::edit_page(terminal, &mut self.document, &id)?;
                    self.after_edit();
                }
            }
            KeyCode::Char('a') | KeyCode::Char('n') => {
                let id = prompts::prompt_line(terminal, "Identifiant de la nouvelle page", None)?;
                if id.trim().is_empty() {
                    return Ok(());
                }
                if self.document.add_page(id.clone()) {
                    self.page_index = self
                        .document
                        .page_ids()
                        .iter()
                        .position(|value| value == &id)
                        .unwrap_or(0);
                    self.after_edit();
                    self.status = format!("Page `{id}` créée");
                } else {
                    self.status = format!("La page `{id}` existe déjà");
                }
            }
            KeyCode::Char('r') => {
                if let Some(old) = self.selected_page_id() {
                    let new = prompts::prompt_line(terminal, "Nouvel identifiant", Some(&old))?;
                    match self.document.rename_page(&old, &new) {
                        Ok(()) => {
                            self.page_index = self
                                .document
                                .page_ids()
                                .iter()
                                .position(|value| value == &new)
                                .unwrap_or(0);
                            self.after_edit();
                            self.status = format!("Page `{old}` renommée en `{new}`");
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            KeyCode::Char('c') => {
                if let Some(source) = self.selected_page_id() {
                    let suggested = format!("{source}_copie");
                    let new = prompts::prompt_line(
                        terminal,
                        "Identifiant de la copie",
                        Some(&suggested),
                    )?;
                    match self.document.duplicate_page(&source, new.clone()) {
                        Ok(()) => {
                            self.page_index = self
                                .document
                                .page_ids()
                                .iter()
                                .position(|value| value == &new)
                                .unwrap_or(0);
                            self.after_edit();
                            self.status = format!("Page `{source}` copiée vers `{new}`");
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(id) = self.selected_page_id() {
                    let refs = self.document.incoming_references(&id);
                    let question = if refs.is_empty() {
                        format!("Supprimer la page `{id}`?")
                    } else {
                        format!(
                            "La page `{id}` possède {} référence(s). La supprimer quand même?",
                            refs.len()
                        )
                    };
                    if prompts::confirm(terminal, &question, false)? {
                        self.document.delete_page(&id);
                        self.after_edit();
                        self.status = format!("Page `{id}` supprimée");
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles keyboard commands for the selected object or state collection.
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    fn handle_entities_key(
        &mut self,
        terminal: &mut AppTerminal,
        key: KeyEvent,
        entity_key: &str,
    ) -> Result<()> {
        let selected = self.selected_entity_id(entity_key);
        match key.code {
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(id) = selected {
                    if entity_key == "objets" {
                        builder::edit_item(terminal, &mut self.document, &id)?;
                    } else {
                        builder::edit_condition_definition(terminal, &mut self.document, &id)?;
                    }
                    self.after_edit();
                }
            }
            KeyCode::Char('a') | KeyCode::Char('n') => {
                let label = if entity_key == "objets" {
                    "objet"
                } else {
                    "état"
                };
                let id = prompts::prompt_line(
                    terminal,
                    &format!("Identifiant du nouvel {label}"),
                    None,
                )?;
                if id.trim().is_empty() {
                    return Ok(());
                }
                let value = if entity_key == "objets" {
                    schema::new_item_definition()
                } else {
                    schema::new_condition_definition()
                };
                if self.document.add_entity(entity_key, id.clone(), value) {
                    let index = self
                        .document
                        .entity_ids(entity_key)
                        .iter()
                        .position(|value| value == &id)
                        .unwrap_or(0);
                    if entity_key == "objets" {
                        self.object_index = index;
                    } else {
                        self.state_index = index;
                    }
                    self.after_edit();
                } else {
                    self.status = format!("L'identifiant `{id}` existe déjà");
                }
            }
            KeyCode::Char('r') => {
                if let Some(old) = selected {
                    let new = prompts::prompt_line(terminal, "Nouvel identifiant", Some(&old))?;
                    match self.document.rename_entity(entity_key, &old, &new) {
                        Ok(()) => {
                            let index = self
                                .document
                                .entity_ids(entity_key)
                                .iter()
                                .position(|value| value == &new)
                                .unwrap_or(0);
                            if entity_key == "objets" {
                                self.object_index = index;
                            } else {
                                self.state_index = index;
                            }
                            self.after_edit();
                            self.status = format!("`{old}` renommé en `{new}`");
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            KeyCode::Char('c') => {
                if let Some(source) = selected {
                    let suggested = format!("{source}_copie");
                    let new = prompts::prompt_line(
                        terminal,
                        "Identifiant de la copie",
                        Some(&suggested),
                    )?;
                    match self
                        .document
                        .duplicate_entity(entity_key, &source, new.clone())
                    {
                        Ok(()) => {
                            let index = self
                                .document
                                .entity_ids(entity_key)
                                .iter()
                                .position(|value| value == &new)
                                .unwrap_or(0);
                            if entity_key == "objets" {
                                self.object_index = index;
                            } else {
                                self.state_index = index;
                            }
                            self.after_edit();
                            self.status = format!("`{source}` copié vers `{new}`");
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(id) = selected {
                    if prompts::confirm(terminal, &format!("Supprimer `{id}`?"), false)? {
                        self.document.delete_entity(entity_key, &id);
                        self.after_edit();
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Moves the current selection by a signed offset and clamps it to valid bounds.
    ///
    /// Handles empty collections and out-of-range indices without panicking.
    fn move_selection(&mut self, delta: isize) {
        match self.view {
            View::Pages => {
                self.page_index = shifted(self.page_index, delta, self.document.page_ids().len());
            }
            View::Objects => {
                self.object_index = shifted(
                    self.object_index,
                    delta,
                    self.document.entity_ids("objets").len(),
                );
            }
            View::States => {
                self.state_index = shifted(
                    self.state_index,
                    delta,
                    self.document.entity_ids("etats").len(),
                );
            }
            View::Graph => {
                self.graph_index = shifted(
                    self.graph_index,
                    delta,
                    graph::analyze(&self.document.root).nodes.len(),
                );
            }
            View::Problems => {
                self.problem_index = shifted(self.problem_index, delta, self.problems.len());
            }
            View::Tale => {}
        }
    }

    /// Returns the ID of the currently selected page, when one exists.
    fn selected_page_id(&self) -> Option<String> {
        self.document.page_ids().get(self.page_index).cloned()
    }

    /// Returns the ID of the currently selected object or state.
    fn selected_entity_id(&self, key: &str) -> Option<String> {
        let index = if key == "objets" {
            self.object_index
        } else {
            self.state_index
        };
        self.document.entity_ids(key).get(index).cloned()
    }

    /// Creates the serializable representation of the current runtime value.
    ///
    /// Captures the complete current state needed by the loader and does not mutate the
    /// runtime value being serialized.
    fn save(&mut self) -> Result<()> {
        self.refresh_problems();
        self.document.save()?;
        let (errors, warnings, _) = validation::problem_counts(&self.problems);
        self.status = format!(
            "Enregistré: {} · {errors} erreur(s), {warnings} avertissement(s)",
            self.document.path.display()
        );
        Ok(())
    }
    /// Opens the entire raw tale in the JSON5 editor and replaces the document only after successful parsing.
    ///
    /// # Errors
    ///
    /// Propagates editor, terminal, or JSON5 parsing errors.
    fn edit_raw_document(&mut self, terminal: &mut AppTerminal) -> Result<()> {
        if let Some(value) = prompts::edit_json(terminal, "Récit complet", &self.document.root)? {
            self.document.root = value;
            self.document.mark_dirty();
            self.after_edit();
        }
        Ok(())
    }

    /// Prompts to save unsaved changes before leaving the Forge.
    fn request_quit(&mut self, terminal: &mut AppTerminal) -> Result<()> {
        if !self.document.dirty {
            self.should_quit = true;
            return Ok(());
        }
        let options = vec![
            ("save".into(), "Enregistrer et quitter".into()),
            ("discard".into(), "Quitter sans enregistrer".into()),
            ("cancel".into(), "Annuler".into()),
        ];
        let Some(choice) =
            prompts::select(terminal, "Modifications non enregistrées", &options, false)?
        else {
            return Ok(());
        };
        match options[choice].0.as_str() {
            "save" => {
                self.save()?;
                self.should_quit = true;
            }
            "discard" => self.should_quit = true,
            _ => {}
        }
        Ok(())
    }

    /// Displays the contextual Forge keyboard shortcuts for the active view.
    fn help(&mut self, terminal: &mut AppTerminal) -> Result<()> {
        prompts::pause(
            terminal,
            "Forge du récit\n\n\
             Tab / Maj+Tab   Changer de vue\n\
             ↑ ↓ ou k       Déplacer la sélection\n\
             Entrée / e     Modifier l'élément\n\
             a ou n         Ajouter\n\
             c              Copier l’élément sélectionné\n\
             r              Renommer\n\
             d / Suppr      Supprimer\n\
             v              Valider et ouvrir Problèmes\n\
             j              Éditer le JSON complet\n\
             Ctrl+S         Enregistrer\n\
             Échap / q      Quitter hors fenêtre modale\n\
             Fenêtres modales:\n\
             ↑ ↓            Choisir une option\n\
             Entrée         Valider un menu ou un champ simple\n\
             Ctrl+Entrée, F2 ou Ctrl+S  Valider un texte multiligne\n\
             Échap          Annuler la fenêtre active\n\n\
             Ordre v2: cellules de connexion → avant → cellules de page → après.\n\
             Les tags de page sont immuables pendant la partie.\n\
             `definir_tag` et `incrementer_tag` utilisent tous deux `filtre`.\n\
             Un filtre réduit à un seul id crée le tag s'il est absent.",
        )
    }

    /// Marks the document as changed and refreshes derived Forge state.
    fn after_edit(&mut self) {
        self.refresh_problems();
        self.clamp_selections();
        self.status = "Modifications non enregistrées".into();
    }

    /// Recomputes the validation problems for the current document.
    fn refresh_problems(&mut self) {
        self.problems = validation::validate_tale(&self.document.root);
        self.problem_index = self
            .problem_index
            .min(self.problems.len().saturating_sub(1));
    }

    /// Clamps every list selection after the document structure changes.
    ///
    /// Handles empty collections and out-of-range indices without panicking.
    fn clamp_selections(&mut self) {
        self.page_index = clamp(self.page_index, self.document.page_ids().len());
        self.object_index = clamp(self.object_index, self.document.entity_ids("objets").len());
        self.state_index = clamp(self.state_index, self.document.entity_ids("etats").len());
        self.graph_index = clamp(
            self.graph_index,
            graph::analyze(&self.document.root).nodes.len(),
        );
        self.problem_index = clamp(self.problem_index, self.problems.len());
    }
}

/// Clamps an index to the valid range of a collection.
///
/// Handles empty collections and out-of-range indices without panicking.
fn clamp(index: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        index.min(len - 1)
    }
}

/// Returns an index shifted by a signed amount within collection bounds.
///
/// Handles empty collections and out-of-range indices without panicking.
fn shifted(index: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + delta).clamp(0, len as isize - 1) as usize
}
