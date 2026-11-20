use crate::Mesh;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    pub objects: BTreeSet<u64>,
}

pub(super) fn members(mesh: &Mesh, value: &Value) -> Result<BTreeSet<u64>, String> {
    let id = value.as_u64().ok_or("collection must be an ID")?;
    let collection = mesh.collections.get(&id).ok_or("unknown collection")?;
    Ok(collection.objects.clone())
}

pub(super) fn validate(mesh: &Mesh) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for collection in mesh.collections.values() {
        if collection.name.trim().is_empty()
            || collection.name.chars().any(char::is_control)
            || !names.insert(&collection.name)
        {
            return Err(
                "collection names must be nonempty, unique and contain no control characters"
                    .into(),
            );
        }
        if collection
            .objects
            .iter()
            .any(|id| !mesh.objects.contains_key(id))
        {
            return Err("collection contains unknown object".into());
        }
    }
    for (object, collection) in &mesh.primary_collections {
        if !mesh.objects.contains_key(object)
            || !mesh
                .collections
                .get(collection)
                .is_some_and(|c| c.objects.contains(object))
        {
            return Err("primary collection must contain its object".into());
        }
    }
    Ok(())
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    let id = if command == "collection_create" {
        let id = mesh
            .collection_id_high_watermark
            .max(mesh.collections.keys().next_back().copied().unwrap_or(0))
            .checked_add(1)
            .ok_or("collection IDs exhausted")?;
        mesh.collection_id_high_watermark = id;
        mesh.collections.insert(
            id,
            Collection {
                name: String::new(),
                objects: BTreeSet::new(),
            },
        );
        id
    } else {
        let id = request["collection"]
            .as_u64()
            .ok_or("collection is required")?;
        members(mesh, &json!(id))?;
        id
    };
    match command {
        "collection_create" | "collection_update" => {
            if request.get("objects").is_some()
                && (request.get("add_objects").is_some() || request.get("remove_objects").is_some())
            {
                return Err("use objects or add_objects/remove_objects, not both".into());
            }
            if let Some(name) = request.get("name") {
                mesh.collections.get_mut(&id).unwrap().name =
                    name.as_str().ok_or("name must be a string")?.into();
            }
            if let Some(objects) = request.get("objects") {
                let objects: BTreeSet<u64> = serde_json::from_value(objects.clone())
                    .map_err(|_| "objects must be an array of object IDs")?;
                mesh.collections.get_mut(&id).unwrap().objects = objects;
            }
            for (key, add) in [("add_objects", true), ("remove_objects", false)] {
                let Some(value) = request.get(key) else {
                    continue;
                };
                let objects: BTreeSet<u64> = serde_json::from_value(value.clone())
                    .map_err(|_| format!("{key} must be an array of object IDs"))?;
                if objects
                    .iter()
                    .any(|object| !mesh.objects.contains_key(object))
                {
                    return Err("unknown collection member object".into());
                }
                for object in objects {
                    if add {
                        mesh.collections
                            .get_mut(&id)
                            .unwrap()
                            .objects
                            .insert(object);
                    } else {
                        mesh.collections
                            .get_mut(&id)
                            .unwrap()
                            .objects
                            .remove(&object);
                    }
                }
            }
            mesh.primary_collections.retain(|object, primary| {
                *primary != id || mesh.collections[&id].objects.contains(object)
            });
            if request.get("primary").is_none() {
                for object in &mesh.collections[&id].objects {
                    mesh.primary_collections.entry(*object).or_insert(id);
                }
            }
            if let Some(primary) = request.get("primary") {
                let primary = primary.as_bool().ok_or("primary must be a boolean")?;
                if primary {
                    for object in &mesh.collections[&id].objects {
                        mesh.primary_collections.insert(*object, id);
                    }
                } else {
                    mesh.primary_collections
                        .retain(|_, collection| *collection != id);
                }
            }
        }
        "collection_delete" => {
            mesh.collections.remove(&id);
            mesh.primary_collections
                .retain(|_, collection| *collection != id);
        }
        "select_collection" => {
            mesh.selection = mesh.collections[&id]
                .objects
                .iter()
                .flat_map(|object| mesh.objects[object].vertices.iter().copied())
                .collect();
        }
        _ => return Err("unknown collection command".into()),
    }
    Ok(json!({"collection":id,"object_count":mesh.collections.get(&id).map(|c| c.objects.len())}))
}

pub(super) struct Palette {
    pub colors: BTreeMap<u64, [u8; 3]>,
    pub legend: BTreeMap<String, u64>,
}

pub(super) fn colors(mesh: &Mesh) -> Result<Palette, String> {
    let mut used = BTreeSet::new();
    let mut colors = BTreeMap::new();
    let mut legend = BTreeMap::new();
    for (index, id) in mesh.collections.keys().enumerate() {
        let color = crate::render::categorical_color(index, &mut used)?;
        colors.insert(*id, color);
        legend.insert(
            format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]),
            *id,
        );
    }
    Ok(Palette { colors, legend })
}
