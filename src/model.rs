//! Typed executable tale model shared by the player and model-level validation.
//!
//! Parsing requires syntax version 2 and rejects legacy structural fields. Formula-bearing
//! values remain deferred until their page, connection, check, item, or state is evaluated
//! against a runtime environment.
use crate::character::Character;
use crate::effects::{Cell, ConditionEffect, Deferred, PageEffect};
use crate::filters::{ConditionFilter, Conditional};
use crate::formula::{dict_to_map, json_to_eval_value, mk_character_env, mk_connection_env, mk_partial_condition_env, mk_partial_item_env, mk_tale_env};
use formulas::{
    Env, EvalKey, EvalValue, FormulaEngine, FormulaFunction, Parser, Tokenizer, full_formula, split_fmt_str, validate_formula_syntax, value_or_formula
};
use crate::images::ImageRegistry;
use crate::json_util::{
    array_or_empty, object, object_or_empty, optional_string, required_string, string_list,
    string_or,
};
use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use std::path::Path;
use std::sync::Arc;

/// Default character attributes used when a tale does not define its own list.
pub const DEFAULT_ATTRIBUTES: &[&str] = &[
    "force",
    "agilite",
    "endurance",
    "presence",
    "chance",
    "connaissance",
    "perception",
];

const RESERVED_WORDS: &[&str] = &[
    "personnage",
    "page",
    "objet",
    "etat",
    "condition",
    "tag",
    "valeur",
    "ancienne",
    "traits",
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
    "test",
    "nom",
    "pv",
    "max_pv",
    "moral",
    "max_moral",
];

/// Contains the fully parsed and executable definition of a tale.
#[derive(Clone, Debug)]
pub struct Tale {
    /// Syntax version declared by the source document. Version 2 is required.
    pub version: u64,
    pub title: Arc<str>,
    pub author: Option<Arc<str>>,
    pub synopsis: Arc<str>,
    pub title_images: Arc<[Arc<str>]>,
    pub attributes: Arc<[Arc<str>]>,
    /// Legacy formula declarations retained for syntax-v2 compatibility.
    pub functions: IndexMap<Arc<str>, FormulaFunction>,
    pub items: IndexMap<Arc<str>, Arc<Item>>,
    pub conditions: IndexMap<Arc<str>, Arc<ConditionTemplate>>,
    pub pages: IndexMap<Arc<str>, Page>,
    /// Legacy initial mutable story tags copied into every new character.
    pub initial_tags: IndexMap<Arc<str>, EvalValue>,
    /// Cells executed before the destination page's own cells.
    pub pre: Arc<[Deferred<Cell>]>,
    /// Cells executed after the destination page's own cells.
    pub post: Arc<[Deferred<Cell>]>,
    pub start: Arc<str>,
    pub custom_status: Option<EvalValue>,
    /// Tale constants and function values exposed through the `recit` dictionary.
    pub data: Arc<IndexMap<Arc<str>, EvalValue>>,
    /// Registry for image definitions used in the tale.
    pub images: ImageRegistry,
}

impl Tale {
    /// Parses a raw version-2 tale into the executable typed model.
    ///
    /// Structural values are validated immediately while formula strings remain deferred until runtime.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported syntax versions, legacy fields, malformed definitions, or invalid required values.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        let root = object(value, "Le récit")?;
        let version = root
            .get("version")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| anyhow!("Le récit doit déclarer `version`: 2. Utilisez `recit-v1-vers-v2` pour convertir une syntaxe v1."))?;
        if version != 2 {
            bail!("Version de syntaxe non supportée: {version}. Ce moteur exige la version 2.");
        }

        let title = string_or(root, "titre", "Récit sans titre")?.into();
        let author = optional_string(root, "auteur")?.map(Arc::from);
        let synopsis = string_or(root, "synopsis", "")?.into();
        let start = required_string(root, "depart", "Recit")?.into();
        let attributes = parse_attributes(root.get("attributs"))?;
        // `global` replaces the old `fonctions` and `donnees` roots. The old roots
        // remain readable when `global` is absent so existing syntax-v2 tales can still be
        // opened and migrated in Forge.
        let has_global = root.contains_key("global");
        let functions = if has_global {
            IndexMap::new()
        } else {
            parse_formula_functions(root.get("fonctions"))?
        };
        let pre = Cell::parse_many(root.get("avant"), "Le champ `avant`")?.into();
        let post = Cell::parse_many(root.get("apres"), "Le champ `apres`")?.into();

        let items_data = object_or_empty(root, "objets")?;
        let mut items = IndexMap::new();
        for (id, item_data) in items_data {
            let item = Item::from_json(&id, &item_data)
                .with_context(|| format!("Objet '{id}' invalide"))?;
            items.insert(id.into(), Arc::new(item));
        }

        let conditions_data = object_or_empty(root, "etats")?;
        let mut conditions = IndexMap::new();
        for (id, condition_data) in conditions_data {
            let condition = ConditionTemplate::from_json(&id, &condition_data)
                .with_context(|| format!("État '{id}' invalide"))?;
            conditions.insert(id.into(), condition.into());
        }

        let pages_data = root
            .get("pages")
            .ok_or_else(|| anyhow!("Une histoire doit inclure un champ `pages`"))?;
        let pages_object = object(pages_data, "Le champ `pages`")?;
        let mut pages = IndexMap::new();
        for (id, page_data) in pages_object {
            let page =
                Page::from_json(id, page_data).with_context(|| format!("Page '{id}' invalide"))?;
            pages.insert(id.clone().into(), page);
        }

        let custom_status = optional_string(root, "statut")?
            .map(Arc::from)
            .map(|s| split_fmt_str(&s))
            .transpose()?;

        let data = if has_global {
            parse_global_values(root.get("global"))?
        } else {
            object_or_empty(root, "donnees")?
                .iter()
                .map(|(k, v)| {
                    let value = json_to_eval_value(v)
                        .with_context(|| format!("Conversion de `donnees.{k}`"))?;
                    Ok((k.as_str().into(), value))
                })
                .collect::<Result<IndexMap<Arc<str>, EvalValue>>>()?
        };

        // initial tags do not depends on the presence of "global" because they are used to initialize characters
        let initial_tags = {
            object_or_empty(root, "tags")?
                .iter()
                .map(|(key, value)| {
                    let value = json_to_eval_value(value)
                        .with_context(|| format!("Conversion du tag initial `{key}`"))?;
                    Ok((key.as_str().into(), value))
                })
                .collect::<Result<IndexMap<_, _>>>()?
        };
        let initial_tags = eval_initial_tags(initial_tags, &data)?;

        let images = root.get("images");
        let images = match images {
            Some(img_data) => ImageRegistry::from_json(img_data.clone())?,
            None => ImageRegistry::new(),
        };

        let title_images: Arc<[Arc<str>]> = match root.get("image") {
            Some(JsonValue::String(s)) => {
                Arc::new([Arc::from(s.as_str())])
            },
            Some(JsonValue::Array(arr)) => {
                let mut images: Vec<Arc<str>> = Vec::new();
                for val in arr {
                    match val {
                        JsonValue::String(s) => images.push(Arc::from(s.as_str())),
                        other => {
                            bail!("Les images de titre doivent être une str ou une list de str, reçu: {:?}", other)
                        },
                    }
                }
                Arc::from(images)
            },
            Some(other) => {
                bail!("Les images de titre doivent être une str ou une list de str, reçu: {:?}", other)
            },
            None => Arc::new([]),
        };

        Ok(Self {
            version,
            title,
            author,
            synopsis,
            attributes,
            functions,
            items,
            conditions,
            pages,
            initial_tags,
            pre,
            start,
            post,
            custom_status,
            images,
            data: data.into(),
            title_images,
        })
    }

    pub fn with_root(mut self, root: &Path) -> Self {
        self.images = self.images.with_root(root);
        self
    }

    /// Returns every invalid static reference found in the parsed tale.
    ///
    /// Formula-based destinations and image IDs are skipped because their final values are only
    /// known at runtime. The method does not evaluate formulas or mutate the tale.
    pub fn validate_references(&self) -> Vec<Arc<str>> {
        let mut errors = Vec::new();
        if !self.pages.contains_key(&self.start) {
            errors.push(format!("La page de départ '{}' n'existe pas.", self.start).into());
        }
        for image in self.title_images.iter() {
            if !self.images.contains(image) {
                errors.push(format!("Image de titre '{image}' introuvable dans `images`.").into());
            }
        }

        validate_cell_references(
            &self.pre,
            &self.pages,
            &self.images,
            "Cellules globales `avant`",
            &mut errors,
        );
        validate_cell_references(
            &self.post,
            &self.pages,
            &self.images,
            "Cellules globales `apres`",
            &mut errors,
        );

        for (page_id, page) in &self.pages {
            validate_cell_references(
                &page.cells,
                &self.pages,
                &self.images,
                &format!("Page '{page_id}'"),
                &mut errors,
            );
            for connection in page.connections.iter().filter_map(|dc| if let Deferred::Literal(conn) = dc {Some(conn)} else {None}) {
                match &connection.destination {
                    EvalValue::Func(_) => {},
                    EvalValue::String(s) => {
                        if !self.pages.contains_key(s) {
                            errors.push(
                                format!(
                                    "Page '{page_id}': page de destination '{}' introuvable.",
                                    s
                                )
                                .into(),
                            );
                        }
                    },
                    autre => {
                        errors.push(
                            format!(
                                "Page '{page_id}': page de destination doit être une chaîne ou une formule (reçu {autre})."
                            )
                            .into(),
                        );
                    }
                }
                validate_cell_references(
                    &connection.cells,
                    &self.pages,
                    &self.images,
                    &format!("Connexion de la page '{page_id}'"),
                    &mut errors,
                );
                if let Some(check) = &connection.check {
                    if let Some(failure) = &check.failure {
                        match failure {
                            EvalValue::Func(_) => {},
                            EvalValue::String(s) => {
                                if !self.pages.contains_key(s) {
                                    errors.push(
                                        format!(
                                            "Page '{page_id}': page de test '{}' introuvable.",
                                            s
                                        )
                                        .into(),
                                    );
                                }
                            },
                            autre => {
                                errors.push(
                                    format!(
                                        "Page '{page_id}': page de test doit être une chaîne ou une formule (reçu {autre})."
                                    )
                                    .into(),
                                );
                            }
                        }
                    }
                    match &check.attribute {
                        EvalValue::Func(_) => {},
                        EvalValue::String(s) => {
                            if !self.attributes.contains(&s) {
                                errors.push(
                                    format!(
                                        "Page '{page_id}': attribut de test '{}' inconnu.",
                                        s
                                    )
                                    .into(),
                                );
                            }
                        },
                        other => {
                            errors.push(
                                format!(
                                    "Page '{page_id}': attribut de test doit être une chaîne ou une formule (reçu {other})."
                                )
                                .into(),
                            );
                        },
                    }
                }
            }
        }

        for (condition_id, condition) in &self.conditions {
            if let Some(death) = &condition.death {
                match death {
                    EvalValue::Func(_) => {},
                    EvalValue::String(s) => {
                        if !self.pages.contains_key(s) {
                            errors.push(
                                format!("État '{condition_id}': page de défaite '{s}' introuvable.").into(),
                            );
                        }
                    },
                    autre => {
                        errors.push(
                            format!("État '{condition_id}': page de défaite doit être une chaîne ou une formule (reçu {autre}).").into(),
                        );
                    }
                }
            }
            validate_cell_references(
                &condition.pre,
                &self.pages,
                &self.images,
                &format!("État '{condition_id}' `avant`"),
                &mut errors,
            );
            validate_cell_references(
                &condition.post,
                &self.pages,
                &self.images,
                &format!("État '{condition_id}' `apres`"),
                &mut errors,
            );
        }
        errors
    }

    pub fn env(&self) -> Env<'static> {
        let mut env = Env::new();
        let data: IndexMap<EvalKey, EvalValue> = self.data.iter()
            .map(|(k, v)| (EvalKey(EvalValue::String(k.clone().into())), v.clone()))
            .collect();
        env.insert("recit", EvalValue::Object(data.into()));
        env
    }
}

