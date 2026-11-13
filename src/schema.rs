//! Static schema catalog used by Forge menus, summaries, and validation.
//!
//! This module describes author-facing JSON shapes but does not parse or execute formulas.
//! Summary helpers tolerate incomplete values because Forge invokes them during editing.
use serde_json::{json, Value};

/// Page-effect types offered by the Forge guided builder.
pub const EFFECT_TYPES: &[(&str, &str)] = &[
    ("ajouter_etat", "Ajouter un état"),
    ("retirer_etat", "Retirer des états"),
    ("modifier_etat", "Modifier des états"),
    ("definir_tag", "Définir des tags"),
    ("incrementer_tag", "Incrémenter des tags"),
    ("ajouter_objet", "Ajouter un objet"),
    ("retirer_objet", "Retirer des objets"),
    ("modifier_objet", "Modifier des objets"),
    ("modifier_pv", "Modifier les PV"),
    ("modifier_attribut", "Modifier un attribut"),
    ("definir_attribut", "Définir un attribut"),
    ("definir_pv", "Définir les PV"),
    ("definir_pv_max", "Définir les PV maximaux"),
    ("modifier_pv_max", "Modifier les PV maximaux"),
    ("definir_moral", "Définir le moral"),
    ("definir_moral_max", "Définir le moral maximal"),
    ("modifier_moral", "Modifier le moral"),
    ("modifier_moral_max", "Modifier le moral maximal"),
    ("modifier_nom", "Modifier le nom du personnage"),
    ("retirer_tag", "Retirer des tags"),
    ("demander", "Demander une question au joueur"),
    ("rediriger", "Rediriger vers une autre page"),
];

/// Conditional types offered by the Forge guided builder.
pub const CONDITIONAL_TYPES: &[(&str, &str)] = &[
    ("et", "Toutes les conditions"),
    ("ou", "Au moins une condition"),
    ("non", "Négation"),
    ("objet_requis", "Objet requis"),
    ("tag_requis", "Tag requis"),
    ("etat_requis", "État requis"),
    ("attribut_egal", "Attribut égal"),
    ("attribut_superieur", "Attribut supérieur"),
    ("attribut_inferieur", "Attribut inférieur"),
    ("pv_egaux", "PV égaux"),
    ("pv_superieurs", "PV supérieurs"),
    ("pv_inferieurs", "PV inférieurs"),
    ("moral_egal", "Moral égal"),
    ("moral_superieur", "Moral supérieur"),
    ("moral_inferieur", "Moral inférieur"),
    ("tag_page", "Tag de page"),
    ("aleatoire", "Aléatoire"),
    ("formule", "Formule"),
];

/// Item-filter types offered by the Forge guided builder.
pub const ITEM_FILTER_TYPES: &[(&str, &str)] = &[
    ("et", "Tous les filtres"),
    ("ou", "Un des filtres"),
    ("non", "Négation"),
    ("id", "Identifiant"),
    ("donnee_egale", "Métadonnée égale"),
    ("donnee_superieure", "Métadonnée supérieure"),
    ("donnee_inferieure", "Métadonnée inférieure"),
    ("quantite_egale", "Quantité égale"),
    ("quantite_superieure", "Quantité supérieure"),
    ("quantite_inferieure", "Quantité inférieure"),
    ("tous_les_traits", "Tous les traits"),
    ("un_des_traits", "Un des traits"),
    ("formule", "Formule"),
];

/// Persistent-state filter types offered by the Forge guided builder.
pub const CONDITION_FILTER_TYPES: &[(&str, &str)] = &[
    ("et", "Tous les filtres"),
    ("ou", "Un des filtres"),
    ("non", "Négation"),
    ("id", "Identifiant"),
    ("donnee_egale", "Métadonnée égale"),
    ("donnee_superieure", "Métadonnée supérieure"),
    ("donnee_inferieure", "Métadonnée inférieure"),
    ("tous_les_traits", "Tous les traits"),
    ("un_des_traits", "Un des traits"),
    ("formule", "Formule"),
];

