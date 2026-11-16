//! Mutable character state, resource clamping, stacking rules, and save conversion.
//!
//! Runtime items and states retain references to definitions from the loaded tale.
//! Restoring a save therefore assumes the same tale definitions are available, while
//! current HP, morale, tags, inventory data, and condition data come from the save.
use std::sync::Arc;

use crate::effects::{TagPathSegment};
use crate::formula::{dict_to_map, map_to_dict, mk_condition_env, mk_partial_character_env};
use crate::model::{ConditionInstance, ItemStack, Modifier, Tale};
use anyhow::{anyhow, bail, Context, Result};
use formulas::{Env, EvalKey, EvalValue, EvalValueSerializer, FormulaEngine};
use indexmap::IndexMap;
use indexmap::map::Entry;
use serde::{Deserialize, Serialize};

/// Stores the mutable runtime state of the player character.
#[derive(Clone, Debug)]
pub struct Character {
    pub name: Option<Arc<str>>,
    pub attributes: IndexMap<Arc<str>, i64>,
    pub inventory: Vec<ItemStack>,
    pub max_hp: i64,
    pub hp: i64,
    pub max_morale: i64,
    pub morale: i64,
    pub conditions: Vec<ConditionInstance>,
    pub story_tags: Arc<IndexMap<EvalKey, EvalValue>>,
}

impl Character {
    /// Creates a character with no initial story tags.
    ///
    /// Most game code should prefer [`Character::new_with_tags`]; this
    /// constructor remains useful for tests and embedding scenarios.
    ///
    /// Every listed attribute starts at zero; HP and morale start at their default maximum of 12.
    pub fn new(attributes: &[Arc<str>], name: Option<Arc<str>>) -> Self {
        Self::new_with_tags(attributes, name, &IndexMap::new())
    }