/// Parses and evaluates the tale's immutable `global` dictionary exactly once.
///
/// All raw values are initially visible by their direct names and through `recit`. Entries
/// are then resolved in insertion order; after each entry, both views observe its resolved
/// value. Lists and dictionaries are traversed once so nested function literals and formulas
/// are initialized too. A formula result is never recursively evaluated during this pass.
fn parse_global_values(value: Option<&JsonValue>) -> Result<IndexMap<Arc<str>, EvalValue>> {
    let Some(value) = value else {
        return Ok(IndexMap::new());
    };
    let object = object(value, "Le champ `global`")?;
    let raw = object
        .iter()
        .map(|(key, value)| {
            let value = json_to_eval_value(value)
                .with_context(|| format!("Conversion de la valeur globale `{key}`"))?;
            Ok((Arc::from(key.as_str()), value))
        })
        .collect::<Result<IndexMap<_, _>>>()?;
    let mut values = raw.clone();
    let mut formula = FormulaEngine::new(None);

    for (key, value) in raw.iter() {
        let mut env = mk_tale_env(&values)?;
        let map = values.iter()
            .map(|(k, v)| (EvalKey(EvalValue::String(k.clone().into())), v.clone()))
            .collect::<IndexMap<_, _>>();
        env.insert("recit", EvalValue::Object(Arc::new(map)));
        let resolved = resolve_global_value(value, &mut formula, &env)
            .with_context(|| format!("Valeur globale `{key}` invalide"))?;
        values.insert(key.clone(), resolved);
    }
    Ok(values)
}

/// Resolves every formula-bearing leaf of one global value once.
fn resolve_global_value(
    value: &EvalValue,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<EvalValue> {
    match value {
        EvalValue::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                resolve_global_value(value, formula, env)
                    .with_context(|| format!("Élément [{index}] de la valeur globale"))
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| EvalValue::Array(values.into())),
        EvalValue::Object(values) => {
            let mut resolved = IndexMap::with_capacity(values.len());
            for (key, value) in values.iter() {
                resolved.insert(
                    key.clone(),
                    resolve_global_value(value, formula, env)
                        .with_context(|| format!("Champ `.{key}` de la valeur globale"))?,
                );
            }
            Ok(EvalValue::Object(resolved.into()))
        }
        _ => formula.resolve(value, env).map_err(|_| anyhow!("Echec lors de l'évaluation d'une formule globale")),
    }
}

fn eval_initial_tags(initial_tags: IndexMap<Arc<str>, EvalValue>, data: &IndexMap<Arc<str>, EvalValue>) -> Result<IndexMap<Arc<str>, EvalValue>> {
    let mut formula = FormulaEngine::new(None);
    let mut env = Env::new();
    let map: IndexMap<EvalKey, EvalValue> = data.iter()
        .map(|(k, v)| (EvalKey(EvalValue::String(k.clone().into())), v.clone()))
        .collect();
    env.insert("recit", EvalValue::Object(Arc::new(map)));
    let mut resolved_tags = IndexMap::new();
    for (key, value) in initial_tags {
        let resolved = resolve_initial_tags(&value, &mut formula, &env)
            .with_context(|| format!("Tag initial `{key}` invalide"))?;
        resolved_tags.insert(key, resolved);
    }
    Ok(resolved_tags)
}

fn resolve_initial_tags(value: &EvalValue, formula: &mut FormulaEngine, env: &Env) -> Result<EvalValue> {
    match value {
        EvalValue::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                resolve_initial_tags(value, formula, env)
                    .with_context(|| format!("Élément [{index}] du tag initial"))
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| EvalValue::Array(values.into())),
        EvalValue::Object(values) => {
            let mut resolved = IndexMap::with_capacity(values.len());
            for (key, value) in values.iter() {
                resolved.insert(
                    key.clone(),
                    resolve_initial_tags(value, formula, env)
                        .with_context(|| format!("Champ `.{key}` du tag initial"))?,
                );
            }
            Ok(EvalValue::Object(resolved.into()))
        }
        _ => Ok(formula.resolve(value, env)?),
    }
}

/// Adds errors for missing static image references and redirect destinations.
///
/// Formula destinations and image IDs are intentionally ignored because reference validation
/// cannot resolve them without a runtime environment.
fn validate_cell_references(
    cells: &[Deferred<Cell>],
    pages: &IndexMap<Arc<str>, Page>,
    images: &ImageRegistry,
    context: &str,
    errors: &mut Vec<Arc<str>>,
) {
    for cell in cells.iter().filter_map(|dc| if let Deferred::Literal(cell) = dc {Some(cell)} else {None}) {
        for image in cell.images.iter().filter_map(|image| match image {
            Deferred::Literal(image) => Some(image),
            Deferred::Formula(_) => None,
        }) {
            if !images.contains(image) {
                errors.push(format!("{context}: image '{image}' introuvable dans `images`.").into());
            }
        }
        for effect in cell.effects.iter().filter_map(|dc| match dc {
            Deferred::Literal(lit) => Some(lit),
            _ => None,
        }) {
            if let PageEffect::Redirection { destination, .. } = effect {
                match destination {
                    EvalValue::Func(_) => {},
                    EvalValue::String(s) => {
                        if !pages.contains_key(s) {
                            errors.push(
                                format!("{context}: page de redirection '{s}' introuvable.").into(),
                            );
                        }
                    },
                    autre => {
                        errors.push(
                            format!("{context}: page de redirection doit être une chaîne ou une formule (reçu {autre}).").into(),
                        );
                    }
                }
            }
        }
    }
}

