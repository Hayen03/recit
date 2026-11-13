//! Resolved poll schemas passed from page effects to the interactive engine.
//!
//! Formula-backed defaults and choices are resolved before these types are constructed.
//! User input is still parsed against the declared primitive data type.
use std::{str::FromStr, sync::Arc};

/// Identifies the primitive answer type accepted by a poll.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PollRequestDataType {
    Str,
    Int,
    Bool,
    Float,
}
impl FromStr for PollRequestDataType {
    type Err = anyhow::Error;
    /// Parses the value from its textual representation.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "str" | "chaine" => Ok(Self::Str),
            "int" | "entier" => Ok(Self::Int),
            "bool" | "booleen" => Ok(Self::Bool),
            "float" | "flottant" => Ok(Self::Float),
            _ => Err(anyhow::anyhow!("Type de données de sondage inconnu: {s}")),
        }
    }
}
/// Stores one resolved option displayed by a poll.
#[derive(Clone, Debug)]
pub struct PollRequestOption<T: std::fmt::Debug + Clone> {
    pub value: T,
    pub label: Arc<str>,
}
/// Describes either free-form input or a list of poll options.
#[derive(Clone, Debug)]
pub enum PollRequestData {
    Str {
        choices: Option<Vec<PollRequestOption<Arc<str>>>>,
        default: Option<Arc<str>>,
    },
    Int {
        choices: Option<Vec<PollRequestOption<i64>>>,
        default: Option<i64>,
    },
    Bool {
        default: Option<bool>,
    },
    Float {
        choices: Option<Vec<PollRequestOption<f64>>>,
        default: Option<f64>,
    },
}
/// Contains the prompt, destination tag, default, and answer schema for a poll.
#[derive(Clone, Debug)]
pub struct PollRequest {
    pub prompt: Arc<str>,
    pub data: PollRequestData,
}
