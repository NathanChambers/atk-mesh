use crate::{Mesh, objects::MeshObject, render::CaptureVisibility};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn obj_material_name(name: Option<&str>) -> String {
    let Some(name) = name else {
        return "off".into();
    };
    if name == "off" {
        return "%6Fff".into();
    }
    let mut label = String::new();
    for ch in name.chars() {
        if ch.is_whitespace() || ch.is_control() || matches!(ch, '%' | '#' | '\\') {
            for byte in ch.to_string().bytes() {
                label.push_str(&format!("%{byte:02X}"));
            }
        } else {
            label.push(ch);
        }
    }
    label
}

pub(crate) struct ExportGroup<'a> {
    pub name: &'a str,
    pub members: Vec<(u64, &'a MeshObject)>,
    pub grouped: bool,
}

impl ExportGroup<'_> {
    pub fn vertices(&self) -> BTreeSet<u64> {
        self.members
            .iter()
            .flat_map(|(_, object)| object.vertices.iter().copied())
            .collect()
    }
    pub fn pivots(&self) -> Value {
        let entries: Vec<_> = self
            .members
            .iter()
            .map(|(id, object)| json!({"object":id,"name":object.name,"origin":object.origin}))
            .collect();
        Value::Array(entries)
    }
}

pub(crate) fn groups<'a>(
    mesh: &'a Mesh,
    visibility: &CaptureVisibility,
    merge: Option<&str>,
) -> Result<Vec<ExportGroup<'a>>, String> {
    if merge.is_some_and(|mode| !matches!(mode, "all" | "collection")) {
        return Err("merge must be all or collection".into());
    }
    let mut batches = BTreeMap::<(bool, u64), Vec<(u64, &MeshObject)>>::new();
    for (&id, object) in &mesh.objects {
        if !object
            .vertices
            .first()
            .is_some_and(|v| visibility.includes_vertex(v))
        {
            continue;
        }
        let key = match merge {
            Some("all") => (true, 0),
            Some("collection") => {
                let collection = mesh.primary_collections.get(&id).copied().or_else(|| {
                    mesh.collections
                        .iter()
                        .find(|(_, group)| group.objects.contains(&id))
                        .map(|(&id, _)| id)
                });
                collection.map_or((false, id), |id| (true, id))
            }
            _ => (false, id),
        };
        batches.entry(key).or_default().push((id, object));
    }
    Ok(batches
        .into_iter()
        .map(|((grouped, key), members)| {
            let name = if grouped && merge == Some("collection") {
                &mesh.collections[&key].name
            } else {
                &members[0].1.name
            };
            ExportGroup {
                name,
                members,
                grouped,
            }
        })
        .collect())
}
