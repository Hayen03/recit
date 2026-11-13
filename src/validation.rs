//! Structural, referential, formula, and conservative graph validation for raw tales.
//!
//! Validation accumulates independent problems instead of stopping at the first error.
//! Dynamic destinations are checked syntactically and treated conservatively for
//! reachability because their runtime page IDs are unknowable statically.
use std::collections::{BTreeSet, HashMap, HashSet};

use base64::Engine;
use serde_json::{Map, Value};

use crate::{
    document::single_id_filter,
    formula::{full_formula, has_formula_substitution, validate_formula_syntax},
    graph,
    images::SUPPORTED_FORMATS,
    json_util::required_value,
    schema::{
        FilterFamily, CONDITIONAL_TYPES, CONDITION_EFFECT_TYPES, EFFECT_TYPES, RESERVED_ATTRIBUTES,
    },
};

/// Classifies a validation problem as an error, warning, or information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    /// Returns the compact symbol used to display this severity.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Error => "✗",
            Self::Warning => "!",
            Self::Info => "·",
        }
    }

    /// Returns whether this severity represents a blocking error.
    pub fn is_error(self) -> bool {
        self == Self::Error
    }
}

/// Describes one validation issue and its JSON location.
#[derive(Debug, Clone)]
pub struct Problem {
    pub severity: Severity,
    pub path: String,
    pub message: String,
}

impl Problem {
    /// Creates a validation problem with error severity.
    fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            path: path.into(),
            message: message.into(),
        }
    }

    /// Creates a validation problem with warning severity.
    fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            path: path.into(),
            message: message.into(),
        }
    }

    /// Creates a validation problem with info severity.
    fn info(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Collects tale identifiers and validation output during a validation pass.
struct Context<'a> {
    pages: HashSet<&'a str>,
    items: HashSet<&'a str>,
    states: HashSet<&'a str>,
    attributes: HashSet<&'a str>,
    images: HashSet<&'a str>,
}
/// Validates raw version-2 tale structure, formulas, references, and conservative reachability.
///
/// The validator accumulates independent problems instead of stopping at the first recoverable issue. It does not mutate the document.
pub fn validate_tale(root: &Value) -> Vec<Problem> {
    let mut out = Vec::new();
    let Some(object) = root.as_object() else {
        return vec![Problem::error("$", "La racine doit être un objet JSON")];
    };

    check_optional_string(object, "titre", "titre", &mut out);
    check_optional_string(object, "auteur", "auteur", &mut out);
    check_optional_string(object, "synopsis", "synopsis", &mut out);
    if object.get("version").and_then(Value::as_u64) != Some(2) {
        out.push(Problem::error(
            "version",
            "La syntaxe courante exige `version`: 2; utilisez recit-v1-vers-v2 pour un ancien récit",
        ));
    }
    check_optional_object(object.get("global"), "global", &mut out);
    validate_global_values(object.get("global"), &mut out);
    if !object.contains_key("global") {
        // Compatibility validation for tales authored before the unified global dictionary.
        validate_formula_functions(object.get("fonctions"), &mut out);
    }

    check_optional_object(object.get("tags"), "tags", &mut out);

    let attributes = validate_attributes(object.get("attributs"), &mut out);
    let pages_map = match object.get("pages").and_then(Value::as_object) {
        Some(pages) => pages,
        None => {
            out.push(Problem::error(
                "pages",
                "Le champ `pages` doit être un objet",
            ));
            return out;
        }
    };
    if pages_map.is_empty() {
        out.push(Problem::warning(
            "pages",
            "Le récit ne contient aucune page",
        ));
    }
    let items_map = optional_object(object, "objets", &mut out);
    let states_map = optional_object(object, "etats", &mut out);
    let images_map = optional_object(object, "images", &mut out);

    let context = Context {
        pages: pages_map.keys().map(String::as_str).collect(),
        items: items_map.keys().map(String::as_str).collect(),
        states: states_map.keys().map(String::as_str).collect(),
        attributes,
        images: images_map.keys().map(String::as_str).collect(),
    };

    validate_image_definitions(images_map, &mut out);
    validate_title_images(object.get("image"), "image", &context, &mut out);

    match object.get("depart").and_then(Value::as_str) {
        Some(start) if context.pages.contains(start) => {}
        Some(start) => out.push(Problem::error(
            "depart",
            format!("La page de départ `{start}` n'existe pas"),
        )),
        None => out.push(Problem::error(
            "depart",
            "Le champ `depart` doit contenir l'identifiant d'une page",
        )),
    }

    validate_cell_container(object.get("avant"), "avant", &context, &mut out);
    validate_cell_container(object.get("apres"), "apres", &context, &mut out);
    if object.contains_key("effets") {
        out.push(Problem::error(
            "effets",
            "`effets` à la racine appartient à la syntaxe v1; utilisez `avant`",
        ));
    }
    for english_key in ["pre", "post"] {
        if object.contains_key(english_key) {
            out.push(Problem::error(
                english_key,
                format!("La syntaxe française v2 utilise `avant`/`apres`, pas `{english_key}`"),
            ));
        }
    }

    for (id, item) in items_map {
        validate_item_definition(item, &format!("objets.{id}"), &mut out);
    }
    for (id, state) in states_map {
        validate_state_definition(state, &format!("etats.{id}"), &context, &mut out);
    }
    for (id, page) in pages_map {
        validate_page(page, &format!("pages.{id}"), &context, &mut out);
    }

    graph_problems(root, &mut out);

    // Validation sémantique partagée avec le moteur de référence. La forge
    // conserve toujours le JSON brut, mais le même modèle typé décide si le
    // récit pourra réellement être joué.
    match crate::model::Tale::from_json(root) {
        Ok(tale) => {
            for error in tale.validate_references() {
                if !out
                    .iter()
                    .any(|problem| problem.message.as_str() == error.as_ref())
                {
                    out.push(Problem::error("moteur", error.as_ref().to_string()));
                }
            }
        }
        Err(error) => out.push(Problem::error(
            "moteur",
            format!("Le moteur de référence refuse ce récit: {error:#}"),
        )),
    }

    out
}

/// Validates formula-bearing leaves in the immutable root `global` dictionary.
fn validate_global_values(value: Option<&Value>, out: &mut Vec<Problem>) {
    fn visit(value: &Value, path: &str, out: &mut Vec<Problem>) {
        match value {
            Value::String(text) => {
                check_formula_text(text, path, out);
                if full_formula(text).is_some() {
                    if let Err(error) = validate_formula_syntax(text) {
                        out.push(Problem::error(path, format!("Formule invalide: {error:#}")));
                    }
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    visit(value, &format!("{path}[{index}]"), out);
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    visit(value, &format!("{path}.{key}"), out);
                }
            }
            _ => {}
        }
    }

    if let Some(Value::Object(values)) = value {
        for (key, value) in values {
            visit(value, &format!("global.{key}"), out);
        }
    }
}