/// Parses tale-defined formula functions from the root `fonctions` dictionary.
///
/// Each key is the callable function name. `parametres` is optional; omitting it makes
/// the function variadic while `args` remains available in every function body.
fn parse_formula_functions(
    value: Option<&JsonValue>,
) -> Result<IndexMap<Arc<str>, FormulaFunction>> {
    let Some(value) = value else {
        return Ok(IndexMap::new());
    };
    let definitions = object(value, "Le champ `fonctions`")?;
    let mut functions = IndexMap::new();
    for (name, definition) in definitions {
        validate_formula_identifier(name, "nom de fonction")?;
        if RESERVED_WORDS.contains(&name.as_str())
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
            bail!("Le nom de fonction `{name}` est réservé");
        }
        let definition = object(definition, &format!("Fonction `{name}`"))?;
        let parameters = match definition.get("parametres") {
            None | Some(JsonValue::Null) => None,
            Some(JsonValue::Array(values)) => {
                let mut parameters = Vec::with_capacity(values.len());
                for (index, value) in values.iter().enumerate() {
                    let parameter = value.as_str().ok_or_else(|| {
                        anyhow!("Fonction `{name}`: parametres[{index}] doit être une chaîne")
                    })?;
                    validate_formula_identifier(parameter, "nom de paramètre")?;
                    if parameter == "args" {
                        bail!("Fonction `{name}`: `args` est réservé à la liste des arguments");
                    }
                    if parameter != "_" && parameters
                        .iter()
                        .any(|existing: &Arc<str>| existing.as_ref() == parameter)
                    {
                        bail!("Fonction `{name}`: paramètre `{parameter}` répété");
                    }
                    parameters.push(parameter.into());
                }
                Some(parameters)
            }
            Some(_) => {
                bail!("Fonction `{name}`: `parametres` doit être un tableau de chaînes ou null")
            }
        };
        let body = required_string(definition, "formule", &format!("Fonction `{name}`"))?;
        let Some(formulastr) = full_formula(&body) else  {
            bail!("Fonction `{name}`: `formule` doit être une formule complète `${{...}}`");
        };
        validate_formula_syntax(&formulastr)
            .with_context(|| format!("Fonction `{name}`: formule invalide"))?;
        let tokens = Tokenizer::new(&formulastr)
            .tokenize()
            .with_context(|| format!("Fonction `{name}`: analyse lexicale impossible"))?;
        let expr = Parser::new(tokens)
            .parse()
            .with_context(|| format!("Fonction `{name}`: construction de l'expression impossible"))?;
        let func = FormulaFunction {
            parameters: parameters.map(|names| names.into_iter().map(Into::into).collect()),
            body: expr.into(),
        };
        functions.insert(
            name.as_str().into(),
            func,
        );
    }
    Ok(functions)
}

/// Checks that a function or parameter name can be tokenized as one formula identifier.
fn validate_formula_identifier(name: &str, label: &str) -> Result<()> {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        bail!("Un {label} ne peut pas être vide");
    };
    if !(first == '_' || first.is_alphabetic())
        || !characters.all(|character| character == '_' || character.is_alphanumeric())
    {
        bail!("`{name}` n'est pas un {label} valide");
    }
    Ok(())
}

/// Parses and deduplicates the tale attribute list.
///
/// Required fields and variant-specific JSON types are checked immediately. Formula
/// strings are preserved for runtime evaluation.
fn parse_attributes(value: Option<&JsonValue>) -> Result<Arc<[Arc<str>]>> {
    let raw = match value {
        None => DEFAULT_ATTRIBUTES
            .iter()
            .map(|attribute| (*attribute).to_owned())
            .collect(),
        Some(value) => string_list(value, "Le champ `attributs`", false)?,
    };

    let mut attributes: Vec<Arc<str>> = Vec::new();
    for attribute in raw {
        if attribute.is_empty() {
            bail!("Les noms d'attributs ne peuvent pas être des chaînes vides.")
        }
        if RESERVED_WORDS.contains(&attribute.as_str()) {
            bail!("Le mot réservé '{attribute}' ne peut pas être utilisé comme nom d'attribut.")
        }
        let rc = attribute.into();
        if !attributes.contains(&rc) {
            attributes.push(rc);
        }
    }
    Ok(attributes.into())
}

/// Stores one categorized numeric bonus or penalty.
#[derive(Clone, Debug)]
pub struct Modifier {
    pub category: Option<Arc<str>>,
    pub value: i64,
}

/// Defines an item template shared by inventory stacks.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: Arc<str>,
    pub name: EvalValue,
    pub description: EvalValue,
    pub traits: Arc<[Deferred<Arc<str>>]>,
    pub data: IndexMap<EvalKey, EvalValue>,
}

