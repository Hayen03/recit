//! Builds the conservative page-transition graph used by Forge and DOT export.
//!
//! The analyzer works from raw JSON so it can inspect incomplete Forge documents. It includes
//! literal connections, formula connections, check-failure destinations, redirect effects,
//! and transitions attached to global or persistent-state cells. Because formula
//! destinations cannot be resolved statically, they are treated as potentially reaching every
//! page; this intentionally favors false reachability over false “unreachable” warnings.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde_json::Value;

use formulas::has_formula_substitution;

const FORMULA_CONNECTION_TARGET: &str = "${connexion_formule}";
const GLOBAL_PRE_SOURCE: &str = "@global:avant";
const GLOBAL_POST_SOURCE: &str = "@global:apres";

/// Stores graph metrics and display information for one tale page.
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub depth: Option<usize>,
    pub incoming: usize,
    pub outgoing: usize,
}

/// Contains analyzed page nodes, local edges, ambient transitions, and breadth-first layers.
#[derive(Debug, Clone)]
pub struct GraphData {
    /// Tale pages, sorted by reachability depth and then identifier.
    pub nodes: Vec<GraphNode>,
    /// All graph edges, including ambient transitions from pseudo-sources.
    pub edges: Vec<(String, String, String)>,
    /// Transitions produced by root or persistent-state data rather than one specific page.
    pub ambient_edges: Vec<(String, String, String)>,
    /// Whether an ambient transition has a formula destination and can therefore reach any page.
    pub ambient_dynamic: bool,
    /// Page identifiers grouped by conservative breadth-first depth; `None` means unreachable.
    pub layers: BTreeMap<Option<usize>, Vec<String>>,
}

