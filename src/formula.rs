use std::sync::Arc;

use formulas::{Env, EvalKey, EvalValue, FormulaEngine};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;

use crate::{character::Character, effects::Deferred, model::{ConditionInstance, Connection, ItemStack, Page, Tale, eval_tags, eval_traits}};
use anyhow::{Context, Result, anyhow, bail};

/// Create a full view of a character, including calculated attributes, items, conditions, and story tags.
/// Intented for formula evaluation.
pub fn mk_character_view(character: &Character, formula: &mut FormulaEngine, env: &Env) -> Result<EvalValue> {
    let env = {
        let partial = mk_partial_character_view(character)?;
        let mut partial_map = IndexMap::new();
        partial_map.insert("personnage".into(), partial);
        Env::from_index_map(&Arc::new(partial_map))
            .with_parent(env)
    };
    let name = character.name.as_ref().map(|s| EvalValue::from(s.clone())).unwrap_or(EvalValue::Null);
    let max_pv = character.actual_max_hp(formula, &env)?;
    let pv = character.hp;
    let max_moral = character.actual_max_morale(formula, &env)?;
    let moral = character.morale;
    let max_pv = EvalValue::from(max_pv);
    let pv = EvalValue::from(pv);
    let max_moral = EvalValue::from(max_moral);
    let moral = EvalValue::from(moral);
    let attrs: IndexMap<EvalKey, EvalValue> = character.attributes
        .iter()
        .map(|(k, _)| {
            let value = character.actual_attribute(k.as_ref(), formula, &env)?;
            Ok((EvalKey(EvalValue::from(k.clone())), EvalValue::from(value)))
        })
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;
    let items = character.inventory
        .iter()
        .map(|item| mk_item_view(item, formula, &env))
        .collect::<Result<Vec<EvalValue>>>()?;
    let etats = character.conditions
        .iter()
        .map(|condition| mk_condition_view(condition, formula, &env))
        .collect::<Result<Vec<EvalValue>>>()?;
    let tags = character.story_tags
        .iter()
        .map(|(k, v)| Ok((k.clone(), EvalValue::from(v.clone()))))
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;

    let mut map = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("name")), name);
    map.insert(EvalKey(EvalValue::from("max_pv")), max_pv);
    map.insert(EvalKey(EvalValue::from("pv")), pv);
    map.insert(EvalKey(EvalValue::from("max_moral")), max_moral);
    map.insert(EvalKey(EvalValue::from("moral")), moral);
    map.insert(EvalKey(EvalValue::from("attributs")), EvalValue::Object(attrs.into()));
    map.insert(EvalKey(EvalValue::from("items")), EvalValue::Array(items.into()));
    map.insert(EvalKey(EvalValue::from("etats")), EvalValue::Array(etats.into()));
    map.insert(EvalKey(EvalValue::from("tags")), EvalValue::Object(tags.into()));

    Ok(EvalValue::Object(map.into()))
}

/// As opposed to `mk_character_view`, this creates a partial view of a character, using only base attributes and ignoring calculated values.
/// Intended for formula evaluation.
pub fn mk_partial_character_view(character: &Character) -> Result<EvalValue> {
    let name = character.name.as_ref().map(|s| EvalValue::from(s.clone())).unwrap_or(EvalValue::Null);
    let max_pv = character.max_hp.into();
    let pv = character.hp.into();
    let max_moral = character.max_morale.into();
    let moral = character.morale.into();
    let attrs: IndexMap<EvalKey, EvalValue> = character.attributes
        .iter()
        .map(|(k, v)| {
            Ok((EvalKey(EvalValue::from(k.clone())), EvalValue::from(*v)))
        })
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;
    let items = character.inventory
        .iter()
        .map(|item| mk_partial_item_view(item))
        .collect::<Result<Vec<EvalValue>>>()?;
    let etats = character.conditions
        .iter()
        .map(|condition| mk_partial_condition_view(condition))
        .collect::<Result<Vec<EvalValue>>>()?;
    let tags = character.story_tags
        .iter()
        .map(|(k, v)| Ok((k.clone(), EvalValue::from(v.clone()))))
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;

    let mut map = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("name")), name);
    map.insert(EvalKey(EvalValue::from("max_pv")), max_pv);
    map.insert(EvalKey(EvalValue::from("pv")), pv);
    map.insert(EvalKey(EvalValue::from("max_moral")), max_moral);
    map.insert(EvalKey(EvalValue::from("moral")), moral);
    map.insert(EvalKey(EvalValue::from("attributs")), EvalValue::Object(attrs.into()));
    map.insert(EvalKey(EvalValue::from("items")), EvalValue::Array(items.into()));
    map.insert(EvalKey(EvalValue::from("etats")), EvalValue::Array(etats.into()));
    map.insert(EvalKey(EvalValue::from("tags")), EvalValue::Object(tags.into()));

    Ok(EvalValue::Object(map.into()))
}