impl Item {
    /// Parses an item template identified by `id`. Item metadata formulas remain deferred until stack instantiation.
    ///
    /// # Errors
    ///
    /// Returns an error when the definition has malformed text, traits, or metadata.
    fn from_json(id: &str, value: &JsonValue) -> Result<Self> {
        let data = object(value, "La définition d'un objet")?;
        let name = split_fmt_str(&string_or(data, "nom", "Objet sans nom")?)?;
        let description = split_fmt_str(&string_or(data, "description", "Pas de description")?)?;
        let traits = collect_traits(data.get("traits"))
            .with_context(|| format!("Erreur en parsant les traits de l'objet {}", id))?;
        let metadata = object_or_empty(data, "donnees")?
            .iter()
            .map(|tup| {
                let (k, v) = tup;
                let eval_value: EvalValue = value_or_formula(json_to_eval_value(v)?)?;
                Ok((EvalKey(EvalValue::String(k.clone().into())), eval_value))
            })
            .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;
        Ok(Self {
            id: id.into(),
            name,
            description,
            traits,
            data: metadata,
        })
    }
    /// Creates an item stack by merging template metadata with instance overrides.
    ///
    /// Metadata values are resolved in insertion order. Each resolved field is added to the formula
    /// environment, so a later field may refer to an earlier one. `qt` is stored unchanged as the stack
    /// quantity.
    ///
    /// # Errors
    ///
    /// Returns an error if a metadata formula cannot be resolved.
    pub fn instantiate(
        self: &Arc<Self>,
        overrides: &EvalValue,
        qt: i64,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<ItemStack> {
        let overrides = match formula.resolve_formula(&overrides, env)? {
            EvalValue::Object(map) => map,
            other => bail!("Les données d'état doivent être un dictionnaire ou être évalué en un, reçu {other}"),
        };
        let mut data = self.data.clone();
        data.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut source_env = Env::from_arc(Arc::new(dict_to_map(data.clone())?)).with_parent(env);
        source_env.insert("id", EvalValue::String(self.id.clone()));
        for (key, value) in &mut data {
            *value = formula
                .resolve_formula(value, &source_env)
                .with_context(|| {
                    format!("Résolution de `donnees.{key}` pour l'objet `{}`", self.id)
                })?;
            let EvalKey(EvalValue::String(key_str)) = key else {
                bail!("Clé non valide dans les données de l'objet");
            };
            source_env.insert(key_str.clone(), value.clone());
        }
        Ok(ItemStack {
            item: self.clone(),
            quantity: qt,
            data: Arc::from(data),
        })
    }
}

/// Stores an instantiated item, its quantity, and resolved metadata.
#[derive(Clone, Debug)]
pub struct ItemStack {
    pub item: Arc<Item>,
    pub quantity: i64,
    pub data: Arc<IndexMap<EvalKey, EvalValue>>,
}

impl ItemStack {
    /// Formats the item name using the stack metadata plus `base_env`.
    ///
    /// `base_env` bindings take precedence over stack bindings with the same name.
    ///
    /// # Errors
    ///
    /// Propagates formula-formatting errors.
    pub fn actual_name(&self, formula: &mut FormulaEngine, base_env: &Env) -> Result<Arc<str>> {
        let env = mk_partial_item_env(self)
            .with_context(|| format!("Calcul de la vue partielle de l'objet {}", self.item.id))?
            .with_parent(base_env);
        formula
            .fmt_value(&self.item.name, &env)
            .with_context(|| {
                format!(
                    "Résolution du nom de l'objet `{}` avec données {}",
                    self.item.id,
                    EvalValue::Object(self.data.clone()).diagnostic()
                )
            })
    }
    /// Formats the item description using the stack metadata plus `base_env`.
    ///
    /// `base_env` bindings take precedence over stack bindings with the same name.
    ///
    /// # Errors
    ///
    /// Propagates formula-formatting errors.
    pub fn actual_description(
        &self,
        formula: &mut FormulaEngine,
        base_env: &Env,
    ) -> Result<Arc<str>> {
        let env = mk_partial_item_env(self)
            .with_context(|| format!("Calcul de la description de l'objet {}", self.item.id))?
            .with_parent(base_env);
        formula
            .fmt_value(&self.item.description, &env)
            .with_context(|| {
                format!(
                    "Résolution de la description de l'objet `{}` avec données {}",
                    self.item.id,
                    EvalValue::Object(self.data.clone()).diagnostic()
                )
            })
    }
    /// Returns metadata `key`, preferring the instantiated override over the template value.
    pub fn get_data(&self, key: &str) -> Option<&EvalValue> {
        let key = EvalKey(EvalValue::String(key.into()));
        self.data.get(&key).or_else(|| self.item.data.get(&key))
    }
    /// Returns merged template metadata and instance overrides.
    ///
    /// Instance values replace template values with the same key; no formula evaluation occurs.
    pub fn actual_data(&self) -> Arc<IndexMap<EvalKey, EvalValue>> {
        self.data.clone()
    }
}

/// Defines a reusable persistent state before it is instantiated.
#[derive(Clone, Debug)]
pub struct ConditionTemplate {
    pub id: Arc<str>,
    pub name: EvalValue,
    pub description: EvalValue,
    pub data: IndexMap<EvalKey, EvalValue>,
    pub effects: Vec<ConditionEffect>,
    pub traits: Arc<[Deferred<Arc<str>>]>,
    pub expiration: Option<ConditionFilter>,
    pub death: Option<EvalValue>,
    /// Cells executed immediately before the current page's own cells.
    pub pre: Arc<[Deferred<Cell>]>,
    /// Cells executed immediately after the current page's own cells.
    pub post: Arc<[Deferred<Cell>]>,
}

impl ConditionTemplate {
    /// Parses a persistent-state template identified by `id`, including modifiers, lifecycle cells, expiration, and defeat destination.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed fields, effects, filters, or cells.
    fn from_json(id: &str, value: &JsonValue) -> Result<Self> {
        let data = object(value, "La définition d'un état")?;
        let name = split_fmt_str(&string_or(data, "nom", "État sans nom")?)?;
        let description = split_fmt_str(&string_or(data, "description", "")?)?;
        let metadata = object_or_empty(data, "donnees")?
            .iter()
            .map(|tup| {
                let (k, v) = tup;
                let eval_value: EvalValue = value_or_formula(json_to_eval_value(v)?)?;
                Ok((EvalKey(EvalValue::from(k.clone())), eval_value))
            })
            .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;
        let traits = collect_traits(data.get("traits"))
            .with_context(|| format!("Erreur en parsant les traits de l'état {id}"))?;
        let effects = array_or_empty(data, "effets")?
            .iter()
            .map(ConditionEffect::from_json)
            .collect::<Result<Vec<_>>>()?;
        let expiration = data
            .get("expiration")
            .filter(|value| !value.is_null())
            .map(ConditionFilter::from_json)
            .transpose()?;
        let death = optional_string(data, "defaite")?
            .map(Arc::from)
            .map(|s| split_fmt_str(&s))
            .transpose()?;
        let pre = Cell::parse_many(data.get("avant"), "Le champ `avant` d'un état")?.into();
        let post = Cell::parse_many(data.get("apres"), "Le champ `apres` d'un état")?.into();
        Ok(Self {
            id: id.to_owned().into(),
            name: name,
            description: description.into(),
            data: metadata,
            effects,
            traits,
            expiration,
            death,
            pre,
            post,
        })
    }
    /// Creates a persistent-state instance by resolving template data, overrides, and modifiers.
    ///
    /// State data is resolved in insertion order, then each modifier template is instantiated against the
    /// resolved state data. Lifecycle cells, expiration, and defeat destination are cloned without
    /// execution.
    ///
    /// # Errors
    ///
    /// Returns an error if state data or a modifier formula cannot be resolved.
    pub fn instantiate(
        self: &Arc<Self>,
        overrides: &EvalValue,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<ConditionInstance> {
        let overrides = match formula.resolve_formula(&overrides, env)? {
            EvalValue::Object(map) => map,
            other => bail!("Les données d'état doivent être un dictionnaire ou être évalué en un, reçu {other}"),
        };
        let mut merged = self.data.clone();
        merged.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut merged = Arc::from(merged);
        let mut cond = ConditionInstance {
            template: self.clone(),
            data: merged.clone(),
        };
        let work_data = Arc::make_mut(&mut cond.data);
        let mut shadow = work_data.clone();
        for (key, value) in work_data.iter_mut() {
            let cenv = {
                let mut obj = IndexMap::new();
                obj.insert(EvalKey("id".into()), EvalValue::String(self.id.clone()));
                obj.insert(EvalKey("donnees".into()), EvalValue::Object(merged.clone()));

                let mini_env = {
                    let map = Arc::new(dict_to_map(shadow.clone())?);
                    let mut mini_env = Env::from_arc(map).with_parent(env);
                    mini_env.insert("etat", EvalValue::Object(Arc::from(obj.clone())));
                    mini_env
                };
                let traits = eval_traits(&self.traits, formula, &mini_env)
                    .with_context(|| format!("Erreur en évaluant les traits d'une instance de l'etat {}", self.id))?;
                obj.insert(
                    EvalKey("traits".into()),
                    EvalValue::Array(traits),
                );

                let mut cenv = Env::new();
                cenv.insert("etat", EvalValue::Object(Arc::from(obj)));
                cenv.with_parent(env)
            };
            *value = formula
                .resolve_formula(value, &cenv)
                .with_context(|| {
                    format!("Résolution de `donnees.{key}` pour l'état `{}`", self.id)
                })?;
            let work = Arc::make_mut(&mut merged);
            work.insert(key.clone(), value.clone());
            shadow.insert(key.clone(), value.clone());
        }
        // println!("Instantiated condition {} with data {:?}", cond.id, cond.data);

        Ok(cond)
    }
}

/// Stores one resolved persistent state attached to a character.
#[derive(Clone, Debug)]
pub struct ConditionInstance {
    pub template: Arc<ConditionTemplate>,
    pub data: Arc<IndexMap<EvalKey, EvalValue>>,
}

impl ConditionInstance {
    /// Formats the persistent state's display name using its own data and `env`; supplied bindings take precedence on name collisions.
    ///
    /// # Errors
    ///
    /// Propagates formula-formatting errors.
    pub fn actual_name(&self, formula: &mut FormulaEngine, env: &Env) -> Result<Arc<str>> {
        let env = mk_partial_condition_env(self)
            .with_context(|| format!("Erreur en évaluant l'environement d'un etat {}", self.template.id))?
            .with_parent(env);
        formula
            .fmt_value(&self.template.name, &env)
            .with_context(|| {
                format!(
                    "Résolution du nom de l'état `{}` avec données {}",
                    self.template.id,
                    EvalValue::Object(self.data.clone()).diagnostic()
                )
            })
    }
    /// Formats the persistent state's display description using its own data and `env`; supplied bindings take precedence on name collisions.
    ///
    /// # Errors
    ///
    /// Propagates formula-formatting errors.
    pub fn actual_description(&self, formula: &mut FormulaEngine, env: &Env) -> Result<Arc<str>> {
        let env = mk_partial_condition_env(self)
            .with_context(|| format!("Erreur en évaluant l'environement d'un etat {}", self.template.id))?
            .with_parent(env);
        formula
            .fmt_value(&self.template.description, &env)
            .with_context(|| {
                format!(
                    "Résolution de la description de l'état `{}` avec données {}",
                    self.template.id,
                    EvalValue::Object(self.data.clone()).diagnostic()
                )
            })
    }
    /// Evaluates this instance's expiration filter.
    ///
    /// A state without an expiration filter never expires.
    ///
    /// # Errors
    ///
    /// Propagates filter or formula evaluation errors.
    pub fn is_expired(&self, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        let env = mk_partial_condition_env(self)
            .with_context(|| format!("Erreur en évaluant l'environement d'un etat {}", self.template.id))?
            .with_parent(env);
        match &self.template.expiration {
            Some(filter) => filter.matches(self, formula, &env),
            None => Ok(false),
        }
    }
    
    pub fn actual_data(&self) -> Arc<IndexMap<EvalKey, EvalValue>> {
        Arc::clone(&self.data)
    }
}

/// Defines an attribute check, its difficulty, traits, and failure destination.
#[derive(Clone, Debug)]
pub struct Check {
    pub attribute: EvalValue,
    pub difficulty: EvalValue,
    pub traits: Arc<[Arc<str>]>,
    pub failure: Option<EvalValue>,
}

impl Check {
    /// Parses one connection check definition. Difficulty/modifier formulas remain deferred.
    ///
    /// # Errors
    ///
    /// Returns an error when required fields, traits, or optional failure destination have invalid types.
    fn from_json(value: &JsonValue) -> Result<Self> {
        let data = object(value, "Un test")?;
        let attribute = match data.get("attribut") {
            Some(JsonValue::String(s)) => split_fmt_str(s)?,
            Some(JsonValue::Null) | None => EvalValue::Null,
            other => bail!("Le champ `attribut` d'un test doit être une chaîne de caractères ou null, trouvé: {:?}", other),
        };
        let difficulty = value_or_formula(
            json_to_eval_value(&data
                .get("difficulte")
                .cloned()
                .unwrap_or(JsonValue::from(10))
            )?,
        )?;
        let traits = match data.get("traits") {
            None => Vec::new(),
            Some(value) => string_list(value, "Les traits d'un test", false)?,
        }
        .into_iter()
        .map(Arc::from)
        .collect();
        let failure = optional_string(data, "echec")?
            .map(Arc::from)
            .map(|s| split_fmt_str(&s))
            .transpose()?;
        Ok(Self {
            attribute,
            difficulty,
            traits,
            failure,
        })
    }