    /// Creates a character and copies the tale's initial mutable story tags.
    ///
    /// Every listed attribute starts at zero, initial tags are cloned, and HP/morale start at 12.
    pub fn new_with_tags(
        attributes: &[Arc<str>],
        name: Option<Arc<str>>,
        initial_tags: &IndexMap<EvalKey, EvalValue>,
    ) -> Self {
        Self {
            name,
            attributes: attributes
                .iter()
                .cloned()
                .map(|attribute| (attribute.into(), 0))
                .collect(),
            inventory: Vec::new(),
            max_hp: 12,
            hp: 12,
            max_morale: 12,
            morale: 12,
            conditions: Vec::new(),
            story_tags: Arc::new(initial_tags.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        }
    }
    /// Returns `Ok(())` when the character defines `attribute`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the unknown attribute when it is absent.
    pub fn ensure_attribute(&self, attribute: &str) -> Result<()> {
        if self.attributes.contains_key(attribute) {
            Ok(())
        } else {
            bail!("L'attribut '{attribute}' n'existe pas pour ce personnage.")
        }
    }

    /// Adds an item stack, merging it with an identical existing stack when possible.
    pub fn add_item(&mut self, item_stack: ItemStack) -> usize {
        for (i, stack) in self.inventory.iter_mut().enumerate() {
            if stack.item.id == item_stack.item.id
                && stack.actual_data() == item_stack.actual_data()
            {
                stack.quantity += item_stack.quantity;
                return i;
            }
        }
        self.inventory.push(item_stack);
        self.inventory.len() - 1
    }
    /// Returns the base attribute plus all stacked persistent-state modifiers.
    ///
    /// # Errors
    ///
    /// Returns an error when `attribute` is not defined by the character.
    pub fn actual_attribute(&self, attribute: &str, formula: &mut FormulaEngine, env: &Env) -> Result<i64> {
        self.ensure_attribute(attribute)?;
        let base = self.attributes[attribute];
        let env = mk_partial_character_env(self)?.with_parent(env);
        let modifiers = self
            .conditions
            .iter()
            .enumerate()
            .map(|(i, condition)| {
                let env = mk_condition_env(&env, i)
                    .with_context(|| "Erreur d'environement d'état en calculant un attribut actuel")?
                    .with_parent(&env);
                let mods = condition.template.effects
                    .iter()
                    .filter_map(|effect| effect.attribute_modifier(attribute, formula, &env).transpose())
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| {
                        format!(
                            "Calcul des modificateurs de l'attribut `{attribute}` provenant de l'état `{}`",
                            condition.template.id
                        )
                    });
                mods
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Ok(base + total_modifiers(&modifiers))
    }
    /// Evaluates active check-modifier effects and applies category stacking.
    ///
    /// `attribute` and `traits` describe the check and are exposed to modifier filters/formulas as `test`.
    ///
    /// # Errors
    ///
    /// Propagates formula or filter evaluation errors from persistent-state effects.
    pub fn check_modifier(
        &self,
        attribute: Option<Arc<str>>,
        traits: &[Arc<str>],
        formula: &mut FormulaEngine,
        env: &Env,
    ) -> Result<i64> {
        let modifiers = self
            .conditions
            .iter()
            .map(|condition| {
                let mods = condition.template.effects
                    .iter()
                    .filter_map(|effect| effect.check_modifier(attribute.clone(), traits, &condition.data, self, formula, &env).transpose())
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| {
                        format!(
                            "Calcul des modificateurs du test `{attribute:?}` provenant de l'état `{}`",
                            condition.template.id
                        )
                    });
                mods
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Ok(total_modifiers(&modifiers))
    }
    /// Returns base maximum hit points plus stacked state modifiers, clamped to zero.
    pub fn actual_max_hp(&self, formula: &mut FormulaEngine, env: &Env) -> Result<i64> {
        let env = mk_partial_character_env(self)?.with_parent(env);
        let modifiers = self
            .conditions
            .iter()
            .map(|condition| {
                let mods = condition.template.effects
                    .iter()
                    .filter_map(|effect| effect.max_hp_modifier(formula, &env).transpose())
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| {
                        format!(
                            "Calcul des modificateurs de PV maximum provenant de l'état `{}`",
                            condition.template.id
                        )
                    });
                mods
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Ok((self.max_hp + total_modifiers(&modifiers)).max(0))
    }
    /// Returns base maximum morale plus stacked state modifiers, clamped to zero.
    pub fn actual_max_morale(&self, formula: &mut FormulaEngine, env: &Env) -> Result<i64> {
        let env = mk_partial_character_env(self)?.with_parent(env);
        let modifiers = self
            .conditions
            .iter()
            .map(|condition| {
                let mods = condition.template.effects
                    .iter()
                    .filter_map(|effect| effect.max_morale_modifier(formula, &env).transpose())
                    .collect::<Result<Vec<_>>>()
                    .with_context(|| {
                        format!(
                            "Calcul des modificateurs de moral maximum provenant de l'état `{}`",
                            condition.template.id
                        )
                    });
                mods
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Ok((self.max_morale + total_modifiers(&modifiers)).max(0))
    }

    /// Changes current hit points, clamps them, and reports whether the character survives.
    pub fn modify_hp(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.hp = self.hp.saturating_add(amount);
        self.fix_hp(formula, env)?;
        Ok(self.hp > 0)
    }
    /// Sets current hit points, clamps them to the effective range, and returns whether HP remains above zero.
    pub fn set_hp(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.hp = amount;
        self.fix_hp(formula, env)?;
        Ok(self.hp > 0)
    }

    /// Changes current morale, clamps it, and reports whether the character remains active.
    pub fn modify_morale(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.morale = self.morale.saturating_add(amount);
        self.fix_morale(formula, env)?;
        Ok(self.morale > 0)
    }
    /// Sets current morale, clamps it to the effective range, and returns whether morale remains above zero.
    pub fn set_morale(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.morale = amount;
        self.fix_morale(formula, env)?;
        Ok(self.morale > 0)
    }
    /// Sets base maximum hit points, reclamps current HP, and returns whether current HP remains above zero.
    pub fn set_max_hp(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.max_hp = amount;
        self.fix_hp(formula, env)?;
        if self.max_hp <= 0 {
            self.max_hp = 0;
        }
        Ok(self.hp > 0)
    }
    /// Adds `amount` to base maximum hit points with saturation, reclamps current HP, and returns whether HP remains above zero.
    pub fn modify_max_hp(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.max_hp = self.max_hp.saturating_add(amount);
        self.fix_hp(formula, env)?;
        if self.max_hp <= 0 {
            self.max_hp = 0;
        }
        Ok(self.hp > 0)
    }
    /// Sets base maximum morale, reclamps current morale, and returns whether current morale remains above zero.
    pub fn set_max_morale(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.max_morale = amount;
        self.fix_morale(formula, env)?;
        if self.max_morale <= 0 {
            self.max_morale = 0;
        }
        Ok(self.morale > 0)
    }
    /// Adds `amount` to base maximum morale with saturation, reclamps current morale, and returns whether morale remains above zero.
    pub fn modify_max_morale(&mut self, amount: i64, formula: &mut FormulaEngine, env: &Env) -> Result<bool> {
        self.max_morale = self.max_morale.saturating_add(amount);
        self.fix_morale(formula, env)?;
        if self.max_morale <= 0 {
            self.max_morale = 0;
        }
        Ok(self.morale > 0)
    }

    /// Clamps the current resource between zero and its effective maximum.
    pub fn fix_hp(&mut self, formula: &mut FormulaEngine, env: &Env) -> Result<()> {
        self.hp = self.hp.clamp(
            0,
            self.actual_max_hp(formula, env)
                .context("Impossible de borner les PV: calcul des PV maximum")?,
        );
        Ok(())
    }

    /// Clamps the current resource between zero and its effective maximum.
    pub fn fix_morale(&mut self, formula: &mut FormulaEngine, env: &Env) -> Result<()> {
        self.morale = self.morale.clamp(
            0,
            self.actual_max_morale(formula, env)
                .context("Impossible de borner le moral: calcul du moral maximum")?,
        );
        Ok(())
    }

    /// Merges identical stacks and removes redundant inventory entries.
    pub fn normalize_inventory(&mut self) {
        let mut normalized: Vec<ItemStack> = Vec::new();
        for stack in self.inventory.drain(..) {
            if let Some(existing) = normalized.iter_mut().find(|existing| {
                existing.item.id == stack.item.id && existing.actual_data() == stack.actual_data()
            }) {
                existing.quantity += stack.quantity;
            } else {
                normalized.push(stack);
            }
        }
        self.inventory = normalized;
    }

    /// Returns whether both hit points and morale remain above zero.
    pub fn is_alive(&self) -> bool {
        self.hp > 0 && self.morale > 0
    }
    /// Converts the character into its serializable save representation.
    ///
    /// Item and state definitions are stored by ID while mutable stack/state data and story tags are converted to JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if an [`EvalValue`] cannot be represented as JSON.
    pub fn save(&self) -> Result<SavedCharacter> {
        let mut story_tags = IndexMap::new();
        for (tag, value) in self.story_tags.iter() {
            story_tags.insert(tag.0.display(), value.into());
        }
        let mut stacks = Vec::new();
        for stack in &self.inventory {
            let mut data = IndexMap::new();
            for (key, value) in stack.data.iter() {
                data.insert(key.clone(), value.clone());
            }
            stacks.push(SavedItemStack {
                item_id: String::from(stack.item.id.as_ref()),
                quantity: stack.quantity,
                data: dict_to_map(data)?.into_iter()
                    .map(|(k, v)| (String::from(k.as_ref()), EvalValueSerializer::from(&v)))
                    .collect(),
            });
        }
        let mut conditions = Vec::new();
        for condition in &self.conditions {
            let mut data = IndexMap::new();
            for (key, value) in condition.data.iter() {
                data.insert(key.clone(), value.clone());
            }
            let data = dict_to_map(data)?.into_iter()
                .map(|(k, v)| (String::from(k.as_ref()), EvalValueSerializer::from(&v)))
                .collect::<IndexMap<_, _>>();
            conditions.push(SavedCondition {
                id: condition.template.id.as_ref().into(),
                data,
            });
        }

        Ok(SavedCharacter {
            attributes: self.attributes.iter().map(|(attribute, value)| (String::from(attribute.as_ref()), *value)).collect(),
            inventory: stacks,
            max_hp: self.max_hp,
            hp: self.hp,
            max_morale: self.max_morale,
            morale: self.morale,
            conditions,
            story_tags,
            name: self.name.clone().map(|name| String::from(name.as_ref())),
        })
    }
    /// Restores mutable character data from a save and rebinds item/state IDs to definitions in `tale`.
    ///
    /// Page effects are not replayed. The loaded tale must contain every item and state definition referenced by the save.
    ///
    /// # Errors
    ///
    /// Returns an error when a referenced definition is missing or saved data cannot be reconstructed.
    pub fn from_saved(saved: SavedCharacter, tale: &Tale) -> Result<Self> {
        let mut character = Self::new(&tale.attributes, saved.name.map(Arc::from));
        character.attributes = saved.attributes.iter().map(|(attribute, value)| (Arc::from(attribute.as_str()), *value)).collect();
        character.max_hp = saved.max_hp;
        character.hp = saved.hp;
        character.max_morale = saved.max_morale;
        character.morale = saved.morale;
        character.story_tags = Arc::from(saved
            .story_tags
            .into_iter()
            .map(|(tag, value)| Ok((EvalKey(EvalValue::String(tag.into())), value.try_into()?)))
            .collect::<Result<IndexMap<_, _>, anyhow::Error>>()?);

        for saved_stack in saved.inventory {
            let item = tale
                .items
                .get::<str>(saved_stack.item_id.as_ref())
                .cloned()
                .ok_or_else(|| {
                    anyhow!(
                        "L'objet '{}' est introuvable dans les objets du conte.",
                        saved_stack.item_id
                    )
                })?;
            character.inventory.push(ItemStack {
                item,
                quantity: saved_stack.quantity,
                data: map_to_dict(saved_stack
                    .data
                    .into_iter()
                    .map(|(k, v)| Ok((k.into(), v.try_into()?)))
                    .collect::<Result<IndexMap<Arc<str>, EvalValue>, anyhow::Error>>()?).into(),
            });
        }

        for saved_condition in saved.conditions {
            let template = tale.conditions.get::<str>(saved_condition.id.as_ref()).ok_or_else(|| {
                anyhow!(
                    "L'état '{}' est introuvable dans le conte.",
                    saved_condition.id
                )
            })?;
            character.conditions.push(ConditionInstance {
                template: template.clone(),
                data: map_to_dict(saved_condition
                    .data
                    .into_iter()
                    .map(|(k, v)| Ok((k.into(), v.try_into()?)))
                    .collect::<Result<IndexMap<Arc<str>, EvalValue>, anyhow::Error>>()?).into(),
            });
        }

        character.normalize_inventory();
        Ok(character)
    }
    /// Returns a mutable story-tag value at a dotted object/list path.
    ///
    /// When `create_path` is true, missing containers are created according to the next path segment and the final value uses `default_value`. Numeric components index arrays.
    ///
    /// # Errors
    ///
    /// Returns an error for empty paths, missing values when creation is disabled, invalid indices, or incompatible intermediate types.
    pub fn find_tag_value(
        &mut self,
        path: &str,
        create_path: bool,
        default_value: EvalValue,
    ) -> Result<&mut EvalValue> {
        let mut parts = path.split('.').collect::<Vec<_>>();
        if parts.is_empty() {
            bail!("Le chemin de tag ne peut pas être vide");
        }
        let mut current_id = parts.remove(0);
        let work = Arc::make_mut(&mut self.story_tags);
        let current_value = work.entry(EvalKey(EvalValue::String(current_id.into())));
        let mut current_value = match current_value {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                if create_path {
                    insert_next(parts.get(0).copied(), entry, &default_value)
                } else {
                    bail!("Le tag '{current_id}' n'existe pas et le chemin ne peut pas être créé");
                }
            },
        };
        while !parts.is_empty() {
            current_id = parts.remove(0);
            match (current_value, current_id.parse::<usize>()) {
                (EvalValue::Object(_map), Ok(_)) => {
                    bail!("Le tag '{current_id}' est un objet et ne peut pas être indexé par un entier");
                }
                (EvalValue::Object(map), Err(_)) => {
                    let work = Arc::make_mut(map);
                    current_value = match work.entry(EvalKey(current_id.into())) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            if create_path {
                                insert_next(parts.get(0).copied(), entry, &default_value)
                            } else {
                                bail!("Le tag '{current_id}' n'existe pas et le chemin ne peut pas être créé");
                            }
                        },
                    };
                }
                (EvalValue::Array(array), Ok(index)) => {
                    if index >= array.len() {
                        if create_path {
                            let mut work = array.to_vec();
                            work.resize(index + 1, EvalValue::Null);
                            work[index] = default_value.clone();
                            *array = work.into();
                        } else {
                            bail!("L'index {index} est hors limites pour le tableau '{current_id}' et le chemin ne peut pas être créé");
                        }
                    }
                    current_value = &mut Arc::make_mut(array)[index];
                }
                (EvalValue::Array(_), Err(_)) => {
                    bail!("Le tag '{current_id}' est un tableau et ne peut pas être indexé par une chaîne de caractères");
                }
                _ => {
                    bail!("Le tag '{current_id}' n'est ni un objet ni un tableau et ne peut pas être indexé");
                }
            }
        }
        Ok(current_value)
    }
    /// Returns a mutable story-tag value at a dotted object/list path.
    ///
    /// When `create_path` is true, missing containers are created according to the next path segment and the final value uses `default_value`. Numeric components index arrays.
    ///
    /// # Errors
    ///
    /// Returns an error for empty paths, missing values when creation is disabled, invalid indices, or incompatible intermediate types.
    pub fn remove_tag_value(&mut self, path: &[TagPathSegment]) -> Result<Option<EvalValue>> {
        let mut parts = path.to_vec();
        if parts.is_empty() {
            bail!("Le chemin de tag ne peut pas être vide");
        }
        let mut current_id = parts.remove(0);
        let work = Arc::make_mut(&mut self.story_tags);
        let current_value = work.entry(match current_id {
            TagPathSegment::Key(ref key) => EvalKey(EvalValue::String(key.clone())),
            TagPathSegment::Index(idx) => bail!("Le chemin de tag ne peut pas commencer par un index de tableau ({idx})"),
        });
        let mut current_value = match current_value {
            Entry::Occupied(entry) => {
                if parts.is_empty() {
                    return Ok(Some(entry.shift_remove()));
                }
                entry.into_mut()
            },
            Entry::Vacant(_entry) => {
                return Ok(None);
            },
        };
        let mut current_path = match current_id {
            TagPathSegment::Key(ref key) => String::from(key.as_ref()),
            TagPathSegment::Index(idx) => idx.to_string(),
        };
        while !parts.is_empty() {
            current_id = parts.remove(0);
            match (current_value, current_id.clone()) {
                (EvalValue::Object(_map), TagPathSegment::Index(_)) => {
                    bail!("Le tag '{current_path}' est un objet et ne peut pas être indexé par un entier");
                }
                (EvalValue::Object(map), TagPathSegment::Key(key)) => {
                    let work = Arc::make_mut(map);
                    current_value = match work.entry(EvalKey(EvalValue::String(key.into()))) {
                        Entry::Occupied(entry) => {
                            if parts.is_empty() {
                                return Ok(Some(entry.shift_remove()));
                            }
                            entry.into_mut()
                        },
                        Entry::Vacant(_entry) => {
                            return Ok(None);
                        },
                    };
                }
                (EvalValue::Array(array), TagPathSegment::Index(idx)) => {
                    if idx >= array.len() {
                        return Ok(None);
                    }
                    if parts.is_empty() {
                        let mut work = array.to_vec();
                        let rem = work.remove(idx);
                        *array = work.into();
                        return Ok(Some(rem));
                    }
                    let work = Arc::make_mut(array);
                    current_value = &mut work[idx];
                }
                (EvalValue::Array(_), TagPathSegment::Key(_)) => {
                    bail!("Le tag '{current_path}' est un tableau et ne peut pas être indexé par une chaîne de caractères");
                }
                _ => {
                    bail!("Le tag '{current_path}' n'est ni un objet ni un tableau et ne peut pas être indexé");
                }
            }
            current_path.push('.');
            current_path.push_str(&match current_id {
                TagPathSegment::Key(ref key) => String::from(key.as_ref()),
                TagPathSegment::Index(idx) => idx.to_string(),
            });
        }
        Ok(None)
    }
}

fn insert_next<'a>(
    next_id: Option<&str>,
    entry: indexmap::map::VacantEntry<'a, EvalKey, EvalValue>,
    default_value: &EvalValue,
) -> &'a mut EvalValue {
    match next_id {
        Some(next_id) => {
            if let Ok(index) = next_id.parse::<usize>() {
                let mut array = Vec::new();
                array.resize(index + 1, EvalValue::Null);
                array[index] = default_value.clone();
                entry.insert(EvalValue::Array(array.into()))
            } else {
                let mut map = IndexMap::new();
                map.insert(EvalKey(EvalValue::from(next_id)), default_value.clone());
                entry.insert(EvalValue::Object(map.into()))
            }
        }
        None => entry.insert(default_value.clone()),
    }
}

/// Combines categorized and untyped modifiers using the game stacking rules.
fn total_modifiers(modifiers: &[Modifier]) -> i64 {
    let mut bonuses: IndexMap<&str, i64> = IndexMap::new();
    let mut penalties: IndexMap<&str, i64> = IndexMap::new();
    let mut untyped = 0_i64;

    for modifier in modifiers {
        match modifier.category.as_deref() {
            None => untyped = untyped.saturating_add(modifier.value),
            Some(category) if modifier.value > 0 => {
                let current = bonuses.entry(category).or_insert(modifier.value);
                *current = (*current).max(modifier.value);
            }
            Some(category) => {
                let current = penalties.entry(category).or_insert(modifier.value);
                *current = (*current).min(modifier.value);
            }
        }
    }

    bonuses
        .values()
        .chain(penalties.values())
        .copied()
        .fold(untyped, i64::saturating_add)
}

/// Serializable representation of a character stored in a save file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedCharacter {
    pub attributes: IndexMap<String, i64>,
    pub inventory: Vec<SavedItemStack>,
    pub max_hp: i64,
    pub hp: i64,
    pub max_morale: i64,
    pub morale: i64,
    pub conditions: Vec<SavedCondition>,
    pub story_tags: IndexMap<String, EvalValueSerializer>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Serializable representation of one inventory stack.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedItemStack {
    #[serde(rename = "objet")]
    pub item_id: String,
    #[serde(rename = "quantite")]
    pub quantity: i64,
    #[serde(rename = "donnees", default)]
    pub data: IndexMap<String, EvalValueSerializer>,
}

/// Serializable representation of one persistent-state instance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedCondition {
    pub id: String,
    #[serde(rename = "donnees")]
    pub data: IndexMap<String, EvalValueSerializer>,
}