/// Validates the root dictionary of tale-defined formula functions.
fn validate_formula_functions(value: Option<&Value>, out: &mut Vec<Problem>) {
    let Some(value) = value else {
        return;
    };
    let Some(functions) = value.as_object() else {
        out.push(Problem::error(
            "fonctions",
            "Le champ `fonctions` doit être un objet dont les clés sont les noms de fonctions",
        ));
        return;
    };

    for (name, definition) in functions {
        let path = format!("fonctions.{name}");
        if !is_formula_identifier(name) {
            out.push(Problem::error(
                &path,
                format!("`{name}` n'est pas un nom de fonction valide"),
            ));
        }
        if RESERVED_ATTRIBUTES.contains(&name.as_str())
            || matches!(
                name.as_str(),
                "true"
                    | "false"
                    | "null"
                    | "None"
                    | "True"
                    | "False"
                    | "for"
                    | "if"
                    | "else"
                    | "let"
                    | "in"
                    | "and"
                    | "or"
                    | "not"
            )
        {
            out.push(Problem::error(
                &path,
                format!("Le nom de fonction `{name}` est réservé"),
            ));
        }
        let Some(definition) = definition.as_object() else {
            out.push(Problem::error(
                &path,
                "Une fonction doit être un objet contenant `formule` et éventuellement `parametres`",
            ));
            continue;
        };

        if let Some(parameters) = definition.get("parametres") {
            match parameters {
                Value::Null => {}
                Value::Array(parameters) => {
                    let mut seen = HashSet::new();
                    for (index, parameter) in parameters.iter().enumerate() {
                        let parameter_path = format!("{path}.parametres[{index}]");
                        let Some(parameter) = parameter.as_str() else {
                            out.push(Problem::error(
                                parameter_path,
                                "Un paramètre doit être une chaîne",
                            ));
                            continue;
                        };
                        if !is_formula_identifier(parameter) {
                            out.push(Problem::error(
                                &parameter_path,
                                format!("`{parameter}` n'est pas un nom de paramètre valide"),
                            ));
                        }
                        if parameter == "args" {
                            out.push(Problem::error(
                                &parameter_path,
                                "`args` est réservé à la liste complète des arguments",
                            ));
                        }
                        if !seen.insert(parameter) {
                            out.push(Problem::error(
                                &parameter_path,
                                format!("Le paramètre `{parameter}` est répété"),
                            ));
                        }
                    }
                }
                _ => out.push(Problem::error(
                    format!("{path}.parametres"),
                    "`parametres` doit être un tableau de chaînes ou null",
                )),
            }
        }

        match definition.get("formule") {
            Some(Value::String(formula)) => {
                if full_formula(formula).is_none() {
                    out.push(Problem::error(
                        format!("{path}.formule"),
                        "Le corps doit être une formule complète `${...}`",
                    ));
                } else if let Err(error) = validate_formula_syntax(formula) {
                    out.push(Problem::error(
                        format!("{path}.formule"),
                        format!("Formule invalide: {error:#}"),
                    ));
                }
            }
            Some(_) => out.push(Problem::error(
                format!("{path}.formule"),
                "`formule` doit être une chaîne",
            )),
            None => out.push(Problem::error(
                format!("{path}.formule"),
                "Champ `formule` manquant",
            )),
        }
    }
}

/// Returns whether `name` can be parsed as one formula identifier.
fn is_formula_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

/// Validates the optional attribute list and returns the usable names.
fn validate_attributes<'a>(value: Option<&'a Value>, out: &mut Vec<Problem>) -> HashSet<&'a str> {
    let defaults: HashSet<&str> = [
        "force",
        "agilite",
        "endurance",
        "presence",
        "chance",
        "connaissance",
        "perception",
    ]
    .into_iter()
    .collect();
    let Some(value) = value else {
        return defaults;
    };
    let Some(array) = value.as_array() else {
        out.push(Problem::error(
            "attributs",
            "Le champ `attributs` doit être un tableau de chaînes",
        ));
        return HashSet::new();
    };
    let mut seen = HashSet::new();
    for (index, value) in array.iter().enumerate() {
        let Some(attribute) = value.as_str() else {
            out.push(Problem::error(
                format!("attributs[{index}]"),
                "Un nom d'attribut doit être une chaîne",
            ));
            continue;
        };
        if attribute.is_empty() {
            out.push(Problem::error(
                format!("attributs[{index}]"),
                "Un nom d'attribut ne peut pas être vide",
            ));
        }
        if !seen.insert(attribute) {
            out.push(Problem::warning(
                format!("attributs[{index}]"),
                format!("L'attribut `{attribute}` est répété"),
            ));
        }
        if RESERVED_ATTRIBUTES.contains(&attribute) {
            out.push(Problem::error(
                format!("attributs[{index}]"),
                format!("`{attribute}` est un mot réservé"),
            ));
        }
    }
    seen
}

/// Validates an optional field as a JSON object and returns it when present.
fn optional_object<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    out: &mut Vec<Problem>,
) -> &'a Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    match object.get(key) {
        None => EMPTY.get_or_init(Map::new),
        Some(Value::Object(value)) => value,
        Some(_) => {
            out.push(Problem::error(
                key,
                format!("Le champ `{key}` doit être un objet"),
            ));
            EMPTY.get_or_init(Map::new)
        }
    }
}

/// Validates one page object and all of its nested content.
fn validate_page(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    let Some(page) = value.as_object() else {
        out.push(Problem::error(path, "Une page doit être un objet"));
        return;
    };
    check_optional_string(page, "titre", &format!("{path}.titre"), out);
    validate_runtime_tags(page.get("tags"), &format!("{path}.tags"), out);
    validate_cell_container(page.get("cellules"), &format!("{path}.cellules"), ctx, out);
    for old_key in ["texte", "traits", "effets", "apres"] {
        if page.contains_key(old_key) {
            out.push(Problem::error(
                format!("{path}.{old_key}"),
                format!("`{old_key}` est un champ de page v1; le contenu appartient maintenant à `cellules`"),
            ));
        }
    }
    validate_connections(page.get("connexions"), &format!("{path}.connexions"), ctx, out);
    if let Some(dyn_conn) = page.get("connexions_dyn") {
        out.push(Problem::warning(
            format!("{path}.connexions_dyn"),
            "`connexions_dyn` est obsolète; placez directement ses formules dans `connexions`",
        ));
        validate_dyn_connection(dyn_conn, &format!("{path}.connexions_dyn"), ctx, out);
    }
}

