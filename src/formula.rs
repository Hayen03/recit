//! Tokenizer, parser, runtime values, and evaluator for the tale formula language.
//!
//! The recursive-descent parser implements explicit precedence levels. Evaluation is
//! dynamically typed through [`EvalValue`] and uses the deterministic RNG owned by
//! [`FormulaEngine`]. Strings returned by formulas are not recursively evaluated unless
//! they are passed through a later explicit resolution step.
use crate::filters::{ConditionFilter, ItemFilter, TagFilter};
use crate::formula::builtins::GLOBAL_ENV;
use anyhow::{anyhow, bail, Context, Result};
use indexmap::{IndexMap, IndexSet};
use lazy_static::lazy_static;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use serde::de::{Error as DeError};

lazy_static! {
    pub static ref INT: Arc<str> = Arc::from("int");
    pub static ref FLOAT: Arc<str> = Arc::from("float");
    pub static ref BOOL: Arc<str> = Arc::from("bool");
    pub static ref STR: Arc<str> = Arc::from("str");
    pub static ref LIST: Arc<str> = Arc::from("list");
    pub static ref DICT: Arc<str> = Arc::from("dict");
    pub static ref NULL: Arc<str> = Arc::from("null");
    pub static ref CHARACTER: Arc<str> = Arc::from("character");
    pub static ref FUNCTION: Arc<str> = Arc::from("func");
}

/// Represents a value understood by the formula language at runtime.
#[derive(Clone, Debug)]
pub enum EvalValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Array(Arc<[EvalValue]>),
    Object(Arc<IndexMap<Arc<str>, EvalValue>>),
    Character(CharacterView),
    Func(Arc<Function>),
}
impl PartialEq for EvalValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (EvalValue::Null, EvalValue::Null) => true,
            (EvalValue::Bool(a), EvalValue::Bool(b)) => a == b,
            (EvalValue::Int(a), EvalValue::Int(b)) => a == b,
            (EvalValue::Float(a), EvalValue::Float(b)) => a == b,
            (EvalValue::Int(a), EvalValue::Float(b)) => (*a as f64) == *b,
            (EvalValue::Int(a), EvalValue::Bool(true)) => *a == 1,
            (EvalValue::Int(a), EvalValue::Bool(false)) => *a == 0,
            (EvalValue::Bool(true), EvalValue::Int(b)) => *b == 1,
            (EvalValue::Bool(false), EvalValue::Int(b)) => *b == 0,
            (EvalValue::Float(a), EvalValue::Bool(true)) => *a == 1.0,
            (EvalValue::Float(a), EvalValue::Bool(false)) => *a == 0.0,
            (EvalValue::Bool(true), EvalValue::Float(b)) => *b == 1.0,
            (EvalValue::Bool(false), EvalValue::Float(b)) => *b == 0.0,
            (EvalValue::Float(a), EvalValue::Int(b)) => *a == (*b as f64),
            (EvalValue::String(a), EvalValue::String(b)) => a == b,
            (EvalValue::Array(a), EvalValue::Array(b)) => a == b,
            (EvalValue::Object(a), EvalValue::Object(b)) => a == b,
            (EvalValue::Character(a), EvalValue::Character(b)) => a == b,
            (EvalValue::Func(a), EvalValue::Func(b)) => a == b,
            _ => false,
        }
    }
}
impl PartialOrd for EvalValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (EvalValue::Int(a), EvalValue::Int(b)) => a.partial_cmp(b),
            (EvalValue::Float(a), EvalValue::Float(b)) => a.partial_cmp(b),
            (EvalValue::Int(a), EvalValue::Float(b)) => (*a as f64).partial_cmp(b),
            (EvalValue::Float(a), EvalValue::Int(b)) => a.partial_cmp(&(*b as f64)),
            (EvalValue::String(a), EvalValue::String(b)) => a.partial_cmp(b),
            (EvalValue::Bool(a), EvalValue::Bool(b)) => a.partial_cmp(b),
            (EvalValue::Null, EvalValue::Null) => Some(Ordering::Equal),
            (EvalValue::Array(a), EvalValue::Array(b)) => a.partial_cmp(b),
            (EvalValue::Int(a), EvalValue::Bool(true)) => (*a).partial_cmp(&1),
            (EvalValue::Int(a), EvalValue::Bool(false)) => (*a).partial_cmp(&0),
            (EvalValue::Bool(true), EvalValue::Int(b)) => (1).partial_cmp(b),
            (EvalValue::Bool(false), EvalValue::Int(b)) => (0).partial_cmp(b),
            (EvalValue::Float(a), EvalValue::Bool(true)) => (*a).partial_cmp(&1.0),
            (EvalValue::Float(a), EvalValue::Bool(false)) => (*a).partial_cmp(&0.0),
            (EvalValue::Bool(true), EvalValue::Float(b)) => (1.0).partial_cmp(b),
            (EvalValue::Bool(false), EvalValue::Float(b)) => (0.0).partial_cmp(b),
            _ => None,
        }
    }
}
impl PartialOrd<i64> for EvalValue {
    fn partial_cmp(&self, other: &i64) -> Option<Ordering> {
        match self {
            EvalValue::Int(a) => a.partial_cmp(other),
            EvalValue::Float(a) => a.partial_cmp(&(*other as f64)),
            EvalValue::Bool(true) => (1).partial_cmp(other),
            EvalValue::Bool(false) => (0).partial_cmp(other),
            _ => None,
        }
    }
}
impl PartialOrd<EvalValue> for i64 {
    fn partial_cmp(&self, other: &EvalValue) -> Option<Ordering> {
        return other.partial_cmp(self).map(|o| o.reverse());
    }
}
impl PartialEq<EvalValue> for i64 {
    fn eq(&self, other: &EvalValue) -> bool {
        other == self
    }
}
impl PartialEq<i64> for EvalValue {
    fn eq(&self, other: &i64) -> bool {
        match self {
            EvalValue::Int(a) => a == other,
            EvalValue::Float(a) => *a == (*other as f64),
            EvalValue::Bool(true) => *other == 1,
            EvalValue::Bool(false) => *other == 0,
            _ => false,
        }
    }
}
impl PartialOrd<f64> for EvalValue {
    fn partial_cmp(&self, other: &f64) -> Option<Ordering> {
        match self {
            EvalValue::Int(a) => (*a as f64).partial_cmp(other),
            EvalValue::Float(a) => a.partial_cmp(other),
            EvalValue::Bool(true) => (1.0).partial_cmp(other),
            EvalValue::Bool(false) => (0.0).partial_cmp(other),
            _ => None,
        }
    }
}
impl PartialEq<f64> for EvalValue {
    fn eq(&self, other: &f64) -> bool {
        match self {
            EvalValue::Int(a) => (*a as f64) == *other,
            EvalValue::Float(a) => *a == *other,
            EvalValue::Bool(true) => *other == 1.0,
            EvalValue::Bool(false) => *other == 0.0,
            _ => false,
        }
    }
}
impl PartialOrd<EvalValue> for f64 {
    fn partial_cmp(&self, other: &EvalValue) -> Option<Ordering> {
        return other.partial_cmp(self).map(|o| o.reverse());
    }
}
impl PartialEq<EvalValue> for f64 {
    fn eq(&self, other: &EvalValue) -> bool {
        other == self
    }
}
impl PartialOrd<bool> for EvalValue {
    fn partial_cmp(&self, other: &bool) -> Option<Ordering> {
        match self {
            EvalValue::Bool(a) => a.partial_cmp(other),
            EvalValue::Int(a) => (*a != 0).partial_cmp(other),
            EvalValue::Float(a) => (*a != 0.0).partial_cmp(other),
            _ => None,
        }
    }
}
impl PartialEq<bool> for EvalValue {
    fn eq(&self, other: &bool) -> bool {
        match self {
            EvalValue::Bool(a) => a == other,
            EvalValue::Int(a) => (*a != 0) == *other,
            EvalValue::Float(a) => (*a != 0.0) == *other,
            _ => false,
        }
    }
}
impl PartialOrd<EvalValue> for bool {
    fn partial_cmp(&self, other: &EvalValue) -> Option<Ordering> {
        return other.partial_cmp(self).map(|o| o.reverse());
    }
}
impl PartialEq<EvalValue> for bool {
    fn eq(&self, other: &EvalValue) -> bool {
        other == self
    }
}
impl PartialOrd<str> for EvalValue {
    fn partial_cmp(&self, other: &str) -> Option<Ordering> {
        match self {
            EvalValue::String(a) => (**a).partial_cmp(other),
            _ => None,
        }
    }
}
impl PartialEq<str> for EvalValue {
    fn eq(&self, other: &str) -> bool {
        match self {
            EvalValue::String(a) => **a == *other,
            _ => false,
        }
    }
}
impl PartialOrd<EvalValue> for str {
    fn partial_cmp(&self, other: &EvalValue) -> Option<Ordering> {
        return other.partial_cmp(self).map(|o| o.reverse());
    }
}
impl PartialEq<EvalValue> for str {
    fn eq(&self, other: &EvalValue) -> bool {
        other == self
    }
}
impl std::fmt::Display for EvalValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalValue::Null => write!(f, "null"),
            EvalValue::Bool(value) => write!(f, "{}", value),
            EvalValue::Int(value) => write!(f, "{}", value),
            EvalValue::Float(value) => write!(f, "{}", value),
            EvalValue::String(value) => write!(f, "\"{}\"", value),
            EvalValue::Array(values) => {
                let strings: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", strings.join(", "))
            }
            EvalValue::Object(values) => {
                let strings: Vec<String> = values
                    .iter()
                    .map(|(k, v)| format!("\"{}\": {}", k, v))
                    .collect();
                write!(f, "{{{}}}", strings.join(", "))
            }
            EvalValue::Character(character) => {
                write!(
                    f,
                    "Personnage(nom={}, pv={}/{}, moral={}/{}, attributs={:?}, tags={:?})",
                    character.name.as_deref().unwrap_or("None"),
                    character.pv,
                    character.max_pv,
                    character.moral,
                    character.max_moral,
                    character.attributes,
                    character.tags
                )
            }
            EvalValue::Func(func) => write!(f, "{func}"),
        }
    }
}

/// Formats positional arguments for a call-site error without losing their values or types.
fn diagnostic_arguments(arguments: &[EvalValue]) -> String {
    if arguments.is_empty() {
        return "aucun argument".to_owned();
    }
    format!(
        "arguments [{}]",
        arguments
            .iter()
            .enumerate()
            .map(|(index, value)| format!("{}: {}", index + 1, value.diagnostic()))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Produces a compact source-like representation of an AST node for call-stack context.
fn diagnostic_expression(expression: &Expr) -> String {
    let rendered = expression.to_string().replace('\n', "\\n");
    let mut characters = rendered.chars();
    let shortened = characters.by_ref().take(240).collect::<String>();
    if characters.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}
impl Eq for EvalValue {}
impl<'de> Deserialize<'de> for EvalValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let deser = EvalValueSerializer::deserialize(deserializer)?;
        Self::try_from(deser).map_err(|err| <D as serde::Deserializer<'de>>::Error::custom(err))
    }
}
impl Serialize for EvalValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        EvalValueSerializer::from(self).serialize(serializer)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvalValueSerializer {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Box<[EvalValueSerializer]>),
    Object(Box<IndexMap<String, EvalValueSerializer>>),
    Character(CharacterViewSerializer),
    Func(Box<FunctionSerializer>),
}
impl From<&EvalValue> for EvalValueSerializer {
    fn from(value: &EvalValue) -> Self {
        match value {
            EvalValue::Null => EvalValueSerializer::Null,
            EvalValue::Bool(b) => EvalValueSerializer::Bool(*b),
            EvalValue::Int(i) => EvalValueSerializer::Int(*i),
            EvalValue::Float(f) => EvalValueSerializer::Float(*f),
            EvalValue::String(s) => EvalValueSerializer::String(s.to_string()),
            EvalValue::Array(arr) => {
                let serialized_arr: Box<[EvalValueSerializer]> =
                    arr.iter().map(EvalValueSerializer::from).collect::<Box<[EvalValueSerializer]>>();
                EvalValueSerializer::Array(serialized_arr)
            }
            EvalValue::Object(obj) => {
                let serialized_obj: Box<IndexMap<String, EvalValueSerializer>> =
                    obj.iter()
                        .map(|(k, v)| (k.to_string(), EvalValueSerializer::from(v)))
                        .collect::<IndexMap<String, EvalValueSerializer>>().into();
                EvalValueSerializer::Object(serialized_obj)
            }
            EvalValue::Character(character) => {
                EvalValueSerializer::Character(CharacterViewSerializer::from(character))
            }
            EvalValue::Func(func) => EvalValueSerializer::Func(Box::new(FunctionSerializer::from(func.as_ref()))),
        }
    }
}
impl TryFrom<EvalValueSerializer> for EvalValue {
    type Error = anyhow::Error;
    fn try_from(value: EvalValueSerializer) -> Result<Self, anyhow::Error> {
        Ok(match value {
            EvalValueSerializer::Null => EvalValue::Null,
            EvalValueSerializer::Bool(b) => EvalValue::Bool(b),
            EvalValueSerializer::Int(i) => EvalValue::Int(i),
            EvalValueSerializer::Float(f) => EvalValue::Float(f),
            EvalValueSerializer::String(s) => EvalValue::String(Arc::from(s)),
            EvalValueSerializer::Array(arr) => {
                let deserialized_arr: Arc<[EvalValue]> =
                    arr.into_vec().into_iter().map(|value| EvalValue::try_from(value)).collect::<Result<Vec<EvalValue>, anyhow::Error>>()?.into();
                EvalValue::Array(deserialized_arr)
            }
            EvalValueSerializer::Object(obj) => {
                let deserialized_obj: Arc<IndexMap<Arc<str>, EvalValue>> =
                    obj.into_iter()
                        .map(|(k, v)| Ok((Arc::from(k.as_str()), EvalValue::try_from(v)?)))
                        .collect::<Result<IndexMap<Arc<str>, EvalValue>, anyhow::Error>>()?.into();
                EvalValue::Object(deserialized_obj)
            }
            EvalValueSerializer::Character(character) => {
                EvalValue::Character(CharacterView::try_from(character)?)
            }
            EvalValueSerializer::Func(func) => {
                let func = Function::try_from(*func)?;
                EvalValue::Func(Arc::new(func))
            }
        })
    }
}

/// Classifies formula evaluation and conversion failures.
#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    InvalidNumber,
    NonSerializableValue,
}
impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::InvalidNumber => write!(f, "Nombre invalide"),
            EvalError::NonSerializableValue => write!(f, "Valeur non sérialisable"),
        }
    }
}
impl std::error::Error for EvalError {}

/// Read-only formula-facing snapshot of relevant character data.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterView {
    pub name: Option<Arc<str>>,
    pub pv: i64,
    pub max_pv: i64,
    pub moral: i64,
    pub max_moral: i64,
    pub attributes: Arc<IndexMap<Arc<str>, EvalValue>>,
    pub tags: Arc<IndexMap<Arc<str>, EvalValue>>,
    /// Inventory snapshots available to `personnage.selectionne`.
    pub items: Arc<[FormulaItemView]>,
    /// Persistent-state snapshots available to `personnage.selectionne`.
    pub conditions: Arc<[FormulaConditionView]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterViewSerializer {
    pub name: Option<String>,
    pub pv: i64,
    pub max_pv: i64,
    pub moral: i64,
    pub max_moral: i64,
    pub attributes: Box<IndexMap<String, EvalValueSerializer>>,
    pub tags: Box<IndexMap<String, EvalValueSerializer>>,
    /// Inventory snapshots available to `personnage.selectionne`.
    pub items: Box<[FormulaItemViewSerializer]>,
    /// Persistent-state snapshots available to `personnage.selectionne`.
    pub conditions: Box<[FormulaConditionViewSerializer]>,
}
impl From<&CharacterView> for CharacterViewSerializer {
    fn from(character: &CharacterView) -> Self {
        let attributes: Box<IndexMap<String, EvalValueSerializer>> =
            character.attributes.iter()
                .map(|(k, v)| (k.to_string(), EvalValueSerializer::from(v)))
                .collect::<IndexMap<String, EvalValueSerializer>>().into();
        let tags: Box<IndexMap<String, EvalValueSerializer>> =
            character.tags.iter()
                .map(|(k, v)| (k.to_string(), EvalValueSerializer::from(v)))
                .collect::<IndexMap<String, EvalValueSerializer>>().into();
        let items: Box<[FormulaItemViewSerializer]> =
            character.items.iter()
                .map(FormulaItemViewSerializer::from)
                .collect::<Vec<FormulaItemViewSerializer>>().into_boxed_slice();
        let conditions: Box<[FormulaConditionViewSerializer]> =
            character.conditions.iter()
                .map(FormulaConditionViewSerializer::from)
                .collect::<Vec<FormulaConditionViewSerializer>>().into_boxed_slice();
        CharacterViewSerializer {
            name: character.name.as_ref().map(|s| s.to_string()),
            pv: character.pv,
            max_pv: character.max_pv,
            moral: character.moral,
            max_moral: character.max_moral,
            attributes,
            tags,
            items,
            conditions,
        }
    }
}
impl TryFrom<CharacterViewSerializer> for CharacterView {
    type Error = anyhow::Error;
    fn try_from(serializer: CharacterViewSerializer) -> Result<Self, Self::Error> {
        let attributes: Arc<IndexMap<Arc<str>, EvalValue>> =
            serializer.attributes.into_iter()
                .map(|(k, v)| Ok((Arc::from(k.as_str()), EvalValue::try_from(v)?)))
                .collect::<Result<IndexMap<Arc<str>, EvalValue>, Self::Error>>()?.into();
        let tags: Arc<IndexMap<Arc<str>, EvalValue>> =
            serializer.tags.into_iter()
                .map(|(k, v)| Ok((Arc::from(k.as_str()), EvalValue::try_from(v)?)))
                .collect::<Result<IndexMap<Arc<str>, EvalValue>, Self::Error>>()?.into();
        let items: Arc<[FormulaItemView]> =
            serializer.items.into_vec().into_iter()
                .map(|item| FormulaItemView::try_from(item))
                .collect::<Result<Vec<FormulaItemView>, _>>()?.into();
        let conditions: Arc<[FormulaConditionView]> =
            serializer.conditions.into_vec().into_iter()
                .map(|condition| FormulaConditionView::try_from(condition))
                .collect::<Result<Vec<FormulaConditionView>, _>>()?.into();
        Ok(CharacterView {
            name: serializer.name.map(|s| Arc::from(s.as_str())),
            pv: serializer.pv,
            max_pv: serializer.max_pv,
            moral: serializer.moral,
            max_moral: serializer.max_moral,
            attributes,
            tags,
            items,
            conditions,
        })
    }
}

impl CharacterView {
    /// Returns one named property exposed to formulas.
    ///
    ///
    /// Returns `None` when the value has no such property or the property is not supported.
    pub fn get_property(
        &self,
        property: &str,
        formula: &mut FormulaEngine,
        base_env: &Env,
    ) -> Option<EvalValue> {
        match property {
            "nom" => Some(
                self.name
                    .clone()
                    .map(|s| EvalValue::String(s))
                    .unwrap_or(EvalValue::Null),
            ),
            "pv" => Some(EvalValue::Int(self.pv)),
            "max_pv" => Some(EvalValue::Int(self.max_pv)),
            "moral" => Some(EvalValue::Int(self.moral)),
            "max_moral" => Some(EvalValue::Int(self.max_moral)),
            "attributs" => Some(EvalValue::Object(self.attributes.clone())),
            "objets" => {
                let mut resolved_items = Vec::new();
                for item in self.items.iter() {
                    let ienv0 = self.formula_env().expand(item.env());
                    let ienv = base_env.merged(&ienv0);
                    resolved_items.push(
                        item.resolved_object(formula, &ienv)
                            .expect(&format!("Failed to resolve item {}", item.id)),
                    );
                }
                Some(EvalValue::Array(resolved_items.into()))
            }
            "etats" => {
                let mut resolved_conditions = Vec::new();
                for condition in self.conditions.iter() {
                    let eenv0 = self.formula_env().expand(condition.env());
                    let eenv = base_env.merged(&eenv0);
                    resolved_conditions.push(
                        condition
                            .resolved_object(formula, &eenv)
                            .expect(&format!("Failed to resolve condition {}", condition.id)),
                    );
                }
                Some(EvalValue::Array(resolved_conditions.into()))
            }
            "tags" => Some(EvalValue::Object(self.tags.clone())),
            _ => None,
        }
    }

    /// Builds the environment used while selecting this character's entities.
    ///
    /// The returned environment owns cloned bindings, so callers may extend it without
    /// mutating the source character, page, item, or state.
    fn formula_env(&self) -> Env<'static> {
        let mut env = Env::new();
        for (attribute, value) in self.attributes.iter() {
            env.insert(attribute.clone(), value.clone());
        }
        for (tag, value) in self.tags.iter() {
            env.insert(tag.clone(), value.clone());
        }
        env.insert("personnage", EvalValue::Character(self.clone()));
        env.insert(
            "nom",
            self.name
                .clone()
                .map(EvalValue::String)
                .unwrap_or(EvalValue::Null),
        );
        env.insert("max_moral", EvalValue::Int(self.max_moral));
        env.insert("max_pv", EvalValue::Int(self.max_pv));
        env.insert("moral", EvalValue::Int(self.moral));
        env.insert("pv", EvalValue::Int(self.pv));
        env
    }
}

/// Read-only formula-facing snapshot of one inventory stack.
#[derive(Clone, Debug, PartialEq)]
pub struct FormulaItemView {
    pub id: Arc<str>,
    pub name: EvalValue,
    pub description: EvalValue,
    pub quantity: i64,
    pub traits: Arc<[Arc<str>]>,
    pub data: Arc<IndexMap<Arc<str>, EvalValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormulaItemViewSerializer {
    pub id: String,
    pub name: EvalValueSerializer,
    pub description: EvalValueSerializer,
    pub quantity: i64,
    pub traits: Box<[String]>,
    pub data: Box<IndexMap<String, EvalValueSerializer>>,
}
impl From<&FormulaItemView> for FormulaItemViewSerializer {
    fn from(item: &FormulaItemView) -> Self {
        let traits: Box<[String]> = item.traits.iter().map(|s| s.to_string()).collect::<Vec<String>>().into();
        let data: Box<IndexMap<String, EvalValueSerializer>> = item.data.iter()
            .map(|(k, v)| (k.to_string(), EvalValueSerializer::from(v)))
            .collect::<IndexMap<String, EvalValueSerializer>>().into();
        FormulaItemViewSerializer {
            id: item.id.to_string(),
            name: EvalValueSerializer::from(&item.name),
            description: EvalValueSerializer::from(&item.description),
            quantity: item.quantity,
            traits,
            data,
        }
    }
}
impl TryFrom<FormulaItemViewSerializer> for FormulaItemView {
    type Error = anyhow::Error;
    fn try_from(serializer: FormulaItemViewSerializer) -> Result<Self, Self::Error> {
        let traits: Arc<[Arc<str>]> = serializer.traits.into_vec().into_iter().map(|s| Arc::from(s)).collect::<Vec<Arc<str>>>().into();
        let data: Arc<IndexMap<Arc<str>, EvalValue>> = serializer.data.into_iter()
            .map(|(k, v)| Ok((Arc::from(k.as_str()), EvalValue::try_from(v)?)))
            .collect::<Result<IndexMap<Arc<str>, EvalValue>, anyhow::Error>>()?.into();
        Ok(FormulaItemView {
            id: Arc::from(serializer.id.as_str()),
            name: EvalValue::try_from(serializer.name)?,
            description: EvalValue::try_from(serializer.description)?,
            quantity: serializer.quantity,
            traits,
            data,
        })
    }
}

impl FormulaItemView {
    /// Builds the flat environment used by item formulas and item filters.
    ///
    /// The returned environment owns cloned bindings, so callers may extend it without
    /// mutating the source character, page, item, or state.
    pub fn env(&self) -> Env<'static> {
        let id = EvalValue::String(self.id.clone());
        let traits = EvalValue::Array(self.traits.iter().cloned().map(EvalValue::String).collect());
        let qt = EvalValue::Int(self.quantity);
        let data = EvalValue::Object(self.data.clone());
        let mut obj = IndexMap::new();
        obj.insert("id".into(), id.clone());
        obj.insert("traits".into(), traits.clone());
        obj.insert("quantite".into(), qt.clone());
        obj.insert("donnees".into(), data.clone());
        let mut env = Env::new();
        for (key, value) in self.data.iter() {
            env.insert(key.clone(), value.clone());
        }
        env.insert("objet", EvalValue::Object(Arc::from(obj)));
        env.insert("id", id);
        env.insert("traits", traits);
        env.insert("quantite", qt);
        env.insert("donnees", data);
        env
    }

    pub fn base_obj(&self) -> EvalValue {
        let mut obj = IndexMap::new();
        obj.insert("id".into(), EvalValue::String(self.id.clone()));
        obj.insert("nom".into(), self.name.clone());
        obj.insert(
            "description".into(),
            self.description.clone(),
        );
        obj.insert("quantite".into(), EvalValue::Int(self.quantity));
        obj.insert(
            "traits".into(),
            EvalValue::Array(self.traits.iter().cloned().map(EvalValue::String).collect()),
        );
        obj.insert("donnees".into(), EvalValue::Object(self.data.clone()));
        EvalValue::Object(Arc::from(obj))
    }

    /// Builds the fully resolved dictionary returned by `personnage.selectionne`.
    ///
    /// Evaluates deferred formulas exactly once against the supplied environment and returns
    /// the resolved value without recursively evaluating strings produced by that formula.
    fn resolved_object(&self, formula: &mut FormulaEngine, base_env: &Env) -> Result<EvalValue> {
        let senv = self.env();
        let item_env = base_env.merged(&senv);
        let mut vals = self.data.clone();
        let values = Arc::make_mut(&mut vals);
        values.insert("id".into(), EvalValue::String(self.id.clone()));
        values.insert(
            "nom".into(),
            EvalValue::String(formula.fmt_value(&self.name, &item_env)?),
        );
        values.insert(
            "description".into(),
            EvalValue::String(formula.fmt_value(&self.description, &item_env)?),
        );
        values.insert("quantite".into(), EvalValue::Int(self.quantity));
        values.insert(
            "traits".into(),
            EvalValue::Array(self.traits.iter().cloned().map(EvalValue::String).collect()),
        );
        values.insert("donnees".into(), EvalValue::Object(self.data.clone()));
        Ok(EvalValue::Object(vals))
    }
}

/// Read-only formula-facing snapshot of one persistent state.
#[derive(Clone, Debug, PartialEq)]
pub struct FormulaConditionView {
    pub id: Arc<str>,
    pub name: EvalValue,
    pub description: EvalValue,
    pub traits: Arc<[Arc<str>]>,
    pub data: Arc<IndexMap<Arc<str>, EvalValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormulaConditionViewSerializer {
    pub id: String,
    pub name: EvalValueSerializer,
    pub description: EvalValueSerializer,
    pub traits: Box<[String]>,
    pub data: Box<IndexMap<String, EvalValueSerializer>>,
}
impl From<&FormulaConditionView> for FormulaConditionViewSerializer {
    fn from(condition: &FormulaConditionView) -> Self {
        let traits: Box<[String]> = condition.traits.iter().map(|s| s.to_string()).collect::<Vec<String>>().into();
        let data: Box<IndexMap<String, EvalValueSerializer>> = condition.data.iter()
            .map(|(k, v)| (k.to_string(), EvalValueSerializer::from(v)))
            .collect::<IndexMap<String, EvalValueSerializer>>().into();
        FormulaConditionViewSerializer {
            id: condition.id.to_string(),
            name: EvalValueSerializer::from(&condition.name),
            description: EvalValueSerializer::from(&condition.description),
            traits,
            data,
        }
    }
}
impl TryFrom<FormulaConditionViewSerializer> for FormulaConditionView {
    type Error = anyhow::Error;
    fn try_from(serializer: FormulaConditionViewSerializer) -> Result<Self, Self::Error> {
        let traits: Arc<[Arc<str>]> = serializer.traits.into_vec().into_iter().map(|s| Arc::from(s)).collect::<Vec<Arc<str>>>().into();
        let data: Arc<IndexMap<Arc<str>, EvalValue>> = serializer.data.into_iter()
            .map(|(k, v)| Ok((Arc::from(k.as_str()), EvalValue::try_from(v)?)))
            .collect::<Result<IndexMap<Arc<str>, EvalValue>>>()?.into();
        Ok(FormulaConditionView {
            id: Arc::from(serializer.id.as_str()),
            name: EvalValue::try_from(serializer.name)?,
            description: EvalValue::try_from(serializer.description)?,
            traits,
            data,
        })
    }
}

impl FormulaConditionView {
    /// Builds the flat environment used by state formulas and state filters.
    ///
    /// The returned environment owns cloned bindings, so callers may extend it without
    /// mutating the source character, page, item, or state.
    pub fn env(&self) -> Env<'static> {
        let id = EvalValue::String(self.id.clone());
        let traits = EvalValue::Array(self.traits.iter().cloned().map(EvalValue::String).collect());
        let data = EvalValue::Object(self.data.clone());
        let mut obj = IndexMap::new();
        obj.insert("id".into(), id.clone());
        obj.insert("traits".into(), traits.clone());
        obj.insert("donnees".into(), data.clone());
        let mut env = Env::new();
        for (key, value) in self.data.iter() {
            env.insert(key.clone(), value.clone());
        }
        env.insert("etat", EvalValue::Object(Arc::from(obj)));
        env.insert("id", id);
        env.insert("traits", traits);
        env.insert("donnees", data);
        env
    }

    /// Builds the fully resolved dictionary returned by `personnage.selectionne`.
    ///
    /// Evaluates deferred formulas exactly once against the supplied environment and returns
    /// the resolved value without recursively evaluating strings produced by that formula.
    fn resolved_object(&self, formula: &mut FormulaEngine, base_env: &Env) -> Result<EvalValue> {
        let senv = self.env();
        let condition_env = base_env.merged(&senv);
        let mut vals = self.data.clone();
        let values = Arc::make_mut(&mut vals);
        values.insert("id".into(), EvalValue::String(self.id.clone()));
        values.insert(
            "nom".into(),
            EvalValue::String(formula.fmt_value(&self.name, &condition_env)?),
        );
        values.insert(
            "description".into(),
            EvalValue::String(formula.fmt_value(&self.description, &condition_env)?),
        );
        values.insert(
            "traits".into(),
            EvalValue::Array(self.traits.iter().cloned().map(EvalValue::String).collect()),
        );
        values.insert("donnees".into(), EvalValue::Object(self.data.clone()));
        Ok(EvalValue::Object(vals))
    }
}

impl EvalValue {
    /// Converts a JSON value into the corresponding formula runtime value.
    ///
    /// # Panics
    ///
    /// Panics only if `value` contains a JSON number that cannot be represented by [`EvalValue`]; ordinary parser-produced JSON values are representable.
    pub fn from_json(value: &JsonValue) -> Self {
        Self::try_from(value).unwrap()
    }
    /// Converts this formula value into save-compatible JSON.
    ///
    /// # Errors
    ///
    /// Returns an error for runtime-only values such as character views or invalid floating-point numbers.
    pub fn to_json(&self) -> Result<JsonValue> {
        self.try_into()
            .map_err(|e| anyhow!("Erreur lors de la conversion en JSON: {}", e))
    }