    fn from_dict(data: &EvalValue) -> Result<Self> {
        let data = match data {
            EvalValue::Object(map) => map,
            _ => bail!("Un test doit être un dictionnaire"),
        };
        let attribute = match data.get(&EvalKey(EvalValue::String("attribut".into()))) {
            Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
            Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
            Some(EvalValue::Null) | None => EvalValue::Null,
            _ => bail!("Un test doit inclure un champ `attribut`"),
        };
        let difficulty = data
            .get(&EvalKey(EvalValue::String("difficulte".into())))
            .cloned()
            .unwrap_or(EvalValue::Int(10));
        let traits = match data.get(&EvalKey(EvalValue::String("traits".into()))) {
            Some(EvalValue::Array(arr)) => arr
                .iter()
                .map(|v| match v {
                    EvalValue::String(s) => Ok(s.clone()),
                    _ => bail!("Les traits d'un test doivent être des chaînes de caractères"),
                })
                .collect::<Result<Vec<_>>>()?,
            Some(EvalValue::Null) | None => Vec::new(),
            _ => bail!("Le champ `traits` d'un test doit être une liste"),
        }
        .into();
        let failure = match data.get(&EvalKey(EvalValue::String("echec".into()))) {
            Some(EvalValue::String(s)) => Some(EvalValue::String(s.clone())),
            Some(EvalValue::Func(f)) => Some(EvalValue::Func(f.clone())),
            Some(EvalValue::Null) | None => None,
            _ => bail!("Le champ `echec` d'un test doit être une chaîne de caractères"),
        };
        Ok(Self {
            attribute,
            difficulty,
            traits,
            failure,
        })
    }
    /// Resolves and rolls this check against the current character.
    ///
    /// The result combines one d20 roll, the effective attribute value, and all matching check modifiers.
    /// Difficulty is resolved with the character environment merged with `env`.
    ///
    /// # Errors
    ///
    /// Returns an error if the attribute is unknown or if difficulty/modifier formulas fail.
    pub fn roll(
        &self,
        character: &Character,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<CheckResult> {
        let base_env = mk_character_env(character, formula, env)
            .context("Construction de l'environnement du test")?;
        let base_env = base_env.merged(env);
        let attr = match &self.attribute {
            EvalValue::Func(_) => {
                let res = self.attribute.clone().call(&[], formula, &base_env).with_context(|| {
                format!("Résolution de l'attribut du test depuis {}", self.attribute.diagnostic())
            })?;
                match res {
                    EvalValue::String(s) => Some(s),
                    EvalValue::Null => None,
                    _ => bail!("L'attribut du test doit être évalué en une chaîne de caractères ou nul"),
                }
            },
            EvalValue::String(_) => Some(formula.fmt_value(&self.attribute, &base_env).with_context(|| {
                format!("Résolution de l'attribut du test depuis {}", self.attribute.diagnostic())
            })?),
            EvalValue::Null => None,
            _ => bail!("L'attribut du test doit être évalué en une chaîne de caractères ou nul"),
        };
        let base_attribute = if let Some(attr) = &attr {
            character
            .actual_attribute(&attr, formula, env)
            .with_context(|| format!("Calcul de l'attribut effectif `{attr}` du test"))?
        } else {
            0
        };
        let modifiers =
            character
                .check_modifier(attr.clone(), &self.traits, formula, env)
                .with_context(|| {
                    format!(
                        "Calcul des modificateurs du test `{attr:?}` avec traits [{}]",
                        self.traits.join(", ")
                    )
                })?;
        let roll = formula.d20();
        let charenv = mk_character_env(character, formula, env)?;
        let tenv = charenv.merged(env);
        let target = formula
            .resolve_formula_int(&self.difficulty, &tenv)
            .with_context(|| {
                format!(
                    "Résolution de la difficulté du test `{attr:?}` depuis {}",
                    self.difficulty.diagnostic()
                )
            })?;
        Ok(CheckResult {
            success: roll + base_attribute + modifiers >= target,
            difficulty: target,
            modifier: modifiers,
            roll,
            traits: self.traits.clone(),
            attribute: attr,
            attribute_value: base_attribute,
            total_modifier: base_attribute + modifiers,
            total: roll + base_attribute + modifiers,
            margin: roll + base_attribute + modifiers - target,
        })
    }
}

#[derive(Clone, Debug)]
/// Stores the fully resolved outcome of one connection check.
pub struct CheckResult {
    pub success: bool,
    pub difficulty: i64,
    pub modifier: i64,
    pub roll: i64,
    pub traits: Arc<[Arc<str>]>,
    pub attribute: Option<Arc<str>>,
    pub attribute_value: i64,
    pub total: i64,
    pub total_modifier: i64,
    pub margin: i64,
}
impl CheckResult {
    /// Returns the check outcome dictionary exposed to connection-cell formulas as `test`.
    pub fn into_dict(&self) -> IndexMap<EvalKey, EvalValue> {
        let mut dict = IndexMap::new();
        dict.insert(EvalKey(EvalValue::String("resultat".into())), EvalValue::Bool(self.success));
        dict.insert(EvalKey(EvalValue::String("difficulte".into())), EvalValue::Int(self.difficulty));
        dict.insert(EvalKey(EvalValue::String("modificateur".into())), EvalValue::Int(self.modifier));
        dict.insert(EvalKey(EvalValue::String("jet".into())), EvalValue::Int(self.roll));
        dict.insert(EvalKey(EvalValue::String("attribut".into())), match &self.attribute {
            Some(s) => EvalValue::String(s.clone()),
            None => EvalValue::Null,
        });
        dict.insert(
            EvalKey(EvalValue::String("valeur_attribut".into())),
            EvalValue::Int(self.attribute_value),
        );
        dict.insert(EvalKey(EvalValue::String("total".into())), EvalValue::Int(self.total));
        dict.insert(
            EvalKey(EvalValue::String("total_modificateur".into())),
            EvalValue::Int(self.total_modifier),
        );
        dict.insert(EvalKey(EvalValue::String("marge".into())), EvalValue::Int(self.margin));
        dict.insert(
            EvalKey(EvalValue::String("traits".into())),
            EvalValue::Array(self.traits.iter().cloned().map(EvalValue::String).collect()),
        );
        dict
    }
}

/// Defines one conditional transition from a page to another page.
#[derive(Clone, Debug)]
pub struct Connection {
    pub destination: EvalValue,
    pub text: EvalValue,
    pub conditional: Option<Conditional>,
    pub check: Option<Check>,
    /// Cells carried to the destination whenever this connection is selected.
    pub cells: Arc<[Deferred<Cell>]>,
    pub tags: IndexMap<EvalKey, EvalValue>,
}

impl Connection {
    
    fn parse_many(value: Option<&JsonValue>) -> Result<Arc<[Deferred<Self>]>> {
        match value {
            Some(JsonValue::Null) | None => Ok(Arc::from([])),
            Some(JsonValue::String(s)) => {
                let formula = value_or_formula(EvalValue::String(s.clone().into()))
                    .with_context(|| "Erreur en parsant une connexion.")?;
                match formula {
                    EvalValue::Func(_) => Ok(Arc::from([Deferred::Formula(formula)])),
                    other => bail!("Une connexion doit être un dictionnaire, une formule, ou une liste de, reçu {}", other.diagnostic()),
                }
            },
            Some(JsonValue::Object(obj)) => {
                let conn = Self::from_json(&JsonValue::Object(obj.clone()))
                    .with_context(|| "Erreur en parsant une connexion")?;
                Ok(Arc::from([Deferred::Literal(conn)]))
            },
            Some(JsonValue::Array(arr)) => arr
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    match v {
                        JsonValue::String(source) => {
                            let formula = value_or_formula(EvalValue::String(source.clone().into()))
                                .with_context(|| format!("Erreur en parsant la connexion à l'index {i}"))?;
                            match formula {
                                EvalValue::Func(_) => Ok(Deferred::Formula(formula)),
                                other => bail!(
                                    "La connexion à l'index {i} doit être un dictionnaire ou une formule complète, reçu {}",
                                    other.diagnostic()
                                ),
                            }
                        }
                        JsonValue::Object(_) => {
                            let conn = Self::from_json(v)
                                .with_context(|| format!("Erreur en parsant la connexion à l'index {i}"))?;
                            Ok(Deferred::Literal(conn))
                        }
                        other => bail!(
                            "La connexion à l'index {i} doit être un dictionnaire ou une formule, reçu {other}"
                        ),
                    }
                })
                .collect::<Result<Vec<_>>>()
                .map(|vec| vec.into()),
            Some(_) => bail!("Le champ `connexions` doit être un tableau"),
        }
    }
    pub fn parse_many_from_value(value: &EvalValue) -> Result<Arc<[Deferred<Self>]>> {
        match value {
            EvalValue::Null => Ok(Arc::from([])),
            EvalValue::Func(_) => Ok(Arc::from([Deferred::Formula(value.clone())])),
            EvalValue::Object(obj) => {
                let conn = Self::from_dict(EvalValue::Object(obj.clone()))
                    .with_context(|| "Erreur en parsant une connexion")?;
                Ok(Arc::from([Deferred::Literal(conn)]))
            },
            EvalValue::Array(arr) => arr
                .iter()
                .map(|v| {
                    match v {
                        EvalValue::Func(_) => Ok(Deferred::Formula(v.clone())),
                        EvalValue::Object(_) => {
                            let conn = Self::from_dict(v.clone())
                                .with_context(|| "Erreur en parsant une connexion")?;
                            Ok(Deferred::Literal(conn))
                        },
                        other => bail!("Une connexion doit être un dictionnaire, une formule, ou une liste de, reçu {}", other.diagnostic())
                    }
                })
                .collect::<Result<Vec<_>>>()
                .map(|vec| vec.into()),
            other => bail!("Le champ `connexions` doit être un tableau, un objet, ou une formule, reçu {}", other.diagnostic()),
        }
    }

