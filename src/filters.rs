//! Conditional expressions and filters for checks, items, states, and story tags.
//!
//! Filters combine static metadata, nested tag paths, and deferred formulas. Matching
//! functions propagate formula errors rather than silently treating malformed predicates
//! as false.
use std::sync::Arc;

use crate::character::Character;
use crate::effects::Deferred;
use crate::formula::{
    Env, EvalValue, FormulaConditionView, FormulaEngine, FormulaItemView, split_fmt_str, value_or_formula
};
use crate::json_util::{object, required_string, string_list, value_or};
use crate::model::{ConditionInstance, ItemStack, collect_traits, eval_traits};
use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;

/// Represents a condition that controls visibility or normalized-cell execution.
#[derive(Clone, Debug)]
pub enum Conditional {
    And(Arc<[Conditional]>),
    Or(Arc<[Conditional]>),
    Not(Arc<Conditional>),
    RequireItem(ItemFilter),
    RequireTag(TagFilter),
    AttributeEq {
        attribute: Arc<str>,
        value: EvalValue,
    },
    AttributeGreater {
        attribute: Arc<str>,
        value: EvalValue,
    },
    AttributeLess {
        attribute: Arc<str>,
        value: EvalValue,
    },
    Formula(EvalValue),
    HpEq(EvalValue),
    HpGreater(EvalValue),
    HpLess(EvalValue),
    MoraleEq(EvalValue),
    MoraleGreater(EvalValue),
    MoraleLess(EvalValue),
    RequireCondition(ConditionFilter),
    Random(EvalValue),
    /// Matches one or more immutable tags on the current page.
    PageTag(TagFilter),
    /// Matches one or more immutable tags on the current connection, which may be a page or a cell.
    ConnectionTag(TagFilter),
}