    /// Returns whether this value is considered true by formula semantics.
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0,
            Self::String(value) => !value.is_empty(),
            Self::Array(value) => !value.is_empty(),
            Self::Object(value) => !value.is_empty(),
            Self::Character(_) => true,
            Self::Func(_) => true,
        }
    }

    /// Describes a runtime value for an error message without flooding the terminal.
    ///
    /// The rendered value is escaped and capped at 180 characters, then followed by its
    /// formula-language type. This makes type errors identify the actual offending value.
    pub fn diagnostic(&self) -> String {
        let rendered = self
            .to_string()
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        let mut characters = rendered.chars();
        let shortened = characters.by_ref().take(180).collect::<String>();
        let rendered = if characters.next().is_some() {
            format!("{shortened}…")
        } else {
            shortened
        };
        format!("{rendered} (type {})", self.type_name())
    }
    /// Converts integers, floats, booleans, and numeric strings to `i64`.
    ///
    /// Floats are truncated using Rust's numeric cast semantics.
    ///
    /// # Errors
    ///
    /// Returns an error for non-numeric strings and unsupported value types.
    pub fn to_i64(&self) -> Result<i64> {
        match self {
            Self::Int(value) => Ok(*value),
            Self::Float(value) => Ok(*value as i64),
            Self::Bool(value) => Ok(if *value { 1 } else { 0 }),
            Self::String(value) => value.parse::<i64>().map_err(|_| {
                anyhow!(
                    "Impossible de convertir {} en entier: la chaîne n'est pas une représentation entière valide",
                    self.diagnostic()
                )
            }),
            _ => bail!(
                "Impossible de convertir {} en entier: seuls int, float, bool et une chaîne numérique sont acceptés",
                self.diagnostic()
            ),
        }
    }
    /// Converts integers, floats, booleans, and numeric strings to `f64`.
    ///
    /// # Errors
    ///
    /// Returns an error for non-numeric strings and unsupported value types.
    pub fn to_f64(&self) -> Result<f64> {
        match self {
            Self::Int(value) => Ok(*value as f64),
            Self::Float(value) => Ok(*value),
            Self::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
            Self::String(value) => value.parse::<f64>().map_err(|_| {
                anyhow!(
                    "Impossible de convertir {} en nombre: la chaîne n'est pas une représentation numérique valide",
                    self.diagnostic()
                )
            }),
            _ => bail!(
                "Impossible de convertir {} en nombre: seuls int, float, bool et une chaîne numérique sont acceptés",
                self.diagnostic()
            ),
        }
    }

    /// Formats the value using the string conventions of the formula language.
    pub fn python_string(&self) -> Arc<str> {
        match self {
            Self::Null => "None".into(),
            Self::Bool(true) => "True".into(),
            Self::Bool(false) => "False".into(),
            Self::Int(value) => value.to_string().into(),
            Self::Float(value) => {
                if value.is_nan() {
                    "nan".into()
                } else if value.is_infinite() {
                    if value.is_sign_negative() {
                        "-inf".into()
                    } else {
                        "inf".into()
                    }
                } else {
                    value.to_string().into()
                }
            }
            Self::String(value) => value.clone(),
            Self::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(|value| match value {
                        Self::String(text) => format!("'{text}'").into(),
                        _ => value.python_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into(),
            Self::Object(values) => format!(
                "{{{}}}",
                values
                    .iter()
                    .map(|(key, value)| format!("'{key}': {}", value.python_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into(),
            Self::Character(character) => format!(
                "Personnage(nom={})",
                character.name.as_deref().unwrap_or("None")
            )
            .into(),
            Self::Func(_) => "<function>".into(),
        }
    }

    /// Returns the formula-language name of this value type.
    pub fn type_name(&self) -> Arc<str> {
        match self {
            Self::Null => NULL.clone(),
            Self::Bool(_) => BOOL.clone(),
            Self::Int(_) => INT.clone(),
            Self::Float(_) => FLOAT.clone(),
            Self::String(_) => STR.clone(),
            Self::Array(_) => LIST.clone(),
            Self::Object(_) => DICT.clone(),
            Self::Character(_) => CHARACTER.clone(),
            Self::Func(_) => FUNCTION.clone(),
        }
    }

    /// Returns a direct data member without consulting the type's method registry.
    ///
    /// Object keys and character properties use string members. Lists and strings accept
    /// integer indexes (including negative indexes); the dotted numeric form is accepted for
    /// lists for compatibility with `liste.0`.
    fn direct_member(
        &self,
        member: &EvalValue,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Option<EvalValue>> {
        match (self, member) {
            (Self::Object(values), Self::String(key)) => Ok(values.get(key).cloned()),
            (Self::Object(values), Self::Int(key)) => {
                Ok(values.get(key.to_string().as_str()).cloned())
            }
            (Self::Character(character), Self::String(property)) => {
                Ok(character.get_property(property, formula, env))
            }
            (Self::Array(values), Self::Int(index)) => {
                let index = normalize_index(*index, values.len())?;
                Ok(values.get(index).cloned())
            }
            (Self::Array(values), Self::String(index)) => {
                let Ok(index) = index.parse::<i64>() else {
                    return Ok(None);
                };
                let index = normalize_index(index, values.len())?;
                Ok(values.get(index).cloned())
            }
            (Self::String(value), Self::Int(index)) => {
                let characters = value.chars().collect::<Vec<_>>();
                let index = normalize_index(*index, characters.len())?;
                Ok(characters
                    .get(index)
                    .map(|character| Self::String(character.to_string().into())))
            }
            _ => Ok(None),
        }
    }

    /// Returns the registry of methods exposed by this value type.
    fn method_registry(&self) -> Option<&'static IndexMap<Arc<str>, EvalValue>> {
        match self {
            Self::Array(_) => Some(&*builtins::ARRAY_METHODS),
            Self::Object(_) => Some(&*builtins::OBJECT_METHODS),
            Self::String(_) => Some(&*builtins::STRING_METHODS),
            Self::Int(_) => Some(&*builtins::INT_METHODS),
            Self::Float(_) => Some(&*builtins::FLOAT_METHODS),
            Self::Character(_) => Some(&*builtins::CHARACTER_METHODS),
            _ => None,
        }
    }

    /// Resolves one object property, character property, indexed item, or registered method.
    ///
    /// Direct data always takes precedence over a method with the same name. Only strings and
    /// integers may be used as members. Method values are returned as ordinary
    /// [`EvalValue::Func`] values; the evaluator supplies the receiver when a registered method
    /// is called with postfix syntax.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported member types, invalid indexes, or unknown members.
    pub fn get_member(
        &self,
        member: &EvalValue,
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if !matches!(member, Self::String(_) | Self::Int(_)) {
            bail!(
                "Impossible d'accéder au membre {} de {}: un membre doit être une chaîne ou un entier",
                member.diagnostic(),
                self.diagnostic()
            )
        }
        if let Some(value) = self.direct_member(member, formula, env)? {
            return Ok(value);
        }
        if let Self::String(name) = member {
            if let Some(method) = self
                .method_registry()
                .and_then(|registry| registry.get(name.as_ref()))
            {
                return Ok(method.clone());
            }
        }
        bail!(
            "Membre {} inconnu sur {}: aucune donnée ni méthode de ce nom n'existe",
            member.diagnostic(),
            self.diagnostic()
        )
    }

    /// Calls this value as a built-in or tale-defined function.
    ///
    /// Tale-defined functions accept extra positional arguments, expose the complete list as
    /// `args`, and bind the leading arguments to their declared parameter names. Their lexical
    /// captures are combined with the dynamic runtime portion of the caller's environment.
    ///
    /// # Errors
    ///
    /// Returns an error when this value is not callable, when too few arguments are supplied,
    /// or when the called function fails.
    pub fn call(
        self,
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        match self {
            Self::Func(function) => match function.as_ref() {
                Function::Builtin(name, function) => function(arguments, formula, env)
                    .with_context(|| {
                        format!(
                            "Échec de l'appel du builtin `{name}` avec {}",
                            diagnostic_arguments(arguments)
                        )
                    }),
                Function::User(function) => formula
                    .call_user_function(function.clone(), arguments, env)
                    .with_context(|| {
                        format!(
                            "Échec de l'appel d'une fonction avec {}",
                            diagnostic_arguments(arguments)
                        )
                    }),
            },
            value => bail!(
                "Impossible d'appeler {}: cette valeur n'est pas une fonction",
                value.diagnostic()
            ),
        }
    }

    /// Attempts to add using formula-language semantics.
    ///
    /// Implements formula-language coercion rules and rejects unsupported operand types or
    /// arithmetic domain errors instead of falling back to string conversion.
    ///
    /// # Errors
    ///
    /// Returns an error when the operand types do not support addition.
    pub fn try_add(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a + b)),
            (Self::Int(a), Self::Bool(b)) => Ok(Self::Bool((*a != 0) || *b)),
            (Self::Bool(a), Self::Int(b)) => Ok(Self::Bool(*a || (*b != 0))),
            (Self::Bool(a), Self::Bool(b)) => Ok(Self::Bool(*a || *b)),
            (Self::Float(a), Self::Float(b)) => Ok(Self::Float(a + b)),
            (Self::Int(a), Self::Float(b)) => Ok(Self::Float((*a as f64) + b)),
            (Self::Float(a), Self::Int(b)) => Ok(Self::Float(a + (*b as f64))),
            (Self::String(a), Self::String(b)) => Ok(Self::String(format!("{a}{b}").into()).into()),
            (Self::Array(a), Self::Array(b)) => {
                Ok(Self::Array(a.iter().chain(b.iter()).cloned().collect()))
            }
            (Self::Object(a), Self::Object(b)) => {
                let mut merged = a.clone();
                Arc::make_mut(&mut merged).extend((**b).clone());
                Ok(Self::Object(merged))
            }
            _ => bail!(
                "Impossible d'additionner {} et {}: les deux valeurs doivent être des nombres compatibles, deux chaînes, deux listes ou deux dictionnaires",
                self.diagnostic(),
                other.diagnostic()
            ),
        }
    }
    /// Attempts to sub using formula-language semantics.
    ///
    /// Implements formula-language coercion rules and rejects unsupported operand types or
    /// arithmetic domain errors instead of falling back to string conversion.
    ///
    /// # Errors
    ///
    /// Returns an error unless both operands are numeric.
    pub fn try_sub(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a - b)),
            (Self::Float(a), Self::Float(b)) => Ok(Self::Float(a - b)),
            (Self::Int(a), Self::Float(b)) => Ok(Self::Float((*a as f64) - b)),
            (Self::Float(a), Self::Int(b)) => Ok(Self::Float(a - (*b as f64))),
            _ => bail!(
                "Impossible de soustraire {} et {}: les deux valeurs doivent être numériques",
                self.diagnostic(),
                other.diagnostic()
            ),
        }
    }
    /// Attempts to mul using formula-language semantics.
    ///
    /// Implements formula-language coercion rules and rejects unsupported operand types or
    /// arithmetic domain errors instead of falling back to string conversion.
    ///
    /// # Errors
    ///
    /// Returns an error when the operand types do not support multiplication.
    pub fn try_mul(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::Int(a), Self::Bool(b)) => Ok(Self::Bool((*a != 0) && *b)),
            (Self::Bool(a), Self::Int(b)) => Ok(Self::Bool(*a && (*b != 0))),
            (Self::Bool(a), Self::Bool(b)) => Ok(Self::Bool(*a && *b)),
            (Self::Int(a), Self::Int(b)) => Ok(Self::Int(a * b)),
            (Self::Float(a), Self::Float(b)) => Ok(Self::Float(a * b)),
            (Self::Int(a), Self::Float(b)) => Ok(Self::Float((*a as f64) * b)),
            (Self::Float(a), Self::Int(b)) => Ok(Self::Float(a * (*b as f64))),
            (Self::String(a), Self::Int(b)) => {
                if *b < 0 {
                    bail!(
                        "Impossible de multiplier {} par {}: le multiplicateur d'une chaîne ne peut pas être négatif",
                        self.diagnostic(),
                        other.diagnostic()
                    );
                }
                if *b == 1 {
                    Ok(Self::String(a.clone()))
                } else {
                    Ok(Self::String((*a).repeat(*b as usize).into()))
                }
            }
            (Self::Int(a), Self::String(b)) => {
                if *a < 0 {
                    bail!(
                        "Impossible de multiplier {} par {}: le multiplicateur d'une chaîne ne peut pas être négatif",
                        self.diagnostic(),
                        other.diagnostic()
                    );
                }
                if *a == 1 {
                    Ok(Self::String(b.clone()))
                } else {
                    Ok(Self::String((*b).repeat(*a as usize).into()))
                }
            }
            _ => bail!(
                "Impossible de multiplier {} et {}: les deux valeurs doivent être numériques, ou l'une doit être une chaîne et l'autre un entier",
                self.diagnostic(),
                other.diagnostic()
            ),
        }
    }
    /// Attempts to div using formula-language semantics.
    ///
    /// Implements formula-language coercion rules and rejects unsupported operand types or
    /// arithmetic domain errors instead of falling back to string conversion.
    ///
    /// # Errors
    ///
    /// Returns an error for non-numeric operands or division by zero.
    pub fn try_div(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => {
                if *b == 0 {
                    bail!("Impossible de diviser {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float((*a as f64) / (*b as f64)))
            }
            (Self::Float(a), Self::Float(b)) => {
                if *b == 0.0 {
                    bail!("Impossible de diviser {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float(a / b))
            }
            (Self::Int(a), Self::Float(b)) => {
                if *b == 0.0 {
                    bail!("Impossible de diviser {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float((*a as f64) / b))
            }
            (Self::Float(a), Self::Int(b)) => {
                if *b == 0 {
                    bail!("Impossible de diviser {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float(a / (*b as f64)))
            }
            _ => bail!(
                "Impossible de diviser {} par {}: les deux valeurs doivent être numériques",
                self.diagnostic(),
                other.diagnostic()
            ),
        }
    }
    /// Attempts to mod using formula-language semantics.
    ///
    /// Implements formula-language coercion rules and rejects unsupported operand types or
    /// arithmetic domain errors instead of falling back to string conversion.
    ///
    /// # Errors
    ///
    /// Returns an error for non-numeric operands or a zero divisor.
    pub fn try_mod(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => {
                if *b == 0 {
                    bail!("Impossible de calculer {} modulo {}: modulo par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Int(a % b))
            }
            (Self::Float(a), Self::Float(b)) => {
                if *b == 0.0 {
                    bail!("Impossible de calculer {} modulo {}: modulo par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float(a - (a / b).floor() * b))
            }
            (Self::Int(a), Self::Float(b)) => {
                if *b == 0.0 {
                    bail!("Impossible de calculer {} modulo {}: modulo par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float((*a as f64) % b))
            }
            (Self::Float(a), Self::Int(b)) => {
                if *b == 0 {
                    bail!("Impossible de calculer {} modulo {}: modulo par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float(a % (*b as f64)))
            }
            _ => bail!(
                "Impossible de calculer le modulo de {} et {}: les deux valeurs doivent être numériques",
                self.diagnostic(),
                other.diagnostic()
            ),
        }
    }
    /// Attempts to floor div using formula-language semantics.
    ///
    /// Implements formula-language coercion rules and rejects unsupported operand types or
    /// arithmetic domain errors instead of falling back to string conversion.
    ///
    /// # Errors
    ///
    /// Returns an error for non-numeric operands or division by zero.
    pub fn try_floor_div(&self, other: &Self) -> Result<Self> {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => {
                if *b == 0 {
                    bail!("Impossible de diviser entièrement {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Int((*a as f64 / *b as f64).floor() as i64))
            }
            (Self::Float(a), Self::Float(b)) => {
                if *b == 0.0 {
                    bail!("Impossible de diviser entièrement {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float((a / b).floor()))
            }
            (Self::Int(a), Self::Float(b)) => {
                if *b == 0.0 {
                    bail!("Impossible de diviser entièrement {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float(((*a as f64) / b).floor()))
            }
            (Self::Float(a), Self::Int(b)) => {
                if *b == 0 {
                    bail!("Impossible de diviser entièrement {} par {}: division par zéro", self.diagnostic(), other.diagnostic());
                }
                Ok(Self::Float((a / (*b as f64)).floor()))
            }
            _ => bail!(
                "Impossible de calculer la division entière de {} et {}: les deux valeurs doivent être numériques",
                self.diagnostic(),
                other.diagnostic()
            ),
        }
    }
}

impl TryFrom<&serde_json::Value> for EvalValue {
    type Error = EvalError;

    fn try_from(value: &serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Null => Ok(EvalValue::Null),
            serde_json::Value::Bool(value) => Ok(EvalValue::Bool(*value)),
            serde_json::Value::String(value) => Ok(EvalValue::String(value.as_str().into())),
            serde_json::Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Ok(EvalValue::Int(value))
                } else if let Some(value) = value.as_u64() {
                    Ok(EvalValue::Int(value.cast_signed()))
                } else if let Some(value) = value.as_f64() {
                    Ok(EvalValue::Float(value))
                } else {
                    Err(EvalError::InvalidNumber)
                }
            }
            serde_json::Value::Array(values) => values
                .iter()
                .map(EvalValue::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map(|arr| EvalValue::Array(arr.into())),
            serde_json::Value::Object(values) => {
                let inner = values
                    .iter()
                    .map(|(key, value)| Ok((Arc::from(key.as_str()), EvalValue::try_from(value)?)))
                    .collect::<Result<IndexMap<_, _>, _>>()?;
                Ok(EvalValue::Object(inner.into()))
            }
        }
    }
}
impl TryFrom<&EvalValue> for serde_json::Value {
    type Error = EvalError;

    fn try_from(value: &EvalValue) -> Result<Self, Self::Error> {
        match value {
            EvalValue::Null => Ok(serde_json::Value::Null),
            EvalValue::Bool(value) => Ok(serde_json::Value::Bool(*value)),
            EvalValue::String(value) => Ok(serde_json::Value::String(value.to_string())),
            EvalValue::Int(value) => {
                Ok(serde_json::Value::Number(serde_json::Number::from(*value)))
            }
            EvalValue::Float(value) => serde_json::Number::from_f64(*value)
                .map(serde_json::Value::Number)
                .ok_or(EvalError::InvalidNumber),
            EvalValue::Array(array) => {
                let mut json_array = Vec::with_capacity(array.len());
                for item in array.iter() {
                    let json_item = serde_json::Value::try_from(item)?;
                    json_array.push(json_item);
                }
                Ok(serde_json::Value::Array(json_array))
            }
            EvalValue::Object(object) => {
                let mut json_object = serde_json::Map::new();
                for (key, value) in object.iter() {
                    let json_value = serde_json::Value::try_from(value)?;
                    json_object.insert(key.to_string(), json_value);
                }
                Ok(serde_json::Value::Object(json_object))
            }
            EvalValue::Character(_) => Err(EvalError::NonSerializableValue),
            EvalValue::Func(func) => match func.as_ref() {
                Function::User(closure) => Ok(serde_json::Value::String(closure.definition.to_string())),
                Function::Builtin(_, _) => Err(EvalError::NonSerializableValue),
            },
        }
    }
}
impl TryFrom<serde_json::Value> for EvalValue {
    type Error = EvalError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        EvalValue::try_from(&value)
    }
}
impl TryFrom<EvalValue> for serde_json::Value {
    type Error = EvalError;

    fn try_from(value: EvalValue) -> Result<Self, Self::Error> {
        serde_json::Value::try_from(&value)
    }
}
impl From<String> for EvalValue {
    fn from(value: String) -> Self {
        EvalValue::String(value.into())
    }
}
impl From<Arc<str>> for EvalValue {
    fn from(value: Arc<str>) -> Self {
        EvalValue::String(value)
    }
}
impl From<&str> for EvalValue {
    fn from(value: &str) -> Self {
        EvalValue::String(value.into())
    }
}
impl From<i64> for EvalValue {
    fn from(value: i64) -> Self {
        EvalValue::Int(value)
    }
}
impl From<f64> for EvalValue {
    fn from(value: f64) -> Self {
        EvalValue::Float(value)
    }
}
impl From<bool> for EvalValue {
    fn from(value: bool) -> Self {
        EvalValue::Bool(value)
    }
}
impl TryFrom<EvalValue> for Arc<str> {
    type Error = anyhow::Error;

    fn try_from(value: EvalValue) -> Result<Self, Self::Error> {
        match value {
            EvalValue::String(s) => Ok(s),
            other => Err(anyhow::anyhow!(
                "Impossible d'utiliser {} comme chaîne: une valeur de type str est requise",
                other.diagnostic()
            )),
        }
    }
}
impl TryFrom<EvalValue> for String {
    type Error = anyhow::Error;

    fn try_from(value: EvalValue) -> Result<Self, Self::Error> {
        match value {
            EvalValue::String(s) => Ok(s.to_string()),
            other => Err(anyhow::anyhow!(
                "Impossible d'utiliser {} comme chaîne: une valeur de type str est requise",
                other.diagnostic()
            )),
        }
    }
}
impl TryFrom<EvalValue> for i64 {
    type Error = anyhow::Error;

    fn try_from(value: EvalValue) -> Result<Self, Self::Error> {
        match value {
            EvalValue::Int(i) => Ok(i),
            other => Err(anyhow::anyhow!(
                "Impossible d'utiliser {} comme entier strict: une valeur de type int est requise",
                other.diagnostic()
            )),
        }
    }
}
impl TryFrom<EvalValue> for f64 {
    type Error = anyhow::Error;

    fn try_from(value: EvalValue) -> Result<Self, Self::Error> {
        match value {
            EvalValue::Float(f) => Ok(f),
            other => Err(anyhow::anyhow!(
                "Impossible d'utiliser {} comme nombre décimal strict: une valeur de type float est requise",
                other.diagnostic()
            )),
        }
    }
}
impl TryFrom<EvalValue> for bool {
    type Error = anyhow::Error;

    fn try_from(value: EvalValue) -> Result<Self, Self::Error> {
        match value {
            EvalValue::Bool(b) => Ok(b),
            other => Err(anyhow::anyhow!(
                "Impossible d'utiliser {} comme booléen strict: une valeur de type bool est requise",
                other.diagnostic()
            )),
        }
    }
}

/// Origin of the binding selected by environment lookup, independent of its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindingOrigin {
    Local,
    TaleGlobal,
    EngineGlobal,
}

/// Stores named values exposed to formula evaluation.
#[derive(Clone, Debug, Default)]
pub struct Env<'a> {
    parent: Vec<&'a Env<'a>>,
    pub vars: Arc<IndexMap<Arc<str>, EvalValue>>,
    /// Origins of direct global bindings. An absent entry means a local binding.
    origins: Arc<IndexMap<Arc<str>, BindingOrigin>>,
}

impl<'a> Env<'a> {
    /// Creates an empty formula environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an environment by converting every entry in a JSON object.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    pub fn from_json_map(values: &JsonMap<String, JsonValue>) -> Env<'static> {
        Env {
            parent: Vec::new(),
            vars: Arc::from(
                values
                    .iter()
                    .map(|(key, value)| (Arc::from(key.as_str()), EvalValue::from_json(value)))
                    .collect::<IndexMap<_, _>>(),
            ),
            origins: Arc::new(IndexMap::new()),
        }
    }

    /// Creates an environment by cloning an existing map of evaluated values.
    pub fn from_index_map(values: &IndexMap<Arc<str>, EvalValue>) -> Env<'static> {
        Env {
            parent: Vec::new(),
            vars: Arc::from(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<IndexMap<_, _>>(),
            ),
            origins: Arc::new(IndexMap::new()),
        }
    }

    /// Creates an environment whose direct entries are known tale-global bindings.
    pub fn from_tale_globals(values: &IndexMap<Arc<str>, EvalValue>) -> Env<'static> {
        Env {
            parent: Vec::new(),
            vars: Arc::new(values.clone()),
            origins: Arc::new(
                values
                    .keys()
                    .map(|name| (name.clone(), BindingOrigin::TaleGlobal))
                    .collect(),
            ),
        }
    }

    /// Creates an owned environment that shares an existing immutable value map.
    pub fn from_arc(values: Arc<IndexMap<Arc<str>, EvalValue>>) -> Env<'static> {
        Env {
            parent: Vec::new(),
            vars: values,
            origins: Arc::new(IndexMap::new()),
        }
    }

    /// Takes an owned snapshot and marks every visible binding as an engine global.
    /// This is done once when installing globals, including names inherited from parents.
    fn into_engine_globals(self) -> Env<'static> {
        let mut globals = self.solidified();
        globals.origins = Arc::new(
            globals
                .vars
                .keys()
                .map(|name| (name.clone(), BindingOrigin::EngineGlobal))
                .collect(),
        );
        globals
    }
    /// Creates a local binding, replacing any existing binding even when the value is equal.
    pub fn insert(&mut self, key: impl Into<Arc<str>>, value: EvalValue) {
        let key: Arc<str> = key.into();
        let inner = Arc::make_mut(&mut self.vars);
        inner.insert(key.clone(), value);
        Arc::make_mut(&mut self.origins).shift_remove(key.as_ref());
    }
    /// Converts a JSON value to [`EvalValue`] and binds it under `key`.
    pub fn insert_json(&mut self, key: impl Into<String>, value: &JsonValue) {
        let key: String = key.into();
        let key: Arc<str> = key.into();
        let inner = Arc::make_mut(&mut self.vars);
        inner.insert(key.clone(), EvalValue::from_json(value));
        Arc::make_mut(&mut self.origins).shift_remove(key.as_ref());
    }

    /// Extends this environment with values from another environment.
    pub fn extend(&mut self, values: &IndexMap<Arc<str>, EvalValue>) {
        let inner = Arc::make_mut(&mut self.vars);
        let origins = Arc::make_mut(&mut self.origins);
        for (key, value) in values {
            inner.insert(key.clone(), value.clone());
            origins.shift_remove(key.as_ref());
        }
    }

    /// Converts and inserts every value from a JSON object.
    pub fn extend_json(&mut self, values: &JsonMap<String, JsonValue>) {
        for (key, value) in values {
            self.insert_json(key.clone(), value);
        }
    }
    /// Returns the value bound to `key`, or `None` when the name is undefined.
    pub fn get(&self, key: &str) -> Option<&EvalValue> {
        self.get_binding(key).map(|(value, _)| value)
    }

    /// Resolves the value and its origin together using the same lookup precedence.
    fn get_binding(&self, key: &str) -> Option<(&EvalValue, BindingOrigin)> {
        if let Some(value) = self.vars.get(key) {
            let origin = self
                .origins
                .get(key)
                .copied()
                .unwrap_or(BindingOrigin::Local);
            Some((value, origin))
        } else {
            self.parent
                .iter()
                .find_map(|parent| parent.get_binding(key))
        }
    }

    /// Lists visible names for an unknown-variable diagnostic.
    fn visible_names_for_error(&self) -> String {
        let solidified = self.visible_names_for_error_inner();
        let total = solidified.len();
        let mut names = solidified.iter()
            .take(24)
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>();
        if total > names.len() {
            names.push(format!("… et {} autre(s)", total - names.len()));
        }
        if names.is_empty() {
            "aucun".to_owned()
        } else {
            names.join(", ")
        }
    }
    fn visible_names_for_error_inner(&self) -> HashSet<String> {
        let mut set = HashSet::new();
        if !Arc::ptr_eq(&self.vars, &GLOBAL_ENV.vars) {
            for key in self.vars.keys() {
                set.insert(key.to_string());
            }
        }
        for parent in &self.parent {
            set.extend(parent.visible_names_for_error_inner());
        }
        set
    }

    /// Returns a cloned environment extended with `other`.
    ///
    /// Bindings from `other` override names already present in `self`.
    pub fn merged<'b: 'a>(&'a self, other: &'b Env<'a>) -> Env<'a> {
        let parents = vec![other, self];
        Env {
            parent: parents,
            vars: Arc::from(IndexMap::new()),
            origins: Arc::new(IndexMap::new()),
        }
    }

    /// Returns a new environment with all parent environments merged together
    pub fn merge(parents: &[&'a Env<'a>]) -> Env<'a> {
        Env {
            parent: {
                let mut p = parents.to_vec();
                p.reverse();
                p
            },
            vars: Arc::from(IndexMap::new()),
            origins: Arc::new(IndexMap::new()),
        }
    }

    /// Add a parent environment to the chain of lookups. This is useful for temporary overrides without cloning the entire environment.
    ///
    pub fn with_parent<'b>(mut self, other: &'b Env<'b>) -> Env<'a>
    where
        'b: 'a,
    {
        self.parent.push(other);
        self
    }
    /// Flattens parent environments while preserving each visible binding's origin.
    pub fn solidified(&self) -> Env<'static> {
        if self.parent.is_empty() {
            return Env {
                parent: Vec::new(),
                vars: self.vars.clone(),
                origins: self.origins.clone(),
            };
        }
        let mut solidified = Env::new();
        for parent in self.parent.iter().rev() {
            solidified = solidified.expand(parent.solidified());
        }
        solidified.expand(Env {
            parent: Vec::new(),
            vars: self.vars.clone(),
            origins: self.origins.clone(),
        })
    }

    /// Merges another environment into this one, overriding existing bindings with those from the other environment.
    pub fn expand<'b: 'a>(mut self, other: Env<'b>) -> Env<'a> {
        self.parent = [other.parent.as_slice(), self.parent.as_slice()].concat();
        let inner = Arc::make_mut(&mut self.vars);
        let origins = Arc::make_mut(&mut self.origins);
        for (key, value) in other.vars.iter() {
            inner.insert(key.clone(), value.clone());
            if let Some(origin) = other.origins.get(key.as_ref()) {
                origins.insert(key.clone(), *origin);
            } else {
                origins.shift_remove(key.as_ref());
            }
        }
        self
    }
}

/// A name or an exact-length list pattern used by `let` and function parameters.
/// The name `_` is an ignore pattern: it consumes a value without binding a name.
/// Untagged serialization keeps existing name-only saved formulas compatible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BindingPattern {
    Name(String),
    List(Vec<BindingPattern>),
}

impl From<&str> for BindingPattern {
    fn from(name: &str) -> Self {
        Self::Name(name.to_owned())
    }
}
impl From<String> for BindingPattern {
    fn from(name: String) -> Self {
        Self::Name(name)
    }
}
impl From<Arc<str>> for BindingPattern {
    fn from(name: Arc<str>) -> Self {
        Self::Name(name.to_string())
    }
}
impl std::fmt::Display for BindingPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(name) => write!(f, "{name}"),
            Self::List(items) => write!(f, "[{}]", format_patterns(items)),
        }
    }
}
fn format_patterns(patterns: &[BindingPattern]) -> String {
    patterns
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
impl BindingPattern {
    fn collect_names(&self, names: &mut IndexSet<Arc<str>>) {
        match self {
            Self::Name(name) => {
                if name != "_" {
                    names.insert(name.as_str().into());
                }
            }
            Self::List(items) => {
                for item in items {
                    item.collect_names(names);
                }
            }
        }
    }

    fn bind(&self, value: EvalValue, env: &mut Env) -> Result<()> {
        match self {
            Self::Name(name) => {
                if name != "_" {
                    env.insert(name.as_str(), value);
                }
            }
            Self::List(patterns) => {
                let EvalValue::Array(values) = &value else {
                    bail!(
                        "Le motif {self} exige une liste, reçu {}",
                        value.diagnostic()
                    );
                };
                if patterns.len() != values.len() {
                    bail!(
                        "Le motif {self} exige exactement {} élément(s), reçu {}",
                        patterns.len(),
                        values.len()
                    );
                }
                for (pattern, value) in patterns.iter().zip(values.iter()) {
                    pattern.bind(value.clone(), env)?;
                }
            }
        }
        Ok(())
    }
}

/// Declares the parsed definition of one tale-defined pure formula function.
///
/// When `parameters` is `Some`, calls must provide at least that many arguments and each
/// leading value is bound using the corresponding name or list pattern. Every call also exposes
/// the complete argument list through `args`. The body uses ordinary formula syntax and is
/// evaluated once per call in the closure environment extended with those bindings.
#[derive(Clone, Debug, PartialEq)]
pub struct FormulaFunction {
    /// Ordered parameter patterns, or `None` when the function accepts any arity through `args`.
    pub parameters: Option<Arc<[BindingPattern]>>,
    /// Complete `${...}` formula resolved once for each call.
    pub body: Arc<Expr>,
}
impl std::fmt::Display for FormulaFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(parameters) = &self.parameters {
            write!(f, "fn {} -> {}", format_patterns(parameters), self.body)
        } else {
            write!(f, "fn -> {}", self.body)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormulaFunctionSerializer {
    parameters: Option<Vec<BindingPattern>>,
    body: Box<ExprSerializer>,
}
impl From<&FormulaFunction> for FormulaFunctionSerializer {
    fn from(function: &FormulaFunction) -> Self {
        Self {
            parameters: function
                .parameters
                .as_ref()
                .map(|params| params.to_vec()),
            body: Box::new(ExprSerializer::from(function.body.as_ref())),
        }
    }
}
impl TryFrom<FormulaFunctionSerializer> for FormulaFunction {
    type Error = anyhow::Error;
    fn try_from(serializer: FormulaFunctionSerializer) -> Result<Self, Self::Error> {
        Ok(Self {
            parameters: serializer.parameters.map(Arc::from),
            body: Arc::new((*serializer.body).try_into()?),
        })
    }
}

/// Stores a function definition together with the lexical values captured when it was created.
///
/// Only referenced free variables are retained. Runtime-wide bindings such as `recit`,
/// `personnage`, and `page` remain dynamic and are therefore absent from `captures`.
#[derive(Clone, Debug, PartialEq)]
pub struct FormulaClosure {
    /// Parsed function definition shared with the cached formula AST.
    pub definition: FormulaFunction,
    /// Free lexical bindings copied by value when the function expression was evaluated.
    pub captures: Arc<IndexMap<Arc<str>, EvalValue>>,
    /// Free names resolved from `recit` at call time instead of copied into the closure.
    pub tale_globals: Arc<IndexSet<Arc<str>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormulaClosureSerializer {
    definition: FormulaFunctionSerializer,
    captures: IndexMap<String, EvalValue>,
    tale_globals: IndexSet<String>,
}
impl From<&FormulaClosure> for FormulaClosureSerializer {
    fn from(closure: &FormulaClosure) -> Self {
        Self {
            definition: FormulaFunctionSerializer::from(&closure.definition),
            captures: closure
                .captures
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone()))
                .collect(),
            tale_globals: closure
                .tale_globals
                .iter()
                .map(|key| key.to_string())
                .collect(),
        }
    }
}
impl TryFrom<FormulaClosureSerializer> for FormulaClosure {
    type Error = anyhow::Error;
    fn try_from(serializer: FormulaClosureSerializer) -> Result<Self, Self::Error> {
        Ok(Self {
            definition: serializer.definition.try_into()?,
            captures: Arc::from(
                serializer
                    .captures
                    .into_iter()
                    .map(|(key, value)| (Arc::from(key), value))
                    .collect::<IndexMap<_, _>>(),
            ),
            tale_globals: Arc::from(
                serializer
                    .tale_globals
                    .into_iter()
                    .map(Arc::from)
                    .collect::<IndexSet<_>>(),
            ),
        })
    }
}

impl FormulaClosure {
    /// Wraps a legacy function definition that has no lexical creation environment.
    pub fn without_captures(definition: FormulaFunction) -> Self {
        Self {
            definition,
            captures: Arc::new(IndexMap::new()),
            tale_globals: Arc::new(IndexSet::new()),
        }
    }
}
impl std::fmt::Display for FormulaClosure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.captures.is_empty() {
            write!(f, "{}", self.definition)
        } else {
            write!(f, "|{}| {}",
                self.captures
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                self.definition
            )
        }
    }
}

/// Callable formula value backed by either a lexical tale closure or a registered built-in.
#[derive(Clone, Debug)]
pub enum Function {
    User(FormulaClosure),
    Builtin(Arc<str>, fn(&[EvalValue], &mut FormulaEngine, &Env) -> Result<EvalValue>),
}
impl std::fmt::Display for Function {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User(closure) => write!(f, "{}", closure),
            Self::Builtin(name, _) => write!(f, "<builtin {}>", name),
        }
    }
}

impl PartialEq for Function {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::User(left), Self::User(right)) => left == right,
            (Self::Builtin(keyl, _left), Self::Builtin(keyr, _right)) => keyl == keyr,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FunctionSerializer {
    User(FormulaClosureSerializer),
    Builtin(String),
}
impl From<&Function> for FunctionSerializer {
    fn from(function: &Function) -> Self {
        match function {
            Function::User(closure) => Self::User(FormulaClosureSerializer::from(closure)),
            Function::Builtin(name, _) => Self::Builtin(name.to_string()),
        }
    }
}
impl TryFrom<FunctionSerializer> for Function {
    type Error = anyhow::Error;
    fn try_from(serializer: FunctionSerializer) -> Result<Self, Self::Error> {
        match serializer {
            FunctionSerializer::User(closure) => Ok(Self::User(closure.try_into()?)),
            FunctionSerializer::Builtin(name) => {
                let name: Arc<str> = name.into();
                let function = builtins::ALL_BUILTINS
                    .get(name.as_ref())
                    .ok_or_else(|| anyhow!("Builtin function not found"))?;
                let EvalValue::Func(func) = function else {
                    bail!("Builtin function was not a function");
                };
                let Function::Builtin(name, ptr) = func.as_ref() else {
                    bail!("Builtin function was not a builtin");
                };
                Ok(Self::Builtin(name.clone(), *ptr))
            }
        }
    }
}

/// Parses, formats, and evaluates formulas with deterministic random support.
pub struct FormulaEngine {
    rng: ChaCha8Rng,
    cache: HashMap<Arc<str>, Expr>,
    globals: Env<'static>,
    call_stack: Vec<Arc<str>>,
}

impl FormulaEngine {
    /// Creates a formula engine with deterministic randomness when `seed` is supplied.
    ///
    /// Parsed expressions are cached by normalized source text for reuse.
    pub fn new(seed: Option<u64>) -> Self {
        let rng = match seed {
            Some(seed) => ChaCha8Rng::seed_from_u64(seed),
            None => ChaCha8Rng::from_entropy(),
        };
        Self {
            rng,
            cache: HashMap::new(),
            globals: GLOBAL_ENV.clone().into_engine_globals(),
            call_stack: Vec::new(),
        }
    }

    /// Replaces engine globals with a snapshot of every visible binding in `globals`.
    /// These bindings remain dynamic in closures unless a local binding shadows them.
    pub fn with_globals(
        self,
        globals: Env<'static>,
    ) -> Self {
        Self {
            globals: globals.into_engine_globals(),
            ..self
        }
    }
    /// Returns a pseudorandom floating-point percentage in the half-open range `0.0..100.0`.
    pub fn random_percent(&mut self) -> f64 {
        self.rng.gen_range(0.0..=100.0)
    }
    /// Returns a pseudorandom d20 result in the inclusive range `1..=20`.
    pub fn d20(&mut self) -> i64 {
        self.rng.gen_range(1..=20)
    }
    /// Parses (or retrieves the cached AST for) `expression` and evaluates it against `env`.
    ///
    /// An optional outer `${...}` wrapper is stripped before parsing.
    ///
    /// # Errors
    ///
    /// Returns contextual tokenizer, parser, lookup, type, arithmetic, or evaluation errors.
    pub fn evaluate(&mut self, expression: &str, env: &Env) -> Result<EvalValue> {
        let expression = full_formula(expression).unwrap_or(expression);
        let expression: Arc<str> = Arc::from(strip_formula_dollars(expression));
        let ast = if let Some(ast) = self.cache.get(&expression) {
            ast.clone()
        } else {
            let tokens = Tokenizer::new(&expression)
                .tokenize()
                .with_context(|| format!("Erreur lexicale dans la formule '{expression}'"))?;
            let ast = Parser::new(tokens).parse().map_err(|err| anyhow!("Erreur de syntaxe dans la formule '{expression}': {err}"))?;
            self.cache.insert(expression.clone(), ast.clone());
            ast
        };
        
        let globals = self.globals.clone();
        let env = env.clone().with_parent(&globals);
        return self
            .evaluate_ast(&ast, &env)
            .with_context(|| format!("Erreur dans la formule '{expression}'"));
    }
    /// Resolves one [`EvalValue`] against `env`.
    ///
    /// A string consisting entirely of `${...}` preserves the evaluated type; other strings are formatted as text. Non-string values are cloned unchanged. Formula-looking strings produced by evaluation are not recursively resolved.
    ///
    /// # Errors
    ///
    /// Propagates formatting or formula evaluation errors.
    pub fn resolve(&mut self, value: &EvalValue, env: &Env) -> Result<EvalValue> {
        match value {
            EvalValue::String(text) => {
                if let Some(expression) = full_formula(text) {
                    self.evaluate(expression, env)
                } else {
                    Ok(EvalValue::String(self.format(text.clone(), env)?))
                }
            }
            _ => Ok(value.clone()),
        }
    }
    /// Resolves `value` once and converts the result to an integer.
    ///
    /// # Errors
    ///
    /// Returns an error when formula evaluation fails or the resolved value is not integer-compatible.
    pub fn resolve_int(&mut self, value: &EvalValue, env: &Env) -> Result<i64> {
        let resolved = self
            .resolve(value, env)
            .context("Impossible de résoudre la valeur attendue comme entier")?;
        resolved
            .to_i64()
            .with_context(|| format!("La résolution a produit {}", resolved.diagnostic()))
    }
    /// Resolves `value` once and converts the result to a floating-point number.
    ///
    /// # Errors
    ///
    /// Returns an error when formula evaluation fails or the resolved value is not numeric.
    pub fn resolve_float(&mut self, value: &EvalValue, env: &Env) -> Result<f64> {
        let resolved = self
            .resolve(value, env)
            .context("Impossible de résoudre la valeur attendue comme nombre")?;
        resolved
            .to_f64()
            .with_context(|| format!("La résolution a produit {}", resolved.diagnostic()))
    }
    /// Resolves `value` once and returns its formula-language truthiness.
    ///
    /// # Errors
    ///
    /// Propagates formula evaluation errors.
    pub fn resolve_bool(&mut self, value: &EvalValue, env: &Env) -> Result<bool> {
        let resolved = self.resolve(value, env)?;
        Ok(resolved.truthy())
    }
    /// Resolves `value` as a non-negative integer quantity.
    ///
    /// # Errors
    ///
    /// Returns an error on formula/conversion failure or when the resolved quantity is negative.
    pub fn resolve_quantity(&mut self, value: &EvalValue, env: &Env) -> Result<i64> {
        let quantity = self.resolve_int(value, env)?;
        if quantity < 0 {
            bail!("Une quantité doit être un entier positif. A reçu {quantity}.")
        }
        Ok(quantity)
    }
    /// Expands `${...}` expressions and `$variable` substitutions embedded in display text.
    ///
    /// Each discovered expression is evaluated once; formula-looking text produced by an expression is not recursively expanded during the same call.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed substitutions or formula evaluation failures.
    pub fn format(&mut self, text: Arc<str>, env: &Env) -> Result<Arc<str>> {
        let chars: Vec<char> = text.chars().collect();
        let mut output = String::new();
        let mut index = 0;

        while index < chars.len() {
            if chars[index] != '$' {
                output.push(chars[index]);
                index += 1;
                continue;
            }

            if chars.get(index + 1) == Some(&'$') {
                output.push('$');
                index += 2;
                continue;
            }

            if chars.get(index + 1) == Some(&'{') {
                let Some(end) = find_formula_end(&chars, index + 2) else {
                    output.push('$');
                    index += 1;
                    continue;
                };
                let expression: String = chars[index + 2..end].iter().collect();
                let value = self.evaluate(&expression, env)?;
                output.push_str(&value.python_string());
                index = end + 1;
                continue;
            }

            let mut end = index + 1;
            while end < chars.len() && is_word(chars[end]) {
                end += 1;
            }
            if end == index + 1 {
                output.push('$');
                index += 1;
                continue;
            }

            let name: String = chars[index + 1..end].iter().collect();
            if let Some(value) = env.get(&name) {
                output.push_str(&value.python_string());
            } else {
                output.push('$');
                output.push_str(&name);
            }
            index = end;
        }

        if output == text.as_ref() {
            return Ok(text);
        }
        Ok(Arc::from(output))
    }
    pub fn fmt_value(&mut self, value: &EvalValue, env: &Env) -> Result<Arc<str>> {
        Ok(self.resolve_formula(value, env)?.python_string())
    }
    /// Resolves `value` once and evaluates it if it is a formula function.
    pub fn resolve_formula(&mut self, value: &EvalValue, env: &Env) -> Result<EvalValue> {
        match value {
            EvalValue::Func(_) => {
                let globals = self.globals.clone();
                let env = env.clone().with_parent(&globals);
                value.clone().call(&[], self, &env)
            },
            _ => Ok(value.clone()),
        }
    }
    /// Resolves `value` once and evaluates it if it is a formula function.
    pub fn resolve_formula_int(&mut self, value: &EvalValue, env: &Env) -> Result<i64> {
        let resolved = self
            .resolve_formula(value, env)
            .context("Impossible d'évaluer la formule attendue comme entier")?;
        resolved
            .to_i64()
            .with_context(|| format!("La formule a produit {}", resolved.diagnostic()))
    }
    pub fn resolve_formula_float(&mut self, value: &EvalValue, env: &Env) -> Result<f64> {
        let resolved = self
            .resolve_formula(value, env)
            .context("Impossible d'évaluer la formule attendue comme nombre")?;
        resolved
            .to_f64()
            .with_context(|| format!("La formule a produit {}", resolved.diagnostic()))
    }
    pub fn resolve_formula_bool(&mut self, value: &EvalValue, env: &Env) -> Result<bool> {
        Ok(self.resolve_formula(value, env)?.truthy())
    }
    pub fn resolve_formula_quantity(&mut self, value: &EvalValue, env: &Env) -> Result<i64> {
        let quantity = self.resolve_formula_int(value, env)?;
        if quantity < 0 {
            bail!("Une quantité doit être un entier positif. A reçu {quantity}.")
        }
        Ok(quantity)
    }

    /// Creates a lexical closure by copying only the free values used by its body.
    ///
    /// Tale and engine globals stay dynamic. Local bindings are captured even when their
    /// values equal a global; capture depends on the binding's origin, never value equality.
    fn close_function(&self, function: &FormulaFunction, env: &Env) -> FormulaClosure {
        let mut captures = IndexMap::new();
        let mut tale_globals = IndexSet::new();
        for name in function.free_variables() {
            if matches!(name.as_ref(), "recit" | "personnage" | "page") {
                continue;
            }
            let Some((value, origin)) = env.get_binding(name.as_ref()) else {
                // An unresolved free name may be supplied dynamically by the call environment.
                continue;
            };
            match origin {
                BindingOrigin::TaleGlobal => {
                    tale_globals.insert(name);
                }
                BindingOrigin::EngineGlobal => {}
                BindingOrigin::Local => {
                    captures.insert(name, value.clone());
                }
            }
        }
        FormulaClosure {
            definition: function.clone(),
            captures: Arc::new(captures),
            tale_globals: Arc::new(tale_globals),
        }
    }

    /// Evaluates one parsed expression against the supplied environment.
    fn evaluate_ast(&mut self, expression: &Expr, env: &Env) -> Result<EvalValue> {
        match expression {
            Expr::Literal(value) => Ok(value.clone()),
            Expr::Variable(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "Nom inconnu `{name}`: aucune variable de ce nom n'est visible. Noms disponibles: {}",
                        env.visible_names_for_error()
                    )
                }),
            Expr::Let { bindings, body } => {
                let mut local_env = env.clone();
                for (name, value) in bindings.iter() {
                    let value = self
                        .evaluate_ast(value, &local_env)
                        .with_context(|| format!("Évaluation de la liaison `let {name}`"))?;
                    name.bind(value, &mut local_env)
                        .with_context(|| format!("Déstructuration de `let {name}`"))?;
                }
                self.evaluate_ast(body, &local_env)
                    .context("Évaluation du corps du `let`")
            }
            Expr::Array(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    self.evaluate_ast(value, env)
                        .with_context(|| format!("Évaluation de l'élément {index} de la liste"))
                })
                .collect::<Result<Vec<_>>>()
                .map(|arr| EvalValue::Array(arr.into())),
            Expr::Object(entries) => {
                let mut values = IndexMap::new();
                for (index, (key, value)) in entries.iter().enumerate() {
                    let key = self
                        .evaluate_ast(key, env)
                        .with_context(|| format!("Évaluation de la clé {index} du dictionnaire"))?;
                    let key = match key {
                        EvalValue::String(key) => key,
                        other => bail!(
                            "La clé {index} du dictionnaire vaut {}: une clé doit être une chaîne",
                            other.diagnostic()
                        ),
                    };
                    let resolved = self
                        .evaluate_ast(value, env)
                        .with_context(|| format!("Évaluation de la valeur associée à la clé `{key}`"))?;
                    values.insert(key, resolved);
                }
                Ok(EvalValue::Object(Arc::from(values)))
            }
            Expr::ListComprehension {
                value,
                comprehension,
            } => {
                let bindings = self.comprehension_bindings(comprehension, env)?;
                let mut values = Vec::new();
                for local_env in bindings {
                    if let Some(filter) = &comprehension.filter {
                        if !self.evaluate_ast(filter, &local_env)?.truthy() {
                            continue;
                        }
                    }
                    values.push(self.evaluate_ast(value, &local_env)?);
                }
                Ok(EvalValue::Array(Arc::from(values)))
            }
            Expr::ObjectComprehension {
                key,
                value,
                comprehension,
            } => {
                let bindings = self.comprehension_bindings(comprehension, env)?;
                let mut values = IndexMap::new();
                for local_env in bindings {
                    if let Some(filter) = &comprehension.filter {
                        if !self.evaluate_ast(filter, &local_env)?.truthy() {
                            continue;
                        }
                    }
                    let resolved_key = match self.evaluate_ast(key, &local_env)? {
                        EvalValue::String(key) => key,
                        other => bail!(
                            "La compréhension a produit la clé {}: une clé de dictionnaire doit être une chaîne",
                            other.diagnostic()
                        ),
                    };
                    values.insert(resolved_key, self.evaluate_ast(value, &local_env)?);
                }
                Ok(EvalValue::Object(Arc::from(values)))
            }
            Expr::Unary(operator, value) => {
                let value = self.evaluate_ast(value, env)?;
                match operator {
                    UnaryOperator::Positive => match value {
                        EvalValue::Int(_) | EvalValue::Float(_) => Ok(value),
                        other => bail!(
                            "Impossible d'appliquer l'opérateur unaire + à {}: un nombre est requis",
                            other.diagnostic()
                        ),
                    },
                    UnaryOperator::Negative => match value {
                        EvalValue::Int(value) => Ok(EvalValue::Int(-value)),
                        EvalValue::Float(value) => Ok(EvalValue::Float(-value)),
                        other => bail!(
                            "Impossible d'appliquer l'opérateur unaire - à {}: un nombre est requis",
                            other.diagnostic()
                        ),
                    },
                    UnaryOperator::Not => Ok(EvalValue::Bool(!value.truthy())),
                    UnaryOperator::BitwiseNot => match value {
                        EvalValue::Int(value) => Ok(EvalValue::Int(!value)),
                        EvalValue::Bool(value) => Ok(EvalValue::Int(!(value as i64))),
                        other => bail!(
                            "Impossible d'appliquer l'opérateur unaire ~ à {}: un entier est requis",
                            other.diagnostic()
                        ),
                    },
                }
            }
            Expr::Binary(left, BinaryOperator::And, right) => {
                let left = self.evaluate_ast(left, env)?;
                if left.truthy() {
                    self.evaluate_ast(right, env)
                } else {
                    Ok(left)
                }
            }
            Expr::Binary(left, BinaryOperator::Or, right) => {
                let left = self.evaluate_ast(left, env)?;
                if left.truthy() {
                    Ok(left)
                } else {
                    self.evaluate_ast(right, env)
                }
            }
            Expr::Binary(left, BinaryOperator::Coalesce, right) => {
                let left = self.evaluate_ast(left, env)?;
                if matches!(left, EvalValue::Null) {
                    self.evaluate_ast(right, env)
                } else {
                    Ok(left)
                }
            }
            Expr::Binary(left, operator, right) => {
                let left = self.evaluate_ast(left, env)?;
                let right = self.evaluate_ast(right, env)?;
                evaluate_binary(operator, left, right)
            }
            Expr::CompareChain(first, comparisons) => {
                let mut left = self.evaluate_ast(first, env)?;
                for (operator, right_expression) in comparisons.iter() {
                    let right = self.evaluate_ast(right_expression, env)?;
                    let result = evaluate_binary(operator, left.clone(), right.clone())?;
                    if !result.truthy() {
                        return Ok(EvalValue::Bool(false));
                    }
                    left = right;
                }
                Ok(EvalValue::Bool(true))
            }
            Expr::Ternary {
                then_expression,
                condition,
                else_expression,
            } => {
                if self.evaluate_ast(condition, env)?.truthy() {
                    self.evaluate_ast(then_expression, env)
                } else {
                    self.evaluate_ast(else_expression, env)
                }
            }
            Expr::Function(function) => {
                Ok(EvalValue::Func(Arc::new(Function::User(
                    self.close_function(function, env),
                ))))
            }
            Expr::Call(callee, arguments) => {
                let arguments = arguments
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        self.evaluate_ast(argument, env).with_context(|| {
                            format!(
                                "Évaluation de l'argument {} de l'appel `{}`",
                                index + 1,
                                diagnostic_expression(callee)
                            )
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;

                // Postfix member calls distinguish stored function values from registered
                // methods. Only the latter receive the object as their leading argument.
                if let Expr::Get(object, member) = callee.as_ref() {
                    let object = self
                        .evaluate_ast(object, env)
                        .with_context(|| {
                            format!(
                                "Résolution du receveur de `{}`",
                                diagnostic_expression(callee)
                            )
                        })?;
                    return self.call_member(
                        object,
                        EvalValue::String(member.clone()),
                        &arguments,
                        env,
                    )
                    .with_context(|| format!("Appel `{}`", diagnostic_expression(callee)));
                }
                if let Expr::Index(object, member) = callee.as_ref() {
                    let object = self
                        .evaluate_ast(object, env)
                        .with_context(|| {
                            format!(
                                "Résolution du receveur de `{}`",
                                diagnostic_expression(callee)
                            )
                        })?;
                    let member = self
                        .evaluate_ast(member, env)
                        .with_context(|| {
                            format!(
                                "Résolution du membre de `{}`",
                                diagnostic_expression(callee)
                            )
                        })?;
                    return self
                        .call_member(object, member, &arguments, env)
                        .with_context(|| format!("Appel `{}`", diagnostic_expression(callee)));
                }

                self.evaluate_ast(callee, env)
                    .with_context(|| {
                        format!(
                            "Résolution de la fonction `{}`",
                            diagnostic_expression(callee)
                        )
                    })?
                    .call(&arguments, self, env)
                    .with_context(|| format!("Appel `{}`", diagnostic_expression(callee)))
            }
            Expr::Get(object, property) => {
                let resolved_object = self.evaluate_ast(object, env)?;
                resolved_object
                    .get_member(&EvalValue::String(property.clone()), self, env)
                    .with_context(|| {
                        format!(
                            "Accès à la propriété `{property}` de {}",
                            resolved_object.diagnostic()
                        )
                    })
            }
            Expr::Index(object, index) => {
                let resolved_object = self.evaluate_ast(object, env)?;
                let resolved_index = self.evaluate_ast(index, env)?;
                resolved_object
                    .get_member(&resolved_index, self, env)
                    .with_context(|| {
                        format!(
                            "Indexation de {} par {}",
                            resolved_object.diagnostic(),
                            resolved_index.diagnostic()
                        )
                    })
            }
            Expr::Slice { object, start, end, step } => {
                let object = self.evaluate_ast(object, env)?;
                let mut bound = |expr: &Option<Arc<Expr>>| -> Result<Option<i64>> {
                    match expr {
                        None => Ok(None),
                        Some(expr) => match self.evaluate_ast(expr, env)? {
                            EvalValue::Null => Ok(None),
                            EvalValue::Int(value) => Ok(Some(value)),
                            other => bail!("Une borne ou un pas de tranche doit être un entier ou null, reçu {}", other.diagnostic()),
                        },
                    }
                };
                let start = bound(start)?;
                let end = bound(end)?;
                let step = bound(step)?.unwrap_or(1);
                slice_value(&object, start, end, step)
            }
        }
    }

    /// Calls a stored member or registered method with the correct receiver semantics.
    fn call_member(
        &mut self,
        object: EvalValue,
        member: EvalValue,
        arguments: &[EvalValue],
        env: &Env,
    ) -> Result<EvalValue> {
        if let Some(function) = object.direct_member(&member, self, env)? {
            return function.call(arguments, self, env);
        }

        let function = object.get_member(&member, self, env)?;
        let mut method_arguments = Vec::with_capacity(arguments.len() + 1);
        method_arguments.push(object);
        method_arguments.extend_from_slice(arguments);
        function.call(&method_arguments, self, env)
    }
    
    /// Evaluates one tale-defined function in its captured lexical environment.
    ///
    /// Parameters and `args` have highest priority. Current `objet` and `etat` bindings then
    /// override captured fallbacks, followed by captured lexical values, tale globals, formula
    /// globals, and finally unresolved caller bindings. A depth limit prevents direct or
    /// indirect recursion from overflowing the Rust stack.
    fn call_user_function(
        &mut self,
        closure: FormulaClosure,
        arguments: &[EvalValue],
        env: &Env,
    ) -> Result<EvalValue> {
        const MAX_CALL_DEPTH: usize = 32;
        let name: Arc<str> = format!(
            "fn({})",
            closure
                .definition
                .parameters
                .as_ref()
                .map(|parameters| format_patterns(parameters))
                .unwrap_or_default()
        )
        .into();
        if self.call_stack.len() >= MAX_CALL_DEPTH {
            let mut chain = self.call_stack.join(" -> ");
            if !chain.is_empty() {
                chain.push_str(" -> ");
            }
            chain.push_str(&name);
            bail!("Profondeur maximale de fonctions ({MAX_CALL_DEPTH}) dépassée: {chain}");
        }
        let FormulaClosure {
            definition: function,
            captures,
            tale_globals,
        } = closure;
        if let Some(parameters) = &function.parameters {
            if arguments.len() < parameters.len() {
                bail!(
                    "{name} exige au moins {} argument(s), mais {} ont été fournis",
                    parameters.len(),
                    arguments.len()
                );
            }
        }

        let captured_env = Env::from_arc(captures);
        let mut current_entity_env = Env::new();
        for name in ["objet", "etat"] {
            if let Some(value) = env.get(name) {
                current_entity_env.insert(name, value.clone());
            }
        }
        let current_tale_globals = match env.get("recit") {
            Some(EvalValue::Object(values)) => tale_globals
                .iter()
                .filter_map(|name| {
                    values
                        .get(name.as_ref())
                        .map(|value| (name.clone(), value.clone()))
                })
                .collect::<IndexMap<_, _>>(),
            _ => IndexMap::new(),
        };
        let tale_globals_env = Env::from_tale_globals(&current_tale_globals);
        // Globals are placed before the remaining caller environment. A local value that
        // shadowed one of them at closure creation was captured and therefore still wins.
        let globals = self.globals.clone();
        let mut local_env = Env::new()
            .with_parent(&current_entity_env)
            .with_parent(&captured_env)
            .with_parent(&tale_globals_env)
            .with_parent(&globals)
            .with_parent(env);
        local_env.insert("args", EvalValue::Array(arguments.to_vec().into()));
        if let Some(parameters) = &function.parameters {
            for (parameter, value) in parameters.iter().zip(arguments.iter()) {
                parameter.bind(value.clone(), &mut local_env)
                    .with_context(|| format!("Paramètre {parameter} de {name}"))?;
            }
        }

        self.call_stack.push(name.to_owned());
        let result = self
            .evaluate_ast(&function.body, &local_env)
            .with_context(|| {
                format!(
                    "Dans {name}, corps `{}`",
                    diagnostic_expression(&function.body)
                )
            });
        self.call_stack.pop();
        result
    }
    

    /// Selects character items, states, or tags with an ordinary tale filter.
    ///
    /// Uses the current value as the initial selection when possible. Escape is reported
    /// as `PromptCancelled` rather than as malformed input.
    fn select_character_entities(
        &mut self,
        character: &CharacterView,
        kind: &str,
        filter: Option<&EvalValue>,
    ) -> Result<EvalValue> {
        //let filter_json = filter.map(EvalValue::to_json).transpose()?;
        let base_env = character.formula_env();
        let mut selected = Vec::new();

        match kind {
            "objet" => {
                let filter = filter
                    .as_ref()
                    .map(|f| ItemFilter::from_dict(f))
                    .transpose()?;
                for item in character.items.iter() {
                    if filter
                        .as_ref()
                        .map(|filter| filter.matches_view(item, self, &base_env))
                        .transpose()?
                        .unwrap_or(true)
                    {
                        selected.push(item.resolved_object(self, &base_env)?);
                    }
                }
            }
            "etat" => {
                let filter = filter
                    .as_ref()
                    .map(|f| ConditionFilter::from_dict(f))
                    .transpose()?;
                for condition in character.conditions.iter() {
                    if filter
                        .as_ref()
                        .map(|filter| filter.matches_view(condition, self, &base_env))
                        .transpose()?
                        .unwrap_or(true)
                    {
                        selected.push(condition.resolved_object(self, &base_env)?);
                    }
                }
            }
            "tag" => {
                let filter = filter.as_ref().map(|f| TagFilter::from_dict(f)).transpose()?;
                for (tag, value) in character.tags.iter() {
                    if filter
                        .as_ref()
                        .map(|filter| filter.matches_with_env(tag, value, self, &base_env))
                        .transpose()?
                        .unwrap_or(true)
                    {
                        let mut tag_object = IndexMap::new();
                        tag_object.insert("tag".into(), EvalValue::String(tag.clone()));
                        tag_object.insert("valeur".into(), value.clone());
                        selected.push(EvalValue::Object(Arc::from(tag_object)));
                    }
                }
            }
            _ => bail!("Type de sélection inconnu: {kind}; attendu: objet, etat ou tag"),
        }
        Ok(EvalValue::Array(Arc::from(selected)))
    }

    /// Expands one comprehension source into local environments with bound loop variables.
    fn comprehension_bindings<'a>(
        &mut self,
        comprehension: &Comprehension,
        env: &Env<'a>,
    ) -> Result<Vec<Env<'a>>> {
        let iterable = self.evaluate_ast(&comprehension.iterable, env)?;
        let rows = comprehension_rows(iterable, comprehension.targets.len())?;
        rows.into_iter()
            .map(|row| {
                let mut local_env = env.clone();
                for (target, value) in comprehension.targets.iter().zip(row.iter()) {
                    if target.as_ref() != "_" {
                        local_env.insert(target.clone(), value.clone());
                    }
                }
                Ok(local_env)
            })
            .collect()
    }
}

