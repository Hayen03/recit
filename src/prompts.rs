//! Modal Ratatui prompts and multiline editing primitives used by Forge.
//!
//! Pressing Escape returns [`PromptCancelled`] through `anyhow::Error`; it is distinct from
//! invalid input. Rendering helpers assume an active Forge frame and clear popup regions
//! before drawing.
use std::{cell::RefCell, error::Error, fmt};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use serde_json::Value;

use crate::terminal::AppTerminal;

thread_local! {
    static MODAL_BACKDROP: RefCell<Option<Buffer>> = RefCell::new(None);
}
/// Replaces the global terminal backdrop used behind modal prompts.
pub fn set_backdrop(buffer: Buffer) {
    MODAL_BACKDROP.with(|backdrop| *backdrop.borrow_mut() = Some(buffer));
}

/// Error marker returned when the user cancels an active modal prompt.
#[derive(Debug)]
pub struct PromptCancelled;

impl fmt::Display for PromptCancelled {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("opération annulée")
    }
}

impl Error for PromptCancelled {}

/// Returns whether an error represents a user-cancelled prompt.
pub fn is_cancelled(error: &anyhow::Error) -> bool {
    error.is::<PromptCancelled>()
}

/// Creates the standard error used to report prompt cancellation.
fn cancelled<T>() -> Result<T> {
    Err(PromptCancelled.into())
}
/// Prompts for a required single-line string.
///
/// `default` initializes the input. Escape returns the prompt-cancellation error.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn prompt_line(
    terminal: &mut AppTerminal,
    label: &str,
    default: Option<&str>,
) -> Result<String> {
    let mut input = default.unwrap_or("").chars().collect::<Vec<_>>();
    let mut cursor = input.len();

    loop {
        terminal.draw(|frame| draw_line_prompt(frame, label, &input, cursor))?;
        match read_event()? {
            Event::Key(key) => match key.code {
                KeyCode::Esc => return cancelled(),
                KeyCode::Enter => return Ok(input.iter().collect()),
                KeyCode::Left => cursor = cursor.saturating_sub(1),
                KeyCode::Right => cursor = (cursor + 1).min(input.len()),
                KeyCode::Home => cursor = 0,
                KeyCode::End => cursor = input.len(),
                KeyCode::Backspace if cursor > 0 => {
                    cursor -= 1;
                    input.remove(cursor);
                }
                KeyCode::Delete if cursor < input.len() => {
                    input.remove(cursor);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.drain(..cursor);
                    cursor = 0;
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    while cursor > 0 && input[cursor - 1].is_whitespace() {
                        cursor -= 1;
                        input.remove(cursor);
                    }
                    while cursor > 0 && !input[cursor - 1].is_whitespace() {
                        cursor -= 1;
                        input.remove(cursor);
                    }
                }
                KeyCode::Char(character)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    input.insert(cursor, character);
                    cursor += 1;
                }
                _ => {}
            },
            Event::Paste(text) => {
                for character in text.replace('\r', " ").replace('\n', " ").chars() {
                    input.insert(cursor, character);
                    cursor += 1;
                }
            }
            _ => {}
        }
    }
}
/// Prompts for an optional single-line string.
///
/// An accepted empty input returns `None`; Escape returns the prompt-cancellation error.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn prompt_optional_line(
    terminal: &mut AppTerminal,
    label: &str,
    default: Option<&str>,
) -> Result<Option<String>> {
    let value = prompt_line(terminal, &format!("{label} (vide = aucun)"), default)?;
    Ok((!value.trim().is_empty()).then_some(value))
}
/// Displays a yes/no modal and returns the selected value.
///
/// `default` controls the initially highlighted answer.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn confirm(terminal: &mut AppTerminal, question: &str, default: bool) -> Result<bool> {
    let options = vec![
        ("oui".to_owned(), "Oui".to_owned()),
        ("non".to_owned(), "Non".to_owned()),
    ];
    let initial = if default { 0 } else { 1 };
    let choice = select_modal(terminal, question, &options, initial)?;
    Ok(choice == 0)
}
/// Displays a modal list and returns the selected zero-based index.
///
/// An empty list returns `None`. The `_allow_cancel` parameter is currently retained for caller
/// compatibility; Escape is reported as prompt cancellation.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn select(
    terminal: &mut AppTerminal,
    title: &str,
    options: &[(String, String)],
    _allow_cancel: bool,
) -> Result<Option<usize>> {
    select_with_initial(terminal, title, options, 0, _allow_cancel)
}
/// Displays a modal list and returns the selected zero-based index.
///
/// `initial` is clamped to the valid option range. An empty list returns `None`. The `_allow_cancel`
/// parameter is currently retained for caller compatibility.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn select_with_initial(
    terminal: &mut AppTerminal,
    title: &str,
    options: &[(String, String)],
    initial: usize,
    _allow_cancel: bool,
) -> Result<Option<usize>> {
    if options.is_empty() {
        return Ok(None);
    }
    Ok(Some(select_modal(terminal, title, options, initial)?))
}
/// Displays a list of strings and returns the selected zero-based index.
///
/// An empty list returns `None`; `allow_cancel` is forwarded to the generic selector.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn select_strings(
    terminal: &mut AppTerminal,
    title: &str,
    options: &[String],
    allow_cancel: bool,
) -> Result<Option<usize>> {
    let values = options
        .iter()
        .map(|value| (value.clone(), value.clone()))
        .collect::<Vec<_>>();
    select(terminal, title, &values, allow_cancel)
}
/// Represents either a selected list index or a formula entered instead of a static choice.
pub enum IndexOrFormula {
    Index(usize),
    Formula(String),
}
/// Lets the user select a static string or enter a formula.
///
/// The synthetic final menu entry opens a line prompt for formula text. The return value distinguishes
/// that formula from a selected zero-based index.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn select_strings_or_formula (
    terminal: &mut AppTerminal,
    title: &str,
    options: &[String],
    allow_cancel: bool,
) -> Result<Option<IndexOrFormula>> {
    let mut values = options
        .iter()
        .map(|value| (value.clone(), value.clone()))
        .collect::<Vec<_>>();
    values.push(("#formule".to_owned(), "Entrer une formule".to_owned()));
    let sel = select(terminal, title, &values, allow_cancel)?;
    match sel {
        None => Ok(None),
        Some(index) => {
            if index == options.len() {
                Ok(Some(IndexOrFormula::Formula(prompt_line(terminal, "Entrez la formule", None)?)))
            } else {
                Ok(Some(IndexOrFormula::Index(index)))
            }
        }
    }

}
/// Prompts for a JSON5-compatible value on one line.
///
/// Empty input produces `null`. If parsing fails, the unparsed input is deliberately returned as a JSON
/// string so ordinary text values do not require quotes.
///
/// # Errors
///
/// Propagates terminal event errors or cancellation.
pub fn prompt_json_value(
    terminal: &mut AppTerminal,
    label: &str,
    default: Option<&Value>,
) -> Result<Value> {
    let default_text = default.map(json_compact);
    let input = prompt_line(terminal, label, default_text.as_deref())?;
    if input.trim().is_empty() {
        return Ok(Value::Null);
    }
    match serde_json5::from_str::<Value>(&input) {
        Ok(value) => Ok(value),
        Err(_) => Ok(Value::String(input)),
    }
}
/// Prompts for a list of strings, initialized from `current`.
///
/// # Errors
///
/// Propagates terminal event errors, cancellation, or invalid list input.
pub fn prompt_string_list(
    terminal: &mut AppTerminal,
    label: &str,
    current: &[String],
) -> Result<Vec<String>> {
    let current = current.join(", ");
    let value = prompt_line(
        terminal,
        &format!("{label} (séparés par des virgules)"),
        Some(&current),
    )?;
    Ok(value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect())
}
/// Opens the multiline editor initialized with `current` and returns the accepted text.
///
/// The input string is not mutated.
///
/// # Errors
///
/// Propagates terminal/event errors or cancellation from the editor.
pub fn edit_multiline(terminal: &mut AppTerminal, title: &str, current: &str) -> Result<String> {
    edit_multiline_validated(terminal, title, current, |_| Ok(()))
}
/// Opens a multiline JSON5 editor and returns the parsed value accepted by the user.
///
/// # Errors
///
/// Returns an error on terminal failure, cancellation, or invalid JSON5.
pub fn edit_json(terminal: &mut AppTerminal, title: &str, value: &Value) -> Result<Option<Value>> {
    let original = serde_json::to_string_pretty(value)?;
    let edited = edit_multiline_validated(terminal, title, &original, |text| {
        serde_json5::from_str::<Value>(text)
            .map(|_| ())
            .map_err(|error| format!("JSON ou JSON5 invalide: {error}"))
    })?;
    Ok(Some(serde_json5::from_str(&edited)?))
}
/// Displays an informational modal until the user dismisses it.
///
/// # Errors
///
/// Propagates terminal event errors.
pub fn pause(terminal: &mut AppTerminal, message: &str) -> Result<()> {
    loop {
        terminal.draw(|frame| draw_message(frame, "Information", message))?;
        match read_event()? {
            Event::Key(key)
                if matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) =>
            {
                return Ok(())
            }
            _ => {}
        }
    }
}