/// Analyzes the tale's conservative page graph.
///
/// Static local transitions create ordinary edges. A local formula destination or dynamic
/// connection generator marks its source page as capable of reaching any page. Static ambient
/// redirects become possible from every reachable page, while an ambient formula redirect makes
/// every page conservatively reachable once the starting page itself is reachable.
pub fn analyze(root: &Value) -> GraphData {
    let pages = root
        .get("pages")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let start = root
        .get("depart")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let mut local_edges = Vec::new();
    for (source, page) in &pages {
        collect_connection_edges(
            page.get("connexions"),
            source,
            &mut local_edges,
        );

        // Compatibility with tales that have not yet moved their generators into
        // the unified `connexions` field.
        if has_dynamic_connections(page.get("connexions_dyn")) {
            local_edges.push((
                source.clone(),
                FORMULA_CONNECTION_TARGET.to_owned(),
                "[ancienne connexion dynamique]".into(),
            ));
        }

        collect_cell_transition_edges(
            page.get("cellules"),
            source,
            "[redirection]",
            &mut local_edges,
        );
    }

    let mut ambient_edges = Vec::new();
    collect_cell_transition_edges(
        root.get("avant"),
        GLOBAL_PRE_SOURCE,
        "[redirection globale avant]",
        &mut ambient_edges,
    );
    collect_cell_transition_edges(
        root.get("apres"),
        GLOBAL_POST_SOURCE,
        "[redirection globale après]",
        &mut ambient_edges,
    );
    if let Some(states) = root.get("etats").and_then(Value::as_object) {
        for (state_id, state) in states {
            if let Some(defeat) = state.get("defaite").and_then(Value::as_str) {
                ambient_edges.push((
                    format!("@etat:{state_id}:defaite"),
                    defeat.to_owned(),
                    "[défaite d'état]".into(),
                ));
            }
            collect_cell_transition_edges(
                state.get("avant"),
                &format!("@etat:{state_id}:avant"),
                "[redirection d'état avant]",
                &mut ambient_edges,
            );
            collect_cell_transition_edges(
                state.get("apres"),
                &format!("@etat:{state_id}:apres"),
                "[redirection d'état après]",
                &mut ambient_edges,
            );
        }
    }

    let ambient_dynamic = ambient_edges
        .iter()
        .any(|(_, target, _)| has_formula_substitution(target));
    let depths = bfs_depths(
        start.as_deref(),
        &pages,
        &local_edges,
        &ambient_edges,
    );

    let mut outgoing: HashMap<String, usize> = HashMap::new();
    let mut incoming: HashMap<String, usize> = HashMap::new();
    for (source, target, _) in &local_edges {
        *outgoing.entry(source.clone()).or_default() += 1;
        if !has_formula_substitution(target) && pages.contains_key(target) {
            *incoming.entry(target.clone()).or_default() += 1;
        }
    }

    let mut layers: BTreeMap<Option<usize>, Vec<String>> = BTreeMap::new();
    let mut nodes = Vec::new();
    for (id, page) in &pages {
        let depth = depths.get(id).copied();
        layers.entry(depth).or_default().push(id.clone());
        nodes.push(GraphNode {
            id: id.clone(),
            title: page
                .get("titre")
                .and_then(Value::as_str)
                .unwrap_or("Page sans titre")
                .to_owned(),
            depth,
            incoming: incoming.get(id).copied().unwrap_or(0),
            outgoing: outgoing.get(id).copied().unwrap_or(0),
        });
    }
    for layer in layers.values_mut() {
        layer.sort();
    }
    nodes.sort_by(|a, b| {
        a.depth
            .is_none()
            .cmp(&b.depth.is_none())
            .then_with(|| a.depth.cmp(&b.depth))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut edges = local_edges;
    edges.extend(ambient_edges.clone());
    GraphData {
        nodes,
        edges,
        ambient_edges,
        ambient_dynamic,
        layers,
    }
}

/// Adds statically discoverable and formula-generated connection edges to `out`.
///
/// A formula entry receives one conservative pseudo-target because its result is unknown
/// before execution. Arrays may freely mix formula strings and literal dictionaries.
fn collect_connection_edges(
    value: Option<&Value>,
    source: &str,
    out: &mut Vec<(String, String, String)>,
) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Null => {}
        Value::Array(values) => {
            for value in values {
                collect_connection_edges(Some(value), source, out);
            }
        }
        Value::String(_) => out.push((
            source.to_owned(),
            FORMULA_CONNECTION_TARGET.to_owned(),
            "[connexion par formule]".into(),
        )),
        Value::Object(connection) => {
            if let Some(destination) = connection.get("destination").and_then(Value::as_str) {
                let label = connection
                    .get("texte")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                out.push((source.to_owned(), destination.to_owned(), label));
            }
            if let Some(failure) = connection
                .get("test")
                .and_then(|test| test.get("echec"))
                .and_then(Value::as_str)
            {
                out.push((source.to_owned(), failure.to_owned(), "[échec]".into()));
            }
            collect_cell_transition_edges(
                connection.get("cellules"),
                source,
                "[redirection de connexion]",
                out,
            );
        }
        _ => {}
    }
}

/// Returns whether the legacy raw `connexions_dyn` field may produce a connection.
///
/// Empty arrays and `null` are treated as absent. Any other value is considered dynamic even if
/// malformed; validation is responsible for reporting its type or formula errors.
fn has_dynamic_connections(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Array(values)) => !values.is_empty(),
        Some(_) => true,
    }
}

/// Adds every redirect and effect-defeat destination found in a cell container to `out`.
///
/// Conditions are deliberately ignored: graph analysis asks whether a transition is possible,
/// not whether it is currently active. Both a single cell object and an array of cells are
/// accepted so incomplete Forge documents remain inspectable.
fn collect_cell_transition_edges(
    cells: Option<&Value>,
    source: &str,
    label: &str,
    out: &mut Vec<(String, String, String)>,
) {
    let Some(cells) = cells else {
        return;
    };
    let values: Vec<&Value> = match cells {
        Value::Array(values) => values.iter().collect(),
        Value::Object(_) => vec![cells],
        _ => return,
    };
    for cell in values {
        let Some(effects) = cell.get("effets").and_then(Value::as_array) else {
            continue;
        };
        for effect in effects {
            if effect.get("type").and_then(Value::as_str) == Some("rediriger") {
                if let Some(destination) = effect.get("destination").and_then(Value::as_str) {
                    out.push((
                        source.to_owned(),
                        destination.to_owned(),
                        label.to_owned(),
                    ));
                }
            }
            if let Some(defeat) = effect.get("defaite").and_then(Value::as_str) {
                out.push((
                    source.to_owned(),
                    defeat.to_owned(),
                    "[défaite d'effet]".into(),
                ));
            }
        }
    }
}