/// Story-tag filter types offered by the Forge guided builder.
pub const TAG_FILTER_TYPES: &[(&str, &str)] = &[
    ("et", "Tous les filtres"),
    ("ou", "Un des filtres"),
    ("non", "Négation"),
    ("id", "Identifiant"),
    ("valeur_egale", "Valeur égale"),
    ("valeur_superieure", "Valeur supérieure"),
    ("valeur_inferieure", "Valeur inférieure"),
    ("formule", "Formule"),
];

/// Check-filter types offered by the Forge guided builder.
pub const CHECK_FILTER_TYPES: &[(&str, &str)] = &[
    ("et", "Tous les filtres"),
    ("ou", "Un des filtres"),
    ("non", "Négation"),
    ("attribut_parmi", "Attribut parmi"),
    ("tous_les_traits", "Tous les traits"),
    ("un_des_traits", "Un des traits"),
    ("formule", "Formule"),
];

/// Persistent-state modifier types offered by the Forge guided builder.
pub const CONDITION_EFFECT_TYPES: &[(&str, &str)] = &[
    ("modificateur_attribut", "Modificateur d'attribut"),
    ("modificateur_test", "Modificateur de test"),
    ("modificateur_pv_max", "Modificateur de PV maximaux"),
    ("modificateur_moral_max", "Modificateur de moral maximal"),
];

/// Default character attributes inserted into newly created tales.
pub const DEFAULT_ATTRIBUTES: &[&str] = &[
    "force",
    "agilite",
    "endurance",
    "presence",
    "chance",
    "connaissance",
    "perception",
];

/// Attribute names reserved by the formula environment and runtime model.
pub const RESERVED_ATTRIBUTES: &[&str] = &[
    "personnage",
    "objet",
    "etat",
    "condition",
    "tag",
    "valeur",
    "ancienne",
    "traits",
    "page",
    "min",
    "max",
    "dice",
    "d",
    "int",
    "float",
    "round",
    "abs",
    "pow",
    "str",
    "bool",
    "len",
    "args",
    "nom",
    "pv",
    "max_pv",
    "moral",
    "max_moral",
    "test",
    "type",
    "recit",
];

/// Identifies the schema family used when building a guided filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterFamily {
    Item,
    Condition,
    Tag,
    Check,
}

impl FilterFamily {
    /// Returns the human-readable name of this filter family.
    pub fn title(self) -> &'static str {
        match self {
            Self::Item => "filtre d'objet",
            Self::Condition => "filtre d'état",
            Self::Tag => "filtre de tag",
            Self::Check => "filtre de test",
        }
    }

    /// Returns the effect-filter types supported by this filter family.
    pub fn types(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Item => ITEM_FILTER_TYPES,
            Self::Condition => CONDITION_FILTER_TYPES,
            Self::Tag => TAG_FILTER_TYPES,
            Self::Check => CHECK_FILTER_TYPES,
        }
    }
    /// Returns the field name used by direct-ID filters in this family.
    ///
    /// Families without direct-ID filters return `None`.
    pub fn id_key(self) -> Option<&'static str> {
        match self {
            Self::Item | Self::Condition => Some("id"),
            Self::Tag => Some("tag"),
            Self::Check => None,
        }
    }
}
/// Creates the minimal editable JSON object for a new item definition.
pub fn new_item_definition() -> Value {
    json!({
        "nom": "Nouvel objet",
        "description": "",
        "traits": [],
        "donnees": {}
    })
}
/// Creates the minimal editable JSON object for a new persistent-state definition.
pub fn new_condition_definition() -> Value {
    json!({
        "nom": "Nouvel état",
        "description": "",
        "donnees": {},
        "traits": [],
        "effets": [],
        "avant": [],
        "apres": [],
        "expiration": null,
        "defaite": null
    })
}