#[cfg(test)]
mod tests {
    use super::Character;
    use formulas::{Env, FormulaEngine};
    use crate::{formula::mk_character_base_env, model::Tale};
    use indexmap::IndexMap;
    use serde_json::json;

    // Verifies that loading a save restores lifecycle cells from the current state template.
    #[test]
    fn saved_conditions_restore_before_and_after_cells() {
        let tale = Tale::from_json(&json!({
            "version": 2,
            "titre": "Sauvegarde des états",
            "depart": "debut",
            "etats": {
                "veille": {
                    "avant": [{"texte": "Avant"}],
                    "apres": [{"texte": "Après"}]
                }
            },
            "pages": {"debut": {"cellules": [], "connexions": []}}
        }))
        .unwrap();
        let mut formula = FormulaEngine::new(Some(1));
        let mut character = Character::new(&tale.attributes, None);
        let env = &mk_character_base_env(&character, &mut formula, &Env::new()).expect("Formula failed");
        let condition = tale.conditions["veille"]
            .instantiate(&formulas::EvalValue::Object(IndexMap::new().into()), &mut formula, &env)
            .unwrap();
        character.conditions.push(condition);

        let restored = Character::from_saved(character.save().unwrap(), &tale).unwrap();
        assert_eq!(restored.conditions[0].template.pre.len(), 1);
        assert_eq!(restored.conditions[0].template.post.len(), 1);
    }
}