/// Create an environment for formula evaluation based on the character's current state.
/// Has a binding for the character, and for each of its properties (attributes, inventory, conditions, and story tags).
pub fn mk_character_env(character: &Character, formula: &mut FormulaEngine, env: &Env) -> Result<Env<'static>> {
    let env = {
        let partial = mk_partial_character_view(character)?;
        let mut partial_map = IndexMap::new();
        partial_map.insert("personnage".into(), partial);
        Env::from_index_map(&Arc::new(partial_map))
            .with_parent(env)
    };
    let name = character.name.as_ref().map(|s| EvalValue::from(s.clone())).unwrap_or(EvalValue::Null);
    let max_pv = character.actual_max_hp(formula, &env)?;
    let pv = character.hp;
    let max_moral = character.actual_max_morale(formula, &env)?;
    let moral = character.morale;
    let max_pv = EvalValue::from(max_pv);
    let pv = EvalValue::from(pv);
    let max_moral = EvalValue::from(max_moral);
    let moral = EvalValue::from(moral);
    let attrs: IndexMap<EvalKey, EvalValue> = character.attributes
        .iter()
        .map(|(k, _)| {
            let value = character.actual_attribute(k.as_ref(), formula, &env)?;
            Ok((EvalKey(EvalValue::from(k.clone())), EvalValue::from(value)))
        })
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;
    let items = character.inventory
        .iter()
        .map(|item| mk_item_view(item, formula, &env))
        .collect::<Result<Vec<EvalValue>>>()?;
    let etats = character.conditions
        .iter()
        .map(|condition| mk_condition_view(condition, formula, &env))
        .collect::<Result<Vec<EvalValue>>>()?;
    let tags = character.story_tags
        .iter()
        .map(|(k, v)| Ok((k.clone(), EvalValue::from(v.clone()))))
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;

    let mut envmap = IndexMap::new();
    envmap.insert("name".into(), name.clone());
    envmap.insert("max_pv".into(), max_pv.clone());
    envmap.insert("pv".into(), pv.clone());
    envmap.insert("max_moral".into(), max_moral.clone());
    envmap.insert("moral".into(), moral.clone());
    envmap.insert("attributs".into(), EvalValue::Object(attrs.clone().into()));
    envmap.insert("objets".into(), EvalValue::Array(items.clone().into()));
    envmap.insert("etats".into(), EvalValue::Array(etats.clone().into()));
    envmap.insert("tags".into(), EvalValue::Object(tags.clone().into()));

    let mut map = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("name")), name);
    map.insert(EvalKey(EvalValue::from("max_pv")), max_pv);
    map.insert(EvalKey(EvalValue::from("pv")), pv);
    map.insert(EvalKey(EvalValue::from("max_moral")), max_moral);
    map.insert(EvalKey(EvalValue::from("moral")), moral);
    map.insert(EvalKey(EvalValue::from("attributs")), EvalValue::Object(attrs.into()));
    map.insert(EvalKey(EvalValue::from("objets")), EvalValue::Array(items.into()));
    map.insert(EvalKey(EvalValue::from("etats")), EvalValue::Array(etats.into()));
    map.insert(EvalKey(EvalValue::from("tags")), EvalValue::Object(tags.into()));
    envmap.insert("personnage".into(), EvalValue::Object(map.clone().into()));

    Ok(Env::from_arc(Arc::new(envmap)))
}
/// Create a base environment for formula evaluation with only the character binding.
/// Only contains a binding for the character under the key "personnage".
pub fn mk_character_base_env(character: &Character, formula: &mut FormulaEngine, env: &Env) -> Result<Env<'static>> {
    let view = mk_character_view(character, formula, env)?;
    let mut envmap = IndexMap::new();
    envmap.insert("personnage".into(), view);
    Ok(Env::from_arc(Arc::new(envmap)))
}
/// Create an environment for formula evaluation based on the character's current state.
/// Has a binding for the character, and for each of its properties (attributes, inventory, conditions, and story tags).
/// Does not evaluate formulas, only constructs the environment based on the character's current state.
pub fn mk_partial_character_env(character: &Character) -> Result<Env<'static>> {
    let EvalValue::Object(partial) = mk_partial_character_view(character)? else {
        bail!("Expected partial character view to be an object");
    };

    let name = partial.get(&EvalKey("name".into())).ok_or_else(|| anyhow!("Name missing from partial character view"))?;
    let max_pv = partial.get(&EvalKey("max_pv".into())).ok_or_else(|| anyhow!("Max PV missing from partial character view"))?;
    let pv = partial.get(&EvalKey("pv".into())).ok_or_else(|| anyhow!("PV missing from partial character view"))?;
    let max_moral = partial.get(&EvalKey("max_moral".into())).ok_or_else(|| anyhow!("Max morale missing from partial character view"))?;
    let moral = partial.get(&EvalKey("moral".into())).ok_or_else(|| anyhow!("Morale missing from partial character view"))?;
    let attrs = partial.get(&EvalKey("attributs".into())).ok_or_else(|| anyhow!("Attributes missing from partial character view"))?;
    let items = partial.get(&EvalKey("objets".into())).ok_or_else(|| anyhow!("Items missing from partial character view"))?;
    let etats = partial.get(&EvalKey("etats".into())).ok_or_else(|| anyhow!("Conditions missing from partial character view"))?;
    let tags = partial.get(&EvalKey("tags".into())).ok_or_else(|| anyhow!("Tags missing from partial character view"))?;

    let mut envmap = IndexMap::new();
    envmap.insert("nom".into(), name.clone());
    envmap.insert("max_pv".into(), max_pv.clone());
    envmap.insert("pv".into(), pv.clone());
    envmap.insert("max_moral".into(), max_moral.clone());
    envmap.insert("moral".into(), moral.clone());
    envmap.insert("attributs".into(), attrs.clone());
    envmap.insert("objets".into(), items.clone());
    envmap.insert("etats".into(), etats.clone());
    envmap.insert("tags".into(), tags.clone());

    envmap.insert("personnage".into(), EvalValue::Object(partial));

    Ok(Env::from_arc(Arc::new(envmap)))
}