    /// Parses one static or generated connection dictionary.
    ///
    /// Explicit `null` optional fields are treated as absent. Connection tags are retained
    /// until the connection is followed, then evaluated once for the destination page.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed destinations, text, checks, cells, conditions, or tags.
    fn from_json(value: &JsonValue) -> Result<Self> {
        let data = object(value, "Une connexion")?;
        if data.contains_key("effets") {
            bail!(
                "Le champ `effets` d'une connexion appartient à la syntaxe v1; utilisez `cellules`"
            );
        }
        let destination = split_fmt_str(&required_string(data, "destination", "Une connexion")?)?;
        let text = split_fmt_str(&required_string(data, "texte", "Une connexion")?)?;
        let conditional = data
            .get("conditions")
            .map(Conditional::from_json)
            .transpose()?;
        let check = data.get("test").map(Check::from_json).transpose()?;
        let cells = Cell::parse_many(data.get("cellules"), "Les cellules d'une connexion")?.into();
        let tags = collect_tags(data.get("tags")).with_context(|| "Erreur en parsant une connexion")?;
        Ok(Self {
            destination,
            text,
            conditional,
            check,
            cells,
            tags,
        })
    }

    fn from_dict(data: EvalValue) -> Result<Self> {
        let data = match data {
            EvalValue::Object(map) => map,
            _ => bail!("Une connexion doit être un dictionnaire"),
        };
        let destination = match data
            .get(&EvalKey(EvalValue::String("destination".into())))
            .ok_or_else(|| anyhow!("Une connexion doit avoir une destination"))?
        {
            EvalValue::String(s) => EvalValue::String(s.clone()),
            EvalValue::Func(f) => EvalValue::Func(f.clone()),
            autre => bail!("La destination d'une connexion doit être une chaîne de caractères ou une formule (reçu {autre})"),
        };
        let text = match data
            .get(&EvalKey(EvalValue::String("texte".into())))
            .ok_or_else(|| anyhow!("Une connexion doit avoir un texte"))?
        {
            EvalValue::String(s) => EvalValue::String(s.clone()),
            EvalValue::Func(f) => EvalValue::Func(f.clone()),
            autre => bail!("Le texte d'une connexion doit être une chaîne de caractères ou une formule (reçu {autre})"),
        };
        let conditional = match data.get(&EvalKey(EvalValue::String("conditions".into()))) {
            Some(cond) => {
                if cond == &EvalValue::Null {
                    None
                } else {
                    Some(Conditional::from_dict(cond)?)
                }
            }
            None => None,
        };
        let check = match data.get(&EvalKey(EvalValue::String("test".into()))) {
            Some(check_value) => {
                if check_value == &EvalValue::Null {
                    None
                } else {
                    Some(Check::from_dict(check_value)?)
                }
            }
            None => None,
        };
        let cells = data.get(&EvalKey(EvalValue::String("cellules".into())));
        let cells = Cell::parse_many_from_value(cells.unwrap_or(&EvalValue::Null), "Connexion à partir d'une valeur")
            .with_context(|| "Erreur en parsant les cellules d'une connexion")?
            .into();
        let tags = match data.get(&EvalKey(EvalValue::String("tags".into()))) {
            Some(EvalValue::Object(obj)) => {
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<IndexMap<_, _>>()
            },
            Some(EvalValue::Null) | None => { IndexMap::new() },
            Some(other) => bail!("Le champ `tags` d'une connexion doit être un dictionnaire, pas {other:?}"),
        };
        Ok(Self {
            destination,
            text,
            conditional,
            check,
            cells,
            tags,
        })
    }
    /// Returns whether the connection's optional condition currently matches.
    ///
    /// `env` is extended with the generated connection context. A connection without a condition is
    /// always valid.
    ///
    /// # Errors
    ///
    /// Propagates conditional or formula evaluation errors.
    /// 
    /// env should contain `recit`, `page`, `test`, `connexion` (from the current/previous page) and `personnage` (final state)
    pub fn valid(
        &self,
        character: &Character,
        page_tags: Arc<IndexMap<EvalKey, EvalValue>>,
        connection_tags: Arc<IndexMap<EvalKey, EvalValue>>,
        tale: &Tale,
        env: &Env,
        formula: &mut FormulaEngine,
    ) -> Result<bool> {
        match &self.conditional {
            None => Ok(true),
            Some(conditional) => conditional
                .matches(
                    character,
                    page_tags,
                    connection_tags,
                    tale,
                    formula,
                    env,
                )
                .with_context(|| {
                    format!(
                        "Évaluation des conditions de la connexion vers {}",
                        self.destination.diagnostic()
                    )
                }),
        }
    }
    /// Formats the menu label for this connection.
    ///
    /// Checked connections append the check attribute in brackets before formula formatting.
    /// Character, page, and captured closure bindings are available.
    ///
    /// # Errors
    ///
    /// Propagates formula-formatting errors.
    /// 
    /// env should contain `recit`, `page`, `test`, `connexion` (from the current/previous page) and `personnage` (final state)
    pub fn shown_text(
        &self,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Arc<str>> {
        let text = match &self.check {
            None => {
                formula
                    .fmt_value(&self.text, &env)
                    .with_context(|| format!("Résolution du texte depuis {}", self.text.diagnostic()))?
            },
            Some(check) => {
                let attr = formula
                    .fmt_value(&check.attribute, &env)
                    .context("Résolution de l'attribut affiché par la connexion")?;
                let text = formula
                    .fmt_value(&self.text, &env)
                    .with_context(|| format!("Résolution du texte depuis {}", self.text.diagnostic()))?;
                format!("{text} [{attr}]").into()
            },
        };
        Ok(text)
    }
    /// Formats and validates the connection destination.
    ///
    /// Character, page, and captured closure bindings are available. The resolved page ID must
    /// not be empty.
    ///
    /// # Errors
    ///
    /// Returns an error if formatting fails or resolves to an empty destination.
    /// 
    /// env should contain `recit`, `page`, `test`, `connexion` (from the current/previous page) and `personnage` (final state)
    pub fn resolved_destination(
        &self,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Arc<str>> {
        let destination = formula
            .fmt_value(&self.destination, &env)
            .with_context(|| {
                format!("Résolution de la destination depuis {}", self.destination.diagnostic())
            })?;
        if destination.is_empty() {
            bail!("La destination résolue d'une connexion ne peut pas être vide")
        }
        Ok(destination)
    }
    /// Resolves the result of selecting this connection.
    ///
    /// Unchecked connections succeed immediately. Checked connections roll once; connection cells are
    /// accompanied by the resolved `test` object. Connection tags are evaluated once and
    /// exposed to the destination page through `connexion`.
    ///
    /// env should contain `recit`, `page`, `test`, `connexion` (from the current/previous page) and `personnage` (final state)
    /// 
    /// # Errors
    ///
    /// Propagates destination-formatting and check-evaluation errors.
    pub fn visit<'a>(
        &self,
        character: &Character,
        formula: &mut FormulaEngine,
        env: &'a Env<'a>,
    ) -> Result<PageResult> {
        let check_result = self
            .check
            .as_ref()
            .map(|check| check.roll(character, formula, env))
            .transpose()
            .context("Résolution du test de la connexion")?;

        // The resolved check shadows the source page's `test` while destination and
        // connection-tag formulas are evaluated. An unchecked connection exposes null.
        let mut selection_env = Env::new().with_parent(env);
        selection_env.insert(
            "test",
            check_result
                .as_ref()
                .map(|result| EvalValue::Object(Arc::from(result.into_dict())))
                .unwrap_or(EvalValue::Null),
        );

        let next_page = match (&self.check, &check_result) {
            (Some(check), Some(result)) if !result.success => check
                .failure
                .as_ref()
                .map(|failure| formula.fmt_value(failure, &selection_env))
                .transpose()?
                .map(Ok)
                .unwrap_or_else(|| self.resolved_destination(formula, &selection_env))
                .context("Erreur lors de l'évaluation de la destination d'une connexion")?,
            _ => self.resolved_destination(formula, &selection_env)?,
        };
        let connection_env = mk_connection_env(self, formula, env)
            .context("Évaluation des tags de la connexion sélectionnée")?;

        Ok(PageResult {
            next_page: Some(next_page),
            added_cells: self.cells.clone(),
            check_result,
            connection_env,
        })
    }
}

/// Stores the immutable tags, ordered cells, and connections for a tale page.
#[derive(Clone, Debug)]
pub struct Page {
    pub id: Arc<str>,
    pub title: EvalValue,
    /// Immutable metadata exposed to formulas through the `page` object.
    pub tags: Arc<IndexMap<EvalKey, EvalValue>>,
    /// Ordered narrative and effect cells executed on entry.
    pub cells: Arc<[Deferred<Cell>]>,
    pub connections: Arc<[Deferred<Connection>]>,
}

impl Page {
    /// Parses page `id`, including immutable page tags, cells, and literal or formula connections.
    ///
    /// # Errors
    ///
    /// Returns an error when any contained page structure is malformed.
    fn from_json(id: &str, value: &JsonValue) -> Result<Self> {
        let data = object(value, "Une page")?;
        let title = split_fmt_str(&string_or(data, "titre", "Page sans titre")?)?;
        let tags = collect_tags(data.get("tags"))
            .with_context(|| format!("Erreur en parsant les tags de la page `{id}`"))?;
        let cells = Cell::parse_many(data.get("cellules"), "Les cellules d'une page")?.into();
        let mut connections = Connection::parse_many(data.get("connexions"))
            .with_context(|| "Erreur en parsant une page")?
            .to_vec();
        let dyn_connections = match data.get("connexions_dyn") { // for backwards compatibility
            Some(JsonValue::Array(arr)) => arr
                .iter()
                .map(parse_dyn_connection)
                .collect::<Result<Vec<_>>>()?,
            Some(val) => vec![parse_dyn_connection(val)?],
            None => Vec::new(),
        }
        .into_iter()
        .map(Deferred::Formula);
        connections.extend(dyn_connections);
        let connections = Arc::from(connections);
        Ok(Self {
            id: id.to_owned().into(),
            title,
            tags: Arc::from(tags),
            cells,
            connections,
        })
    }

