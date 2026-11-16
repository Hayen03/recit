//! Interactive tale runner, page lifecycle, rendering, and in-process save/load control.
//!
//! A fresh visit executes global-before, state-before, connection, page, state-after, and
//! global-after cells in that order. Loading restores the saved rendered presentation
//! instead of replaying effects; the connection menu is intentionally regenerated.
use crate::character::Character;
use crate::effects::{Cell, Deferred, PageEffect};
use crate::error_report::format_error;
use crate::formula::{mk_character_base_env, mk_character_env, mk_condition_env, mk_new_condition_env, mk_page_env};
use formulas::builtins::GLOBAL_ENV;
use formulas::{Env, EvalKey, EvalValue, FormulaClosure, FormulaEngine, Function};
use crate::images::{ImageRegistry, ImageResult, fit_in_terminal};
use crate::model::{CheckResult, Page, PageResult, Tale, eval_cells};
use crate::polls::{PollRequest, PollRequestData};
use crate::save::{SavedPageProgress, load_state, save_state};
use anyhow::{anyhow, Context, Result};
use crossterm::cursor::MoveToNextLine;
use crossterm::execute;
use indexmap::{IndexMap};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::io::{self, Write, stdout};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

#[allow(dead_code)]
const ENTER_ALTERNATE_SCREEN: &str = "\x1b[?1049h";
#[allow(dead_code)]
const LEAVE_ALTERNATE_SCREEN: &str = "\x1b[?1049l";
#[allow(dead_code)]
const CLEAR_SEQUENCE: &str = "\x1b[2J"; // \x1b[3J\x1b[H

const COMMANDS: &[(&str, &str)] = &[
    ("S", "[S]tatut"),
    ("I", "[I]nventaire"),
    ("E", "[E]tats"),
    ("A", "S[A]uvegarder"),
    ("C", "[C]harger"),
    ("Q", "[Q]uitter"),
];
const DEBUG_COMMAND: (&str, &str) = ("T", "[T]ags");
pub const CAPTURE_IGNORE: &[&str] = &[
    "personnage",
    "recit",
    "page",
    "test",
    "etat",
    "objet",
    "connexion",
];

/// Coordinates tale execution, character state, page transitions, and user interaction.
pub struct Game {
    tale: Tale,
    formula: FormulaEngine,
    terminal: Terminal,
    interaction: Interaction,
    page_count: usize,
    debug: bool,
    client: Client,
}

/// Associates rendered text with its presentation category.
#[allow(dead_code)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Message {
    Notification(String),
    Text(String),
    Poll(String),
    Alert(String),
    News(String),
    Image(String),
}

/// Accumulates formatted terminal output before it is displayed.
struct StringBuilder {
    buffer: Vec<String>,
}
impl StringBuilder {
    /// Creates an empty rendered-output accumulator.
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Appends rendered text to the output accumulator.
    fn push(&mut self, message: String) {
        self.buffer.push(message);
    }

    /// Builds the rendered output represented by the current accumulator or page progress.
    fn build(self) -> String {
        self.buffer.join("")
    }
}

/// Abstracts the input and output operations required while executing effects.
pub struct Interaction {
    reader: fn(&str) -> Result<Arc<str>>,
}
impl Interaction {
    /// Creates an interaction adapter using `reader` for every poll response.
    pub fn new(reader: fn(&str) -> Result<Arc<str>>) -> Self {
        Self { reader }
    }
    /// Presents a poll request and returns the validated answer.
    ///
    ///
    /// # Arguments
    ///
    /// - `term` — Active player terminal used for prompts or rendered output.
    /// - `request` — Resolved poll request presented to the player.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
    /// invalid, or a delegated parser/evaluator fails.
    pub fn poll(&mut self, term: &Terminal, request: PollRequest) -> Result<(EvalValue, Arc<str>)> {
        let prompt = term.styled_text(&request.prompt, term.poll());
        print!("{}{}{}", term.poll(), prompt, term.reset());
        match request.data {
            PollRequestData::Bool { default } => {
                let labels = ["Oui".into(), "Non".into()];
                let options = vec![true, false];
                let (choice, text) = self.read_choice(term, options, &labels, default)?;
                Ok((EvalValue::Bool(choice), text))
            }
            PollRequestData::Str { choices, default } => {
                let options_label = choices
                    .as_ref()
                    .map(|c| c.iter().map(|o| o.label.clone()).collect::<Vec<_>>());
                let options_value = choices
                    .as_ref()
                    .map(|c| c.iter().map(|o| o.value.clone()).collect::<Vec<_>>());
                let (choice, text) =
                    if let (Some(labels), Some(values)) = (options_label, options_value) {
                        self.read_choice(term, values, &labels, default)?
                    } else {
                        self.read_single(term, default.map(|s| String::from(s.as_ref())), None)
                            .map(|(s, text)| (Arc::from(s), text))?
                    };
                Ok((EvalValue::String(choice), text))
            }
            PollRequestData::Int { choices, default } => {
                let options_label = choices
                    .as_ref()
                    .map(|c| c.iter().map(|o| o.label.clone()).collect::<Vec<_>>());
                let options_value = choices
                    .as_ref()
                    .map(|c| c.iter().map(|o| o.value).collect::<Vec<_>>());
                let (choice, text) =
                    if let (Some(labels), Some(values)) = (options_label, options_value) {
                        self.read_choice(term, values, &labels, default)?
                    } else {
                        self.read_single(term, default, None)?
                    };
                Ok((EvalValue::Int(choice), text))
            }
            PollRequestData::Float { choices, default } => {
                let options_label = choices
                    .as_ref()
                    .map(|c| c.iter().map(|o| o.label.clone()).collect::<Vec<_>>());
                let options_value = choices
                    .as_ref()
                    .map(|c| c.iter().map(|o| o.value).collect::<Vec<_>>());
                let (choice, text) =
                    if let (Some(labels), Some(values)) = (options_label, options_value) {
                        self.read_choice(term, values, &labels, default)?
                    } else {
                        self.read_single(term, default, Some(|f| f.is_finite()))?
                    };
                Ok((EvalValue::Float(choice), text))
            }
        }
    }
    /// Repeatedly prompts for a numbered choice and returns its value plus displayed label.
    ///
    /// An empty input selects `default` when present. `options_value` and `options_label` are assumed to have matching lengths.
    ///
    /// # Errors
    ///
    /// Propagates input-reader errors.
    fn read_choice<T: PartialEq>(
        &mut self,
        term: &Terminal,
        mut options_value: Vec<T>,
        options_label: &[Arc<str>],
        mut default: Option<T>,
    ) -> Result<(T, Arc<str>)> {
        let default_index = default
            .as_ref()
            .map(|b| options_value.iter().position(|v| v == b))
            .flatten();
        print!("\n{}", term.poll());
        for (index, option) in options_label.iter().enumerate() {
            let option = term.styled_text(option, term.poll());
            if Some(index) == default_index {
                print!("({}) [{option}]\n", index + 1);
            } else {
                print!("({}) {option}\n", index + 1);
            }
        }
        print!("{}", term.reset());
        loop {
            std::io::stdout().flush().unwrap();
            let input_string = (self.reader)("> ")?;
            let input = input_string.trim();
            if input.is_empty() && default.is_some() {
                let text = default_index
                    .map(|i| options_label[i].clone())
                    .unwrap_or_default();
                return Ok((default.take().unwrap(), text));
            }
            if let Ok(choice) = input.parse::<usize>() {
                if choice >= 1 && choice <= options_value.len() {
                    return Ok((
                        options_value.remove(choice - 1),
                        options_label[choice - 1].clone(),
                    ));
                }
            }
            println!(
                "{}Choix invalide. Entre un nombre entre 1 et {}, ou appuie sur Entrée pour la valeur par défaut.{}",
                term.error(),
                options_value.len(),
                term.reset()
            );
        }
    }
    /// Repeatedly parses one free-form value, optionally applying `validation`.
    ///
    /// An empty input selects `default` when present. The returned string is the accepted textual answer.
    ///
    /// # Errors
    ///
    /// Propagates input-reader errors.
    fn read_single<T: FromStr + ToString>(
        &mut self,
        term: &Terminal,
        mut default: Option<T>,
        validation: Option<fn(&T) -> bool>,
    ) -> Result<(T, Arc<str>)> {
        loop {
            let input_string = (self.reader)("> ")?;
            let input = input_string.trim();
            if input.is_empty() && default.is_some() {
                let value = default
                    .take()
                    .expect("la valeur par défaut vient d'être vérifiée");
                let text = value.to_string();
                return Ok((value, text.into()));
            }
            if let Ok(value) = input.parse::<T>() {
                if let Some(validate) = validation {
                    if !validate(&value) {
                        println!(
                            "{}Entrée invalide. Veuillez réessayer.{}",
                            term.error(),
                            term.reset()
                        );
                        continue;
                    }
                }
                return Ok((value, Arc::from(input.to_string())));
            }
            println!(
                "{}Entrée invalide. Veuillez réessayer.{}",
                term.error(),
                term.reset()
            );
        }
    }
}