/// Validates every entry of the root image registry.
fn validate_image_definitions(images: &Map<String, Value>, out: &mut Vec<Problem>) {
    for (id, value) in images {
        let path = format!("images.{id}");
        if id.is_empty() {
            out.push(Problem::error(&path, "L'identifiant d'une image ne peut pas être vide"));
        }
        let Some(definition) = value.as_object() else {
            out.push(Problem::error(
                &path,
                "Une définition d'image doit être un objet",
            ));
            continue;
        };
        check_optional_string(definition, "alt", &format!("{path}.alt"), out);
        let Some(kind) = definition.get("type").and_then(Value::as_str) else {
            out.push(Problem::error(
                format!("{path}.type"),
                "Le champ `type` d'une image doit être une chaîne",
            ));
            continue;
        };
        let allowed_source = match kind {
            "fichier" => {
                require_nonempty_string(definition, "chemin", &path, out);
                Some("chemin")
            }
            "url" => {
                require_nonempty_string(definition, "url", &path, out);
                if let Some(url) = definition.get("url").and_then(Value::as_str) {
                    let lowercase_url = url.to_ascii_lowercase();
                    if !url.is_empty()
                        && !lowercase_url.starts_with("http://")
                        && !lowercase_url.starts_with("https://")
                    {
                        out.push(Problem::error(
                            format!("{path}.url"),
                            "L'URL d'une image doit utiliser HTTP ou HTTPS",
                        ));
                    }
                }
                Some("url")
            }
            "base64" => {
                require_nonempty_string(definition, "donnees", &path, out);
                if let Some(data) = definition.get("donnees").and_then(Value::as_str) {
                    if !data.is_empty()
                        && base64::engine::general_purpose::STANDARD
                            .decode(data)
                            .is_err()
                    {
                        out.push(Problem::error(
                            format!("{path}.donnees"),
                            "Les données de l'image ne sont pas du base64 standard valide",
                        ));
                    }
                }
                match definition.get("format").and_then(Value::as_str) {
                    Some(format) if SUPPORTED_FORMATS.contains(&format) => {}
                    Some(format) => out.push(Problem::error(
                        format!("{path}.format"),
                        format!(
                            "Format d'image intégré non pris en charge: `{format}`; utilisez {}",
                            SUPPORTED_FORMATS
                                .iter()
                                .map(|format| format!("`{format}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )),
                    None => out.push(Problem::error(
                        format!("{path}.format"),
                        "Une image base64 doit déclarer son `format`",
                    )),
                }
                Some("donnees")
            }
            other => {
                out.push(Problem::error(
                    format!("{path}.type"),
                    format!(
                        "Type d'image inconnu: `{other}`; utilisez `chemin`, `url` ou `base64`"
                    ),
                ));
                None
            }
        };

        for key in definition.keys() {
            let allowed = matches!(key.as_str(), "type" | "alt")
                || allowed_source == Some(key.as_str())
                || (kind == "base64" && key == "format");
            if !allowed {
                out.push(Problem::warning(
                    format!("{path}.{key}"),
                    format!("Le champ `{key}` n'est pas utilisé par une image de type `{kind}`"),
                ));
            }
        }
    }
}

/// Validates the image or ordered image list displayed before the synopsis.
fn validate_title_images(
    value: Option<&Value>,
    path: &str,
    ctx: &Context<'_>,
    out: &mut Vec<Problem>,
) {
    match value {
        None => {}
        Some(Value::String(reference)) => {
            validate_image_reference(reference, path, ctx, false, out)
        }
        Some(Value::Array(references)) => {
            for (index, reference) in references.iter().enumerate() {
                let reference_path = format!("{path}[{index}]");
                match reference.as_str() {
                    Some(reference) => {
                        validate_image_reference(reference, &reference_path, ctx, false, out)
                    }
                    None => out.push(Problem::error(
                        reference_path,
                        "Une image de titre doit être référencée par une chaîne",
                    )),
                }
            }
        }
        Some(_) => out.push(Problem::error(
            path,
            "`image` doit être un identifiant d'image ou une liste d'identifiants",
        )),
    }
}

/// Validates the optional image field of a cell, including deferred formula entries.
fn validate_cell_images(
    value: Option<&Value>,
    path: &str,
    ctx: &Context<'_>,
    out: &mut Vec<Problem>,
) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::String(reference)) => {
            validate_image_reference(reference, path, ctx, true, out)
        }
        Some(Value::Array(references)) => {
            for (index, reference) in references.iter().enumerate() {
                let reference_path = format!("{path}[{index}]");
                match reference {
                    Value::Null => {}
                    Value::String(reference) => {
                        validate_image_reference(reference, &reference_path, ctx, true, out)
                    }
                    _ => out.push(Problem::error(
                        reference_path,
                        "Une image de cellule doit être un identifiant, une formule ou null",
                    )),
                }
            }
        }
        Some(_) => out.push(Problem::error(
            path,
            "`images` doit être un identifiant, une formule ou une liste de ces formes",
        )),
    }
}

/// Validates one static image ID, or one formula where that context permits formulas.
fn validate_image_reference(
    reference: &str,
    path: &str,
    ctx: &Context<'_>,
    allow_formula: bool,
    out: &mut Vec<Problem>,
) {
    if has_formula_substitution(reference) {
        if !allow_formula {
            out.push(Problem::error(
                path,
                "Les images de titre n'acceptent pas les formules",
            ));
            return;
        }
        check_formula_text(reference, path, out);
        if full_formula(reference).is_some() {
            if let Err(error) = validate_formula_syntax(reference) {
                out.push(Problem::error(
                    path,
                    format!("Formule d'image invalide: {error:#}"),
                ));
            }
        }
        return;
    }
    if !ctx.images.contains(reference) {
        out.push(Problem::error(
            path,
            format!("L'image `{reference}` n'existe pas dans `images`"),
        ));
    }
}

/// Validates the unified page-connection container.
///
/// The field may be absent, null, one connection object, one complete formula, or an
/// array mixing objects and complete formulas. Formula results are necessarily checked
/// only at runtime.
fn validate_connections(
    value: Option<&Value>,
    path: &str,
    ctx: &Context<'_>,
    out: &mut Vec<Problem>,
) {
    match value {
        None | Some(Value::Null) => {}
        Some(value @ Value::Object(_)) => validate_connection(value, path, ctx, out),
        Some(value @ Value::String(_)) => validate_connection_formula(value, path, out),
        Some(Value::Array(connections)) => {
            for (index, connection) in connections.iter().enumerate() {
                let entry_path = format!("{path}[{index}]");
                match connection {
                    Value::Object(_) => validate_connection(connection, &entry_path, ctx, out),
                    Value::String(_) => validate_connection_formula(connection, &entry_path, out),
                    _ => out.push(Problem::error(
                        entry_path,
                        "Une entrée de `connexions` doit être un dictionnaire ou une formule complète",
                    )),
                }
            }
        }
        Some(_) => out.push(Problem::error(
            path,
            "`connexions` doit être une connexion, une formule complète ou une liste de ces formes",
        )),
    }
}

/// Validates one formula that will create a connection or a connection list at runtime.
fn validate_connection_formula(value: &Value, path: &str, out: &mut Vec<Problem>) {
    let Some(source) = value.as_str() else {
        return;
    };
    if full_formula(source).is_none() {
        out.push(Problem::error(
            path,
            "Une connexion sous forme de chaîne doit être une formule complète `${...}`",
        ));
        return;
    }
    if let Err(error) = validate_formula_syntax(source) {
        out.push(Problem::error(path, format!("Formule de connexion invalide: {error:#}")));
        return;
    }
    out.push(Problem::info(
        path,
        "Connexion produite par formule: sa structure et ses destinations seront confirmées à l'exécution",
    ));
}

/// Validates one static connection and its check and cells.
fn validate_connection(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    let Some(connection) = value.as_object() else {
        out.push(Problem::error(path, "Une connexion doit être un objet"));
        return;
    };
    require_string(connection, "texte", path, out);
    match connection.get("destination").and_then(Value::as_str) {
        Some("") => out.push(Problem::error(
            format!("{path}.destination"),
            "La destination d'une connexion ne peut pas être vide",
        )),
        Some(destination) if has_formula_substitution(destination) => {
            check_formula_text(destination, &format!("{path}.destination"), out);
            out.push(Problem::info(
                format!("{path}.destination"),
                "Destination dynamique: son existence sera vérifiée pendant l'exécution",
            ));
        }
        Some(destination) if ctx.pages.contains(destination) => {}
        Some(destination) => out.push(Problem::error(
            format!("{path}.destination"),
            format!("La page `{destination}` n'existe pas"),
        )),
        None => out.push(Problem::error(
            format!("{path}.destination"),
            "Une connexion doit avoir une destination",
        )),
    }
    if let Some(conditional) = connection.get("conditions") {
        validate_conditional(conditional, &format!("{path}.conditions"), ctx, out);
    }
    if let Some(check) = connection.get("test") {
        validate_check(check, &format!("{path}.test"), ctx, out);
    }
    validate_cell_container(
        connection.get("cellules"),
        &format!("{path}.cellules"),
        ctx,
        out,
    );
    validate_runtime_tags(connection.get("tags"), &format!("{path}.tags"), out);
    if connection.contains_key("contexte") {
        out.push(Problem::error(
            format!("{path}.contexte"),
            "`contexte` n'est plus pris en charge; utilisez des captures pour les formules de la connexion et `tags` pour transmettre des valeurs dans `connexion`",
        ));
    }
    if connection.contains_key("effets") {
        out.push(Problem::error(
            format!("{path}.effets"),
            "Les effets de connexion v2 doivent être placés dans `cellules`",
        ));
    }
}

/// Validates page or connection tags whose values may be literals or formulas.
fn validate_runtime_tags(value: Option<&Value>, path: &str, out: &mut Vec<Problem>) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let Some(tags) = value.as_object() else {
        out.push(Problem::error(path, "Les tags doivent être un dictionnaire"));
        return;
    };
    for (key, value) in tags {
        let tag_path = format!("{path}.{key}");
        if let Value::String(source) = value {
            check_formula_text(source, &tag_path, out);
            if full_formula(source).is_some() {
                if let Err(error) = validate_formula_syntax(source) {
                    out.push(Problem::error(
                        tag_path,
                        format!("Formule de tag invalide: {error:#}"),
                    ));
                }
            }
        }
    }
}

fn validate_dyn_connection(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    match value {
        Value::Array(arr) => {
            for (index, conn) in arr.iter().enumerate() {
                validate_dyn_connection(conn, &format!("{path}.{index}"), ctx, out);
            }
        },
        Value::Object(obj) => {
            require_string(obj, "formule", path, out);
            if let Some(form) = obj.get("formule") {
                validate_connection_formula(form, &format!("{path}.formule"), out);
            }
        },
        Value::String(_) => validate_connection_formula(value, path, out),
        _ => out.push(Problem::error(
            path,
            format!("{path} doit être un tableau de connexions dynamiques, une formule ou un objet contenant un champ `formule`."),
        )),
    }
}

/// Validates one connection check definition.
fn validate_check(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    let Some(check) = value.as_object() else {
        out.push(Problem::error(path, "Un test doit être un objet"));
        return;
    };
    match check.get("attribut").and_then(Value::as_str) {
        Some(attribute) if ctx.attributes.contains(attribute) => {}
        Some(attribute) => out.push(Problem::error(
            format!("{path}.attribut"),
            format!("L'attribut `{attribute}` n'existe pas"),
        )),
        None => out.push(Problem::error(
            format!("{path}.attribut"),
            "Un test doit spécifier un attribut",
        )),
    }
    check_formula_value(
        check.get("difficulte"),
        Some("int"),
        false,
        &format!("{path}.difficulte"),
        out,
    );
    check_string_list(check.get("traits"), &format!("{path}.traits"), false, out);
    if let Some(failure) = check.get("echec") {
        if !failure.is_null() {
            match failure.as_str() {
                Some(form) if has_formula_substitution(form) => {}
                Some(id) if ctx.pages.contains(id) => {}
                Some(id) => out.push(Problem::error(
                    format!("{path}.echec"),
                    format!("La page `{id}` n'existe pas"),
                )),
                None => out.push(Problem::error(
                    format!("{path}.echec"),
                    "`echec` doit être une page ou null",
                )),
            }
        }
    }
}

/// Validates one reusable item definition.
fn validate_item_definition(value: &Value, path: &str, out: &mut Vec<Problem>) {
    let Some(item) = value.as_object() else {
        out.push(Problem::error(
            path,
            "Une définition d'objet doit être un objet",
        ));
        return;
    };
    check_optional_string(item, "nom", &format!("{path}.nom"), out);
    check_optional_string(item, "description", &format!("{path}.description"), out);
    check_string_list(item.get("traits"), &format!("{path}.traits"), false, out);
    check_optional_object(item.get("donnees"), &format!("{path}.donnees"), out);
}

/// Validates one reusable persistent-state definition.
fn validate_state_definition(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    let Some(state) = value.as_object() else {
        out.push(Problem::error(
            path,
            "Une définition d'état doit être un objet",
        ));
        return;
    };
    check_optional_string(state, "nom", &format!("{path}.nom"), out);
    check_optional_string(state, "description", &format!("{path}.description"), out);
    check_string_list(state.get("traits"), &format!("{path}.traits"), false, out);
    check_optional_object(state.get("donnees"), &format!("{path}.donnees"), out);
    validate_cell_container(state.get("avant"), &format!("{path}.avant"), ctx, out);
    validate_cell_container(state.get("apres"), &format!("{path}.apres"), ctx, out);
    match state.get("effets") {
        None => {}
        Some(Value::Array(effects)) => {
            for (index, effect) in effects.iter().enumerate() {
                validate_condition_effect(effect, &format!("{path}.effets[{index}]"), ctx, out);
            }
        }
        Some(_) => out.push(Problem::error(
            format!("{path}.effets"),
            "Les effets d'état doivent être un tableau",
        )),
    }
    if let Some(expiration) = state.get("expiration") {
        if !expiration.is_null() {
            validate_filter(
                expiration,
                &format!("{path}.expiration"),
                FilterFamily::Condition,
                out,
            );
        }
    }
    if let Some(defeat) = state.get("defaite") {
        validate_optional_page_ref(defeat, &format!("{path}.defaite"), ctx, out);
    }
}

/// Validates one persistent-state modifier definition.
fn validate_condition_effect(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    let Some(effect) = value.as_object() else {
        out.push(Problem::error(path, "Un effet d'état doit être un objet"));
        return;
    };
    let Some(kind) = effect.get("type").and_then(Value::as_str) else {
        out.push(Problem::error(
            format!("{path}.type"),
            "Type d'effet manquant",
        ));
        return;
    };
    if !CONDITION_EFFECT_TYPES.iter().any(|(id, _)| *id == kind) {
        out.push(Problem::warning(
            format!("{path}.type"),
            format!("Type d'effet d'état inconnu: `{kind}`; il sera conservé tel quel"),
        ));
        return;
    }
    match kind {
        "modificateur_attribut" => match effect.get("attribut").and_then(Value::as_str) {
            Some(attribute) if ctx.attributes.contains(attribute) => {}
            Some(attribute) => out.push(Problem::error(
                format!("{path}.attribut"),
                format!("L'attribut `{attribute}` n'existe pas"),
            )),
            None => out.push(Problem::error(
                format!("{path}.attribut"),
                "Attribut manquant",
            )),
        },
        "modificateur_test" => {
            if let Some(filter) = effect.get("filtre") {
                validate_filter(filter, &format!("{path}.filtre"), FilterFamily::Check, out);
            }
        }
        _ => {}
    }
    check_formula_value(
        effect.get("valeur"),
        Some("int"),
        false,
        &format!("{path}.valeur"),
        out,
    );
    if let Some(category) = effect.get("categorie") {
        if !category.is_null() && !category.is_string() {
            out.push(Problem::error(
                format!("{path}.categorie"),
                "La catégorie doit être une chaîne ou null",
            ));
        }
    }
}

/// Validates an ordered array of normalized cells and unique optional IDs.
fn validate_cell_container(
    value: Option<&Value>,
    path: &str,
    ctx: &Context<'_>,
    out: &mut Vec<Problem>,
) {
    let Some(value) = value else { return };
    let cells = match value {
        Value::Null => return,
        Value::String(_) => {
            validate_deferred_formula(value, path, "cellule", out);
            return;
        }
        Value::Array(arr) => arr,
        Value::Object(_) => return validate_cell(value, path, ctx, out),
        _ => {
            out.push(Problem::error(
                path,
                "Les cellules doivent être un objet, une formule ou une liste de ces formes",
            ));
            return;
        }
    };
    let mut ids = HashSet::new();
    for (index, cell) in cells.iter().enumerate() {
        let cell_path = format!("{path}[{index}]");
        if cell.is_string() {
            validate_deferred_formula(cell, &cell_path, "cellule", out);
            continue;
        }
        if let Some(id) = cell.get("id").and_then(Value::as_str) {
            if id.is_empty() {
                out.push(Problem::error(
                    format!("{cell_path}.id"),
                    "L'ID de cellule ne peut pas être vide",
                ));
            } else if !ids.insert(id) {
                out.push(Problem::error(
                    format!("{cell_path}.id"),
                    format!("ID de cellule dupliqué: `{id}`"),
                ));
            }
        }
        validate_cell(cell, &cell_path, ctx, out);
    }
}

/// Validates one normalized cell and its optional narrative/effect content.
fn validate_cell(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    let Some(object) = value.as_object() else {
        out.push(Problem::error(path, "Une cellule doit être un objet"));
        return;
    };
    check_optional_string(object, "id", &format!("{path}.id"), out);
    check_optional_string(object, "libelle", &format!("{path}.libelle"), out);
    check_optional_string(object, "texte", &format!("{path}.texte"), out);
    if let Some(conditions) = object.get("conditions") {
        validate_conditional(conditions, &format!("{path}.conditions"), ctx, out);
    }
    if let Some(effects) = object.get("effets") {
        validate_atomic_effects(effects, &format!("{path}.effets"), ctx, out);
    }
    let has_images = match object.get("images") {
        Some(Value::String(_)) => true,
        Some(Value::Array(images)) => images.iter().any(|image| !image.is_null()),
        _ => false,
    };
    validate_cell_images(object.get("images"), &format!("{path}.images"), ctx, out);
    if !object.contains_key("texte")
        && !object.contains_key("effets")
        && !object.contains_key("conditions")
        && !has_images
    {
        out.push(Problem::error(
            path,
            "Une cellule doit contenir `texte`, `effets`, `conditions` ou `images`",
        ));
    }
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "id" | "libelle" | "texte" | "effets" | "conditions" | "images"
        ) {
            let problem = if key == "label" {
                Problem::error(
                    format!("{path}.{key}"),
                    "La syntaxe française v2 utilise `libelle`, pas `label`",
                )
            } else {
                Problem::warning(format!("{path}.{key}"), "Clé inconnue dans une cellule")
            };
            out.push(problem);
        }
    }
}