/// Computes conservative breadth-first depths from the tale's starting page.
fn bfs_depths(
    start: Option<&str>,
    pages: &serde_json::Map<String, Value>,
    local_edges: &[(String, String, String)],
    ambient_edges: &[(String, String, String)],
) -> HashMap<String, usize> {
    let Some(start) = start.filter(|start| pages.contains_key(*start)) else {
        return HashMap::new();
    };

    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    let mut dynamic_sources = HashSet::new();
    for (source, target, _) in local_edges {
        if target == FORMULA_CONNECTION_TARGET || has_formula_substitution(target) {
            dynamic_sources.insert(source.clone());
        } else if pages.contains_key(target) {
            adjacency
                .entry(source.clone())
                .or_default()
                .push(target.clone());
        }
    }

    let ambient_dynamic = ambient_edges
        .iter()
        .any(|(_, target, _)| has_formula_substitution(target));
    let ambient_targets = ambient_edges
        .iter()
        .filter_map(|(_, target, _)| {
            (!has_formula_substitution(target) && pages.contains_key(target))
                .then_some(target.clone())
        })
        .collect::<Vec<_>>();

    let mut depth = HashMap::from([(start.to_owned(), 0usize)]);
    let mut queue = VecDeque::from([start.to_owned()]);
    while let Some(source) = queue.pop_front() {
        let next_depth = depth.get(&source).copied().unwrap_or(0) + 1;

        if ambient_dynamic || dynamic_sources.contains(&source) {
            for target in pages.keys() {
                enqueue_depth(target, next_depth, &mut depth, &mut queue);
            }
        }
        for target in &ambient_targets {
            enqueue_depth(target, next_depth, &mut depth, &mut queue);
        }
        for target in adjacency.get(&source).into_iter().flatten() {
            enqueue_depth(target, next_depth, &mut depth, &mut queue);
        }
    }
    depth
}

/// Inserts a page into the breadth-first frontier only on its first discovery.
fn enqueue_depth(
    target: &str,
    depth_value: usize,
    depths: &mut HashMap<String, usize>,
    queue: &mut VecDeque<String>,
) {
    if !depths.contains_key(target) {
        depths.insert(target.to_owned(), depth_value);
        queue.push_back(target.to_owned());
    }
}

/// Formats the graph analysis as map and detail panes for Forge.
///
/// `selected` is an index into the analyzed node list, not a page identifier. `width` controls
/// wrapping in the map pane and is clamped to a usable minimum. An out-of-range selection falls
/// back to the first page.
pub fn overview_lines(root: &Value, selected: usize, width: usize) -> (Vec<String>, Vec<String>) {
    let graph = analyze(root);
    let mut map_lines = Vec::new();
    let ordered_layers = graph
        .layers
        .iter()
        .filter(|(depth, _)| depth.is_some())
        .chain(graph.layers.iter().filter(|(depth, _)| depth.is_none()));
    for (depth, ids) in ordered_layers {
        let label = match depth {
            Some(value) => format!("Niveau {value}"),
            None => "Inatteignables".into(),
        };
        map_lines.push(label);
        let mut line = String::from("  ");
        for id in ids {
            let node = format!("[{id}]");
            if line.len() + node.len() + 2 > width.max(20) {
                map_lines.push(line);
                line = String::from("  ");
            }
            if line.len() > 2 {
                line.push_str("  ");
            }
            line.push_str(&node);
        }
        if line.len() > 2 {
            map_lines.push(line);
        }
        map_lines.push(String::new());
    }

    if graph.ambient_dynamic {
        map_lines.push("Une redirection ambiante dynamique peut atteindre toute page.".into());
        map_lines.push(String::new());
    }

    let node = graph.nodes.get(selected).or_else(|| graph.nodes.first());
    let mut detail = Vec::new();
    if let Some(node) = node {
        detail.push(format!("{} — {}", node.id, node.title));
        detail.push(format!(
            "Entrées: {}   Sorties locales: {}   Niveau: {}",
            node.incoming,
            node.outgoing,
            node.depth
                .map(|depth| depth.to_string())
                .unwrap_or_else(|| "inatteignable".into())
        ));
        detail.push(String::new());
        detail.push("Entrantes locales".into());
        let mut any = false;
        for (source, target, label) in &graph.edges {
            if target == &node.id && !is_ambient_source(source) {
                detail.push(format!(
                    "  {} ──{}──> {}",
                    source_display(source),
                    trim_label(label, 30),
                    node.id
                ));
                any = true;
            }
        }
        if !any {
            detail.push("  —".into());
        }

        detail.push(String::new());
        detail.push("Sortantes locales".into());
        any = false;
        for (source, target, label) in &graph.edges {
            if source == &node.id {
                detail.push(format!(
                    "  {} ──{}──> {}",
                    node.id,
                    trim_label(label, 30),
                    target_display(target)
                ));
                any = true;
            }
        }
        if !any {
            detail.push("  —".into());
        }

        if !graph.ambient_edges.is_empty() {
            detail.push(String::new());
            detail.push("Transitions ambiantes possibles".into());
            for (source, target, label) in &graph.ambient_edges {
                detail.push(format!(
                    "  {} ──{}──> {}",
                    source_display(source),
                    trim_label(label, 30),
                    target_display(target)
                ));
            }
        }
    }
    (map_lines, detail)
}

