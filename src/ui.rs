//! Pure Ratatui rendering functions for Forge views and popup layout.
//!
//! This module reads [`App`] but does not mutate the tale. Renderers tolerate missing raw
//! fields with placeholders, while selection indices are expected to have been clamped by
//! `App::after_edit`.
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
    Frame,
};
use serde_json::Value;

use crate::{
    app::{App, View},
    graph, schema,
    validation::{problem_counts, Severity},
};

/// Renders the complete Forge interface for the current application state.
///
///
/// # Arguments
///
/// - `frame` — Current Ratatui frame.
/// - `app` — Read-only or mutable Forge application state.
pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);
    draw_header(frame, app, vertical[0]);
    match app.view {
        View::Tale => draw_tale(frame, app, vertical[1]),
        View::Pages => draw_pages(frame, app, vertical[1]),
        View::Objects => draw_entities(frame, app, vertical[1], "objets"),
        View::States => draw_entities(frame, app, vertical[1], "etats"),
        View::Graph => draw_graph(frame, app, vertical[1]),
        View::Problems => draw_problems(frame, app, vertical[1]),
    }
    draw_footer(frame, app, vertical[2]);
}

/// Renders the view tabs, tale title, and dirty-state marker in the header area.
fn draw_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let titles = View::ALL
        .iter()
        .map(|view| Line::from(Span::raw(format!(" {} ", view.title()))))
        .collect::<Vec<_>>();
    let selected = View::ALL
        .iter()
        .position(|view| *view == app.view)
        .unwrap_or(0);
    let dirty = if app.document.dirty { " ●" } else { "" };
    let title = format!(" Forge du récit — {}{} ", app.document.title(), dirty);
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(title))
        .select(selected)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider("│");
    frame.render_widget(tabs, area);
}

/// Renders status text and validation-problem counts in the footer area.
fn draw_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let (errors, warnings, info) = problem_counts(&app.problems);
    let left = format!(
        " {}{}",
        app.status,
        if app.document.dirty {
            " [modifié]"
        } else {
            ""
        }
    );
    let right = format!("✗ {errors}  ! {warnings}  · {info} ");
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(right.len() as u16 + 1),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(left).style(Style::default().fg(Color::DarkGray)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(if errors > 0 {
                Color::Red
            } else if warnings > 0 {
                Color::Yellow
            } else {
                Color::Green
            })),
        chunks[1],
    );
}

