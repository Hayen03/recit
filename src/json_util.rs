//! Contextual helpers for extracting typed fields from raw JSON.
//!
//! Absent optional fields return the documented neutral value. Present fields with the
//! wrong JSON type always return an error containing the supplied tale context.
use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

/// Returns a JSON object reference or a contextual type error.
pub fn object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| anyhow!("{context} doit être un objet JSON"))
}
/// Returns a required string field.
///
/// `context` is included in the error message to identify the containing tale object.
///
/// # Errors
///
/// Returns an error when the field is absent or not a string.
pub fn required_string(map: &Map<String, Value>, key: &str, context: &str) -> Result<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{context} doit inclure un champ `{key}` textuel"))
}
/// Returns an optional string field.
///
/// Missing and explicit `null` values return `None`.
///
/// # Errors
///
/// Returns an error when a present value is neither a string nor `null`.
pub fn optional_string(map: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => bail!("Le champ `{key}` doit être une chaîne ou null"),
    }
}
/// Returns a string field, or `default` when the field is absent.
///
/// # Errors
///
/// Returns an error when a present value is not a string.
pub fn string_or(map: &Map<String, Value>, key: &str, default: &str) -> Result<String> {
    match map.get(key) {
        None => Ok(default.to_owned()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => bail!("Le champ `{key}` doit être une chaîne"),
    }
}

/// Returns a JSON array field or an empty slice when it is absent.
pub fn array_or_empty<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a [Value]> {
    match map.get(key) {
        None => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        Some(_) => bail!("Le champ `{key}` doit être un tableau"),
    }
}
/// Returns a cloned object field, or an empty object when absent.
///
/// # Errors
///
/// Returns an error when a present value is not an object.
pub fn object_or_empty(map: &Map<String, Value>, key: &str) -> Result<Map<String, Value>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(values)) => Ok(values.clone()),
        Some(other) => bail!("Le champ `{key}` doit être un objet (reçu {:?})", other),
    }
}
/// Parses a string array, optionally accepting one string as a one-element list.
///
/// # Errors
///
/// Returns an error when the input has the wrong shape or contains a non-string element.
pub fn string_list(value: &Value, context: &str, allow_single: bool) -> Result<Vec<String>> {
    match value {
        Value::String(value) if allow_single => Ok(vec![value.clone()]),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("{context} doit contenir uniquement des chaînes"))
            })
            .collect(),
        _ => bail!(
            "{context} doit être un tableau de chaînes{}",
            if allow_single { " ou une chaîne" } else { "" }
        ),
    }
}

/// Returns a cloned JSON field or the supplied default value.
pub fn value_or(map: &Map<String, Value>, key: &str, default: Value) -> Value {
    map.get(key).cloned().unwrap_or(default)
}
/// Returns a cloned required field without constraining its JSON type.
///
/// `context` is included in the missing-field error.
///
/// # Errors
///
/// Returns an error when the field is absent.
pub fn required_value(map: &Map<String, Value>, key: &str, context: &str) -> Result<Value> {
    map.get(key)
        .cloned()
        .ok_or_else(|| anyhow!("{context} doit inclure un champ `{key}`"))
}