/// Validates the number of arguments supplied to a formula function or method.
fn expect_arity(name: &str, arguments: &[EvalValue], expected: usize) -> Result<()> {
    if arguments.len() != expected {
        bail!(
            "{name} exige {expected} argument(s), mais {} ont été fournis",
            arguments.len()
        )
    }
    Ok(())
}

/// Raises one numeric value to another numeric power.
fn power_values(base: &EvalValue, exponent: &EvalValue) -> Result<EvalValue> {
    if let (EvalValue::Int(base), EvalValue::Int(exponent)) = (base, exponent) {
        if *exponent >= 0 {
            let exponent = u32::try_from(*exponent)
                .map_err(|_| anyhow!("L'exposant de pow est trop grand"))?;
            return base
                .checked_pow(exponent)
                .map(EvalValue::Int)
                .ok_or_else(|| anyhow!("Dépassement numérique dans pow"));
        }
    }

    let base = base.to_f64()?;
    let exponent = exponent.to_f64()?;
    if base == 0.0 && exponent < 0.0 {
        bail!("pow ne peut pas élever zéro à une puissance négative")
    }
    if base < 0.0 && exponent.fract() != 0.0 {
        bail!("pow ne prend pas en charge les résultats complexes")
    }
    let result = base.powf(exponent);
    if result.is_nan() {
        bail!("Résultat numérique invalide dans pow")
    }
    Ok(EvalValue::Float(result))
}