/// Validates the atomic effect array nested inside one cell.
fn validate_atomic_effects(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    let effects: Vec<&Value> = match value {
        Value::Null => return,
        Value::String(_) => {
            validate_deferred_formula(value, path, "liste d'effets", out);
            return;
        }
        Value::Object(_) => vec![value],
        Value::Array(effects) => effects.iter().collect(),
        _ => {
            out.push(Problem::error(
                path,
                "Les effets d'une cellule doivent être un objet, une formule ou une liste de ces formes",
            ));
            return;
        }
    };
    for (index, effect) in effects.into_iter().enumerate() {
        let effect_path = format!("{path}[{index}]");
        if effect.is_string() {
            validate_deferred_formula(effect, &effect_path, "effet", out);
            continue;
        }
        let Some(effect) = effect.as_object() else {
            out.push(Problem::error(effect_path, "Un effet doit être un objet"));
            continue;
        };
        if effect.contains_key("effets") {
            out.push(Problem::error(
                effect_path,
                "Les cellules ne peuvent pas être imbriquées dans `effets`",
            ));
            continue;
        }
        validate_simple_effect(effect, &effect_path, ctx, out);
    }
}

/// Validates one atomic page effect.
fn validate_simple_effect(
    effect: &Map<String, Value>,
    path: &str,
    ctx: &Context<'_>,
    out: &mut Vec<Problem>,
) {
    if let Some(notification) = effect.get("notification") {
        match notification {
            Value::Null => {}
            Value::String(template) => {
                check_formula_text(template, &format!("{path}.notification"), out)
            }
            _ => {
                out.push(Problem::error(
                    format!("{path}.notification"),
                    "`notification` doit être une chaîne ou null",
                ));
            }
        }
    }
    let Some(kind) = effect.get("type").and_then(Value::as_str) else {
        out.push(Problem::error(
            format!("{path}.type"),
            "Type d'effet manquant",
        ));
        return;
    };
    if !EFFECT_TYPES.iter().any(|(id, _)| *id == kind) {
        out.push(Problem::warning(
            format!("{path}.type"),
            format!("Type d'effet inconnu: `{kind}`; il sera conservé tel quel"),
        ));
        return;
    }
    match kind {
        "ajouter_etat" => {
            validate_named_ref(effect, "etat", &ctx.states, path, "état", out);
            validate_object_or_formula(effect.get("donnees"), &format!("{path}.donnees"), out);
        }
        "retirer_etat" => {
            require_filter(effect, path, FilterFamily::Condition, out);
            validate_selection(effect, path, out);
        }
        "modifier_etat" => {
            require_filter(effect, path, FilterFamily::Condition, out);
            validate_selection(effect, path, out);
            validate_object_or_formula(effect.get("donnees"), &format!("{path}.donnees"), out);
        }
        "definir_tag" | "incrementer_tag" => {
            require_filter(effect, path, FilterFamily::Tag, out);
            validate_selection(effect, path, out);
            validate_tag_path(effect, path, out);
            if let Some(filter) = effect.get("filtre") {
                if single_id_filter(filter, "tag").is_none() {
                    out.push(Problem::info(
                        format!("{path}.filtre"),
                        "Un filtre composite ne crée aucun tag lorsqu'il ne trouve rien",
                    ));
                }
            }
            let value_key = if kind == "definir_tag" {
                "valeur"
            } else {
                "valeur"
            };
            check_formula_value(
                effect.get(value_key),
                None,
                true,
                &format!("{path}.{value_key}"),
                out,
            );
            if effect.contains_key("tag") {
                out.push(Problem::error(
                    format!("{path}.tag"),
                    "La nouvelle syntaxe utilise `filtre`, pas `tag`",
                ));
            }
        }
        "ajouter_objet" => {
            validate_named_ref(effect, "objet", &ctx.items, path, "objet", out);
            check_formula_value(
                effect.get("quantite"),
                Some("int"),
                false,
                &format!("{path}.quantite"),
                out,
            );
            validate_object_or_formula(effect.get("donnees"), &format!("{path}.donnees"), out);
        }
        "retirer_objet" => {
            require_filter(effect, path, FilterFamily::Item, out);
            validate_selection(effect, path, out);
            check_formula_value(
                effect.get("quantite"),
                Some("int"),
                false,
                &format!("{path}.quantite"),
                out,
            );
        }
        "modifier_objet" => {
            require_filter(effect, path, FilterFamily::Item, out);
            validate_selection(effect, path, out);
            validate_object_or_formula(effect.get("donnees"), &format!("{path}.donnees"), out);
        }
        "modifier_attribut" | "definir_attribut" => {
            match effect.get("attribut").and_then(Value::as_str) {
                Some(attribute) if ctx.attributes.contains(attribute) => {}
                Some(attribute) => out.push(Problem::error(
                    format!("{path}.attribut"),
                    format!("L'attribut `{attribute}` n'existe pas"),
                )),
                None => out.push(Problem::error(
                    format!("{path}.attribut"),
                    "Attribut manquant",
                )),
            }
            check_formula_value(
                effect.get("valeur"),
                Some("int"),
                false,
                &format!("{path}.valeur"),
                out,
            );
        }
        "modifier_pv" => {
            check_formula_value(
                effect.get("valeur"),
                Some("int"),
                false,
                &format!("{path}.valeur"),
                out,
            );
            validate_effect_defeat(effect, path, ctx, out);
        }
        "modifier_nom" => {
            let p = &format!("{path}.valeur");
            require_string(effect, "valeur", p, out);
            check_formula_value(effect.get("valeur"), Some("str"), false, p, out);
        }
        "retirer_tag" => {
            require_filter(effect, path, FilterFamily::Tag, out);
            validate_selection(effect, path, out);
        }
        "demander" => {
            require_string(effect, "tag", &format!("{path}.tag"), out);
            require_string(effect, "question", &format!("{path}.question"), out);
            if let Some(question) = effect.get("question").and_then(Value::as_str) {
                check_formula_text(question, &format!("{path}.question"), out);
            }
            let donnees = effect.get("donnees");
            if let Some(donnees) = donnees {
                validate_poll_data_type(donnees, &format!("{path}.donnees"), out);
            }
            let dtype = donnees.and_then(Value::as_str).unwrap_or("str");
            check_formula_value(
                effect.get("defaut"),
                Some(dtype),
                true,
                &format!("{path}.defaut"),
                out,
            );
            if let Some(choix) = effect.get("choix") {
                if let Some(donnees) = donnees {
                    if donnees == "bool" {
                        out.push(Problem::warning(
                            &format!("{path}.choix"),
                            "Les choix sont ignorés pour un type booléen",
                        ))
                    }
                }
                validate_poll_choices(choix, dtype, &format!("{path}.choix"), out);
            }
        }
        "rediriger" => match effect.get("destination").and_then(Value::as_str) {
            Some(destination) if has_formula_substitution(destination) => {
                check_formula_text(destination, &format!("{path}.destination"), out);
            }
            Some(destination) if ctx.pages.contains(destination) => {}
            Some(destination) => out.push(Problem::error(
                format!("{path}.destination"),
                format!("La page `{destination}` n'existe pas"),
            )),
            None => out.push(Problem::error(
                format!("{path}.destination"),
                "Destination manquante",
            )),
        },
        "definir_pv" | "definir_pv_max" | "modifier_pv_max" | "definir_moral"
        | "definir_moral_max" | "modifier_moral" | "modifier_moral_max" => {
            check_formula_value(
                effect.get("valeur"),
                Some("int"),
                false,
                &format!("{path}.valeur"),
                out,
            );
            validate_effect_defeat(effect, path, ctx, out);
        }
        _ => {}
    }
}