    /// Formats the page title with character and immutable page-tag bindings.
    ///
    /// env should contain `recit`, `test`, `connexion`, `page` (this page) and `personnage`.
    /// 
    /// # Errors
    ///
    /// Propagates formula-formatting errors.
    pub fn actual_title(
        &self,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Arc<str>> {
        formula
            .fmt_value(&self.title, env)
            .with_context(|| format!("Résolution du titre de la page `{}`", self.id))
    }
    /// Builds the currently visible static and dynamic connection list.
    ///
    /// A deferred formula must return either one connection dictionary or a list of
    /// connection dictionaries. All resulting connections are filtered by their conditions.
    /// 
    /// env should contain `recit`, `test`, `connexion`, `page` (this page) and `personnage`.
    ///
    /// # Errors
    ///
    /// Returns an error if generation, conversion, or condition evaluation fails.
    pub fn valid_connections(
        &self,
        character: &Character,
        tale: &Tale,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Vec<Connection>> {
        let mut connections = Vec::new();
        let candidates = eval_connections(&self.connections, formula, env)
            .with_context(|| "Erreur en évaluant les connexions")?;
        // extract page and connection tags from the environment, defaulting to empty if not present
        let evaluated_page_tags = match env.get("page") {
            Some(EvalValue::Object(tags)) => tags.clone(),
            _ => Arc::new(IndexMap::new()),
        };
        let evaluated_connection_tags = match env.get("connexion") {
            Some(EvalValue::Object(tags)) => tags.clone(),
            _ => Arc::new(IndexMap::new()),
        };
        for (index, connection) in candidates.into_iter().enumerate() {
            if connection
                .valid(character, evaluated_page_tags.clone(), evaluated_connection_tags.clone(), tale, env, formula)
                .with_context(|| {
                    format!("Connexion #{} de la page `{}`", index + 1, self.id)
                })?
            {
                connections.push(connection);
            }
        }
        Ok(connections)
    }
}

/// Contains the destination and connection cells selected by a transition.
#[derive(Clone, Debug, Default)]
pub struct PageResult {
    pub next_page: Option<Arc<str>>,
    pub added_cells: Arc<[Deferred<Cell>]>,
    pub check_result: Option<CheckResult>,
    pub connection_env: Env<'static>,
}

fn parse_dyn_connection(value: &JsonValue) -> Result<EvalValue> {
    let val = match value {
        JsonValue::String(s) => {
            formulas::value_or_formula(EvalValue::String(s.clone().into()))
        },
        JsonValue::Object(obj) => {
            if let Some(formula) = obj.get("formule") {
                if let JsonValue::String(s) = formula {
                    formulas::value_or_formula(EvalValue::String(s.clone().into()))
                } else {
                    bail!("Le champ `formule` d'une connexion dynamique doit être une formule");
                }
            } else {
                bail!("Une connexion dynamique doit être une chaîne de caractères ou un objet contenant un champ `formule`");
            }
        },
        _ => bail!("Une connexion dynamique doit être une formule, ou un objet contenant un champ `formule`"),
    }?;
    match val {
        EvalValue::Func(_) => Ok(val),
        _ => bail!("Une connexion dynamique doit être une formule"),
    }
}

/// Parse and collects tags for pages and connections. Tags are expected to be a dictionary of string keys to values or formulas.
pub fn collect_tags(value: Option<&JsonValue>) -> Result<IndexMap<EvalKey, EvalValue>> {
    match value {
        None | Some(JsonValue::Null) => Ok(IndexMap::new()),
        Some(JsonValue::Object(obj)) => obj
            .iter()
            .map(|(key, value)| {
                let val = formulas::value_or_formula(json_to_eval_value(value)
                    .map_err(|err| anyhow::Error::from(err))
                    .with_context(|| format!("Erreur en lisant le tag `{key}`"))?)
                    .with_context(|| format!("Erreur en évaluant le tag `{key}`"))?;
                Ok((EvalKey(EvalValue::from(key.clone())), val))
            })
            .collect::<Result<IndexMap<EvalKey, EvalValue>>>(),
        Some(_) => bail!("Le champ `tags` doit être un dictionnaire"),
    }
}
/// Evaluates all tags in a given dictionary, resolving any formulas with the provided formula engine and environment.
pub fn eval_tags(tags: &IndexMap<EvalKey, EvalValue>, formula: &mut FormulaEngine, env: &Env) -> Result<IndexMap<EvalKey, EvalValue>> {
    let mut evaluated_tags = IndexMap::new();
    for (key, value) in tags.iter() {
        let evaluated_value = formula.resolve_formula(value, env)
            .with_context(|| format!("Erreur en évaluant le tag `{key}`"))?;
        evaluated_tags.insert(key.clone(), evaluated_value);
    }
    Ok(evaluated_tags)
}
/// Evaluates all deferred cells in a given slice, resolving any formulas with the provided formula engine and environment.
pub fn eval_cells(cells: &[Deferred<Cell>], formula: &mut FormulaEngine, env: &Env) -> Result<Vec<Cell>> {
    let mut evaluated_cells = Vec::new();
    for deferred_cell in cells.iter() {
        match deferred_cell {
            Deferred::Literal(cell) => { evaluated_cells.push(cell.clone()); },
            Deferred::Formula(f) => {
                let evaluated_cell = formula.resolve_formula(f, env)
                    .with_context(|| format!("Erreur en évaluant la cellule différée `{}`", f.diagnostic()))?;
                match evaluated_cell {
                    EvalValue::Object(_) => {
                        let cell = Cell::from_dict(&evaluated_cell)
                            .with_context(|| format!("Erreur en parsant la cellule différée `{}`", f.diagnostic()))?;
                        evaluated_cells.push(cell);
                    },
                    EvalValue::Array(arr) => {
                        for cell_data in arr.iter() {
                            let cell = Cell::from_dict(cell_data)
                                .with_context(|| format!("Erreur en parsant la cellule différée `{}`", f.diagnostic()))?;
                            evaluated_cells.push(cell);
                        }
                    },
                    other => {
                        bail!("Une cellule différée doit être évaluée en dictionnaire ou une liste de dictionnaires. Reçu: {}", other.diagnostic());
                    }
                }
            },
        }
    }
    Ok(evaluated_cells)
}
/// Evaluates all deferred connections in a given slice, resolving any formulas with the provided formula engine and environment.
pub fn eval_connections(connections: &[Deferred<Connection>], formula: &mut FormulaEngine, env: &Env) -> Result<Vec<Connection>> {
    let mut evaluated_connections = Vec::new();
    for deferred_connection in connections.iter() {
        match deferred_connection {
            Deferred::Literal(conn) => { evaluated_connections.push(conn.clone()); },
            Deferred::Formula(f) => {
                let evaluated_conn = formula.resolve_formula(f, env)
                    .with_context(|| format!("Erreur en évaluant la connexion différée `{}`", f.diagnostic()))?;
                match evaluated_conn {
                    EvalValue::Object(_) => {
                        let conn = Connection::from_dict(evaluated_conn)
                            .with_context(|| format!("Erreur en parsant la connexion différée `{}`", f.diagnostic()))?;
                        evaluated_connections.push(conn);
                    },
                    EvalValue::Array(arr) => {
                        for conn_data in arr.to_vec().into_iter() {
                            let conn = Connection::from_dict(conn_data)
                                .with_context(|| format!("Erreur en parsant la connexion différée `{}`", f.diagnostic()))?;
                            evaluated_connections.push(conn);
                        }
                    },
                    other => {
                        bail!("Une connexion différée doit être évaluée en dictionnaire ou une liste de dictionnaires. Reçu: {}", other.diagnostic());
                    }
                }
            },
        }
    }
    Ok(evaluated_connections)
}
/// Parses and collects data fields for a tale, resolving any formulas with the provided formula engine and environment.
pub fn collect_data(value: Option<&JsonValue>) -> Result<EvalValue> {
    match value {
        None | Some(JsonValue::Null) => Ok(EvalValue::Object(IndexMap::new().into())),
        Some(JsonValue::Object(obj)) => obj
            .iter()
            .map(|(key, value)| {
                let val = value_or_formula(json_to_eval_value(value)
                    .map_err(|err| anyhow::Error::from(err))
                    .with_context(|| format!("Erreur en lisant le champ de données `{key}`"))?)
                    .with_context(|| format!("Erreur en évaluant le champ de données `{key}`"))?;
                Ok((EvalKey(EvalValue::from(key.clone())), val))
            })
            .collect::<Result<IndexMap<EvalKey, EvalValue>>>()
            .map(|map| EvalValue::Object(map.into())),
        Some(JsonValue::String(s)) => {
            let val = value_or_formula(EvalValue::String(s.clone().into()))
                .with_context(|| "Erreur en évaluant le champ de données")?;
            match val {
                EvalValue::Func(_) => Ok(val),
                _ => bail!("Le champ `donnees` doit être un dictionnaire ou une formule, reçu {}", val.diagnostic()),
            }
        },
        Some(other) => bail!("Le champ `donnees` doit être un dictionnaire, une formule ou null, reçu {other}"),
    }
}
pub fn collect_traits(value: Option<&JsonValue>) -> Result<Arc<[Deferred<Arc<str>>]>> {
    match value {
        None | Some(JsonValue::Null) => Ok(Arc::from([])),
        Some(JsonValue::Array(arr)) => arr
            .iter()
            .map(|v| {
                if let JsonValue::String(s) = v {
                    value_or_formula(s.as_str().into())
                        .map_err(|err| anyhow!("Erreur en évaluant le trait: {}", err))
                        .and_then(|val| match val {
                            EvalValue::String(s) => Ok(Deferred::Literal(s)),
                            EvalValue::Func(_) => Ok(Deferred::Formula(val)),
                            _ => Err(anyhow!("Chaque trait doit être une chaîne de caractères ou une formule, reçu {}", val.diagnostic())),
                        })
                } else {
                    Err(anyhow!("Chaque trait doit être une chaîne de caractères, reçu {}", v))
                }
            })
            .collect::<Result<Arc<[Deferred<Arc<str>>]>>>(),
        Some(JsonValue::String(s)) => {
            value_or_formula(s.as_str().into())
                .map_err(|err| anyhow!("Erreur en évaluant le trait: {}", err))
                .and_then(|val| match val {
                    EvalValue::String(s) => Ok(Arc::from([Deferred::Literal(s)])),
                    EvalValue::Func(_) => Ok(Arc::from([Deferred::Formula(val)])),
                    _ => bail!("Le champ `traits` doit être un tableau de chaînes de caractères ou une formule, reçu {}", val.diagnostic()),
                })
        },
        Some(other) => bail!("Le champ `traits` doit être un tableau de chaînes de caractères, reçu {other}"),
    }
}
pub fn eval_traits(traits: &[Deferred<Arc<str>>], formula: &mut FormulaEngine, env: &Env) -> Result<Arc<[EvalValue]>> {
    let mut evaluated_traits = Vec::new();
    for deferred_trait in traits.iter() {
        match deferred_trait {
            Deferred::Literal(lit) => { evaluated_traits.push(EvalValue::String(lit.clone())); },
            Deferred::Formula(f) => {
                let evaluated_trait = formula.resolve_formula(f, env)
                    .with_context(|| format!("Erreur en évaluant le trait différé `{}`", f.diagnostic()))?;
                match evaluated_trait {
                    EvalValue::String(_) => { evaluated_traits.push(evaluated_trait); },
                    EvalValue::Array(arr) => {
                        let arr = arr
                            .into_iter()
                            .map(|v| match v {
                                EvalValue::String(_) => Ok(v.clone()),
                                other => bail!("Chaque trait différé dans un tableau doit être une chaîne de caractères. Reçu: {}", other.diagnostic()),
                            })
                            .collect::<Result<Vec<_>>>()
                            .with_context(|| "Erreur en évaluant un trait différé")?;
                        evaluated_traits.extend(arr);
                    },
                    other => bail!("Un trait différé doit être évalué en chaîne de caractères. Reçu: {}", other.diagnostic()),
                }
            },
        }
    }
    Ok(evaluated_traits.into())
}
pub fn noneval_traits(traits: &[Deferred<Arc<str>>]) -> Arc<[EvalValue]> {
    traits
        .iter()
        .map(|deferred_trait| match deferred_trait {
            Deferred::Literal(lit) => EvalValue::String(lit.clone()),
            Deferred::Formula(f) => f.clone(),
        })
        .collect::<Vec<_>>()
        .into()
}

#[cfg(test)]
mod global_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_function_parameters_allow_repeated_ignores() {
        let definitions = json!({
            "keep": {"parametres": ["_", "_", "value"], "formule": "${value}"}
        });
        let functions = parse_formula_functions(Some(&definitions)).unwrap();
        let function = &functions["keep"];
        assert_eq!(function.parameters.as_ref().unwrap().len(), 3);
        let mut engine = FormulaEngine::new(Some(17));
        assert_eq!(
            engine.evaluate(&format!("({function})(10, 20, 30)"), &Env::new()).unwrap(),
            EvalValue::Int(30)
        );
        let duplicate = json!({
            "keep": {"parametres": ["_value", "_value"], "formule": "${_value}"}
        });
        assert!(parse_formula_functions(Some(&duplicate)).is_err());
    }

    /// Verifies ordered one-pass initialization and function storage in `recit`.
    #[test]
    fn global_values_are_initialized_once_in_order() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Global",
            "depart": "debut",
            "global": {
                "base": 2,
                "double": "${fn value -> value * recit.base}",
                "answer": "${recit.double(21)}",
                "nested": {"value": "${recit.base + 3}"}
            },
            "pages": {
                "debut": {"cellules": [], "connexions": []}
            }
        }))
        .unwrap();