/// Returns a compact human-readable summary of an atomic effect.
pub fn effect_summary(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return "Effet invalide".into();
    };
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("type manquant");
    match kind {
        "ajouter_etat" => format!("Ajouter état {}", string_field(object, "etat")),
        "retirer_etat" => "Retirer état(s)".into(),
        "modifier_etat" => "Modifier état(s)".into(),
        "definir_tag" => format!(
            "Définir tag(s) {}{}",
            filter_brief(object.get("filtre")),
            path_brief(object.get("chemin"))
        ),
        "incrementer_tag" => {
            format!(
                "Incrémenter tag(s) {}{}",
                filter_brief(object.get("filtre")),
                path_brief(object.get("chemin"))
            )
        }
        "ajouter_objet" => format!("Ajouter objet {}", string_field(object, "objet")),
        "retirer_objet" => "Retirer objet(s)".into(),
        "modifier_objet" => "Modifier objet(s)".into(),
        "modifier_attribut" | "definir_attribut" => format!(
            "{} {}",
            kind,
            object
                .get("attribut")
                .and_then(Value::as_str)
                .unwrap_or("?")
        ),
        _ => kind.replace('_', " "),
    }
}

/// Returns a compact suffix for an optional nested tag path.
fn path_brief(value: Option<&Value>) -> String {
    value
        .map(|value| format!(" chemin={}", compact_json(value)))
        .unwrap_or_default()
}

/// Serializes one small JSON value for summaries without failing the editor.
fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "?".into())
}
/// Returns the Forge list label for a cell.
///
/// The cell's explicit label is preferred; otherwise a short summary is derived from its text or effects,
/// and `index` is used as a stable fallback.
pub fn cell_summary(value: &Value, index: usize) -> String {
    let Some(object) = value.as_object() else {
        return format!("Cellule {} invalide", index + 1);
    };
    let identity = object
        .get("libelle")
        .and_then(Value::as_str)
        .or_else(|| object.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Cellule {}", index + 1));
    let effects = object
        .get("effets")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let images = match object.get("images") {
        None | Some(Value::Null) => 0,
        Some(Value::Array(images)) => images.iter().filter(|image| !image.is_null()).count(),
        Some(_) => 1,
    };
    let has_text = object
        .get("texte")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty());
    let guarded = object.get("conditions").is_some();
    format!(
        "{identity} · {}{}{}{}",
        if has_text { "texte" } else { "sans texte" },
        if effects > 0 {
            format!(" · {effects} effet(s)")
        } else {
            String::new()
        },
        if images > 0 {
            format!(" · {images} image(s)")
        } else {
            String::new()
        },
        if guarded { " · conditionnelle" } else { "" },
    )
}

/// Returns a compact human-readable summary of a conditional expression.
pub fn conditional_summary(value: &Value) -> String {
    if let Some(array) = value.as_array() {
        return format!("ET implicite ({} conditions)", array.len());
    }
    let Some(object) = value.as_object() else {
        return "Condition invalide".into();
    };
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("type manquant");
    match kind {
        "et" | "ou" => format!(
            "{} ({} conditions)",
            kind.to_uppercase(),
            object
                .get("conditions")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        ),
        "non" => "NON (…)".into(),
        "objet_requis" => format!("Objet requis: {}", filter_brief(object.get("filtre"))),
        "tag_requis" => format!("Tag requis: {}", filter_brief(object.get("filtre"))),
        "etat_requis" => format!("État requis: {}", filter_brief(object.get("filtre"))),
        "tag_page" => format!("Tag de page: {}", filter_brief(object.get("filtre"))),
        "formule" => format!("Formule: {}", string_field(object, "formule")),
        "aleatoire" => format!("Aléatoire: {}%", value_field(object.get("poids"))),
        _ => kind.replace('_', " "),
    }
}

/// Returns a compact human-readable summary of a filter.
pub fn filter_brief(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "?".into();
    };
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    if let Some(array) = value.as_array() {
        return format!("{} filtres", array.len());
    }
    let Some(object) = value.as_object() else {
        return value.to_string();
    };
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("?");
    match kind {
        "id" => object
            .get("id")
            .or_else(|| object.get("tag"))
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned(),
        "et" | "ou" => format!(
            "{}({})",
            kind,
            object
                .get("filtres")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        ),
        "non" => "non(…)".into(),
        _ => kind.to_owned(),
    }
}

/// Returns a textual object field or a placeholder when it is missing.
fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_owned()
}

/// Formats an optional JSON value for a compact summary.
fn value_field(value: Option<&Value>) -> String {
    value.map_or_else(|| "?".into(), Value::to_string)
}
