//! Normalized cells, page effects, nested tag mutation, and persistent-state effects.
//!
//! Parsing validates effect structure but defers formulas. Effects execute in order, so
//! later effects observe earlier mutations. A redirection terminates the remaining cell
//! and page-lifecycle phases after preserving already rendered output.
use std::str::FromStr;
use std::sync::Arc;

use crate::character::Character;
use crate::engine::{Interaction, Terminal};
use crate::filters::{CheckFilter, ConditionFilter, Conditional, ItemFilter, TagFilter};
use crate::formula::{dict_to_map, json_to_eval_value, mk_character_base_env, mk_character_env, mk_condition_env, mk_item_env, mk_partial_condition_env, mk_partial_item_env, resolve_formula_quantity};
use formulas::{Env, EvalKey, EvalValue, FormulaEngine, split_fmt_str, value_or_formula};
use crate::json_util::{
    object, optional_string, required_string, required_value, value_or,
};
use crate::model::{Modifier, Tale, collect_data};
use crate::polls::{PollRequest, PollRequestData, PollRequestDataType, PollRequestOption};
use anyhow::{Context, Result, anyhow, bail};
use indexmap::IndexMap;
use serde_json::{Value as JsonValue};

/// Describes a modifier supplied by a persistent-state definition before instantiation.
#[derive(Clone, Debug)]
pub enum ConditionEffect {
    AttributeModifier {
        attribute: Arc<[Deferred<Arc<str>>]>,
        category: Option<Arc<str>>,
        value: EvalValue,
    },
    CheckModifier {
        category: Option<Arc<str>>,
        value: EvalValue,
        filter: Option<CheckFilter>,
    },
    MaxHpModifier {
        category: Option<Arc<str>>,
        value: EvalValue,
    },
    MaxMoraleModifier {
        category: Option<Arc<str>>,
        value: EvalValue,
    },
}

impl ConditionEffect {
    /// Parses one persistent-state modifier template while preserving its value formula for instantiation.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown modifier type or malformed required field/filter.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        let data = object(value, "Un effet d'état")?;
        let kind = required_string(data, "type", "Un effet d'état")?;
        let category = optional_string(data, "categorie")?.map(Arc::from);
        let amount = value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?;
        Ok(match kind.as_str() {
            "modificateur_attribut" => Self::AttributeModifier {
                attribute: {
                    let attr = data.get("attribut");
                    match attr {
                        Some(JsonValue::Null) | None => Vec::new().into(),
                        Some(JsonValue::String(s)) => {
                            let full_form = value_or_formula(EvalValue::String(s.clone().into()))
                                .with_context(|| "Analysant la formule d'attribut")?;
                            match full_form {
                                EvalValue::Func(_) => vec![Deferred::Formula(full_form)].into(),
                                EvalValue::String(s) => vec![Deferred::Literal(Arc::from(s))].into(),
                                _ => bail!("L'attribut doit être une chaîne ou une formule, ou une liste de"),
                            }
                        },
                        Some(JsonValue::Array(arr)) => {
                            let mut result = Vec::new();
                            for item in arr {
                                let full_form = value_or_formula(json_to_eval_value(item)?)
                                    .with_context(|| "Analysant la formule d'attribut")?;
                                match full_form {
                                    EvalValue::Func(_) => result.push(Deferred::Formula(full_form)),
                                    EvalValue::String(s) => result.push(Deferred::Literal(Arc::from(s))),
                                    _ => bail!("L'attribut doit être une chaîne ou une formule, ou une liste de"),
                                }
                            }
                            result.into()
                        }
                        other => bail!("L'attribut doit être une chaîne ou une formule, ou une liste de, reçu {:?}", other),
                    } 
                },
                category,
                value: value_or_formula(amount.try_into()?)?,
            },
            "modificateur_test" => Self::CheckModifier {
                category,
                value: value_or_formula(amount.try_into()?)?,
                filter: {
                    let filter = data.get("filtre");
                    match filter {
                        Some(JsonValue::Null) | None => None,
                        Some(v) => Some(CheckFilter::from_json(v)?),
                    }
                },
            },
            "modificateur_pv_max" => Self::MaxHpModifier {
                category,
                value: value_or_formula(amount.try_into()?)?,
            },
            "modificateur_moral_max" => Self::MaxMoraleModifier {
                category,
                value: value_or_formula(amount.try_into()?)?,
            },
            _ => bail!("Type d'effet d'état non supporté: {kind}"),
        })
    }

    /// Returns this effect as a categorized modifier when it targets `attribute`; otherwise returns `None`.
    pub fn attribute_modifier(&self, attribute: &str, formula: &mut FormulaEngine, env: &Env) -> Result<Option<Modifier>> {
        match self {
            Self::AttributeModifier {
                attribute: target,
                category,
                value,
            } => {
                // compute deferred value for the attribute
                let mut attrs = Vec::new();
                for deferred in target.iter() {
                    match deferred {
                        Deferred::Literal(s) => {attrs.push(s.clone());},
                        Deferred::Formula(f) => {
                            let evaled = formula.resolve_formula(f, env)?;
                            match evaled {
                                EvalValue::String(s) => {attrs.push(s.clone());},
                                EvalValue::Array(arr) => {
                                    for item in arr.iter() {
                                        if let EvalValue::String(s) = item {
                                            attrs.push(s.clone());
                                        } else {
                                            bail!("Les attributs doivent s'évaluer à des chaines de caractères, reçu: {:?}", item);
                                        }
                                    }
                                },
                                _ => bail!("Les attributs doivent s'évaluer à des chaines de caractères ou des tableaux de chaines, reçu: {:?}", evaled),
                            }
                        },
                    }
                }
                let attribute: Arc<str> = Arc::from(attribute);
                if attrs.contains(&attribute) {
                    let val = formula.resolve_formula_int(value, env)?;
                    Ok(Some(Modifier {
                        category: category.clone(),
                        value: val,
                    }))
                } else {
                    Ok(None)
                }
            },
            _ => Ok(None),
        }
    }
    /// Returns this effect as a categorized check modifier when its optional filter matches.
    ///
    /// `attribute` and `traits` are exposed as `test`, condition data is added to the environment, and `personnage` contains the current character snapshot.
    ///
    /// # Errors
    ///
    /// Propagates check-filter or formula evaluation errors.
    pub fn check_modifier(
        &self,
        attribute: Option<Arc<str>>,
        traits: &[Arc<str>],
        condition_data: &IndexMap<EvalKey, EvalValue>,
        character: &Character,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Option<Modifier>> {
        match self {
            Self::CheckModifier {
                category,
                value,
                filter,
            } => {
                let mut env = Env::from_arc(dict_to_map(condition_data.clone())?.into())
                    .expand(mk_character_base_env(character, formula, env)?)
                    .with_parent(env);
                {
                    let mut check = IndexMap::new();
                    check.insert(EvalKey("attribut".into()), match attribute.clone() {
                        Some(attr) => EvalValue::String(attr.clone()),
                        None => EvalValue::Null,
                    });
                    check.insert(EvalKey("traits".into()), EvalValue::Array(traits.iter().cloned().map(|s| EvalValue::String(s.into())).collect()));
                    env.insert("test", EvalValue::Object(Arc::from(check)));
                }
                if let Some(filter) = filter {
                    if !filter.matches(attribute, traits, &env, formula)? {
                        return Ok(None);
                    }
                }
                Ok(Some(Modifier {
                    category: category.clone(),
                    value: formula.resolve_formula_int(value, &env)?,
                }))
            }
            _ => Ok(None),
        }
    }
    /// Returns this effect as a maximum-HP modifier, or `None` for another effect variant.
    pub fn max_hp_modifier(&self, formula: &mut FormulaEngine, env: &Env) -> Result<Option<Modifier>> {
        match self {
            Self::MaxHpModifier { category, value } => Ok(Some(Modifier {
                category: category.clone(),
                value: formula.resolve_formula_int(value, env)?,
            })),
            _ => Ok(None),
        }
    }
    /// Returns this effect as a maximum-morale modifier, or `None` for another effect variant.
    pub fn max_morale_modifier(&self, formula: &mut FormulaEngine, env: &Env) -> Result<Option<Modifier>> {
        match self {
            Self::MaxMoraleModifier { category, value } => Ok(Some(Modifier {
                category: category.clone(),
                value: formula.resolve_formula_int(value, env)?,
            })),
            _ => Ok(None),
        }
    }

}

/// Controls whether an effect targets the first matching value or every match.
#[derive(Clone, Copy, Debug)]
pub enum Selection {
    All,
    First,
}

impl Selection {
    /// Parses the complete input into its typed representation.
    fn parse(value: Option<&JsonValue>, context: &str) -> Result<Self> {
        match value.and_then(JsonValue::as_str).unwrap_or("tous") {
            "tous" => Ok(Self::All),
            "premier" => Ok(Self::First),
            value => bail!("Type de sélection non supporté pour {context}: {value}"),
        }
    }

    fn parse_dict(value: Option<&EvalValue>, context: &str) -> Result<Self> {
        match value {
            None => Ok(Self::All),
            Some(EvalValue::String(s)) => match s.as_ref() {
                "tous" => Ok(Self::All),
                "premier" => Ok(Self::First),
                value => bail!("Type de sélection non supporté pour {context}: {value}"),
            },
            Some(_) => bail!("Type de sélection non supporté pour {context}: doit être une chaîne"),
        }
    }

    /// Applies the operation to the supplied runtime state.
    fn apply<T>(&self, mut values: Vec<T>) -> Vec<T> {
        if matches!(self, Self::First) {
            values.truncate(1);
        }
        values
    }
}

/// A normalized executable unit used by pages, connections, and global phases.
///
/// A cell may contain narrative text, effects, a conditional guard, or any
/// combination of those fields. `id` is intended as a stable machine-facing
/// identifier, while JSON field `libelle` is a short author-facing description used by the
/// Forge. Neither field changes runtime behavior.
#[derive(Clone, Debug)]
pub struct Cell {
    pub id: Option<Arc<str>>,
    pub label: Option<Arc<str>>,
    pub effects: Vec<Deferred<PageEffect>>,
    pub trigger: Option<Conditional>,
    pub images: Vec<Deferred<Arc<str>>>,
    pub text: Option<EvalValue>,
}

/// Reports whether a cell applied and the results it produced.
pub struct CellResult {
    pub applied: bool,
    pub results: Vec<PageEffectResult>,
}

impl Cell {
    /// Parses an optional array of cells.
    ///
    /// Missing or `null` values produce an empty vector.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not an array or a contained cell is malformed.
    pub fn parse_many(value: Option<&JsonValue>, context: &str) -> Result<Vec<Deferred<Self>>> {
        match value {
            None | Some(JsonValue::Null) => Ok(Vec::new()),
            Some(JsonValue::String(s)) => { // single formula form
                let formula = value_or_formula(EvalValue::String(s.clone().into())).with_context(|| "Erreur en parsant une cellule")?;
                match formula {
                    EvalValue::Func(_) => Ok(vec![Deferred::Formula(formula)]),
                    other => bail!("Une cellule sous forme de formule doit être une fonction, pas {other:?}"),
                }
            },
            Some(JsonValue::Array(values)) => values
                .iter()
                .map(|value| match value {
                    JsonValue::Object(_) => Ok(Deferred::Literal(Self::from_json(value, context).with_context(|| "Erreur en parsant une cellule")?)),
                    JsonValue::String(s) => {
                        let formula = value_or_formula(EvalValue::String(s.clone().into())).with_context(|| "Erreur en parsant une cellule")?;
                        match formula {
                            EvalValue::Func(_) => Ok(Deferred::Formula(formula)),
                            other => bail!("Une cellule sous forme de formule doit être une fonction, pas {other:?}"),
                        }
                    },
                    other => bail!("{context} doit être un tableau de cellules ou de formules, pas {other:?}"),
                })
                .collect::<Result<Vec<_>>>(),
            Some(JsonValue::Object(obj)) => Ok(vec![Deferred::Literal(Self::from_json(&JsonValue::Object(obj.clone()), context).with_context(|| "Erreur en parsant une cellule")?)]),
            Some(_) => bail!("{context} doit être un tableau de cellules ou de formules"),
        }
    }
    /// Parses an optional array of cells from a formula-produced value.
    pub fn parse_many_from_value(value: &EvalValue, context: &str) -> Result<Vec<Deferred<Self>>> {
        match value {
            EvalValue::Null => Ok(Vec::new()),
            EvalValue::Func(_) => Ok(vec![Deferred::Formula(value.clone())]),
            EvalValue::Array(values) => values
                .iter()
                .map(|value| match value {
                    EvalValue::Object(_) => Ok(Deferred::Literal(Self::from_dict(value).with_context(|| "Erreur en parsant une cellule")?)),
                    EvalValue::Func(_) => Ok(Deferred::Formula(value.clone())),
                    other => bail!("{context} doit être un tableau de cellules ou de formules, pas {other:?}"),
                })
                .collect::<Result<Vec<_>>>(),
            EvalValue::Object(_) => Ok(vec![Deferred::Literal(Self::from_dict(value).with_context(|| "Erreur en parsant une cellule")?)]),
            _ => bail!("{context} doit être un tableau de cellules ou de formules"),
        }
    }