        assert_eq!(tale.data.get("answer"), Some(&EvalValue::Int(42)));
        assert_eq!(
            tale.data.get("nested").and_then(|value| match value {
                EvalValue::Object(values) => values.get(&EvalKey(EvalValue::String("value".into()))),
                _ => None,
            }),
            Some(&EvalValue::Int(5))
        );

        let mut formula = FormulaEngine::new(Some(1));
        assert_eq!(
            formula.evaluate("recit.double(5)", &tale.env()).unwrap(),
            EvalValue::Int(10)
        );
    }

    /// Verifies late global lookup and capture of a local value shadowing a global name.
    #[test]
    fn global_functions_resolve_globals_without_capturing_the_initial_snapshot() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Fermetures globales",
            "depart": "debut",
            "global": {
                "lire_plus_tard": "${fn -> valeur_tardive}",
                "valeur_tardive": "${20 + 1}",
                "lire_masque": "${let valeur_tardive = 7 for fn -> valeur_tardive}"
            },
            "pages": {
                "debut": {"cellules": [], "connexions": []}
            }
        }))
        .unwrap();

        let mut formula = FormulaEngine::new(Some(1));
        assert_eq!(
            formula
                .evaluate("recit.lire_plus_tard()", &tale.env())
                .unwrap(),
            EvalValue::Int(21)
        );
        assert_eq!(
            formula
                .evaluate("recit.lire_masque()", &tale.env())
                .unwrap(),
            EvalValue::Int(7)
        );
    }

    /// Verifies that model-level validation covers cover and cell image references.
    #[test]
    fn validates_static_image_references_without_rejecting_formula_references() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "depart": "debut",
            "images": {
                "connue": {"type": "chemin", "chemin": "connue.png"}
            },
            "image": ["connue", "couverture_absente"],
            "pages": {
                "debut": {
                    "cellules": [{
                        "images": ["connue", "cellule_absente", "${\"connue\"}"]
                    }],
                    "connexions": []
                }
            }
        }))
        .unwrap();

        let errors = tale.validate_references();
        assert!(errors
            .iter()
            .any(|error| error.contains("couverture_absente")));
        assert!(errors
            .iter()
            .any(|error| error.contains("cellule_absente")));
        assert!(!errors.iter().any(|error| error.contains("${")));
    }
}