/// Tracks the partially rendered state of a page while effects and polls execute.
pub struct PageProgress<'a> {
    pub page_number: usize,
    pub page: &'a Page,
    pub content: Vec<Message>,
}
impl<'a> PageProgress<'a> {
    /// Starts an empty rendered presentation for visit number `num` of `page`.
    fn new(page: &'a Page, num: usize) -> Self {
        Self {
            page_number: num,
            page,
            content: Vec::new(),
        }
    }
    /// Builds the rendered output represented by the current accumulator or page progress.
    pub fn build(
        &self,
        term: &Terminal,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<(String, String)> {
        let mut builder = StringBuilder::new();

        let actual_title = self
            .page
            .actual_title(formula, env)?
            .to_uppercase();
        //let actual_title = term.styled_text(&actual_title, term.title());
        /*
        builder.push(format!(
            "{}=== {} ==={}\n",
            term.title(),
            actual_title,
            term.reset()
        ));
        builder.push("\n".into());
        */
        // only push text
        push_messages(term, &self.content, &mut builder);

        Ok((actual_title, builder.build()))
    }
    /// Appends an effect notification to the partially rendered page.
    fn push_content(&mut self, message: Message) {
        self.content.push(message);
    }
    /// Clears the terminal and displays the currently available page content.
    fn show(
        &self,
        title: Option<Arc<str>>,
        term: &mut Terminal,
        images: &mut ImageRegistry,
        client: &Client,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Arc<str>> {
        let actual_title = if let Some(title) = title { title } else { self.page.actual_title(formula, env)? };
        term.clear()?;
        print!("{}=== {} ==={}\n", term.title(), term.styled_text(&actual_title.to_uppercase(), term.title()), term.reset());

        let mut notification: Option<MessageType> = None;
        for message in &self.content {
            match message {
                Message::Alert(msg) => {
                    if notification != Some(MessageType::Alert) {
                        term.print(term.error().into());
                        notification = Some(MessageType::Alert);
                    }
                    term.print(&term.styled_text(msg, term.error()));
                },
                Message::News(msg) => {
                    if notification != Some(MessageType::News) {
                        term.print(term.news().into());
                        notification = Some(MessageType::News);
                    }
                    term.print(&term.styled_text(msg, term.news()));
                },
                Message::Notification(msg) => {
                    if notification != Some(MessageType::Notification) {
                        term.print(term.effects().into());
                        notification = Some(MessageType::Notification);
                    }
                    term.print(&term.styled_text(msg, term.effects()));
                },
                Message::Text(msg) => {
                    if notification != Some(MessageType::Text) {
                        term.print(term.reset().into());
                        notification = Some(MessageType::Text);
                    }
                    term.print("\n".into());
                    term.print(&term.styled_text(msg, term.reset()));
                },
                Message::Poll(msg) => {
                    if notification != Some(MessageType::Poll) {
                        term.print(term.poll().into());
                        notification = Some(MessageType::Poll);
                    }
                    term.print(&term.styled_text(msg, term.poll()));
                },
                Message::Image(key) => {
                    if notification != Some(MessageType::Image) {
                        notification = Some(MessageType::Image);
                    }
                    term.print("\n");
                    // fetch the image
                    let img = images.get_and_cache(&key, client);
                    let _ = term.show(img, key);
                },
            }
        }

        Ok(actual_title)
    }
    /// Creates an owned save snapshot containing the page ID, visit number, and already rendered semantic messages.
    pub fn to_save_data(&self) -> SavedPageProgress {
        SavedPageProgress {
            page_id: self.page.id.as_ref().into(),
            page_number: self.page_number,
            content: self.content.clone(),
        }
    }
    /// Restores page progress from a saved snapshot without reevaluating text or effects.
    ///
    /// # Errors
    ///
    /// Returns an error when the saved page ID is absent from `tale`.
    pub fn from_save_data(data: &SavedPageProgress, tale: &'a Tale) -> Result<Self> {
        let page = tale.pages.get(data.page_id.as_str())
            .ok_or_else(|| anyhow!("Page '{}' introuvable dans l'histoire", data.page_id))?;
        Ok(Self {
            page_number: data.page_number,
            page,
            content: data.content.clone(),
        })
    }
}

/// Returns a callback that appends text to the current page-progress buffer.
fn printer(message: &str) {
    print!("{message}");
    let _ = io::stdout().flush();
}

impl Game {
    /// Creates a game runner for `tale`.
    ///
    /// `seed` controls deterministic formula randomness; `clear` selects alternate-screen clearing, `color` controls ANSI styling, and `debug` enables the tag viewer.
    pub fn new(tale: Tale, seed: Option<u64>, clear: bool, color: bool, debug: bool) -> Self {
        let globals = {
            let mut env = Env::new();
            for (name, value) in &tale.functions {
                env.insert(name.clone(), EvalValue::Func(
                    Function::User(
                        FormulaClosure { 
                            definition: value.clone(), 
                            captures: IndexMap::new().into(),
                        }
                    ).into()
                ));
            }
            env.with_parent(&GLOBAL_ENV)
        };
        let formula = FormulaEngine::new(seed)
            .with_globals(globals)
            .with_capture_ignore(CAPTURE_IGNORE.iter().map(|s| Arc::from(*s)).collect());
        Self {
            tale,
            formula,
            terminal: Terminal {
                clear,
                color,
                printer,
            },
            interaction: Interaction::new(|msg| prompt(msg).map(|s| Arc::from(s))),
            page_count: 0,
            debug,
            client: Client::new(),
        }
    }
    /// Runs the tale synchronously until normal completion or explicit quit.
    ///
    /// Fresh page visits execute lifecycle cells; loaded pages restore their saved rendered messages and regenerate only the connection menu.
    ///
    /// # Errors
    ///
    /// Propagates terminal, input, formula, effect, save/load, or missing-page errors.
    pub fn run(&mut self) -> Result<()> {
        let _terminal_session = self.terminal.session()?;
        let (mut character, mut current_page, mut recovery) =
            self.show_title()?;
        let mut added_cells: Arc<[Deferred<Cell>]> = Vec::new().into();
        let mut connection_env = Env::new();
        let mut check_result: Option<CheckResult> = None;
        let mut previous_env = Game::initial_env(&mut self.formula, &character, &self.tale.env())
            .with_context(|| "Impossible de construire l'environnement initial du personnage")?;

        while let Some(page_id) = current_page.clone() {
            let page = self
                .tale
                .pages
                .get(AsRef::<str>::as_ref(&page_id))
                .cloned()
                .ok_or_else(|| anyhow!("Page '{page_id}' introuvable dans l'histoire"))?;
            if recovery.is_none() {
                self.page_count += 1;
            }

            let result = self
                .run_page(
                    &page,
                    &mut character,
                    (added_cells.clone(), &connection_env),
                    recovery.take(),
                    check_result.as_ref(),
                    &previous_env,
                )
                .with_context(|| {
                    format!(
                        "Exécution de la page `{}` (visite #{})",
                        page.id, self.page_count
                    )
                })?;
            recovery = None;

            match result {
                LoopResult::Next{page_result, previous_base_env} => {
                    current_page = page_result.next_page;
                    added_cells = page_result.added_cells.as_ref().into();
                    check_result = page_result.check_result;
                    connection_env = page_result.connection_env;
                    previous_env = previous_base_env;
                }
                LoopResult::Load {
                    character: loaded_character,
                    pageid,
                    pagetitle,
                    pagenumber,
                    pagetext,
                    pending,
                    previous_base_env,
                } => {
                    current_page = Some(pageid.into());
                    self.page_count = pagenumber;
                    recovery = Some((pagetext, pending.map(Arc::from), pagetitle.into(), previous_base_env));
                    character = loaded_character;
                    previous_env = Game::initial_env(&mut self.formula, &character, &self.tale.env())
                        .with_context(|| "Erreur en chargeant une sauvegarde")?;
                }
                LoopResult::End => break,
                LoopResult::Quit => return Ok(()),
            }
        }

        self.terminal.clear()?;
        println!(
            "{}=== FIN ==={}",
            self.terminal.title(),
            self.terminal.reset()
        );
        println!("Nombre de page visitées: {}", self.page_count);
        wait_for_enter()?;
        Ok(())
    }

    pub fn initial_env(formula: &mut FormulaEngine, character: &Character, env: &Env) -> Result<Env<'static>> {
        let mut base_map = IndexMap::new();
        base_map.insert("page".into(), EvalValue::Object(IndexMap::new().into()));
        base_map.insert("connexion".into(), EvalValue::Object(IndexMap::new().into()));
        base_map.insert("test".into(), EvalValue::Null);
        let base_env = Env::from_index_map(&base_map);
        Ok(mk_character_base_env(character, formula, &base_env.merged(env))
            .with_context(|| "Construction de l'environnement initial du personnage")?
            .expand(base_env))
    }

    /// Displays title in the runner interface.
    fn show_title(&mut self) -> Result<(Character, Option<Arc<str>>, Option<(Vec<Message>, Option<Arc<str>>, Arc<str>, Env<'static>)>)> {
        loop {
            self.terminal.clear()?;
            self.terminal.print(
                &format!(
                    "{}=== {} ==={}\n",
                    self.terminal.title(),
                    self.terminal.styled_text(
                        &self.tale.title.to_uppercase(),
                        self.terminal.title(),
                    ),
                    self.terminal.reset()
                )
            );
            if let Some(author) = &self.tale.author {
                self.terminal.print(
                    &format!(
                        "{}Par {}{}\n",
                        self.terminal.author(),
                        self.terminal.styled_text(author, self.terminal.author()),
                        self.terminal.reset()
                    )
                );
            }
            self.terminal.print(&format!(
                "{}\n",
                self.terminal
                    .styled_text(&self.tale.synopsis, self.terminal.reset())
            ));
            for img_id in self.tale.title_images.iter() {
                let img = self.tale.images.get_and_cache(img_id, &self.client);
                let _ = self.terminal.show(img, img_id);
            }

            match self.choose_basic(&["Commencer".into(), "Charger".into(), "Quitter".into()])? {
                0 => {
                    let name = prompt("Entre ton nom: ")?;
                    return Ok((
                        Character::new_with_tags(
                            &self.tale.attributes,
                            Some(name.into()),
                            &self.tale.initial_tags
                                .iter()
                                .map(|(k, v)| (EvalKey(k.clone().into()), v.clone()))
                                .collect(),
                        ),
                        Some(self.tale.start.clone()),
                        None,
                    ));
                }
                1 => {
                    let filename = prompt_default(
                        "Entrez le nom du fichier de sauvegarde (défaut 'save.json'): ",
                        "save.json",
                    )?;
                    match load_state(&self.tale, Path::new(&filename)) {
                        Ok((character, pageid, pagetitle, pagenumber, pagetext, pending, previous_env)) => {
                            self.page_count = pagenumber;
                            return Ok((character, Some(pageid.into()), Some((pagetext, pending.map(Arc::from), pagetitle.into(), previous_env))));
                        }
                        Err(error) => {
                            eprintln!(
                                "{}{}\nVeuillez recommencer ou créer une nouvelle partie.{}",
                                self.terminal.error(),
                                format_error(&error),
                                self.terminal.reset()
                            );
                            wait_for_enter()?;
                        }
                    }
                }
                _ => {
                    std::process::exit(0);
                }
            }
        }
    }

    /// Executes cells in order and stops immediately after a forced redirect.
    ///
    /// When `execute_effects` is false (for example after loading a save), cell
    /// text is rendered but persistent mutations and polls are not repeated.
    ///
    /// Effects are processed in deterministic order. The return value carries any visible
    /// messages, poll result, defeat state, or forced destination produced by the operation.
    fn apply_cells(
        &mut self,
        cells: &[Cell],
        character: &mut Character,
        page: &Page,
        base_env: &Env,
        progress: &mut PageProgress,
        next_page: &mut Option<Arc<str>>,
    ) -> Result<()> {
        for (cell_index, cell) in cells.iter().enumerate() {
            if next_page.is_some() {
                break;
            }
            let cell_name = cell
                .id
                .as_deref()
                .or(cell.label.as_deref())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("#{}", cell_index + 1));
            let cell_context = || {
                format!("Cellule `{cell_name}` de la page `{}`", page.id)
            };
            // extract page and connection tags from the environment for conditional evaluation
            let evaluated_page_tags = match base_env.get("page") {
                Some(EvalValue::Object(tags)) => tags.clone(),
                _ => Arc::new(IndexMap::new()),
            };
            let evaluated_connection_tags = match base_env.get("connexion") {
                Some(EvalValue::Object(tags)) => tags.clone(),
                _ => Arc::new(IndexMap::new()),
            };
            if !cell.applies_with_env(
                character,
                evaluated_page_tags,
                evaluated_connection_tags,
                &self.tale,
                &mut self.formula,
                base_env,
            )
            .with_context(|| format!("Évaluation des conditions de la {}", cell_context()))?
            {
                continue;
            }
            let denv = mk_character_env(character, &mut self.formula, &base_env)
                .with_context(|| format!("Construction de l'environnement pour évaluer les effets de la cellule {}", cell_context()))?
                .with_parent(&base_env);
            let effects = PageEffect::eval_many(&cell.effects, &mut self.formula, &denv)
                .with_context(|| format!("Construction des effets de la {}", cell_context()))?;
            for (effect_index, effect) in effects.into_iter().enumerate() {
                if matches!(&effect, crate::effects::PageEffect::Poll { .. }) {
                    let charenv = mk_character_env(character, &mut self.formula, &base_env)
                        .with_context(|| format!("Préparation de l'effet #{} de la {}", effect_index + 1, cell_context()))?
                        .with_parent(base_env);
                    progress
                        .show(None, &mut self.terminal, &mut self.tale.images, &self.client, &mut self.formula, &charenv)
                        .with_context(|| format!("Affichage avant le sondage de la {}", cell_context()))?;
                }
                let env = mk_character_env(character, &mut self.formula, base_env)
                    .with_context(|| format!("Préparation de l'effet #{} de la {}", effect_index + 1, cell_context()))?
                    .with_parent(&base_env);
                let effect_kind = effect.kind();
                let result = effect
                    .apply(
                        character,
                        &self.tale,
                        &mut self.formula,
                        &env,
                        &self.terminal,
                        &mut self.interaction,
                    )
                    .with_context(|| {
                        format!(
                            "Effet #{} `{effect_kind}` de la {}",
                            effect_index + 1,
                            cell_context()
                        )
                    })?;
                character
                    .fix_hp(&mut self.formula, &base_env)
                    .with_context(|| format!("Recalcul des PV après l'effet `{effect_kind}`"))?;
                character
                    .fix_morale(&mut self.formula, &base_env)
                    .with_context(|| format!("Recalcul du moral après l'effet `{effect_kind}`"))?;
                if let Some(message) = result.poll_result {
                    progress.push_content(Message::Poll(String::from(message.as_ref())));
                }
                for notif in result.shows.iter() {
                    progress.push_content(Message::Notification(String::from(notif.as_ref())));
                }
                if let Some(destination) = result.next_page {
                    *next_page = Some(destination);
                    break;
                }
            }
            let charenv = mk_character_env(character, &mut self.formula, &base_env)
                .with_context(|| format!("Construction de l'environnement du texte de la {}", cell_context()))?;
            let env = charenv.merged(&base_env);
            if let Some(text) = cell.text.clone() {
                let text = self
                    .formula
                    .fmt_value(&text, &env)
                    .with_context(|| format!("Résolution du texte de la {}", cell_context()))?;
                progress.push_content(Message::Text(String::from(text.as_ref())));
            }
            let mut resolved_image_ids = Vec::new();
            for img_id in cell.images.iter() {
                match img_id {
                    Deferred::Literal(s) => resolved_image_ids.push(s.to_string()),
                    Deferred::Formula(f) => {
                        let resolved = self.formula.fmt_value(f, &env).with_context(|| "Erreur lors de la résolution d'une formule")?;
                        resolved_image_ids.push(resolved.to_string());
                    },
                }
            }
            for img_id in resolved_image_ids.into_iter() {
                progress.push_content(Message::Image(img_id));
            }
        }
        Ok(())
    }

    /// Executes one lifecycle phase for every state active at the start of that phase.
    ///
    /// Effects are processed in deterministic order. The return value carries any visible
    /// messages, poll result, defeat state, or forced destination produced by the operation.
    fn apply_condition_cells(
        &mut self,
        before_page: bool,
        character: &mut Character,
        page: &Page,
        env: &Env,
        progress: &mut PageProgress,
        next_page: &mut Option<Arc<str>>,
    ) -> Result<()> {
        let conditions = character.conditions.clone();
        for (_idx, condition) in conditions.iter().enumerate() {
            if next_page.is_some() {
                break;
            }
            let cells = if before_page {
                condition.template.pre.clone()
            } else {
                condition.template.post.clone()
            };
            let condition_id = condition.template.id.clone();
            let phase = if before_page { "avant" } else { "apres" };
            let charenv = mk_character_env(character, &mut self.formula, &env)
                .with_context(|| {
                    format!("Environnement des cellules `{phase}` de l'état `{condition_id}`")
                })?;
            let base_env = charenv.merged(&env);

            let condition_env = mk_new_condition_env(&condition, &mut self.formula, &base_env)?.with_parent(&base_env);
            let cells = eval_cells(&cells, &mut self.formula, &condition_env)
                .with_context(|| format!("Erreur en évaluant les cellules de l'état {}", condition_id))?;
            if cells.is_empty() {
                continue;
            }
            self.apply_cells(
                &cells,
                character,
                page,
                &condition_env,
                progress,
                next_page,
            )
            .with_context(|| format!("Cellules `{phase}` de l'état `{condition_id}`"))?;
        }
        Ok(())
    }

    /// Executes one page lifecycle, including effects, polls, text, and connection selection.
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    fn run_page(
        &mut self,
        page: &Page,
        character: &mut Character,
        connection_data: (Arc<[Deferred<Cell>]>, &Env),
        recovery: Option<(Vec<Message>, Option<Arc<str>>, Arc<str>, Env)>,
        check_result: Option<&CheckResult>,
        _previous_env: &Env,
    ) -> Result<LoopResult> {
        let tenv = self.tale.env();

        let (progress, next_page, pagetitle, base_env) = if let Some((progress, pending_page, pagetitle, base_env)) = recovery {
            let progress = PageProgress {
                page_number: self.page_count,
                page,
                content: progress,
            };
            (progress, pending_page, Some(pagetitle), base_env)
        } else {
            let mut next_page = None;

            let mut progress = PageProgress::new(page, self.page_count);

            let (added_cells, incoming_connection_env) = connection_data;

            // Loading a save or entering the initial page may not have selected a
            // connection. Keep the page-entry contract stable by exposing an empty
            // `connexion` object in that case.
            let mut empty_connection_env = Env::new();
            empty_connection_env.insert(
                "connexion",
                EvalValue::Object(Arc::new(IndexMap::new())),
            );
            let connection_env =
                Env::merge(&[&empty_connection_env, incoming_connection_env]).solidified();


            let cenv = if let Some(check_result) = check_result {
                let mut env = Env::new();
                env.insert(
                    "test",
                    EvalValue::Object(check_result.into_dict().into()),
                );
                env
            } else {
                let mut env = Env::new();
                env.insert("test", EvalValue::Null);
                env
            };
            // Page tags are entry-time constants. Deliberately expose only the
            // incoming connection tags, its resolved test, and tale globals here.
            let tag_env = Env::merge(&[&cenv, &connection_env, &tenv]);
            let penv = mk_page_env(page, &mut self.formula, &tag_env)
                .with_context(|| {
                    format!(
                        "Évaluation des tags immuables de la page `{}`",
                        page.id
                    )
                })?;
            let base_env = Env::merge(&[&cenv, &connection_env, &penv]).solidified();
            // `test` + `connexion` + `page` + `recit` form the immutable page-entry environment.
            let cenv = Env::merge(&[&base_env, &tenv]);
            // Connection cells
            {
                let phase_env = mk_character_env(character, &mut self.formula, &cenv)
                    .context("Environnement des cellules de connexion")?
                    .with_parent(&cenv);
                let cells = eval_cells(&added_cells, &mut self.formula, &phase_env)
                    .with_context(|| "Erreur en évaluant les cellules de connexion")?;
                self.apply_cells(
                    &cells, 
                    character, 
                    page, 
                    &cenv, 
                    &mut progress, 
                    &mut next_page
                )
                .context("Phase: cellules de la connexion entrante")?;
            }
            
            // tale pre cells
            {
                let phase_env = mk_character_env(character, &mut self.formula, &cenv)
                    .context("Environnement des cellules globales `avant`")?
                    .with_parent(&cenv);
                let cells = eval_cells(&self.tale.pre, &mut self.formula, &phase_env)
                        .with_context(|| "Erreur en évaluant les cellules avant page du récit")?;
                self.apply_cells(
                    &cells,
                    character,
                    page,
                    &cenv,
                    &mut progress,
                    &mut next_page,
                )
                .context("Phase: cellules globales `avant`")?;
            }

            // condition pre cells
            self.apply_condition_cells(
                true,
                character,
                page,
                &cenv,
                &mut progress,
                &mut next_page,
            )
            .context("Phase: cellules `avant` des états")?;

            // page cells
            {
                let phase_env = mk_character_env(character, &mut self.formula, &cenv)
                    .context("Environnement des cellules de la page")?
                    .with_parent(&cenv);
                let cells = eval_cells(&page.cells, &mut self.formula, &phase_env)
                        .with_context(|| "Erreur en évaluant les cellules de la page")?;
                self.apply_cells(
                    &cells,
                    character,
                    page,
                    &cenv,
                    &mut progress,
                    &mut next_page,
                )
                .context("Phase: cellules de la page")?;
            }

            // condition post cells
            self.apply_condition_cells(
                false,
                character,
                page,
                &cenv,
                &mut progress,
                &mut next_page,
            )
            .context("Phase: cellules `apres` des états")?;

            // tale post cells
            {
                let phase_env = mk_character_env(character, &mut self.formula, &cenv)
                    .context("Environnement des cellules globales `apres`")?
                    .with_parent(&cenv);
                let cells = eval_cells(&self.tale.post, &mut self.formula, &phase_env)
                        .with_context(|| "Erreur en évaluant les cellules après page du récit")?;
                self.apply_cells(
                    &cells,
                    character,
                    page,
                    &cenv,
                    &mut progress,
                    &mut next_page,
                )
                .context("Phase: cellules globales `apres`")?;
            }

            if next_page.is_none() {
                self.expire_conditions(character, &mut next_page, &mut progress, &cenv)
                    .context("Phase: expiration des états")?;
            }
            (progress, next_page, None, base_env)
        };

        // work env
        let wenv = Env::merge(&[&tenv, &base_env]);
        let final_base_env = mk_character_env(character, &mut self.formula, &wenv)
            .with_context(|| format!("Erreur lors de l'évaluation de l'environement final pour la page {}", page.id))?
            .with_parent(&base_env)
            .solidified(); // final base environment that will be returned to the next page, including character state and page env
        let fenv = Env::merge(&[&tenv, &final_base_env]); 

        let title = if let Some(title) = pagetitle {
            title
        } else {
            page.actual_title(&mut self.formula, &fenv)?
        };

        let mut additional_text = StringBuilder::new();

        if !character.is_alive() && next_page.is_some() {
            additional_text.push(self.terminal.error().into());
            additional_text.push("Tu t'évanouis !".into());
            additional_text.push(self.terminal.reset().into());
            additional_text.push('\n'.into());
        }
        additional_text.push("\n".into());

        let additional_text = additional_text.build();
        if let Some(destination) = next_page {
            match self.menu(
                &additional_text,
                &["Continuer".into()],
                character,
                title,
                page.id.clone(),
                &progress,
                Some(destination.clone()),
                &fenv,
                &final_base_env,
            )? {
                MenuResult::Choice(_) => Ok(LoopResult::Next{
                    page_result: PageResult {
                        next_page: Some(destination),
                        ..Default::default()
                    },
                    previous_base_env: final_base_env,
                }),
                MenuResult::Load {
                    character,
                    pageid,
                    pagetitle,
                    pagetext,
                    pending,
                    page_count,
                    previous_base_env,
                } => Ok(LoopResult::Load {
                    character,
                    pageid,
                    pagetitle,
                    pagetext,
                    pending,
                    pagenumber: page_count,
                    previous_base_env,
                }),
                MenuResult::Quit => Ok(LoopResult::Quit),
            }
        } else {
            let connections = page
                .valid_connections(character, &self.tale, &mut self.formula, &fenv)
                .with_context(|| format!("Calcul des connexions visibles de la page `{}`", page.id))?;
            if connections.is_empty() {
                return match self.menu(
                        &additional_text, 
                        &["Fin".into()], 
                        character,
                        title, 
                        page.id.clone(), 
                        &progress, 
                        None, 
                        &fenv,
                        &final_base_env
                    )? {
                    MenuResult::Choice(_) => Ok(LoopResult::End),
                    MenuResult::Load {
                        character,
                        pageid,
                        pagetitle,
                        pagetext,
                        pending,
                        page_count,
                        previous_base_env,
                    } => Ok(LoopResult::Load {
                        character,
                        pageid,
                        pagetitle,
                        pagetext,
                        pending,
                        pagenumber: page_count,
                        previous_base_env,
                    }),
                    MenuResult::Quit => Ok(LoopResult::Quit),
                };
            }

            let mut options = Vec::with_capacity(connections.len());
            for (index, connection) in connections.iter().enumerate() {
                options.push(
                    connection
                        .shown_text(&mut self.formula, &fenv)
                        .with_context(|| {
                            format!(
                                "Rendu de la connexion #{} de la page `{}`",
                                index + 1,
                                page.id
                            )
                        })?,
                );
            }
            match self.menu(
                    &additional_text, 
                    &options, 
                    character, 
                    title, 
                    page.id.clone(), 
                    &progress, 
                    None, 
                    &fenv,
                    &final_base_env
                )? {
                MenuResult::Choice(index) => Ok(LoopResult::Next{
                    page_result: connections[index]
                        .visit(character, &mut self.formula, &fenv)
                        .with_context(|| {
                            format!(
                                "Sélection de la connexion #{} `{}` depuis la page `{}`",
                                index + 1,
                                options[index],
                                page.id
                            )
                        })?,
                    previous_base_env: final_base_env,
                }),
                MenuResult::Load {
                    character,
                    pageid,
                    pagetitle,
                    pagetext,
                    pending,
                    page_count,
                    previous_base_env,
                } => Ok(LoopResult::Load {
                    character,
                    pageid,
                    pagetitle,
                    pagetext,
                    pending,
                    pagenumber: page_count,
                    previous_base_env,
                }),
                MenuResult::Quit => Ok(LoopResult::Quit),
            }
        }
    }

    /// Removes expired states and returns any defeat redirect they trigger.
    fn expire_conditions(
        &mut self,
        character: &mut Character,
        next_page: &mut Option<Arc<str>>,
        progress: &mut PageProgress,
        env: &Env,
    ) -> Result<()> {
        let base_env = mk_character_base_env(character, &mut self.formula, &env)?.with_parent(&env);
        let mut expired_indices = Vec::new();
        for (index, condition) in character.conditions.iter().enumerate() {
            if condition
                .is_expired(&mut self.formula, &base_env)
                .with_context(|| {
                    format!("Test d'expiration de l'état `{}`", condition.template.id)
                })?
            {
                expired_indices.push(index);
            }
        }

        let mut removed = 0;
        let mut death_page = None;
        let base_env = mk_character_base_env(character, &mut self.formula, &env)?.with_parent(&env);
        for original_index in expired_indices {
            let index = original_index - removed;
            let condition = character.conditions.remove(index);
            removed += 1;
            progress.push_content(
                Message::Notification(format!(
                    "Ton état '{}' a expiré.",
                    condition.actual_name(&mut self.formula, &base_env)?
                )),
            );
            character.fix_hp(&mut self.formula, &base_env)?;
            character.fix_morale(&mut self.formula, &base_env)?;
            if !character.is_alive() && death_page.is_none() {
                let env = mk_condition_env(&base_env, original_index)?.with_parent(&base_env);
                death_page = condition.template.death.as_ref().map(|d| self.formula.fmt_value(&d, &env)).transpose()?;
            } else if character.is_alive() {
                death_page = None;
            }
        }
        if death_page.is_some() {
            *next_page = death_page;
        }
        Ok(())
    }

    /// Displays the page menu and reads the next player command or connection.
    fn menu(
        &mut self,
        text: &str,
        options: &[Arc<str>],
        character: &mut Character,
        pagetitle: Arc<str>,
        current_page: Arc<str>,
        progress: &PageProgress,
        pending_page: Option<Arc<str>>,
        env: &Env,
        previous_env: &Env,
    ) -> Result<MenuResult> {
        loop {
            self.terminal.clear()?;
            let _ = progress.show(
                Some(pagetitle.clone()), 
                &mut self.terminal,
                &mut self.tale.images,
                &self.client,
                &mut self.formula,
                &env,
            )?;
            print!("\n{text}");
            for (index, option) in options.iter().enumerate() {
                let option = self
                    .terminal
                    .styled_text(option, self.terminal.reset());
                println!("({}) {option}", index + 1);
            }
            let mut labels = COMMANDS
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>();
            if self.debug {
                labels.insert(labels.len() - 1, DEBUG_COMMAND.1);
            }
            println!("{}", labels.join(" "));
            println!();
            io::stdout().flush()?;

            let input = prompt("> ")?;
            if let Ok(choice) = input.parse::<usize>() {
                if (1..=options.len()).contains(&choice) {
                    return Ok(MenuResult::Choice(choice - 1));
                }
            }
            match input.to_uppercase().as_str() {
                "S" => self.show_status(character, env)?,
                "I" => self.show_inventory(character, env)?,
                "E" => self.show_conditions(character, env)?,
                "A" => self.save_game(character, 
                    &PageProgress { 
                        page_number: self.page_count, 
                        page: self.tale.pages.get(current_page.as_ref()).ok_or_else(|| anyhow!("Page '{current_page}' introuvable dans l'histoire"))?, 
                        content: progress.content.clone(),
                    },
                        pagetitle.clone(),
                        pending_page.clone(),
                        previous_env,
                )?,
                "C" => {
                    if let Some((character, pageid, pagetitle, page_count, pagetext, pending, previous_env)) = self.load_game()? {
                        return Ok(MenuResult::Load {
                            character,
                            pageid,
                            pagetitle,
                            pagetext,
                            pending,
                            previous_base_env: previous_env,
                            page_count,
                        });
                    }
                }
                "T" if self.debug => self.show_tags(character)?,
                "Q" => {
                    self.terminal.clear()?;
                    println!(
                        "{}=== QUITTER ==={}\n\nMerci d'avoir joué !",
                        self.terminal.title(),
                        self.terminal.reset()
                    );
                    return Ok(MenuResult::Quit);
                }
                _ => {
                    let commands = if self.debug {
                        "S, I, E, A, C, T ou Q"
                    } else {
                        "S, I, E, A, C ou Q"
                    };
                    eprintln!(
                        "{}Choix invalide. Entre un nombre entre 1 et {}, ou {commands}.{}",
                        self.terminal.error(),
                        options.len(),
                        self.terminal.reset()
                    );
                    wait_for_enter()?;
                }
            }
        }
    }

    /// Prompts the user to choose basic.
    ///
    /// Uses the current value as the initial selection when possible. Escape is reported
    /// as `PromptCancelled` rather than as malformed input.
    fn choose_basic(&self, options: &[Arc<str>]) -> Result<usize> {
        for (index, option) in options.iter().enumerate() {
            println!("({}) {option}", index + 1);
        }
        loop {
            println!();
            let input = prompt("> ")?;
            if let Ok(choice) = input.parse::<usize>() {
                if (1..=options.len()).contains(&choice) {
                    return Ok(choice - 1);
                }
            }
            eprintln!(
                "{}Choix invalide, veuillez rentrer un nombre entre 1 et {}.{}",
                self.terminal.error(),
                options.len(),
                self.terminal.reset()
            );
        }
    }

    /// Displays status in the runner interface.
    fn show_status(&mut self, character: &Character, env: &Env) -> Result<()> {
        self.terminal.clear()?;
        println!(
            "{}=== STATUT ==={}\n",
            self.terminal.title(),
            self.terminal.reset()
        );
        if let Some(custom) = &self.tale.custom_status {
            let charenv = mk_character_env(character, &mut self.formula, &env)
                .context("Construction de l'environnement de l'écran de statut")?;
            let env = charenv.with_parent(&env);
            let resolved_string = self
                .formula
                .fmt_value(custom, &env)
                .context("Rendu du statut personnalisé")?;
            let formatted_string = self.terminal.styled_text(&resolved_string, self.terminal.reset());
            println!("{formatted_string}");
        } else {
            if let Some(name) = &character.name {
                println!(
                    "Nom: {}",
                    self.terminal
                        .styled_text(name, self.terminal.reset())
                );
            }
            println!(
                "PV: {}/{}",
                character.hp,
                character
                    .actual_max_hp(&mut self.formula, &env)
                    .context("Affichage du statut: calcul des PV maximum")?
            );
            println!(
                "Moral: {}/{}",
                character.morale,
                character
                    .actual_max_morale(&mut self.formula, &env)
                    .context("Affichage du statut: calcul du moral maximum")?
            );
            for attribute in character.attributes.keys() {
                println!(
                    "{attribute}: {}",
                    character
                        .actual_attribute(attribute, &mut self.formula, &env)
                        .with_context(|| {
                            format!("Affichage du statut: calcul de l'attribut `{attribute}`")
                        })?
                );
            }
        }
        wait_for_enter()
    }

    /// Displays conditions in the runner interface.
    fn show_conditions(&mut self, character: &Character, env: &Env) -> Result<()> {
        self.terminal.clear()?;
        println!(
            "{}=== ÉTATS ==={}\n",
            self.terminal.title(),
            self.terminal.reset()
        );
        if character.conditions.is_empty() {
            println!("Vous n'avez aucun état.");
        } else {
            let base_env = mk_character_env(character, &mut self.formula, &env)
                .context("Construction de l'environnement de l'écran des états")?
                .with_parent(&env);
            for condition in &character.conditions {
                let condition_id = &condition.template.id;
                let name = condition
                    .actual_name(&mut self.formula, &base_env)
                    .with_context(|| format!("Rendu du nom de l'état `{condition_id}`"))?;
                let description = condition
                    .actual_description(&mut self.formula, &base_env)
                    .with_context(|| format!("Rendu de la description de l'état `{condition_id}`"))?;
                let name = self
                    .terminal
                    .styled_text(&name, self.terminal.reset());
                let description = self
                    .terminal
                    .styled_text(&description, self.terminal.reset());
                if description.is_empty() {
                    println!("- {name}");
                } else {
                    println!("- {name}: {description}");
                }
            }
        }
        wait_for_enter()
    }

    /// Displays inventory in the runner interface.
    fn show_inventory(&mut self, character: &Character, env: &Env) -> Result<()> {
        self.terminal.clear()?;
        println!(
            "{}=== INVENTAIRE ==={}\n",
            self.terminal.title(),
            self.terminal.reset()
        );
        if character.inventory.is_empty() {
            println!("Vous n'avez aucun objet.");
        } else {
            let base_env = mk_character_env(character, &mut self.formula, &env)
                .context("Construction de l'environnement de l'inventaire")?
                .with_parent(&env);
            for (stack_index, stack) in character.inventory.iter().enumerate() {
                let item_id = &stack.item.id;
                let stack_context = || {
                    format!("pile d'objet #{} `{item_id}`", stack_index + 1)
                };
                let name = stack
                    .actual_name(&mut self.formula, &base_env)
                    .with_context(|| format!("Rendu du nom de la {}", stack_context()))?;
                let description = stack
                    .actual_description(&mut self.formula, &base_env)
                    .with_context(|| format!("Rendu de la description de la {}", stack_context()))?;
                let name = self
                    .terminal
                    .styled_text(&name, self.terminal.reset());
                let description = self
                    .terminal
                    .styled_text(&description, self.terminal.reset());
                let quantity = if stack.quantity > 1 {
                    format!(" x{}", stack.quantity)
                } else {
                    String::new()
                };
                if description.is_empty() {
                    println!("- {name}{quantity}");
                } else {
                    println!("- {name}{quantity}: {description}");
                }
            }
        }
        wait_for_enter()
    }

    /// Displays every mutable story tag when debug mode is enabled.
    fn show_tags(&self, character: &Character) -> Result<()> {
        self.terminal.clear()?;
        println!(
            "{}=== TAGS ==={}\n",
            self.terminal.title(),
            self.terminal.reset()
        );
        if character.story_tags.is_empty() {
            println!("Aucun tag narratif.");
        } else {
            for (tag, value) in character.story_tags.iter() {
                let rendered = value.pretty_print();
                if rendered.contains('\n') {
                    println!("{tag}:\n{rendered}");
                } else {
                    println!("{tag}: {rendered}");
                }
            }
        }
        wait_for_enter()
    }
    /// Prompts for a save file and loads it without terminating the current process.
    ///
    /// Load errors are displayed to the player and returned as `Ok(None)` after acknowledgment; input/terminal failures are propagated.
    fn load_game(&self) -> Result<Option<(Character, Arc<str>, Arc<str>, usize, Vec<Message>, Option<Arc<str>>, Env<'static>)>> {
        self.terminal.clear()?;
        let filename = prompt_default(
            "Entrez le nom du fichier de sauvegarde (défaut 'save.json'): ",
            "save.json",
        )?;
        match load_state(&self.tale, Path::new(&filename)) {
            Ok((char, curr_page, title, page_number, page_content, pending_page, previous_env)) => Ok(Some(
                (
                    char,
                    curr_page.into(),
                    title.into(),
                    page_number,
                    page_content,
                    pending_page.map(Arc::from),
                    previous_env,
                )
            )),
            Err(error) => {
                eprintln!(
                    "{}=== ERREUR LORS DU CHARGEMENT ===\n{}{}",
                    self.terminal.error(),
                    format_error(&error),
                    self.terminal.reset()
                );
                wait_for_enter()?;
                Ok(None)
            }
        }
    }
    /// Prompts for a destination and writes the current character, rendered page snapshot, and pending redirect.
    ///
    /// Save errors are displayed to the player; prompt/terminal failures are propagated.
    fn save_game(
        &self,
        character: &Character,
        progress: &PageProgress,
        title: Arc<str>,
        pending_page: Option<Arc<str>>,
        previous_env: &Env,
    ) -> Result<()> {
        self.terminal.clear()?;
        let filename = prompt_default(
            "Entrez le nom du fichier pour sauvegarder l'état du jeu (par défaut : save.json) : ",
            "save.json",
        )?;
        let path = PathBuf::from(filename);
        let title = String::from(title.as_ref());
        let pending_page = pending_page.map(|p| String::from(p.as_ref()));
        match save_state(character, progress, title, pending_page.as_deref(), &path, previous_env) {
            Ok(()) => println!(
                "{}=== JEU SAUVÉ ==={}\n",
                self.terminal.title(),
                self.terminal.reset()
            ),
            Err(error) => eprintln!(
                "{}=== ERREUR LORS DE LA SAUVEGARDE ===\n{}{}",
                self.terminal.error(),
                format_error(&error),
                self.terminal.reset()
            ),
        }
        wait_for_enter()
    }
}

/// Provides the concrete command-line interaction used by the tale runner.
#[derive(Clone, Copy)]
pub struct Terminal {
    pub clear: bool,
    pub color: bool,
    pub printer: fn(&str),
}

impl Terminal {
    /// Enters the runner terminal mode and returns its restoration guard.
    fn session(&self) -> Result<TerminalSession> {
        if self.clear {
            //(self.printer)(&format!("{ENTER_ALTERNATE_SCREEN}{CLEAR_SEQUENCE}"));
            //io::stdout().flush()?;
            execute!(
                stdout(),
//                crossterm::terminal::EnterAlternateScreen, 
                crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
                crossterm::cursor::MoveTo(0, 0),
            )?;
        }
        Ok(TerminalSession { active: self.clear })
    }