/// Serializes a JSON value into a compact one-line representation.
pub fn json_compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}
/// Runs a modal list selector with optional cancellation and initial selection.
///
/// `options` and `labels` must describe the same number of entries. The returned value is cloned from the selected option.
///
/// # Errors
///
/// Propagates terminal event errors and reports cancellation when disallowed by the caller contract.
fn select_modal(
    terminal: &mut AppTerminal,
    title: &str,
    options: &[(String, String)],
    initial: usize,
) -> Result<usize> {
    if options.is_empty() {
        return cancelled();
    }
    let mut selected = initial.min(options.len() - 1);

    loop {
        terminal.draw(|frame| draw_select(frame, title, options, selected))?;
        match read_event()? {
            Event::Key(key) => match key.code {
                KeyCode::Esc => return cancelled(),
                KeyCode::Enter => return Ok(selected),
                KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    selected = (selected + 1).min(options.len() - 1)
                }
                KeyCode::Home => selected = 0,
                KeyCode::End => selected = options.len() - 1,
                KeyCode::PageUp => selected = selected.saturating_sub(8),
                KeyCode::PageDown => selected = (selected + 8).min(options.len() - 1),
                KeyCode::Char(character) if character.is_ascii_digit() => {
                    let digit = character.to_digit(10).unwrap_or(0) as usize;
                    if digit > 0 && digit <= options.len() {
                        selected = digit - 1;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}
/// Runs the multiline editor until `validate` accepts the text or the user cancels.
///
/// Validation errors are displayed inside the editor rather than returned immediately.
///
/// # Errors
///
/// Propagates terminal/event errors or cancellation.
fn edit_multiline_validated<F>(
    terminal: &mut AppTerminal,
    title: &str,
    current: &str,
    validator: F,
) -> Result<String>
where
    F: Fn(&str) -> std::result::Result<(), String>,
{
    let mut buffer = TextBuffer::new(current);
    let mut error = None::<String>;

    loop {
        terminal.draw(|frame| draw_multiline(frame, title, &mut buffer, error.as_deref()))?;
        match read_event()? {
            Event::Key(key) => {
                if key.code == KeyCode::Esc {
                    return cancelled();
                }
                if (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::CONTROL))
                    || key.code == KeyCode::F(2)
                    || (key.code == KeyCode::Char('s')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    let text = buffer.to_string();
                    match validator(&text) {
                        Ok(()) => return Ok(text),
                        Err(message) => error = Some(message),
                    }
                    continue;
                }
                error = None;
                buffer.handle_key(key);
            }
            Event::Paste(text) => {
                error = None;
                buffer.insert_text(&text);
            }
            _ => {}
        }
    }
}

/// Reads the next terminal event accepted by modal prompts.
fn read_event() -> Result<Event> {
    loop {
        let event = event::read()?;
        match &event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                return Ok(event)
            }
            Event::Paste(_) | Event::Resize(_, _) => return Ok(event),
            _ => {}
        }
    }
}

/// Restores the captured Forge frame behind a modal, or draws a neutral fallback backdrop.
fn draw_modal_backdrop(frame: &mut Frame<'_>) {
    let area = frame.area();
    let restored = MODAL_BACKDROP.with(|backdrop| {
        let backdrop = backdrop.borrow();
        let Some(snapshot) = backdrop.as_ref() else {
            return false;
        };
        if snapshot.area != area {
            return false;
        }
        frame.buffer_mut().merge(snapshot);
        true
    });
    if !restored {
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(" Forge du récit — fenêtre modale ")
                .style(Style::default().fg(Color::DarkGray).bg(Color::Black)),
            area,
        );
    }
}