    /// Parses one cell from its JSON representation.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    ///
    /// # Arguments
    ///
    /// - `value` — Raw cell JSON object to parse.
    /// - `context` — Human-readable tale context included in errors.
    ///
    /// # Errors
    ///
    /// Returns an error when required cell fields, effects, or conditions are malformed.
    pub fn from_json(value: &JsonValue, context: &str) -> Result<Self> {
        let data = object(value, context)?;
        /*
        if data.contains_key("label") {
            bail!(
                "Le champ anglais `label` n'appartient pas à la syntaxe française v2; utilisez `libelle`"
            );
        }
        */
        let effects = data.get("effets");
        let effects = PageEffect::parse_many(effects)
            .with_context(|| format!("Erreur en parsant les effets de {context}"))?;
        let images = match data.get("images") {
            None | Some(JsonValue::Null) => Vec::new(),
            Some(JsonValue::Array(values)) => {
                let mut def = Vec::new();
                for value in values {
                    match value {
                        JsonValue::Null => {},
                        JsonValue::String(s) => {
                            let formula = value_or_formula(s.clone().into())
                                .with_context(|| "Erreur en parsant une formule")?;
                            match formula {
                                EvalValue::Func(_) => def.push(Deferred::Formula(formula)),
                                EvalValue::String(s) => def.push(Deferred::Literal(s)),
                                other => unreachable!("La formule doit s'évaluer en str ou list de str, pas {other:?}"),
                            }
                        },
                        other => bail!("Le champ `images` d'une cellule doit être une str, une list de str ou de formules, ou une formule, pas {other:?}\nLes formules doivent s'évaluer en str ou list de str."),
                    }
                }
                def
            },
            Some(JsonValue::String(s)) => {
                let formula = value_or_formula(s.clone().into())
                    .with_context(|| "Erreur en parsant une formule")?;
                match formula {
                    EvalValue::Func(_) => vec![Deferred::Formula(formula)],
                    EvalValue::String(s) => vec![Deferred::Literal(s)],
                    other => unreachable!("La formule doit s'évaluer en str ou list de str, pas {other:?}"),
                }
            },
            other => {
                bail!("Le champ `images` d'une cellule doit être une str, une list de str ou de formules, ou une formule, pas {other:?}\nLes formules doivent s'évaluer en str ou list de str.");
            },
        };
        let trigger = match data.get("conditions") {
            None | Some(JsonValue::Null) => None,
            Some(JsonValue::Array(values)) if values.is_empty() => None,
            Some(JsonValue::Object(values)) if values.is_empty() => None,
            Some(value) => Some(Conditional::from_json(value)?),
        };
        let text = optional_string(data, "texte")?
            .map(Arc::from)
            .map(|s| split_fmt_str(&s))
            .transpose()?;
        let id = optional_string(data, "id")?.map(Arc::from);
        let label = optional_string(data, "libelle")?.map(Arc::from);
        if effects.is_empty() && text.is_none() && trigger.is_none() && images.is_empty() {
            bail!("{context} doit contenir `texte`, `effets`, `conditions` ou `images`");
        }
        Ok(Self {
            id,
            label,
            effects,
            trigger,
            text,
            images,
        })
    }
    /// Builds a cell from a formula-produced dictionary.
    ///
    /// Explicit `null` optional fields are treated as absent.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed effects, conditions, text, or metadata.
    pub fn from_dict(data: &EvalValue) -> Result<Self> {
        let data = match data {
            EvalValue::Object(map) => map,
            _ => bail!("Une cellule doit être un objet"),
        };
        let effects = PageEffect::parse_many_from_value(&data.get(&EvalKey::from("effets")).unwrap_or(&EvalValue::Null))
            .with_context(|| "Erreur en parsant une cellule")?;
        let trigger = match data.get(&EvalKey::from("conditions")) {
            None | Some(EvalValue::Null) => None,
            Some(EvalValue::Array(values)) if values.is_empty() => None,
            Some(EvalValue::Object(values)) if values.is_empty() => None,
            Some(value) => Some(Conditional::from_dict(&value)?),
            //_ => bail!("Le champ `conditions` d'une cellule doit être un tableau ou un objet"),
        };
        let text = match data.get(&EvalKey::from("texte")) {
            None | Some(EvalValue::Null) => None,
            Some(EvalValue::String(s)) => Some(EvalValue::String(s.clone())),
            Some(EvalValue::Func(f)) => Some(EvalValue::Func(f.clone())),
            _ => bail!("Le champ `texte` d'une cellule doit être une chaîne de caractères"),
        };
        let id = match data.get(&EvalKey::from("id")) {
            None | Some(EvalValue::Null) => None,
            Some(EvalValue::String(s)) => Some(s.clone()),
            _ => bail!("Le champ `id` d'une cellule doit être une chaîne de caractères"),
        };
        let label = match data.get(&EvalKey::from("libelle")) {
            None | Some(EvalValue::Null) => None,
            Some(EvalValue::String(s)) => Some(s.clone()),
            _ => bail!("Le champ `libelle` d'une cellule doit être une chaîne de caractères"),
        };
        let images = match data.get(&EvalKey::from("images")) {
            None | Some(EvalValue::Null) => Vec::new(),
            Some(EvalValue::Array(arr)) => {
                let mut def = Vec::new();
                for item in arr.iter() {
                    match item {
                        EvalValue::String(s) => def.push(Deferred::Literal(s.clone())),
                        EvalValue::Func(_) => def.push(Deferred::Formula(item.clone())),
                        other => bail!("Le champ `images` d'une cellule doit être une str ou une fonction, reçu {:?}", other),
                    }
                }
                def
            },
            Some(EvalValue::Func(f)) => vec![Deferred::Formula(EvalValue::Func(f.clone()))],
            Some(EvalValue::String(s)) => vec![Deferred::Literal(s.clone())],
            other => bail!("Le champ `images` d'une cellule doit être une list, une str ou une fonction, reçu {:?}", other),
        };
        if effects.is_empty() && text.is_none() && trigger.is_none() && images.is_empty() {
            bail!("Une cellule doit contenir `texte`, `effets`, `conditions` ou `images`");
        }
        Ok(Self {
            id,
            label,
            effects,
            trigger,
            text,
            images,
        })
    }

    /// Returns whether this cell should execute in the current context.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `character` — Current mutable or read-only character state.
    /// - `page_tags` — Immutable tags of the current page.
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    ///
    /// # Errors
    ///
    /// Propagates conditional or formula-evaluation errors.
    pub fn applies(
        &self,
        character: &Character,
        page_tags: Arc<IndexMap<EvalKey, EvalValue>>,
        connection_tags: Arc<IndexMap<EvalKey, EvalValue>>,
        tale: &Tale,
        formula: &mut FormulaEngine,
    ) -> Result<bool> {
        match &self.trigger {
            None => Ok(true),
            Some(trigger) => trigger.matches(character, page_tags, connection_tags, tale, formula, &Env::new()),
        }
    }

    /// Returns whether this cell executes when an additional lifecycle environment is present.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `character` — Current mutable or read-only character state.
    /// - `page_tags` — Immutable tags of the current page.
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `additional_env` — Temporary lifecycle bindings merged into formula evaluation.
    ///
    /// # Errors
    ///
    /// Propagates conditional or formula-evaluation errors.
    pub fn applies_with_env(
        &self,
        character: &Character,
        page_tags: Arc<IndexMap<EvalKey, EvalValue>>,
        connection_tags: Arc<IndexMap<EvalKey, EvalValue>>,
        tale: &Tale,
        formula: &mut FormulaEngine,
        additional_env: &Env,
    ) -> Result<bool> {
        match &self.trigger {
            None => Ok(true),
            Some(trigger) => {
                trigger.matches(character, page_tags, connection_tags, tale, formula, additional_env)
            }
        }
    }

    /// Applies this cell's effects without rendering its optional text.
    ///
    /// Effects are processed in deterministic order. The return value carries any visible
    /// messages, poll result, defeat state, or forced destination produced by the operation.
    ///
    /// # Arguments
    ///
    /// - `character` — Current mutable or read-only character state.
    /// - `tale` — Loaded tale definition used to resolve IDs and lifecycle behavior.
    /// - `page_tags` — Immutable tags of the current page.
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `term` — Active player terminal used for prompts or rendered output.
    /// - `interaction` — Input adapter used to present polls.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
    /// invalid, or a delegated parser/evaluator fails.
    pub fn apply(
        &self,
        character: &mut Character,
        tale: &Tale,
        formula: &mut FormulaEngine,
        env: &Env,
        term: &Terminal,
        interaction: &mut Interaction,
    ) -> Result<CellResult> {
        // extract page and connection tags from the environment for conditional evaluation
        let page_tags = env.get("page").and_then(|v| match v {
            EvalValue::Object(map) => Some(map.clone()),
            _ => None,
        }).unwrap_or_else(|| IndexMap::new().into());
        let connection_tags = env.get("connexion").and_then(|v| match v {
            EvalValue::Object(map) => Some(map.clone()),
            _ => None,
        }).unwrap_or_else(|| IndexMap::new().into());
        if !self.applies_with_env(character, page_tags, connection_tags, tale, formula, env)? {
            return Ok(CellResult {
                applied: false,
                results: Vec::new(),
            });
        }

        let mut results = Vec::new();

        let denv = mk_character_base_env(character, formula, &env)?.with_parent(&env);
        let effects = PageEffect::eval_many(&self.effects, formula, &denv)
            .with_context(|| "Erreur en évaluant les effets lors de l'application d'une cellule")?;

        for res in effects
            .iter()
            .map(|effect| effect.apply(character, tale, formula, &env, term, interaction))
        {
            results.push(res?);
        }
        Ok(CellResult {
            applied: true,
            results,
        })
    }
}

pub fn page_effect_builder(val: &EvalValue, formula: &mut FormulaEngine, env: &Env) -> Result<Vec<PageEffect>> {
    match val {
        EvalValue::Func(_) => {
            let res = val
                .clone()
                .call(&[], formula, env)
                .context("Évaluation de la formule qui construit les effets de la cellule")?;
            match res {
                EvalValue::Array(values) => values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        PageEffect::from_dict(value).with_context(|| {
                            format!(
                                "Construction de l'effet dynamique #{}, produit sous la forme {}",
                                index + 1,
                                value.diagnostic()
                            )
                        })
                    })
                    .collect::<Result<Vec<_>>>(),
                object @ EvalValue::Object(_) => Ok(vec![PageEffect::from_dict(&object)
                    .with_context(|| {
                        format!(
                            "Construction de l'effet dynamique produit sous la forme {}",
                            object.diagnostic()
                        )
                    })?]),
                other => bail!(
                    "La formule du champ `effets` a produit {}, mais elle doit produire un objet d'effet ou un tableau d'objets d'effet",
                    other.diagnostic()
                ),
            }
        },
        other => bail!(
            "Le champ `effets` contient {}, mais une fonction de formule était attendue à cet endroit",
            other.diagnostic()
        ),
    }
}

/// Stores one displayed label and resolved value for a choice poll.
#[derive(Debug, Clone)]
pub struct PollChoice {
    pub value: EvalValue,
    pub label: EvalValue,
}
#[derive(Debug, Clone)]
pub enum Deferred<T> {
    Literal(T),
    Formula(EvalValue),
}
impl<T: Clone> Deferred<T> {
    pub fn build(self, builder: &mut dyn FnMut(&EvalValue) -> Result<T>) -> Result<T> {
        match self {
            Deferred::Literal(lit) => Ok(lit),
            Deferred::Formula(val) => builder(&val),
        }
    }
}