/// Sums numeric list values while preserving an integral result when possible.
fn sum_values(values: &[EvalValue]) -> Result<EvalValue> {
    let mut integer_total = 0_i64;
    let mut float_total = 0.0_f64;
    let mut has_float = false;
    for value in values {
        match value {
            EvalValue::Int(value) => {
                if has_float {
                    float_total += *value as f64;
                } else {
                    integer_total = integer_total
                        .checked_add(*value)
                        .ok_or_else(|| anyhow!("Dépassement numérique dans sum"))?;
                }
            }
            EvalValue::Bool(value) => {
                let value = if *value { 1_i64 } else { 0_i64 };
                if has_float {
                    float_total += value as f64;
                } else {
                    integer_total = integer_total
                        .checked_add(value)
                        .ok_or_else(|| anyhow!("Dépassement numérique dans sum"))?;
                }
            }
            EvalValue::Float(value) => {
                if !has_float {
                    float_total = integer_total as f64;
                    has_float = true;
                }
                float_total += value;
            }
            value => bail!(
                "sum ne peut pas additionner {}: chaque élément de la liste doit être numérique",
                value.diagnostic()
            ),
        }
    }
    if has_float {
        Ok(EvalValue::Float(float_total))
    } else {
        Ok(EvalValue::Int(integer_total))
    }
}

/// Multiplies numeric list values while preserving an integral result when possible.
fn mul_values(values: &[EvalValue]) -> Result<EvalValue> {
    let mut integer_total = 1_i64;
    let mut float_total = 1.0_f64;
    let mut has_float = false;
    for value in values {
        match value {
            EvalValue::Int(value) => {
                if has_float {
                    float_total *= *value as f64;
                } else {
                    integer_total = integer_total
                        .checked_mul(*value)
                        .ok_or_else(|| anyhow!("Dépassement numérique dans mul"))?;
                }
            }
            EvalValue::Bool(value) => {
                let value = if *value { 1_i64 } else { 0_i64 };
                if has_float {
                    float_total *= value as f64;
                } else {
                    integer_total = integer_total
                        .checked_mul(value)
                        .ok_or_else(|| anyhow!("Dépassement numérique dans mul"))?;
                }
            }
            EvalValue::Float(value) => {
                if !has_float {
                    float_total = integer_total as f64;
                    has_float = true;
                }
                float_total *= value;
            }
            value => bail!(
                "mul ne peut pas multiplier {}: chaque élément de la liste doit être numérique",
                value.diagnostic()
            ),
        }
    }
    if has_float {
        Ok(EvalValue::Float(float_total))
    } else {
        Ok(EvalValue::Int(integer_total))
    }
}

fn indexof_str(haystack: &str, needle: &str) -> EvalValue {
    if let Some(index) = haystack.find(needle) {
        EvalValue::Int(index as i64)
    } else {
        EvalValue::Null
    }
}
fn indexof_object(haystack: &IndexMap<Arc<str>, EvalValue>, needle: &EvalValue) -> EvalValue {
    for (key, value) in haystack.iter() {
        if value == needle {
            return EvalValue::String(key.clone());
        }
    }
    EvalValue::Null
}
fn zip_arrays(arrays: &[Arc<[EvalValue]>]) -> EvalValue {
    let mut res = Vec::new();
    let mut its = arrays.iter().map(|array| array.iter()).collect::<Vec<_>>();
    loop {
        let mut row = Vec::new();
        for it in &mut its {
            if let Some(value) = it.next() {
                row.push(value.clone());
            } else {
                return EvalValue::Array(res.into());
            }
        }
        res.push(EvalValue::Array(row.into()));
    }
}

fn get_path(obj: &EvalValue, path: &[&str], default: Option<EvalValue>) -> Result<EvalValue> {
    if path.is_empty() {
        return Ok(obj.clone());
    }
    match (obj, path[0].parse::<usize>()) {
        (EvalValue::Array(values), Ok(index)) => {
            if let Some(value) = values.get(index) {
                get_path(value, &path[1..], default)
            } else {
                default.ok_or_else(|| anyhow!("Index de liste hors limites: {index}"))
            }
        }
        (EvalValue::Object(values), Err(_)) => {
            if let Some(value) = values.get(path[0]) {
                get_path(value, &path[1..], default)
            } else {
                default.ok_or_else(|| anyhow!("Clé de dictionnaire inconnue: {}", path[0]))
            }
        }
        _ => bail!(
            "Impossible d'accéder au segment `{}` dans {}: seuls une liste indexée par un entier ou un dictionnaire indexé par une chaîne peuvent être parcourus",
            path[0],
            obj.diagnostic()
        ),
    }
}
fn has_path(obj: &EvalValue, path: &[&str]) -> bool {
    if path.is_empty() {
        return true;
    }
    match (obj, path[0].parse::<usize>()) {
        (EvalValue::Array(values), Ok(index)) => values
            .get(index)
            .map_or(false, |value| has_path(value, &path[1..])),
        (EvalValue::Object(values), Err(_)) => values
            .get(path[0])
            .map_or(false, |value| has_path(value, &path[1..])),
        _ => false,
    }
}
fn shuffle(values: &mut [EvalValue], rng: &mut impl Rng) {
    for i in (1..values.len()).rev() {
        let j = rng.gen_range(0..=i);
        values.swap(i, j);
    }
}
fn minmax(values: &[EvalValue], ord: Ordering) -> Result<EvalValue> {
    let mut iter = values.iter();
    let first = iter
        .next()
        .ok_or_else(|| anyhow!("min/max exige au moins un argument"))?;
    let mut best = first.clone();
    for value in iter {
        if value.partial_cmp(&best) == Some(ord) {
            best = value.clone();
        }
    }
    Ok(best)
}

/// Parses a formula expression without evaluating it.
///
/// An optional outer `${...}` wrapper is accepted. This is used while loading declared
/// functions and by authoring tools that need syntax validation without a runtime environment.
///
/// # Errors
///
/// Returns tokenizer or parser errors for malformed input.
pub fn validate_formula_syntax(expression: &str) -> Result<()> {
    let expression = full_formula(expression).unwrap_or(expression);
    let expression = strip_formula_dollars(expression);
    let tokens = Tokenizer::new(&expression)
        .tokenize()
        .with_context(|| format!("Erreur lexicale dans la formule '{expression}'"))?;
    Parser::new(tokens).parse().map_err(|err| anyhow!("Erreur de syntaxe dans la formule '{expression}': {err}"))?;
    Ok(())
}

/// Returns whether a text contains at least one unescaped formula substitution.
pub fn has_formula_substitution(text: &str) -> bool {
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '$' {
            index += 1;
            continue;
        }
        if chars.get(index + 1) == Some(&'$') {
            index += 2;
            continue;
        }
        if chars.get(index + 1) == Some(&'{') {
            return true;
        }
        if chars.get(index + 1).copied().map(is_word).unwrap_or(false) {
            return true;
        }
        index += 1;
    }
    false
}

/// Extracts the expression from a string that consists entirely of one formula.
///
///
/// Returns the inner expression only when `text` consists of one complete `${...}` formula.
pub fn full_formula(text: &str) -> Option<&str> {
    if !text.starts_with("${") {
        return None;
    }
    let chars = text.chars().collect::<Vec<_>>();
    let end = find_formula_end(&chars, 2)?;
    if end + 1 != chars.len() || end == 2 {
        return None;
    }
    Some(&text[2..text.len() - 1])
}

/// Finds the closing quote of a string whose entire contents are one `${...}` formula.
///
/// Filter dictionaries commonly use the same quote character for the outer
/// formula-language string and for literals inside the embedded formula, such
/// as `'${id == 'potion'}'`. The `${...}` delimiters make the intended outer
/// boundary unambiguous, so the tokenizer can preserve the complete formula
/// string instead of stopping at its first inner quote.
fn quoted_formula_literal_end(chars: &[char], quote_start: usize) -> Option<usize> {
    let quote = *chars.get(quote_start)?;
    if !matches!(quote, '\'' | '"')
        || chars.get(quote_start + 1) != Some(&'$')
        || chars.get(quote_start + 2) != Some(&'{')
    {
        return None;
    }

    let formula_end = find_formula_end(chars, quote_start + 3)?;
    (chars.get(formula_end + 1) == Some(&quote)).then_some(formula_end + 1)
}

/// Finds the closing delimiter of an inline formula while ignoring dictionary braces and strings.
fn find_formula_end(chars: &[char], start: usize) -> Option<usize> {
    let mut brace_depth = 0_usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in chars.iter().copied().enumerate().skip(start) {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '{' => brace_depth += 1,
            '}' if brace_depth == 0 => return Some(index),
            '}' => brace_depth -= 1,
            _ => {}
        }
    }
    None
}

/// Converts an iterable value into rows suitable for comprehension variable binding.
fn comprehension_rows(iterable: EvalValue, target_count: usize) -> Result<Arc<[Arc<[EvalValue]>]>> {
    match (iterable, target_count) {
        (EvalValue::Array(values), 1) => Ok(values.into_iter().map(|value| vec![value.clone()].into()).collect::<Arc<[Arc<[EvalValue]>]>>()),
        (EvalValue::Array(values), expected) => values
            .iter()
            .map(|value| match value {
                EvalValue::Array(row) if row.len() == expected => Ok(row.clone()),
                EvalValue::Array(row) => bail!(
                    "Impossible de décomposer {} valeur(s) dans {expected} variable(s)",
                    row.len()
                ),
                _ => bail!("La décomposition d'une liste exige des sous-listes"),
            })
            .collect(),
        (EvalValue::Object(values), 1) => Ok(values
            .keys()
            .map(|key| vec![EvalValue::String(key.clone())].into())
            .collect::<Arc<[Arc<[EvalValue]>]>>()),
        (EvalValue::Object(values), 2) => Ok(values
            .iter()
            .map(|(key, value)| vec![EvalValue::String(key.clone()), value.clone()].into())
            .collect::<Arc<[Arc<[EvalValue]>]>>()),
        (EvalValue::Object(_), expected) => bail!(
            "Un dictionnaire peut être parcouru avec une variable (clé) ou deux variables (clé, valeur), pas {expected}"
        ),
        (EvalValue::String(value), 1) => Ok(value
            .chars()
            .map(|character| vec![EvalValue::String(Arc::from(character.to_string()))].into())
            .collect::<Arc<[Arc<[EvalValue]>]>>()),
        (EvalValue::String(_), _) => {
            bail!("Une chaîne ne peut être parcourue qu'avec une variable")
        }
        (value, _) => bail!(
            "Une compréhension ne peut pas parcourir {}: une liste, un dictionnaire ou une chaîne est requis",
            value.diagnostic()
        ),
    }
}

/// Rounds a number using Python-compatible halfway-to-even semantics.
fn python_round(value: f64, digits: i64) -> Result<f64> {
    let digits = i32::try_from(digits).map_err(|_| anyhow!("Nombre de décimales hors limites"))?;
    let factor = 10_f64.powi(digits);
    if !factor.is_finite() || factor == 0.0 {
        bail!("Nombre de décimales hors limites")
    }
    let scaled = value * factor;
    if !scaled.is_finite() {
        bail!("Valeur impossible à arrondir")
    }
    let floor = scaled.floor();
    let fraction = scaled - floor;
    let tolerance = f64::EPSILON * scaled.abs().max(1.0) * 4.0;
    let rounded = if (fraction - 0.5).abs() <= tolerance {
        if floor.rem_euclid(2.0) == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    } else {
        scaled.round()
    };
    Ok(rounded / factor)
}

/// Removes optional formula sigils from a variable reference.
fn strip_formula_dollars(expression: &str) -> String {
    let chars: Vec<char> = expression.chars().collect();
    let mut output = String::new();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    while index < chars.len() {
        let character = chars[index];
        if let Some(active_quote) = quote {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if let Some(quote_end) = quoted_formula_literal_end(&chars, index) {
                output.extend(chars[index..=quote_end].iter());
                index = quote_end + 1;
                continue;
            }
            quote = Some(character);
            output.push(character);
            index += 1;
            continue;
        }
        if character == '$'
            && chars
                .get(index + 1)
                .is_some_and(|character| is_word(*character))
        {
            index += 1;
            continue;
        }
        output.push(character);
        index += 1;
    }
    output
}

/// Returns whether a character may participate in an identifier.
fn is_word(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

/// Extracts a list or Unicode-character string slice with clamped, exclusive bounds.
fn slice_value(
    value: &EvalValue,
    start: Option<i64>,
    end: Option<i64>,
    step: i64,
) -> Result<EvalValue> {
    if step == 0 {
        bail!("Le pas d'une tranche ne peut pas être zéro");
    }
    let characters;
    let length = match value {
        EvalValue::Array(values) => {
            characters = Vec::new();
            values.len()
        }
        EvalValue::String(text) => {
            characters = text.chars().collect::<Vec<_>>();
            characters.len()
        }
        other => bail!(
            "Une tranche exige une liste ou une chaîne, reçu {}",
            other.diagnostic()
        ),
    } as i128;
    let reverse = step < 0;
    // i128 avoids overflow for extreme i64 bounds and steps, including i64::MIN.
    let normalize = |bound: i64| {
        let bound = bound as i128;
        let bound = if bound < 0 { length + bound } else { bound };
        if reverse {
            bound.clamp(-1, length - 1)
        } else {
            bound.clamp(0, length)
        }
    };
    let mut index = start
        .map(normalize)
        .unwrap_or(if reverse { length - 1 } else { 0 });
    let stop = end
        .map(normalize)
        .unwrap_or(if reverse { -1 } else { length });
    let mut values = Vec::new();
    let mut text = String::new();
    while if reverse { index > stop } else { index < stop } {
        match value {
            EvalValue::Array(source) => values.push(source[index as usize].clone()),
            EvalValue::String(_) => text.push(characters[index as usize]),
            _ => unreachable!(),
        }
        index += step as i128;
    }
    Ok(match value {
        EvalValue::Array(_) => EvalValue::Array(values.into()),
        _ => EvalValue::String(text.into()),
    })
}

/// Normalizes a possibly negative index against a collection length.
fn normalize_index(index: i64, length: usize) -> Result<usize> {
    let requested = index;
    let length = length as i64;
    let index = if index < 0 { length + index } else { index };
    if index < 0 || index >= length {
        if length == 0 {
            bail!("Index {requested} hors limites: la séquence est vide")
        }
        bail!(
            "Index {requested} hors limites: la séquence contient {length} élément(s), donc les index valides vont de {} à {}",
            -length,
            length - 1
        )
    }
    Ok(index as usize)
}

/// Applies one binary operator to two already evaluated operands.
///
/// # Errors
///
/// Returns an error for unsupported operand types, division by zero, invalid shifts, or other
/// operator-specific domain failures.
fn evaluate_binary(
    operator: &BinaryOperator,
    left: EvalValue,
    right: EvalValue,
) -> Result<EvalValue> {
    match operator {
        BinaryOperator::Add => add_values(left, right),
        BinaryOperator::Subtract => {
            numeric_binary(left, right, |a, b| a - b, |a, b| a.checked_sub(b))
        }
        BinaryOperator::Multiply => multiply_values(left, right),
        BinaryOperator::Divide => {
            let divisor = right
                .to_f64()
                .with_context(|| format!("Le diviseur est {}", right.diagnostic()))?;
            if divisor == 0.0 {
                bail!(
                    "Impossible de diviser {} par {}: division par zéro",
                    left.diagnostic(),
                    right.diagnostic()
                )
            }
            let dividend = left
                .to_f64()
                .with_context(|| format!("Le dividende est {}", left.diagnostic()))?;
            Ok(EvalValue::Float(dividend / divisor))
        }
        BinaryOperator::FloorDivide => floor_divide(left, right),
        BinaryOperator::Modulo => modulo_values(left, right),
        BinaryOperator::Equal => Ok(EvalValue::Bool(values_equal(&left, &right))),
        BinaryOperator::NotEqual => Ok(EvalValue::Bool(!values_equal(&left, &right))),
        BinaryOperator::Less => Ok(EvalValue::Bool(
            compare_values(&left, &right)? == Ordering::Less,
        )),
        BinaryOperator::LessEqual => Ok(EvalValue::Bool(matches!(
            compare_values(&left, &right)?,
            Ordering::Less | Ordering::Equal
        ))),
        BinaryOperator::Greater => Ok(EvalValue::Bool(
            compare_values(&left, &right)? == Ordering::Greater,
        )),
        BinaryOperator::GreaterEqual => Ok(EvalValue::Bool(matches!(
            compare_values(&left, &right)?,
            Ordering::Greater | Ordering::Equal
        ))),
        BinaryOperator::In => Ok(EvalValue::Bool(contains_value(&right, &left)?)),
        BinaryOperator::NotIn => Ok(EvalValue::Bool(!contains_value(&right, &left)?)),
        BinaryOperator::BitAnd => bitwise_binary(left, right, "&", |a, b| a & b, None),
        BinaryOperator::BitXor => bitwise_binary(left, right, "^", |a, b| a ^ b, None),
        BinaryOperator::BitOr => bitwise_binary(left, right, "|", |a, b| a | b, None),
        BinaryOperator::Xor => Ok(EvalValue::Bool(left.truthy() != right.truthy())),
        BinaryOperator::And | BinaryOperator::Or | BinaryOperator::Coalesce => unreachable!("traité avec court-circuit"),
        BinaryOperator::LeftShift => bitwise_binary(
            left, 
            right, 
            "<<", 
            |a, b| if b > u32::MAX as i64 { 0 } else { a.unbounded_shl(b as u32) },
            Some(&|_, b| b >= 0),
        ),
        BinaryOperator::RightShift => bitwise_binary(
            left, 
            right, 
            ">>", 
            |a, b| if b > u32::MAX as i64 { if a >= 0 { 0 } else { -1 } } else { a.unbounded_shr(b as u32) },
            Some(&|_, b| b >= 0),
        ),
        BinaryOperator::LogicRightShift => bitwise_binary(
            left,
            right,
            ">>>",
            |a, b| if b > u32::MAX as i64 { 0 } else { (a as u64).unbounded_shr(b as u32) as i64 },
            Some(&|_, b| b >= 0),
        ),
    }
}
/// Applies formula-language `+` semantics to two already evaluated operands.
///
/// # Errors
///
/// Returns an error for unsupported type pairs.
fn add_values(left: EvalValue, right: EvalValue) -> Result<EvalValue> {
    left.try_add(&right)
}
fn range_values(args: &[&EvalValue]) -> Result<EvalValue> {
    let (start, stop, step) = match args.len() {
        1 => (0_i64, args[0].to_i64()?, 1_i64),
        2 => (args[0].to_i64()?, args[1].to_i64()?, 1_i64),
        3 => (args[0].to_i64()?, args[1].to_i64()?, args[2].to_i64()?),
        _ => bail!("range exige 1 à 3 arguments"),
    };
    if step == 0 {
        bail!("range ne peut pas avoir un pas de zéro")
    }
    let mut values = Vec::new();
    let mut current = start;
    if step > 0 {
        while current < stop {
            values.push(EvalValue::Int(current));
            current += step;
        }
    } else {
        while current > stop {
            values.push(EvalValue::Int(current));
            current += step;
        }
    }
    Ok(EvalValue::Array(values.into()))
}
fn pop_values(array: &[&EvalValue], i: Option<usize>) -> Result<EvalValue> {
    if array.is_empty() {
        bail!("pop ne peut pas être appelé sur une liste vide")
    }
    let index = i.unwrap_or(array.len() - 1);
    if index >= array.len() {
        bail!("Index de liste hors limites: {index}")
    }
    let mut values = array.iter().map(|v| (*v).clone()).collect::<Vec<_>>();
    let popped = values.remove(index);
    Ok(EvalValue::Array(
        vec![EvalValue::Array(values.into()), popped].into(),
    ))
}

/// Multiplies numeric values or repeats supported sequence values.
fn multiply_values(left: EvalValue, right: EvalValue) -> Result<EvalValue> {
    left.try_mul(&right)
}

/// Applies a numeric binary operator after coercing compatible operands.
fn numeric_binary(
    left: EvalValue,
    right: EvalValue,
    float_operation: impl FnOnce(f64, f64) -> f64,
    int_operation: impl FnOnce(i64, i64) -> Option<i64>,
) -> Result<EvalValue> {
    match (&left, &right) {
        (EvalValue::Int(left), EvalValue::Int(right)) => int_operation(*left, *right)
            .map(EvalValue::Int)
            .ok_or_else(|| anyhow!("Dépassement numérique")),
        _ => Ok(EvalValue::Float(float_operation(
            left.to_f64()?,
            right.to_f64()?,
        ))),
    }
}

/// Performs Python-style floor division and reports division by zero.
fn floor_divide(left: EvalValue, right: EvalValue) -> Result<EvalValue> {
    left.try_floor_div(&right)
}

/// Computes a Python-style remainder and reports invalid operands.
fn modulo_values(left: EvalValue, right: EvalValue) -> Result<EvalValue> {
    left.try_mod(&right)
}

/// Applies one bitwise operation to two integer operands.
fn bitwise_binary(
    left: EvalValue,
    right: EvalValue,
    symbol: &str,
    operation: impl FnOnce(i64, i64) -> i64,
    constraint: Option<&dyn Fn(i64, i64) -> bool>,
) -> Result<EvalValue> {
    let left = if let EvalValue::Bool(val) = left {
        EvalValue::Int(if val { 1 } else { 0 })
    } else {
        left
    };
    let right = if let EvalValue::Bool(val) = right {
        EvalValue::Int(if val { 1 } else { 0 })
    } else {
        right
    };
    let (EvalValue::Int(left), EvalValue::Int(right)) = (&left, &right) else {
        bail!(
            "Impossible d'appliquer l'opérateur bit à bit {symbol} à {} et {}: deux entiers sont requis",
            left.diagnostic(),
            right.diagnostic()
        )
    };
    if let Some(constraint) = constraint {
        if !constraint(*left, *right) {
            bail!(
                "Les valeurs {} et {} ne respectent pas la contrainte pour l'opérateur bit à bit {symbol}",
                left,
                right
            )
        }
    }
    Ok(EvalValue::Int(operation(*left, *right)))
}

/// Compares evaluated values using formula-language equality semantics.
fn values_equal(left: &EvalValue, right: &EvalValue) -> bool {
    left == right
}

/// Orders compatible evaluated values for comparison operators.
fn compare_values(left: &EvalValue, right: &EvalValue) -> Result<Ordering> {
    left.partial_cmp(right).ok_or_else(|| {
        anyhow!(
            "Impossible de comparer {} et {}: ces types n'ont pas d'ordre commun",
            left.diagnostic(),
            right.diagnostic()
        )
    })
}

/// Returns whether a value participates in Python-style numeric operations.
#[allow(dead_code)]
fn is_python_number(value: &EvalValue) -> bool {
    matches!(
        value,
        EvalValue::Bool(_) | EvalValue::Int(_) | EvalValue::Float(_)
    )
}

/// Tests membership using formula-language equality semantics.
fn contains_value(container: &EvalValue, needle: &EvalValue) -> Result<bool> {
    match container {
        EvalValue::Array(values) => Ok(values.iter().any(|value| values_equal(value, needle))),
        EvalValue::String(value) => match needle {
            EvalValue::String(needle) => Ok(value.contains(needle.as_ref())),
            _ => bail!(
                "Impossible de rechercher {} dans {}: une chaîne ne peut contenir qu'une autre chaîne",
                needle.diagnostic(),
                container.diagnostic()
            ),
        },
        EvalValue::Object(values) => match needle {
            EvalValue::String(needle) => Ok(values.contains_key(needle)),
            _ => bail!(
                "Impossible de rechercher {} dans {}: une clé de dictionnaire doit être une chaîne",
                needle.diagnostic(),
                container.diagnostic()
            ),
        },
        _ => bail!(
            "Impossible de rechercher {} dans {}: l'opérateur `in` exige une liste, une chaîne ou un dictionnaire comme conteneur",
            needle.diagnostic(),
            container.diagnostic()
        ),
    }
}

/// Returns the current value except in the case it is a formula string, in which case it is parsed and transformed into a no argument function.
pub fn value_or_formula(
    value: EvalValue,
) -> Result<EvalValue> {
    match value {
        EvalValue::String(s) => {
            match full_formula(&s)  {
                None => Ok(EvalValue::String(s)),
                Some(inner) => {
                    let expr = Arc::from(strip_formula_dollars(inner).as_str());
                    let tokens = Tokenizer::new(&expr)
                        .tokenize()
                        .with_context(|| format!("Erreur lexicale dans la formule '{expr}'"))?;
                    let ast = Parser::new(tokens).parse().map_err(|err| anyhow!("Erreur de syntaxe dans la formule '{expr}': {err}"))?;
                    Ok(EvalValue::Func(Arc::new(Function::User(FormulaClosure { 
                        definition: FormulaFunction { 
                            parameters: None,
                            body: Arc::from(ast),
                        }, 
                        captures: IndexMap::new().into(), 
                        tale_globals: IndexSet::new().into(),
                    }))))
                },
            }
        },
        _ => Ok(value),
    }
}

/// Split a string into a vector of alternating literal and formula segments, preserving the original text.
/// and returns a function that will evaluate the formula segments and concatenate the results with the literal segments.
/// Or returns a str if there are no formula segments.
pub fn split_fmt_str(text: &str) -> Result<EvalValue>{
    let mut segments = Vec::new();
    let chars = text.chars().collect::<Vec<_>>();
    let mut current_segment = String::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] != '$' {
            current_segment.push(chars[index]);
            index += 1;
            continue;
        }

        if chars.get(index + 1) == Some(&'$') {
            current_segment.push('$');
            index += 2;
            continue;
        }

        if chars.get(index + 1) == Some(&'{') {
            let Some(end) = find_formula_end(&chars, index + 2) else {
                current_segment.push('$');
                index += 1;
                continue;
            };
            let expression: String = chars[index + 2..end].iter().collect();
            let expression = strip_formula_dollars(&expression);
            
            if !current_segment.is_empty() {
                segments.push(EvalValue::String(Arc::from(current_segment)));
                current_segment = String::new();
            }
            let tokens = Tokenizer::new(&expression)
                .tokenize()
                .with_context(|| format!("Erreur lexicale dans la formule '{expression}'"))?;
            let ast = Parser::new(tokens).parse().map_err(|err| anyhow!("Erreur de syntaxe dans la formule '{expression}': {err}"))?;
            segments.push(EvalValue::Func(Arc::new(Function::User(FormulaClosure {
                definition: FormulaFunction {
                    parameters: None,
                    body: Arc::from(ast),
                },
                captures: IndexMap::new().into(),
                tale_globals: IndexSet::new().into(),
            }))));

            index = end + 1;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && is_word(chars[end]) {
            end += 1;
        }
        if end == index + 1 {
            current_segment.push('$');
            index += 1;
            continue;
        }

        let name: String = chars[index + 1..end].iter().collect();
        let formulastr = format!("{name}");
        let tokens = Tokenizer::new(&formulastr)
            .tokenize()
            .with_context(|| format!("Erreur lexicale dans la formule '{formulastr}'"))?;
        let ast = Parser::new(tokens).parse().map_err(|err| anyhow!("Erreur de syntaxe dans la formule '{formulastr}': {err}"))?;
        let func = EvalValue::Func(Arc::new(Function::User(FormulaClosure {
            definition: FormulaFunction {
                parameters: None,
                body: Arc::from(ast),
            },
            captures: IndexMap::new().into(),
            tale_globals: IndexSet::new().into(),
        })));
        if !current_segment.is_empty() {
            segments.push(EvalValue::String(Arc::from(current_segment)));
            current_segment = String::new();
        }
        segments.push(func);
        index = end;
    }

    if !current_segment.is_empty() {
        segments.push(EvalValue::String(Arc::from(current_segment)));
    }

    if segments.iter().all(|s| if let EvalValue::String(_) = s {true} else {false}) {
        let concatenated = segments.iter().map(|s| if let EvalValue::String(s) = s {s.clone()} else {Arc::from("")}).collect::<Vec<_>>().join("");
        //println!("Resolved string: {}", concatenated);
        Ok(EvalValue::String(Arc::from(concatenated)))
    } else {
        if segments.len() == 1 { // If there is only one segment, return it directly without creating a function.
            //println!("Resolved single segment: {}", segments[0]);
            return Ok(segments[0].clone());
        }
        let segments = EvalValue::Array(Arc::from(segments));
        let body = Expr::Call(
            Arc::new(Expr::Get(
                Arc::new(Expr::Literal(segments)),
                "fmt".into(),
            )),
            Arc::from([]),
        );
        let func = EvalValue::Func(Arc::new(Function::User(FormulaClosure {
            definition: FormulaFunction {
                parameters: None,
                body: Arc::from(body),
            },
            captures: IndexMap::new().into(),
            tale_globals: IndexSet::new().into(),
        })));
        //println!("Resolved function: {}", func);
        Ok(func)
    }
}