/// Renders a centered selectable list and keyboard hint for the modal selector.
fn draw_select(frame: &mut Frame<'_>, title: &str, options: &[(String, String)], selected: usize) {
    draw_modal_backdrop(frame);
    let longest = options
        .iter()
        .map(|(id, label)| id.chars().count() + label.chars().count() + 5)
        .max()
        .unwrap_or(24)
        .max(title.chars().count() + 4);
    let max_width = frame.area().width.saturating_sub(4).max(1);
    let max_height = frame.area().height.saturating_sub(4).max(1);
    let width = (longest as u16 + 4).max(38).min(max_width);
    let height = (options.len() as u16 + 4).max(7).min(max_height);
    let area = centered_rect(width, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let items = options
        .iter()
        .enumerate()
        .map(|(index, (id, label))| {
            let prefix = if index < 9 {
                format!("{}. ", index + 1)
            } else {
                "   ".to_owned()
            };
            let text = if id == label {
                format!("{prefix}{label}")
            } else {
                format!("{prefix}{label}  ({id})")
            };
            ListItem::new(text)
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(selected));
    frame.render_stateful_widget(
        List::new(items).highlight_symbol("▶ ").highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        chunks[0],
        &mut state,
    );
    frame.render_widget(
        Paragraph::new("↑↓ choisir · Entrée valider · Échap annuler")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );
}

/// Renders the single-line editor, horizontal viewport, and cursor.
fn draw_line_prompt(frame: &mut Frame<'_>, label: &str, input: &[char], cursor: usize) {
    draw_modal_backdrop(frame);
    let width = frame.area().width.saturating_sub(8).clamp(36, 90);
    let area = centered_rect(
        width,
        7.min(frame.area().height.saturating_sub(2)),
        frame.area(),
    );
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {label} "))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let input_area = chunks[0];

    let content_width = input_area.width.saturating_sub(1) as usize;
    let scroll = cursor.saturating_sub(content_width.saturating_sub(1));
    let visible = input
        .iter()
        .skip(scroll)
        .take(content_width)
        .collect::<String>();
    frame.render_widget(
        Paragraph::new(visible).block(Block::default().borders(Borders::BOTTOM)),
        input_area,
    );
    frame.render_widget(
        Paragraph::new("Entrée valider · Échap annuler")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray)),
        chunks[1],
    );
    let cursor_x = input_area
        .x
        .saturating_add((cursor.saturating_sub(scroll)) as u16)
        .min(input_area.right().saturating_sub(1));
    frame.set_cursor_position((cursor_x, input_area.y));
}