    /// Clears the visible terminal and its scrollback when supported.
    fn clear(&self) -> Result<()> {
        if self.clear {
            //(self.printer)(&format!("{CLEAR_SEQUENCE}"));
            //stdout().flush()?;
            execute!(
                stdout(), 
                crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
                crossterm::cursor::MoveTo(0, 0),
            )?;
        }
        Ok(())
    }
    /// Returns the ANSI sequence used for title text, or an empty string when color is disabled.
    fn title(&self) -> &'static str {
        if self.color {
            "\x1b[32m\x1b[1m"
        } else {
            ""
        }
    }

    /// Returns the ANSI style used for image messages, or an empty string when color is disabled.
    fn image(&self) -> &'static str {
        if self.color {
            "\x1b[35m"
        } else {
            ""
        }
    }

    /// Appends effects text using the corresponding message style.
    fn effects(&self) -> &'static str {
        if self.color {
            "\x1b[33m"
        } else {
            ""
        }
    }

    /// Appends error text using the corresponding message style.
    fn error(&self) -> &'static str {
        if self.color {
            "\x1b[31m"
        } else {
            ""
        }
    }

    /// Returns the ANSI style used for news messages, or an empty string when color is disabled.
    fn news(&self) -> &'static str {
        if self.color {
            "\x1b[92m"
        } else {
            ""
        }
    }

    /// Appends author text using the corresponding message style.
    fn author(&self) -> &'static str {
        if self.color {
            "\x1b[34m"
        } else {
            ""
        }
    }

    /// Presents a poll request and returns the validated answer.
    fn poll(&self) -> &'static str {
        if self.color {
            "\x1b[34m"
        } else {
            ""
        }
    }

    /// Returns the ANSI style used for `$[…]` highlighted text.
    fn highlight(&self) -> &'static str {
        if self.color {
            "\x1b[95m"
        } else {
            ""
        }
    }

    /// Removes `$[…]` delimiters and colors their contents when colors are enabled.
    fn styled_text(&self, text: &str, resume_style: &str) -> String {
        render_highlights(text, self.highlight(), resume_style)
    }

    /// Restores the terminal to its normal screen and input mode.
    fn reset(&self) -> &'static str {
        if self.color {
            "\x1b[0m"
        } else {
            ""
        }
    }

    /// Print text on the terminal using the current message style.
    fn print(&mut self, message: &str) {
        print!("{message}");
        io::stdout().flush().unwrap();
    }

    /// Show an image on the terminal
    fn show(&mut self, img: Option<ImageResult>, key: &str) -> Result<()> {
        match img {
            Some(ImageResult::Defined(img)) => {
                execute!(stdout(), MoveToNextLine(1))?;
                let (terminal_width, terminal_height) =
                    crossterm::terminal::size().unwrap_or((80, 24));

                let max_width = terminal_width.saturating_sub(4).min(256);
                let max_height = terminal_height.saturating_sub(4).min(256);

                let (width, height) =
                    fit_in_terminal(img.as_ref(), max_width, max_height);
                let centering_offset = (terminal_width.saturating_sub(width as u16)) / 2;
                let cfg = viuer::Config {
                    absolute_offset: false,
                    restore_cursor: false,
                    x: centering_offset,
                    width: Some(width),
                    height: Some(height),
                    ..Default::default()
                };
                if viuer::print(&img, &cfg).is_err() {
                    print!("{}",self.error());
                    print!("<Image {key} n'a pu être affichée>");
                    print!("{}", self.reset());
                }
            },
            Some(ImageResult::Alt(alt)) => {
                print!("{}",self.image());
                if let Some(alt) = alt {
                    print!("<{alt}>");
                } else {
                    print!("<Image {key}>")
                }
                print!("{}", self.reset());
            },
            None => {
                print!("{}",self.error());
                print!("<Image {key} n'a pu être affichée>");
                print!("{}", self.reset());
            },
        }
        Ok(())
    }
}