/// Abstract syntax tree node for a parsed formula expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(EvalValue),
    Variable(Arc<str>),
    /// Anonymous user-defined function created by `fn ... -> ...`.
    Function(FormulaFunction),
    Let {
        bindings: Arc<[(BindingPattern, Expr)]>,
        body: Arc<Expr>,
    },
    Array(Arc<[Expr]>),
    Object(Arc<[(Expr, Expr)]>),
    ListComprehension {
        value: Arc<Expr>,
        comprehension: Comprehension,
    },
    ObjectComprehension {
        key: Arc<Expr>,
        value: Arc<Expr>,
        comprehension: Comprehension,
    },
    Unary(UnaryOperator, Arc<Expr>),
    Binary(Arc<Expr>, BinaryOperator, Arc<Expr>),
    CompareChain(Arc<Expr>, Arc<[(BinaryOperator, Expr)]>),
    Ternary {
        then_expression: Arc<Expr>,
        condition: Arc<Expr>,
        else_expression: Arc<Expr>,
    },
    Call(Arc<Expr>, Arc<[Expr]>),
    Get(Arc<Expr>, Arc<str>),
    Index(Arc<Expr>, Arc<Expr>),
    Slice {
        object: Arc<Expr>,
        start: Option<Arc<Expr>>,
        end: Option<Arc<Expr>>,
        step: Option<Arc<Expr>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprSerializer {
    Literal(EvalValueSerializer),
    Variable(String),
    /// Anonymous user-defined function created by `fn ... -> ...`.
    Function(FormulaFunctionSerializer),
    Let {
        bindings: Box<[(BindingPattern, ExprSerializer)]>,
        body: Box<ExprSerializer>,
    },
    Array(Box<[ExprSerializer]>),
    Object(Box<[(ExprSerializer, ExprSerializer)]>),
    ListComprehension {
        value: Box<ExprSerializer>,
        comprehension: ComprehensionSerializer,
    },
    ObjectComprehension {
        key: Box<ExprSerializer>,
        value: Box<ExprSerializer>,
        comprehension: ComprehensionSerializer,
    },
    Unary(UnaryOperator, Box<ExprSerializer>),
    Binary(Box<ExprSerializer>, BinaryOperator, Box<ExprSerializer>),
    CompareChain(Box<ExprSerializer>, Box<[(BinaryOperator, ExprSerializer)]>),
    Ternary {
        then_expression: Box<ExprSerializer>,
        condition: Box<ExprSerializer>,
        else_expression: Box<ExprSerializer>,
    },
    Call(Box<ExprSerializer>, Box<[ExprSerializer]>),
    Get(Box<ExprSerializer>, String),
    Index(Box<ExprSerializer>, Box<ExprSerializer>),
    Slice {
        object: Box<ExprSerializer>,
        start: Option<Box<ExprSerializer>>,
        end: Option<Box<ExprSerializer>>,
        step: Option<Box<ExprSerializer>>,
    },
}
impl From<&Expr> for ExprSerializer {
    fn from(value: &Expr) -> Self {
        match value {
            Expr::Literal(val) => Self::Literal(EvalValueSerializer::from(val)),
            Expr::Variable(name) => Self::Variable(name.to_string()),
            Expr::Function(func) => Self::Function(FormulaFunctionSerializer::from(func)),
            Expr::Let { bindings, body } => Self::Let {
                bindings: bindings
                    .iter()
                    .map(|(name, value)| (name.clone(), ExprSerializer::from(value)))
                    .collect(),
                body: Box::new(ExprSerializer::from(body.as_ref())),
            },
            Expr::Array(values) => Self::Array(values.iter().map(ExprSerializer::from).collect()),
            Expr::Object(entries) => Self::Object(
                entries
                    .iter()
                    .map(|(key, value)| (ExprSerializer::from(key), ExprSerializer::from(value)))
                    .collect(),
            ),
            Expr::ListComprehension { value, comprehension } => Self::ListComprehension {
                value: Box::new(ExprSerializer::from(value.as_ref())),
                comprehension: ComprehensionSerializer::from(comprehension),
            },
            Expr::ObjectComprehension { key, value, comprehension } => Self::ObjectComprehension {
                key: Box::new(ExprSerializer::from(key.as_ref())),
                value: Box::new(ExprSerializer::from(value.as_ref())),
                comprehension: ComprehensionSerializer::from(comprehension),
            },
            Expr::Unary(op, value) => Self::Unary(*op, Box::new(ExprSerializer::from(value.as_ref()))),
            Expr::Binary(left, op, right) => Self::Binary(
                Box::new(ExprSerializer::from(left.as_ref())),
                *op,
                Box::new(ExprSerializer::from(right.as_ref())),
            ),
            Expr::CompareChain(first, comparisons) => Self::CompareChain(
                Box::new(ExprSerializer::from(first.as_ref())),
                comparisons
                    .iter()
                    .map(|(op, value)| (*op, ExprSerializer::from(value)))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            Expr::Ternary { then_expression, condition, else_expression } => Self::Ternary {
                then_expression: Box::new(ExprSerializer::from(then_expression.as_ref())),
                condition: Box::new(ExprSerializer::from(condition.as_ref())),
                else_expression: Box::new(ExprSerializer::from(else_expression.as_ref())),
            },
            Expr::Call(callee, arguments) => Self::Call(
                Box::new(ExprSerializer::from(callee.as_ref())),
                arguments.iter().map(|arg| ExprSerializer::from(arg)).collect(),
            ),
            Expr::Get(object, key) => Self::Get(Box::new(ExprSerializer::from(object.as_ref())), key.to_string()),
            Expr::Index(object, index) => Self::Index(
                Box::new(ExprSerializer::from(object.as_ref())),
                Box::new(ExprSerializer::from(index.as_ref())),
            ),
            Expr::Slice { object, start, end, step } => Self::Slice {
                object: Box::new(Self::from(object.as_ref())),
                start: start.as_ref().map(|expr| Box::new(Self::from(expr.as_ref()))),
                end: end.as_ref().map(|expr| Box::new(Self::from(expr.as_ref()))),
                step: step.as_ref().map(|expr| Box::new(Self::from(expr.as_ref()))),
            },
        }
    }
}
impl TryFrom<ExprSerializer> for Expr {
    type Error = anyhow::Error;
    fn try_from(value: ExprSerializer) -> Result<Self, Self::Error> {
        match value {
            ExprSerializer::Literal(val) => Ok(Self::Literal(EvalValue::try_from(val)?)),
            ExprSerializer::Variable(name) => Ok(Self::Variable(Arc::from(name))),
            ExprSerializer::Function(func) => Ok(Self::Function(FormulaFunction::try_from(func)?)),
            ExprSerializer::Let { bindings, body } => Ok(Self::Let {
                bindings: bindings
                    .into_vec()
                    .into_iter()
                    .map(|(name, value)| Ok((name, Expr::try_from(value)?)))
                    .collect::<Result<Vec<_>, anyhow::Error>>()?
                    .into(),
                body: Arc::new(Expr::try_from(*body)?),
            }),
            ExprSerializer::Array(values) => Ok(Self::Array(values.into_vec().into_iter().map(|expr| Expr::try_from(expr)).collect::<Result<Vec<_>, _>>()?.into())),
            ExprSerializer::Object(entries) => Ok(Self::Object(
                entries
                    .into_vec()
                    .into_iter()
                    .map(|(key, value)| Ok((Expr::try_from(key)?, Expr::try_from(value)?)))
                    .collect::<Result<Vec<(Expr, Expr)>, anyhow::Error>>()?
                    .into(),
            )),
            ExprSerializer::ListComprehension { value, comprehension } => Ok(Self::ListComprehension {
                value: Arc::new(Expr::try_from(*value)?),
                comprehension: Comprehension::try_from(comprehension)?,
            }),
            ExprSerializer::ObjectComprehension { key, value, comprehension } => Ok(Self::ObjectComprehension {
                key: Arc::new(Expr::try_from(*key)?),
                value: Arc::new(Expr::try_from(*value)?),
                comprehension: Comprehension::try_from(comprehension)?,
            }),
            ExprSerializer::Unary(op, value) => Ok(Self::Unary(op, Arc::new(Expr::try_from(*value)?))),
            ExprSerializer::Binary(left, op, right) => Ok(Self::Binary(
                Arc::new(Expr::try_from(*left)?),
                op,
                Arc::new(Expr::try_from(*right)?),
            )),
            ExprSerializer::CompareChain(first, comparisons) => Ok(Self::CompareChain(
                Arc::new(Expr::try_from(*first)?),
                comparisons
                    .into_vec()
                    .into_iter()
                    .map(|(op, value)| Ok((op, Expr::try_from(value)?)))
                    .collect::<Result<Vec<_>, anyhow::Error>>()?
                    .into(),
            )),
            ExprSerializer::Ternary { then_expression, condition, else_expression } => Ok(Self::Ternary {
                then_expression: Arc::new(Expr::try_from(*then_expression)?),
                condition: Arc::new(Expr::try_from(*condition)?),
                else_expression: Arc::new(Expr::try_from(*else_expression)?),
            }),
            ExprSerializer::Call(callee, arguments) => Ok(Self::Call(
                Arc::new(Expr::try_from(*callee)?),
                arguments.into_vec().into_iter().map(|expr| Expr::try_from(expr)).collect::<Result<Vec<_>>>()?.into(),
            )),
            ExprSerializer::Get(object, key) => Ok(Self::Get(Arc::new(Expr::try_from(*object)?), key.into())),
            ExprSerializer::Index(object, index) => Ok(Self::Index(
                Arc::new(Expr::try_from(*object)?),
                Arc::new(Expr::try_from(*index)?),
            )),
            ExprSerializer::Slice { object, start, end, step } => Ok(Self::Slice {
                object: Arc::new(Self::try_from(*object)?),
                start: start.map(|expr| Self::try_from(*expr).map(Arc::new)).transpose()?,
                end: end.map(|expr| Self::try_from(*expr).map(Arc::new)).transpose()?,
                step: step.map(|expr| Self::try_from(*expr).map(Arc::new)).transpose()?,
            }),
        }
    }
}

/// Stores the shared loop clause used by list and dictionary comprehensions.
#[derive(Clone, Debug, PartialEq)]
pub struct Comprehension {
    targets: Arc<[Arc<str>]>,
    iterable: Arc<Expr>,
    filter: Option<Arc<Expr>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComprehensionSerializer {
    targets: Box<[String]>,
    iterable: Box<ExprSerializer>,
    filter: Option<Box<ExprSerializer>>,
}
impl From<&Comprehension> for ComprehensionSerializer {
    fn from(value: &Comprehension) -> Self {
        Self {
            targets: value.targets.iter().map(|name| name.to_string()).collect(),
            iterable: Box::new(ExprSerializer::from(value.iterable.as_ref())),
            filter: value.filter.as_ref().map(|expr| Box::new(ExprSerializer::from(expr.as_ref()))),
        }
    }
}
impl TryFrom<ComprehensionSerializer> for Comprehension {
    type Error = anyhow::Error;
    fn try_from(value: ComprehensionSerializer) -> Result<Self, Self::Error> {
        Ok(Self {
            targets: value.targets.into_vec().into_iter().map(|s| Arc::from(s)).collect(),
            iterable: Arc::new(Expr::try_from(*value.iterable)?),
            filter: value.filter.map(|expr| Ok::<Arc<Expr>, anyhow::Error>(Arc::new(Expr::try_from(*expr)?))).transpose()?,
        })
    }
}

impl FormulaFunction {
    /// Returns the distinct free names referenced by this function, in source traversal order.
    fn free_variables(&self) -> IndexSet<Arc<str>> {
        let mut bound = IndexSet::new();
        bound.insert("args".into());
        if let Some(parameters) = &self.parameters {
            for pattern in parameters.iter() { pattern.collect_names(&mut bound); }
        }
        let mut free = IndexSet::new();
        self.body.collect_free_variables(&bound, &mut free);
        free
    }
}

impl Expr {
    /// Traverses an expression and records names not bound by its surrounding lexical scopes.
    fn collect_free_variables(
        &self,
        bound: &IndexSet<Arc<str>>,
        free: &mut IndexSet<Arc<str>>,
    ) {
        match self {
            Self::Literal(_) => {}
            Self::Variable(name) => {
                if !bound.contains(name) {
                    free.insert(name.clone());
                }
            }
            Self::Function(function) => {
                // Keep the outer scope visible while introducing the nested function's own
                // parameters. This lets an outer closure retain values needed only by a child.
                let mut nested_bound = bound.clone();
                nested_bound.insert("args".into());
                if let Some(parameters) = &function.parameters {
                    for pattern in parameters.iter() { pattern.collect_names(&mut nested_bound); }
                }
                function
                    .body
                    .collect_free_variables(&nested_bound, free);
            }
            Self::Let { bindings, body } => {
                let mut local_bound = bound.clone();
                for (name, value) in bindings.iter() {
                    value.collect_free_variables(&local_bound, free);
                    name.collect_names(&mut local_bound);
                }
                body.collect_free_variables(&local_bound, free);
            }
            Self::Array(values) => {
                for value in values.iter() {
                    value.collect_free_variables(bound, free);
                }
            }
            Self::Object(entries) => {
                for (key, value) in entries.iter() {
                    key.collect_free_variables(bound, free);
                    value.collect_free_variables(bound, free);
                }
            }
            Self::ListComprehension {
                value,
                comprehension,
            } => {
                comprehension
                    .iterable
                    .collect_free_variables(bound, free);
                let mut local_bound = bound.clone();
                local_bound.extend(
                    comprehension.targets.iter().filter(|name| name.as_ref() != "_").cloned(),
                );
                if let Some(filter) = &comprehension.filter {
                    filter.collect_free_variables(&local_bound, free);
                }
                value.collect_free_variables(&local_bound, free);
            }
            Self::ObjectComprehension {
                key,
                value,
                comprehension,
            } => {
                comprehension
                    .iterable
                    .collect_free_variables(bound, free);
                let mut local_bound = bound.clone();
                local_bound.extend(
                    comprehension.targets.iter().filter(|name| name.as_ref() != "_").cloned(),
                );
                if let Some(filter) = &comprehension.filter {
                    filter.collect_free_variables(&local_bound, free);
                }
                key.collect_free_variables(&local_bound, free);
                value.collect_free_variables(&local_bound, free);
            }
            Self::Unary(_, value) => value.collect_free_variables(bound, free),
            Self::Binary(left, _, right) => {
                left.collect_free_variables(bound, free);
                right.collect_free_variables(bound, free);
            }
            Self::CompareChain(first, comparisons) => {
                first.collect_free_variables(bound, free);
                for (_, value) in comparisons.iter() {
                    value.collect_free_variables(bound, free);
                }
            }
            Self::Ternary {
                then_expression,
                condition,
                else_expression,
            } => {
                then_expression.collect_free_variables(bound, free);
                condition.collect_free_variables(bound, free);
                else_expression.collect_free_variables(bound, free);
            }
            Self::Call(callee, arguments) => {
                callee.collect_free_variables(bound, free);
                for argument in arguments.iter() {
                    argument.collect_free_variables(bound, free);
                }
            }
            Self::Get(object, _) => object.collect_free_variables(bound, free),
            Self::Index(object, index) => {
                object.collect_free_variables(bound, free);
                index.collect_free_variables(bound, free);
            }
            Self::Slice { object, start, end, step } => {
                object.collect_free_variables(bound, free);
                for expr in [start, end, step].into_iter().flatten() {
                    expr.collect_free_variables(bound, free);
                }
            }
        }
    }
}

impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Literal(val) => write!(f, "{val}"),
            Self::Variable(name) => write!(f, "{name}"),
            Self::Function(func) => write!(f, "fn {} -> ({})", func.parameters.as_ref().map_or("".to_string(), |params| format_patterns(params)), func.body),
            Self::Let {bindings, body} => {
                write!(f, "let {} for ({})",
                    bindings.iter().map(|(name, value)| format!("{name} = {value}")).collect::<Vec<_>>().join(", "),
                    body
                )
            },
            Self::Array(values) => {
                write!(f, "[{}]", values.iter().map(|value| format!("{value}")).collect::<Vec<_>>().join(", "))
            }
            Self::Object(entries) => {
                write!(f, "{{{}}}", entries.iter().map(|(key, value)| format!("{key}: {value}")).collect::<Vec<_>>().join(", "))
            },
            Self::ListComprehension { value, comprehension } => {
                write!(f, "[({}) for {} in ({}){}]",
                    value,
                    comprehension.targets.iter().map(|name| name.as_ref()).collect::<Vec<_>>().join(", "),
                    comprehension.iterable,
                    if let Some(filter) = &comprehension.filter {
                        format!(" if ({filter})")
                    } else {
                        "".to_string()
                    }
                )
            },
            Self::ObjectComprehension { key, value, comprehension } => {
                write!(f, "{{({}): ({}) for {} in ({}){}}}",
                    key,
                    value,
                    comprehension.targets.iter().map(|name| name.as_ref()).collect::<Vec<_>>().join(", "),
                    comprehension.iterable,
                    if let Some(filter) = &comprehension.filter {
                        format!(" if ({filter})")
                    } else {
                        "".to_string()
                    }
                )
            },
            Self::Unary(op, value) => {
                let op_str = match op {
                    UnaryOperator::Positive => "+",
                    UnaryOperator::Negative => "-",
                    UnaryOperator::Not => "!",
                    UnaryOperator::BitwiseNot => "~",
                };
                write!(f, "{op_str}({value})")
            },
            Self::Binary(left, op, right) => {
                let op_str = op.symbol();
                write!(f, "({left}) {op_str} ({right})")
            },
            Self::CompareChain(first, comparisons) => {
                write!(f, "({first})")?;
                for (op, value) in comparisons.iter() {
                    let op_str = op.symbol();
                    write!(f, " {op_str} ({value})")?;
                }
                Ok(())
            },
            Self::Ternary { then_expression, condition, else_expression } => {
                write!(f, "({then_expression}) if ({condition}) else ({else_expression})")
            },
            Self::Call(callee, arguments) => {
                write!(f, "({callee})({})", arguments.iter().map(|arg| format!("{arg}")).collect::<Vec<_>>().join(", "))
            },
            Self::Get(object, property) => {
                write!(f, "{object}.{property}")
            },
            Self::Index(object, index) => {
                write!(f, "{object}[{index}]")
            },
            Self::Slice { object, start, end, step } => {
                write!(f, "({object})[")?;
                if let Some(start) = start { write!(f, "{start}")?; }
                write!(f, ":")?;
                if let Some(end) = end { write!(f, "{end}")?; }
                if let Some(step) = step { write!(f, ":{step}")?; }
                write!(f, "]")
            },
        }
    }
}

/// Lists unary operators supported by the formula language.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Copy)]
pub enum UnaryOperator {
    Positive,
    Negative,
    Not,
    BitwiseNot,
}

/// Lists binary operators supported by the formula language.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Copy)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    FloorDivide,
    Modulo,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    In,
    NotIn,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
    LeftShift,
    RightShift,
    LogicRightShift,
    Xor,
    Coalesce,
}

impl BinaryOperator {
    /// Returns the source-language spelling used when an expression is serialized or shown
    /// in an error context. This must remain total because diagnostics must never panic.
    fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::FloorDivide => "//",
            Self::Modulo => "%",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
            Self::In => "in",
            Self::NotIn => "not in",
            Self::BitAnd => "&",
            Self::BitXor => "^",
            Self::BitOr => "|",
            Self::And => "and",
            Self::Or => "or",
            Self::Xor => "xor",
            Self::Coalesce => "??",
            Self::LeftShift => "<<",
            Self::RightShift => ">>",
            Self::LogicRightShift => ">>>",
        }
    }
}

/// Represents one lexical token emitted by the formula tokenizer.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Int(i64),
    Float(f64),
    String(String),
    Identifier(String),
    Plus,
    Minus,
    Star,
    Slash,
    FloorSlash,
    Percent,
    Ampersand,
    AndAnd,
    Caret,
    Pipe,
    OrOr,
    Equal,
    EqualEqual,
    BangEqual,
    Bang,
    QuestionQuestion,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Comma,
    Colon,
    Dot,
    Arrow,
    End,
    Tilde,
    LeftShift,
    RightShift,
    LogicRightShift,
}

/// Converts a formula source string into a token stream.
pub struct Tokenizer {
    chars: Vec<char>,
    position: usize,
    pending: VecDeque<Token>,
}

impl Tokenizer {
    /// Creates a tokenizer over `source` at character offset zero.
    pub fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            position: 0,
            pending: VecDeque::new(),
        }
    }

    /// Tokenizes the complete formula source.
    pub fn tokenize(mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_end = token == Token::End;
            tokens.push(token);
            if is_end {
                break;
            }
        }
        Ok(tokens)
    }

    /// Scans and returns the next token from the formula source.
    fn next_token(&mut self) -> Result<Token> {
        if let Some(token) = self.pending.pop_front() {
            return Ok(token);
        }
        while self
            .chars
            .get(self.position)
            .is_some_and(|character| character.is_whitespace())
        {
            self.position += 1;
        }
        let Some(character) = self.chars.get(self.position).copied() else {
            return Ok(Token::End);
        };

        if character == '#' {
            let start = self.position;
            self.position += 1;
            return self.radix_integer(16, start);
        }
        if character.is_ascii_digit() {
            return self.number_or_dice();
        }
        if character == '\'' || character == '"' {
            return self.string(character);
        }
        if character == '$' {
            if self
                .chars
                .get(self.position + 1)
                .is_some_and(|character| is_word(*character))
            {
                self.position += 1;
                return self.identifier();
            }
            bail!("Caractère '$' inattendu dans une formule")
        }
        if character == '_' || character.is_alphabetic() {
            return self.identifier();
        }

        self.position += 1;
        Ok(match character {
            '+' => Token::Plus,
            '-' => {
                if self.chars.get(self.position) == Some(&'>') {
                    self.position += 1;
                    Token::Arrow
                } else {
                    Token::Minus
                }
            }
            '*' => Token::Star,
            '%' => Token::Percent,
            '&' => {
                if self.chars.get(self.position) == Some(&'&') {
                    self.position += 1;
                    Token::AndAnd
                } else {
                    Token::Ampersand
                }
            }
            '^' => Token::Caret,
            '|' => {
                if self.chars.get(self.position) == Some(&'|') {
                    self.position += 1;
                    Token::OrOr
                } else {
                    Token::Pipe
                }
            }
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '[' => Token::LeftBracket,
            ']' => Token::RightBracket,
            '{' => Token::LeftBrace,
            '}' => Token::RightBrace,
            ',' => Token::Comma,
            ':' => Token::Colon,
            '.' => Token::Dot,
            '~' => Token::Tilde,
            '?' => {
                if self.chars.get(self.position) == Some(&'?') {
                    self.position += 1;
                    Token::QuestionQuestion
                } else {
                    bail!("Un '?' isolé est invalide; utilisez '??'")
                }
            }
            '/' => {
                if self.chars.get(self.position) == Some(&'/') {
                    self.position += 1;
                    Token::FloorSlash
                } else {
                    Token::Slash
                }
            }
            '=' => {
                if self.chars.get(self.position) == Some(&'=') {
                    self.position += 1;
                    Token::EqualEqual
                } else {
                    Token::Equal
                }
            }
            '!' => {
                if self.chars.get(self.position) == Some(&'=') {
                    self.position += 1;
                    Token::BangEqual
                } else {
                    Token::Bang
                }
            }
            '<' => {
                let n = self.chars.get(self.position);
                if n == Some(&'=') {
                    self.position += 1;
                    Token::LessEqual
                } else if n == Some(&'<') {
                    self.position += 1;
                    Token::LeftShift
                } else {
                    Token::Less
                }
            }
            '>' => {
                let n = self.chars.get(self.position);
                if n == Some(&'=') {
                    self.position += 1;
                    Token::GreaterEqual
                } else if n == Some(&'>') {
                    self.position += 1;
                    let m = self.chars.get(self.position);
                    if m == Some(&'>') {
                        self.position += 1;
                        Token::LogicRightShift
                    } else {
                        Token::RightShift
                    }
                } else {
                    Token::Greater
                }
            }
            _ => bail!("Caractère inattendu dans la formule: '{character}'"),
        })
    }

    /// Reads digits after a radix prefix, applying identical validation to `#` and `0x`.
    fn radix_integer(&mut self, radix: u32, literal_start: usize) -> Result<Token> {
        let digits_start = self.position;
        while self
            .chars
            .get(self.position)
            .is_some_and(|character| character.to_digit(radix).is_some())
        {
            self.position += 1;
        }
        if self.position == digits_start {
            bail!("Le littéral en base {radix} ne contient aucun chiffre")
        }
        if self
            .chars
            .get(self.position)
            .is_some_and(|character| character.is_ascii_alphanumeric() || *character == '_')
        {
            bail!("Chiffre invalide dans un littéral en base {radix}")
        }
        let digits: String = self.chars[digits_start..self.position].iter().collect();
        let value = i64::from_str_radix(&digits, radix).map_err(|_| {
            let literal: String = self.chars[literal_start..self.position].iter().collect();
            anyhow!("Littéral entier hors limites: {literal}")
        })?;
        Ok(Token::Int(value))
    }

    /// Tokenizes an integer, float, radix-prefixed integer, or dice notation.
    fn number_or_dice(&mut self) -> Result<Token> {
        let start = self.position;

        if self.chars.get(start) == Some(&'0') {
            if let Some(prefix) = self.chars.get(start + 1).copied() {
                let radix = match prefix {
                    'x' | 'X' => Some(16),
                    'b' | 'B' => Some(2),
                    'o' | 'O' => Some(8),
                    _ => None,
                };
                if let Some(radix) = radix {
                    self.position += 2;
                    return self.radix_integer(radix, start);
                }
            }
        }

        while self
            .chars
            .get(self.position)
            .is_some_and(|character| character.is_ascii_digit())
        {
            self.position += 1;
        }

        let integer_text: String = self.chars[start..self.position].iter().collect();
        if self.chars.get(self.position) == Some(&'d')
            && self
                .chars
                .get(self.position + 1)
                .is_some_and(|character| character.is_ascii_digit())
        {
            self.position += 1;
            let sides_start = self.position;
            while self
                .chars
                .get(self.position)
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.position += 1;
            }
            let sides_text: String = self.chars[sides_start..self.position].iter().collect();
            let number = integer_text.parse::<i64>()?;
            let sides = sides_text.parse::<i64>()?;
            self.pending.extend([
                Token::LeftParen,
                Token::Int(number),
                Token::Comma,
                Token::Int(sides),
                Token::RightParen,
            ]);
            return Ok(Token::Identifier("dice".to_owned()));
        }

        if self.chars.get(self.position) == Some(&'.')
            && self
                .chars
                .get(self.position + 1)
                .is_some_and(|character| character.is_ascii_digit())
        {
            self.position += 1;
            while self
                .chars
                .get(self.position)
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.position += 1;
            }
            let text: String = self.chars[start..self.position].iter().collect();
            return Ok(Token::Float(text.parse::<f64>()?));
        }

        Ok(Token::Int(integer_text.parse::<i64>()?))
    }

    /// Tokenizes a quoted string literal.
    fn string(&mut self, quote: char) -> Result<Token> {
        let quote_start = self.position;
        if let Some(quote_end) = quoted_formula_literal_end(&self.chars, quote_start) {
            let output = self.chars[quote_start + 1..quote_end].iter().collect();
            self.position = quote_end + 1;
            return Ok(Token::String(output));
        }

        self.position += 1;
        let mut output = String::new();
        while let Some(character) = self.chars.get(self.position).copied() {
            self.position += 1;
            if character == quote {
                return Ok(Token::String(output));
            }
            if character == '\\' {
                let escaped = self
                    .chars
                    .get(self.position)
                    .copied()
                    .ok_or_else(|| anyhow!("Échappement incomplet dans une chaîne"))?;
                self.position += 1;
                output.push(match escaped {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '\\' => '\\',
                    '\'' => '\'',
                    '"' => '"',
                    other => other,
                });
            } else {
                output.push(character);
            }
        }
        bail!("Chaîne non terminée dans la formule")
    }

    /// Tokenizes an identifier or reserved keyword.
    fn identifier(&mut self) -> Result<Token> {
        let start = self.position;
        while self
            .chars
            .get(self.position)
            .is_some_and(|character| is_word(*character))
        {
            self.position += 1;
        }
        if start == self.position {
            bail!("Identifiant attendu")
        }
        Ok(Token::Identifier(
            self.chars[start..self.position].iter().collect(),
        ))
    }
}