/// Renders the multiline editor, cursor, scroll position, and optional validation error.
fn draw_multiline(
    frame: &mut Frame<'_>,
    title: &str,
    buffer: &mut TextBuffer,
    error: Option<&str>,
) {
    draw_modal_backdrop(frame);
    let width = frame.area().width.saturating_sub(4).max(1);
    let height = frame.area().height.saturating_sub(4).max(1);
    let area = centered_rect(width, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if error.is_some() { 2 } else { 1 }),
        ])
        .split(inner);
    let editor = chunks[0];
    buffer.ensure_visible(editor.width as usize, editor.height as usize);
    frame.render_widget(
        Paragraph::new(buffer.to_string()).scroll((buffer.scroll_y as u16, buffer.scroll_x as u16)),
        editor,
    );

    let status = if let Some(error) = error {
        Line::from(vec![
            Span::styled(
                "Erreur: ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(error, Style::default().fg(Color::Red)),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                format!("Ln {}, Col {}", buffer.row + 1, buffer.col + 1),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(" · Entrée nouvelle ligne · Ctrl+Entrée/F2/Ctrl+S valider · Échap annuler"),
        ])
    };
    frame.render_widget(Paragraph::new(status), chunks[1]);

    let cursor_x = editor
        .x
        .saturating_add(buffer.col.saturating_sub(buffer.scroll_x) as u16)
        .min(editor.right().saturating_sub(1));
    let cursor_y = editor
        .y
        .saturating_add(buffer.row.saturating_sub(buffer.scroll_y) as u16)
        .min(editor.bottom().saturating_sub(1));
    frame.set_cursor_position((cursor_x, cursor_y));
}

/// Renders a centered read-only informational message modal.
fn draw_message(frame: &mut Frame<'_>, title: &str, message: &str) {
    draw_modal_backdrop(frame);
    let lines = message.lines().count() as u16;
    let width = frame.area().width.saturating_sub(8).clamp(40, 100);
    let height = (lines + 4)
        .max(7)
        .min(frame.area().height.saturating_sub(4).max(1));
    let area = centered_rect(width, height, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Text::from(message))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {title} ")),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
    let hint_area = Rect::new(
        area.x.saturating_add(2),
        area.bottom().saturating_sub(2),
        area.width.saturating_sub(4),
        1,
    );
    frame.render_widget(
        Paragraph::new("Entrée ou Échap fermer")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray)),
        hint_area,
    );
}

/// Computes a centered popup rectangle inside the supplied area.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width).max(1);
    let height = height.min(area.height).max(1);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// Maintains editable multiline text, cursor position, and viewport state.
#[derive(Debug, Clone)]
struct TextBuffer {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    scroll_y: usize,
    scroll_x: usize,
}