/// Shortens a graph label after replacing embedded line breaks with spaces.
fn trim_label(label: &str, max: usize) -> String {
    let single_line = label.replace('\n', " ").replace('\r', " ");
    if single_line.chars().count() <= max {
        return single_line;
    }
    let mut value = single_line
        .chars()
        .take(max.saturating_sub(1))
        .collect::<String>();
    value.push('…');
    value
}

/// Returns whether an edge source denotes a root or persistent-state pseudo-node.
fn is_ambient_source(source: &str) -> bool {
    source.starts_with('@')
}

/// Produces a human-readable label for a real page or ambient pseudo-source.
fn source_display(source: &str) -> String {
    match source {
        GLOBAL_PRE_SOURCE => "global avant".into(),
        GLOBAL_POST_SOURCE => "global après".into(),
        _ if source.starts_with("@etat:") => source
            .trim_start_matches("@etat:")
            .replace(":avant", " — état avant")
            .replace(":apres", " — état après")
            .replace(":defaite", " — défaite d’état"),
        _ => source.to_owned(),
    }
}

/// Produces a readable label for static and dynamically resolved targets.
fn target_display(target: &str) -> String {
    if target == FORMULA_CONNECTION_TARGET || has_formula_substitution(target) {
        format!("destination dynamique ({target})")
    } else {
        target.to_owned()
    }
}

/// Exports the conservative tale graph in Graphviz DOT format.
///
/// Formula targets are emitted as dashed diamonds and ambient redirects start at dashed ellipse
/// pseudo-nodes. The returned string is self-contained DOT source and performs no file I/O.
pub fn to_dot(root: &Value) -> String {
    let graph = analyze(root);
    let start = root.get("depart").and_then(Value::as_str);
    let mut result = String::from("digraph recit {\n  rankdir=LR;\n  node [shape=box];\n");
    for node in &graph.nodes {
        let mut attrs = vec![format!(
            "label=\"{}\\n{}\"",
            dot_escape(&node.id),
            dot_escape(&node.title)
        )];
        if start == Some(node.id.as_str()) {
            attrs.push("penwidth=2".into());
        }
        if node.depth.is_none() {
            attrs.push("style=dashed".into());
        }
        result.push_str(&format!(
            "  \"{}\" [{}];\n",
            dot_escape(&node.id),
            attrs.join(", ")
        ));
    }

    let mut pseudo_sources = HashSet::new();
    let mut dynamic_targets = HashSet::new();
    for (source, target, _) in &graph.edges {
        if is_ambient_source(source) && pseudo_sources.insert(source.clone()) {
            result.push_str(&format!(
                "  \"{}\" [shape=ellipse, style=dashed, label=\"{}\"];\n",
                dot_escape(source),
                dot_escape(&source_display(source))
            ));
        }
        if (target == FORMULA_CONNECTION_TARGET || has_formula_substitution(target))
            && dynamic_targets.insert(target.clone())
        {
            result.push_str(&format!(
                "  \"{}\" [shape=diamond, style=dashed, label=\"destination dynamique\\n{}\"];\n",
                dot_escape(target),
                dot_escape(target)
            ));
        }
    }

    let mut seen = HashSet::new();
    for (source, target, label) in &graph.edges {
        if seen.insert((source.clone(), target.clone(), label.clone())) {
            result.push_str(&format!(
                "  \"{}\" -> \"{}\" [label=\"{}\"];\n",
                dot_escape(source),
                dot_escape(target),
                dot_escape(label)
            ));
        }
    }
    result.push_str("}\n");
    result
}