/// Validates one conditional expression recursively.
fn validate_conditional(value: &Value, path: &str, ctx: &Context<'_>, out: &mut Vec<Problem>) {
    if let Some(array) = value.as_array() {
        for (index, child) in array.iter().enumerate() {
            validate_conditional(child, &format!("{path}[{index}]"), ctx, out);
        }
        return;
    }
    let Some(object) = value.as_object() else {
        out.push(Problem::error(
            path,
            "Une condition doit être un objet ou un tableau",
        ));
        return;
    };
    let Some(kind) = object.get("type").and_then(Value::as_str) else {
        out.push(Problem::error(
            format!("{path}.type"),
            "Type de condition manquant",
        ));
        return;
    };
    if !CONDITIONAL_TYPES.iter().any(|(id, _)| *id == kind) {
        out.push(Problem::warning(
            format!("{path}.type"),
            format!("Type de condition inconnu: `{kind}`; il sera conservé tel quel"),
        ));
        return;
    }
    match kind {
        "et" | "ou" => match object.get("conditions") {
            Some(Value::Array(children)) => {
                if children.is_empty() {
                    out.push(Problem::warning(
                        format!("{path}.conditions"),
                        "La liste de conditions est vide",
                    ));
                }
                for (index, child) in children.iter().enumerate() {
                    validate_conditional(child, &format!("{path}.conditions[{index}]"), ctx, out);
                }
            }
            _ => out.push(Problem::error(
                format!("{path}.conditions"),
                "`conditions` doit être un tableau",
            )),
        },
        "non" => match object.get("condition") {
            Some(child) => validate_conditional(child, &format!("{path}.condition"), ctx, out),
            None => out.push(Problem::error(
                format!("{path}.condition"),
                "Condition imbriquée manquante",
            )),
        },
        "objet_requis" => require_filter(object, path, FilterFamily::Item, out),
        "tag_requis" => require_filter(object, path, FilterFamily::Tag, out),
        "etat_requis" => require_filter(object, path, FilterFamily::Condition, out),
        "attribut_egal" | "attribut_superieur" | "attribut_inferieur" => {
            match object.get("attribut").and_then(Value::as_str) {
                Some(attribute) if ctx.attributes.contains(attribute) => {}
                Some(attribute) => out.push(Problem::error(
                    format!("{path}.attribut"),
                    format!("L'attribut `{attribute}` n'existe pas"),
                )),
                None => out.push(Problem::error(
                    format!("{path}.attribut"),
                    "Attribut manquant",
                )),
            }
            check_formula_value(
                object.get("valeur"),
                Some("int"),
                false,
                &format!("{path}.valeur"),
                out,
            );
        }
        "pv_egaux" | "pv_superieurs" | "pv_inferieurs" | "moral_egal" | "moral_superieur"
        | "moral_inferieur" => {
            check_formula_value(
                object.get("valeur"),
                Some("int"),
                false,
                &format!("{path}.valeur"),
                out,
            );
        }
        "tag_page" => require_filter(object, path, FilterFamily::Tag, out),
        "aleatoire" => check_formula_value(
            object.get("poids"),
            None,
            false,
            &format!("{path}.poids"),
            out,
        ),
        "formule" => match object.get("formule").and_then(Value::as_str) {
            Some(formula) => check_formula_text(formula, &format!("{path}.formule"), out),
            None => out.push(Problem::error(
                format!("{path}.formule"),
                "Formule manquante",
            )),
        },
        _ => {}
    }
}