/// Create a view of an item for formula evaluation.
/// Contains the item's id, name, description, quantity, traits, and actual data.
pub fn mk_item_view(item: &ItemStack, formula: &mut FormulaEngine, env: &Env) -> Result<EvalValue> {
    let env = {
        let partial = mk_partial_item_view(item)?;
        let mut partial_map = IndexMap::new();
        partial_map.insert("objet".into(), partial);
        Env::from_index_map(&Arc::new(partial_map))
            .with_parent(env)
    };
    let id = item.item.id.clone();
    let name = item.actual_name(formula, &env)?;
    let description = item.actual_description(formula, &env)?;
    let quantity = item.quantity;
    let traits = eval_traits(&item.item.traits, formula, &env)
        .with_context(|| format!("Erreur en évaluant les traits d'une instance de l'etat {}", item.item.id))?;
    let data: IndexMap<EvalKey, EvalValue> = item.actual_data()
        .iter()
        .map(|(k, v)| Ok((k.clone(), EvalValue::from(v.clone()))))
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;
    
    let mut map: IndexMap<EvalKey, EvalValue> = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("id")), EvalValue::from(id));
    map.insert(EvalKey(EvalValue::from("nom")), name.into());
    map.insert(EvalKey(EvalValue::from("description")), description.into());
    map.insert(EvalKey(EvalValue::from("quantite")), EvalValue::from(quantity));
    map.insert(EvalKey(EvalValue::from("traits")), EvalValue::Array(traits.into()));
    map.insert(EvalKey(EvalValue::from("donnees")), EvalValue::Object(data.into()));

    Ok(EvalValue::Object(map.into()))
}
/// Create a partial view of an item for formula evaluation.
/// Does not evaluate deferred traits, name, description, or data; only includes literal values.
pub fn mk_partial_item_view(item: &ItemStack) -> Result<EvalValue> {
    let id = item.item.id.clone();
    let name = item.item.name.clone();
    let description = item.item.description.clone();
    let quantity = item.quantity;
    let traits = item.item.traits.iter()
        .filter_map(|t| match t {
            Deferred::Literal(lit) => Some(lit.clone().into()),
            _ => None,
        })
        .collect::<Vec<EvalValue>>();
    let data: IndexMap<EvalKey, EvalValue> = item.actual_data()
        .iter()
        .map(|(k, v)| Ok((k.clone(), EvalValue::from(v.clone()))))
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;

    let mut map: IndexMap<EvalKey, EvalValue> = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("id")), EvalValue::from(id));
    map.insert(EvalKey(EvalValue::from("nom")), EvalValue::from(name));
    map.insert(EvalKey(EvalValue::from("description")), EvalValue::from(description));
    map.insert(EvalKey(EvalValue::from("quantite")), EvalValue::from(quantity));
    map.insert(EvalKey(EvalValue::from("traits")), EvalValue::Array(traits.into()));
    map.insert(EvalKey(EvalValue::from("donnees")), EvalValue::Object(data.into()));

    Ok(EvalValue::Object(map.into()))
}
/// Create an environment for formula evaluation based on a specific item of the character.
/// Extract the information from an existing env containing a character at "personnage" to ensure no differences with the character's current state.
/// The resulting environment will have bindings for the item itself and its key properties.
pub fn mk_item_env(character_env: &Env, item_idx: usize) -> Result<Env<'static>> {
    let EvalValue::Object(character) = character_env.get("personnage").ok_or_else(|| anyhow!("Aucun personnage dans `character_env`"))? else {
        bail!("Le personnage dans `character_env` n'est pas un objet");
    };
    let EvalValue::Array(items) = character.get(&EvalKey(EvalValue::from("objets"))).ok_or_else(|| anyhow!("Aucun objet dans le personnage"))? else {
        bail!("Les objets dans le personnage ne sont pas un tableau");
    };
    let EvalValue::Object(item) = items.get(item_idx).ok_or_else(|| anyhow!("Index d'objet invalide"))? else {
        bail!("L'objet à l'index donné n'est pas un objet");
    };
    
    let mut envmap: IndexMap<Arc<str>, EvalValue> = IndexMap::new();
    envmap.insert(Arc::from("objet"), EvalValue::Object(item.clone()));
    envmap.insert(Arc::from("nom"), item.get(&EvalKey(EvalValue::from("nom"))).ok_or_else(|| anyhow!("L'objet n'a pas de nom"))?.clone());
    envmap.insert(Arc::from("description"), item.get(&EvalKey(EvalValue::from("description"))).ok_or_else(|| anyhow!("L'objet n'a pas de description"))?.clone());
    envmap.insert(Arc::from("quantite"), item.get(&EvalKey(EvalValue::from("quantite"))).ok_or_else(|| anyhow!("L'objet n'a pas de quantite"))?.clone());
    envmap.insert(Arc::from("traits"), item.get(&EvalKey(EvalValue::from("traits"))).ok_or_else(|| anyhow!("L'objet n'a pas de traits"))?.clone());
    envmap.insert(Arc::from("donnees"), item.get(&EvalKey(EvalValue::from("donnees"))).ok_or_else(|| anyhow!("L'objet n'a pas de donnees"))?.clone());

    Ok(Env::from_index_map(&Arc::new(envmap)))
}
/// Create a base environment for formula evaluation for a specific item of the character.
/// Extract the information from an existing env containing a character at "personnage" to ensure no differences with the character's current state.
/// Only contains a binding for the item itself under the key "objet".
pub fn mk_item_base_env(character_env: &Env, item_idx: usize) -> Result<Env<'static>> {
    let EvalValue::Object(character) = character_env.get("personnage").ok_or_else(|| anyhow!("Aucun personnage dans `character_env`"))? else {
        bail!("Le personnage dans `character_env` n'est pas un objet");
    };
    let EvalValue::Array(items) = character.get(&EvalKey(EvalValue::from("objets"))).ok_or_else(|| anyhow!("Aucun objet dans le personnage"))? else {
        bail!("Les objets dans le personnage ne sont pas un tableau");
    };
    let EvalValue::Object(item) = items.get(item_idx).ok_or_else(|| anyhow!("Index d'objet invalide"))? else {
        bail!("L'objet à l'index donné n'est pas un objet");
    };
    
    let mut envmap: IndexMap<Arc<str>, EvalValue> = IndexMap::new();
    envmap.insert(Arc::from("objet"), EvalValue::Object(item.clone()));

    Ok(Env::from_index_map(&Arc::new(envmap)))
}
/// Creates a partial environment for an item stack, containing its name, description, traits, data, and the item itself.
/// Does not evaluate formulas; it only extracts the raw fields from the item stack.
pub fn mk_partial_item_env(item: &ItemStack) -> Result<Env<'static>> {
    let view = mk_partial_item_view(item)?;
    let EvalValue::Object(view_map) = view else {
        bail!("La vue partielle de l'objet n'est pas un objet");
    };

    let name = view_map.get(&EvalKey(EvalValue::String("nom".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de nom"))?;
    let description = view_map.get(&EvalKey(EvalValue::String("description".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de description"))?;
    let traits = view_map.get(&EvalKey(EvalValue::String("traits".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de traits"))?;
    let data = view_map.get(&EvalKey(EvalValue::String("donnees".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de donnees"))?;

    let mut env = Env::new();
    env.insert("nom", name.clone());
    env.insert("description", description.clone());
    env.insert("traits", traits.clone());
    env.insert("donnees", data.clone());
    env.insert("objet", EvalValue::Object(view_map));

    Ok(env)
}
pub fn mk_new_item_env(item: &ItemStack, formula: &mut FormulaEngine, env: &Env) -> Result<Env<'static>> {
    let env = {
        let partial = mk_partial_item_view(item)?;
        let mut partial_map = IndexMap::new();
        partial_map.insert("objet".into(), partial);
        Env::from_index_map(&Arc::new(partial_map))
            .with_parent(env)
    };
    let id = item.item.id.clone();
    let name = item.actual_name(formula, &env)?;
    let description = item.actual_description(formula, &env)?;
    let quantity = item.quantity;
    let traits = eval_traits(&item.item.traits, formula, &env)
        .with_context(|| format!("Erreur en évaluant les traits d'une instance de l'etat {}", item.item.id))?;
    let data: IndexMap<EvalKey, EvalValue> = item.actual_data()
        .iter()
        .map(|(k, v)| Ok((k.clone(), EvalValue::from(v.clone()))))
        .collect::<Result<IndexMap<EvalKey, EvalValue>>>()?;
    let data = Arc::new(data);
    
    let mut map: IndexMap<EvalKey, EvalValue> = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("id")), EvalValue::from(id.clone()));
    map.insert(EvalKey(EvalValue::from("nom")), name.clone().into());
    map.insert(EvalKey(EvalValue::from("description")), description.clone().into());
    map.insert(EvalKey(EvalValue::from("quantite")), EvalValue::from(quantity));
    map.insert(EvalKey(EvalValue::from("traits")), EvalValue::Array(traits.clone()));
    map.insert(EvalKey(EvalValue::from("donnees")), EvalValue::Object(data.clone()));

    let mut env = Env::new();
    env.insert("id", id.into());
    env.insert("nom", name.into());
    env.insert("description", description.into());
    env.insert("quantite", quantity.into());
    env.insert("traits", EvalValue::Array(traits));
    env.insert("donnees", EvalValue::Object(data));
    env.insert("objet", EvalValue::Object(map.into()));

    Ok(env)
}

/// Create a view of a condition for formula evaluation.
/// Contains the condition's id, actual name, actual description, evaluated traits, and actual data.
pub fn mk_condition_view(condition: &ConditionInstance, formula: &mut FormulaEngine, env: &Env) -> Result<EvalValue> {
    let env = {
        let partial = mk_partial_condition_view(condition)?;
        let mut partial_map = IndexMap::new();
        partial_map.insert("etat".into(), partial);
        Env::from_index_map(&Arc::new(partial_map))
            .with_parent(env)
    };
    let id = condition.template.id.clone();
    let name = condition.actual_name(formula, &env)?;
    let description = condition.actual_description(formula, &env)?;
    let traits: Vec<EvalValue> = condition.template.traits.iter()
        .map(|t| {
            match t {
                Deferred::Literal(lit) => Ok(lit.clone().into()),
                Deferred::Formula(form) => {
                    form.clone().call(&[], formula, &env)
                        .map_err(|err| anyhow!("Erreur évaluant le trait différé: {}", err))
                },
            }
        })
        .collect::<Result<Vec<EvalValue>>>()?;
    let data: IndexMap<EvalKey, EvalValue> = Arc::unwrap_or_clone(condition.actual_data())
        .into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<IndexMap<EvalKey, EvalValue>>();

    let mut map: IndexMap<EvalKey, EvalValue> = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("id")), EvalValue::from(id));
    map.insert(EvalKey(EvalValue::from("nom")), EvalValue::from(name));
    map.insert(EvalKey(EvalValue::from("description")), EvalValue::from(description));
    map.insert(EvalKey(EvalValue::from("traits")), EvalValue::Array(traits.into()));
    map.insert(EvalKey(EvalValue::from("donnees")), EvalValue::Object(data.into()));

    Ok(EvalValue::Object(map.into()))
}
/// Create a partial view of a condition for formula evaluation.
/// Does not evaluate deferred traits, name, description, or data; only includes literal values.
pub fn mk_partial_condition_view(condition: &ConditionInstance) -> Result<EvalValue> {
    let id = condition.template.id.clone();
    let name = condition.template.name.clone();
    let description = condition.template.description.clone();
    let traits: Vec<EvalValue> = condition.template.traits.iter()
        .filter_map(|t| {
            match t {
                Deferred::Literal(lit) => Some(lit.clone().into()),
                Deferred::Formula(_form) => None,
            }
        })
        .collect();
    let data: IndexMap<EvalKey, EvalValue> = Arc::unwrap_or_clone(condition.actual_data())
        .into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<IndexMap<EvalKey, EvalValue>>();

    let mut map: IndexMap<EvalKey, EvalValue> = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("id")), EvalValue::from(id));
    map.insert(EvalKey(EvalValue::from("nom")), EvalValue::from(name));
    map.insert(EvalKey(EvalValue::from("description")), EvalValue::from(description));
    map.insert(EvalKey(EvalValue::from("traits")), EvalValue::Array(traits.into()));
    map.insert(EvalKey(EvalValue::from("donnees")), EvalValue::Object(data.into()));

    Ok(EvalValue::Object(map.into()))
}
/// Create an environment for formula evaluation for a specific condition of the character.
/// Extracts the condition at the given index from the character's "etats" array and binds it under "etat".
/// Also binds the condition's name and description under "nom" and "description".
/// Additionally binds the condition's quantity, traits, and data under "quantite", "traits", and "donnees".
pub fn mk_condition_env(character_env: &Env, item_idx: usize) -> Result<Env<'static>> {
    let EvalValue::Object(character) = character_env.get("personnage").ok_or_else(|| anyhow!("Aucun personnage dans `character_env`"))? else {
        bail!("Le personnage dans `character_env` n'est pas un objet");
    };
    let EvalValue::Array(conditions) = character.get(&EvalKey(EvalValue::from("etats"))).ok_or_else(|| anyhow!("Aucun objet dans le personnage"))? else {
        bail!("Les états dans le personnage ne sont pas un tableau");
    };
    let EvalValue::Object(condition) = conditions.get(item_idx).ok_or_else(|| anyhow!("Index d'état invalide"))? else {
        bail!("L'état à l'index donné n'est pas un objet");
    };
    
    let mut envmap: IndexMap<Arc<str>, EvalValue> = IndexMap::new();
    envmap.insert(Arc::from("etat"), EvalValue::Object(condition.clone()));
    envmap.insert(Arc::from("nom"), condition.get(&EvalKey(EvalValue::from("nom"))).ok_or_else(|| anyhow!("L'état n'a pas de nom"))?.clone());
    envmap.insert(Arc::from("description"), condition.get(&EvalKey(EvalValue::from("description"))).ok_or_else(|| anyhow!("L'état n'a pas de description"))?.clone());
    envmap.insert(Arc::from("quantite"), condition.get(&EvalKey(EvalValue::from("quantite"))).ok_or_else(|| anyhow!("L'état n'a pas de quantite"))?.clone());
    envmap.insert(Arc::from("traits"), condition.get(&EvalKey(EvalValue::from("traits"))).ok_or_else(|| anyhow!("L'état n'a pas de traits"))?.clone());
    envmap.insert(Arc::from("donnees"), condition.get(&EvalKey(EvalValue::from("donnees"))).ok_or_else(|| anyhow!("L'état n'a pas de donnees"))?.clone());

    Ok(Env::from_index_map(&Arc::new(envmap)))
}
/// Create a minimal environment for formula evaluation for a specific condition of the character.
/// Extracts the condition at the given index from the character's "etats" array.
/// Only binds the condition itself under "etat".
pub fn mk_condition_base_env(character_env: &Env, item_idx: usize) -> Result<Env<'static>> {
    let EvalValue::Object(character) = character_env.get("personnage").ok_or_else(|| anyhow!("Aucun personnage dans `character_env`"))? else {
        bail!("Le personnage dans `character_env` n'est pas un objet");
    };
    let EvalValue::Array(conditions) = character.get(&EvalKey(EvalValue::from("etats"))).ok_or_else(|| anyhow!("Aucun objet dans le personnage"))? else {
        bail!("Les états dans le personnage ne sont pas un tableau");
    };
    let EvalValue::Object(condition) = conditions.get(item_idx).ok_or_else(|| anyhow!("Index d'état invalide"))? else {
        bail!("L'état à l'index donné n'est pas un objet");
    };
    
    let mut envmap: IndexMap<Arc<str>, EvalValue> = IndexMap::new();
    envmap.insert(Arc::from("etat"), EvalValue::Object(condition.clone()));

    Ok(Env::from_index_map(&Arc::new(envmap)))
}
/// Create a minimal environment for formula evaluation for a specific condition instance.
/// Extracts the condition's partial view and binds its name, description, traits, and data under "nom", "description", "traits", and "donnees" respectively.
/// Also binds the entire condition under "etat".
pub fn mk_partial_condition_env(condition: &ConditionInstance) -> Result<Env<'static>> {
    let view = mk_partial_condition_view(condition)?;
    let EvalValue::Object(view_map) = view else {
        bail!("La vue partielle de l'objet n'est pas un objet");
    };

    let name = view_map.get(&EvalKey(EvalValue::String("nom".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de nom"))?;
    let description = view_map.get(&EvalKey(EvalValue::String("description".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de description"))?;
    let traits = view_map.get(&EvalKey(EvalValue::String("traits".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de traits"))?;
    let data = view_map.get(&EvalKey(EvalValue::String("donnees".into()))).ok_or_else(|| anyhow!("La vue partielle de l'objet ne contient pas de donnees"))?;

    let mut env = Env::new();
    env.insert("nom", name.clone());
    env.insert("description", description.clone());
    env.insert("traits", traits.clone());
    env.insert("donnees", data.clone());
    env.insert("etat", EvalValue::Object(view_map));

    Ok(env)
}
pub fn mk_new_condition_env(condition: &ConditionInstance, formula: &mut FormulaEngine, env: &Env) -> Result<Env<'static>> {
    let env = {
        let partial = mk_partial_condition_view(condition)?;
        let mut partial_map = IndexMap::new();
        partial_map.insert("etat".into(), partial);
        Env::from_index_map(&Arc::new(partial_map))
            .with_parent(env)
    };
    let id = condition.template.id.clone();
    let name = condition.actual_name(formula, &env)?;
    let description = condition.actual_description(formula, &env)?;
    let traits: Vec<EvalValue> = condition.template.traits.iter()
        .map(|t| {
            match t {
                Deferred::Literal(lit) => Ok(lit.clone().into()),
                Deferred::Formula(form) => {
                    form.clone().call(&[], formula, &env)
                        .map_err(|err| anyhow!("Erreur évaluant le trait différé: {}", err))
                },
            }
        })
        .collect::<Result<Vec<EvalValue>>>()?;
    let data: IndexMap<EvalKey, EvalValue> = Arc::unwrap_or_clone(condition.actual_data())
        .into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<IndexMap<EvalKey, EvalValue>>();

    let traits: Arc<[EvalValue]> = Arc::from(traits);
    let data = Arc::from(data);

    let mut env = Env::new();
    env.insert("id", EvalValue::from(id.clone()));
    env.insert("nom", EvalValue::from(name.clone()));
    env.insert("description", EvalValue::from(description.clone()));
    env.insert("traits", EvalValue::Array(traits.clone()));
    env.insert("donnees", EvalValue::Object(data.clone()));

    let mut map: IndexMap<EvalKey, EvalValue> = IndexMap::new();
    map.insert(EvalKey(EvalValue::from("id")), EvalValue::from(id));
    map.insert(EvalKey(EvalValue::from("nom")), EvalValue::from(name));
    map.insert(EvalKey(EvalValue::from("description")), EvalValue::from(description));
    map.insert(EvalKey(EvalValue::from("traits")), EvalValue::Array(traits.clone()));
    map.insert(EvalKey(EvalValue::from("donnees")), EvalValue::Object(data.clone()));

    env.insert("etat", EvalValue::Object(map.into()));
    Ok(env)
}

/// Create the environment for the entire tale, including all global data and the "recit" object.
pub fn mk_tale_env(tale: &IndexMap<Arc<str>, EvalValue>) -> Result<Env<'static>> {
    let mut envmap: IndexMap<Arc<str>, EvalValue> = IndexMap::new();
    let data: IndexMap<EvalKey, EvalValue> = tale.iter()
        .map(|(k, v)| {
            envmap.insert(k.clone(), v.clone());
            (EvalKey(EvalValue::String(k.clone().into())), v.clone())
        })
        .collect();
    envmap.insert(Arc::from("recit"), EvalValue::Object(data.into()));
    Ok(Env::from_index_map(&Arc::new(envmap)))
}
/// Create the minimal environment for the tale, including only the "recit" object with global data.
pub fn mk_tale_base_env(tale: &Tale) -> Result<Env<'static>> {
    let mut envmap: IndexMap<Arc<str>, EvalValue> = IndexMap::new();
    let data: IndexMap<EvalKey, EvalValue> = tale.data.iter()
        .map(|(k, v)| {
            (EvalKey(EvalValue::String(k.clone().into())), v.clone())
        })
        .collect();
    envmap.insert(Arc::from("recit"), EvalValue::Object(data.into()));
    Ok(Env::from_index_map(&Arc::new(envmap)))
}

/// Builds the formula environment exposed by the current runtime value.
///
/// The env arg is used purely to resolve formulas in the page's tags; it is not retained in the returned environment.
/// 
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
/// 
/// env should contain `recit`, `page`, `test`, `connexion` (from the current/previous page) and `personnage`
pub fn mk_connection_env(connection: &Connection, formula: &mut FormulaEngine, env: &Env) -> Result<Env<'static>> {
    let mut work = Env::new();
    let tags = eval_tags(&connection.tags, formula, env)
        .context("Erreur en évaluant les tags de la connexion")?;
    work.insert("connexion", EvalValue::Object(tags.into()));
    Ok(work)
}

/// Builds the formula environment exposed by the current runtime value.
///
/// The env arg is used purely to resolve formulas in the page's tags; it is not retained in the returned environment.
/// 
/// The returned environment owns cloned bindings, so callers may extend it without
/// mutating the source character, page, item, or state.
/// 
/// env should contain only `recit`, the selected connection's `test`, and its
/// immutable `connexion` tags. In particular, page tags do not read `personnage`.
pub fn mk_page_env(page: &Page, formula: &mut FormulaEngine, env: &Env) -> Result<Env<'static>> {
    let mut work = Env::new();
    let tags = eval_tags(&page.tags, formula, env)
        .with_context(|| format!("Erreur en évaluant les tags de la page `{}`", page.id))?;
    work.insert("page", EvalValue::Object(tags.into()));
    Ok(work)
}

/// Converts a JSON value into an `EvalValue`, recursively handling arrays and objects.
pub fn json_to_eval_value(value: &JsonValue) -> Result<EvalValue> {
    match value {
        JsonValue::Null => Ok(EvalValue::Null),
        JsonValue::Bool(b) => Ok(EvalValue::Bool(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(EvalValue::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(EvalValue::Float(f))
            } else {
                bail!("Nombre JSON invalide")
            }
        }
        JsonValue::String(s) => Ok(EvalValue::String(s.clone().into())),
        JsonValue::Array(arr) => {
            let mut vec = Vec::with_capacity(arr.len());
            for v in arr {
                vec.push(json_to_eval_value(v)?);
            }
            Ok(EvalValue::Array(vec.into()))
        }
        JsonValue::Object(obj) => {
            let mut map = IndexMap::new();
            for (k, v) in obj {
                map.insert(EvalKey(EvalValue::String(k.clone().into())), json_to_eval_value(v)?);
            }
            Ok(EvalValue::Object(map.into()))
        }
    }
}

/// Converts a dictionary with `EvalKey` keys into a map with `Arc<str>` keys.
/// Returns an error if any key is not a string.
pub fn dict_to_map(dict: IndexMap<EvalKey, EvalValue>) -> Result<IndexMap<Arc<str>, EvalValue>> {
    let map: IndexMap<Arc<str>, EvalValue> = dict.into_iter()
        .map(|(k, v)| match k.0 {
            EvalValue::String(s) => Ok((s.into(), v)),
            _ => bail!("Clé non valide dans le dictionnaire"),
        })
        .collect::<Result<IndexMap<Arc<str>, EvalValue>>>()?;
    Ok(map)
}
/// Converts a map with `Arc<str>` keys into a dictionary with `EvalKey` keys.
/// Each string key is wrapped in an `EvalKey` containing an `EvalValue::String`.
pub fn map_to_dict(map: IndexMap<Arc<str>, EvalValue>) -> IndexMap<EvalKey, EvalValue> {
    map.into_iter()
        .map(|(k, v)| (EvalKey(EvalValue::String(k.into())), v))
        .collect()
}

///
/// 
pub fn map_to_json(map: IndexMap<Arc<str>, EvalValue>) -> Result<JsonValue> {
    let mut obj = serde_json::Map::new();
    for (k, v) in map {
        obj.insert(k.as_ref().to_owned(), eval_value_to_json(&v)?);
    }
    Ok(JsonValue::Object(obj))
}

/// Converts an `EvalValue` into a JSON value, recursively handling arrays and objects.
pub fn eval_value_to_json(value: &EvalValue) -> Result<JsonValue> {
    match value {
        EvalValue::Null => Ok(JsonValue::Null),
        EvalValue::Bool(b) => Ok(JsonValue::Bool(*b)),
        EvalValue::Int(i) => Ok(JsonValue::Number((*i).into())),
        EvalValue::Float(f) => Ok(JsonValue::Number(serde_json::Number::from_f64(*f).ok_or_else(|| anyhow!("Nombre flottant invalide"))?)),
        EvalValue::String(s) => Ok(JsonValue::String(s.as_ref().into())),
        EvalValue::Array(arr) => {
            let mut vec = Vec::with_capacity(arr.len());
            for v in arr.iter() {
                vec.push(eval_value_to_json(v)?);
            }
            Ok(JsonValue::Array(vec))
        }
        EvalValue::Object(obj) => {
            let mut map = IndexMap::new();
            for (k, v) in obj.iter() {
                if let EvalValue::String(s) = &k.0 {
                    map.insert(s.as_ref().into(), eval_value_to_json(v)?);
                } else {
                    bail!("Clé non valide dans l'objet");
                }
            }
            let jmap: serde_json::Map<String, JsonValue> = map.into_iter().collect();
            Ok(JsonValue::Object(jmap))
        }
        _ => bail!("Type de valeur non pris en charge pour la conversion en JSON"),
    }
}

/// Resolves a formula for a quantity, ensuring it is a non-negative integer.
/// 
/// # Arguments
/// * `value` - The formula value to resolve.
/// * `formula` - The formula engine used for resolution.
/// * `env` - The environment in which to resolve the formula.
/// 
/// # Returns
/// * `Result<i64>` - The resolved quantity as a non-negative integer.
pub fn resolve_formula_quantity(value: &EvalValue, formula: &mut FormulaEngine, env: &Env) -> Result<i64> {
    let quantity = formula.resolve_formula_int(value, env)?;
    if quantity < 0 {
        bail!("Une quantité doit être un entier positif. A reçu {quantity}.")
    }
    Ok(quantity)
}