impl TextBuffer {
    /// Creates an editor buffer from `text`, splitting it into editable lines and placing the cursor at the start.
    fn new(text: &str) -> Self {
        let mut lines = text
            .split('\n')
            .map(|line| line.chars().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        Self {
            lines,
            row: 0,
            col: 0,
            scroll_y: 0,
            scroll_x: 0,
        }
    }

    /// Converts the value to string.
    fn to_string(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Handles one keyboard event for the current application state.
    ///
    /// Processes events synchronously and assumes the caller has already initialized the
    /// appropriate player or Forge terminal session.
    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Up => self.move_up(),
            KeyCode::Down => self.move_down(),
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.current_line().len(),
            KeyCode::PageUp => {
                self.row = self.row.saturating_sub(10);
                self.clamp_col();
            }
            KeyCode::PageDown => {
                self.row = (self.row + 10).min(self.lines.len() - 1);
                self.clamp_col();
            }
            KeyCode::Enter => self.newline(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Tab => self.insert_text("    "),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => self.col = 0,
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.col = self.current_line().len()
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.lines[self.row].drain(..self.col);
                self.col = 0;
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.lines[self.row].insert(self.col, character);
                self.col += 1;
            }
            _ => {}
        }
    }

    /// Inserts text at the cursor, preserving embedded line breaks.
    fn insert_text(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        for (index, part) in normalized.split('\n').enumerate() {
            if index > 0 {
                self.newline();
            }
            for character in part.chars() {
                self.lines[self.row].insert(self.col, character);
                self.col += 1;
            }
        }
    }

    /// Moves the text cursor one position to the left, crossing lines when needed.
    ///
    /// Handles empty collections and out-of-range indices without panicking.
    fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.current_line().len();
        }
    }

    /// Moves the text cursor one position to the right, crossing lines when needed.
    ///
    /// Handles empty collections and out-of-range indices without panicking.
    fn move_right(&mut self) {
        if self.col < self.current_line().len() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    /// Moves the text cursor to the previous line while preserving its column when possible.
    ///
    /// Handles empty collections and out-of-range indices without panicking.
    fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.clamp_col();
        }
    }

    /// Moves the text cursor to the next line while preserving its column when possible.
    ///
    /// Handles empty collections and out-of-range indices without panicking.
    fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.clamp_col();
        }
    }

    /// Splits the current line at the cursor and inserts a new line.
    ///
    /// Moves the text after the cursor into a newly inserted following line.
    fn newline(&mut self) {
        let tail = self.lines[self.row].split_off(self.col);
        self.row += 1;
        self.lines.insert(self.row, tail);
        self.col = 0;
    }

    /// Deletes the character before the cursor or joins with the previous line.
    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
            self.lines[self.row].remove(self.col);
        } else if self.row > 0 {
            let current = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].extend(current);
        }
    }

    /// Deletes the character under the cursor or joins with the next line.
    fn delete(&mut self) {
        if self.col < self.current_line().len() {
            self.lines[self.row].remove(self.col);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].extend(next);
        }
    }

    /// Returns the character buffer for the current line.
    fn current_line(&self) -> &[char] {
        &self.lines[self.row]
    }

    /// Clamps the cursor column to the length of the current line.
    ///
    /// Handles empty collections and out-of-range indices without panicking.
    fn clamp_col(&mut self) {
        self.col = self.col.min(self.current_line().len());
    }

    /// Adjusts the scroll offsets so the cursor remains visible.
    fn ensure_visible(&mut self, width: usize, height: usize) {
        let width = width.max(1);
        let height = height.max(1);
        if self.row < self.scroll_y {
            self.scroll_y = self.row;
        } else if self.row >= self.scroll_y + height {
            self.scroll_y = self.row + 1 - height;
        }
        if self.col < self.scroll_x {
            self.scroll_x = self.col;
        } else if self.col >= self.scroll_x + width {
            self.scroll_x = self.col + 1 - width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextBuffer;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // Verifies that multiline insert and join.
    #[test]
    fn multiline_insert_and_join() {
        let mut buffer = TextBuffer::new("abc\ndef");
        buffer.row = 0;
        buffer.col = 3;
        buffer.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        buffer.insert_text("XYZ");
        assert_eq!(buffer.to_string(), "abc\nXYZ\ndef");
    }

    // Verifies that backspace joins lines.
    #[test]
    fn backspace_joins_lines() {
        let mut buffer = TextBuffer::new("abc\ndef");
        buffer.row = 1;
        buffer.col = 0;
        buffer.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(buffer.to_string(), "abcdef");
        assert_eq!((buffer.row, buffer.col), (0, 3));
    }

    // Verifies that paste preserves newlines.
    #[test]
    fn paste_preserves_newlines() {
        let mut buffer = TextBuffer::new("");
        buffer.insert_text("a\r\nb\rc");
        assert_eq!(buffer.to_string(), "a\nb\nc");
    }
}