/// Validates one filter expression for the requested family.
fn validate_filter(value: &Value, path: &str, family: FilterFamily, out: &mut Vec<Problem>) {
    if let Some(id) = value.as_str() {
        if family == FilterFamily::Check {
            out.push(Problem::error(
                path,
                "Un filtre de test ne peut pas être une chaîne",
            ));
        } else if has_formula_substitution(id) {
            check_formula_text(id, path, out);
        }
        return;
    }
    if let Some(array) = value.as_array() {
        for (index, child) in array.iter().enumerate() {
            validate_filter(child, &format!("{path}[{index}]"), family, out);
        }
        return;
    }
    let Some(object) = value.as_object() else {
        out.push(Problem::error(
            path,
            format!(
                "Un {} doit être un objet, un tableau ou un id",
                family.title()
            ),
        ));
        return;
    };
    let Some(raw_kind) = object.get("type").and_then(Value::as_str) else {
        out.push(Problem::error(
            format!("{path}.type"),
            "Type de filtre manquant",
        ));
        return;
    };
    let kind = match (family, raw_kind) {
        (FilterFamily::Condition, "egale") => "donnee_egale",
        (FilterFamily::Condition, "superieure") => "donnee_superieure",
        (FilterFamily::Condition, "inferieure") => "donnee_inferieure",
        (FilterFamily::Tag, "egale") => "valeur_egale",
        (FilterFamily::Tag, "superieure") => "valeur_superieure",
        (FilterFamily::Tag, "inferieure") => "valeur_inferieure",
        (_, "tous") => "tous_les_traits",
        (_, "un") => "un_des_traits",
        _ => raw_kind,
    };
    if !family.types().iter().any(|(id, _)| *id == kind) {
        out.push(Problem::warning(
            format!("{path}.type"),
            format!("Type inconnu pour ce filtre: `{raw_kind}`; il sera conservé tel quel"),
        ));
        return;
    }
    match kind {
        "et" | "ou" => match object.get("filtres") {
            Some(Value::Array(children)) => {
                for (index, child) in children.iter().enumerate() {
                    validate_filter(child, &format!("{path}.filtres[{index}]"), family, out);
                }
            }
            _ => out.push(Problem::error(
                format!("{path}.filtres"),
                "`filtres` doit être un tableau",
            )),
        },
        "non" => match object.get("filtre") {
            Some(child) => validate_filter(child, &format!("{path}.filtre"), family, out),
            None => out.push(Problem::error(
                format!("{path}.filtre"),
                "Filtre imbriqué manquant",
            )),
        },
        "id" => {
            let key = family.id_key().unwrap_or("id");
            require_string(object, key, path, out);
            if let Some(id) = object.get(key).and_then(Value::as_str) {
                if has_formula_substitution(id) {
                    check_formula_text(id, &format!("{path}.{key}"), out);
                }
            }
        }
        "donnee_egale" | "donnee_superieure" | "donnee_inferieure" => {
            require_string(object, "cle", path, out);
            check_formula_value(
                object.get("valeur"),
                None,
                false,
                &format!("{path}.valeur"),
                out,
            );
        }
        "quantite_egale" | "quantite_superieure" | "quantite_inferieure" => {
            check_formula_value(
                object.get("quantite"),
                Some("int"),
                false,
                &format!("{path}.quantite"),
                out,
            );
        }
        "valeur_egale" | "valeur_superieure" | "valeur_inferieure" => {
            check_formula_value(
                object.get("valeur"),
                None,
                false,
                &format!("{path}.valeur"),
                out,
            );
        }
        "attribut_parmi" => {
            let value = object.get("attribut");
            match value {
                Some(Value::String(_)) | Some(Value::Array(_)) => {}
                _ => out.push(Problem::error(
                    format!("{path}.attribut"),
                    "`attribut` doit être une chaîne ou un tableau",
                )),
            }
        }
        "tous_les_traits" | "un_des_traits" => {
            check_string_list(object.get("traits"), &format!("{path}.traits"), true, out);
        }
        "formule" => match object.get("formule").and_then(Value::as_str) {
            Some(formula) => check_formula_text(formula, &format!("{path}.formule"), out),
            None => out.push(Problem::error(
                format!("{path}.formule"),
                "Formule manquante",
            )),
        },
        _ => {}
    }
}

/// Finds unreachable pages and other page-graph validation problems.
fn graph_problems(root: &Value, out: &mut Vec<Problem>) {
    let graph = graph::analyze(root);
    for node in &graph.nodes {
        if node.depth.is_none() {
            out.push(Problem::warning(
                format!("pages.{}", node.id),
                "Cette page n'est pas atteignable depuis `depart` selon l'analyse conservatrice",
            ));
        }
        if node.outgoing == 0 {
            out.push(Problem::info(
                format!("pages.{}", node.id),
                "Page terminale: aucune connexion ou redirection locale",
            ));
        }
    }
}