/// Describes how a page or menu loop should continue.
enum LoopResult {
    Next{
        page_result: PageResult,
        previous_base_env: Env<'static>,
    },
    Load {
        character: Character,
        pageid: Arc<str>,
        pagetitle: Arc<str>,
        pagenumber: usize,
        pagetext: Vec<Message>,
        pending: Option<Arc<str>>,
        previous_base_env: Env<'static>,
    },
    End,
    Quit,
}

/// Guards the player terminal mode and restores it when dropped.
struct TerminalSession {
    active: bool,
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.active {
            execute!(
                stdout(), 
                //crossterm::terminal::LeaveAlternateScreen,
            ).unwrap();
        }
    }
}

/// Represents the result of reading a menu command or connection choice.
enum MenuResult {
    Choice(usize),
    Load {
        character: Character,
        pageid: Arc<str>,
        pagetitle: Arc<str>,
        pagetext: Vec<Message>,
        pending: Option<Arc<str>>,
        page_count: usize,
        previous_base_env: Env<'static>,
    },
    Quit,
}

/// Reads one line from standard input after displaying a prompt.
fn prompt(message: &str) -> Result<String> {
    print!("{message}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_owned())
}

/// Prompts the user for default.
///
/// Uses the current value as the initial selection when possible. Escape is reported
/// as `PromptCancelled` rather than as malformed input.
fn prompt_default(message: &str, default: &str) -> Result<String> {
    let input = prompt(message)?;
    if input.is_empty() {
        Ok(default.to_owned())
    } else {
        Ok(input)
    }
}

/// Waits until the user confirms with Enter.
fn wait_for_enter() -> Result<()> {
    print!("\nAppuie sur Entrée pour revenir...");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}

/// Classifies runner messages for formatting and ordering.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageType {
    Notification,
    Text,
    Poll,
    Alert,
    News,
    Image,
}
/// Appends a sequence of typed messages to the output buffer.
#[allow(dead_code)]
fn push_messages(term: &Terminal, messages: &[Message], text: &mut StringBuilder) {
    let mut notification: Option<MessageType> = None;
    for message in messages {
        match message {
            Message::Notification(message) => {
                if notification != Some(MessageType::Notification) {
                    text.push(term.effects().into());
                    notification = Some(MessageType::Notification);
                }
                text.push(term.styled_text(message, term.effects()));
            }
            Message::Text(message) => {
                if notification != Some(MessageType::Text) {
                    text.push(term.reset().into());
                    notification = Some(MessageType::Text);
                }
                text.push("\n".into());
                text.push(term.styled_text(message, term.reset()));
            }
            Message::Poll(message) => {
                if notification != Some(MessageType::Poll) {
                    text.push(term.poll().into());
                    notification = Some(MessageType::Poll);
                }
                text.push(term.styled_text(message, term.poll()));
            }
            Message::Alert(message) => {
                if notification != Some(MessageType::Alert) {
                    text.push(term.error().into());
                    notification = Some(MessageType::Alert);
                }
                text.push(term.styled_text(message, term.error()));
            }
            Message::News(message) => {
                if notification != Some(MessageType::News) {
                    text.push(term.news().into());
                    notification = Some(MessageType::News);
                }
                text.push(term.styled_text(message, term.news()));
            },
            Message::Image(_img_id) => {},
        }
        text.push('\n'.into());
    }
    text.push(term.reset().into());
}