/// Renders root tale metadata and the main editing shortcuts.
///
/// Missing or malformed optional raw fields are displayed with neutral placeholders.
fn draw_tale(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let root = app.document.root.as_object();
    let title = root
        .and_then(|object| object.get("titre"))
        .and_then(Value::as_str)
        .unwrap_or("Récit sans titre");
    let version = root
        .and_then(|object| object.get("version"))
        .and_then(Value::as_u64)
        .map_or_else(|| "—".to_owned(), |version| version.to_string());
    let author = root
        .and_then(|object| object.get("auteur"))
        .and_then(Value::as_str)
        .unwrap_or("—");
    let synopsis = root
        .and_then(|object| object.get("synopsis"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let start = root
        .and_then(|object| object.get("depart"))
        .and_then(Value::as_str)
        .unwrap_or("—");
    let attributes = app.document.attribute_names().join(", ");
    let pre_cells = root
        .and_then(|object| object.get("avant"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let post_cells = root
        .and_then(|object| object.get("apres"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let pages = app.document.page_ids().len();
    let objects = app.document.entity_ids("objets").len();
    let states = app.document.entity_ids("etats").len();
    let globals = app.document.entity_ids("global").len();
    let images = app.document.entity_ids("images").len();
    let title_images = match root.and_then(|object| object.get("image")) {
        Some(Value::String(_)) => 1,
        Some(Value::Array(images)) => images.len(),
        _ => 0,
    };
    let known_tags = app
        .document
        .known_tags()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");

    let text = Text::from(vec![
        Line::from(vec![bold("Titre: "), Span::raw(title)]),
        Line::from(vec![bold("Syntaxe: "), Span::raw(format!("v{version}"))]),
        Line::from(vec![bold("Auteur: "), Span::raw(author)]),
        Line::from(vec![bold("Départ: "), Span::raw(start)]),
        Line::from(vec![bold("Attributs: "), Span::raw(attributes)]),
        Line::from(vec![
            bold("Contenu: "),
            Span::raw(format!(
                "{pages} pages · {objects} objets · {states} états · {images} images · {globals} valeurs globales"
            )),
        ]),
        Line::from(vec![
            bold("Cellules avant: "),
            Span::raw(pre_cells.to_string()),
        ]),
        Line::from(vec![
            bold("Images de couverture: "),
            Span::raw(title_images.to_string()),
        ]),
        Line::from(vec![
            bold("Cellules après: "),
            Span::raw(post_cells.to_string()),
        ]),
        Line::from(vec![
            bold("Tags repérés: "),
            Span::raw(if known_tags.is_empty() {
                "—".to_owned()
            } else {
                known_tags.clone()
            }),
        ]),
        Line::raw(""),
        Line::from(bold("Synopsis")),
        Line::raw(synopsis),
        Line::raw(""),
        Line::styled(
            "Entrée/e: modifier · j: JSON complet · Ctrl+S: enregistrer",
            Style::default().fg(Color::DarkGray),
        ),
        Line::styled(
            "Ordre v2: connexion → avant → page → après",
            Style::default().fg(Color::Cyan),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Récit "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Renders the page list and a summary of the selected raw page.
fn draw_pages(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
        .split(area);
    let ids = app.document.page_ids();
    let items = ids
        .iter()
        .map(|id| {
            let title = app
                .document
                .page(id)
                .and_then(|page| page.get("titre"))
                .and_then(Value::as_str)
                .unwrap_or("Page sans titre");
            let start =
                app.document.root.get("depart").and_then(Value::as_str) == Some(id.as_str());
            ListItem::new(format!(
                "{}{} — {}",
                if start { "★ " } else { "" },
                id,
                title
            ))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.page_index));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Pages "))
            .highlight_symbol("▶ ")
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        chunks[0],
        &mut state,
    );

    let detail = ids
        .get(app.page_index)
        .and_then(|id| app.document.page(id).map(|page| page_detail(id, page)))
        .unwrap_or_else(|| Text::from("Aucune page."));
    frame.render_widget(
        Paragraph::new(detail)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Page sélectionnée "),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

/// Builds the detailed text shown for the selected page.
fn page_detail<'a>(id: &'a str, page: &'a Value) -> Text<'a> {
    let title = page
        .get("titre")
        .and_then(Value::as_str)
        .unwrap_or("Page sans titre");
    let tags = page
        .get("tags")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let cells = page
        .get("cellules")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let connections = page
        .get("connexions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut lines = vec![
        Line::from(vec![bold("ID: "), Span::raw(id)]),
        Line::from(vec![bold("Titre: "), Span::raw(title)]),
        Line::from(vec![bold("Tags immuables: "), Span::raw(tags.to_string())]),
        Line::raw(""),
        Line::from(bold(format!("Cellules ({})", cells.len()))),
    ];
    for (index, cell) in cells.iter().take(8).enumerate() {
        lines.push(Line::raw(format!(
            "  • {}",
            schema::cell_summary(cell, index)
        )));
    }
    if cells.len() > 8 {
        lines.push(Line::raw("  …"));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(bold(format!(
        "Connexions ({})",
        connections.len()
    ))));
    for connection in connections.iter().take(8) {
        let destination = connection
            .get("destination")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let label = connection
            .get("texte")
            .and_then(Value::as_str)
            .unwrap_or("Connexion sans texte")
            .replace('\n', " ");
        let gated = if connection.get("conditions").is_some() {
            " ◇"
        } else {
            ""
        };
        let check = if connection.get("test").is_some() {
            " ⚄"
        } else {
            ""
        };
        lines.push(Line::raw(format!(
            "  → {destination}{gated}{check}: {label}"
        )));
    }
    if connections.len() > 8 {
        lines.push(Line::raw("  …"));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Entrée/e: modifier · a: ajouter · c: copier · r: renommer · d: supprimer",
        Style::default().fg(Color::DarkGray),
    ));
    Text::from(lines)
}

/// Renders one top-level entity collection and the selected entity's raw summary.
fn draw_entities(frame: &mut Frame<'_>, app: &App, area: Rect, key: &str) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
        .split(area);
    let ids = app.document.entity_ids(key);
    let selected = if key == "objets" {
        app.object_index
    } else {
        app.state_index
    };
    let title = if key == "objets" {
        " Objets "
    } else {
        " États "
    };
    let items = ids
        .iter()
        .map(|id| {
            let name = app
                .document
                .entity(key, id)
                .and_then(|value| value.get("nom"))
                .and_then(Value::as_str)
                .unwrap_or("Sans nom");
            ListItem::new(format!("{id} — {name}"))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(selected));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_symbol("▶ ")
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        chunks[0],
        &mut state,
    );
    let detail = ids
        .get(selected)
        .and_then(|id| {
            app.document
                .entity(key, id)
                .map(|value| entity_detail(id, value, key))
        })
        .unwrap_or_else(|| Text::from("Aucun élément."));
    frame.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::ALL).title(" Définition "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

/// Builds the detailed text shown for the selected object or state.
fn entity_detail<'a>(id: &'a str, value: &'a Value, key: &str) -> Text<'a> {
    let name = value
        .get("nom")
        .and_then(Value::as_str)
        .unwrap_or("Sans nom");
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let traits = string_list(value.get("traits"));
    let data = value
        .get("donnees")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let mut lines = vec![
        Line::from(vec![bold("ID: "), Span::raw(id)]),
        Line::from(vec![bold("Nom: "), Span::raw(name)]),
        Line::from(vec![
            bold("Traits: "),
            Span::raw(if traits.is_empty() {
                "—".to_owned()
            } else {
                traits.clone()
            }),
        ]),
        Line::raw(""),
        Line::from(bold("Description")),
        Line::raw(description),
        Line::raw(""),
        Line::from(bold("Données")),
        Line::raw(serde_json::to_string_pretty(&data).unwrap_or_else(|_| "{}".into())),
    ];
    if key == "etats" {
        let effects = value
            .get("effets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        lines.push(Line::raw(""));
        lines.push(Line::from(bold(format!(
            "Modificateurs ({})",
            effects.len()
        ))));
        for effect in effects {
            lines.push(Line::raw(format!(
                "  • {}",
                effect.get("type").and_then(Value::as_str).unwrap_or("?")
            )));
        }
        lines.push(Line::from(vec![
            bold("Expiration: "),
            Span::raw(
                value
                    .get("expiration")
                    .filter(|value| !value.is_null())
                    .map(|value| schema::filter_brief(Some(value)))
                    .unwrap_or_else(|| "—".into()),
            ),
        ]));
        lines.push(Line::from(vec![
            bold("Défaite: "),
            Span::raw(value.get("defaite").and_then(Value::as_str).unwrap_or("—")),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Entrée/e: modifier · a: ajouter · c: copier · r: renommer · d: supprimer",
        Style::default().fg(Color::DarkGray),
    ));
    Text::from(lines)
}

/// Renders the conservative page graph and selected-node transition details.
fn draw_graph(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    let (map, detail) = graph::overview_lines(
        &app.document.root,
        app.graph_index,
        chunks[0].width.saturating_sub(4) as usize,
    );
    frame.render_widget(
        Paragraph::new(map.join("\n"))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Vue par niveaux "),
            )
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
    let graph_data = graph::analyze(&app.document.root);
    let items = graph_data
        .nodes
        .iter()
        .map(|node| {
            let depth = node
                .depth
                .map(|value| value.to_string())
                .unwrap_or_else(|| "∞".into());
            ListItem::new(format!("[{depth}] {}  {}→", node.id, node.outgoing))
        })
        .collect::<Vec<_>>();
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(chunks[1]);
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.graph_index));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Nœuds "))
            .highlight_symbol("▶ ")
            .highlight_style(Style::default().fg(Color::Yellow)),
        right[0],
        &mut state,
    );
    let mut detail_lines = detail;
    detail_lines.push(String::new());
    detail_lines.push("Entrée: ouvrir la page dans l'éditeur".into());
    frame.render_widget(
        Paragraph::new(detail_lines.join("\n"))
            .block(Block::default().borders(Borders::ALL).title(" Voisinage "))
            .wrap(Wrap { trim: false }),
        right[1],
    );
}

/// Renders validation problems and the full message for the selected problem.
fn draw_problems(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    let items = app
        .problems
        .iter()
        .map(|problem| {
            let style = severity_style(problem.severity);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", problem.severity.symbol()), style),
                Span::raw(format!("{} — {}", problem.path, problem.message)),
            ]))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.problem_index));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" Validation "))
            .highlight_symbol("▶ ")
            .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
        chunks[0],
        &mut state,
    );
    let detail = app
        .problems
        .get(app.problem_index)
        .map(|problem| {
            Text::from(vec![
                Line::styled(problem.severity.symbol(), severity_style(problem.severity)),
                Line::from(vec![bold("Chemin: "), Span::raw(&problem.path)]),
                Line::raw(""),
                Line::raw(&problem.message),
                Line::raw(""),
                Line::styled(
                    "Les erreurs sont des structures que le moteur ne devrait pas accepter.\nLes avertissements signalent des références ou branches suspectes.\nLes informations décrivent notamment les pages terminales.",
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        })
        .unwrap_or_else(|| Text::from("Aucun problème détecté."));
    frame.render_widget(
        Paragraph::new(detail)
            .block(Block::default().borders(Borders::ALL).title(" Détail "))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

/// Creates a bold text span for the Forge interface.
fn bold<'a>(text: impl Into<std::borrow::Cow<'a, str>>) -> Span<'a> {
    Span::styled(text, Style::default().add_modifier(Modifier::BOLD))
}

/// Returns the visual style associated with a validation severity.
fn severity_style(severity: Severity) -> Style {
    Style::default().fg(match severity {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Info => Color::Cyan,
    })
}

/// Parses an optional string-or-array field into a string list.
fn string_list(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

/// Computes a centered popup rectangle inside the supplied area.
#[allow(dead_code)]
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Clears a popup region before rendering a modal widget.
#[allow(dead_code)]
fn clear_popup(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);
}