/// Validates that an effect or conditional contains a filter of the requested family.
fn require_filter(
    object: &Map<String, Value>,
    path: &str,
    family: FilterFamily,
    out: &mut Vec<Problem>,
) {
    match object.get("filtre") {
        Some(filter) => validate_filter(filter, &format!("{path}.filtre"), family, out),
        None => out.push(Problem::error(
            format!("{path}.filtre"),
            format!("{} manquant", family.title()),
        )),
    }
}

/// Validates the optional `selection` mode of an effect.
fn validate_selection(object: &Map<String, Value>, path: &str, out: &mut Vec<Problem>) {
    if let Some(selection) = object.get("selection") {
        match selection.as_str() {
            Some("tous" | "premier") => {}
            Some(other) => out.push(Problem::error(
                format!("{path}.selection"),
                format!("Sélection inconnue: `{other}`"),
            )),
            None => out.push(Problem::error(
                format!("{path}.selection"),
                "La sélection doit être `tous` ou `premier`",
            )),
        }
    }
}

/// Validates an optional defeat destination on a resource effect.
fn validate_effect_defeat(
    object: &Map<String, Value>,
    path: &str,
    ctx: &Context<'_>,
    out: &mut Vec<Problem>,
) {
    if let Some(value) = object.get("defaite") {
        validate_optional_page_ref(value, &format!("{path}.defaite"), ctx, out);
    }
}

/// Validates a poll primitive data-type declaration.
fn validate_poll_data_type(value: &Value, path: &str, out: &mut Vec<Problem>) {
    match value.as_str() {
        Some("chaine" | "entier" | "flottant" | "booleen" | "str" | "int" | "bool" | "float") => {}
        Some(other) => out.push(Problem::error(
            path,
            format!("Type de données inconnu: `{other}`"),
        )),
        None => out.push(Problem::error(
            path,
            "Le type de données doit être une chaîne",
        )),
    }
}

/// Validates every choice in a poll choice array.
fn validate_poll_choices(value: &Value, dtype: &str, path: &str, out: &mut Vec<Problem>) {
    if value.is_null() {
        return;
    }
    if value.is_string() {
        validate_deferred_formula(value, path, "liste de choix", out);
        return;
    }
    let Some(array) = value.as_array() else {
        out.push(Problem::error(
            path,
            "Les choix doivent être un tableau, une formule ou null",
        ));
        return;
    };
    for (index, choice) in array.iter().enumerate() {
        validate_poll_choice(choice, dtype, &format!("{path}[{index}]"), out);
    }
}

/// Validates one poll choice against the declared answer type.
fn validate_poll_choice(choice: &Value, dtype: &str, path: &str, out: &mut Vec<Problem>) {
    let Some(object) = choice.as_object() else {
        out.push(Problem::error(path, "Un choix doit être un objet"));
        return;
    };
    require_string(object, "texte", &format!("{path}.texte"), out);
    if let Some(text) = object.get("texte").and_then(Value::as_str) {
        check_formula_text(text, &format!("{path}.texte"), out);
    }
    match required_value(object, "valeur", &format!("{path}.valeur")) {
        Err(err) => {
            out.push(Problem::error(
                format!("{path}.valeur"),
                format!("Valeur manquante: {}", err),
            ));
        }
        Ok(value) => {
            if !match_dtype(&value, dtype, false) {
                out.push(Problem::error(
                    format!("{path}.valeur"),
                    format!("Type de valeur incorrect: attendu `{dtype}`"),
                ));
            }
        }
    }
}

/// Validates optional page ref or a formula
fn validate_optional_page_ref(
    value: &Value,
    path: &str,
    ctx: &Context<'_>,
    out: &mut Vec<Problem>,
) {
    if value.is_null() {
        return;
    }
    match value.as_str() {
        Some(id) if has_formula_substitution(id) => {}
        Some(id) if ctx.pages.contains(id) => {}
        Some(id) => out.push(Problem::error(path, format!("La page `{id}` n'existe pas"))),
        None => out.push(Problem::error(
            path,
            "La référence doit être une chaîne ou null",
        )),
    }
}

/// Validates a required static reference to a known tale entity.
fn validate_named_ref(
    object: &Map<String, Value>,
    key: &str,
    known: &HashSet<&str>,
    path: &str,
    label: &str,
    out: &mut Vec<Problem>,
) {
    match object.get(key).and_then(Value::as_str) {
        Some(id) if has_formula_substitution(id) => {
            check_formula_text(id, &format!("{path}.{key}"), out);
        }
        Some(id) if known.contains(id) => {}
        Some(id) => out.push(Problem::error(
            format!("{path}.{key}"),
            format!("L'{label} `{id}` n'existe pas"),
        )),
        None => out.push(Problem::error(
            format!("{path}.{key}"),
            format!("Identifiant d'{label} manquant"),
        )),
    }
}

/// Validates an optional string field.
fn check_optional_string(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    out: &mut Vec<Problem>,
) {
    if object
        .get(key)
        .is_some_and(|value| !value.is_string() && !value.is_null())
    {
        out.push(Problem::error(
            path,
            format!("`{key}` doit être une chaîne"),
        ));
    }
}

/// Requires a textual field and records a contextual validation error when absent.
fn require_string(object: &Map<String, Value>, key: &str, path: &str, out: &mut Vec<Problem>) {
    if !object.get(key).is_some_and(Value::is_string) {
        out.push(Problem::error(
            format!("{path}.{key}"),
            format!("Le champ `{key}` doit être une chaîne"),
        ));
    }
}

/// Requires a non-empty textual field and reports its own exact field path.
fn require_nonempty_string(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    out: &mut Vec<Problem>,
) {
    match object.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => {}
        Some(_) => out.push(Problem::error(
            format!("{path}.{key}"),
            format!("Le champ `{key}` ne peut pas être vide"),
        )),
        None => out.push(Problem::error(
            format!("{path}.{key}"),
            format!("Le champ `{key}` doit être une chaîne"),
        )),
    }
}

/// Validates an optional object field.
fn check_optional_object(value: Option<&Value>, path: &str, out: &mut Vec<Problem>) {
    if value.is_some_and(|value| !value.is_object() && !value.is_null()) {
        out.push(Problem::error(path, "Ce champ doit être un objet"));
    }
}

/// Validates an optional dictionary whose complete value may be produced by a formula.
fn validate_object_or_formula(value: Option<&Value>, path: &str, out: &mut Vec<Problem>) {
    let Some(value) = value else { return };
    match value {
        Value::Null | Value::Object(_) => {}
        Value::String(_) => {
            validate_deferred_formula(value, path, "dictionnaire", out);
        }
        _ => out.push(Problem::error(
            path,
            "Ce champ doit être un dictionnaire, une formule complète ou null",
        )),
    }
}

/// Validates a complete `${...}` formula used as one deferred structured value.
fn validate_deferred_formula(value: &Value, path: &str, label: &str, out: &mut Vec<Problem>) -> bool {
    let Some(source) = value.as_str() else {
        return false;
    };
    if full_formula(source).is_none() {
        out.push(Problem::error(
            path,
            format!("La valeur différée « {label} » doit être une formule complète `${{...}}`"),
        ));
        return false;
    }
    if let Err(error) = validate_formula_syntax(source) {
        out.push(Problem::error(
            path,
            format!("Formule de {label} invalide: {error:#}"),
        ));
        return false;
    }
    true
}

