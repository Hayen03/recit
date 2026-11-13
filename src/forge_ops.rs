//! Small raw-JSON mutation helpers shared by Forge command handlers.
//!
//! These functions update collections but do not mark a [`crate::document::TaleDocument`] dirty or refresh
//! validation. Callers perform those application-level steps after a successful change.
use anyhow::{bail, Result};
use serde_json::{json, Value};
/// Clones array entry `index` and inserts the clone immediately after it.
///
/// # Errors
///
/// Returns an error when `index` is outside the array.
pub fn duplicate_array_entry(container: &mut Value, index: usize) -> Result<usize> {
    let values = array_mut(container)?;
    let Some(value) = values.get(index).cloned() else {
        bail!("Index {index} hors limites");
    };
    let inserted = index + 1;
    values.insert(inserted, value);
    Ok(inserted)
}
/// Moves array entry `index` to `destination` and returns its final index.
///
/// The destination is clamped to the last valid position. Arrays with at most one entry are unchanged.
///
/// # Errors
///
/// Returns an error when `index` is outside the array.
pub fn move_array_entry(container: &mut Value, index: usize, destination: usize) -> Result<usize> {
    let values = array_mut(container)?;
    if index >= values.len() {
        bail!("Index {index} hors limites");
    }
    if values.len() <= 1 {
        return Ok(index);
    }
    let destination = destination.min(values.len() - 1);
    if index == destination {
        return Ok(index);
    }
    let value = values.remove(index);
    values.insert(destination, value);
    Ok(destination)
}
/// Copies or moves one entry from `source` to the end of `target`.
///
/// When `copy` is false, the source entry is removed after it is cloned. The returned index is the new
/// position in `target`.
///
/// # Errors
///
/// Returns an error when `index` is outside the source array.
pub fn transfer_array_entry(
    source: &mut Value,
    target: &mut Value,
    index: usize,
    copy: bool,
) -> Result<usize> {
    let value = {
        let source_values = array_mut(source)?;
        let Some(value) = source_values.get(index).cloned() else {
            bail!("Index {index} hors limites");
        };
        if !copy {
            source_values.remove(index);
        }
        value
    };
    let target_values = array_mut(target)?;
    let index = target_values.len();
    target_values.push(value);
    Ok(index)
}

/// Returns the mutable array, replacing a non-array value with an empty array.
fn array_mut(value: &mut Value) -> Result<&mut Vec<Value>> {
    if !value.is_array() {
        *value = json!([]);
    }
    value
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("La valeur doit être un tableau JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verifies that duplicate and move preserve unknown json.
    #[test]
    fn duplicate_and_move_preserve_unknown_json() {
        let mut values = json!([
            {"type": "effet_futur", "configuration": {"x": 1}},
            {"type": "modifier_pv", "montant": -1}
        ]);
        duplicate_array_entry(&mut values, 0).unwrap();
        assert_eq!(values.as_array().unwrap().len(), 3);
        assert_eq!(values[1]["configuration"]["x"], 1);
        move_array_entry(&mut values, 1, 2).unwrap();
        assert_eq!(values[2]["type"], "effet_futur");
    }

    // Verifies that transfer can copy or move.
    #[test]
    fn transfer_can_copy_or_move() {
        let mut pre = json!([
            {"libelle": "PV", "effets": [{"type": "modifier_pv", "montant": -1}]},
            {"libelle": "Moral", "effets": [{"type": "modifier_moral", "valeur": -1}]}
        ]);
        let mut post = json!([]);
        transfer_array_entry(&mut pre, &mut post, 0, true).unwrap();
        assert_eq!(pre.as_array().unwrap().len(), 2);
        assert_eq!(post.as_array().unwrap().len(), 1);
        assert_eq!(post[0]["libelle"], "PV");
        transfer_array_entry(&mut pre, &mut post, 1, false).unwrap();
        assert_eq!(pre.as_array().unwrap().len(), 1);
        assert_eq!(post.as_array().unwrap().len(), 2);
    }
}