impl Conditional {
    /// Returns the tale-syntax family used to identify this predicate in errors.
    fn kind(&self) -> &'static str {
        match self {
            Self::And(_) => "et",
            Self::Or(_) => "ou",
            Self::Not(_) => "non",
            Self::RequireItem(_) => "objet_requis",
            Self::RequireTag(_) => "tag_requis",
            Self::AttributeEq { .. } => "attribut_egal",
            Self::AttributeGreater { .. } => "attribut_superieur",
            Self::AttributeLess { .. } => "attribut_inferieur",
            Self::Formula(_) => "formule",
            Self::HpEq(_) => "pv_egaux",
            Self::HpGreater(_) => "pv_superieurs",
            Self::HpLess(_) => "pv_inferieurs",
            Self::MoraleEq(_) => "moral_egal",
            Self::MoraleGreater(_) => "moral_superieur",
            Self::MoraleLess(_) => "moral_inferieur",
            Self::RequireCondition(_) => "etat_requis",
            Self::Random(_) => "aleatoire",
            Self::PageTag(_) => "tag_page",
            Self::ConnectionTag(_) => "tag_connexion",
        }
    }

    /// Parses a conditional tree controlling visibility or cell execution.
    ///
    /// Array shorthand is interpreted as logical AND; formula values remain deferred.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown condition types or malformed required fields and nested filters.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        if let JsonValue::Array(values) = value {
            return Ok(Self::And(
                values
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<Vec<_>>>()?
                    .into(),
            ));
        }
        let data = object(value, "Une condition de connexion")?;
        let kind = required_string(data, "type", "Une condition")?;
        Ok(match kind.as_str() {
            "et" => Self::And(parse_conditionals(data.get("conditions"))?.into()),
            "ou" => Self::Or(parse_conditionals(data.get("conditions"))?.into()),
            "non" => Self::Not(Arc::new(Self::from_json(
                data.get("condition")
                    .ok_or_else(|| anyhow!("Une condition NON doit inclure une sous-condition"))?,
            )?)),
            "objet_requis" => Self::RequireItem(ItemFilter::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("OBJET_REQUIS doit inclure un filtre"))?,
            )?),
            "tag_requis" => Self::RequireTag(TagFilter::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("TAG_REQUIS doit inclure un filtre"))?,
            )?),
            "attribut_egal" => Self::AttributeEq {
                attribute: required_string(data, "attribut", "ATTRIBUT_EGAL")?.into(),
                value: value_or_formula(data
                    .get("valeur")
                    .cloned()
                    .ok_or_else(|| anyhow!("ATTRIBUT_EGAL doit inclure un champ `valeur`"))?
                    .try_into()?)?,
            },
            "attribut_superieur" => Self::AttributeGreater {
                attribute: required_string(data, "attribut", "ATTRIBUT_SUPERIEUR")?.into(),
                value: value_or_formula(data
                    .get("valeur")
                    .cloned()
                    .ok_or_else(|| anyhow!("ATTRIBUT_SUPERIEUR doit inclure un champ `valeur`"))?
                    .try_into()?)?,
            },
            "attribut_inferieur" => Self::AttributeLess {
                attribute: required_string(data, "attribut", "ATTRIBUT_INFERIEUR")?.into(),
                value: value_or_formula(data
                    .get("valeur")
                    .cloned()
                    .ok_or_else(|| anyhow!("ATTRIBUT_INFERIEUR doit inclure un champ `valeur`"))?
                    .try_into()?)?,
            },
            "formule" => Self::Formula(
                value_or_formula(data
                    .get("formule")
                    .cloned()
                    .ok_or_else(|| anyhow!("FORMULE doit inclure une formule"))?
                    .try_into()?)?,
            ),
            "pv_egaux" => Self::HpEq(value_or_formula(required_value(data, "valeur", "PV_EGAUX")?.try_into()?)?),
            "pv_superieurs" => {
                Self::HpGreater(value_or_formula(required_value(data, "valeur", "PV_SUPERIEURS")?.try_into()?)?)
            }
            "pv_inferieurs" => {
                Self::HpLess(value_or_formula(required_value(data, "valeur", "PV_INFERIEURS")?.try_into()?)?)
            }
            "moral_egal" => {
                Self::MoraleEq(value_or_formula(required_value(data, "valeur", "MORAL_EGAL")?.try_into()?)?)
            }
            "moral_superieur" => {
                Self::MoraleGreater(value_or_formula(required_value(data, "valeur", "MORAL_SUPERIEUR")?.try_into()?)?)
            }
            "moral_inferieur" => {
                Self::MoraleLess(value_or_formula(required_value(data, "valeur", "MORAL_INFERIEUR")?.try_into()?)?)
            }
            "etat_requis" => Self::RequireCondition(ConditionFilter::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("ETAT_REQUIS doit inclure un filtre"))?,
            )?),
            "aleatoire" => Self::Random(value_or_formula(required_value(data, "poids", "ALEATOIRE")?.try_into()?)?),
            "tag_page" => Self::PageTag(TagFilter::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("TAG_PAGE doit inclure un filtre"))?,
            )?),
            "tag_connexion" => Self::ConnectionTag(TagFilter::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("TAG_CONNEXION doit inclure un filtre"))?,
            )?),
            _ => bail!("Type de condition non supporté: {kind}"),
        })
    }
    /// Builds a conditional tree from a formula-produced dictionary/list.
    ///
    /// # Errors
    ///
    /// Returns an error when generated fields have incompatible runtime types or unknown variants.
    pub fn from_dict(data: &EvalValue) -> Result<Self> {
        let data = match data {
            EvalValue::Array(arr) => {
                return Ok(Self::And(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?));
            },
            EvalValue::Object(obj) => obj,
            _ => bail!("Une condition doit être un tableau ou un objet"),
        };
        let kind = if let EvalValue::String(s) = data
            .get("type")
            .ok_or_else(|| anyhow!("Une condition doit inclure un champ `type`"))? {
            s.clone()
        } else {
            bail!("Le champ `type` d'une condition doit être une chaîne de caractères")
        };
        Ok(match kind.as_ref() {
            "et" => {
                match data.get("conditions") {
                    Some(EvalValue::Array(arr)) => {
                        Self::And(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::And(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::And(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `conditions` d'une condition ET doit être un tableau"),
                }
            },
            "ou" => {
                match data.get("conditions") {
                    Some(EvalValue::Array(arr)) => {
                        Self::Or(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::Or(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::Or(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `conditions` d'une condition OU doit être un tableau"),
                }
            },
            "non" => {
                if let Some(condition) = data.get("condition") {
                    Self::Not(Arc::new(Self::from_dict(condition)?))
                } else {
                    bail!("Une condition NON doit inclure une sous-condition")
                }
            },
            "objet_requis" => {
                Self::RequireItem(ItemFilter::from_dict(data.get("filtre").ok_or_else(|| anyhow!("OBJET_REQUIS doit inclure un filtre"))?)?)
            },
            "tag_requis" => {
                Self::RequireTag(TagFilter::from_dict(data.get("filtre").ok_or_else(|| anyhow!("TAG_REQUIS doit inclure un filtre"))?)?)
            },
            "attribut_egal" => {
                let attribut = match data.get("attribut") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("ATTRIBUT_EGAL doit inclure un champ `attribut`"),
                };
                Self::AttributeEq {
                    attribute: attribut,
                    value: data.get("valeur").cloned().ok_or_else(|| anyhow!("ATTRIBUT_EGAL doit inclure un champ `valeur`"))?,
                }
            },
            "attribut_superieur" => {
                let attribut = match data.get("attribut") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("ATTRIBUT_SUPERIEUR doit inclure un champ `attribut`"),
                };
                Self::AttributeGreater {
                    attribute: attribut,
                    value: data.get("valeur").cloned().ok_or_else(|| anyhow!("ATTRIBUT_SUPERIEUR doit inclure un champ `valeur`"))?,
                }
            },
            "attribut_inferieur" => {
                let attribut = match data.get("attribut") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("ATTRIBUT_INFERIEUR doit inclure un champ `attribut`"),
                };
                Self::AttributeLess {
                    attribute: attribut,
                    value: data.get("valeur").cloned().ok_or_else(|| anyhow!("ATTRIBUT_INFERIEUR doit inclure un champ `valeur`"))?,
                }
            },
            "formule" => {
                Self::Formula(data.get("formule").cloned().ok_or_else(|| anyhow!("FORMULE doit inclure une formule"))?)
            },
            "pv_egaux" => {
                Self::HpEq(data.get("valeur").cloned().ok_or_else(|| anyhow!("PV_EGAUX doit inclure un champ `valeur`"))?)
            },
            "pv_superieurs" => {
                Self::HpGreater(data.get("valeur").cloned().ok_or_else(|| anyhow!("PV_SUPERIEURS doit inclure un champ `valeur`"))?)
            },
            "pv_inferieurs" => {
                Self::HpLess(data.get("valeur").cloned().ok_or_else(|| anyhow!("PV_INFERIEURS doit inclure un champ `valeur`"))?)
            },
            "moral_egal" => {
                Self::MoraleEq(data.get("valeur").cloned().ok_or_else(|| anyhow!("MORAL_EGAL doit inclure un champ `valeur`"))?)
            },
            "moral_superieur" => {
                Self::MoraleGreater(data.get("valeur").cloned().ok_or_else(|| anyhow!("MORAL_SUPERIEUR doit inclure un champ `valeur`"))?)
            },
            "moral_inferieur" => {
                Self::MoraleLess(data.get("valeur").cloned().ok_or_else(|| anyhow!("MORAL_INFERIEUR doit inclure un champ `valeur`"))?)
            },
            "etat_requis" => {
                Self::RequireCondition(ConditionFilter::from_dict(data.get("filtre").ok_or_else(|| anyhow!("ETAT_REQUIS doit inclure un filtre"))?)?)
            },
            "aleatoire" => {
                let poids = match data.get("poids") {
                    Some(EvalValue::Int(i)) => EvalValue::Int(*i),
                    Some(EvalValue::Float(f)) => EvalValue::Float(*f),
                    Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                    _ => bail!("ALEATOIRE doit inclure un champ `poids`"),
                };
                Self::Random(poids)
            },
            "tag_page" => {
                Self::PageTag(TagFilter::from_dict(data.get("filtre").ok_or_else(|| anyhow!("TAG_PAGE doit inclure un filtre"))?)?)
            },
            "tag_connexion" => {
                Self::ConnectionTag(TagFilter::from_dict(data.get("filtre").ok_or_else(|| anyhow!("TAG_CONNEXION doit inclure un filtre"))?)?)
            },
            _ => bail!("Type de condition non supporté: {kind}"),
        })
    }

    /// Evaluates this predicate with additional bindings supplied by a lifecycle cell.
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
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches(
        &self,
        character: &Character,
        page_tags: Arc<IndexMap<Arc<str>, EvalValue>>,
        connection_tags: Arc<IndexMap<Arc<str>, EvalValue>>,
        tale: &crate::model::Tale,
        formula: &mut FormulaEngine,
        additional_env: &Env,
    ) -> Result<bool> {
        let result = (|| {
            match self {
            Self::And(values) => {
                for value in values.iter() {
                    if !value.matches(character, page_tags.clone(), connection_tags.clone(), tale, formula, additional_env)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(values) => {
                for value in values.iter() {
                    if value.matches(character, page_tags.clone(), connection_tags.clone(), tale, formula, additional_env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(value) => Ok(!value.matches(
                character,
                page_tags,
                connection_tags,
                tale,
                formula,
                additional_env,
            )?),
            Self::RequireItem(filter) => {
                let base_env = conditional_env(character, page_tags, formula, additional_env)?;
                for stack in &character.inventory {
                    if filter.matches(stack, formula, &base_env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::RequireTag(filter) => {
                let base_env = conditional_env(character, page_tags, formula, additional_env)?;
                for (tag, value) in character.story_tags.iter() {
                    if filter.matches(tag, value, character, formula, &base_env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::AttributeEq { attribute, value } => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.actual_attribute(attribute, formula, additional_env)? == formula.resolve_formula_int(value, &env)?)
            }
            Self::AttributeGreater { attribute, value } => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.actual_attribute(attribute, formula, additional_env)? > formula.resolve_formula_int(value, &env)?)
            }
            Self::AttributeLess { attribute, value } => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.actual_attribute(attribute, formula, additional_env)? < formula.resolve_formula_int(value, &env)?)
            }
            Self::Formula(value) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                formula.resolve_formula_bool(value, &env)
            }
            Self::HpEq(value) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.hp == formula.resolve_formula_int(value, &env)?)
            }
            Self::HpGreater(value) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.hp > formula.resolve_formula_int(value, &env)?)
            }
            Self::HpLess(value) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.hp < formula.resolve_formula_int(value, &env)?)
            }
            Self::MoraleEq(value) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.morale == formula.resolve_formula_int(value, &env)?)
            }
            Self::MoraleGreater(value) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.morale > formula.resolve_formula_int(value, &env)?)
            }
            Self::MoraleLess(value) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                Ok(character.morale < formula.resolve_formula_int(value, &env)?)
            }
            Self::RequireCondition(filter) => {
                let base_env = conditional_env(character, page_tags, formula, additional_env)?;
                for condition in &character.conditions {
                    if filter.matches(condition, formula, &base_env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Random(weight) => {
                let env = conditional_env(character, page_tags, formula, additional_env)?;
                let weight = formula.resolve_formula_int(weight, &env)?;
                Ok(formula.random_percent() < weight as f64)
            }
            Self::PageTag(filter) => {
                let env = conditional_env(character, page_tags.clone(), formula, additional_env)?;
                for (tag, value) in page_tags.iter() {
                    if filter.matches(tag, value, character, formula, &env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            },
            Self::ConnectionTag(filter) => {
                let env = conditional_env(character, page_tags.clone(), formula, additional_env)?;
                for (tag, value) in connection_tags.iter() {
                    if filter.matches(tag, value, character, formula, &env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            },
        }
        })();
        result.with_context(|| format!("Évaluation de la condition `{}`", self.kind()))
    }
}

/// Builds the formula environment used to evaluate a page conditional.
///
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
fn conditional_env<'a>(
    character: &Character,
    page_tags: Arc<IndexMap<Arc<str>, EvalValue>>,
    formula: &mut FormulaEngine,
    additional_env: &'a Env<'a>,
) -> Result<Env<'a>> {
    let mut env = character.env(formula, additional_env)?;
    env.insert("page", EvalValue::Object(page_tags.clone()));
    Ok(env.with_parent(additional_env))
}

/// Parses conditionals according to formula precedence rules.
///
/// Required fields and variant-specific JSON types are checked immediately. Formula
/// strings are preserved for runtime evaluation.
fn parse_conditionals(value: Option<&JsonValue>) -> Result<Vec<Conditional>> {
    match value {
        None => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(Conditional::from_json)
            .collect::<Result<Vec<_>>>(),
        Some(_) => bail!("Le champ `conditions` doit être un tableau"),
    }
}

/// Returns a required JSON field while preserving its original value type.
fn required_value(
    data: &serde_json::Map<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<JsonValue> {
    data.get(key)
        .cloned()
        .ok_or_else(|| anyhow!("{context} doit inclure un champ `{key}`"))
}

fn follow_map_path<'a>(data: &'a IndexMap<Arc<str>, EvalValue>, path: &[&str]) -> Option<&'a EvalValue> {
    let mut current: Option<&EvalValue> = None;
    for key in path {
        match (current, key.parse::<usize>().ok()) {
            (None, None) => {
                current = data.get(*key);
                if current == None {
                    return None;
                }
            },
            (Some(EvalValue::Object(obj)), None) => {
                current = obj.get(*key);
                if current == None {
                    return None;
                }
            },
            (Some(EvalValue::Array(arr)), Some(index)) => {
                current = arr.get(index);
                if current == None {
                    return None;
                }
            },
            _ => {
                return None;
            }
        }
    }
    current
}

/// Selects persistent-state instances by ID, traits, data, or formulas.
#[derive(Clone, Debug)]
pub enum ConditionFilter {
    And(Arc<[ConditionFilter]>),
    Or(Arc<[ConditionFilter]>),
    Not(Arc<ConditionFilter>),
    DataEq { key: Arc<str>, value: EvalValue },
    DataGreater { key: Arc<str>, value: EvalValue },
    DataLess { key: Arc<str>, value: EvalValue },
    Formula(EvalValue),
    TraitAll(Arc<[Deferred<Arc<str>>]>),
    TraitAny(Arc<[Deferred<Arc<str>>]>),
    Id(EvalValue),
}

impl ConditionFilter {
    /// Parses a persistent-state filter.
    ///
    /// A string is accepted as ID shorthand and arrays are interpreted as logical AND.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown filter types or malformed nested filters/values.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        if let JsonValue::Array(values) = value {
            return Ok(Self::And(
                values
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<Arc<_>>>()?,
            ));
        }
        if let JsonValue::String(id) = value {
            return Ok(Self::Id(split_fmt_str(id)?));
        }
        let data = object(value, "Un filtre d'état")?;
        let kind = required_string(data, "type", "Un filtre d'état")?;
        Ok(match kind.as_str() {
            "et" => Self::And(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "ou" => Self::Or(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "non" => Self::Not(Arc::new(Self::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("Un filtre d'état NON doit inclure `filtre`"))?,
            )?)),
            "donnee_egale" | "egale" => Self::DataEq {
                key: required_string(data, "cle", "DONNEE_EGALE")?.into(),
                value: value_or_formula(value_or(data, "valeur", JsonValue::Null).try_into()?)?,
            },
            "donnee_superieure" | "superieure" => Self::DataGreater {
                key: required_string(data, "cle", "DONNEE_SUPERIEURE")?.into(),
                value: value_or_formula(value_or(data, "valeur", JsonValue::Null).try_into()?)?,
            },
            "donnee_inferieure" | "inferieure" => Self::DataLess {
                key: required_string(data, "cle", "DONNEE_INFERIEURE")?.into(),
                value: value_or_formula(value_or(data, "valeur", JsonValue::Null).try_into()?)?,
            },
            "formule" => Self::Formula(value_or_formula(required_value(data, "formule", "FORMULE")?.try_into()?)?),
            "tous_les_traits" | "tous" => Self::TraitAll(collect_traits(data.get("traits"))
                .with_context(|| "Parsing des traits pour filtre 'tous_les_traits'")?),
            "un_des_traits" | "un" => Self::TraitAny(collect_traits(data.get("traits"))
                .with_context(|| "Parsing des traits pour filtre 'un_des_traits'")?),
            "id" => Self::Id(split_fmt_str(&required_string(data, "id", "Filtre ID")?)?),
            _ => bail!("Type de filtre d'état non supporté: {kind}"),
        })
    }
    /// Builds a persistent-state filter from formula-produced data.
    ///
    /// String shorthand selects an ID and arrays mean logical AND.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed runtime values or unknown filter variants.
    pub fn from_dict(data: &EvalValue) -> Result<Self> {
        let data = match data {
            EvalValue::Array(arr) => {
                return Ok(Self::And(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?));
            },
            EvalValue::String(_s) => {
                return Ok(Self::Id(data.clone()));
            },
            EvalValue::Func(_f) => {
                return Ok(Self::Formula(data.clone()));
            },
            EvalValue::Object(obj) => obj,
            _ => bail!("Un filtre d'état doit être un tableau ou un objet"),
        };
        let kind = if let EvalValue::String(s) = data
            .get("type")
            .ok_or_else(|| anyhow!("Un filtre d'état doit inclure un champ `type`"))? {
            s.clone()
        } else {
            bail!("Le champ `type` d'un filtre d'état doit être une chaîne de caractères")
        };
        Ok(match kind.as_ref() {
            "et" => {
                match data.get("filtres") {
                    Some(EvalValue::Array(arr)) => {
                        Self::And(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::And(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::And(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `filtres` d'un filtre d'état ET doit être un tableau"),
                }
            },
            "ou" => {
                match data.get("filtres") {
                    Some(EvalValue::Array(arr)) => {
                        Self::Or(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::Or(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::Or(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `filtres` d'un filtre d'état OU doit être un tableau"),
                }
            },
            "non" => {
                if let Some(filter) = data.get("filtre") {
                    Self::Not(Arc::new(Self::from_dict(filter)?))
                } else {
                    bail!("Un filtre d'état NON doit inclure un sous-filtre")
                }
            },
            "donnee_egale" | "egale" => Self::DataEq {
                key: match data.get("cle") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("DONNEE_EGALE doit inclure un champ `cle`"),
                },
                value: data.get("valeur").cloned().ok_or_else(|| anyhow!("DONNEE_EGALE doit inclure un champ `valeur`"))?,
            },
            "donnee_superieure" | "superieure" => Self::DataGreater {
                key: match data.get("cle") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("DONNEE_SUPERIEURE doit inclure un champ `cle`"),
                },
                value: data.get("valeur").cloned().ok_or_else(|| anyhow!("DONNEE_SUPERIEURE doit inclure un champ `valeur`"))?,
            },
            "donnee_inferieure" | "inferieure" => Self::DataLess {
                key: match data.get("cle") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("DONNEE_INFERIEURE doit inclure un champ `cle`"),
                },
                value: data.get("valeur").cloned().ok_or_else(|| anyhow!("DONNEE_INFERIEURE doit inclure un champ `valeur`"))?,
            },
            "formule" => Self::Formula(data.get("formule").cloned().ok_or_else(|| anyhow!("FORMULE doit inclure un champ `formule`"))?),
            "tous_les_traits" | "tous" => {
                let traits = match data.get("traits") {
                    Some(EvalValue::Array(arr)) => {
                        arr
                            .iter()
                            .map(|val| match val {
                                EvalValue::String(s) => Ok(Deferred::Literal(s.clone())),
                                EvalValue::Func(_) => Ok(Deferred::Formula(val.clone())),
                                _ => bail!("Le champ `traits` d'un filtre d'état TOUS doit être un tableau de chaînes ou de formules"),
                            }).collect::<Result<Vec<_>>>()?
                    },
                    Some(f @ EvalValue::Func(_)) => vec![Deferred::Formula(f.clone())],
                    Some(EvalValue::String(s)) => vec![Deferred::Literal(s.clone())],
                    None | Some(EvalValue::Null) => Vec::new(),
                    _ => bail!("Le champ `traits` d'un filtre d'état TOUS doit être un tableau ou une chaîne"),
                }.into();
                Self::TraitAll(traits)
            },
            "un_des_traits" | "un" => {
                let traits = match data.get("traits") {
                    Some(EvalValue::Array(arr)) => {
                        arr
                            .iter()
                            .map(|val| match val {
                                EvalValue::String(s) => Ok(Deferred::Literal(s.clone())),
                                EvalValue::Func(_) => Ok(Deferred::Formula(val.clone())),
                                _ => bail!("Le champ `traits` d'un filtre d'état UN doit être un tableau de chaînes ou de formules"),
                            }).collect::<Result<Vec<_>>>()?
                    },
                    Some(f @ EvalValue::Func(_)) => vec![Deferred::Formula(f.clone())],
                    Some(EvalValue::String(s)) => vec![Deferred::Literal(s.clone())],
                    None | Some(EvalValue::Null) => Vec::new(),
                    _ => bail!("Le champ `traits` d'un filtre d'état UN doit être un tableau ou une chaîne"),
                }.into();
                Self::TraitAny(traits)
            },
            "id" => {
                let id = match data.get("id") {
                    Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                    Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
                    _ => bail!("Filtre ID doit inclure un champ `id`"),
                };
                Self::Id(id)
            },
            _ => bail!("Type de filtre d'état non supporté: {kind}"),
        })
    }

    /// Evaluates this predicate against the supplied runtime value.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `env` — Additional formula bindings for the current lifecycle operation.
    ///
    /// # Errors
    ///
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches(
        &self,
        condition: &ConditionInstance,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<bool> {
        match self {
            Self::And(filters) => all_condition_filters(filters, condition, formula, &env),
            Self::Or(filters) => any_condition_filters(filters, condition, formula, &env),
            Self::Not(filter) => Ok(!filter.matches(condition, formula, env)?),
            Self::DataEq { key, value } => {
                let env = condition.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'etat pour un filtre de donnée égale")?
                    .with_parent(env);
                let expected = formula.resolve_formula(value, &env)?;
                let path = key.split('.').collect::<Vec<_>>();
                Ok(follow_map_path(&condition.data, &path)
                    .is_some_and(|actual| actual == &expected))
            }
            Self::DataGreater { key, value } => {
                let env = condition.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'etat pour un filtre de donnée supérieur")?
                    .with_parent(env);
                let expected = match formula.resolve_formula_float(value, &env) {
                    Ok(value) => value,
                    Err(_) => return Ok(false),
                };
                let path = key.split('.').collect::<Vec<_>>();
                Ok(follow_map_path(&condition.data, &path)
                    .is_some_and(|actual| actual > &expected))
            }
            Self::DataLess { key, value } => {
                let env = condition.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'etat pour un filtre de donnée inférieur")?
                    .with_parent(env);
                let expected = match formula.resolve_formula_float(value, &env) {
                    Ok(value) => value,
                    Err(_) => return Ok(false),
                };
                let path = key.split('.').collect::<Vec<_>>();
                Ok(follow_map_path(&condition.data, &path)
                    .is_some_and(|actual| actual < &expected))
            }
            Self::Formula(value) => {
                let env = condition.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'etat pour un filtre de formule")?
                    .with_parent(env);
                formula.resolve_formula_bool(value, &env)
            },
            Self::TraitAll(traits) => {
                let condition_env = condition.env(formula, env)?;
                // extract the traits
                let condition_traits = condition_env
                    .get("etat")
                    .map(|obj| {
                        if let EvalValue::Object(obj_map) = obj {
                            obj_map.get("traits").cloned()
                        } else {
                            None
                        }
                    }).expect("Condition env should have 'etat' key").expect("'etat' should have 'traits' key");
                let EvalValue::Array(condtion_traits) = condition_traits else {
                    panic!("Condition's 'etat.traits' should be an array");
                };
                let env = condition_env.with_parent(env);
                let target_traits = eval_traits(traits, formula, &env)?;
                Ok(target_traits.iter().all(|trait_name| condtion_traits.contains(trait_name)))
            },
            Self::TraitAny(traits) => {
                let condition_env = condition.env(formula, env)?;
                // extract the traits
                let condition_traits = condition_env
                    .get("etat")
                    .map(|obj| {
                        if let EvalValue::Object(obj_map) = obj {
                            obj_map.get("traits").cloned()
                        } else {
                            None
                        }
                    }).expect("Condition env should have 'etat' key").expect("'etat' should have 'traits' key");
                let EvalValue::Array(condtion_traits) = condition_traits else {
                    panic!("Condition's 'etat.traits' should be an array");
                };
                let env = condition_env.with_parent(env);
                let target_traits = eval_traits(traits, formula, &env)?;
                Ok(target_traits.iter().any(|trait_name| condtion_traits.contains(trait_name)))
            },
            Self::Id(id) => {
                let env = condition.env(formula, env)
                    .with_context(|| "Erreur en créant l'environement d'état pour un filtre id")?
                    .with_parent(env);
                Ok(condition.template.id == formula.fmt_value(id, &env)?)
            },
        }
    }

    /// Returns whether a formula-facing state snapshot satisfies this tale filter.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `env` — Additional formula bindings for the current lifecycle operation.
    ///
    /// # Errors
    ///
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches_view(
        &self,
        condition: &FormulaConditionView,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<bool> {
        match self {
            Self::And(filters) => {
                for filter in filters.iter() {
                    if !filter.matches_view(condition, formula, env)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(filters) => {
                for filter in filters.iter() {
                    if filter.matches_view(condition, formula, env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(filter) => Ok(!filter.matches_view(condition, formula, env)?),
            Self::DataEq { key, value } => {
                let expected = formula.resolve_formula(value, &env.merged(&condition.env()))?;
                Ok(condition
                    .data
                    .get(key)
                    .is_some_and(|actual| actual == &expected))
            }
            Self::DataGreater { key, value } => {
                let expected = match formula.resolve_formula_float(value, &env.merged(&condition.env())) {
                    Ok(value) => value,
                    Err(_) => return Ok(false),
                };
                Ok(condition
                    .data
                    .get(key)
                    .is_some_and(|actual| actual > &expected))
            }
            Self::DataLess { key, value } => {
                let expected = match formula.resolve_formula_float(value, &env.merged(&condition.env())) {
                    Ok(value) => value,
                    Err(_) => return Ok(false),
                };
                Ok(condition
                    .data
                    .get(key)
                    .is_some_and(|actual| actual < &expected))
            }
            Self::Formula(value) => formula.resolve_formula_bool(value, &env.merged(&condition.env())),
            Self::Id(id) => {
                let env = condition.env().with_parent(env);
                Ok(condition.id == formula.fmt_value(id, &env)?)
            },
            Self::TraitAll(traits) => {
                let cond_env = condition.env().with_parent(env);
                let targets = eval_traits(traits, formula, &cond_env)
                    .with_context(|| "Erreur lors de l'évaluation des traits d'un filtre d'état 'tous les traits'")?;
                let res = targets.iter().try_fold(true, |matched, trait_name| {
                    let EvalValue::String(trait_name) = trait_name else {
                        bail!(
                            "Trait évalué d'un filtre d'état 'tous les traits' \
                            n'est pas une chaîne : {trait_name:?}"
                        );
                    };

                    Ok(matched && condition.traits.contains(trait_name))
                })?;
                Ok(res)
            },
            Self::TraitAny(traits) => {
                let cond_env = condition.env().with_parent(env);
                let targets = eval_traits(traits, formula, &cond_env)
                    .with_context(|| "Erreur lors de l'évaluation des traits d'un filtre d'état 'au moins un des traits'")?;
                let res = targets.iter().try_fold(false, |matched, trait_name| {
                    let EvalValue::String(trait_name) = trait_name else {
                        bail!(
                            "Trait évalué d'un filtre d'état 'au moins un des traits' \
                            n'est pas une chaîne : {trait_name:?}"
                        );
                    };

                    Ok(matched || condition.traits.contains(trait_name))
                })?;
                Ok(res)
            },
        }
    }
}

/// Returns whether every state filter in a list matches the instance.
fn all_condition_filters(
    filters: &[ConditionFilter],
    condition: &ConditionInstance,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<bool> {
    for filter in filters {
        if !filter.matches(condition, formula, env)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Returns whether at least one state filter in a list matches the instance.
fn any_condition_filters(
    filters: &[ConditionFilter],
    condition: &ConditionInstance,
    formula: &mut FormulaEngine,
    env: &Env,
) -> Result<bool> {
    for filter in filters {
        if filter.matches(condition, formula, env)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Selects checks by attribute, traits, or formula predicates.
#[derive(Clone, Debug)]
pub enum CheckFilter {
    And(Arc<[CheckFilter]>),
    Or(Arc<[CheckFilter]>),
    Not(Arc<CheckFilter>),
    HasAttribute(Arc<[Arc<str>]>),
    TraitAll(Arc<[Arc<str>]>),
    TraitAny(Arc<[Arc<str>]>),
    Formula(EvalValue),
}

impl CheckFilter {
    /// Parses a check filter over attribute, traits, and optional formulas.
    ///
    /// Arrays are interpreted as logical AND.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown filter types or malformed operands.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        if let JsonValue::Array(values) = value {
            return Ok(Self::And(
                values
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<Arc<_>>>()?,
            ));
        }
        let data = object(value, "Un filtre de test")?;
        let kind = required_string(data, "type", "Un filtre de test")?;
        Ok(match kind.as_str() {
            "et" => Self::And(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "ou" => Self::Or(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "non" => Self::Not(Arc::new(Self::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("Un filtre de test NON doit inclure `filtre`"))?,
            )?)),
            "attribut_parmi" => Self::HasAttribute(parse_string_or_list(
                data.get("attribut"),
                "Le champ `attribut` du filtre",
            )?),
            "tous_les_traits" | "tous" => Self::TraitAll(parse_traits(data.get("traits"), true)?),
            "un_des_traits" | "un" => Self::TraitAny(parse_traits(data.get("traits"), true)?),
            "formule" => Self::Formula(value_or_formula(required_value(data, "formule", "FORMULE")?.try_into()?)?),
            _ => bail!("Type de filtre de test non supporté: {kind}"),
        })
    }

    /// Evaluates this predicate against the supplied runtime value.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `data` — Source object or runtime data used by the operation.
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    ///
    /// # Errors
    ///
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches(
        &self,
        attribute: Option<Arc<str>>,
        traits: &[Arc<str>],
        data: &Env,
        formula: &mut FormulaEngine,
    ) -> Result<bool> {
        match self {
            Self::And(filters) => {
                for filter in filters.iter() {
                    if !filter.matches(attribute.clone(), traits, data, formula)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(filters) => {
                for filter in filters.iter() {
                    if filter.matches(attribute.clone(), traits, data, formula)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(filter) => Ok(!filter.matches(attribute.clone(), traits, data, formula)?),
            Self::HasAttribute(attributes) => if let Some(attribute) = attribute {
                Ok(attributes.iter().any(|item| item == &attribute))
            } else {
                Ok(false)
            },
            Self::TraitAll(required) => Ok(required.iter().all(|item| traits.contains(item))),
            Self::TraitAny(required) => Ok(required.iter().any(|item| traits.contains(item))),
            Self::Formula(value) => {
                let mut env = data.clone();
                env.insert("attribut", match attribute {
                    Some(attr) => EvalValue::String(attr.clone()),
                    None => EvalValue::Null,
                });
                env.insert(
                    "traits",
                    EvalValue::Array(traits.iter().cloned().map(EvalValue::String).collect()),
                );
                formula.resolve_formula_bool(value, &env)
            }
        }
    }
}

/// Selects inventory stacks by ID, quantity, traits, data, or formulas.
#[derive(Clone, Debug)]
pub enum ItemFilter {
    And(Arc<[ItemFilter]>),
    Or(Arc<[ItemFilter]>),
    Not(Arc<ItemFilter>),
    Id(EvalValue),
    DataEq { key: Arc<str>, value: EvalValue },
    DataGreater { key: Arc<str>, value: EvalValue },
    DataLess { key: Arc<str>, value: EvalValue },
    QuantityEq(EvalValue),
    QuantityGreater(EvalValue),
    QuantityLess(EvalValue),
    TraitAll(Arc<[Deferred<Arc<str>>]>),
    TraitAny(Arc<[Deferred<Arc<str>>]>),
    Formula(EvalValue),
}

impl ItemFilter {
    /// Parses an inventory-stack filter.
    ///
    /// A string is accepted as item-ID shorthand and arrays are interpreted as logical AND. Formula comparison values remain deferred.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown filter types or malformed operands.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        if let JsonValue::Array(values) = value {
            return Ok(Self::And(
                values
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<Arc<_>>>()?,
            ));
        }
        if let JsonValue::String(id) = value {
            return Ok(Self::Id(split_fmt_str(id)?));
        }
        let data = object(value, "Un filtre d'objet")?;
        let kind = required_string(data, "type", "Un filtre d'objet")?;
        Ok(match kind.as_str() {
            "et" => Self::And(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "ou" => Self::Or(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "non" => Self::Not(Arc::new(Self::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("Un filtre d'objet NON doit inclure `filtre`"))?,
            )?)),
            "id" => Self::Id(split_fmt_str(&required_string(data, "id", "Filtre ID")?)?),
            "donnee_egale" => Self::DataEq {
                key: required_string(data, "cle", "DONNEE_EGALE")?.into(),
                value: value_or_formula(value_or(data, "valeur", JsonValue::Null).try_into()?)?,
            },
            "donnee_superieure" => Self::DataGreater {
                key: required_string(data, "cle", "DONNEE_SUPERIEURE")?.into(),
                value: value_or_formula(value_or(data, "valeur", JsonValue::Null).try_into()?)?,
            },
            "donnee_inferieure" => Self::DataLess {
                key: required_string(data, "cle", "DONNEE_INFERIEURE")?.into(),
                value: value_or_formula(value_or(data, "valeur", JsonValue::Null).try_into()?)?,
            },
            "quantite_egale" => {
                Self::QuantityEq(value_or_formula(required_value(data, "quantite", "QUANTITE_EGALE")?.try_into()?)?)
            }
            "quantite_superieure" => Self::QuantityGreater(
                value_or_formula(required_value(data, "quantite", "QUANTITE_SUPERIEURE")?.try_into()?)?,
            ),
            "quantite_inferieure" => Self::QuantityLess(
                value_or_formula(required_value(data, "quantite", "QUANTITE_INFERIEURE")?.try_into()?)?,
            ),
            "tous_les_traits" | "tous" => Self::TraitAll(collect_traits(data.get("traits"))
                .with_context(|| "Parsing des traits pour filtre 'tous_les_traits'")?),
            "un_des_traits" | "un" => Self::TraitAny(collect_traits(data.get("traits"))
                .with_context(|| "Parsing des traits pour filtre 'un_des_traits'")?),
            "formule" => Self::Formula(value_or_formula(required_value(data, "formule", "FORMULE")?.try_into()?)?),
            _ => bail!("Type de filtre d'objet non supporté: {kind}"),
        })
    }
    /// Builds an item filter from formula-produced data.
    ///
    /// String shorthand selects an ID and arrays mean logical AND.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed runtime values or unknown filter variants.
    pub fn from_dict(data: &EvalValue) -> Result<Self> {
        let data = match data {
            EvalValue::Array(arr) => {
                return Ok(Self::And(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?));
            },
            EvalValue::String(_s) => {
                return Ok(Self::Id(data.clone()));
            },
            EvalValue::Func(_f) => {
                return Ok(Self::Formula(data.clone()));
            }
            EvalValue::Object(obj) => obj,
            _ => bail!("Un filtre d'objet doit être un tableau ou un objet"),
        };
        let kind = if let EvalValue::String(s) = data
            .get("type")
            .ok_or_else(|| anyhow!("Un filtre d'objet doit inclure un champ `type`"))? {
            s.clone()
        } else {
            bail!("Le champ `type` d'un filtre d'objet doit être une chaîne de caractères")
        };
        Ok(match kind.as_ref() {
            "et" => {
                match data.get("filtres") {
                    Some(EvalValue::Array(arr)) => {
                        Self::And(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::And(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::And(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `filtres` d'un filtre d'objet ET doit être un tableau"),
                }
            },
            "ou" => {
                match data.get("filtres") {
                    Some(EvalValue::Array(arr)) => {
                        Self::Or(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::Or(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::Or(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `filtres` d'un filtre d'objet OU doit être un tableau"),
                }
            },
            "non" => {
                if let Some(filter) = data.get("filtre") {
                    Self::Not(Arc::new(Self::from_dict(filter)?))
                } else {
                    bail!("Un filtre d'objet NON doit inclure un sous-filtre")
                }
            },
            "id" => {
                let id = match data.get("id") {
                    Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                    Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
                    _ => bail!("Filtre ID doit inclure un champ `id`"),
                };
                Self::Id(id)
            },
            "donnee_egale" => Self::DataEq {
                key: match data.get("cle") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("DONNEE_EGALE doit inclure un champ `cle`"),
                },
                value: data.get("valeur").cloned().ok_or_else(|| anyhow!("DONNEE_EGALE doit inclure un champ `valeur`"))?,
            },
            "donnee_superieure" => Self::DataGreater {
                key: match data.get("cle") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("DONNEE_SUPERIEURE doit inclure un champ `cle`"),
                },
                value: data.get("valeur").cloned().ok_or_else(|| anyhow!("DONNEE_SUPERIEURE doit inclure un champ `valeur`"))?,
            },
            "donnee_inferieure" => Self::DataLess {
                key: match data.get("cle") {
                    Some(EvalValue::String(s)) => s.clone(),
                    _ => bail!("DONNEE_INFERIEURE doit inclure un champ `cle`"),
                },
                value: data.get("valeur").cloned().ok_or_else(|| anyhow!("DONNEE_INFERIEURE doit inclure un champ `valeur`"))?,
            },
            "quantite_egale" => {
                let quantity = match data.get("quantite") {
                    Some(value) => value.clone(),
                    None => bail!("QUANTITE_EGALE doit inclure un champ `quantite`"),
                };
                Self::QuantityEq(quantity)
            }
            "quantite_superieure" => {
                let quantity = match data.get("quantite") {
                    Some(value) => value.clone(),
                    None => bail!("QUANTITE_SUPERIEURE doit inclure un champ `quantite`"),
                };
                Self::QuantityGreater(quantity)
            }
            "quantite_inferieure" => {
                let quantity = match data.get("quantite") {
                    Some(value) => value.clone(),
                    None => bail!("QUANTITE_INFERIEURE doit inclure un champ `quantite`"),
                };
                Self::QuantityLess(quantity)
            }
            "tous_les_traits" | "tous" => {
                let traits = match data.get("traits") {
                    Some(EvalValue::Array(arr)) => {
                        arr
                            .iter()
                            .map(|val| match val {
                                EvalValue::String(s) => Ok(Deferred::Literal(s.clone())),
                                EvalValue::Func(_) => Ok(Deferred::Formula(val.clone())),
                                _ => bail!("Le champ `traits` d'un filtre d'état UN doit être un tableau de chaînes ou de formules"),
                            }).collect::<Result<Vec<_>>>()?
                    },
                    Some(f @ EvalValue::Func(_)) => vec![Deferred::Formula(f.clone())],
                    Some(EvalValue::String(s)) => vec![Deferred::Literal(s.clone())],
                    None | Some(EvalValue::Null) => Vec::new(),
                    _ => bail!("Le champ `traits` d'un filtre d'état UN doit être un tableau ou une chaîne"),
                }.into();
                Self::TraitAll(traits)
            },
            "un_des_traits" | "un" => {
                let traits = match data.get("traits") {
                    Some(EvalValue::Array(arr)) => {
                        arr
                            .iter()
                            .map(|val| match val {
                                EvalValue::String(s) => Ok(Deferred::Literal(s.clone())),
                                EvalValue::Func(_) => Ok(Deferred::Formula(val.clone())),
                                _ => bail!("Le champ `traits` d'un filtre d'état UN doit être un tableau de chaînes ou de formules"),
                            }).collect::<Result<Vec<_>>>()?
                    },
                    Some(f @ EvalValue::Func(_)) => vec![Deferred::Formula(f.clone())],
                    Some(EvalValue::String(s)) => vec![Deferred::Literal(s.clone())],
                    None | Some(EvalValue::Null) => Vec::new(),
                    _ => bail!("Le champ `traits` d'un filtre d'état UN doit être un tableau ou une chaîne"),
                }.into();
                Self::TraitAny(traits)
            },
            "formule" => {
                let formula = match data.get("formule") {
                    Some(value) => value.clone(),
                    None => bail!("FORMULE doit inclure un champ `formule`"),
                };
                Self::Formula(formula)
            }
            _ => bail!("Type de filtre d'objet non supporté: {kind}"),
        })
    }

    /// Evaluates this predicate against the supplied runtime value.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `env` — Additional formula bindings for the current lifecycle operation.
    ///
    /// # Errors
    ///
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches(
        &self,
        stack: &ItemStack,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<bool> {
        match self {
            Self::And(filters) => {
                for filter in filters.iter() {
                    if !filter.matches(stack, formula, env)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(filters) => {
                for filter in filters.iter() {
                    if filter.matches(stack, formula, env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(filter) => Ok(!filter.matches(stack, formula, env)?),
            Self::Id(id) => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre id")?
                    .with_parent(env);
                Ok(stack.item.id == formula.fmt_value(id, &env)?)
            },
            Self::DataEq { key, value } => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre de donnée égale")?
                    .with_parent(env);
                let expected = formula.resolve_formula(value, &env)?;
                let path = key.split('.').collect::<Vec<_>>();
                Ok(follow_map_path(&stack.data, &path)
                    .is_some_and(|actual| actual == &expected))
            }
            Self::DataGreater { key, value } => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre de donnée supérieure")?
                    .with_parent(env);
                let expected = formula.resolve_formula_float(value, &env)?;
                let path = key.split('.').collect::<Vec<_>>();
                Ok(follow_map_path(&stack.data, &path)
                    .is_some_and(|actual| actual > &expected))
            }
            Self::DataLess { key, value } => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre de donnée inférieure")?
                    .with_parent(env);
                let expected = formula.resolve_formula_float(value, &env)?;
                let path = key.split('.').collect::<Vec<_>>();
                Ok(follow_map_path(&stack.data, &path)
                    .is_some_and(|actual| actual < &expected))
            }
            Self::QuantityEq(value) => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre de quantité égale")?
                    .with_parent(env);
                Ok(stack.quantity == formula.resolve_formula_int(value, &env)?)
            }
            Self::QuantityGreater(value) => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre de quantité supérieure")?
                    .with_parent(env);
                Ok(stack.quantity > formula.resolve_formula_int(value, &env)?)
            }
            Self::QuantityLess(value) => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre de quantité inférieure")?
                    .with_parent(env);
                Ok(stack.quantity < formula.resolve_formula_int(value, &env)?)
            }
            Self::TraitAll(traits) => {
                let condition_env = stack.env(formula, env)?;
                // extract the traits
                let stack_traits = condition_env
                    .get("objet")
                    .map(|obj| {
                        if let EvalValue::Object(obj_map) = obj {
                            obj_map.get("traits").cloned()
                        } else {
                            None
                        }
                    }).expect("Condition env should have 'objet' key").expect("'objet' should have 'traits' key");
                let EvalValue::Array(stack_traits) = stack_traits else {
                    panic!("Condition's 'objet.traits' should be an array");
                };
                let env = condition_env.with_parent(env);
                let target_traits = eval_traits(traits, formula, &env)?;
                Ok(target_traits.iter().all(|trait_name| stack_traits.contains(trait_name)))
            },
            Self::TraitAny(traits) => {
                let condition_env = stack.env(formula, env)?;
                // extract the traits
                let stack_traits = condition_env
                    .get("objet")
                    .map(|obj| {
                        if let EvalValue::Object(obj_map) = obj {
                            obj_map.get("traits").cloned()
                        } else {
                            None
                        }
                    }).expect("Condition env should have 'objet' key").expect("'objet' should have 'traits' key");
                let EvalValue::Array(stack_traits) = stack_traits else {
                    panic!("Condition's 'objet.traits' should be an array");
                };
                let env = condition_env.with_parent(env);
                let target_traits = eval_traits(traits, formula, &env)?;
                Ok(target_traits.iter().any(|trait_name| stack_traits.contains(trait_name)))
            },
            Self::Formula(value) => {
                let env = stack.env(formula, env)
                    .with_context(|| "Erreur en créant l'environnement d'objet pour un filtre de formule")?
                    .with_parent(env);
                formula.resolve_formula_bool(value, &env)
            },
        }
    }

    /// Returns whether a formula-facing inventory snapshot satisfies this tale filter.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `env` — Additional formula bindings for the current lifecycle operation.
    ///
    /// # Errors
    ///
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches_view(
        &self,
        item: &FormulaItemView,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<bool> {
        match self {
            Self::And(filters) => {
                for filter in filters.iter() {
                    if !filter.matches_view(item, formula, env)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(filters) => {
                for filter in filters.iter() {
                    if filter.matches_view(item, formula, env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(filter) => Ok(!filter.matches_view(item, formula, env)?),
            Self::Id(id) => Ok(item.id == formula.fmt_value(id, &env.merged(&item.env()))?),
            Self::DataEq { key, value } => {
                let expected = formula.resolve_formula(value, &env.merged(&item.env()))?;
                Ok(item.data.get(key).is_some_and(|actual| actual == &expected))
            }
            Self::DataGreater { key, value } => {
                let expected = formula.resolve_formula_float(value, &env.merged(&item.env()))?;
                Ok(item.data.get(key).is_some_and(|actual| actual > &expected))
            }
            Self::DataLess { key, value } => {
                let expected = formula.resolve_formula_float(value, &env.merged(&item.env()))?;
                Ok(item.data.get(key).is_some_and(|actual| actual < &expected))
            }
            Self::QuantityEq(value) => {
                Ok(item.quantity == formula.resolve_formula_int(value, &env.merged(&item.env()))?)
            }
            Self::QuantityGreater(value) => {
                Ok(item.quantity > formula.resolve_formula_int(value, &env.merged(&item.env()))?)
            }
            Self::QuantityLess(value) => {
                Ok(item.quantity < formula.resolve_formula_int(value, &env.merged(&item.env()))?)
            }
            Self::TraitAll(traits) => {
                let cond_env = item.env().with_parent(env);
                let targets = eval_traits(traits, formula, &cond_env)
                    .with_context(|| "Erreur lors de l'évaluation des traits d'un filtre d'objet 'tous les traits'")?;
                let res = targets.iter().try_fold(true, |matched, trait_name| {
                    let EvalValue::String(trait_name) = trait_name else {
                        bail!(
                            "Trait évalué d'un filtre d'objet 'tous les traits' \
                            n'est pas une chaîne : {trait_name:?}"
                        );
                    };

                    Ok(matched && item.traits.contains(trait_name))
                })?;
                Ok(res)
            },
            Self::TraitAny(traits) => {
                let cond_env = item.env().with_parent(env);
                let targets = eval_traits(traits, formula, &cond_env)
                    .with_context(|| "Erreur lors de l'évaluation des traits d'un filtre d'objet 'au moins un des traits'")?;
                let res = targets.iter().try_fold(false, |matched, trait_name| {
                    let EvalValue::String(trait_name) = trait_name else {
                        bail!(
                            "Trait évalué d'un filtre d'objet 'au moins un des traits' \
                            n'est pas une chaîne : {trait_name:?}"
                        );
                    };

                    Ok(matched || item.traits.contains(trait_name))
                })?;
                Ok(res)
            },
            Self::Formula(value) => formula.resolve_formula_bool(value, &env.merged(&item.env())),
        }
    }
}

/// Selects story tags by ID, value, or formula predicates.
#[derive(Clone, Debug)]
pub enum TagFilter {
    And(Arc<[TagFilter]>),
    Or(Arc<[TagFilter]>),
    Not(Arc<TagFilter>),
    ValueEq{
        value: EvalValue,
        path: Option<EvalValue>,
    },
    ValueGreater{
        value: EvalValue,
        path: Option<EvalValue>,
    },
    ValueLess{
        value: EvalValue,
        path: Option<EvalValue>,
    },
    Id(EvalValue),
    Formula(EvalValue),
}

impl TagFilter {
    /// Parses a story-tag filter, including nested paths for ID and value comparisons.
    ///
    /// A string is accepted as ID/path shorthand and arrays are interpreted as logical AND.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown filter types or malformed paths/values.
    pub fn from_json(value: &JsonValue) -> Result<Self> {
        if let JsonValue::Array(values) = value {
            return Ok(Self::And(
                values
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<Arc<_>>>()?,
            ));
        }
        if let JsonValue::String(id) = value {
            return Ok(Self::Id(split_fmt_str(id)?));
        }
        let data = object(value, "Un filtre de tag")?;
        let kind = required_string(data, "type", "Un filtre de tag")?;
        Ok(match kind.as_str() {
            "et" => Self::And(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "ou" => Self::Or(parse_filter_array(data.get("filtres"), Self::from_json)?.into()),
            "non" => Self::Not(Arc::new(Self::from_json(
                data.get("filtre")
                    .ok_or_else(|| anyhow!("Un filtre de tag NON doit inclure `filtre`"))?,
            )?)),
            "valeur_egale" | "egale" => {
                let path = data.get("chemin");
                let path = match path {
                    Some(JsonValue::String(path)) => Some(path.clone()),
                    Some(JsonValue::Array(arr)) => {
                        let mut parts = Vec::new();
                        for part in arr {
                            if let JsonValue::String(s) = part {
                                parts.push(s.clone());
                            } else {
                                bail!("Le champ `chemin` du filtre de tag doit être une chaîne ou une liste de chaînes");
                            }
                        }
                        Some(parts.join("."))
                    },
                    Some(_) => bail!("Le champ `chemin` du filtre de tag doit être une chaîne ou une liste de chaînes"),
                    None => None,
                }.map(|s| split_fmt_str(&s)).transpose()?;
                Self::ValueEq{
                    value: value_or_formula(required_value(data, "valeur", "VALEUR_EGALE")?.try_into()?)?,
                    path,
                }
            }
            "valeur_superieure" | "superieure" => {
                let path = data.get("chemin");
                let path = match path {
                    Some(JsonValue::String(path)) => Some(path.clone()),
                    Some(JsonValue::Array(arr)) => {
                        let mut parts = Vec::new();
                        for part in arr {
                            if let JsonValue::String(s) = part {
                                parts.push(s.clone());
                            } else {
                                bail!("Le champ `chemin` du filtre de tag doit être une chaîne ou une liste de chaînes");
                            }
                        }
                        Some(parts.join("."))
                    },
                    Some(_) => bail!("Le champ `chemin` du filtre de tag doit être une chaîne ou une liste de chaînes"),
                    None => None,
                }.map(|s| split_fmt_str(&s)).transpose()?;
                Self::ValueGreater{
                    value: value_or_formula(required_value(data, "valeur", "VALEUR_SUPERIEURE")?.try_into()?)?,
                    path,
                }
            }
            "valeur_inferieure" | "inferieure" => {
                let path = data.get("chemin");
                let path = match path {
                    Some(JsonValue::String(path)) => Some(path.clone()),
                    Some(JsonValue::Array(arr)) => {
                        let mut parts = Vec::new();
                        for part in arr {
                            if let JsonValue::String(s) = part {
                                parts.push(s.clone());
                            } else {
                                bail!("Le champ `chemin` du filtre de tag doit être une chaîne ou une liste de chaînes");
                            }
                        }
                        Some(parts.join("."))
                    },
                    Some(_) => bail!("Le champ `chemin` du filtre de tag doit être une chaîne ou une liste de chaînes"),
                    None => None,
                }.map(|s| split_fmt_str(&s)).transpose()?;
                Self::ValueLess{
                    value: value_or_formula(required_value(data, "valeur", "VALEUR_INFERIEURE")?.try_into()?)?,
                    path,
                }
            }
            "id" => Self::Id(split_fmt_str(&required_string(data, "tag", "Filtre ID")?)?),
            "formule" => Self::Formula(value_or_formula(required_value(data, "formule", "FORMULE")?.try_into()?)?),
            _ => bail!("Type de filtre de tag non supporté: {kind}"),
        })
    }
    /// Builds a tag filter from formula-produced data, including dotted nested paths.
    ///
    /// String shorthand selects an ID/path and arrays mean logical AND.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed runtime values or unknown filter variants.
    pub fn from_dict(data: &EvalValue) -> Result<Self> {
        if let EvalValue::Array(arr) = data {
            return Ok(Self::And(
                arr.iter()
                    .map(|item| Self::from_dict(item))
                    .collect::<Result<Arc<_>>>()?,
            ));
        }
        if let EvalValue::String(_id) = data {
            return Ok(Self::Id(data.clone()));
        }
        if let EvalValue::Func(_f) = data {
            return Ok(Self::Formula(data.clone()));
        }
        let data = match data {
            EvalValue::Object(obj) => obj,
            _ => bail!("Un filtre de tag doit être un tableau ou un objet"),
        };
        let kind = match data.get("type") {
            Some(EvalValue::String(s)) => s.clone(),
            _ => bail!("Un filtre de tag doit inclure un champ `type`"),
        };
        Ok(match kind.as_ref() {
            "et" => {
                match data.get("filtres") {
                    Some(EvalValue::Array(arr)) => {
                        Self::And(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::And(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::And(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `filtres` d'un filtre de tag ET doit être un tableau"),
                }
            },
            "ou" => {
                match data.get("filtres") {
                    Some(EvalValue::Array(arr)) => {
                        Self::Or(arr.iter().map(|item| Self::from_dict(item)).collect::<Result<Arc<_>>>()?)
                    },
                    None => Self::Or(Vec::new().into()),
                    Some(EvalValue::Object(obj)) => {
                        Self::Or(vec![Self::from_dict(&EvalValue::Object(obj.clone()))?].into())
                    },
                    _ => bail!("Le champ `filtres` d'un filtre de tag OU doit être un tableau"),
                }
            },
            "non" => {
                if let Some(filter) = data.get("filtre") {
                    Self::Not(Arc::new(Self::from_dict(filter)?))
                } else {
                    bail!("Un filtre de tag NON doit inclure un sous-filtre")
                }
            },
            "valeur_egale" | "egale" => {
                let path = match data.get("chemin") {
                    Some(EvalValue::String(s)) => Some(EvalValue::String(s.clone())),
                    Some(EvalValue::Func(f)) => Some(EvalValue::Func(f.clone())),
                    _ => None,
                };
                Self::ValueEq{
                    value: data.get("valeur").cloned().ok_or_else(|| anyhow!("Le filtre de tag VALEUR_EGALE doit inclure un champ `valeur`"))?,
                    path,
                }
            }
            "valeur_superieure" | "superieure" => {
                let path = match data.get("chemin") {
                    Some(EvalValue::String(s)) => Some(EvalValue::String(s.clone())),
                    Some(EvalValue::Func(f)) => Some(EvalValue::Func(f.clone())),
                    _ => None,
                };
                Self::ValueGreater{
                    value: data.get("valeur").cloned().ok_or_else(|| anyhow!("Le filtre de tag VALEUR_SUPERIEURE doit inclure un champ `valeur`"))?,
                    path,
                }
            }
            "valeur_inferieure" | "inferieure" => {
                let path = match data.get("chemin") {
                    Some(EvalValue::String(s)) => Some(EvalValue::String(s.clone())),
                    Some(EvalValue::Func(f)) => Some(EvalValue::Func(f.clone())),
                    _ => None,
                };
                Self::ValueLess{
                    value: data.get("valeur").cloned().ok_or_else(|| anyhow!("Le filtre de tag VALEUR_INFERIEURE doit inclure un champ `valeur`"))?,
                    path,
                }
            }
            "id" => {
                let id = match data.get("tag") {
                    Some(EvalValue::String(s)) => EvalValue::String(s.clone()),
                    Some(EvalValue::Func(f)) => EvalValue::Func(f.clone()),
                    _ => bail!("Filtre ID doit inclure un champ `tag`"),
                };
                Self::Id(id)
            }
            "formule" => {
                let formula = match data.get("formule") {
                    Some(value) => value.clone(),
                    None => bail!("FORMULE doit inclure un champ `formule`"),
                };
                Self::Formula(formula)
            }
            _ => bail!("Type de filtre de tag non supporté: {kind}"),
        })
    }

    /// Evaluates this predicate against the supplied runtime value.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `value` — Formula-facing item, state, tag, or check value tested by this filter.
    /// - `character` — Current mutable or read-only character state.
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    /// - `env` — Additional formula bindings for the current lifecycle operation.
    ///
    /// # Errors
    ///
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches(
        &self,
        tag: &str,
        value: &EvalValue,
        character: &Character,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<bool> {
        let charenv = character.env(formula, env)?;
        let env = charenv.merged(env);
        self.matches_with_env(tag, value, formula, &env)
    }

    /// Matches a tag using a prebuilt character environment.
    ///
    /// Formula predicates use the supplied runtime bindings. Evaluation errors propagate
    /// instead of being converted to a false result.
    ///
    /// # Arguments
    ///
    /// - `value` — Formula-facing item, state, tag, or check value tested by this filter.
    /// - `formula` — Formula engine used for evaluation and deterministic randomness.
    ///
    /// # Errors
    ///
    /// Propagates formula-evaluation and nested-filter errors.
    pub fn matches_with_env(
        &self,
        tag: &str,
        value: &EvalValue,
        formula: &mut FormulaEngine,
        base_env: &Env,
    ) -> Result<bool> {
        match self {
            Self::And(filters) => {
                for filter in filters.iter() {
                    if !filter.matches_with_env(tag, value, formula, base_env)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(filters) => {
                for filter in filters.iter() {
                    if filter.matches_with_env(tag, value, formula, base_env)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Not(filter) => Ok(!filter.matches_with_env(tag, value, formula, base_env)?),
            Self::ValueEq{value: expected, path} => {
                let env = tag_formula_env(tag, value).with_parent(base_env);
                let expected = formula.resolve_formula(expected, &env)?;
                if let Some(path) = path {
                    let path = formula.fmt_value(path, &env)?;
                    let mut parts = path.split(".").collect::<Vec<_>>();
                    parts.insert(0, tag);
                    match follow_path(&parts, tag, value) {
                        Ok(actual) => Ok(actual == &expected),
                        Err(_) => Ok(false),
                    }
                } else {
                    Ok(value == &expected)
                }
            }
            Self::ValueGreater{value: expected, path} => {
                let env = tag_formula_env(tag, value).with_parent(base_env);
                let expected = formula.resolve_formula_float(expected, &env)?;
                if let Some(path) = path {
                    let path = formula.fmt_value(path, &env)?;
                    let mut parts = path.split(".").collect::<Vec<_>>();
                    parts.insert(0, tag);
                    match follow_path(&parts, tag, value) {
                        Ok(actual) => Ok(actual > &expected),
                        Err(_) => Ok(false),
                    }
                } else {
                    Ok(value > &expected)
                }
            }
            Self::ValueLess{value: expected, path} => {
                let env = tag_formula_env(tag, value).with_parent(base_env);
                let expected = formula.resolve_formula_float(expected, &env)?;
                if let Some(path) = path {
                    let path = formula.fmt_value(path, &env)?;
                    let mut parts = path.split(".").collect::<Vec<_>>();
                    parts.insert(0, tag);
                    match follow_path(&parts, tag, value) {
                        Ok(actual) => Ok(actual < &expected),
                        Err(_) => Ok(false),
                    }
                } else {
                    Ok(value < &expected)
                }
            }
            Self::Id(expected) => {
                let target = formula.fmt_value(expected, &tag_formula_env(tag, value).with_parent(base_env))?;
                let path = target.split(".").collect::<Vec<_>>();
                Ok(follow_path(&path, tag, value).is_ok())
            },
            Self::Formula(expression) => {
                let env = tag_formula_env(tag, value).with_parent(base_env);
                formula.resolve_formula_bool(expression, &env)
            }
        }
    }
}

fn follow_path<'a>(target: &[&str], current: &str, value: &'a EvalValue) -> Result<&'a EvalValue> {
    if target.is_empty() {
        bail!("Le chemin de tag est vide");
    }
    if target.len() == 1 && target[0] == current {
        Ok(value)
    } else if target[0] == current {
        if let EvalValue::Object(map) = value {
            let next = target[1];
            if let Some(next_value) = map.get(next) {
                follow_path(&target[1..], next, next_value).map_err(|_err| anyhow!("Le tag {} n'existe pas", target.join(".")))
            } else {
                bail!("Le tag {current} n'inclut pas le chemin {}", target.join("."))
            }
        } else if let EvalValue::Array(arr) = value {
            if let Ok(index) = target[1].parse::<usize>() {
                if let Some(next_value) = arr.get(index) {
                    follow_path(&target[1..], target[1], next_value).map_err(|_err| anyhow!("Le tag {} n'existe pas", target.join(".")))
                } else {
                    bail!("Le tag {current} n'inclut pas le chemin {}", target.join("."))
                }
            } else {
                bail!("Le tag {current} n'inclut pas le chemin {}", target.join("."))
            }
        } else {
            bail!("Le tag {current} n'est pas un objet, impossible de suivre le chemin {}", target.join("."))
        }
    } else {
        bail!("Le tag {current} n'inclut pas le chemin {}", target.join("."))
    }
}

/// Builds the formula environment used to evaluate one story tag.
///
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
///
/// # Arguments
///
/// - `character` — Current mutable or read-only character state.
/// - `value` — Story-tag value exposed at the top level and through `tag.value`.
pub fn tag_env<'a>(character: &Character, tag: &str, value: &EvalValue, formula: &mut FormulaEngine, env: &'a Env<'a>) -> Result<Env<'a>> {
    let charenv = character.env(formula, env)?;
    Ok(charenv.expand(tag_formula_env(tag, value)).with_parent(env))
}

/// Adds the selected tag bindings after all compatibility bindings.
///
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
fn tag_formula_env<'a>(tag: &str, value: &EvalValue) -> Env<'static> {
    let mut env = Env::new();
    env.insert("tag", EvalValue::String(tag.into()));
    env.insert("valeur", value.clone());
    env
}

/// Parses filter array according to formula precedence rules.
fn parse_filter_array<T>(
    value: Option<&JsonValue>,
    parser: impl Fn(&JsonValue) -> Result<T>,
) -> Result<Vec<T>> {
    match value {
        None => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => values.iter().map(parser).collect(),
        Some(_) => bail!("Le champ `filtres` doit être un tableau"),
    }
}

/// Parses traits according to formula precedence rules.
///
/// Required fields and variant-specific JSON types are checked immediately. Formula
/// strings are preserved for runtime evaluation.
fn parse_traits(value: Option<&JsonValue>, allow_single: bool) -> Result<Arc<[Arc<str>]>> {
    match value {
        None => Ok(Vec::new().into()),
        Some(value) => string_list(value, "Le champ `traits`", allow_single).map(|list| list.into_iter().map(Arc::from).collect::<Vec<_>>().into()),
    }
}

/// Parses string or list according to formula precedence rules.
///
/// Required fields and variant-specific JSON types are checked immediately. Formula
/// strings are preserved for runtime evaluation.
fn parse_string_or_list(value: Option<&JsonValue>, context: &str) -> Result<Arc<[Arc<str>]>> {
    let value = value.ok_or_else(|| anyhow!("{context} est obligatoire"))?;
    string_list(value, context, true).map(|list| list.into_iter().map(Arc::from).collect::<Vec<_>>().into())
}