/// Validates the optional nested path used by story-tag mutation effects.
fn validate_tag_path(effect: &Map<String, Value>, path: &str, out: &mut Vec<Problem>) {
    let path_field = format!("{path}.chemin");
    match effect.get("chemin") {
        None => {
            if effect.contains_key("creer_chemin") {
                out.push(Problem::warning(
                    format!("{path}.creer_chemin"),
                    "`creer_chemin` n'a aucun effet sans `chemin`",
                ));
            }
        }
        Some(Value::Array(values)) => {
            if values.is_empty() {
                out.push(Problem::error(
                    path_field.clone(),
                    "Le chemin ne peut pas être vide",
                ));
            }
            for (index, value) in values.iter().enumerate() {
                let item_path = format!("{path_field}[{index}]");
                match value {
                    Value::String(text) => check_formula_text(text, &item_path, out),
                    Value::Number(number) if number.as_i64().is_some_and(|value| value >= 0) => {}
                    Value::Number(_) => out.push(Problem::error(
                        item_path,
                        "Un index de chemin doit être un entier positif ou nul",
                    )),
                    _ => out.push(Problem::error(
                        item_path,
                        "Une entrée de chemin doit être une chaîne ou un entier positif ou nul",
                    )),
                }
            }
        }
        Some(value) if is_formula(value) => {
            if let Some(text) = value.as_str() {
                check_formula_text(text, &path_field, out);
            }
        }
        Some(_) => out.push(Problem::error(
            path_field.clone(),
            "Le chemin doit être une liste ou une formule retournant une liste",
        )),
    }
    if effect
        .get("creer_chemin")
        .is_some_and(|value| !value.is_boolean())
    {
        out.push(Problem::error(
            format!("{path}.creer_chemin"),
            "`creer_chemin` doit être un booléen",
        ));
    }
}

/// Validates a string list or permitted single-string shorthand.
fn check_string_list(
    value: Option<&Value>,
    path: &str,
    string_allowed: bool,
    out: &mut Vec<Problem>,
) {
    let Some(value) = value else { return };
    if string_allowed && value.is_string() {
        return;
    }
    let Some(array) = value.as_array() else {
        out.push(Problem::error(
            path,
            "Ce champ doit être un tableau de chaînes",
        ));
        return;
    };
    for (index, item) in array.iter().enumerate() {
        if !item.is_string() {
            out.push(Problem::error(
                format!("{path}[{index}]"),
                "Cette valeur doit être une chaîne",
            ));
        }
    }
}

/// Validates a literal-or-formula field and its optional declared result type.
fn check_formula_value(
    value: Option<&Value>,
    dtype: Option<&str>,
    nullable: bool,
    path: &str,
    out: &mut Vec<Problem>,
) {
    let Some(value) = value else { return };
    match value {
        Value::String(text) => check_formula_text(text, path, out),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {}
    }
    if let Some(dtype) = dtype {
        if !match_dtype(value, dtype, nullable) {
            out.push(Problem::error(
                path,
                format!("Type de valeur incorrect: {value} n'est pas un {dtype}"),
            ));
        }
    }
}

// TODO meilleur version qui vérifie que chaque `${...}` est bien fermée, et que les accolades ne sont pas imbriquées
/// Validates every formula substitution embedded in text.
fn check_formula_text(text: &str, path: &str, out: &mut Vec<Problem>) {
    let opens = text.matches("${").count();
    let closes = text.matches('}').count();
    if opens > closes {
        out.push(Problem::warning(
            path,
            "Une formule `${...}` semble incomplète",
        ));
    }
}

/// Counts validation problems by severity.
pub fn problem_counts(problems: &[Problem]) -> (usize, usize, usize) {
    let mut counts = HashMap::new();
    for problem in problems {
        *counts.entry(problem.severity).or_insert(0usize) += 1;
    }
    (
        *counts.get(&Severity::Error).unwrap_or(&0),
        *counts.get(&Severity::Warning).unwrap_or(&0),
        *counts.get(&Severity::Info).unwrap_or(&0),
    )
}

/// Collects story-tag IDs referenced throughout the raw tale.
pub fn referenced_tags(root: &Value) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    collect_tags(root, &mut tags);
    tags
}

/// Recursively collects tag references from a JSON value.
///
/// Traverses supported static structures and appends matches to the supplied collection;
/// references generated only by formulas cannot be enumerated.
fn collect_tags(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Array(values) => values.iter().for_each(|value| collect_tags(value, out)),
        Value::Object(object) => {
            if let Some(filter) = object.get("filtre") {
                if let Some(id) = single_id_filter(filter, "tag") {
                    let effect_type = object.get("type").and_then(Value::as_str);
                    if matches!(
                        effect_type,
                        Some("definir_tag" | "incrementer_tag" | "tag_requis")
                    ) {
                        out.insert(id.to_owned());
                    }
                }
            }
            object.values().for_each(|value| collect_tags(value, out));
        }
        _ => {}
    }
}

/// Checks if a value matches the expected data type, considering nullability and formulas.
fn match_dtype(value: &Value, dtype: &str, nullable: bool) -> bool {
    match dtype {
        "str" | "chaine" => value.is_string() || (nullable && value.is_null()),
        "int" | "entier" => value.is_i64() || is_formula(value) || (nullable && value.is_null()),
        "float" | "flottant" => {
            value.is_f64() || is_formula(value) || (nullable && value.is_null())
        }
        "bool" | "booleen" => {
            value.is_boolean() || is_formula(value) || (nullable && value.is_null())
        }
        "list" | "liste" => value.is_array() || is_formula(value) || (nullable && value.is_null()),
        "dict" | "objet" => value.is_object() || is_formula(value) || (nullable && value.is_null()),
        _ => false,
    }
}

/// Returns whether a JSON value is exactly one `${...}` formula string.
pub fn is_formula(value: &Value) -> bool {
    value
        .as_str()
        .map(|s| full_formula(s).is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{validate_tale, Severity};
    use serde_json::{json, Value};

    fn tale_with_images(images: Value, title_images: Value, cell_images: Value) -> Value {
        json!({
            "version": 2,
            "titre": "Images",
            "depart": "debut",
            "images": images,
            "image": title_images,
            "pages": {
                "debut": {
                    "cellules": [{"images": cell_images}],
                    "connexions": []
                }
            }
        })
    }

    fn has_error_at(problems: &[super::Problem], path: &str) -> bool {
        problems
            .iter()
            .any(|problem| problem.severity == Severity::Error && problem.path == path)
    }

    #[test]
    fn accepts_all_image_sources_and_deferred_cell_references() {
        let tale = tale_with_images(
            json!({
                "plan": {"type": "fichier", "chemin": "images/plan.png", "alt": "Plan"},
                "vue": {"type": "url", "url": "https://example.test/vue.webp"},
                "sceau": {"type": "base64", "donnees": "AA==", "format": "png"}
            }),
            json!(["plan", "vue"]),
            json!(["sceau", "${\"plan\"}", null]),
        );
        let problems = validate_tale(&tale);
        assert!(
            !problems.iter().any(|problem| problem.severity == Severity::Error),
            "unexpected errors: {problems:#?}"
        );
    }

    #[test]
    fn reports_invalid_definitions_and_dangling_references_at_exact_paths() {
        let tale = tale_with_images(
            json!({
                "local": {"type": "fichier", "chemin": ""},
                "remote": {"type": "url", "url": "ftp://example.test/vue.png"},
                "embedded": {"type": "base64", "donnees": "AA==", "format": "gif"},
                "broken_data": {"type": "base64", "donnees": "pas du base64", "format": "png"}
            }),
            json!(["absente", null]),
            json!(["local", "inconnue", 12]),
        );
        let problems = validate_tale(&tale);
        for path in [
            "images.local.chemin",
            "images.remote.url",
            "images.embedded.format",
            "images.broken_data.donnees",
            "image[0]",
            "image[1]",
            "pages.debut.cellules[0].images[1]",
            "pages.debut.cellules[0].images[2]",
        ] {
            assert!(has_error_at(&problems, path), "missing error at {path}: {problems:#?}");
        }
    }

    #[test]
    fn rejects_formulas_for_title_images_but_allows_them_in_cells() {
        let tale = tale_with_images(
            json!({"plan": {"type": "fichier", "chemin": "plan.png"}}),
            json!("${\"plan\"}"),
            json!("${\"plan\"}"),
        );
        let problems = validate_tale(&tale);
        assert!(has_error_at(&problems, "image"));
        assert!(!has_error_at(&problems, "pages.debut.cellules[0].images"));
    }
}