/// Replaces every complete `$[…]` segment with an optionally styled span.
///
/// A decimal RGB integer in `0..=0xFFFFFF` or a `#RRGGBB` prefix followed by `|`
/// selects a custom foreground color. An empty `highlight_style`
/// disables colors but still removes valid color prefixes and span delimiters.
/// Unclosed markers are copied literally. `resume_style` is inserted after each highlighted span
/// so the caller's surrounding message style remains active.
fn render_highlights(text: &str, highlight_style: &str, resume_style: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("$[") {
        output.push_str(&remaining[..start]);
        let content_start = start + 2;
        let Some(end) = remaining[content_start..].find(']') else {
            output.push_str(&remaining[start..]);
            return output;
        };
        let content = &remaining[content_start..content_start + end];
        let custom_color = content.split_once('|').and_then(|(header, body)| {
            let rgb = if let Some(hex) = header.strip_prefix('#') {
                if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                u32::from_str_radix(hex, 16).ok()?
            } else {
                if header.is_empty() || !header.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                header.parse::<u32>().ok()?
            };
            (rgb <= 0xFFFFFF).then_some((rgb, body))
        });
        if let Some((rgb, body)) = custom_color {
            if !highlight_style.is_empty() {
                output.push_str(&format!(
                    "\x1b[38;2;{};{};{}m",
                    (rgb >> 16) & 0xff,
                    (rgb >> 8) & 0xff,
                    rgb & 0xff
                ));
            }
            output.push_str(body);
        } else {
            output.push_str(highlight_style);
            output.push_str(content);
        }
        output.push_str(resume_style);
        remaining = &remaining[content_start + end + 1..];
    }
    output.push_str(remaining);
    output
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;

use super::{
        Game, Interaction, PageProgress, Terminal, COMMANDS, DEBUG_COMMAND
    };
    use crate::{character::Character, formula::{mk_character_base_env, mk_character_env, mk_page_env}};
    use formulas::{Env, EvalKey, EvalValue};
    use crate::model::{Tale, eval_cells};
    use anyhow::Result;
    use indexmap::IndexMap;
    use serde_json::json;

    /// Replaces the interaction output sink with a no-op sink for tests.
    fn discard_output(_: &str) {}

    /// Returns an empty scripted poll answer for a test interaction.
    fn empty_answer(_: &str) -> Result<Arc<str>> {
        Ok(String::new().into())
    }

    /// Returns the scripted numeric answer used by poll tests.
    fn answer_seven(_: &str) -> Result<Arc<str>> {
        Ok("7".into())
    }

    /// Returns the scripted second-choice answer used by poll tests.
    fn answer_second_choice(_: &str) -> Result<Arc<str>> {
        Ok("2".into())
    }

    /// Creates an isolated in-memory terminal interaction for engine tests.
    fn test_terminal() -> Terminal {
        Terminal {
            clear: false,
            color: false,
            printer: discard_output,
        }
    }

    // Verifies complete highlight spans, plain-text fallback, and literal incomplete markers.
    #[test]
    fn highlighted_text_uses_magenta_and_preserves_surrounding_style() {
        let plain = test_terminal();
        assert_eq!(
            plain.styled_text("Une $[lueur] puis $[deux]", plain.effects()),
            "Une lueur puis deux"
        );
        assert_eq!(
            plain.styled_text("Une $[lueur inachevée", plain.effects()),
            "Une $[lueur inachevée"
        );

        let colored = Terminal {
            clear: false,
            color: true,
            printer: discard_output,
        };
        assert_eq!(
            colored.styled_text("Une $[lueur] dorée", colored.effects()),
            "Une \x1b[95mlueur\x1b[33m dorée"
        );
    }

    #[test]
    fn highlighted_text_supports_rgb_and_plain_text() {
        let plain = test_terminal();
        let colored = Terminal {
            color: true,
            ..test_terminal()
        };
        let text = "Une $[#fF8800|lueur | dorée] puis $[deux]$[#000000|noir]$[#FFFFFF|blanc].";
        assert_eq!(
            colored.styled_text(text, colored.effects()),
            "Une \x1b[38;2;255;136;0mlueur | dorée\x1b[33m puis \x1b[95mdeux\x1b[33m\x1b[38;2;0;0;0mnoir\x1b[33m\x1b[38;2;255;255;255mblanc\x1b[33m."
        );
        assert_eq!(
            plain.styled_text(text, plain.effects()),
            "Une lueur | dorée puis deuxnoirblanc."
        );
        assert_eq!(plain.styled_text("$[#123456|]", plain.reset()), "");
    }

    #[test]
    fn highlighted_text_supports_integer_rgb_bounds_and_plain_text() {
        let plain = test_terminal();
        let colored = Terminal {
            color: true,
            ..test_terminal()
        };
        let text = "Une $[16746496|lueur | dorée] puis $[0|noir]$[16777215|blanc].";
        assert_eq!(
            colored.styled_text(text, colored.effects()),
            "Une \x1b[38;2;255;136;0mlueur | dorée\x1b[33m puis \x1b[38;2;0;0;0mnoir\x1b[33m\x1b[38;2;255;255;255mblanc\x1b[33m."
        );
        assert_eq!(
            plain.styled_text(text, plain.effects()),
            "Une lueur | dorée puis noirblanc."
        );
        assert_eq!(plain.styled_text("$[0|]", plain.reset()), "");
    }

    #[test]
    fn highlighted_text_preserves_invalid_color_headers_and_unclosed_spans() {
        let plain = test_terminal();
        let colored = Terminal {
            color: true,
            ..test_terminal()
        };
        for body in [
            "#FFF|court",
            "#1234567|long",
            "#GG8800|invalide",
            "#123456",
            "#ééé|texte",
            "choix|autre",
            "16777216|hors limites",
            "4294967296|débordement",
            "999999999999999999999999999999|trop grand",
            "-1|négatif",
            "+1|signe",
            "1.5|flottant",
            " 1|espace",
            "١|chiffre non ASCII",
            "|sans couleur",
            "123456",
        ] {
            let text = format!("$[{body}]");
            assert_eq!(plain.styled_text(&text, plain.reset()), body);
            assert_eq!(
                colored.styled_text(&text, colored.reset()),
                format!("\x1b[95m{body}\x1b[0m")
            );
        }
        for terminal in [plain, colored] {
            for text in ["Une $[#FF8800|lueur inachevée", "Une $[16746496|lueur inachevée"] {
                assert_eq!(terminal.styled_text(text, terminal.reset()), text);
            }
        }
    }

    #[test]
    fn highlighted_text_accepts_interpolated_integer_colors() {
        let mut formula = formulas::FormulaEngine::new(Some(1));
        let mut env = Env::new();
        env.insert("couleur", EvalValue::Int(0x44CC88));
        env.insert("pv", 7.into());
        let colored = Terminal {
            color: true,
            ..test_terminal()
        };
        let plain = test_terminal();
        for (source, pv, rgb) in [
            ("$[${couleur}|PV : ${pv}]", 7, "68;204;136"),
            ("$[${#44CC88}|PV : ${pv}]", 7, "68;204;136"),
            ("$[${#FF4444 if pv < 5 else #44CC88}|PV : ${pv}]", 3, "255;68;68"),
            ("$[${#FF4444 if pv < 5 else #44CC88}|PV : ${pv}]", 7, "68;204;136"),
        ] {
            env.insert("pv", pv.into());
            let value = formulas::split_fmt_str(source).unwrap();
            for text in [
                formula.format(source.into(), &env).unwrap(),
                formula.fmt_value(&value, &env).unwrap(),
            ] {
                assert_eq!(
                    colored.styled_text(&text, colored.reset()),
                    format!("\x1b[38;2;{rgb}mPV : {pv}\x1b[0m")
                );
                assert_eq!(plain.styled_text(&text, plain.reset()), format!("PV : {pv}"));
            }
        }
    }

    // Verifies that null poll default means the answer is required.
    #[test]
    fn null_poll_default_means_the_answer_is_required() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Sondage obligatoire",
            "depart": "debut",
            "tags": {},
            "pages": {
                "debut": {
                    "cellules": [{"effets": [{
                            "type": "demander",
                            "tag": "nombre",
                            "question": "Donne un nombre",
                            "donnees": "int",
                            "defaut": null
                    }]}],
                    "connexions": []
                }
            }
        }))
        .unwrap();

        let page = tale.pages["debut"].clone();
        let cells = page.cells.clone();
        let mut game = Game::new(tale, Some(1), false, false, false);
        game.terminal = test_terminal();
        game.interaction = Interaction::new(answer_seven);

        let mut character = Character::new(&game.tale.attributes, Some("Lene".into()));
        let mut progress = PageProgress::new(&page, 0);
        let mut next_page = None;

        let env = Game::initial_env(&mut game.formula, &character, &game.tale.env()).unwrap();
        let env = game.tale.env()
            .expand(mk_page_env(&page, &mut game.formula, &env).unwrap())
            .with_parent(&env);

        let cells = eval_cells(&cells, &mut game.formula, &env).unwrap();
        game.apply_cells(
            &cells,
            &mut character,
            &page,
            &Env::new(),
            &mut progress,
            &mut next_page,
        )
        .unwrap();

        assert_eq!(character.story_tags.get(&EvalKey("nombre".into())), Some(&EvalValue::Int(7)));
        assert!(next_page.is_none());
    }

    // Verifies that sequential polls see valued page tags and previous answers.
    #[test]
    fn sequential_polls_see_page_tags_and_previous_answers() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Sondages successifs",
            "depart": "debut",
            "tags": {},
            "pages": {
                "debut": {
                    "titre": "Le marais",
                    "tags": {"marais": true},
                    "cellules": [
                        {"effets": [{
                            "type": "demander",
                            "tag": "premiere",
                            "question": "Valeur initiale?",
                            "donnees": "int",
                            "defaut": "${2 if 'marais' in page else 0}"
                        }]},
                        {"effets": [{
                            "type": "demander",
                            "tag": "seconde",
                            "question": "Valeur après $premiere?",
                            "donnees": "int",
                            "defaut": "${premiere + 3}"
                        }]}
                    ],
                    "connexions": []
                }
            }
        }))
        .unwrap();

        let page = tale.pages["debut"].clone();
        let cells = page.cells.clone();
        let mut game = Game::new(tale, Some(2), false, false, false);
        game.terminal = test_terminal();
        game.interaction = Interaction::new(empty_answer);

        let mut character = Character::new(&game.tale.attributes, Some("Lene".into()));
        let mut progress = PageProgress::new(&page, 0);
        let mut next_page = None;

        let env = Game::initial_env(&mut game.formula, &character, &game.tale.env()).unwrap();
        let env = game.tale.env()
            .expand(mk_page_env(&page, &mut game.formula, &env).unwrap())
            .with_parent(&env);

        let cells = eval_cells(&cells, &mut game.formula, &env).unwrap();

        game.apply_cells(
            &cells,
            &mut character,
            &page,
            &env,
            &mut progress,
            &mut next_page,
        )
        .unwrap();

        assert_eq!(
            character.story_tags.get(&EvalKey("premiere".into())),
            Some(&EvalValue::Int(2))
        );
        assert_eq!(
            character.story_tags.get(&EvalKey("seconde".into())),
            Some(&EvalValue::Int(5))
        );

        let cenv = mk_character_base_env(&character, &mut game.formula, &env).unwrap();
        let env = env.expand(cenv);

        let (_title, rendered) = progress
            .build(&game.terminal, &mut game.formula, &env)
            .unwrap();
        assert!(rendered.contains("Valeur initiale? > 2"));
        assert!(rendered.contains("Valeur après 2? > 5"));
    }

    // Verifies that choice poll stores the selected typed value.
    #[test]
    fn choice_poll_stores_the_selected_typed_value() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Choix typé",
            "depart": "debut",
            "tags": {},
            "pages": {
                "debut": {
                    "cellules": [{"effets": [{
                        "type": "demander",
                        "tag": "gnomes",
                        "question": "Combien de gnomes?",
                        "donnees": "int",
                        "choix": [
                            {"valeur": 5, "texte": "$valeur gnomes"},
                            {"valeur": "${3 + 4}", "texte": "$valeur gnomes"}
                        ]
                    }]}],
                    "connexions": []
                }
            }
        }))
        .unwrap();

        let page = tale.pages["debut"].clone();
        let cells = page.cells.clone();
        let mut game = Game::new(tale, Some(3), false, false, false);
        game.terminal = test_terminal();
        game.interaction = Interaction::new(answer_second_choice);

        let mut character = Character::new(&game.tale.attributes, None);
        let mut progress = PageProgress::new(&page, 0);
        let mut next_page = None;

        let env = Game::initial_env(&mut game.formula, &character, &game.tale.env()).unwrap();
        let env = game.tale.env()
            .expand(mk_page_env(&page, &mut game.formula, &env).unwrap())
            .with_parent(&env);

        let cells = eval_cells(&cells, &mut game.formula, &env).unwrap();

        game.apply_cells(
            &cells,
            &mut character,
            &page,
            &env,
            &mut progress,
            &mut next_page,
        )
        .unwrap();

        assert_eq!(character.story_tags.get(&EvalKey("gnomes".into())), Some(&EvalValue::Int(7)));
    }

    /// Verifies that deferred page cells and effects receive the current character environment.
    #[test]
    fn deferred_cells_and_effects_receive_current_character_values() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Structures différées",
            "depart": "debut",
            "tags": {"compteur": 2},
            "pages": {
                "debut": {
                    "cellules": [
                        "${[{'effets': [{'type': 'definir_tag', 'filtre': 'cellule', 'valeur': personnage.tags.compteur + 1, 'notification': ''}]}]}",
                        {
                            "effets": "${[{'type': 'definir_tag', 'filtre': 'effet', 'valeur': cellule + 1, 'notification': ''}]}"
                        }
                    ],
                    "connexions": []
                }
            }
        }))
        .unwrap();
        let page = tale.pages["debut"].clone();
        let mut game = Game::new(tale, Some(31), false, false, false);
        game.terminal = test_terminal();
        let mut character = Character::new_with_tags(
            &game.tale.attributes,
            Some("Lene".into()),
            &game.tale.initial_tags.iter()
                .map(|(k, v)| (EvalKey(k.clone().into()), v.clone()))
                .collect(),
        );
        let tale_env = game.tale.env();
        let entry_env = Env::new();
        let page_env = mk_page_env(&page, &mut game.formula, &tale_env).unwrap();
        let base_env = Env::merge(&[&entry_env, &page_env, &tale_env]);
        let phase_env = mk_character_env(&character, &mut game.formula, &base_env)
            .unwrap()
            .with_parent(&base_env);
        let cells = eval_cells(&page.cells, &mut game.formula, &phase_env).unwrap();
        let mut progress = PageProgress::new(&page, 0);
        let mut next_page = None;
        game.apply_cells(
            &cells,
            &mut character,
            &page,
            &base_env,
            &mut progress,
            &mut next_page,
        )
        .unwrap();

        assert_eq!(character.story_tags.get(&EvalKey("cellule".into())), Some(&EvalValue::Int(3)));
        assert_eq!(character.story_tags.get(&EvalKey("effet".into())), Some(&EvalValue::Int(4)));
    }

    // Verifies state lifecycle cells and their resolved `etat` environment.
    #[test]
    fn condition_cells_receive_metadata_before_and_after_page_cells() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Cycle des états",
            "depart": "debut",
            "tags": {},
            "etats": {
                "brulure": {
                    "nom": "Brûlure $intensite",
                    "donnees": {"intensite": 2},
                    "avant": [
                        "${[{'effets': [{'type': 'definir_tag', 'filtre': 'avant_formule', 'valeur': etat.id, 'notification': ''}]}]}",
                        {
                            "conditions": {"type": "formule", "formule": "${etat.donnees.intensite == 2}"},
                            "texte": "Avant: ${etat.nom}",
                            "effets": [{
                                "type": "definir_tag",
                                "filtre": {"type": "id", "tag": "avant_id"},
                                "valeur": "${etat.id}",
                                "notification": ""
                            }]
                        }
                    ],
                    "apres": [{
                        "effets": [{
                            "type": "definir_tag",
                            "filtre": {"type": "id", "tag": "apres_intensite"},
                            "valeur": "${etat.donnees.intensite}",
                            "notification": ""
                        }]
                    }]
                }
            },
            "pages": {
                "debut": {"cellules": [], "connexions": []}
            }
        }))
        .unwrap();
        let page = tale.pages["debut"].clone();
        let mut game = Game::new(tale, Some(4), false, false, false);
        game.terminal = test_terminal();
        let mut character = Character::new(&game.tale.attributes, None);
        let template = game.tale.conditions["brulure"].clone();
        let env = Game::initial_env(&mut game.formula, &character, &game.tale.env()).unwrap();
        let env = game.tale.env()
            .expand(mk_page_env(&page, &mut game.formula, &env).unwrap())
            .with_parent(&env);
        let condition = template
            .instantiate(
                &&EvalValue::Object(IndexMap::new().into()),
                &mut game.formula,
                &env,
            )
            .unwrap();
        character.conditions.push(condition);
        let mut progress = PageProgress::new(&page, 0);
        let mut next_page = None;

        game.apply_condition_cells(
            true,
            &mut character,
            &page,
            &game.tale.env(),
            &mut progress,
            &mut next_page,
        )
        .unwrap();
        assert_eq!(
            character.story_tags.get(&EvalKey("avant_id".into())),
            Some(&EvalValue::String("brulure".into()))
        );
        assert_eq!(
            character.story_tags.get(&EvalKey("avant_formule".into())),
            Some(&EvalValue::String("brulure".into()))
        );

        let cenv = mk_character_base_env(&character, &mut game.formula, &env).unwrap();
        let env = env.expand(cenv);
        let cells = eval_cells(&page.cells, &mut game.formula, &env).unwrap();
        game.apply_cells(
            &cells,
            &mut character,
            &page,
            &env,
            &mut progress,
            &mut next_page,
        )
        .unwrap();
        game.apply_condition_cells(
            false,
            &mut character,
            &page,
            &game.tale.env(),
            &mut progress,
            &mut next_page,
        )
        .unwrap();
        assert_eq!(
            character.story_tags.get(&EvalKey("apres_intensite".into())),
            Some(&EvalValue::Int(2))
        );
        let cenv = mk_character_base_env(&character, &mut game.formula, &env).unwrap();
        let env = env.expand(cenv);
        let (_title, rendered) = progress
            .build(&game.terminal, &mut game.formula, &env)
            .unwrap();
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open("test_output.txt")
            .unwrap();
        f.write_all(rendered.as_bytes()).unwrap();
        assert!(rendered.contains("Avant: Brûlure 2"));
    }
    // Verifies the load command is always available and the tag command remains debug-only.
    #[test]
    fn runtime_menu_exposes_load_and_debug_tags_commands() {
        assert!(COMMANDS.iter().any(|(key, label)| *key == "C" && label.contains("[C]harger")));
        assert!(!COMMANDS.iter().any(|(key, _)| *key == "T"));
        assert_eq!(DEBUG_COMMAND.0, "T");
        assert!(DEBUG_COMMAND.1.contains("[T]ags"));
    }

}