/// Represents one executable mutation or poll in a page-effect sequence.
#[derive(Clone, Debug)]
pub enum PageEffect {
    AddCondition {
        condition_id: EvalValue,
        data: EvalValue,
        notification: Option<EvalValue>,
    },
    RemoveCondition {
        filter: ConditionFilter,
        selection: Selection,
        notification: Option<EvalValue>,
    },
    ModifyCondition {
        filter: ConditionFilter,
        data: EvalValue,
        selection: Selection,
        notification: Option<EvalValue>,
    },
    SetTag {
        filter: TagFilter,
        path: Option<EvalValue>,
        create_path: bool,
        value: EvalValue,
        selection: Selection,
        notification: Option<EvalValue>,
    },
    IncrementTag {
        filter: TagFilter,
        path: Option<EvalValue>,
        create_path: bool,
        amount: EvalValue,
        selection: Selection,
        notification: Option<EvalValue>,
    },
    AddItem {
        item_id: EvalValue,
        quantity: EvalValue,
        data: EvalValue,
        notification: Option<EvalValue>,
    },
    RemoveItem {
        filter: ItemFilter,
        quantity: EvalValue,
        selection: Selection,
        notification: Option<EvalValue>,
    },
    ModifyItem {
        filter: ItemFilter,
        data: EvalValue,
        selection: Selection,
        notification: Option<EvalValue>,
    },
    ModifyHp {
        amount: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    ModifyAttribute {
        attribute: Arc<str>,
        value: EvalValue,
        notification: Option<EvalValue>,
    },
    SetAttribute {
        attribute: Arc<str>,
        value: EvalValue,
        notification: Option<EvalValue>,
    },
    SetHp {
        value: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    SetMaxHp {
        value: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    ModifyMaxHp {
        value: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    SetMorale {
        value: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    SetMaxMorale {
        value: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    ModifyMaxMorale {
        value: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    ModifyMorale {
        value: EvalValue,
        death: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    ModifyName {
        value: EvalValue,
        notification: Option<EvalValue>,
    },
    RemoveTag {
        filter: TagFilter,
        selection: Selection,
        notification: Option<EvalValue>,
        path: Option<EvalValue>,
    },
    Poll {
        destination: EvalValue,
        prompt: EvalValue,
        data_type: PollRequestDataType,
        choices: Option<Deferred<Vec<PollChoice>>>,
        default: Option<EvalValue>,
        notification: Option<EvalValue>,
    },
    Redirection {
        destination: EvalValue,
        notification: Option<EvalValue>,
    },
}

impl PageEffect {
    /// Returns the tale-syntax name used to identify this effect in runtime errors.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AddCondition { .. } => "ajouter_etat",
            Self::RemoveCondition { .. } => "retirer_etat",
            Self::ModifyCondition { .. } => "modifier_etat",
            Self::SetTag { .. } => "definir_tag",
            Self::IncrementTag { .. } => "incrementer_tag",
            Self::AddItem { .. } => "ajouter_objet",
            Self::RemoveItem { .. } => "retirer_objet",
            Self::ModifyItem { .. } => "modifier_objet",
            Self::ModifyHp { .. } => "modifier_pv",
            Self::ModifyAttribute { .. } => "modifier_attribut",
            Self::SetAttribute { .. } => "definir_attribut",
            Self::SetHp { .. } => "definir_pv",
            Self::SetMaxHp { .. } => "definir_pv_max",
            Self::ModifyMaxHp { .. } => "modifier_pv_max",
            Self::SetMorale { .. } => "definir_moral",
            Self::SetMaxMorale { .. } => "definir_moral_max",
            Self::ModifyMaxMorale { .. } => "modifier_moral_max",
            Self::ModifyMorale { .. } => "modifier_moral",
            Self::ModifyName { .. } => "modifier_nom",
            Self::RemoveTag { .. } => "retirer_tag",
            Self::Poll { .. } => "demander",
            Self::Redirection { .. } => "rediriger",
        }
    }

    pub fn parse_many(value: Option<&JsonValue>) -> Result<Vec<Deferred<Self>>> {
        match value {
            None | Some(JsonValue::Null) => Ok(Vec::new()),
            Some(JsonValue::Array(values)) => values
                .iter()
                .map(|value| match value {
                    JsonValue::Object(_) => Ok(Deferred::Literal(Self::from_json(value)?)),
                    JsonValue::String(s) => {
                        let formula = value_or_formula(s.as_str().into())
                            .with_context(|| "erreur en parsant une formule")?;
                        match formula {
                            EvalValue::Func(_) => Ok(Deferred::Formula(formula)),
                            other => bail!("Une cellule sous forme de formule doit être une fonction, pas {other:?}"),
                        }
                    },
                    other => bail!("Le champ `effets` d'une cellule doit être un tableau d'objets ou de formules, pas {other:?}"),
                })
                .collect::<Result<Vec<_>>>(),
            Some(JsonValue::Object(_obj)) => Ok(vec![Deferred::Literal(Self::from_json(value.unwrap())?)]),
            Some(JsonValue::String(s)) => {
                let formula = value_or_formula(s.as_str().into())
                    .with_context(|| "erreur en parsant une formule")?;
                match formula {
                    EvalValue::Func(_) => Ok(vec![Deferred::Formula(formula)]),
                    other => bail!("Une cellule sous forme de formule doit être une fonction, pas {other:?}"),
                }
            },
            Some(other) => bail!("Le champ `effets` d'une cellule doit être un tableau ou un objet, pas {other:?}"),
        }
    }
    pub fn parse_many_from_value(value: &EvalValue) -> Result<Vec<Deferred<Self>>> {
        match value {
            EvalValue::Null => Ok(Vec::new()),
            EvalValue::Func(_) => Ok(vec![Deferred::Formula(value.clone())]),
            EvalValue::Array(values) => values
                .iter()
                .map(|value| match value {
                    EvalValue::Object(_) => Ok(Deferred::Literal(Self::from_dict(value)?)),
                    EvalValue::Func(_) => Ok(Deferred::Formula(value.clone())),
                    other => bail!("Le champ `effets` d'une cellule doit être un tableau d'objets ou de formules, pas {other:?}"),
                })
                .collect::<Result<Vec<_>>>(),
            EvalValue::Object(_) => Ok(vec![Deferred::Literal(Self::from_dict(value)?)]),
            _ => bail!("Le champ `effets` d'une cellule doit être un tableau d'objets ou de formules"),
        }
    }

    pub fn eval_many(value: &[Deferred<Self>], formula: &mut FormulaEngine, env: &Env) -> Result<Vec<Self>> {
        let mut effects = Vec::new();
        for deferred in value {
            match deferred {
                Deferred::Literal(lit) => effects.push(lit.clone()),
                Deferred::Formula(f @ EvalValue::Func(_)) => {
                    let res = f.clone().call(&[], formula, env)
                        .with_context(|| "Évaluation de la formule qui construit les effets de la cellule")?;
                    match res {
                        EvalValue::Array(values) => {
                            for (index, value) in values.iter().enumerate() {
                                effects.push(PageEffect::from_dict(value).with_context(|| {
                                    format!(
                                        "Construction de l'effet dynamique #{}, produit sous la forme {}",
                                        index + 1,
                                        value.diagnostic()
                                    )
                                })?);
                            }
                        },
                        object @ EvalValue::Object(_) => {
                            effects.push(PageEffect::from_dict(&object).with_context(|| {
                                format!(
                                    "Construction de l'effet dynamique produit sous la forme {}",
                                    object.diagnostic()
                                )
                            })?);
                        },
                        other => bail!(
                            "La formule du champ `effets` a produit {}, mais elle doit produire un objet d'effet ou un tableau d'objets d'effet",
                            other.diagnostic()
                        ),
                    }
                },
                Deferred::Formula(other) => bail!("Une cellule sous forme de formule doit être une fonction, pas {other:?}"),
            }
        }
        Ok(effects)
    }

    /// Parses one atomic page effect and preserves formula-bearing values for execution.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown effect types, missing required fields, malformed filters/paths, or incompatible literal values.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        let data = object(value, "Un effet de page")?;
        let kind = required_string(data, "type", "Un effet de page")?;
        let notification = optional_string(data, "notification")?
            .map(Arc::from)
            .map(|s| split_fmt_str(&s))
            .transpose()?;
        let effect = match kind.as_str() {
            "ajouter_etat" => Self::AddCondition {
                condition_id: value_or_formula(required_string(data, "etat", "AJOUTER_ETAT")?.into())?,
                data: collect_data(data.get("donnees"))
                    .with_context(|| "Erreur en collectant les donnees de ajouter etat")?.into(),
                notification,
            },
            "retirer_etat" => Self::RemoveCondition {
                filter: ConditionFilter::from_json(
                    data.get("filtre")
                        .ok_or_else(|| anyhow!("RETIRER_ETAT doit inclure un filtre"))?,
                )?,
                selection: Selection::parse(data.get("selection"), "retirer_etat")?,
                notification,
            },
            "modifier_etat" => Self::ModifyCondition {
                filter: ConditionFilter::from_json(
                    data.get("filtre")
                        .ok_or_else(|| anyhow!("MODIFIER_ETAT doit inclure un filtre"))?,
                )?,
                data: collect_data(data.get("donnees"))
                    .with_context(|| "Erreur en collectant les donnees de modifier etat")?.into(),
                selection: Selection::parse(data.get("selection"), "modifier_etat")?,
                notification,
            },
            "definir_tag" => {
                let filter = TagFilter::from_json(
                    data.get("filtre")
                        .ok_or_else(|| anyhow!("DEFINIR_TAG doit inclure un filtre"))?,
                )?;
                let targetted = if let TagFilter::Id(_) = filter { true } else { false };
                let create_path = match data.get("creer_chemin").cloned() {
                    Some(JsonValue::Bool(b)) => b,
                    None => targetted,
                    other => bail!("DEFINIR_TAG: creer_chemin doit être un booléen, pas {other:?}")
                };
                let path = match data.get("chemin") {
                    Some(JsonValue::Null) | None => None,
                    Some(JsonValue::String(s)) if s.is_empty() => None,
                    Some(JsonValue::String(s)) => Some(s.clone()),
                    Some(JsonValue::Number(n)) if n.is_i64() => Some(n.to_string()),
                    Some(JsonValue::Array(arr)) => {
                        let mut path_parts = Vec::new();
                        for part in arr {
                            match part {
                                JsonValue::String(s) => path_parts.push(s.clone()),
                                JsonValue::Number(n) if n.is_i64() => path_parts.push(n.to_string()),
                                _ => bail!("DEFINIR_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {part:?}"),
                            }
                        }
                        Some(path_parts.join("."))
                    },
                    other => bail!("DEFINIR_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {other:?}"),
                }.map(Arc::from)
                .map(|s| split_fmt_str(&s))
                .transpose()?;
                Self::SetTag {
                    filter,
                    path,
                    create_path,
                    value: value_or_formula(
                        json_to_eval_value(&data.get("valeur")
                            .cloned()
                            .unwrap_or(JsonValue::Null)
                        )?
                    )?,
                    selection: Selection::parse(data.get("selection"), "definir_tag")?,
                    notification,
                }
            },
            "incrementer_tag" => {
                let filter = TagFilter::from_json(
                    data.get("filtre")
                        .ok_or_else(|| anyhow!("INCREMENTER_TAG doit inclure un filtre"))?,
                )?;
                let targetted = if let TagFilter::Id(_) = filter { true } else { false };
                let create_path = match data.get("creer_chemin").cloned() {
                    Some(JsonValue::Bool(b)) => b,
                    None => targetted,
                    other => bail!("INCREMENTER_TAG: creer_chemin doit être un booléen, pas {other:?}")
                };
                let path = match data.get("chemin") {
                    Some(JsonValue::Null) | None => None,
                    Some(JsonValue::String(s)) if s.is_empty() => None,
                    Some(JsonValue::String(s)) => Some(s.clone()),
                    Some(JsonValue::Number(n)) if n.is_i64() => Some(n.to_string()),
                    Some(JsonValue::Array(arr)) => {
                        let mut path_parts = Vec::new();
                        for part in arr {
                            match part {
                                JsonValue::String(s) => path_parts.push(s.clone()),
                                JsonValue::Number(n) if n.is_i64() => path_parts.push(n.to_string()),
                                _ => bail!("INCREMENTER_TAG: chemin doit être une chaîne, un entier ou un tableau de chaînes ou d'entier, pas {part:?}"),
                            }
                        }
                        Some(path_parts.join("."))
                    },
                    other => bail!("INCREMENTER_TAG: chemin doit être une chaîne, un entier ou un tableau de chaînes ou d'entier, pas {other:?}"),
                }.map(Arc::from)
                .map(|s| split_fmt_str(&s))
                .transpose()?;
                Self::IncrementTag {
                    filter,
                    path,
                    create_path,
                    amount: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(1)))?)?,
                    selection: Selection::parse(data.get("selection"), "incrementer_tag")?,
                    notification,
                }
            },
            "ajouter_objet" => Self::AddItem {
                item_id: value_or_formula(required_string(data, "objet", "AJOUTER_OBJET")?.into())?,
                quantity: value_or_formula(json_to_eval_value(&value_or(data, "quantite", JsonValue::from(1)))?)?,
                data: collect_data(data.get("donnees"))
                    .with_context(|| "Erreur en collectant les donnees de ajouter objet")?.into(),
                notification,
            },
            "retirer_objet" => Self::RemoveItem {
                filter: ItemFilter::from_json(
                    data.get("filtre")
                        .ok_or_else(|| anyhow!("RETIRER_OBJET doit inclure un filtre"))?,
                )?,
                quantity: value_or_formula(json_to_eval_value(&value_or(data, "quantite", JsonValue::from(1)))?)?,
                selection: Selection::parse(data.get("selection"), "retirer_objet")?,
                notification,
            },
            "modifier_objet" => Self::ModifyItem {
                filter: ItemFilter::from_json(
                    data.get("filtre")
                        .ok_or_else(|| anyhow!("MODIFIER_OBJET doit inclure un filtre"))?,
                )?,
                data: collect_data(data.get("donnees"))
                    .with_context(|| "Erreur en collectant les donnees de modifier objet")?.into(),
                selection: Selection::parse(data.get("selection"), "modifier_objet")?,
                notification,
            },
            "modifier_pv" => Self::ModifyHp {
                amount: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s))
                    .transpose()?,
                notification,
            },
            "modifier_attribut" => Self::ModifyAttribute {
                attribute: required_string(data, "attribut", "MODIFIER_ATTRIBUT")?.into(),
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                notification,
            },
            "definir_attribut" => Self::SetAttribute {
                attribute: required_string(data, "attribut", "DEFINIR_ATTRIBUT")?.into(),
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                notification,
            },
            "definir_pv" => Self::SetHp {
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "definir_pv_max" => Self::SetMaxHp {
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_pv_max" => Self::ModifyMaxHp {
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "definir_moral" => Self::SetMorale {
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "definir_moral_max" => Self::SetMaxMorale {
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_moral_max" => Self::ModifyMaxMorale {
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_moral" => Self::ModifyMorale {
                value: value_or_formula(json_to_eval_value(&value_or(data, "valeur", JsonValue::from(0)))?)?,
                death: optional_string(data, "defaite")?.map(Arc::from).map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_nom" => Self::ModifyName {
                value: split_fmt_str(&required_string(data, "valeur", "MODIFIER_NOM")?)?,
                notification,
            },
            "retirer_tag" => Self::RemoveTag {
                filter: TagFilter::from_json(
                    data.get("filtre")
                        .ok_or_else(|| anyhow!("RETIRER_TAG doit inclure un filtre"))?,
                )?,
                path: match data.get("chemin") {
                    Some(JsonValue::Null) | None => None,
                    Some(JsonValue::String(s)) if s.is_empty() => Some("".into()),
                    Some(JsonValue::String(s)) => Some(s.clone().into()),
                    Some(JsonValue::Number(n)) if n.is_i64() => Some(n.to_string().into()),
                    Some(JsonValue::Array(arr)) => {
                        let mut path_parts = Vec::new();
                        for part in arr {
                            match part {
                                JsonValue::String(s) => path_parts.push(s.clone()),
                                JsonValue::Number(n) if n.is_i64() => path_parts.push(n.to_string()),
                                _ => bail!("RETIRER_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {part:?}"),
                            }
                        }
                        Some(path_parts.join(".").into())
                    },
                    other => bail!("RETIRER_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {other:?}"),
                },
                selection: Selection::parse(data.get("selection"), "retirer_tag")?,
                notification,
            },
            "demander" => Self::Poll {
                destination: required_string(data, "tag", "DEMANDER")?.into(),
                prompt: split_fmt_str(&required_string(data, "question", "DEMANDER")?)?,
                data_type: {
                    let type_str = data
                        .get("donnees")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("str");
                    PollRequestDataType::from_str(type_str)?
                },
                choices: match data.get("choix") {
                    Some(JsonValue::Null) | None => None,
                    Some(JsonValue::Array(arr)) => {
                        let mut choices = Vec::new();
                        for choice in arr {
                            let choice_data = object(choice, "Un choix de DEMANDER")?;
                            let value =
                                required_value(choice_data, "valeur", "Un choix de DEMANDER")?;
                            let label =
                                required_string(choice_data, "texte", "Un choix de DEMANDER")?;
                            choices.push(PollChoice {
                                value: value_or_formula(json_to_eval_value(&value)?)?,
                                label: split_fmt_str(&label)?,
                            });
                        }
                        if choices.is_empty() {
                            None
                        } else {
                            Some(Deferred::Literal(choices))
                        }
                    },
                    Some(JsonValue::String(s)) => {
                        let formula = value_or_formula(s.as_str().into())?;
                        if let EvalValue::Func(_) = formula {
                            Some(Deferred::Formula(formula))
                        } else {
                            bail!("Le champ `choix` de DEMANDER doit être un tableau, une formule ou null")
                        }
                    },
                    _ => bail!("Le champ `choix` de DEMANDER doit être un tableau, une formule ou null"),
                },
                default: match data.get("defaut") {
                    None | Some(JsonValue::Null) => None,
                    Some(value) => Some(value_or_formula(json_to_eval_value(&value)?)?),
                },
                notification,
            },
            "rediriger" => {
                let destination = required_string(data, "destination", "REDIRIGER")?.into();
                Self::Redirection {
                    destination,
                    notification,
                }
            },
            _ => bail!("Type d'effet de page non supporté: {kind}"),
        };
        Ok(effect)
    }
    /// Builds one atomic page effect from a formula-produced dictionary.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown effect types, missing required fields, or incompatible runtime values.
    pub fn from_dict(data: &EvalValue) -> Result<Self> {
        let data = match data {
            EvalValue::Object(map) => map,
            _ => bail!("Un effet de page doit être un objet"),
        };
        let kind = match data.get(&EvalKey::from("type")) {
            Some(EvalValue::String(s)) => s.as_ref(),
            _ => bail!("Un effet de page doit contenir un champ `type`"),
        };
        let notification = match data.get(&EvalKey::from("notification")) {
            Some(EvalValue::String(s)) => Some(s.clone()),
            Some(EvalValue::Null) | None => None,
            _ => bail!("Le champ `notification` d'un effet de page doit être une chaîne ou null"),
        }.map(|s| split_fmt_str(&s)).transpose()?;
        let effect = match kind {
            "ajouter_etat" => Self::AddCondition {
                condition_id: {
                    let data = data.get(&EvalKey::from("etat"))
                        .ok_or_else(|| anyhow!("AJOUTER_ETAT doit inclure un champ `etat`"))?
                        .clone();
                    match data {
                        EvalValue::String(_) => data,
                        EvalValue::Func(_) => data,
                        _ => bail!("Le champ `etat` d'AJOUTER_ETAT doit être une chaîne ou une formule"),
                    }
                },
                data: match data.get(&EvalKey::from("donnees")) {
                    Some(EvalValue::Object(obj)) => EvalValue::Object(obj.clone()),
                    Some(EvalValue::Null) | None => EvalValue::Object(IndexMap::new().into()),
                    Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
                    _ => bail!("Le champ `donnees` d'AJOUTER_ETAT doit être un objet, une formule, ou null"),
                },
                notification,
            },
            "retirer_etat" => Self::RemoveCondition {
                filter: ConditionFilter::from_dict(
                    data.get(&EvalKey::from("filtre"))
                        .ok_or_else(|| anyhow!("RETIRER_ETAT doit inclure un filtre"))?,
                )?,
                selection: Selection::parse_dict(data.get(&EvalKey::from("selection")), "retirer_etat")?,
                notification,
            },
            "modifier_etat" => Self::ModifyCondition {
                filter: ConditionFilter::from_dict(
                    data.get(&EvalKey::from("filtre"))
                        .ok_or_else(|| anyhow!("MODIFIER_ETAT doit inclure un filtre"))?,
                )?,
                data: match data.get(&EvalKey::from("donnees")) {
                    Some(EvalValue::Object(obj)) => EvalValue::Object(obj.clone()),
                    Some(EvalValue::Null) | None => EvalValue::Object(IndexMap::new().into()),
                    Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
                    _ => bail!("Le champ `donnees` de MODIFIER_ETAT doit être un objet, une formule, ou null"),
                },
                selection: Selection::parse_dict(data.get(&EvalKey::from("selection")), "modifier_etat")?,
                notification,
            },
            "definir_tag" => {
                let filter = TagFilter::from_dict(
                    data.get(&EvalKey::from("filtre"))
                        .ok_or_else(|| anyhow!("DEFINIR_TAG doit inclure un filtre"))?,
                )?;
                let targetted = if let TagFilter::Id(_) = filter { true } else { false };
                let create_path = match data.get(&EvalKey::from("creer_chemin")) {
                    Some(EvalValue::Bool(b)) => *b,
                    None => targetted,
                    other => bail!("DEFINIR_TAG: creer_chemin doit être un booléen, pas {other:?}")
                };
                let path = match data.get(&EvalKey::from("chemin")) {
                    Some(EvalValue::String(s)) if s.is_empty() => None,
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Array(arr)) => {
                        let mut path_parts = Vec::new();
                        for part in arr.iter() {
                            match part {
                                EvalValue::String(s) => path_parts.push(s.clone()),
                                _ => bail!("DEFINIR_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {part:?}"),
                            }
                        }
                        Some(path_parts.join(".").into())
                    },
                    None => None,
                    other => bail!("DEFINIR_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {other:?}"),
                }.map(|s| split_fmt_str(&s)).transpose()?;
                Self::SetTag {
                    filter,
                    path,
                    create_path,
                    value: data.get(&EvalKey::from("valeur"))
                        .cloned()
                        .unwrap_or(EvalValue::Null),
                    selection: Selection::parse_dict(data.get(&EvalKey::from("selection")), "definir_tag")?,
                    notification,
                }
            },
            "incrementer_tag" => {
                let filter = TagFilter::from_dict(
                    data.get(&EvalKey::from("filtre"))
                        .ok_or_else(|| anyhow!("INCREMENTER_TAG doit inclure un filtre"))?,
                )?;
                let targetted = if let TagFilter::Id(_) = filter { true } else { false };
                let create_path = match data.get(&EvalKey::from("creer_chemin")) {
                    Some(EvalValue::Bool(b)) => *b,
                    None => targetted,
                    other => bail!("INCREMENTER_TAG: creer_chemin doit être un booléen, pas {other:?}")
                };
                let path = match data.get(&EvalKey::from("chemin")) {
                    Some(EvalValue::String(s)) if s.is_empty() => None,
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Array(arr)) => {
                        let mut path_parts = Vec::new();
                        for part in arr.iter() {
                            match part {
                                EvalValue::String(s) => path_parts.push(s.clone()),
                                _ => bail!("INCREMENTER_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {part:?}"),
                            }
                        }
                        Some(path_parts.join(".").into())
                    },
                    None => None,
                    other => bail!("INCREMENTER_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {other:?}"),
                }.map(|s| split_fmt_str(&s)).transpose()?;
                Self::IncrementTag {
                    filter,
                    path,
                    create_path,
                    amount: data.get(&EvalKey::from("valeur"))
                        .cloned()
                        .unwrap_or(EvalValue::Int(1)),
                    selection: Selection::parse_dict(data.get(&EvalKey::from("selection")), "incrementer_tag")?,
                    notification,
                }
            },
            "ajouter_objet" => Self::AddItem {
                item_id:  {
                    let data = data.get(&EvalKey::from("objet")).cloned();
                    match data {
                        Some(EvalValue::String(_)) => data.unwrap(),
                        Some(EvalValue::Func(_)) => data.unwrap(),
                        _ => bail!("Le champ `objet` d'AJOUTER_OBJET doit être une chaîne ou une formule"),
                    }
                },
                quantity: data.get(&EvalKey::from("quantite"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(1)),
                data: match data.get(&EvalKey::from("donnees")) {
                    Some(EvalValue::Object(obj)) => EvalValue::Object(obj.clone()),
                    Some(EvalValue::Null) | None => EvalValue::Object(IndexMap::new().into()),
                    Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
                    _ => bail!("Le champ `donnees` d'AJOUTER_OBJET doit être un objet, une formule, ou null"),
                },
                notification,
            },
            "retirer_objet" => Self::RemoveItem {
                filter: ItemFilter::from_dict(
                    data.get(&EvalKey::from("filtre"))
                        .ok_or_else(|| anyhow!("RETIRER_OBJET doit inclure un filtre"))?,
                )?,
                quantity: data.get(&EvalKey::from("quantite"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(1)),
                selection: Selection::parse_dict(data.get(&EvalKey::from("selection")), "retirer_objet")?,
                notification,
            },
            "modifier_objet" => Self::ModifyItem {
                filter: ItemFilter::from_dict(
                    data.get(&EvalKey::from("filtre"))
                        .ok_or_else(|| anyhow!("MODIFIER_OBJET doit inclure un filtre"))?,
                )?,
                data: match data.get(&EvalKey::from("donnees")) {
                    Some(EvalValue::Object(obj)) => EvalValue::Object(obj.clone()),
                    Some(EvalValue::Null) | None => EvalValue::Object(IndexMap::new().into()),
                    Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
                    _ => bail!("Le champ `donnees` de MODIFIER_OBJET doit être un objet, une formule, ou null"),
                },
                selection: Selection::parse_dict(data.get(&EvalKey::from("selection")), "modifier_objet")?,
                notification,
            },
            "modifier_pv" => Self::ModifyHp {
                amount: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet MODIFIER_PV doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_attribut" => Self::ModifyAttribute {
                attribute: match data.get(&EvalKey::from("attribut")) {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("MODIFIER_ATTRIBUT doit inclure un champ `attribut`"),
                },
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                notification,
            },
            "definir_attribut" => Self::SetAttribute {
                attribute: match data.get(&EvalKey::from("attribut")) {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("DEFINIR_ATTRIBUT doit inclure un champ `attribut`"),
                },
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                notification,
            },
            "definir_pv" => Self::SetHp {
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet DEFINIR_PV doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "definir_pv_max" => Self::SetMaxHp {
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet DEFINIR_PV_MAX doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_pv_max" => Self::ModifyMaxHp {
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet MODIFIER_PV_MAX doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "definir_moral" => Self::SetMorale {
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet DEFINIR_MORAL doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "definir_moral_max" => Self::SetMaxMorale {
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet DEFINIR_MORAL_MAX doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_moral_max" => Self::ModifyMaxMorale {
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet MODIFIER_MORAL_MAX doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_moral" => Self::ModifyMorale {
                value: data.get(&EvalKey::from("valeur"))
                    .cloned()
                    .unwrap_or(EvalValue::Int(0)),
                death: match data.get(&EvalKey::from("defaite")) {
                    Some(EvalValue::String(s)) => Some(s.clone()),
                    Some(EvalValue::Null) | None => None,
                    _ => bail!("Le champ `defaite` d'un effet MODIFIER_MORAL doit être une chaîne ou null"),
                }.map(|s| split_fmt_str(&s)).transpose()?,
                notification,
            },
            "modifier_nom" => Self::ModifyName {
                value: match data.get(&EvalKey::from("valeur")) {
                    Some(EvalValue::String(s)) => split_fmt_str(&s)?,
                    _ => bail!("MODIFIER_NOM doit inclure un champ `valeur`"),
                },
                notification,
            },
            "retirer_tag" => Self::RemoveTag {
                filter: TagFilter::from_dict(
                    data.get(&EvalKey::from("filtre"))
                        .ok_or_else(|| anyhow!("RETIRER_TAG doit inclure un filtre"))?,
                )?,
                path: match data.get(&EvalKey::from("chemin")) {
                    Some(EvalValue::String(s)) if s.is_empty() => Some("".into()),
                    Some(EvalValue::String(s)) => Some(split_fmt_str(&s)?),
                    Some(EvalValue::Array(arr)) => {
                        let mut path_parts = Vec::new();
                        for part in arr.iter() {
                            match part {
                                EvalValue::String(s) => path_parts.push(s.clone()),
                                _ => bail!("RETIRER_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {part:?}"),
                            }
                        }
                        Some(path_parts.join(".").into())
                    },
                    None | Some(EvalValue::Null) => None,
                    other => bail!("RETIRER_TAG: chemin doit être une chaîne ou un tableau de chaînes, pas {other:?}"),
                },
                selection: Selection::parse_dict(data.get(&EvalKey::from("selection")), "retirer_tag")?,
                notification,
            },
            "demander" => Self::Poll {
                destination: match data.get(&EvalKey::from("tag")) {
                    Some(EvalValue::String(s)) => split_fmt_str(&s)?,
                    _ => bail!("DEMANDER doit inclure un champ `tag`"),
                },
                prompt: match data.get(&EvalKey::from("question")) {
                    Some(EvalValue::String(s)) => split_fmt_str(&s)?,
                    _ => bail!("DEMANDER doit inclure un champ `question`"),
                },
                data_type: {
                    let type_str = data
                        .get(&EvalKey::from("donnees"))
                        .and_then(|v| match v {
                            EvalValue::String(s) => Some(s.as_ref()),
                            _ => None,
                        })
                        .unwrap_or("str");
                    PollRequestDataType::from_str(type_str)?
                },
                choices: {
                    let choices_source = data.get(&EvalKey::from("choix")).and_then(|v| match v {
                        EvalValue::Array(_) => Some(v.clone()),
                        EvalValue::Func(_) => Some(v.clone()),
                        _ => None,
                    });
                    if let Some(source) = choices_source {
                        if let EvalValue::Array(arr) = source {
                            let mut choices = Vec::new();
                            for choice in arr.iter() {
                                let choice_data = match choice {
                                    EvalValue::Object(map) => map,
                                    _ => bail!("Un choix de DEMANDER doit être un objet"),
                                };
                                let value = match choice_data.get(&EvalKey::from("valeur")) {
                                    Some(v) => v.clone(),
                                    None => bail!("Un choix de DEMANDER doit inclure un champ `valeur`"),
                                };
                                let label = match choice_data.get(&EvalKey::from("texte")) {
                                    Some(EvalValue::String(s)) => split_fmt_str(&s)?,
                                    _ => bail!("Un choix de DEMANDER doit inclure un champ `texte`"),
                                };
                                choices.push(PollChoice {
                                    value,
                                    label,
                                });
                            }
                            if choices.is_empty() {
                                None
                            } else {
                                Some(Deferred::Literal(choices))
                            }
                        } else {
                            Some(Deferred::Formula(source))
                        }
                    } else {
                        None
                    }
                },
                default: match data.get(&EvalKey::from("defaut")) {
                    None | Some(EvalValue::Null) => None,
                    Some(value) => Some(value.clone()),
                },
                notification,
            },
            "rediriger" => {
                let destination = match data.get(&EvalKey::from("destination")) {
                    Some(EvalValue::String(s)) if s.is_empty() => bail!("REDIRIGER doit inclure un champ `destination` non vide"),
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("REDIRIGER doit inclure un champ `destination`"),
                };
                Self::Redirection {
                    destination: split_fmt_str(&destination)?,
                    notification,
                }
            },
            _ => bail!("Type d'effet de page non supporté: {kind}"),
        };
        Ok(effect)
    }

    /// Presents a poll request and returns the validated answer.
    ///
    ///
    /// # Arguments
    ///
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `env` — Additional formula bindings for the current lifecycle operation.
    ///
    /// # Errors
    ///
    /// Propagates formula-formatting, choice-resolution, or default-conversion errors.
    pub fn poll(
        &self,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Option<(Arc<str>, Arc<str>, PollRequest)>> {
        match self {
            Self::Poll {
                destination,
                prompt,
                data_type,
                choices,
                default,
                notification: _notification,
            } => {
                let prompt = formula.fmt_value(prompt, env)?;
                let destination = formula.fmt_value(destination, env)?;
                let choices = match choices {
                    None => None,
                    Some(Deferred::Literal(choices)) => {Some(choices.clone())},
                    Some(Deferred::Formula(EvalValue::Func(f))) => {
                        let resolved_choices = EvalValue::Func(f.clone()).call(&[], formula, env)?;
                        match resolved_choices {
                            EvalValue::Array(arr) if arr.is_empty() => None,
                            EvalValue::Array(arr) => Some(
                                arr.iter()
                                    .cloned()
                                    .map(|choice| {
                                        match choice {
                                            EvalValue::Object(data) => {
                                                let Some(value) = data.get(&EvalKey("valeur".into())).cloned() else {
                                                    bail!("Un choix de DEMANDER doit inclure un champ `valeur`")
                                                };
                                                let Some(EvalValue::String(label)) = data.get(&EvalKey("texte".into())).cloned() else {
                                                    bail!("Un choix de DEMANDER doit inclure un champ `texte`")
                                                };
                                                Ok(PollChoice {
                                                    value,
                                                    label: split_fmt_str(&label)?,
                                                })
                                            },
                                            other => bail!("Les choix de DEMANDER doivent être des objets, pas {other}"),
                                        }
                                    })
                                    .collect::<Result<Vec<_>>>()?
                                ),
                            other => bail!("Le champ `choix` de DEMANDER doit se résoudre à un tableau ou null, pas {other}"),
                        }
                    },
                    Some(Deferred::Formula(other)) => bail!("Le champ `choix` de DEMANDER doit être un tableau, une formule ou null, pas {other}"),
                };
                match data_type {
                    PollRequestDataType::Str => {
                        let (default, resolved_choices) =
                            create_poll_request(default.as_ref(), choices.as_ref(), formula, env)?;
                        Ok(Some((
                            destination,
                            prompt.clone(),
                            PollRequest {
                                prompt,
                                data: PollRequestData::Str {
                                    choices: resolved_choices,
                                    default,
                                },
                            },
                        )))
                    }
                    PollRequestDataType::Int => {
                        let (default, resolved_choices) =
                            create_poll_request(default.as_ref(), choices.as_ref(), formula, env)?;
                        Ok(Some((
                            destination,
                            prompt.clone(),
                            PollRequest {
                                prompt,
                                data: PollRequestData::Int {
                                    choices: resolved_choices,
                                    default,
                                },
                            },
                        )))
                    }
                    PollRequestDataType::Bool => {
                        let (default, _) =
                            create_poll_request(default.as_ref(), None, formula, env)?;
                        Ok(Some((
                            destination,
                            prompt.clone(),
                            PollRequest {
                                prompt,
                                data: PollRequestData::Bool { default },
                            },
                        )))
                    }
                    PollRequestDataType::Float => {
                        let (default, resolved_choices) =
                            create_poll_request(default.as_ref(), choices.as_ref(), formula, env)?;
                        Ok(Some((
                            destination,
                            prompt.clone(),
                            PollRequest {
                                prompt,
                                data: PollRequestData::Float {
                                    choices: resolved_choices,
                                    default,
                                },
                            },
                        )))
                    }
                }
            }
            _ => Ok(None),
        }
    }

    /// Applies the operation to the supplied runtime state.
    ///
    /// Effects are processed in deterministic order. The return value carries any visible
    /// messages, poll result, defeat state, or forced destination produced by the operation.
    ///
    /// # Arguments
    ///
    /// - `character` — Current mutable or read-only character state.
    /// - `tale` — Loaded tale definition used to resolve IDs and lifecycle behavior.
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `env` — Additional formula bindings for the current lifecycle operation.
    /// - `term` — Active player terminal used for prompts or rendered output.
    /// - `interaction` — Input adapter used to present polls.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal I/O fails, a prompt is cancelled, entered data is
    /// invalid, or a delegated parser/evaluator fails.
    pub fn apply(
        &self,
        character: &mut Character,
        tale: &Tale,
        formula: &mut FormulaEngine,
        oenv: &Env,
        term: &Terminal,
        interaction: &mut Interaction,
    ) -> Result<PageEffectResult> {
        let result = (|| {
            match self {
            Self::AddCondition { condition_id, data, notification } => {
                let condition_id = match formula.resolve_formula(condition_id, oenv)? {
                    EvalValue::String(s) => s,
                    other => bail!("AJOUTER_ETAT: l'identifiant de l'état doit se résoudre à une chaîne, pas {other}"),
                };
                let template = tale
                    .conditions
                    .get(&condition_id)
                    .ok_or_else(|| anyhow!("État '{condition_id}' introuvable"))?;
                let base_env = mk_character_base_env(character, formula, oenv)?.with_parent(oenv);
                let condition = template.instantiate(data, formula, &base_env)?;
                let actual_name = condition.actual_name(formula, &base_env)?;
                let death = condition.template.death.clone();
                character.conditions.push(condition);
                let idx = character.conditions.len() - 1;
                character.fix_hp(formula, oenv)?;
                character.fix_morale(formula, oenv)?;
                let charenv = mk_character_base_env(character, formula, oenv)?;
                let notice_env = condition_notification_env(&charenv, idx, &base_env)?;
                let notice_env = charenv.expand(notice_env);
                let notif = notification
                    .as_ref()
                    .map(|n| formula.fmt_value(n, &notice_env))
                    .transpose()?
                    .unwrap_or_else(|| format!(
                        "Tu as reçu l'état: {}.",
                        actual_name
                ).into());
                let notifs = if notif.is_empty() {
                    Vec::new()
                } else {
                    vec![notif]
                };
                Ok(PageEffectResult {
                    next_page: (!character.is_alive())
                        .then_some(death.map(|d| formula.fmt_value(&d, &notice_env)).transpose()?)
                        .flatten(),
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::RemoveCondition { filter, selection , notification} => {
                self.remove_conditions(character, filter, *selection, notification.as_ref(), formula, &oenv)
            }
            Self::ModifyCondition {
                filter,
                data,
                selection,
                notification,
            } => self.modify_conditions(character, filter, data.clone(), *selection, notification.as_ref(), formula, &oenv),
            Self::SetTag {
                filter,
                path,
                create_path,
                value,
                selection,
                notification,
            } => apply_set_tags(
                character,
                filter,
                path.as_ref(),
                *create_path,
                value,
                *selection,
                notification.as_ref(),
                formula,
                oenv,
            ),
            Self::IncrementTag {
                filter,
                path,
                create_path,
                amount,
                selection,
                notification,
            } => apply_increment_tags(
                character,
                filter,
                path.as_ref(),
                *create_path,
                amount,
                *selection,
                notification.as_ref(),
                formula,
                oenv,
            ),
            Self::AddItem {
                item_id,
                quantity,
                data,
                notification,
            } => {
                let item_id = match formula.resolve_formula(item_id, oenv)? {
                    EvalValue::String(s) => s,
                    other => bail!("AJOUTER_OBJET: l'identifiant de l'objet doit se résoudre à une chaîne, pas {other}"),
                };
                let item = tale
                    .items
                    .get(&item_id)
                    .cloned()
                    .ok_or_else(|| anyhow!("Objet '{item_id}' introuvable"))?;
                let charenv = mk_character_env(character, formula, oenv)?;
                let env = charenv.merged(&oenv);
                let quantity = resolve_formula_quantity(quantity, formula, &env)?;
                if quantity <= 0 {
                    return Ok(PageEffectResult::default());
                }
                // add the default data to the stack if it is absent
                let stack = item.instantiate(&data, quantity, formula, &env)?;
                let idx = character.add_item(stack.clone());
                let env = mk_partial_item_env(&stack)
                    .with_context(|| "Erreur en construisant l'environement d'objet dans effet 'ajouter objet'")?
                    .with_parent(&env);
                let name = stack.actual_name(formula, &env)?;
                let charenv = mk_character_env(character, formula, &env)?;
                let notice_env = mk_item_env(&charenv, idx)?
                    .expand(charenv)
                    .with_parent(&env);
                let quantity_text = if quantity > 1 {
                    format!(" x{quantity}")
                } else {
                    String::new()
                };
                let notif = notification
                    .as_ref()
                    .map(|n| formula.fmt_value(n, &notice_env))
                    .transpose()?
                    .unwrap_or_else(|| format!("Tu as reçu: {name}{quantity_text}").into());
                let notifs = if notif.is_empty() {
                    Vec::new()
                } else {
                    vec![notif]
                };
                Ok(PageEffectResult {
                    next_page: None,
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::RemoveItem {
                filter,
                quantity,
                selection,
                notification,
            } => self.remove_items(character, filter, quantity, *selection, notification.as_ref(), formula, &oenv),
            Self::ModifyItem {
                filter,
                data,
                selection,
                notification,
            } => self.modify_items(character, filter, data.clone(), *selection, notification.as_ref(), formula, &oenv),
            Self::ModifyHp { amount, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(amount, &base_env)?;
                let old_value = character.hp;
                if !character.modify_hp(amount, formula, oenv)? && death.is_some() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.hp));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(),
                    old_value.into(), 
                    character.hp.into(), 
                    &|| signed(amount, "PV"), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::ModifyAttribute { attribute, value, notification } => {
                character.ensure_attribute(attribute)?;
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.attributes.get(attribute).cloned().unwrap_or(0);
                *character.attributes.get_mut(attribute).expect("vérifié") += amount;
                let new_value = character.attributes.get(attribute).cloned().unwrap_or(0);
                let mut nenv = Env::new();
                nenv.insert("attribut", EvalValue::String(attribute.clone()));
                nenv.insert("ancienne", EvalValue::Int(old_value));
                nenv.insert("valeur", EvalValue::Int(new_value));
                let charenv = mk_character_env(character, formula, oenv)?;
                let nenv = nenv.with_parent(oenv).with_parent(&charenv);
                let notif = notification
                    .as_ref()
                    .map(|n| formula.fmt_value(n, &nenv))
                    .transpose()?
                    .unwrap_or_else(|| signed(amount, attribute));
                let notifs = if notif.is_empty() {
                    Vec::new()
                } else {
                    vec![notif]
                };
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::SetAttribute { attribute, value , notification} => {
                character.ensure_attribute(attribute)?;
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let ancienne = character.attributes.get(attribute).cloned().unwrap_or(0);
                character.attributes.insert(attribute.clone(), amount);
                let mut nenv = Env::new();
                nenv.insert("attribut", EvalValue::String(attribute.clone()));
                nenv.insert("ancienne", EvalValue::Int(ancienne));
                nenv.insert("valeur", EvalValue::Int(amount));
                let charenv = mk_character_env(character, formula, oenv)?;
                let nenv = nenv.with_parent(oenv).with_parent(&charenv);
                let notif = notification
                    .as_ref()
                    .map(|n| formula.fmt_value(n, &nenv))
                    .transpose()?
                    .unwrap_or_else(|| format!("{attribute} défini à {amount}").into());
                let notifs = if notif.is_empty() {
                    Vec::new()
                } else {
                    vec![notif]
                };
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::SetHp { value, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.hp;
                if !character.set_hp(amount, formula, oenv)? && death.is_some() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.hp));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(), 
                    old_value.into(), 
                    character.hp.into(), 
                    &|| format!("PV défini à {}", character.hp).into(), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::SetMaxHp { value, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.max_hp;
                character.set_max_hp(amount, formula, oenv)?;
                character.fix_hp(formula, oenv)?;
                if !character.is_alive() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.max_hp));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(),
                    old_value.into(), 
                    character.max_hp.into(), 
                    &|| format!("PV Max défini à {}", character.max_hp).into(), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::ModifyMaxHp { value, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.max_hp;
                character.modify_max_hp(amount, formula, oenv)?;
                character.fix_hp(formula, oenv)?;
                if !character.is_alive() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.max_hp));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(), 
                    old_value.into(), 
                    character.max_hp.into(), 
                    &|| signed(amount, "Max PV"), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::SetMorale { value, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.morale;
                if !character.set_morale(amount, formula, oenv)? && death.is_some() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.morale));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(), 
                    old_value.into(), 
                    character.morale.into(), 
                    &|| format!("Moral défini à {}", character.morale).into(), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::SetMaxMorale { value, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.max_morale;
                character.set_max_morale(amount, formula, oenv)?;
                character.fix_morale(formula, oenv)?;
                if !character.is_alive() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.max_morale));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(), 
                    old_value.into(), 
                    character.max_morale.into(), 
                    &|| format!("Moral Max défini à {}", character.max_morale).into(), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::ModifyMaxMorale { value, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.max_morale;
                character.modify_max_morale(amount, formula, oenv)?;
                character.fix_morale(formula, oenv)?;
                if !character.is_alive() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.max_morale));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(), 
                    old_value.into(), 
                    character.max_morale.into(), 
                    &|| signed(amount, "Moral Max"), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::ModifyMorale { value, death, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let amount = formula.resolve_formula_int(value, &base_env)?;
                let old_value = character.morale;
                if !character.modify_morale(amount, formula, oenv)? && death.is_some() {
                    return Ok(PageEffectResult {
                        next_page: {
                            let mut env = Env::new().with_parent(&base_env);
                            env.insert("ancienne", EvalValue::Int(old_value));
                            env.insert("valeur", EvalValue::Int(character.morale));
                            death.as_ref().map(|val| formula.fmt_value(val, &env)).transpose()?
                        },
                        ..PageEffectResult::default()
                    });
                }
                let notifs = simple_notification(
                    character, 
                    notification.clone(), 
                    old_value.into(), 
                    character.morale.into(), 
                    &|| signed(amount, "Moral"), 
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::ModifyName { value, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let new_name = formula.fmt_value(value, &base_env)?;
                let old_name = character.name.clone().unwrap_or_else(|| "Inconnu".into());
                character.name = Some(new_name.clone());
                let notifs = simple_notification(
                    character, 
                    notification.clone(),
                    old_name.into(), 
                    new_name.clone().into(), 
                    &|| format!("Ton nom est maintenant: {}", new_name).into(),
                    formula, 
                    &oenv
                )?;
                Ok(PageEffectResult {
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            }
            Self::RemoveTag { filter, selection , notification, path} => {
                apply_remove_tags(
                    character, 
                    filter, 
                    path.as_ref(), 
                    *selection, 
                    notification.as_ref(),
                    formula, 
                    oenv
                )
            }
            Self::Poll { notification, .. } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                if let Some((destination, prompt, request)) = self.poll(formula, &base_env)? {
                    let (res, text) = interaction.poll(term, request)?;
                    let msg = format!("{} > {}", prompt, text);
                    let tag = character
                        .find_tag_value(&destination, true, EvalValue::Null)?;
                    let old_value = tag.clone();
                    *tag = res.clone();
                    if let Some(notif) = notification {
                        let mut nenv = mk_character_env(character, formula, oenv)?.with_parent(&oenv);
                        nenv.insert("tag", EvalValue::String(destination.clone()));
                        nenv.insert("valeur", res.clone());
                        nenv.insert("ancienne", old_value);
                        let notif = formula.fmt_value(notif, &nenv)?;
                        if !notif.is_empty() {
                            return Ok(PageEffectResult {
                                poll_result: Some(notif),
                                ..PageEffectResult::default()
                            });
                        } else {
                            return Ok(PageEffectResult::default())
                        }
                    }
                    Ok(PageEffectResult {
                        poll_result: Some(msg.into()),
                        ..PageEffectResult::default()
                    })
                } else {
                    Ok(PageEffectResult::default())
                }
            },
            Self::Redirection { destination: page_id, notification } => {
                let charenv = mk_character_env(character, formula, oenv)?;
                let base_env = charenv.merged(&oenv);
                let page_id = formula.fmt_value(page_id, &base_env)?;
                let notif = notification
                    .as_ref()
                    .map(|n| formula.fmt_value(n, &base_env))
                    .transpose()?
                    .unwrap_or_else(|| "".into());
                let notifs = if notif.is_empty() {
                    Vec::new()
                } else {
                    vec![notif]
                };
                Ok(PageEffectResult {
                    next_page: Some(page_id),
                    shows: notifs.into(),
                    ..PageEffectResult::default()
                })
            },
        }
        })();
        result.with_context(|| format!("Échec de l'effet `{}`", self.kind()))
    }
    /// Removes matching persistent states according to `selection`.
    ///
    /// Removal consequences and notifications are resolved against each selected state's environment; resource maxima are reclamped afterward.
    ///
    /// # Errors
    ///
    /// Propagates filter, formula, consequence, or notification errors.
    fn remove_conditions(
        &self,
        character: &mut Character,
        filter: &ConditionFilter,
        selection: Selection,
        notification: Option<&EvalValue>,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<PageEffectResult> {
        let base_env = mk_character_base_env(character, formula, env)?.with_parent(env);
        let mut indices = Vec::new();
        for (index, condition) in character.conditions.iter().enumerate() {
            if filter.matches(condition, formula, &base_env)? {
                indices.push(index);
            }
        }
        let indices = selection.apply(indices);
        let mut death_page = None;
        //let base_env = env.merged(&character.env());
        let mut notifs = Vec::new();
        for (removed_count, original_index) in indices.into_iter().enumerate() {
            let index = original_index - removed_count;
            let condition = character.conditions.remove(index);
            let name = condition.actual_name(formula, &base_env)?;
            character.fix_hp(formula, &env)?;
            character.fix_morale(formula, &env)?;
            let charenv = mk_character_env(character, formula, &env)?;
            let nenv = condition_notification_env(&charenv, index, &base_env)?;
            let nenv = charenv
                .expand(nenv);
            let notif = notification
                .as_ref()
                .map(|n| formula.fmt_value(n, &nenv))
                .transpose()?
                .unwrap_or_else(|| format!("Tu as perdu l'état: {name}").into());
            if !notif.is_empty() {
                notifs.push(notif);
            }
            if !character.is_alive() && death_page.is_none() {
                death_page = condition.template.death
                    .clone()
                    .map(|d| formula.fmt_value(&d, &nenv))
                    .transpose()?;
            } else if character.is_alive() {
                death_page = None;
            }
        }
        
        Ok(PageEffectResult {
            shows: notifs.into(),
            next_page: (!character.is_alive()).then_some(death_page).flatten(),
            ..PageEffectResult::default()
        })
    }
    /// Applies resolved data updates to matching persistent states while preserving their template identity.
    ///
    /// # Errors
    ///
    /// Propagates filter, formula, or notification errors.
    fn modify_conditions(
        &self,
        character: &mut Character,
        filter: &ConditionFilter,
        updates: EvalValue,
        selection: Selection,
        notification: Option<&EvalValue>,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<PageEffectResult> {
        let updates = match formula.resolve_formula(&updates, env)? {
            EvalValue::Object(map) => map.clone(),
            other => bail!("Les données d'état doivent être un dictionnaire ou être évalué en un, reçu {other}"),
        };
        let base_env = mk_character_base_env(character, formula, env)?.with_parent(env);
        let mut indices = Vec::new();
        for (index, condition) in character.conditions.iter().enumerate() {
            if filter.matches(condition, formula, &base_env)? {
                indices.push(index);
            }
        }
        let indices = selection.apply(indices);
        let mut death_page = None;
        //let base_env = env.merged(&character.env());
        let mut notifs = Vec::new();
        for index in indices {
            let old_metadata = EvalValue::Object(character.conditions[index].data.clone());
            let env = mk_partial_condition_env(&character.conditions[index])
                .with_context(|| "Erreur en construisant l'environement d'objet dans l'effet 'modifier etat'")?
                .expand(mk_character_base_env(character, formula, env)?).with_parent(env);
            let new_data = resolve_map(updates.clone(), &env, formula)?;
            {
                let condition = &mut character.conditions[index];
                let work = Arc::make_mut(&mut condition.data);
                for (key, value) in new_data {
                    work.insert(key, value);
                }
            }
            character.fix_hp(formula, &env)?;
            character.fix_morale(formula, &env)?;
            let name = character.conditions[index].actual_name(formula, &base_env)?;
            let mut nenv = Env::new();
            nenv.insert("ancienne", old_metadata);
            let charenv = mk_character_env(character, formula, &env)?;
            let nenv = condition_notification_env(&charenv, index, &base_env)?
                .expand(charenv)
                .with_parent(&nenv);
            let notif = notification
                .as_ref()
                .map(|n| formula.fmt_value(n, &nenv))
                .transpose()?
                .unwrap_or_else(|| format!("Tu as modifié l'état: {name}").into());
            if !notif.is_empty() {
                notifs.push(notif);
            }
            if !character.is_alive() && death_page.is_none() {
                death_page = character.conditions[index].template.death
                    .clone()
                    .map(|d| formula.fmt_value(&d, &nenv))
                    .transpose()?;
            } else if character.is_alive() {
                death_page = None;
            }
        }
        
        Ok(PageEffectResult {
            shows: notifs.into(),
            next_page: (!character.is_alive()).then_some(death_page).flatten(),
            ..PageEffectResult::default()
        })
    }
    /// Subtracts the resolved quantity from matching inventory stacks and removes depleted stacks.
    ///
    /// # Errors
    ///
    /// Propagates filter, formula, or notification errors.
    fn remove_items(
        &self,
        character: &mut Character,
        filter: &ItemFilter,
        quantity: &EvalValue,
        selection: Selection,
        notification: Option<&EvalValue>,
        formula: &mut FormulaEngine,
        oenv: &Env,
    ) -> Result<PageEffectResult> {
        let base_env = mk_character_base_env(character, formula, oenv)?.with_parent(oenv);
        let mut indices = Vec::new();
        for (index, stack) in character.inventory.iter().enumerate() {
            let env = mk_partial_item_env(&stack)
                .with_context(|| "Erreur en construisant l'environement d'objet dans effet 'retirer objet'")?
                .with_parent(&base_env);
            if filter.matches(stack, formula, &env)? {
                indices.push(index);
            }
        }
        let indices = selection.apply(indices);
        //let base_env = env.merged(&character.env());
        let mut notifs = Vec::new();
        for index in indices {
            let (old_value, stack_qt, removed, name, suffix) = {
                let stack = &mut character.inventory[index];
                let env = mk_partial_item_env(&stack)
                    .with_context(|| "Erreur en construisant l'environement d'objet dans effet 'retirer objet'")?
                    .with_parent(&base_env);
                let quantity = resolve_formula_quantity(quantity, formula, &env)?;
                let old_value = stack.quantity;
                let removed = quantity.min(stack.quantity);
                let name = stack.actual_name(formula, &base_env)?;
                let suffix = if removed > 1 {
                    format!(" x{removed}")
                } else {
                    String::new()
                };
                let mut removed_stack = stack.clone();
                removed_stack.quantity = removed;
                stack.quantity -= removed;
                (old_value, stack.quantity, removed, name, suffix)
            };
            let charenv = mk_character_base_env(character, formula, oenv)?;
            let nenv = item_notification_env(&charenv, index, &base_env)?;
            let mut nenv = charenv.expand(nenv);
            nenv.insert("ancienne", EvalValue::Int(old_value));
            nenv.insert("valeur", EvalValue::Int(stack_qt));
            nenv.insert("retires", EvalValue::Int(removed));
            let notif = notification
                .as_ref()
                .map(|n| formula.fmt_value(n, &nenv))
                .transpose()?
                .unwrap_or_else(|| format!("Tu as perdu: {name}{suffix}").into());
            if !notif.is_empty() {
                notifs.push(notif);
            }

        }
        character.inventory.retain(|stack| stack.quantity > 0);
        character.normalize_inventory();
        Ok(PageEffectResult {
            shows: notifs.into(),
            ..PageEffectResult::default()
        })
    }
    /// Applies resolved metadata updates to matching item stacks using each stack's formula environment.
    ///
    /// # Errors
    ///
    /// Propagates filter, formula, or notification errors.
    fn modify_items(
        &self,
        character: &mut Character,
        filter: &ItemFilter,
        updates: EvalValue,
        selection: Selection,
        notification: Option<&EvalValue>,
        formula: &mut FormulaEngine,
        oenv: &Env,
    ) -> Result<PageEffectResult> {
        let updates = match formula.resolve_formula(&updates, oenv)? {
            EvalValue::Object(map) => map.clone(),
            other => bail!("Les données d'état doivent être un dictionnaire ou être évalué en un, reçu {other}"),
        };
        let base_env = mk_character_base_env(character, formula, oenv)?.with_parent(oenv);
        let mut indices = Vec::new();
        for (index, stack) in character.inventory.iter().enumerate() {
            let env = mk_partial_item_env(&stack)
                .with_context(|| "Erreur en construisant l'environement d'objet dans effet 'modifier objet'")?
                .with_parent(&base_env);
            if filter.matches(stack, formula, &env)? {
                indices.push(index);
            }
        }
        let indices = selection.apply(indices);
        //let base_env = env.merged(&character.env());
        let mut notifs = Vec::new();
        for index in indices {
            let old_metadata = EvalValue::Object(character.inventory[index].data.clone());
            let env = mk_character_base_env(character, formula, oenv)?.with_parent(oenv);
            let ienv = mk_partial_item_env(&character.inventory[index])
                .with_context(|| "Erreur en évaluant l'environement d'objet pour l'effet 'modifier objet'")?;
            let env = env.expand(ienv);
            let new_data = resolve_map(updates.clone(), &env, formula)?;
            for (key, value) in new_data {
                let work = Arc::make_mut(&mut character.inventory[index].data);
                work.insert(key, value);
            }
            let name = character.inventory[index].actual_name(formula, &base_env)?;
            let mut nenv = Env::new();
            nenv.insert("ancienne", old_metadata);
            let charenv = mk_character_base_env(character, formula, oenv)?;
            let inenv = item_notification_env(&charenv, index, &base_env)?;
            let nenv = charenv.expand(inenv)
                .with_parent(&nenv);
            let notif = notification
                .as_ref()
                .map(|n| formula.fmt_value(n, &nenv))
                .transpose()?
                .unwrap_or_else(|| format!("Tu as modifié l'objet: {name}").into());
            if !notif.is_empty() {
                notifs.push(notif);
            }
        }
        character.normalize_inventory();
        Ok(PageEffectResult {
            shows: notifs.into(),
            ..PageEffectResult::default()
        })
    }
}

/// Resolves every formula-bearing value in a map.
///
/// Evaluates deferred formulas exactly once against the supplied environment and returns
/// the resolved value without recursively evaluating strings produced by that formula.
fn resolve_map(
    values: Arc<IndexMap<EvalKey, EvalValue>>,
    env: &Env,
    formula: &mut FormulaEngine,
) -> Result<IndexMap<EvalKey, EvalValue>> {
    values
        .iter()
        .map(|(key, value)| Ok((key.clone(), formula.resolve_formula(value, env)?)))
        .collect()
}

/// One resolved component of a nested story-tag path.
#[derive(Clone, Debug)]
pub enum TagPathSegment {
    Key(Arc<str>),
    Index(usize),
}

/*
/// Reads an optional boolean effect field and defaults it to false.
fn optional_bool(data: &serde_json::Map<String, JsonValue>, key: &str) -> Result<bool> {
    match data.get(key) {
        None => Ok(false),
        Some(JsonValue::Bool(value)) => Ok(*value),
        Some(_) => bail!("Le champ `{key}` doit être un booléen"),
    }
}
*/

/// Resolves an explicit path list or a formula returning one.
///
/// Evaluates deferred formulas exactly once against the supplied environment and returns
/// the resolved value without recursively evaluating strings produced by that formula.
fn resolve_tag_path(
    path: &EvalValue,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<Vec<TagPathSegment>> {
    let path = formula.fmt_value(path, env)?;
    let values = path.trim().split(".").map(|s| {
        if let Ok(val) = s.parse::<usize>() {
            EvalValue::Int(val as i64)
        } else {
            EvalValue::String(s.to_string().into())
        }
    }).collect::<Vec<_>>();
    if values.is_empty() {
        bail!("Le champ `chemin` ne peut pas être vide");
    }
    values
        .into_iter()
        .map(|value| match value {
            EvalValue::String(key) => Ok(TagPathSegment::Key(key)),
            EvalValue::Int(index) if index >= 0 => Ok(TagPathSegment::Index(index as usize)),
            EvalValue::Int(index) => bail!("Un index de chemin ne peut pas être négatif: {index}"),
            other => bail!(
                "Chaque entrée de `chemin` doit être une chaîne ou un entier positif ou nul; reçu {other}"
            ),
        })
        .collect()
}

/// Converts a resolved path back to the list exposed to formulas and notifications.
fn tag_path_value(path: &[TagPathSegment]) -> EvalValue {
    EvalValue::Array(
        path.iter()
            .map(|segment| match segment {
                TagPathSegment::Key(key) => EvalValue::String(key.clone()),
                TagPathSegment::Index(index) => EvalValue::Int(*index as i64),
            })
            .collect(),
    )
}

/// Reads a nested value. Missing final object keys are allowed; missing
/// intermediates are allowed only when path creation was requested.
fn nested_tag_value(
    current: &EvalValue,
    path: &[TagPathSegment],
    create_path: bool,
) -> Result<Option<EvalValue>> {
    let Some((segment, remaining)) = path.split_first() else {
        return Ok(Some(current.clone()));
    };
    match (current, segment) {
        (EvalValue::Object(values), TagPathSegment::Key(key)) => match values.get(&EvalKey(key.clone().into())) {
            Some(value) => nested_tag_value(value, remaining, create_path),
            None if remaining.is_empty() || create_path => Ok(None),
            None => bail!("La clé intermédiaire `{key}` du chemin n'existe pas"),
        },
        (EvalValue::Array(values), TagPathSegment::Index(index)) => match values.get(*index) {
            Some(value) => nested_tag_value(value, remaining, create_path),
            None if create_path => Ok(None),
            None => bail!(
                "L'index {index} dépasse la taille {} de la liste ciblée",
                values.len()
            ),
        },
        (EvalValue::Null, _) if create_path => Ok(None),
        (value, TagPathSegment::Key(key)) => bail!(
            "Impossible de sélectionner la clé `{key}` dans une valeur {}",
            value
        ),
        (value, TagPathSegment::Index(index)) => bail!(
            "Impossible de sélectionner l'index {index} dans une valeur {}",
            value
        ),
    }
}

/// Creates the container needed by the next path component.
fn container_for(segment: &TagPathSegment) -> EvalValue {
    match segment {
        TagPathSegment::Key(_) => EvalValue::Object(IndexMap::new().into()),
        TagPathSegment::Index(_) => EvalValue::Array(Vec::new().into()),
    }
}

/// Writes one nested value while enforcing the path-creation policy.
fn write_nested_tag_value(
    current: &mut EvalValue,
    path: &[TagPathSegment],
    value: EvalValue,
    create_path: bool,
) -> Result<()> {
    let Some((segment, remaining)) = path.split_first() else {
        *current = value;
        return Ok(());
    };
    if matches!(current, EvalValue::Null) && create_path {
        *current = container_for(segment);
    }
    match (current, segment) {
        (EvalValue::Object(values), TagPathSegment::Key(key)) => {
            let work = Arc::make_mut(values);
            if remaining.is_empty() {
                work.insert(EvalKey(key.clone().into()), value);
                return Ok(());
            }
            let eval_key = EvalKey(key.clone().into());
            if !work.contains_key(&eval_key) {
                if !create_path {
                    bail!("La clé intermédiaire `{key}` du chemin n'existe pas");
                }
                work.insert(eval_key.clone(), container_for(&remaining[0]));
            }
            write_nested_tag_value(
                work.get_mut(&eval_key).expect("clé insérée"),
                remaining,
                value,
                create_path,
            )
        }
        (EvalValue::Array(values), TagPathSegment::Index(index)) => {
            if *index >= values.len() {
                if !create_path {
                    bail!(
                        "L'index {index} dépasse la taille {} de la liste ciblée",
                        values.len()
                    );
                }
                let mut new_vec = values.iter().cloned().collect::<Vec<_>>();
                new_vec.resize(*index + 1, EvalValue::Null);
                *values = new_vec.into();
            }
            let work = Arc::make_mut(values);
            if remaining.is_empty() {
                work[*index] = value;
                return Ok(());
            }
            if matches!(work[*index], EvalValue::Null) && create_path {
                work[*index] = container_for(&remaining[0]);
            }
            write_nested_tag_value(&mut work[*index], remaining, value, create_path)
        }
        (value, TagPathSegment::Key(key)) => bail!(
            "Impossible de sélectionner la clé `{key}` dans une valeur {}",
            value
        ),
        (value, TagPathSegment::Index(index)) => bail!(
            "Impossible de sélectionner l'index {index} dans une valeur {}",
            value
        ),
    }
}

/// Collects the top-level story tags selected by an effect filter.
fn selected_tag_ids(
    character: &Character,
    filter: &TagFilter,
    selection: Selection,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<Vec<Arc<str>>> {
    let cenv = mk_character_env(character, formula, env)?;
    let base_env = cenv.merged(env);
    if let TagFilter::Id(tag) = filter {
        return Ok(vec![formula.fmt_value(tag, &base_env)?.into()]);
    }
    let mut candidates = Vec::new();
    for (tag, value) in character.story_tags.iter() {
        let EvalKey(EvalValue::String(tag)) = tag else {
            bail!("Tag key is not a string: {tag:?}");
        };
        if filter.matches(tag, value, character, formula, &base_env)? {
            candidates.push(tag.clone());
        }
    }
    Ok(selection.apply(candidates))
}

/// Applies `definir_tag`, including optional nested-path mutation.
///
/// Effects are processed in deterministic order. The return value carries any visible
/// messages, poll result, defeat state, or forced destination produced by the operation.
fn apply_set_tags(
    character: &mut Character,
    filter: &TagFilter,
    path: Option<&EvalValue>,
    create_path: bool,
    value: &EvalValue,
    selection: Selection,
    notification: Option<&EvalValue>,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<PageEffectResult> {
    if let (TagFilter::Id(id), None) = (filter, &path) {
        let mut base_env = mk_character_base_env(character, formula, env)?.with_parent(env);
        let resolved_path = formula.fmt_value(id, &base_env)?;
        let destination = character.find_tag_value(&resolved_path, create_path, EvalValue::Null)?;
        let old_value = destination.clone();
        base_env.insert("tag", EvalValue::String(resolved_path.clone()));
        base_env.insert("valeur", destination.clone());
        let resolved_value = formula.resolve_formula(value, &base_env)?;
        *destination = resolved_value.clone();
        let notif_env = mk_character_base_env(character, formula, env)?
            .expand(tag_notification_env(&resolved_path, resolved_value, old_value))
            .with_parent(env);
        let notice = notification
            .map(|text| formula.fmt_value(text, &notif_env))
            .transpose()?
            .unwrap_or_default();
        return Ok(PageEffectResult {
            shows: if notice.is_empty() {
                Vec::new()
            } else {
                vec![notice]
            }.into(),
            ..PageEffectResult::default()
        });
    }
    let tags = selected_tag_ids(character, filter, selection, formula, env)?;
    let mut shows = Vec::new();
    for tag in tags {
        let tag_exists = character.story_tags.contains_key(&EvalKey(tag.clone().into()));
        let root_old = character
            .story_tags
            .get(&EvalKey(tag.clone().into()))
            .cloned()
            .unwrap_or(EvalValue::Null);
        let charenv = mk_character_env(character, formula, env)?;
        let mut effect_env = charenv.merged(env);
        effect_env.insert("tag", EvalValue::String(tag.clone()));
        effect_env.insert("valeur", root_old.clone());

        let (old_value, resolved, resolved_path, root_new) = if let Some(path) = &path {
            if !tag_exists && !create_path {
                bail!(
                    "Le tag `{tag}` n'existe pas; `creer_chemin: true` est requis pour créer sa structure"
                );
            }
            let resolved_path = resolve_tag_path(path, formula, &effect_env)?;
            let path_value = tag_path_value(&resolved_path);
            let old_value = nested_tag_value(&root_old, &resolved_path, create_path)?
                .unwrap_or(EvalValue::Null);
            effect_env.insert("chemin", path_value.clone());
            effect_env.insert("valeur", old_value.clone());
            let resolved = formula.resolve_formula(value, &effect_env)?;
            let mut root_new = root_old;
            write_nested_tag_value(
                &mut root_new,
                &resolved_path,
                resolved.clone(),
                create_path,
            )?;
            (old_value, resolved, Some(path_value), root_new)
        } else {
            let resolved = formula.resolve_formula(value, &effect_env)?;
            (root_old.clone(), resolved.clone(), None, resolved)
        };
        let work = Arc::make_mut(&mut character.story_tags);
        work.insert(EvalKey(tag.clone().into()), root_new);
        let mut notif_env = mk_character_base_env(character, formula, env)?
            .expand(tag_notification_env(&tag, resolved, old_value))
            .with_parent(env);
        if let Some(path) = resolved_path {
            notif_env.insert("chemin", path);
        }
        let notice = notification.clone()
            .map(|text| formula.fmt_value(text, &notif_env))
            .transpose()?
            .unwrap_or_default();
        if !notice.is_empty() {
            shows.push(notice);
        }
    }
    Ok(PageEffectResult {
        shows: shows.into(),
        ..PageEffectResult::default()
    })
}

/// Applies `incrementer_tag`, including optional nested-path mutation.
///
/// Effects are processed in deterministic order. The return value carries any visible
/// messages, poll result, defeat state, or forced destination produced by the operation.
fn apply_increment_tags(
    character: &mut Character,
    filter: &TagFilter,
    path: Option<&EvalValue>,
    create_path: bool,
    amount: &EvalValue,
    selection: Selection,
    notification: Option<&EvalValue>,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<PageEffectResult> {
    if let (TagFilter::Id(id), None) = (filter, &path) {
        let mut base_env = mk_character_base_env(character, formula, env)?.with_parent(env);
        let resolved_path = formula.fmt_value(id, &base_env)?;
        let destination = character.find_tag_value(&resolved_path, create_path, EvalValue::Int(0))?;
        let old_value = destination.clone();
        base_env.insert("tag", EvalValue::String(resolved_path.clone()));
        base_env.insert("valeur", destination.clone());
        let resolved_value = formula.resolve_formula(amount, &base_env)?;
        let result = destination.try_add(&resolved_value)?;
        *destination = result.clone();
        let mut notif_env = mk_character_base_env(character, formula, env)?
            .expand(tag_notification_env(&resolved_path, result, old_value))
            .with_parent(env);
        notif_env.insert("valeur", resolved_value);
        let notice = notification
            .map(|text| formula.fmt_value(text, &notif_env))
            .transpose()?
            .unwrap_or_default();
        return Ok(PageEffectResult {
            shows: if notice.is_empty() {
                Vec::new()
            } else {
                vec![notice]
            }.into(),
            ..PageEffectResult::default()
        });
    }
    let tags = selected_tag_ids(character, filter, selection, formula, env)?;
    let mut shows = Vec::new();
    for tag in tags {
        let tag_exists = character.story_tags.contains_key(&EvalKey(tag.clone().into()));
        let root_old = character
            .story_tags
            .get(&EvalKey(tag.clone().into()))
            .cloned()
            .unwrap_or_else(|| {
                if path.is_some() {
                    EvalValue::Null
                } else {
                    EvalValue::Int(0)
                }
            });
        let charenv = mk_character_env(character, formula, env)?;
        let mut effect_env = charenv.merged(env);
        effect_env.insert("tag", EvalValue::String(tag.clone()));
        effect_env.insert("valeur", root_old.clone());

        let (old_value, resolved, increment, resolved_path, root_new) = if let Some(path) = &path {
            if !tag_exists && !create_path {
                bail!(
                    "Le tag `{tag}` n'existe pas; `creer_chemin: true` est requis pour créer sa structure"
                );
            }
            let resolved_path = resolve_tag_path(path, formula, &effect_env)?;
            let path_value = tag_path_value(&resolved_path);
            let old_value = nested_tag_value(&root_old, &resolved_path, create_path)?
                .unwrap_or(EvalValue::Int(0));
            effect_env.insert("chemin", path_value.clone());
            effect_env.insert("valeur", old_value.clone());
            let increment = formula.resolve_formula(amount, &effect_env)?;
            let resolved = old_value.clone().try_add(&increment)?;
            let mut root_new = root_old;
            write_nested_tag_value(
                &mut root_new,
                &resolved_path,
                resolved.clone(),
                create_path,
            )?;
            (
                old_value,
                resolved,
                increment,
                Some(path_value),
                root_new,
            )
        } else {
            let increment = formula.resolve_formula(amount, &effect_env)?;
            let resolved = root_old.clone().try_add(&increment)?;
            (
                root_old.clone(),
                resolved.clone(),
                increment,
                None,
                resolved,
            )
        };
        let work = Arc::make_mut(&mut character.story_tags);
        work.insert(tag.clone().into(), root_new);
        let mut notif_env = mk_character_base_env(character, formula, env)?
            .expand(tag_notification_env(&tag, resolved, old_value))
            .with_parent(env);
        notif_env.insert("montant", increment);
        if let Some(path) = resolved_path {
            notif_env.insert("chemin", path);
        }
        let notice = notification.clone()
            .map(|text| formula.fmt_value(text, &notif_env))
            .transpose()?
            .unwrap_or_default();
        if !notice.is_empty() {
            shows.push(notice);
        }
    }
    Ok(PageEffectResult {
        shows: shows.into(),
        ..PageEffectResult::default()
    })
}

fn apply_remove_tags(
    character: &mut Character,
    filter: &TagFilter,
    path: Option<&EvalValue>,
    selection: Selection,
    notification: Option<&EvalValue>,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<PageEffectResult> {
    if let (TagFilter::Id(id), None) = (filter, &path) {
        let base_env = mk_character_base_env(character, formula, env)?.with_parent(env);
        let resolved_path = formula.fmt_value(id, &base_env)?;
        let Some(old_value) = character.remove_tag_value(
            &resolved_path
                .split('.')
                .map(|s| match s.parse::<usize>() {
                    Ok(index) => TagPathSegment::Index(index),
                    Err(_) => TagPathSegment::Key(s.into()),
                })
                .collect::<Vec<_>>(),
        )? else {
            return Ok(PageEffectResult::default());
        };
        let mut notif_env = Env::new();
        notif_env.insert("tag", EvalValue::String(resolved_path.clone()));
        notif_env.insert("valeur", old_value);
        let notif_env = mk_character_base_env(character, formula, env)?
            .expand(notif_env)
            .with_parent(env);
        let notice = notification
            .map(|text| formula.fmt_value(text, &notif_env))
            .transpose()?
            .unwrap_or_default();
        return Ok(PageEffectResult {
            shows: if notice.is_empty() {
                Vec::new()
            } else {
                vec![notice]
            }.into(),
            ..PageEffectResult::default()
        });
    }
    let tags = selected_tag_ids(character, filter, selection, formula, env)?;
    let mut shows = Vec::new();
    for tag in tags {
        let charenv = mk_character_env(character, formula, env)?;
        let base_env = charenv.merged(env);
        let mut tenv = Env::new();
        tenv.insert("tag", EvalValue::String(tag.clone()));
        tenv = base_env.expand(tenv);
        let resolved_path = if let Some(path) = &path {
            let resolved_path = resolve_tag_path(path, formula, &tenv)?;
            vec![vec![TagPathSegment::Key(tag)], resolved_path].concat()
        } else {vec![TagPathSegment::Key(tag)]};
        let Some(old_value) = character.remove_tag_value(&resolved_path)? else {
            continue;
        };
        tenv.insert("tag", EvalValue::String(resolved_path.into_iter().map(|p| match p {
            TagPathSegment::Key(key) => key,
            TagPathSegment::Index(index) => index.to_string().into(),
        }).collect::<Vec<_>>().join(".").into()));
        tenv.insert("valeur", old_value);
        let notice = notification.clone()
            .map(|text| formula.fmt_value(text, &tenv))
            .transpose()?
            .unwrap_or_default();
        if !notice.is_empty() {
            shows.push(notice);
        }
    }
    Ok(PageEffectResult {
        shows: shows.into(),
        ..PageEffectResult::default()
    })
}

/// Builds notification variables for one story-tag mutation.
///
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
fn tag_notification_env(tag: &str, value: EvalValue, old_value: EvalValue) -> Env<'static> {
    let mut env = Env::new();
    env.insert("tag", EvalValue::String(tag.to_owned().into()));
    env.insert("valeur", value);
    env.insert("ancienne", old_value.clone());
    env.insert("ancienne_valeur", old_value);
    env
}

/// Builds the resolved `objet` object exposed to notification templates.
///
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
fn item_notification_env<'a>(
    character_env: &Env<'_>,
    item_idx: usize,
    base_env: &'a Env<'a>,
) -> Result<Env<'a>> {
    let env = mk_item_env(character_env, item_idx)?;
    Ok(env.with_parent(base_env))
}

/// Builds the resolved `etat` object exposed to notification templates.
///
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
fn condition_notification_env<'a>(
    charenv: &Env<'_>,
    condition_idx: usize,
    base_env: &'a Env<'a>,
) -> Result<Env<'a>> {
    let env = mk_condition_env(charenv, condition_idx)?;
    Ok(env.with_parent(base_env))
}

/*
/// Builds either a defeat redirect or a resource-change notification.
fn death_result_or_message(
    character: &Character,
    death: &Option<String>,
    message: String,
) -> Result<PageEffectResult> {
    if !character.is_alive() && death.is_some() {
        Ok(PageEffectResult {
            next_page: death.clone(),
            ..PageEffectResult::default()
        })
    } else {
        Ok(PageEffectResult {
            shows: vec![message],
            ..PageEffectResult::default()
        })
    }
}
*/

/// Builds a notification message for a simple resource change.
fn simple_notification(
    character: &Character,
    notification: Option<EvalValue>,
    ancienne: EvalValue,
    valeur: EvalValue,
    default: &dyn Fn() -> Arc<str>,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<Vec<Arc<str>>> {
    let mut nenv = Env::new();
    nenv.insert("ancienne", ancienne);
    nenv.insert("valeur", valeur);
    let charenv = mk_character_env(character, formula, env)?;
    let nenv = nenv.with_parent(env).with_parent(&charenv);
    let notif = notification
        .as_ref()
        .map(|n| Ok::<Arc<str>, anyhow::Error>(formula.resolve_formula(&n, &nenv)?.repr()))
        .transpose()?
        .unwrap_or_else(default);
    Ok(if notif.is_empty() {
        Vec::new()
    } else {
        vec![notif]
    })
}

/// Formats a numeric value with an explicit sign when appropriate.
fn signed(value: i64, label: &str) -> Arc<str> {
    if value >= 0 {
        format!("+{value} {label}").into()
    } else {
        format!("{value} {label}").into()
    }
}
/*
/// Converts a JSON number into a finite floating-point value.
fn json_to_f64(value: &JsonValue) -> Option<f64> {
    match value {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Converts a floating-point result into a canonical JSON number.
fn number_json(value: f64) -> Result<JsonValue> {
    Ok(JsonValue::Number(
        JsonNumber::from_f64(value)
            .ok_or_else(|| anyhow!("Résultat numérique non sérialisable: {value}"))?,
    ))
}
*/

/// Resolves a poll effect into a concrete request ready for user interaction.
fn create_poll_request<T>(
    default: Option<&EvalValue>,
    choices: Option<&Vec<PollChoice>>,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<(Option<T>, Option<Vec<PollRequestOption<T>>>)>
where
    T: TryFrom<EvalValue> + Clone + std::fmt::Debug,
    <T as TryFrom<EvalValue>>::Error: std::fmt::Display,
{
    let default = default
        .as_ref()
        .map(|v| formula.resolve_formula(v, env))
        .transpose()?;
    let default_diagnostic = default.as_ref().map(EvalValue::diagnostic);
    let default = default
        .clone()
        .map(|v| v.try_into())
        .transpose()
        .map_err(|error| {
            anyhow!(
                "La valeur par défaut du sondage est incompatible: {}. Raison: {error}",
                default_diagnostic.as_deref().unwrap_or("aucune valeur")
            )
        })?;
    let resolved_choices = if let Some(choices) = choices {
        let mut resolved = Vec::new();
        for (index, choice) in choices.into_iter().enumerate() {
            let value = formula.resolve_formula(&choice.value, env)?;
            let mut env = env.clone();
            env.insert("valeur", value.clone());
            let value_diagnostic = value.diagnostic();
            let value = value
                .try_into()
                .map_err(|error| {
                    anyhow!(
                        "Le choix #{} du sondage est incompatible: {value_diagnostic}. Raison: {error}",
                        index + 1
                    )
                })?;
            let label = formula
                .fmt_value(&choice.label, &env)
                .with_context(|| format!("Rendu du libellé du choix #{} du sondage", index + 1))?;
            resolved.push(PollRequestOption::<T> { value, label });
        }
        Some(resolved)
    } else {
        None
    };
    Ok((default, resolved_choices))
}

/// Carries an optional notification, redirect, or poll produced by an effect.
#[derive(Clone, Debug, Default)]
pub struct PageEffectResult {
    pub next_page: Option<Arc<str>>,
    /// Additional notifications emitted separately for multi-target effects.
    pub shows: Arc<[Arc<str>]>,
    pub poll_result: Option<Arc<str>>,
}