/// Escapes text for inclusion in a Graphviz DOT string literal.
fn dot_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FORMULA_CONNECTION_TARGET, analyze};

    #[test]
    fn includes_local_page_and_connection_redirects() {
        let root = json!({
            "depart": "start",
            "pages": {
                "start": {
                    "cellules": [{"effets": [{"type": "rediriger", "destination": "page_redirect"}]}],
                    "connexions": [{
                        "destination": "normal",
                        "texte": "Go",
                        "cellules": [{"effets": [{"type": "rediriger", "destination": "connection_redirect"}]}]
                    }]
                },
                "normal": {},
                "page_redirect": {},
                "connection_redirect": {}
            }
        });
        let graph = analyze(&root);
        assert!(graph.edges.iter().any(|(_, target, _)| target == "page_redirect"));
        assert!(graph.edges.iter().any(|(_, target, _)| target == "connection_redirect"));
        assert!(graph.nodes.iter().all(|node| node.depth.is_some()));
    }

    /// Verifies that any connection formula conservatively makes every page reachable.
    #[test]
    fn formula_connections_make_every_page_conservatively_reachable() {
        let root = json!({
            "depart": "start",
            "pages": {
                "start": {"connexions": [
                    {"texte": "Fin", "destination": "known"},
                    "${recit.connexions()}"
                ]},
                "known": {},
                "otherwise_unreachable": {}
            }
        });
        let graph = analyze(&root);
        assert!(graph.nodes.iter().all(|node| node.depth.is_some()));
        assert!(graph.edges.iter().any(|(source, target, _)| {
            source == "start" && target == FORMULA_CONNECTION_TARGET
        }));
    }

    #[test]
    fn ambient_static_redirect_reaches_only_its_target() {
        let root = json!({
            "depart": "start",
            "avant": [{"effets": [{"type": "rediriger", "destination": "ambient_target"}]}],
            "pages": {
                "start": {},
                "ambient_target": {},
                "unrelated": {}
            }
        });
        let graph = analyze(&root);
        assert!(graph.nodes.iter().find(|node| node.id == "ambient_target").unwrap().depth.is_some());
        assert!(graph.nodes.iter().find(|node| node.id == "unrelated").unwrap().depth.is_none());
    }
    #[test]
    fn includes_effect_and_state_defeat_transitions() {
        let root = json!({
            "depart": "start",
            "etats": {
                "poison": {
                    "defaite": "state_defeat",
                    "avant": [{"effets": [{"type": "modifier_pv", "montant": -1, "defaite": "effect_defeat"}]}]
                }
            },
            "pages": {
                "start": {},
                "state_defeat": {},
                "effect_defeat": {},
                "unrelated": {}
            }
        });
        let graph = analyze(&root);
        assert!(graph.ambient_edges.iter().any(|(_, target, _)| target == "state_defeat"));
        assert!(graph.ambient_edges.iter().any(|(_, target, _)| target == "effect_defeat"));
        assert!(graph.nodes.iter().find(|node| node.id == "state_defeat").unwrap().depth.is_some());
        assert!(graph.nodes.iter().find(|node| node.id == "effect_defeat").unwrap().depth.is_some());
        assert!(graph.nodes.iter().find(|node| node.id == "unrelated").unwrap().depth.is_none());
    }

}