/// Builds a formula expression tree from a token stream.
pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    /// Creates a parser positioned before the first token.
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    /// Parses the complete input into its typed representation.
    pub fn parse(mut self) -> Result<Expr> {
        let expression = self.parse_expression()?;
        if !matches!(self.peek(), Token::End) {
            bail!(
                "Jeton inattendu après la fin de la formule: {:?}",
                self.peek()
            )
        }
        Ok(expression)
    }

    /// Parses a scoped `let` expression or an ordinary conditional expression.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_expression(&mut self) -> Result<Expr> {
        if self.match_identifier("fn") {
            self.parse_function()
        } else if self.match_identifier("let") {
            self.parse_let()
        } else {
            self.parse_conditional()
        }
    }

    /// Parses `fn x, y -> expression` into an ordinary first-class function value.
    ///
    /// Empty parameter lists are written `fn -> expression`. Named parameters must be
    /// distinct identifiers and may not reuse formula-language reserved words.
    fn parse_function(&mut self) -> Result<Expr> {
        let mut parameters = Vec::new();
        let mut names = HashSet::new();
        if !matches!(self.peek(), Token::Arrow) {
            loop {
                parameters.push(self.parse_binding_pattern(&mut names, true)?);
                if !matches!(self.peek(), Token::Comma) {
                    break;
                }
                self.advance();
            }
        }
        self.expect(Token::Arrow)?;
        let body = self.parse_expression()?;
        Ok(Expr::Function(FormulaFunction {
            parameters: Some(parameters.into()),
            body: Arc::new(body),
        }))
    }

    /// Parses `let x = value, y = value for expression`.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_let(&mut self) -> Result<Expr> {
        let mut bindings = Vec::new();
        let mut names = HashSet::new();
        loop {
            let pattern = self.parse_binding_pattern(&mut names, false)?;
            self.expect(Token::Equal)?;
            let value = self.parse_expression()?;
            bindings.push((pattern, value));
            if !matches!(self.peek(), Token::Comma) {
                break;
            }
            self.advance();
        }
        self.expect_identifier("for")?;
        let body = self.parse_expression()?;
        Ok(Expr::Let {
            bindings: bindings.into(),
            body: Arc::from(body),
        })
    }

    /// Parses a recursive exact-length list pattern and rejects duplicate names other than `_`.
    fn parse_binding_pattern(
        &mut self,
        names: &mut HashSet<String>,
        parameter: bool,
    ) -> Result<BindingPattern> {
        match self.advance().clone() {
            Token::Identifier(name) => {
                if is_reserved_binding(&name) || (parameter && name == "args") {
                    bail!("Nom réservé dans un motif de liaison: {name}");
                }
                if name != "_" && !names.insert(name.clone()) {
                    bail!("Nom déclaré deux fois dans les motifs de liaison: {name}");
                }
                Ok(BindingPattern::Name(name))
            }
            Token::LeftBracket => {
                let mut items = Vec::new();
                if !matches!(self.peek(), Token::RightBracket) {
                    loop {
                        items.push(self.parse_binding_pattern(names, parameter)?);
                        if !self.match_token(Token::Comma)
                            || matches!(self.peek(), Token::RightBracket)
                        {
                            break;
                        }
                    }
                }
                self.expect(Token::RightBracket)?;
                Ok(BindingPattern::List(items))
            }
            token => bail!("Nom ou motif de liste attendu, reçu {token:?}"),
        }
    }

    /// Parses conditional according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_conditional(&mut self) -> Result<Expr> {
        let then_expression = self.parse_coalesce()?;
        if self.match_identifier("if") {
            let condition = self.parse_coalesce()?;
            self.expect_identifier("else")?;
            let else_expression = self.parse_expression()?;
            Ok(Expr::Ternary {
                then_expression: Arc::from(then_expression),
                condition: Arc::from(condition),
                else_expression: Arc::from(else_expression),
            })
        } else {
            Ok(then_expression)
        }
    }

    /// Null fallback binds below OR and associates to the right.
    fn parse_coalesce(&mut self) -> Result<Expr> {
        let left = self.parse_or()?;
        if self.match_token(Token::QuestionQuestion) {
            Ok(Expr::Binary(
                Arc::new(left),
                BinaryOperator::Coalesce,
                Arc::new(self.parse_coalesce()?),
            ))
        } else {
            Ok(left)
        }
    }

    /// Parses or according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_or(&mut self) -> Result<Expr> {
        let mut expression = self.parse_xor()?;
        while self.match_identifier("or") || self.match_token(Token::OrOr) {
            expression = Expr::Binary(
                Arc::from(expression),
                BinaryOperator::Or,
                Arc::from(self.parse_xor()?),
            );
        }
        Ok(expression)
    }

    /// Parses logical XOR between the OR and AND precedence levels.
    fn parse_xor(&mut self) -> Result<Expr> {
        let mut expression = self.parse_and()?;
        while self.match_identifier("xor") {
            expression = Expr::Binary(
                Arc::from(expression),
                BinaryOperator::Xor,
                Arc::from(self.parse_and()?),
            );
        }
        Ok(expression)
    }

    /// Parses and according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_and(&mut self) -> Result<Expr> {
        let mut expression = self.parse_not()?;
        while self.match_identifier("and") || self.match_token(Token::AndAnd) {
            expression = Expr::Binary(
                Arc::from(expression),
                BinaryOperator::And,
                Arc::from(self.parse_not()?),
            );
        }
        Ok(expression)
    }

    /// Parses not according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_not(&mut self) -> Result<Expr> {
        if self.match_identifier("not") {
            Ok(Expr::Unary(
                UnaryOperator::Not,
                Arc::from(self.parse_not()?),
            ))
        } else {
            self.parse_comparison()
        }
    }

    /// Parses comparison according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_comparison(&mut self) -> Result<Expr> {
        let first = self.parse_bit_or()?;
        let mut comparisons = Vec::new();
        loop {
            let operator = match self.peek() {
                Token::EqualEqual => Some(BinaryOperator::Equal),
                Token::BangEqual => Some(BinaryOperator::NotEqual),
                Token::Less => Some(BinaryOperator::Less),
                Token::LessEqual => Some(BinaryOperator::LessEqual),
                Token::Greater => Some(BinaryOperator::Greater),
                Token::GreaterEqual => Some(BinaryOperator::GreaterEqual),
                Token::Identifier(value) if value == "in" => Some(BinaryOperator::In),
                Token::Identifier(value)
                    if value == "not"
                        && matches!(self.peek_n(1), Token::Identifier(next) if next == "in") =>
                {
                    self.advance();
                    Some(BinaryOperator::NotIn)
                }
                _ => None,
            };
            let Some(operator) = operator else {
                break;
            };
            self.advance();
            comparisons.push((operator, self.parse_bit_or()?));
        }
        if comparisons.is_empty() {
            Ok(first)
        } else {
            Ok(Expr::CompareChain(Arc::from(first), comparisons.into()))
        }
    }

    /// Parses bitwise OR according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_bit_or(&mut self) -> Result<Expr> {
        let mut expression = self.parse_bit_xor()?;
        while matches!(self.peek(), Token::Pipe) {
            self.advance();
            expression = Expr::Binary(
                Arc::from(expression),
                BinaryOperator::BitOr,
                Arc::from(self.parse_bit_xor()?),
            );
        }
        Ok(expression)
    }

    /// Parses bitwise XOR according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_bit_xor(&mut self) -> Result<Expr> {
        let mut expression = self.parse_bit_and()?;
        while matches!(self.peek(), Token::Caret) {
            self.advance();
            expression = Expr::Binary(
                Arc::from(expression),
                BinaryOperator::BitXor,
                Arc::from(self.parse_bit_and()?),
            );
        }
        Ok(expression)
    }

    /// Parses bitwise AND according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_bit_and(&mut self) -> Result<Expr> {
        let mut expression = self.parse_shift()?;
        while matches!(self.peek(), Token::Ampersand) {
            self.advance();
            expression = Expr::Binary(
                Arc::from(expression),
                BinaryOperator::BitAnd,
                Arc::from(self.parse_shift()?),
            );
        }
        Ok(expression)
    }

    /// Parses bitwise shifts according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_shift(&mut self) -> Result<Expr> {
        let mut expression = self.parse_sum()?;
        loop {
            let operator = match self.peek() {
                Token::LeftShift => Some(BinaryOperator::LeftShift),
                Token::RightShift => Some(BinaryOperator::RightShift),
                Token::LogicRightShift => Some(BinaryOperator::LogicRightShift),
                _ => None,
            };
            let Some(operator) = operator else {
                break;
            };
            self.advance();
            expression = Expr::Binary(
                Arc::from(expression),
                operator,
                Arc::from(self.parse_sum()?),
            );
        }
        Ok(expression)
    }

    /// Parses sum according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_sum(&mut self) -> Result<Expr> {
        let mut expression = self.parse_term()?;
        loop {
            let operator = match self.peek() {
                Token::Plus => Some(BinaryOperator::Add),
                Token::Minus => Some(BinaryOperator::Subtract),
                _ => None,
            };
            let Some(operator) = operator else {
                break;
            };
            self.advance();
            expression = Expr::Binary(
                Arc::from(expression),
                operator,
                Arc::from(self.parse_term()?),
            );
        }
        Ok(expression)
    }

    /// Parses term according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_term(&mut self) -> Result<Expr> {
        let mut expression = self.parse_unary()?;
        loop {
            let operator = match self.peek() {
                Token::Star => Some(BinaryOperator::Multiply),
                Token::Slash => Some(BinaryOperator::Divide),
                Token::FloorSlash => Some(BinaryOperator::FloorDivide),
                Token::Percent => Some(BinaryOperator::Modulo),
                _ => None,
            };
            let Some(operator) = operator else {
                break;
            };
            self.advance();
            expression = Expr::Binary(
                Arc::from(expression),
                operator,
                Arc::from(self.parse_unary()?),
            );
        }
        Ok(expression)
    }

    /// Parses unary according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_unary(&mut self) -> Result<Expr> {
        match self.peek() {
            Token::Bang => {
                self.advance();
                Ok(Expr::Unary(
                    UnaryOperator::Not,
                    Arc::from(self.parse_unary()?),
                ))
            }
            Token::Plus => {
                self.advance();
                Ok(Expr::Unary(
                    UnaryOperator::Positive,
                    Arc::from(self.parse_unary()?),
                ))
            }
            Token::Minus => {
                self.advance();
                Ok(Expr::Unary(
                    UnaryOperator::Negative,
                    Arc::from(self.parse_unary()?),
                ))
            }
            Token::Tilde => {
                self.advance();
                Ok(Expr::Unary(
                    UnaryOperator::BitwiseNot,
                    Arc::from(self.parse_unary()?),
                ))
            }
            _ => self.parse_postfix(),
        }
    }

    /// Parses postfix according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expression = self.parse_primary()?;
        loop {
            match self.peek() {
                Token::LeftParen => {
                    self.advance();
                    let mut arguments = Vec::new();
                    if !matches!(self.peek(), Token::RightParen) {
                        loop {
                            arguments.push(self.parse_expression()?);
                            if matches!(self.peek(), Token::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Token::RightParen)?;
                    expression = Expr::Call(Arc::from(expression), arguments.into());
                }
                Token::Dot => {
                    self.advance();
                    let property = match self.advance().clone() {
                        Token::Identifier(property) => property,
                        Token::Int(index) if index >= 0 => index.to_string(),
                        _ => bail!("Nom de propriété ou index attendu après '.'"),
                    };
                    expression = Expr::Get(Arc::from(expression), Arc::from(property.as_str()));
                }
                Token::LeftBracket => {
                    self.advance();
                    let start = if matches!(self.peek(), Token::Colon) {
                        None
                    } else {
                        Some(Arc::new(self.parse_expression()?))
                    };
                    if self.match_token(Token::Colon) {
                        let end = if matches!(self.peek(), Token::Colon | Token::RightBracket) {
                            None
                        } else {
                            Some(Arc::new(self.parse_expression()?))
                        };
                        let step = if self.match_token(Token::Colon) && !matches!(self.peek(), Token::RightBracket) {
                            Some(Arc::new(self.parse_expression()?))
                        } else {
                            None
                        };
                        self.expect(Token::RightBracket)?;
                        expression = Expr::Slice { object: Arc::new(expression), start, end, step };
                    } else {
                        self.expect(Token::RightBracket)?;
                        expression = Expr::Index(Arc::new(expression), start.expect("an index was parsed"));
                    }
                }
                _ => break,
            }
        }
        Ok(expression)
    }

    /// Parses primary according to formula precedence rules.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_primary(&mut self) -> Result<Expr> {
        match self.advance().clone() {
            Token::Int(value) => Ok(Expr::Literal(EvalValue::Int(value))),
            Token::Float(value) => Ok(Expr::Literal(EvalValue::Float(value))),
            Token::String(value) => Ok(Expr::Literal(EvalValue::String(value.into()))),
            Token::Identifier(value) if value == "True" || value == "true" => {
                Ok(Expr::Literal(EvalValue::Bool(true)))
            }
            Token::Identifier(value) if value == "False" || value == "false" => {
                Ok(Expr::Literal(EvalValue::Bool(false)))
            }
            Token::Identifier(value) if value == "None" || value == "null" => {
                Ok(Expr::Literal(EvalValue::Null))
            }
            Token::Identifier(value) => Ok(Expr::Variable(Arc::from(value))),
            Token::LeftParen => {
                let expression = self.parse_expression()?;
                self.expect(Token::RightParen)?;
                Ok(expression)
            }
            Token::LeftBracket => {
                if matches!(self.peek(), Token::RightBracket) {
                    self.advance();
                    return Ok(Expr::Array(Arc::from(Vec::new())));
                }
                let first = self.parse_expression()?;
                if self.match_identifier("for") {
                    let (targets, iterable, filter) = self.parse_comprehension_clause()?;
                    self.expect(Token::RightBracket)?;
                    return Ok(Expr::ListComprehension {
                        value: Arc::from(first),
                        comprehension: Comprehension {
                            targets: targets.into_iter().map(Arc::from).collect(),
                            iterable: Arc::from(iterable),
                            filter: filter.map(Arc::from),
                        },
                    });
                }
                let mut values = vec![first];
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    if matches!(self.peek(), Token::RightBracket) {
                        break;
                    }
                    values.push(self.parse_expression()?);
                }
                self.expect(Token::RightBracket)?;
                Ok(Expr::Array(Arc::from(values)))
            }
            Token::LeftBrace => {
                if matches!(self.peek(), Token::RightBrace) {
                    self.advance();
                    return Ok(Expr::Object(Vec::new().into()));
                }
                let first_key = self.parse_expression()?;
                self.expect(Token::Colon)?;
                let first_value = self.parse_expression()?;
                if self.match_identifier("for") {
                    let (targets, iterable, filter) = self.parse_comprehension_clause()?;
                    self.expect(Token::RightBrace)?;
                    return Ok(Expr::ObjectComprehension {
                        key: Arc::from(first_key),
                        value: Arc::from(first_value),
                        comprehension: Comprehension {
                            targets: targets.into_iter().map(Arc::from).collect(),
                            iterable: Arc::from(iterable),
                            filter: filter.map(Arc::from),
                        },
                    });
                }
                let mut entries = vec![(first_key, first_value)];
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    if matches!(self.peek(), Token::RightBrace) {
                        break;
                    }
                    let key = self.parse_expression()?;
                    self.expect(Token::Colon)?;
                    let value = self.parse_expression()?;
                    entries.push((key, value));
                }
                self.expect(Token::RightBrace)?;
                Ok(Expr::Object(Arc::from(entries)))
            }
            token => bail!("Expression attendue, reçu: {token:?}"),
        }
    }

    /// Parses the variable binding, iterable, and optional filter of a comprehension.
    ///
    /// Required fields and variant-specific JSON types are checked immediately. Formula
    /// strings are preserved for runtime evaluation.
    fn parse_comprehension_clause(&mut self) -> Result<(Vec<String>, Expr, Option<Expr>)> {
        let mut targets = Vec::new();
        loop {
            let Token::Identifier(target) = self.advance().clone() else {
                bail!("Nom de variable attendu après 'for'")
            };
            if is_reserved_binding(&target) {
                bail!("Nom de variable invalide dans une compréhension: {target}")
            }
            targets.push(target);
            if !matches!(self.peek(), Token::Comma) {
                break;
            }
            self.advance();
        }
        self.expect_identifier("in")?;
        let iterable = self.parse_coalesce()?;
        let filter = if self.match_identifier("if") {
            Some(self.parse_coalesce()?)
        } else {
            None
        };
        Ok((targets, iterable, filter))
    }

    /// Returns the current token without consuming it.
    fn peek(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::End)
    }

    /// Returns a later token without consuming the stream.
    fn peek_n(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.position + offset)
            .unwrap_or(&Token::End)
    }

    /// Consumes and returns the current token.
    fn advance(&mut self) -> &Token {
        let position = self.position;
        if self.position < self.tokens.len() {
            self.position += 1;
        }
        self.tokens.get(position).unwrap_or(&Token::End)
    }

    /// Consumes an identifier when it matches the requested text.
    fn match_identifier(&mut self, expected: &str) -> bool {
        if matches!(self.peek(), Token::Identifier(value) if value == expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consumes a token when it matches the requested kind.
    fn match_token(&mut self, expected: Token) -> bool {
        if self.peek() == &expected {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consumes an identifier or returns a parser error.
    fn expect_identifier(&mut self, expected: &str) -> Result<()> {
        if self.match_identifier(expected) {
            Ok(())
        } else {
            bail!("Mot-clé attendu: {expected}")
        }
    }

    /// Consumes an expected token or returns a parser error.
    fn expect(&mut self, expected: Token) -> Result<()> {
        if self.peek() == &expected {
            self.advance();
            Ok(())
        } else {
            bail!("Jeton attendu: {expected:?}, reçu: {:?}", self.peek())
        }
    }
}

/// Returns whether a formula word cannot be used as a local binding name.
fn is_reserved_binding(name: &str) -> bool {
    matches!(
        name,
        "let"
            | "for"
            | "in"
            | "if"
            | "else"
            | "and"
            | "or"
            | "xor"
            | "not"
            | "True"
            | "False"
            | "None"
            | "true"
            | "false"
            | "null"
            | "fn"
    )
}

#[allow(unused)]
pub mod builtins {
    use std::{cmp::Ordering, sync::Arc};

    use crate::formula::{
        expect_arity, full_formula, get_path, has_path, indexof_object, indexof_str, minmax,
        mul_values, normalize_index, pop_values, power_values, python_round, range_values, shuffle,
        sum_values, zip_arrays, Env, EvalValue, FormulaEngine, Function,
    };
    use anyhow::{anyhow, bail, Context, Result};
    use indexmap::IndexMap;
    use lazy_static::lazy_static;
    use rand::Rng;

    fn build_env(env: &mut Env, name: &str, func: fn(&[EvalValue], &mut FormulaEngine, &Env) -> Result<EvalValue>) {
        let key: Arc<str> = Arc::from(format!("global_{name}"));
        env.insert(Arc::from(name), EvalValue::Func(Function::Builtin(key, func).into()));
    }
    fn build_dict(dict: &mut IndexMap<Arc<str>, EvalValue>, typ: &str, name: &str, func: fn(&[EvalValue], &mut FormulaEngine, &Env) -> Result<EvalValue>) {
        let key: Arc<str> = Arc::from(format!("{typ}_{name}"));
        dict.insert(name.into(), EvalValue::Func(Function::Builtin(key, func).into()));
    }
    fn insert_registry(dict: &mut IndexMap<Arc<str>, EvalValue>, reg: &IndexMap<Arc<str>, EvalValue>) {
        for value in reg.values() {
            if let EvalValue::Func(function) = value {
                if let Function::Builtin(key, _) = function.as_ref() {
                    dict.insert(key.clone(), value.clone());
                }
            }
        }
    }

    /////// GLOBALS \\\\\\\
    ///
    lazy_static! {
        pub static ref GLOBAL_ENV: Env<'static> = {
            let mut env = Env::new();

            build_env(&mut env, "dice", dice);
            build_env(&mut env, "min", min);
            build_env(&mut env, "max", max);
            build_env(&mut env, "abs", abs);
            build_env(&mut env, "pow", pow);
            build_env(&mut env, "round", round);
            build_env(&mut env, "str", fn_str);
            build_env(&mut env, "int", int);
            build_env(&mut env, "float", float);
            build_env(&mut env, "bool", fn_bool);
            build_env(&mut env, "zip", zip);
            build_env(&mut env, "len", len);
            build_env(&mut env, "type", fn_type);
            build_env(&mut env, "exists", exists);
            build_env(&mut env, "fromentries", fromentries);
            build_env(&mut env, "random", random);
            build_env(&mut env, "range", range);
            build_env(&mut env, "eval", eval);
            build_env(&mut env, "error", error);
            build_env(&mut env, "sum", array_sum);
            build_env(&mut env, "mul", array_mul);
            build_env(&mut env, "indexof", indexof);
            build_env(&mut env, "enum", array_enum);
            build_env(&mut env, "flatten", array_flatten);
            build_env(&mut env, "map", array_map);
            build_env(&mut env, "filter", array_filter);
            build_env(&mut env, "drop", array_drop);
            build_env(&mut env, "cumul", array_cumul);
            build_env(&mut env, "reduce", array_reduce);
            build_env(&mut env, "apply", apply);

            env
        };
    }

    pub fn dice(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("dice", &arguments, 2)?;
        let number = arguments[0].to_i64()?;
        let sides = arguments[1].to_i64()?;
        if number < 0 || sides <= 0 {
            bail!("dice exige un nombre de dés positif et un nombre de faces supérieur à 0")
        }
        let mut total = 0_i64;
        for _ in 0..number {
            total = total
                .checked_add(formula.rng.gen_range(1..=sides))
                .ok_or_else(|| anyhow!("Dépassement numérique lors du jet de dés"))?;
        }
        Ok(EvalValue::Int(total))
    }
    pub fn min(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        minmax(&arguments, Ordering::Less)
    }
    pub fn max(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        minmax(&arguments, Ordering::Greater)
    }
    pub fn abs(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("abs", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Int(value) => Ok(EvalValue::Int(value.abs())),
            EvalValue::Float(value) => Ok(EvalValue::Float(value.abs())),
            _ => bail!("abs exige un nombre"),
        }
    }
    pub fn pow(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("pow", &arguments, 2)?;
        power_values(&arguments[0], &arguments[1])
    }
    pub fn round(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if !(1..=2).contains(&arguments.len()) {
            bail!("round exige un ou deux arguments")
        }
        let value = arguments[0].to_f64()?;
        let digits = if arguments.len() == 2 {
            arguments[1].to_i64()?
        } else {
            0
        };
        let rounded = python_round(value, digits)?;
        if digits <= 0 {
            Ok(EvalValue::Int(rounded as i64))
        } else {
            Ok(EvalValue::Float(rounded))
        }
    }
    pub fn fn_str(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("str", &arguments, 1)?;
        Ok(EvalValue::String(arguments[0].python_string()))
    }
    pub fn int(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("int", &arguments, 1)?;
        Ok(EvalValue::Int(arguments[0].to_i64()?))
    }
    pub fn float(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("float", &arguments, 1)?;
        Ok(EvalValue::Float(arguments[0].to_f64()?))
    }
    pub fn fn_bool(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("bool", &arguments, 1)?;
        Ok(EvalValue::Bool(arguments[0].truthy()))
    }
    pub fn zip(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        let mut args = Vec::new();
        for argument in arguments.iter() {
            match argument {
                EvalValue::Array(values) => args.push(values.clone()),
                _ => bail!("zip exige des listes comme arguments"),
            }
        }
        Ok(zip_arrays(&args))
    }
    pub fn len(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("len", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => Ok(EvalValue::Int(values.len() as i64)),
            EvalValue::Object(entries) => Ok(EvalValue::Int(entries.len() as i64)),
            EvalValue::String(value) => Ok(EvalValue::Int(value.chars().count() as i64)),
            _ => bail!("len exige une liste, un objet ou une chaîne"),
        }
    }
    pub fn fn_type(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("type", &arguments, 1)?;
        Ok(EvalValue::String(arguments[0].type_name()))
    }
    pub fn exists(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("exists", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Null => Ok(EvalValue::Bool(false)),
            EvalValue::Bool(false) => Ok(EvalValue::Bool(false)),
            EvalValue::String(s) if s.is_empty() => Ok(EvalValue::Bool(false)),
            EvalValue::Int(0) => Ok(EvalValue::Bool(false)),
            EvalValue::Float(0.0) => Ok(EvalValue::Bool(false)),
            EvalValue::Array(arr) if arr.is_empty() => Ok(EvalValue::Bool(false)),
            EvalValue::Object(obj) if obj.is_empty() => Ok(EvalValue::Bool(false)),
            _ => Ok(EvalValue::Bool(true)),
        }
    }
    pub fn fromentries(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("fromentries", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(entries) => {
                let mut obj = IndexMap::new();
                for entry in entries.iter() {
                    match entry {
                        EvalValue::Array(pair) if pair.len() == 2 => {
                            let key = pair[0].clone().try_into()?;
                            obj.insert(key, pair[1].clone());
                        }
                        _ => bail!("fromentries exige une liste de paires"),
                    }
                }
                Ok(EvalValue::Object(Arc::from(obj)))
            }
            _ => bail!("fromentries exige une liste"),
        }
    }
    pub fn random(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("random", &arguments, 0)?;
        Ok(EvalValue::Float(formula.rng.gen_range(0.0..=1.0)))
    }
    pub fn range(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        range_values(&arguments.iter().collect::<Vec<_>>())
    }
    pub fn eval(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if arguments.len() < 1 || arguments.len() > 2 {
            bail!("eval exige un ou deux arguments");
        }
        let expr = &arguments[0];
        let expression = match expr {
            EvalValue::String(s) => s.clone(),
            _ => return Ok(expr.clone()),
        };
        let local_env = if arguments.len() == 2 {
            match &arguments[1] {
                EvalValue::Object(obj) => Env::from_index_map(&**obj),
                _ => bail!("eval exige un dictionnaire comme second argument"),
            }
        } else {
            Env::new()
        };
        let local_env = env.merged(&local_env);
        if let Some(full) = full_formula(&expression) {
            formula.evaluate(full, &local_env)
        } else {
            Ok(EvalValue::String(formula.format(expression, &local_env)?))
        }
    }
    pub fn error(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if let Some(msg) = arguments.get(0) {
            bail!(msg.to_string());
        } else {
            bail!("Erreur!")
        }
    }
    /// Finds a value in a list, substring in a string, or value in a dictionary.
    pub fn indexof(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("indexof", arguments, 2)?;
        match &arguments[0] {
            EvalValue::Array(_) => array_indexof(arguments, formula, env),
            EvalValue::String(haystack) => {
                let EvalValue::String(needle) = &arguments[1] else {
                    bail!("indexof exige une chaîne à chercher dans une chaîne")
                };
                Ok(indexof_str(haystack, needle))
            }
            EvalValue::Object(values) => Ok(indexof_object(values, &arguments[1])),
            value => bail!(
                "indexof ne peut pas chercher dans {}: une liste, une chaîne ou un dictionnaire est requis",
                value.diagnostic()
            ),
        }
    }

    /// Calls a function with every item from a list as one positional argument.
    pub fn apply(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("apply", arguments, 2)?;
        let EvalValue::Array(values) = &arguments[1] else {
            bail!("apply exige une liste comme second argument")
        };
        arguments[0].clone().call(values, formula, env)
    }

    ////// ARRAY METHODS \\\\\\\
    lazy_static! {
        pub static ref ARRAY_METHODS: IndexMap<Arc<str>, EvalValue> = {
            let mut map = IndexMap::new();
            build_dict(&mut map, "array", "sum", array_sum);
            build_dict(&mut map, "array", "mul", array_mul);
            build_dict(&mut map, "array", "indexof", array_indexof);
            build_dict(&mut map, "array", "enum", array_enum);
            build_dict(&mut map, "array", "flatten", array_flatten);
            build_dict(&mut map, "array", "flat", array_flat);
            build_dict(&mut map, "array", "len", array_len);
            build_dict(&mut map, "array", "unique", array_unique);
            build_dict(&mut map, "array", "sort", array_sort);
            build_dict(&mut map, "array", "sort_by", array_sort_by);
            build_dict(&mut map, "array", "min_by", array_min_by);
            build_dict(&mut map, "array", "max_by", array_max_by);
            build_dict(&mut map, "array", "group_by", array_group_by);
            build_dict(&mut map, "array", "reverse", array_reverse);
            build_dict(&mut map, "array", "take", array_take);
            build_dict(&mut map, "array", "skip", array_skip);
            build_dict(&mut map, "array", "slice", array_slice);
            build_dict(&mut map, "array", "chunks", array_chunks);
            build_dict(&mut map, "array", "windows", array_windows);
            build_dict(&mut map, "array", "all", array_all);
            build_dict(&mut map, "array", "any", array_any);
            build_dict(&mut map, "array", "join", array_join);
            build_dict(&mut map, "array", "choice", array_choice);
            build_dict(&mut map, "array", "weighted", array_weighted);
            build_dict(&mut map, "array", "shuffle", array_shuffle);
            build_dict(&mut map, "array", "coalesce", array_coalesce);
            build_dict(&mut map, "array", "tryget", array_tryget);
            build_dict(&mut map, "array", "pop", array_pop);
            build_dict(&mut map, "array", "map", array_map);
            build_dict(&mut map, "array", "filter", array_filter);
            build_dict(&mut map, "array", "drop", array_drop);
            build_dict(&mut map, "array", "cumul", array_cumul);
            build_dict(&mut map, "array", "reduce", array_reduce);
            build_dict(&mut map, "array", "apply", array_apply);
            build_dict(&mut map, "array", "find", array_find);
            build_dict(&mut map, "array", "fmt", array_fmt);
            build_dict(&mut map, "array", "max", array_max);
            build_dict(&mut map, "array", "min", array_min);
            map
        };
    }
    pub fn array_sum(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("sum", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => sum_values(values),
            _ => bail!("sum exige une liste"),
        }
    }
    pub fn array_mul(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("mul", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => mul_values(values),
            _ => bail!("mul exige une liste"),
        }
    }
    pub fn array_indexof(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("indexof", &arguments, 2)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                let target = &arguments[1];
                for (i, value) in values.iter().enumerate() {
                    if value == target {
                        return Ok(EvalValue::Int(i as i64));
                    }
                }
                Ok(EvalValue::Null)
            }
            _ => bail!("indexof exige une liste comme premier argument"),
        }
    }
    pub fn array_enum(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("enum", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                let mut enumerated = Vec::new();
                for (i, value) in values.iter().enumerate() {
                    enumerated.push(EvalValue::Array(Arc::from(vec![
                        EvalValue::Int(i as i64),
                        value.clone(),
                    ])));
                }
                Ok(EvalValue::Array(Arc::from(enumerated)))
            }
            _ => bail!("enum exige une liste"),
        }
    }
    pub fn array_flatten(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("flatten", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                let mut flattened = Vec::new();
                for value in values.iter() {
                    match value {
                        EvalValue::Array(inner) => {
                            flattened.extend(inner.iter().cloned());
                        }
                        _ => flattened.push(value.clone()),
                    }
                }
                Ok(EvalValue::Array(Arc::from(flattened)))
            }
            _ => bail!("flatten exige une liste"),
        }
    }
    /// Flattens all nested lists in source order without recursive Rust calls.
    pub fn array_flat(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("flat", arguments, 1)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("flat exige une liste");
        };
        let mut flattened = Vec::new();
        let mut stack = vec![values.iter()];
        while let Some(items) = stack.last_mut() {
            match items.next() {
                Some(EvalValue::Array(inner)) => stack.push(inner.iter()),
                Some(value) => flattened.push(value.clone()),
                None => {
                    stack.pop();
                }
            }
        }
        Ok(EvalValue::Array(flattened.into()))
    }

    pub fn array_len(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("len", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => Ok(EvalValue::Int(values.len() as i64)),
            _ => bail!("len exige une liste"),
        }
    }
    pub fn array_unique(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("unique", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                let mut seen = Vec::new();
                for value in values.iter() {
                    if !seen.contains(value) {
                        seen.push(value.clone());
                    }
                }
                Ok(EvalValue::Array(Arc::from(seen)))
            }
            _ => bail!("unique exige une liste"),
        }
    }
    pub fn array_sort(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("sort", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                let mut sorted = values.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                Ok(EvalValue::Array(Arc::from(sorted)))
            }
            _ => bail!("sort exige une liste"),
        }
    }
    /// Evaluates each key exactly once, in input order, with the usual value/index callback.
    fn keyed_values(
        name: &str,
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<Vec<(EvalValue, EvalValue)>> {
        expect_arity(name, arguments, 2)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("{name} exige une liste");
        };
        if !matches!(arguments[1], EvalValue::Func(_)) {
            bail!("{name} exige une fonction de clé");
        }
        values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let key = arguments[1]
                    .clone()
                    .call(&[value.clone(), EvalValue::Int(index as i64)], formula, env)
                    .with_context(|| format!("Clé de {name} pour l'élément {index}"))?;
                Ok((key, value.clone()))
            })
            .collect()
    }

    /// A fallible stable merge sort propagates incomparable-key errors without panicking.
    fn sort_keyed(mut values: Vec<(EvalValue, EvalValue)>) -> Result<Vec<(EvalValue, EvalValue)>> {
        if values.len() < 2 {
            return Ok(values);
        }
        let right = values.split_off(values.len() / 2);
        let mut left = sort_keyed(values)?.into_iter().peekable();
        let mut right = sort_keyed(right)?.into_iter().peekable();
        let mut result = Vec::with_capacity(left.len() + right.len());
        while let (Some(a), Some(b)) = (left.peek(), right.peek()) {
            if super::compare_values(&a.0, &b.0)? != Ordering::Greater {
                result.push(left.next().unwrap());
            } else {
                result.push(right.next().unwrap());
            }
        }
        result.extend(left);
        result.extend(right);
        Ok(result)
    }

    pub fn array_sort_by(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        let values = keyed_values("sort_by", arguments, formula, env)?;
        for (key, _) in &values {
            super::compare_values(key, key)?;
        }
        let sorted = sort_keyed(values).context("Comparaison des clés de sort_by")?;
        Ok(EvalValue::Array(
            sorted.into_iter().map(|(_, value)| value).collect(),
        ))
    }

    fn array_extreme_by(
        name: &str,
        direction: Ordering,
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        let values = keyed_values(name, arguments, formula, env)?;
        for (key, _) in &values {
            super::compare_values(key, key)?;
        }
        let mut values = values.into_iter();
        let Some(mut best) = values.next() else {
            bail!("{name} exige une liste non vide");
        };
        for candidate in values {
            if super::compare_values(&candidate.0, &best.0)
                .with_context(|| format!("Comparaison des clés de {name}"))?
                == direction
            {
                best = candidate;
            }
        }
        Ok(best.1)
    }

    pub fn array_min_by(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        array_extreme_by("min_by", Ordering::Less, arguments, formula, env)
    }

    pub fn array_max_by(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        array_extreme_by("max_by", Ordering::Greater, arguments, formula, env)
    }

    pub fn array_group_by(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        let values = keyed_values("group_by", arguments, formula, env)?;
        let mut groups: IndexMap<Arc<str>, Vec<EvalValue>> = IndexMap::new();
        for (key, value) in values {
            let EvalValue::String(key) = key else {
                bail!(
                    "group_by exige des clés de type str, reçu {}",
                    key.diagnostic()
                );
            };
            groups.entry(key).or_default().push(value);
        }
        Ok(EvalValue::Object(Arc::new(
            groups
                .into_iter()
                .map(|(key, values)| (key, EvalValue::Array(values.into())))
                .collect(),
        )))
    }

    pub fn array_reverse(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("reverse", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                let mut reversed = values.to_vec();
                reversed.reverse();
                Ok(EvalValue::Array(Arc::from(reversed)))
            }
            _ => bail!("reverse exige une liste"),
        }
    }
    pub fn array_take(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("take", &arguments, 2)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("take exige une liste comme premier argument"),
        };
        let count = arguments[1].to_i64()?;
        if count < 0 {
            bail!("take exige un entier positif comme second argument")
        }
        Ok(EvalValue::Array(
            values.iter().take(count as usize).cloned().collect(),
        ))
    }
    pub fn array_skip(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("skip", &arguments, 2)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("skip exige une liste comme premier argument"),
        };
        let count = arguments[1].to_i64()?;
        if count < 0 {
            bail!("skip exige un entier positif comme second argument")
        }
        Ok(EvalValue::Array(
            values.iter().skip(count as usize).cloned().collect(),
        ))
    }
    pub fn array_slice(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("slice", &arguments, 3)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("slice exige une liste comme premier argument"),
        };
        let start = arguments[1].to_i64()?;
        let end = arguments[2].to_i64()?;
        if start < 0 {
            bail!("slice exige des entiers positifs comme premier et deuxième arguments")
        }
        let end = if end < 0 {
            values.len() as i64 + end
        } else {
            end
        };
        if end < start {
            bail!("slice exige un premier argument inférieur ou égal au deuxième")
        }
        Ok(EvalValue::Array(
            values
                .iter()
                .skip(start as usize)
                .take((end - start) as usize)
                .cloned()
                .collect(),
        ))
    }
    pub fn array_chunks(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("chunks", &arguments, 2)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("chunks exige une liste comme premier argument"),
        };
        let size = arguments[1].to_i64()?;
        if size <= 0 {
            bail!("chunks exige un entier positif comme second argument")
        }
        Ok(EvalValue::Array(
            values
                .chunks(size as usize)
                .map(|chunk| EvalValue::Array(Arc::from(chunk.to_vec())))
                .collect(),
        ))
    }
    pub fn array_windows(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("windows", &arguments, 2)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("windows exige une liste comme premier argument"),
        };
        let size = arguments[1].to_i64()?;
        if size <= 0 {
            bail!("windows exige un entier positif comme second argument")
        }
        Ok(EvalValue::Array(
            values
                .windows(size as usize)
                .map(|window| EvalValue::Array(Arc::from(window.to_vec())))
                .collect(),
        ))
    }
    pub fn array_all(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        array_quantify("all", true, arguments, formula, env)
    }
    pub fn array_any(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        array_quantify("any", false, arguments, formula, env)
    }

    fn array_quantify(
        name: &str,
        all: bool,
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if !(1..=2).contains(&arguments.len()) {
            bail!("{name} exige zéro ou une fonction après la liste");
        }
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("{name} exige une liste");
        };
        let predicate = arguments.get(1);
        if predicate.is_some_and(|value| !matches!(value, EvalValue::Func(_))) {
            bail!("{name} exige une fonction comme prédicat");
        }
        for (index, value) in values.iter().enumerate() {
            let truth = if let Some(predicate) = predicate {
                predicate
                    .clone()
                    .call(&[value.clone(), EvalValue::Int(index as i64)], formula, env)
                    .with_context(|| format!("Prédicat de {name} pour l'élément {index}"))?
                    .truthy()
            } else {
                value.truthy()
            };
            if truth != all {
                return Ok(EvalValue::Bool(!all));
            }
        }
        Ok(EvalValue::Bool(all))
    }
    pub fn array_join(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("join", &arguments, 2)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("join exige une liste comme premier argument"),
        };
        let separator = match &arguments[1] {
            EvalValue::String(s) => s,
            _ => bail!("join exige une chaîne comme second argument"),
        };
        let to_joined: Vec<String> = values
            .iter()
            .map(|value| String::from(value.python_string().as_ref()))
            .collect::<Vec<_>>();
        Ok(EvalValue::String(to_joined.join(separator).into()))
    }
    pub fn array_choice(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("choice", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                if values.is_empty() {
                    bail!("choice exige une liste non vide")
                }
                let index = formula.rng.gen_range(0..values.len());
                Ok(values[index].clone())
            }
            _ => bail!("choice exige une liste"),
        }
    }
    pub fn array_weighted(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("weighted", &arguments, 1)?;
        match &arguments[0] {
            EvalValue::Array(values) => {
                if values.is_empty() {
                    bail!("weighted exige une liste non vide")
                }
                let mut total_weight = 0.0;
                let mut weighted_values = Vec::new();
                for value in values.iter() {
                    match value {
                        EvalValue::Array(pair) if pair.len() == 2 => {
                            let weight = pair[1].to_f64()?;
                            if weight < 0.0 {
                                bail!("weighted_choice exige des poids non négatifs")
                            }
                            total_weight += weight;
                            weighted_values.push((pair[0].clone(), weight));
                        }
                        _ => bail!("weighted_choice exige une liste de paires"),
                    }
                }
                if total_weight == 0.0 {
                    bail!("weighted_choice exige des poids totaux supérieurs à zéro")
                }
                let mut rng_value = formula.rng.gen_range(0.0..total_weight);
                for (value, weight) in weighted_values.iter() {
                    if rng_value < *weight {
                        return Ok(value.clone());
                    }
                    rng_value -= *weight;
                }
                unreachable!()
            }
            _ => bail!("weighted_choice exige une liste"),
        }
    }
    pub fn array_shuffle(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("shuffle", &arguments, 1)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("shuffle exige une liste comme premier argument"),
        };
        let mut shuffled = values.clone();
        shuffle(Arc::make_mut(&mut shuffled), &mut formula.rng);
        Ok(EvalValue::Array(shuffled))
    }
    pub fn array_coalesce(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("coalesce", &arguments, 1)?;
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("coalesce exige une liste comme premier argument"),
        };
        for value in values.iter() {
            if value != &EvalValue::Null {
                return Ok(value.clone());
            }
        }
        Ok(EvalValue::Null)
    }
    pub fn array_tryget(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("tryget exige une liste comme premier argument"),
        };
        let arguments = &arguments[1..];
        if arguments.len() < 1 || arguments.len() > 2 {
            bail!("tryget exige un ou deux arguments")
        }
        let mut index = arguments[0].to_i64()?;
        if index < 0 {
            index = values.len() as i64 + index;
        }
        if index < 0 || index >= values.len() as i64 {
            let default = arguments.get(1).cloned().unwrap_or(EvalValue::Null);
            Ok(default)
        } else {
            Ok(values[index as usize].clone())
        }
    }
    pub fn array_pop(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        let values = match &arguments[0] {
            EvalValue::Array(values) => values,
            _ => bail!("pop exige une liste comme premier argument"),
        };
        let arguments = &arguments[1..];
        if arguments.len() > 1 {
            bail!("pop exige zéro ou un argument")
        }
        let index = arguments
            .get(0)
            .map(EvalValue::to_i64)
            .transpose()?
            .map(|index| normalize_index(index, values.len()))
            .transpose()?;
        pop_values(&values.iter().collect::<Vec<_>>(), index)
    }

    /// Applies a callback to every list item. The callback receives value and index.
    pub fn array_map(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("map", arguments, 2)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("map exige une liste comme premier argument")
        };
        let function = arguments[1].clone();
        let mut mapped = Vec::with_capacity(values.len());
        for (index, value) in values.iter().enumerate() {
            mapped.push(function.clone().call(
                &[value.clone(), EvalValue::Int(index as i64)],
                formula,
                env,
            )?);
        }
        Ok(EvalValue::Array(mapped.into()))
    }

    /// Keeps list items for which the callback returns a true value.
    pub fn array_filter(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("filter", arguments, 2)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("filter exige une liste comme premier argument")
        };
        let function = arguments[1].clone();
        let mut filtered = Vec::new();
        for (index, value) in values.iter().enumerate() {
            if function
                .clone()
                .call(&[value.clone(), EvalValue::Int(index as i64)], formula, env)?
                .truthy()
            {
                filtered.push(value.clone());
            }
        }
        Ok(EvalValue::Array(filtered.into()))
    }

    /// Removes the first `count` values from a list.
    pub fn array_drop(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        array_skip(arguments, formula, env)
    }

    /// Returns every intermediate accumulator produced by a callback.
    ///
    /// The callback receives accumulator, current value, and index. Without an explicit
    /// initial value the first list item is both the initial accumulator and the first output.
    pub fn array_cumul(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if !(2..=3).contains(&arguments.len()) {
            bail!("cumul exige deux ou trois arguments")
        }
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("cumul exige une liste comme premier argument")
        };
        let function = arguments[1].clone();
        let (mut accumulator, start, mut output) = match arguments.get(2) {
            Some(initial) => (initial.clone(), 0, Vec::with_capacity(values.len())),
            None => {
                let Some(first) = values.first() else {
                    return Ok(EvalValue::Array(Vec::new().into()));
                };
                (first.clone(), 1, vec![first.clone()])
            }
        };
        for (index, value) in values.iter().enumerate().skip(start) {
            accumulator = function.clone().call(
                &[accumulator, value.clone(), EvalValue::Int(index as i64)],
                formula,
                env,
            )?;
            output.push(accumulator.clone());
        }
        Ok(EvalValue::Array(output.into()))
    }

    /// Folds a list into one value with an optional initial accumulator.
    pub fn array_reduce(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if !(2..=3).contains(&arguments.len()) {
            bail!("reduce exige deux ou trois arguments")
        }
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("reduce exige une liste comme premier argument")
        };
        let function = arguments[1].clone();
        let (mut accumulator, start) = match arguments.get(2) {
            Some(initial) => (initial.clone(), 0),
            None => (
                values.first().cloned().ok_or_else(|| {
                    anyhow!("reduce exige une valeur initiale pour une liste vide")
                })?,
                1,
            ),
        };
        for (index, value) in values.iter().enumerate().skip(start) {
            accumulator = function.clone().call(
                &[accumulator, value.clone(), EvalValue::Int(index as i64)],
                formula,
                env,
            )?;
        }
        Ok(accumulator)
    }

    /// Calls a function with the list's values as positional arguments.
    pub fn array_apply(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("apply", arguments, 2)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("apply exige une liste comme receveur")
        };
        arguments[1].clone().call(values, formula, env)
    }

    /// Returns the first list item accepted by a callback, or an optional default.
    pub fn array_find(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        if !(2..=3).contains(&arguments.len()) {
            bail!("find exige un ou deux arguments après la liste")
        }
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("find exige une liste comme premier argument")
        };
        let function = arguments[1].clone();
        for (index, value) in values.iter().enumerate() {
            if function
                .clone()
                .call(&[value.clone(), EvalValue::Int(index as i64)], formula, env)?
                .truthy()
            {
                return Ok(value.clone());
            }
        }
        Ok(arguments.get(2).cloned().unwrap_or(EvalValue::Null))
    }
    /// Iter through a list of values, transforming each values into a str. If a value is a func, it is evaluated with no arguments.
    pub fn array_fmt(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("fmt", arguments, 1)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("fmt exige une liste comme premier argument")
        };
        let mut formatted = Vec::with_capacity(values.len());
        for value in values.iter() {
            let formatted_value = match value {
                EvalValue::Func(_) => value.clone().call(&[], formula, env)?.python_string(),
                _ => value.python_string(),
            };
            formatted.push(formatted_value);
        }
        let final_string = formatted
            .iter()
            .map(|s| s.as_ref())
            .collect::<Vec<_>>()
            .join("");
        Ok(EvalValue::String(final_string.into()))
    }

    pub fn array_max(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("max", arguments, 1)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("max exige une liste comme premier argument")
        };
        if values.is_empty() {
            bail!("max exige une liste non vide")
        }
        let max_value = values
            .iter()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
            .unwrap();
        Ok(max_value.clone())
    }
    pub fn array_min(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("min", arguments, 1)?;
        let EvalValue::Array(values) = &arguments[0] else {
            bail!("min exige une liste comme premier argument")
        };
        if values.is_empty() {
            bail!("min exige une liste non vide")
        }
        let min_value = values
            .iter()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
            .unwrap();
        Ok(min_value.clone())
    }

    //////// OBJECT METHODS \\\\\\\
    lazy_static! {
        pub static ref OBJECT_METHODS: IndexMap<Arc<str>, EvalValue> = {
            let mut map = IndexMap::new();
            build_dict(&mut map, "object", "len", object_len);
            build_dict(&mut map, "object", "indexof", object_indexof);
            build_dict(&mut map, "object", "keys", object_keys);
            build_dict(&mut map, "object", "values", object_values);
            build_dict(&mut map, "object", "entries", object_entries);
            build_dict(&mut map, "object", "haspath", object_haspath);
            build_dict(&mut map, "object", "getpath", object_getpath);
            build_dict(&mut map, "object", "tryget", object_tryget);
            build_dict(&mut map, "object", "remove", object_remove);
            build_dict(&mut map, "object", "map", object_map);
            build_dict(&mut map, "object", "filter", object_filter);
            map
        };
    }

    fn object_argument<'a>(
        name: &str,
        arguments: &'a [EvalValue],
    ) -> Result<&'a Arc<IndexMap<Arc<str>, EvalValue>>> {
        match arguments.first() {
            Some(EvalValue::Object(values)) => Ok(values),
            _ => bail!("{name} exige un dictionnaire comme premier argument"),
        }
    }

    pub fn object_len(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("len", arguments, 1)?;
        Ok(EvalValue::Int(
            object_argument("len", arguments)?.len() as i64
        ))
    }

    pub fn object_indexof(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("indexof", arguments, 2)?;
        Ok(indexof_object(
            object_argument("indexof", arguments)?,
            &arguments[1],
        ))
    }

    pub fn object_keys(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("keys", arguments, 1)?;
        Ok(EvalValue::Array(
            object_argument("keys", arguments)?
                .keys()
                .cloned()
                .map(EvalValue::String)
                .collect(),
        ))
    }

    pub fn object_values(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("values", arguments, 1)?;
        Ok(EvalValue::Array(
            object_argument("values", arguments)?
                .values()
                .cloned()
                .collect(),
        ))
    }

    pub fn object_entries(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("entries", arguments, 1)?;
        Ok(EvalValue::Array(
            object_argument("entries", arguments)?
                .iter()
                .map(|(key, value)| {
                    EvalValue::Array(vec![EvalValue::String(key.clone()), value.clone()].into())
                })
                .collect(),
        ))
    }

    fn object_path<'a>(name: &str, value: &'a EvalValue) -> Result<Vec<&'a str>> {
        let EvalValue::String(path) = value else {
            bail!("{name} exige une chaîne comme chemin")
        };
        Ok(path.split('.').collect())
    }

    pub fn object_haspath(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("haspath", arguments, 2)?;
        object_argument("haspath", arguments)?;
        Ok(EvalValue::Bool(has_path(
            &arguments[0],
            &object_path("haspath", &arguments[1])?,
        )))
    }

    pub fn object_getpath(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        if !(2..=3).contains(&arguments.len()) {
            bail!("getpath exige un chemin et, facultativement, une valeur par défaut")
        }
        object_argument("getpath", arguments)?;
        get_path(
            &arguments[0],
            &object_path("getpath", &arguments[1])?,
            arguments.get(2).cloned(),
        )
    }

    pub fn object_tryget(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        if !(2..=3).contains(&arguments.len()) {
            bail!("tryget exige une clé et, facultativement, une valeur par défaut")
        }
        let values = object_argument("tryget", arguments)?;
        let EvalValue::String(key) = &arguments[1] else {
            bail!("tryget exige une chaîne comme clé (reçue : {})", &arguments[1])
        };
        Ok(values
            .get(key)
            .cloned()
            .unwrap_or_else(|| arguments.get(2).cloned().unwrap_or(EvalValue::Null)))
    }

    pub fn object_remove(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("remove", arguments, 2)?;
        let values = object_argument("remove", arguments)?;
        let EvalValue::String(key) = &arguments[1] else {
            bail!("remove exige une chaîne comme clé")
        };
        let mut result = values.clone();
        Arc::make_mut(&mut result).shift_remove(key);
        Ok(EvalValue::Object(result))
    }

    /// Maps dictionary values while preserving their keys. The callback receives value and key.
    pub fn object_map(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("map", arguments, 2)?;
        let values = object_argument("map", arguments)?;
        let function = arguments[1].clone();
        let mut result = IndexMap::with_capacity(values.len());
        for (key, value) in values.iter() {
            result.insert(
                key.clone(),
                function.clone().call(
                    &[value.clone(), EvalValue::String(key.clone())],
                    formula,
                    env,
                )?,
            );
        }
        Ok(EvalValue::Object(result.into()))
    }

    /// Keeps dictionary entries accepted by a callback receiving value and key.
    pub fn object_filter(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("filter", arguments, 2)?;
        let values = object_argument("filter", arguments)?;
        let function = arguments[1].clone();
        let mut result = IndexMap::new();
        for (key, value) in values.iter() {
            if function
                .clone()
                .call(
                    &[value.clone(), EvalValue::String(key.clone())],
                    formula,
                    env,
                )?
                .truthy()
            {
                result.insert(key.clone(), value.clone());
            }
        }
        Ok(EvalValue::Object(result.into()))
    }

    //////// STRING METHODS \\\\\\\
    lazy_static! {
        pub static ref STRING_METHODS: IndexMap<Arc<str>, EvalValue> = {
            let mut map = IndexMap::new();
            build_dict(&mut map, "string", "len", string_len);
            build_dict(&mut map, "string", "indexof", string_indexof);
            build_dict(&mut map, "string", "lower", string_lower);
            build_dict(&mut map, "string", "upper", string_upper);
            build_dict(&mut map, "string", "trim", string_trim);
            build_dict(&mut map, "string", "replace", string_replace);
            build_dict(&mut map, "string", "split", string_split);
            build_dict(&mut map, "string", "startswith", string_startswith);
            build_dict(&mut map, "string", "endswith", string_endswith);
            build_dict(&mut map, "string", "capitalize", string_capitalize);
            build_dict(&mut map, "string", "title", string_title);
            build_dict(&mut map, "string", "slug", string_slug);
            build_dict(&mut map, "string", "zfill", string_zfill);
            build_dict(&mut map, "string", "fmt", string_fmt);
            build_dict(&mut map, "string", "slice", string_slice);
            map
        };
    }

    fn string_argument<'a>(name: &str, arguments: &'a [EvalValue]) -> Result<&'a Arc<str>> {
        match arguments.first() {
            Some(EvalValue::String(value)) => Ok(value),
            _ => bail!("{name} exige une chaîne comme premier argument"),
        }
    }

    pub fn string_len(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("len", arguments, 1)?;
        Ok(EvalValue::Int(
            string_argument("len", arguments)?.chars().count() as i64,
        ))
    }

    pub fn string_indexof(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("indexof", arguments, 2)?;
        let value = string_argument("indexof", arguments)?;
        let EvalValue::String(needle) = &arguments[1] else {
            bail!("indexof exige une chaîne comme argument")
        };
        Ok(indexof_str(value, needle))
    }

    pub fn string_lower(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("lower", arguments, 1)?;
        Ok(EvalValue::String(
            string_argument("lower", arguments)?.to_lowercase().into(),
        ))
    }

    pub fn string_upper(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("upper", arguments, 1)?;
        Ok(EvalValue::String(
            string_argument("upper", arguments)?.to_uppercase().into(),
        ))
    }

    pub fn string_trim(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("trim", arguments, 1)?;
        let value = string_argument("trim", arguments)?;
        let trimmed = value.trim();
        Ok(EvalValue::String(if trimmed == value.as_ref() {
            value.clone()
        } else {
            trimmed.into()
        }))
    }

    pub fn string_replace(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("replace", arguments, 3)?;
        let value = string_argument("replace", arguments)?;
        let (EvalValue::String(from), EvalValue::String(to)) = (&arguments[1], &arguments[2])
        else {
            bail!("replace exige deux chaînes comme arguments")
        };
        Ok(EvalValue::String(
            value.replace(from.as_ref(), to.as_ref()).into(),
        ))
    }

    pub fn string_split(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("split", arguments, 2)?;
        let value = string_argument("split", arguments)?;
        let EvalValue::String(separator) = &arguments[1] else {
            bail!("split exige une chaîne comme séparateur")
        };
        Ok(EvalValue::Array(
            value
                .split(separator.as_ref())
                .map(|part| EvalValue::String(part.into()))
                .collect(),
        ))
    }

    pub fn string_startswith(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("startswith", arguments, 2)?;
        let value = string_argument("startswith", arguments)?;
        let EvalValue::String(prefix) = &arguments[1] else {
            bail!("startswith exige une chaîne comme préfixe")
        };
        Ok(EvalValue::Bool(value.starts_with(prefix.as_ref())))
    }

    pub fn string_endswith(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("endswith", arguments, 2)?;
        let value = string_argument("endswith", arguments)?;
        let EvalValue::String(suffix) = &arguments[1] else {
            bail!("endswith exige une chaîne comme suffixe")
        };
        Ok(EvalValue::Bool(value.ends_with(suffix.as_ref())))
    }

    pub fn string_capitalize(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("capitalize", arguments, 1)?;
        let value = string_argument("capitalize", arguments)?;
        let mut characters = value.chars();
        let result = characters.next().map_or_else(String::new, |first| {
            first.to_uppercase().collect::<String>() + characters.as_str()
        });
        Ok(EvalValue::String(result.into()))
    }

    pub fn string_title(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("title", arguments, 1)?;
        let value = string_argument("title", arguments)?;
        let result = value
            .split_whitespace()
            .map(|word| {
                let mut characters = word.chars();
                characters.next().map_or_else(String::new, |first| {
                    first.to_uppercase().collect::<String>() + characters.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" ");
        Ok(EvalValue::String(result.into()))
    }

    pub fn string_slug(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("slug", arguments, 1)?;
        let result = string_argument("slug", arguments)?
            .to_lowercase()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        Ok(EvalValue::String(result.into()))
    }

    pub fn string_zfill(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        if !(2..=3).contains(&arguments.len()) {
            bail!("zfill exige une largeur et, facultativement, un caractère")
        }
        let value = string_argument("zfill", arguments)?;
        let width = arguments[1].to_i64()?;
        if width < 0 {
            bail!("zfill exige une largeur positive")
        }
        let fill = match arguments.get(2) {
            Some(EvalValue::String(fill)) if fill.chars().count() == 1 => {
                fill.chars().next().expect("one character")
            }
            Some(_) => bail!("zfill exige une chaîne d'un caractère comme remplissage"),
            None => '0',
        };
        let mut result = value.to_string();
        while result.chars().count() < width as usize {
            result.insert(0, fill);
        }
        Ok(EvalValue::String(result.into()))
    }

    pub fn string_fmt(arguments: &[EvalValue], formula: &mut FormulaEngine, env: &Env) -> Result<EvalValue> {
        let Some(EvalValue::String(fmtstr)) = arguments.get(0) else {
            bail!("Le premier argument de fmt doit être une chaîne.");
        };
        let local_env = match arguments.get(1) {
            None => Env::new().with_parent(env),
            Some(EvalValue::Object(obj)) => Env::from_arc(obj.clone()).with_parent(env),
            Some(_) => bail!("L'environement de fmt doit être un dictionnaire."),
        };
        let res = formula.format(fmtstr.clone(), &local_env)?;
        Ok(EvalValue::String(res))
    }

    /// Returns a substring of a string, given start and end indices.
    pub fn string_slice(arguments: &[EvalValue], formula: &mut FormulaEngine, env: &Env) -> Result<EvalValue> {
        expect_arity("slice", arguments, 3)?;
        let value = string_argument("slice", arguments)?;
        let start = arguments[1].to_i64()?;
        let end = arguments[2].to_i64()?;
        let len = value.chars().count() as i64;
        let start = if start < 0 { len + start } else { start };
        let end = if end < 0 { len + end } else { end };
        if start < 0 || end < 0 || start > len || end > len || start > end {
            bail!("slice indices are out of bounds");
        }
        let result: String = value.chars().skip(start as usize).take((end - start) as usize).collect();
        Ok(EvalValue::String(result.into()))
    }

    //////// NUMBER METHODS \\\\\\\
    lazy_static! {
        pub static ref INT_METHODS: IndexMap<Arc<str>, EvalValue> = {
            let mut map = IndexMap::new();
            build_dict(&mut map, "int", "abs", int_abs);
            build_dict(&mut map, "int", "pow", int_pow);
            build_dict(&mut map, "int", "clamp", int_clamp);
            build_dict(&mut map, "int", "sign", int_sign);
            build_dict(&mut map, "int", "lerp", int_lerp);
            build_dict(&mut map, "int", "divmod", int_divmod);
            build_dict(&mut map, "int", "max", max);
            build_dict(&mut map, "int", "min", min);
            build_dict(&mut map, "int", "rotr", int_rotr);
            build_dict(&mut map, "int", "rotl", int_rotl);
            map
        };
        pub static ref FLOAT_METHODS: IndexMap<Arc<str>, EvalValue> = {
            let mut map = IndexMap::new();
            build_dict(&mut map, "float", "round", float_round);
            build_dict(&mut map, "float",  "abs", float_abs);
            build_dict(&mut map, "float", "pow", float_pow);
            build_dict(&mut map, "float", "ceil", float_ceil);
            build_dict(&mut map, "float", "floor", float_floor);
            build_dict(&mut map, "float", "clamp", float_clamp);
            build_dict(&mut map, "float", "sign", float_sign);
            build_dict(&mut map, "float", "lerp", float_lerp);
            build_dict(&mut map, "float", "max", max);
            build_dict(&mut map, "float", "min", min);
            build_dict(&mut map, "float", "bits", float_bits);
            map
        };
    }

    fn int_argument(name: &str, arguments: &[EvalValue]) -> Result<i64> {
        match arguments.first() {
            Some(EvalValue::Int(value)) => Ok(*value),
            _ => bail!("{name} exige un entier comme premier argument"),
        }
    }

    fn float_argument(name: &str, arguments: &[EvalValue]) -> Result<f64> {
        match arguments.first() {
            Some(EvalValue::Float(value)) => Ok(*value),
            _ => bail!("{name} exige un flottant comme premier argument"),
        }
    }

    pub fn int_abs(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("abs", arguments, 1)?;
        Ok(EvalValue::Int(int_argument("abs", arguments)?.abs()))
    }
    pub fn int_pow(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("pow", arguments, 2)?;
        power_values(&arguments[0], &arguments[1])
    }
    pub fn int_clamp(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("clamp", arguments, 3)?;
        let value = int_argument("clamp", arguments)?;
        let minimum = arguments[1].to_i64()?;
        let maximum = arguments[2].to_i64()?;
        if minimum > maximum {
            bail!("clamp exige une borne minimale inférieure ou égale à la borne maximale")
        }
        Ok(EvalValue::Int(value.clamp(minimum, maximum)))
    }
    pub fn int_sign(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("sign", arguments, 1)?;
        Ok(EvalValue::Int(int_argument("sign", arguments)?.signum()))
    }
    pub fn int_lerp(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("lerp", arguments, 3)?;
        let factor = int_argument("lerp", arguments)? as f64;
        let start = arguments[1].to_f64()?;
        let end = arguments[2].to_f64()?;
        Ok(EvalValue::Float(start + (end - start) * factor))
    }
    pub fn int_divmod(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("divmod", arguments, 2)?;
        let value = int_argument("divmod", arguments)?;
        let divisor = arguments[1].to_i64()?;
        if divisor == 0 {
            bail!("divmod ne peut pas diviser par zéro")
        }
        Ok(EvalValue::Array(
            vec![
                EvalValue::Int(value.div_euclid(divisor)),
                EvalValue::Int(value.rem_euclid(divisor)),
            ]
            .into(),
        ))
    }
    pub fn int_rotr(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("rotr", arguments, 2)?;
        let value = int_argument("rotr", arguments)?;
        let shift = match &arguments[1] {
            EvalValue::Int(i) => *i,
            _ => bail!("rotr exige un entier comme décalage"),
        };
        if shift < 0 {
            bail!("rotr exige un décalage non négatif")
        }
        let result = value.rotate_right(shift as u32);
        Ok(EvalValue::Int(result))
    }
    pub fn int_rotl(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("rotl", arguments, 2)?;
        let value = int_argument("rotl", arguments)?;
        let shift = match &arguments[1] {
            EvalValue::Int(i) => *i,
            _ => bail!("rotl exige un entier comme décalage"),
        };
        if shift < 0 {
            bail!("rotl exige un décalage non négatif")
        }
        let result = value.rotate_left(shift as u32);
        Ok(EvalValue::Int(result))
    }

    pub fn float_round(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        if !(1..=2).contains(&arguments.len()) {
            bail!("round exige zéro ou un argument après le flottant")
        }
        let value = float_argument("round", arguments)?;
        let digits = arguments
            .get(1)
            .map(EvalValue::to_i64)
            .transpose()?
            .unwrap_or(0);
        let rounded = python_round(value, digits)?;
        if digits <= 0 {
            Ok(EvalValue::Int(rounded as i64))
        } else {
            Ok(EvalValue::Float(rounded))
        }
    }
    pub fn float_abs(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("abs", arguments, 1)?;
        Ok(EvalValue::Float(float_argument("abs", arguments)?.abs()))
    }
    pub fn float_pow(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("pow", arguments, 2)?;
        power_values(&arguments[0], &arguments[1])
    }
    pub fn float_ceil(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("ceil", arguments, 1)?;
        Ok(EvalValue::Int(
            float_argument("ceil", arguments)?.ceil() as i64
        ))
    }
    pub fn float_floor(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("floor", arguments, 1)?;
        Ok(EvalValue::Int(
            float_argument("floor", arguments)?.floor() as i64
        ))
    }
    pub fn float_clamp(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("clamp", arguments, 3)?;
        let value = float_argument("clamp", arguments)?;
        let minimum = arguments[1].to_f64()?;
        let maximum = arguments[2].to_f64()?;
        if minimum > maximum {
            bail!("clamp exige une borne minimale inférieure ou égale à la borne maximale")
        }
        Ok(EvalValue::Float(value.clamp(minimum, maximum)))
    }
    pub fn float_sign(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("sign", arguments, 1)?;
        Ok(EvalValue::Int(
            float_argument("sign", arguments)?.signum() as i64
        ))
    }
    pub fn float_lerp(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("lerp", arguments, 3)?;
        let factor = float_argument("lerp", arguments)?;
        let start = arguments[1].to_f64()?;
        let end = arguments[2].to_f64()?;
        Ok(EvalValue::Float(start + (end - start) * factor))
    }
    pub fn float_bits(
        arguments: &[EvalValue],
        _formula: &mut FormulaEngine,
        _env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("bits", arguments, 1)?;
        let value = float_argument("bits", arguments)?;
        Ok(EvalValue::Int(value.to_bits() as i64))
    }

    //////// CHARACTER METHODS \\\\\\\
    lazy_static! {
        pub static ref CHARACTER_METHODS: IndexMap<Arc<str>, EvalValue> = {
            let mut map = IndexMap::new();
            build_dict(&mut map, "character", "obtenir_pv_max", char_max_hp);
            build_dict(&mut map, "character", "obtenir_moral_max", char_max_morale);
            build_dict(&mut map, "character", "obtenir_attribut_effectif", char_actual_attribute);
            build_dict(&mut map, "character", "selectionne", char_select);
            map
        };
    }
    pub fn char_max_hp(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("obtenir_pv_max", &arguments, 1)?;
        let character = match &arguments[0] {
            EvalValue::Character(character) => character,
            _ => bail!("obtenir_pv_max exige un personnage comme argument"),
        };
        Ok(EvalValue::Int(character.max_pv))
    }
    pub fn char_max_morale(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("obtenir_moral_max", &arguments, 1)?;
        let character = match &arguments[0] {
            EvalValue::Character(character) => character,
            _ => bail!("obtenir_moral_max exige un personnage comme argument"),
        };
        Ok(EvalValue::Int(character.max_moral))
    }
    pub fn char_actual_attribute(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        expect_arity("obtenir_attribut_actuel", &arguments, 2)?;
        let character = match &arguments[0] {
            EvalValue::Character(character) => character,
            _ => bail!("obtenir_attribut_actuel exige un personnage comme premier argument"),
        };
        let attribute_name = match &arguments[1] {
            EvalValue::String(s) => s.as_ref(),
            _ => bail!("obtenir_attribut_actuel exige une chaîne comme second argument"),
        };
        match character.attributes.get(attribute_name) {
            Some(value) => Ok(value.clone()),
            None => bail!("L'attribut '{attribute_name}' n'existe pas pour ce personnage"),
        }
    }
    pub fn char_select(
        arguments: &[EvalValue],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<EvalValue> {
        let character = match &arguments[0] {
            EvalValue::Character(character) => character,
            _ => bail!("selectionne exige un personnage comme premier argument"),
        };
        let arguments = &arguments[1..];
        if !(1..=2).contains(&arguments.len()) {
            bail!(
                "selectionne exige un type et, facultativement, un filtre; {} argument(s) fourni(s)",
                arguments.len()
            )
        }
        let kind = match &arguments[0] {
            EvalValue::String(kind) => kind.as_ref(),
            _ => bail!("Le premier argument de selectionne doit être une chaîne"),
        };
        if arguments
            .get(1)
            .is_some_and(|filter| !matches!(filter, EvalValue::Object(_)))
        {
            bail!("Le second argument de selectionne doit être un filtre dictionnaire")
        }
        formula.select_character_entities(character, kind, arguments.get(1))
    }

    lazy_static!{
        pub static ref ALL_BUILTINS: IndexMap<Arc<str>, EvalValue> = {
            let mut map = IndexMap::new();
            insert_registry(&mut map, &GLOBAL_ENV.vars);
            insert_registry(&mut map, &ARRAY_METHODS);
            insert_registry(&mut map, &OBJECT_METHODS);
            insert_registry(&mut map, &STRING_METHODS);
            insert_registry(&mut map, &INT_METHODS);
            insert_registry(&mut map, &FLOAT_METHODS);
            insert_registry(&mut map, &CHARACTER_METHODS);
            map
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates the deterministic formula engine used by parser and evaluator tests.
    fn engine() -> FormulaEngine {
        FormulaEngine::new(Some(1))
    }

    // Verifies that arithmetic and floor division.
    #[test]
    fn arithmetic_and_floor_division() {
        let mut engine = engine();
        let env = Env::new();
        assert_eq!(
            engine.evaluate("2 + 3 * 4", &env).unwrap(),
            EvalValue::Int(14)
        );
        assert_eq!(
            engine.evaluate("-5 // 2", &env).unwrap(),
            EvalValue::Int(-3)
        );
    }

    // Verifies that python ternary and membership.
    #[test]
    fn python_ternary_and_membership() {
        let mut engine = engine();
        let mut env = Env::new();
        env.insert("chance", EvalValue::Int(3));
        env.insert(
            "traits",
            EvalValue::Array(vec![EvalValue::String("danger".into())].into()),
        );
        assert_eq!(
            engine
                .evaluate("'or' if chance >= 2 else 'blanc'", &env)
                .unwrap(),
            EvalValue::String("or".into())
        );
        assert_eq!(
            engine.evaluate("'danger' in traits", &env).unwrap(),
            EvalValue::Bool(true)
        );
        assert_eq!(
            engine
                .evaluate("(fn value -> value in traits)('danger')", &env)
                .unwrap(),
            EvalValue::Bool(true)
        );
        assert_eq!(
            engine.evaluate("'safe' not in traits", &env).unwrap(),
            EvalValue::Bool(true)
        );

        let membership = Parser::new(
            Tokenizer::new("'danger' in traits")
                .tokenize()
                .unwrap(),
        )
        .parse()
        .unwrap();
        let non_membership = Parser::new(
            Tokenizer::new("'safe' not in traits")
                .tokenize()
                .unwrap(),
        )
        .parse()
        .unwrap();
        assert!(membership.to_string().contains(" in "));
        assert!(non_membership.to_string().contains(" not in "));
    }

    // Verifies exponentiation with integral and fractional results.
    #[test]
    fn pow_function() {
        let mut engine = engine();
        let env = Env::new();
        assert_eq!(
            engine.evaluate("pow(2, 10)", &env).unwrap(),
            EvalValue::Int(1024)
        );
        assert_eq!(
            engine.evaluate("pow(9, 0.5)", &env).unwrap(),
            EvalValue::Float(3.0)
        );
        assert_eq!(
            engine.evaluate("pow(2, -2)", &env).unwrap(),
            EvalValue::Float(0.25)
        );
        assert!(engine.evaluate("pow(-1, 0.5)", &env).is_err());
    }

    #[test]
    fn logical_aliases_preserve_values_and_short_circuiting() {
        let mut engine = engine();
        let env = Env::new();
        for (source, expected) in [
            ("False && (1 / 0)", EvalValue::Bool(false)),
            ("True || (1 / 0)", EvalValue::Bool(true)),
            ("0 && missing", EvalValue::Int(0)),
            ("7 || missing", EvalValue::Int(7)),
            ("2 && 3", EvalValue::Int(3)),
            ("0 || 3", EvalValue::Int(3)),
            ("'' || 'Anonymous'", EvalValue::String("Anonymous".into())),
            ("0 or 2 && 3", EvalValue::Int(3)),
            ("0 || 2 and 3", EvalValue::Int(3)),
            ("1 || 0 && missing", EvalValue::Int(1)),
            ("1 & 3 && 0 | 4", EvalValue::Int(4)),
            ("'&& || ! xor'", EvalValue::String("&& || ! xor".into())),
        ] {
            assert_eq!(engine.evaluate(source, &env).unwrap(), expected, "{source}");
        }
        for source in ["True && missing", "False || missing", "True &&& False", "True ||| False"] {
            assert!(engine.evaluate(source, &env).is_err(), "{source}");
        }
    }

    #[test]
    fn logical_negation_precedence_and_format_roundtrip() {
        let mut engine = engine();
        let env = Env::new();
        for (source, expected) in [
            ("!2 == 1", false),
            ("not 2 == 1", true),
            ("!(2 == 1)", true),
            ("!!2", true),
            ("!None", true),
            ("![]", true),
            ("![False][0]", true),
            ("2 != 1 && !False", true),
            ("not 2 in [2]", false),
            ("2 not in [1]", true),
        ] {
            assert_eq!(engine.evaluate(source, &env).unwrap(), EvalValue::Bool(expected), "{source}");
            let ast = Parser::new(Tokenizer::new(source).tokenize().unwrap()).parse().unwrap();
            assert_eq!(engine.evaluate(&ast.to_string(), &env).unwrap(), EvalValue::Bool(expected), "formatted {source}");
        }
    }

    #[test]
    fn logical_xor_truthiness_precedence_and_evaluation() {
        let mut engine = engine();
        let env = Env::new();
        for (source, expected) in [
            ("False xor False", false),
            ("False xor True", true),
            ("True xor False", true),
            ("True xor True", false),
            ("2 xor 3", false),
            ("[] xor 'present'", true),
            ("True xor True xor True", true),
            ("True or False xor True", true),
            ("True || False xor True", true),
            ("True xor True and False", true),
            ("True xor True && False", true),
            ("2 == 2 xor 3 == 4", true),
            ("not True xor True", true),
        ] {
            assert_eq!(engine.evaluate(source, &env).unwrap(), EvalValue::Bool(expected), "{source}");
            let ast = Parser::new(Tokenizer::new(source).tokenize().unwrap()).parse().unwrap();
            let serialized = serde_json::to_string(&ExprSerializer::from(&ast)).unwrap();
            let restored = Expr::try_from(serde_json::from_str::<ExprSerializer>(&serialized).unwrap()).unwrap();
            assert_eq!(engine.evaluate(&restored.to_string(), &env).unwrap(), EvalValue::Bool(expected), "roundtrip {source}");
        }
        for source in ["True xor missing", "False xor missing", "let xor = 1 for xor", "fn xor -> xor"] {
            assert!(engine.evaluate(source, &env).is_err(), "{source}");
        }
        // Different predicates expose operand order; the next roll exposes extra evaluations.
        for seed in 0..16 {
            let mut actual = FormulaEngine::new(Some(seed));
            let mut reference = FormulaEngine::new(Some(seed));
            let left = reference.evaluate("dice(1, 6) > 2", &env).unwrap().truthy();
            let right = reference.evaluate("dice(1, 6) > 4", &env).unwrap().truthy();
            assert_eq!(actual.evaluate("dice(1, 6) > 2 xor dice(1, 6) > 4", &env).unwrap(), EvalValue::Bool(left != right));
            assert_eq!(actual.evaluate("dice(1, 6)", &env).unwrap(), reference.evaluate("dice(1, 6)", &env).unwrap());
        }
    }

    #[test]
    fn hash_hex_literals_match_hex_prefix() {
        let mut engine = engine();
        let env = Env::new();
        for digits in [
            "0",
            "f",
            "FFF",
            "12aBcD",
            "0000FF",
            "2d6",
            "7FFFFFFFFFFFFFFF",
        ] {
            let hash = format!("#{digits}");
            let hex = format!("0x{digits}");
            assert_eq!(
                Tokenizer::new(&hash).tokenize().unwrap(),
                Tokenizer::new(&hex).tokenize().unwrap()
            );
            assert_eq!(
                engine.evaluate(&hash, &env).unwrap(),
                engine.evaluate(&hex, &env).unwrap()
            );
        }
        assert_eq!(
            engine.evaluate("(#123456 >> 8) & #FF", &env).unwrap(),
            EvalValue::Int(0x34)
        );
        assert_eq!(
            engine.evaluate("-#FF + #100", &env).unwrap(),
            EvalValue::Int(1)
        );
        assert_eq!(
            engine.evaluate("${#FFFFFF}", &env).unwrap(),
            EvalValue::Int(0xFFFFFF)
        );
        assert_eq!(
            engine.evaluate("'#123456'", &env).unwrap(),
            EvalValue::String("#123456".into())
        );
        assert_eq!(
            engine.format("value=${#FF}".into(), &env).unwrap().as_ref(),
            "value=255"
        );
    }

    #[test]
    fn hash_hex_literals_reject_invalid_digits_and_overflow() {
        let mut engine = engine();
        for digits in [
            "",
            "G",
            "12G4",
            "FF_FF",
            "8000000000000000",
            "FFFFFFFFFFFFFFFF",
            "0x10",
        ] {
            for prefix in ["#", "0x"] {
                let source = format!("{prefix}{digits}");
                assert!(engine.evaluate(&source, &Env::new()).is_err(), "{source}");
            }
        }
        assert!(engine.evaluate("# FF", &Env::new()).is_err());
    }

    // Verifies bitwise operators, radix literals, and their precedence.
    #[test]
    fn bitwise_operators_and_radix_literals() {
        let mut engine = engine();
        let mut env = Env::new();
        env.insert("flags", EvalValue::Int(5));

        assert_eq!(
            engine.evaluate("0xF0 & 0b10101100", &env).unwrap(),
            EvalValue::Int(0xA0)
        );
        assert_eq!(
            engine.evaluate("0o17 ^ 0b0011", &env).unwrap(),
            EvalValue::Int(12)
        );
        assert_eq!(
            engine.evaluate("3 | 4 & 1", &env).unwrap(),
            EvalValue::Int(3)
        );
        assert_eq!(
            engine.evaluate("flags & 0x04 != 0", &env).unwrap(),
            EvalValue::Bool(true)
        );
        assert!(engine.evaluate("1.0 & 1", &env).is_err());
        assert!(engine.evaluate("0b102", &env).is_err());
    }

    /// Verifies named parameters, args, caller environment access, and function composition.
    #[test]
    fn tale_defined_functions() {
        let mut functions: IndexMap<String, FormulaFunction> = IndexMap::new();
        let func = Parser::new(Tokenizer::new("'trained' if rank == 'untrained' else 'expert' if rank == 'trained' else 'master' if rank == 'expert' else 'legend' if rank == 'master' else rank").tokenize().unwrap()).parse().unwrap();
        functions.insert(
            "adv_rank".into(),
            FormulaFunction {
                parameters: Some(vec!["rank".into()].into()),
                body: func.into(),
            },
        );
        let func = Parser::new(Tokenizer::new("args.0 + args[1]").tokenize().unwrap()).parse().unwrap();
        functions.insert(
            "sum_args".into(),
            FormulaFunction {
                parameters: None,
                body: func.into(),
            },
        );
        let func = Parser::new(Tokenizer::new("prefix + adv_rank(rank)").tokenize().unwrap()).parse().unwrap();
        functions.insert(
            "rank_label".into(),
            FormulaFunction {
                parameters: Some(vec!["rank".into()].into()),
                body: func.into(),
            },
        );
        let functions = {
            let mut env = Env::new();
            for (name, value) in functions {
                env.insert(name, EvalValue::Func(
                    Function::User(
                        FormulaClosure::without_captures(value)
                    ).into()
                ));
            }
            env.with_parent(&GLOBAL_ENV)
        };
        let mut engine = FormulaEngine::new(Some(1)).with_globals(functions);
        let mut env = Env::new();
        env.insert("prefix", EvalValue::String("Rang: ".into()));

        assert_eq!(
            engine.evaluate("adv_rank('expert')", &env).unwrap(),
            EvalValue::String("master".into())
        );
        assert_eq!(
            engine.evaluate("sum_args(4, 7)", &env).unwrap(),
            EvalValue::Int(11)
        );
        assert_eq!(
            engine.evaluate("rank_label('trained')", &env).unwrap(),
            EvalValue::String("Rang: expert".into())
        );
        assert!(engine.evaluate("adv_rank()", &env).is_err());
    }

    /// Verifies that recursive tale functions fail with a bounded call-chain error.
    #[test]
    fn tale_function_recursion_is_bounded() {
        let mut functions: IndexMap<String, FormulaFunction> = IndexMap::new();
        let func = Parser::new(Tokenizer::new("loop()").tokenize().unwrap()).parse().unwrap();
        functions.insert(
            "loop".into(),
            FormulaFunction {
                parameters: None,
                body: func.into(),
            },
        );
        let functions = {
            let mut env = Env::new();
            for (name, value) in functions {
                env.insert(name, EvalValue::Func(
                    Function::User(
                        FormulaClosure::without_captures(value)
                    ).into()
                ));
            }
            env.with_parent(&GLOBAL_ENV)
        };
        let mut engine = FormulaEngine::new(Some(1)).with_globals(functions);
        let error = engine.evaluate("loop()", &Env::new()).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("fonction `loop`") || error.contains("Profondeur maximale"));
    }

    /// Verifies anonymous function parsing, ordinary value dispatch, and extra arguments.
    #[test]
    fn anonymous_functions_are_first_class_values() {
        let mut engine = engine();
        let mut env = Env::new();
        env.insert("offset", EvalValue::Int(3));
        assert_eq!(
            engine
                .evaluate("(fn value -> value + offset)(4, 99)", &env)
                .unwrap(),
            EvalValue::Int(7)
        );
        assert_eq!(
            engine
                .evaluate(
                    "let add = fn left, right -> left + right for add(2, 5)",
                    &env
                )
                .unwrap(),
            EvalValue::Int(7)
        );
        assert!(engine
            .evaluate("(fn left, right -> left + right)(2)", &env)
            .is_err());
    }

    /// A type error identifies the offending value, the contract it violated, and every
    /// formula/function layer crossed while it bubbled up.
    #[test]
    fn nested_call_errors_keep_value_reason_and_context_chain() {
        let mut engine = engine();
        let error = engine
            .evaluate("(fn value -> int(value))([1, 2])", &Env::new())
            .unwrap_err();
        let chain = error
            .chain()
            .map(|cause| cause.to_string())
            .collect::<Vec<_>>();
        let report = chain.join("\n");

        assert!(report.contains("[1, 2] (type list)"));
        assert!(report.contains("seuls int, float, bool"));
        assert!(report.contains("builtin `global_int`"));
        assert!(report.contains("Dans fn(value)"));
        assert!(report.contains("Appel `int`"));
        assert!(report.contains("Erreur dans la formule"));
        assert!(chain.len() >= 6, "chaîne trop courte: {report}");
    }

    /// Unknown-name diagnostics explain the lookup failure and expose useful visible names.
    #[test]
    fn unknown_name_errors_list_visible_bindings() {
        let mut engine = engine();
        let mut env = Env::new();
        env.insert("personnage", EvalValue::Null);
        let error = engine.evaluate("perssonnage", &env).unwrap_err();
        let report = format!("{error:#}");

        assert!(report.contains("Nom inconnu `perssonnage`"));
        assert!(report.contains("aucune variable de ce nom"));
    }

    /// Verifies that returned functions retain outer parameters after their caller returns.
    #[test]
    fn nested_functions_capture_outer_bindings() {
        let mut engine = engine();
        let env = Env::new();

        assert_eq!(
            engine
                .evaluate("(fn x -> fn y -> x + y)(2)(3)", &env)
                .unwrap(),
            EvalValue::Int(5)
        );
        assert_eq!(
            engine
                .evaluate(
                    "let offset = 4 for (fn -> fn value -> value + offset)()(3)",
                    &env,
                )
                .unwrap(),
            EvalValue::Int(7)
        );
        assert_eq!(
            engine
                .evaluate(
                    "let funcs = [fn -> value for value in [2, 5]] for funcs[0]() + funcs[1]()",
                    &env,
                )
                .unwrap(),
            EvalValue::Int(7)
        );
    }

    /// Verifies selective capture and the deliberately dynamic tale runtime bindings.
    #[test]
    fn closures_capture_only_referenced_lexical_values() {
        let mut engine = engine();
        let mut creation_env = Env::new();
        creation_env.insert("used", EvalValue::Int(4));
        creation_env.insert("unused", EvalValue::Int(99));
        creation_env.insert("recit", EvalValue::Int(1));
        creation_env.insert("personnage", EvalValue::Int(1));
        creation_env.insert("page", EvalValue::Int(1));
        creation_env.insert("objet", EvalValue::String("ancien objet".into()));
        creation_env.insert("etat", EvalValue::String("ancien état".into()));

        let closure = engine
            .evaluate(
                "fn value -> [value, used, recit, personnage, page, objet, etat]",
                &creation_env,
            )
            .unwrap();
        let EvalValue::Func(function) = &closure else {
            panic!("une expression fn doit produire une fonction")
        };
        let Function::User(function) = function.as_ref() else {
            panic!("une expression fn doit produire une fonction utilisateur")
        };
        assert_eq!(
            function
                .captures
                .keys()
                .map(|name| name.as_ref())
                .collect::<Vec<_>>(),
            vec!["used", "objet", "etat"]
        );

        let mut call_env = Env::new();
        call_env.insert("recit", EvalValue::Int(2));
        call_env.insert("personnage", EvalValue::Int(2));
        call_env.insert("page", EvalValue::Int(2));
        call_env.insert("objet", EvalValue::String("objet courant".into()));
        call_env.insert("etat", EvalValue::String("état courant".into()));
        assert_eq!(
            closure.call(&[EvalValue::Int(7)], &mut engine, &call_env).unwrap(),
            EvalValue::Array(
                vec![
                    EvalValue::Int(7),
                    EvalValue::Int(4),
                    EvalValue::Int(2),
                    EvalValue::Int(2),
                    EvalValue::Int(2),
                    EvalValue::String("objet courant".into()),
                    EvalValue::String("état courant".into()),
                ]
                .into()
            )
        );
    }

    /// Verifies entity fallbacks and explicit snapshots of otherwise dynamic bindings.
    #[test]
    fn closure_entity_fallbacks_and_explicit_snapshots() {
        let mut engine = engine();
        let mut creation_env = Env::new();
        creation_env.insert("personnage", EvalValue::Int(3));
        creation_env.insert("objet", EvalValue::String("ancien objet".into()));
        creation_env.insert("etat", EvalValue::String("ancien état".into()));

        let entities = engine
            .evaluate("fn -> [objet, etat]", &creation_env)
            .unwrap();
        assert_eq!(
            entities
                .clone()
                .call(&[], &mut engine, &Env::new())
                .unwrap(),
            EvalValue::Array(
                vec![
                    EvalValue::String("ancien objet".into()),
                    EvalValue::String("ancien état".into()),
                ]
                .into()
            )
        );

        let snapshot = engine
            .evaluate(
                "let personnage_fige = personnage for fn -> personnage_fige",
                &creation_env,
            )
            .unwrap();
        let mut call_env = Env::new();
        call_env.insert("personnage", EvalValue::Int(8));
        assert_eq!(
            snapshot.call(&[], &mut engine, &call_env).unwrap(),
            EvalValue::Int(3)
        );
    }

    #[test]
    fn binding_origins_survive_environment_merges() {
        let mut tale_values = IndexMap::new();
        tale_values.insert("x".into(), EvalValue::Int(1));
        tale_values.insert("tale_only".into(), EvalValue::Int(2));
        let tale = Env::from_tale_globals(&tale_values);
        let mut global = Env::new();
        global.insert("x", EvalValue::Int(1));
        global.insert("engine_only", EvalValue::Int(3));
        let global = global.into_engine_globals();

        for merged in [
            Env::new().with_parent(&global).with_parent(&tale),
            Env::merge(&[&tale, &global]),
            tale.merged(&global),
            tale.clone().expand(global.clone()),
        ] {
            for env in [merged.clone(), merged.solidified()] {
                assert_eq!(env.get_binding("x").unwrap().1, BindingOrigin::EngineGlobal);
                assert_eq!(
                    env.get_binding("engine_only").unwrap().1,
                    BindingOrigin::EngineGlobal
                );
                assert_eq!(
                    env.get_binding("tale_only").unwrap().1,
                    BindingOrigin::TaleGlobal
                );
            }
        }
        let reversed = Env::new()
            .with_parent(&tale)
            .with_parent(&global)
            .solidified();
        assert_eq!(
            reversed.get_binding("x").unwrap().1,
            BindingOrigin::TaleGlobal
        );

        // All visible names become engine globals when this environment is installed.
        let installed = Env::new()
            .with_parent(&tale)
            .with_parent(&global)
            .into_engine_globals();
        assert_eq!(
            installed.get_binding("tale_only").unwrap().1,
            BindingOrigin::EngineGlobal
        );
        assert_eq!(
            installed.get_binding("engine_only").unwrap().1,
            BindingOrigin::EngineGlobal
        );
    }

    #[test]
    fn assigning_equal_values_creates_local_bindings() {
        let values = IndexMap::from([("x".into(), EvalValue::Int(1))]);
        let json = serde_json::json!({"x": 1});
        let global = Env::from_index_map(&values).into_engine_globals();
        for operation in 0..7 {
            let mut local = global.clone();
            match operation {
                0 => local.insert("x", EvalValue::Int(1)),
                1 => local.insert_json("x", &json["x"]),
                2 => local.extend(&values),
                3 => local.extend_json(json.as_object().unwrap()),
                4 => local = local.expand(Env::from_index_map(&values)),
                5 => local = local.expand(Env::from_json_map(json.as_object().unwrap())),
                6 => local = local.expand(Env::from_arc(Arc::new(values.clone()))),
                _ => unreachable!(),
            }
            assert_eq!(local.get_binding("x").unwrap().1, BindingOrigin::Local);
            assert_eq!(
                global.get_binding("x").unwrap().1,
                BindingOrigin::EngineGlobal
            );
        }
        let local = Env::from_index_map(&values);
        assert_eq!(
            global
                .merged(&local)
                .solidified()
                .get_binding("x")
                .unwrap()
                .1,
            BindingOrigin::Local
        );
    }

    /// Verifies that globals remain global unless a lexical binding shadows their name.
    #[test]
    fn closures_distinguish_globals_from_lexical_shadows() {
        let mut engine = engine();
        let env = Env::new();
        let global_sum = engine.evaluate("fn -> sum([1, 2, 3])", &env).unwrap();

        let shadow = engine.evaluate("fn values -> 99", &env).unwrap();
        let mut shadow_env = Env::new();
        shadow_env.insert("sum", shadow);
        let captured_sum = engine
            .evaluate("fn -> sum([1, 2, 3])", &shadow_env)
            .unwrap();

        assert_eq!(
            global_sum.call(&[], &mut engine, &shadow_env).unwrap(),
            EvalValue::Int(6)
        );
        assert_eq!(
            captured_sum.call(&[], &mut engine, &env).unwrap(),
            EvalValue::Int(99)
        );

        let mut initial_globals = IndexMap::new();
        initial_globals.insert("scale".into(), EvalValue::Int(2));
        let mut creation_env = Env::from_tale_globals(&initial_globals);
        creation_env.insert("recit", EvalValue::Object(initial_globals.into()));
        let tale_global = engine.evaluate("fn -> scale", &creation_env).unwrap();

        let mut current_globals = IndexMap::new();
        current_globals.insert("scale".into(), EvalValue::Int(3));
        let mut call_env = Env::new();
        call_env.insert("recit", EvalValue::Object(current_globals.into()));
        assert_eq!(
            tale_global.call(&[], &mut engine, &call_env).unwrap(),
            EvalValue::Int(3)
        );

        let mut shadowed_creation_env = creation_env.clone();
        // Even an equal-valued local binding is a lexical shadow, not the global itself.
        shadowed_creation_env.insert("scale", EvalValue::Int(2));
        let shadowed_tale_global = engine
            .evaluate("fn -> scale", &shadowed_creation_env)
            .unwrap();
        assert_eq!(
            shadowed_tale_global
                .call(&[], &mut engine, &call_env)
                .unwrap(),
            EvalValue::Int(2)
        );
    }

    /// Verifies receiver insertion, higher-order list helpers, and stored-member precedence.
    #[test]
    fn registered_methods_and_higher_order_helpers() {
        let mut engine = engine();
        let env = Env::new();

        assert_eq!(
            engine
                .evaluate("[1, 2, 3].map(fn value -> value * 2)", &env)
                .unwrap(),
            EvalValue::Array(vec![EvalValue::Int(2), EvalValue::Int(4), EvalValue::Int(6)].into())
        );
        assert_eq!(
            engine
                .evaluate("[1, 2, 3, 4].filter(fn value -> value % 2 == 0)", &env)
                .unwrap(),
            EvalValue::Array(vec![EvalValue::Int(2), EvalValue::Int(4)].into())
        );
        assert_eq!(
            engine
                .evaluate("[1, 2, 3].cumul(fn total, value -> total + value)", &env)
                .unwrap(),
            EvalValue::Array(vec![EvalValue::Int(1), EvalValue::Int(3), EvalValue::Int(6)].into())
        );
        assert_eq!(
            engine
                .evaluate("[2, 3].apply(fn left, right -> left * right)", &env)
                .unwrap(),
            EvalValue::Int(6)
        );
        assert_eq!(
            engine
                .evaluate("{'map': fn value -> value + 1}.map(4)", &env)
                .unwrap(),
            EvalValue::Int(5)
        );
    }

    /// Verifies that built-ins are supplied as a parent without explicit environment setup.
    #[test]
    fn builtins_are_ordinary_global_values() {
        let mut engine = engine();
        let env = Env::new();
        assert_eq!(
            engine.evaluate("sum([1, 2, 3])", &env).unwrap(),
            EvalValue::Int(6)
        );
        assert_eq!(
            engine
                .evaluate("apply(fn left, right -> left - right, [7, 2])", &env)
                .unwrap(),
            EvalValue::Int(5)
        );
    }

    // Verifies detection of dynamic text substitutions.
    #[test]
    fn formula_substitution_detection() {
        assert!(has_formula_substitution("$destination"));
        assert!(has_formula_substitution("acte_${numero}"));
        assert!(!has_formula_substitution("page_fixe"));
        assert!(!has_formula_substitution("$$destination"));
    }

    // Verifies that formatting and escaped dollar.
    #[test]
    fn formatting_and_escaped_dollar() {
        let mut engine = engine();
        let mut env = Env::new();
        env.insert("nom", EvalValue::String("Violet".into()));
        env.insert("pv", EvalValue::Int(7));
        assert_eq!(
            engine
                .format("$nom a ${pv + 1} PV et $$3.".into(), &env)
                .unwrap(),
            "Violet a 8 PV et $3.".into()
        );
    }

    // Verifies list literals, concatenation, comprehensions, filters, and destructuring.
    #[test]
    fn list_expressions_and_comprehensions() {
        let mut engine = engine();
        let mut env = Env::new();
        env.insert(
            "nums",
            EvalValue::Array(vec![EvalValue::Int(1), EvalValue::Int(2), EvalValue::Int(3)].into()),
        );
        env.insert(
            "pairs",
            EvalValue::Array(
                vec![
                    EvalValue::Array(vec![EvalValue::Int(1), EvalValue::Int(4)].into()),
                    EvalValue::Array(vec![EvalValue::Int(2), EvalValue::Int(5)].into()),
                ]
                .into(),
            ),
        );
        assert_eq!(
            engine
                .evaluate("[0] + [i + 1 for i in nums if i > 1]", &env)
                .unwrap(),
            EvalValue::Array(vec![EvalValue::Int(0), EvalValue::Int(3), EvalValue::Int(4)].into())
        );
        assert_eq!(
            engine
                .evaluate("[key + val for key, val in pairs]", &env)
                .unwrap(),
            EvalValue::Array(vec![EvalValue::Int(5), EvalValue::Int(7)].into())
        );
    }

    // Verifies Python-like list reducers, including their empty-list identities.
    #[test]
    fn list_reducer_methods() {
        let mut engine = engine();
        let env = Env::new();
        assert_eq!(
            engine.evaluate("[1, 2, 3].sum()", &env).unwrap(),
            EvalValue::Int(6)
        );
        assert_eq!(
            engine.evaluate("[1, 2.5, True].sum()", &env).unwrap(),
            EvalValue::Float(4.5)
        );
        assert_eq!(
            engine.evaluate("[].sum()", &env).unwrap(),
            EvalValue::Int(0)
        );
        assert_eq!(
            engine.evaluate("[1, 'oui', True].all()", &env).unwrap(),
            EvalValue::Bool(true)
        );
        assert_eq!(
            engine.evaluate("[0, '', 4].any()", &env).unwrap(),
            EvalValue::Bool(true)
        );
        assert_eq!(
            engine.evaluate("[].all()", &env).unwrap(),
            EvalValue::Bool(true)
        );
        assert_eq!(
            engine.evaluate("[].any()", &env).unwrap(),
            EvalValue::Bool(false)
        );
    }

    // Verifies scoped, sequential single and multiple `let` bindings.
    #[test]
    fn let_expressions_are_scoped_and_sequential() {
        let mut engine = engine();
        let mut env = Env::new();
        env.insert("x", EvalValue::Int(20));
        assert_eq!(
            engine
                .evaluate("let x = 2, y = x + 3 for x * y", &env)
                .unwrap(),
            EvalValue::Int(10)
        );
        assert_eq!(
            engine
                .evaluate("let x = 2 for let y = x + 1 for [x, y]", &env)
                .unwrap(),
            EvalValue::Array(vec![EvalValue::Int(2), EvalValue::Int(3)].into())
        );
        assert_eq!(env.get("x"), Some(&EvalValue::Int(20)));
        assert!(env.get("y").is_none());
    }

    // Verifies dictionary literals, right-biased merging, and key/value comprehensions.
    #[test]
    fn dictionary_expressions_and_comprehensions() {
        let mut engine = engine();
        let mut numbers = IndexMap::new();
        numbers.insert("un".into(), EvalValue::Int(1));
        numbers.insert("deux".into(), EvalValue::Int(2));
        let mut env = Env::new();
        env.insert("nums", EvalValue::Object(numbers.into()));

        let mut expected_merge = IndexMap::new();
        expected_merge.insert("a".into(), EvalValue::Int(3));
        expected_merge.insert("b".into(), EvalValue::Int(2));
        assert_eq!(
            engine
                .evaluate("{'a': 1} + {'b': 2, 'a': 3}", &env)
                .unwrap(),
            EvalValue::Object(expected_merge.into())
        );

        assert_eq!(
            engine
                .evaluate("{key: val + 1 for key, val in nums if val > 1}", &env)
                .unwrap(),
            EvalValue::Object(IndexMap::from([("deux".into(), EvalValue::Int(3),)]).into())
        );
    }

    // Verifies character selection with unchanged tale item, state, and tag filters.
    #[test]
    fn character_selection_uses_tale_filters() {
        let character = CharacterView {
            name: Some("Violet".into()),
            pv: 8,
            max_pv: 12,
            moral: 9,
            max_moral: 12,
            attributes: Arc::from(IndexMap::new()),
            tags: Arc::from(IndexMap::from([
                ("reputation".into(), EvalValue::Int(4)),
                ("secret".into(), EvalValue::Bool(false)),
                ("id".into(), EvalValue::String("collision".into())),
                ("tag".into(), EvalValue::String("collision".into())),
            ])),
            items: vec![
                FormulaItemView {
                    id: "potion".into(),
                    name: split_fmt_str("Potion ${qualite}").unwrap(),
                    description: split_fmt_str("Rend ${soin} PV").unwrap(),
                    quantity: 2,
                    traits: Arc::from(vec!["soin".into()]),
                    data: Arc::from(IndexMap::from([
                        ("qualite".into(), EvalValue::String("fine".into())),
                        ("soin".into(), EvalValue::Int(5)),
                    ])),
                },
                FormulaItemView {
                    id: "cle".into(),
                    name: "Clé".into(),
                    description: "".into(),
                    quantity: 1,
                    traits: Arc::from(vec!["outil".into()]),
                    data: Arc::from(IndexMap::new()),
                },
            ]
            .into(),
            conditions: vec![FormulaConditionView {
                id: "inspire".into(),
                name: "Inspiré".into(),
                description: split_fmt_str("+${bonus} au prochain test").unwrap(),
                traits: vec!["positif".into()].into(),
                data: Arc::from(IndexMap::from([("bonus".into(), EvalValue::Int(2))])),
            }]
            .into(),
        };
        let mut env = Env::new();
        env.insert("personnage", EvalValue::Character(character));
        let mut engine = engine();

        assert_eq!(
            engine
                .evaluate("len(personnage.selectionne('objet'))", &env)
                .unwrap(),
            EvalValue::Int(2)
        );
        assert_eq!(
            engine
                .evaluate(
                    r#"personnage.selectionne("objet", {"type": "et", "filtres": [{"type": "quantite_superieure", "quantite": 1}, {"type": "donnee_egale", "cle": "soin", "valeur": 5}, {"type": "formule", "formule": fn -> id == "potion"}]})[0].nom"#,
                    &env,
                )
                .unwrap(),
            EvalValue::String("Potion fine".into())
        );
        assert_eq!(
            engine
                .evaluate(
                    r#"personnage.selectionne("etat", {"type": "et", "filtres": [{"type": "tous_les_traits", "traits": ["positif"]}, {"type": "formule", "formule": fn -> id == "inspire" and bonus == 2 }]})[0].description"#,
                    &env,
                )
                .unwrap(),
            EvalValue::String("+2 au prochain test".into())
        );
        assert_eq!(
            engine
                .evaluate(
                    r#"personnage.selectionne('tag', {'type': 'formule', 'formule': fn -> tag == 'reputation' and valeur > 3 })[0].valeur"#,
                    &env,
                )
                .unwrap(),
            EvalValue::Int(4)
        );
    }

    // Verifies that dictionary braces and quoted braces do not terminate inline formulas early.
    #[test]
    fn dictionary_formulas_are_delimited_correctly() {
        let mut engine = engine();
        assert_eq!(
            engine
                .format(
                    "Résultat: ${{'accolade': '}'}.accolade}.".into(),
                    &Env::new()
                )
                .unwrap(),
            "Résultat: }.".into()
        );
        assert_eq!(
            full_formula("${{'a': 1} + {'b': 2}}"),
            Some("{'a': 1} + {'b': 2}".into())
        );
        assert_eq!(full_formula("${1} ${2}"), None);

        assert_eq!(
            engine
                .evaluate(
                    r#"{'formule': '${id == 'potion' and cible == $cible}'}"#,
                    &Env::new(),
                )
                .unwrap(),
            EvalValue::Object(
                IndexMap::from([(
                    "formule".into(),
                    EvalValue::String("${id == 'potion' and cible == $cible}".into()),
                )])
                .into()
            )
        );
        assert_eq!(
            engine
                .evaluate(r##"{"formule": "${id == "potion"}"}"##, &Env::new(),)
                .unwrap(),
            EvalValue::Object(
                IndexMap::from([(
                    "formule".into(),
                    EvalValue::String("${id == \"potion\"}".into()),
                )])
                .into()
            )
        );
    }
}
