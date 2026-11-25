use crate::{Mesh, objects, topology};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) mod generator_uv;
mod triangle_report;

#[derive(Clone, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub faces: BTreeSet<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<Value>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub uv_cap_faces: BTreeSet<u64>,
}

pub(super) fn is_zero(value: &u64) -> bool {
    *value == 0
}

pub(super) fn generator(command: &str) -> bool {
    matches!(
        command,
        "object_create_box"
            | "object_add_box"
            | "object_profile"
            | "extrude_polygon"
            | "revolve"
            | "sweep"
            | "loft"
            | "quad_strip"
            | "solid_boolean"
            | "subdivision_surface"
    )
}

pub(super) fn bounds(mesh: &Mesh, vertices: &BTreeSet<u64>) -> Value {
    bounds_points(mesh, vertices).map_or(Value::Null, |[min, max]| json!({"min":min,"max":max}))
}

pub(super) fn bounds_points(mesh: &Mesh, vertices: &BTreeSet<u64>) -> Option<[[f64; 3]; 2]> {
    if vertices.is_empty() {
        return None;
    }
    let min: [f64; 3] = std::array::from_fn(|axis| {
        vertices
            .iter()
            .map(|id| mesh.vertices[id][axis])
            .fold(f64::INFINITY, f64::min)
    });
    let max: [f64; 3] = std::array::from_fn(|axis| {
        vertices
            .iter()
            .map(|id| mesh.vertices[id][axis])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    Some([min, max])
}

pub(super) fn vertices(mesh: &Mesh, faces: &BTreeSet<u64>) -> BTreeSet<u64> {
    faces
        .iter()
        .filter_map(|id| mesh.faces.get(id))
        .flatten()
        .copied()
        .collect()
}

pub(super) fn named(mesh: &Mesh, name: &str) -> Result<u64, String> {
    mesh.components
        .iter()
        .find(|(_, component)| component.name == name)
        .map(|(id, _)| *id)
        .ok_or_else(|| format!("unknown named region: {name}"))
}

pub(super) fn reconcile(mesh: &mut Mesh) {
    for component in mesh.components.values_mut() {
        component.faces.retain(|id| mesh.faces.contains_key(id));
        component.uv_cap_faces.retain(|id| component.faces.contains(id));
    }
    mesh.components
        .retain(|_, component| !component.faces.is_empty());
    mesh.intersection_allowances.retain(|a| {
        a.faces
            .is_none_or(|ids| ids.iter().all(|id| mesh.faces.contains_key(id)))
            && a.regions
                .is_none_or(|ids| ids.iter().all(|id| mesh.components.contains_key(id)))
    });
}

pub(super) fn inherit(before: &Mesh, after: &mut Mesh) {
    for (id, component) in &before.components {
        let Some(current) = after.components.get(id) else {
            continue;
        };
        let mut known = vertices(before, &component.faces);
        known.extend(vertices(after, &current.faces));
        let created: Vec<_> = after
            .faces
            .iter()
            .filter(|(face, points)| {
                !before.faces.contains_key(face) && points.iter().all(|v| known.contains(v))
            })
            .map(|(id, _)| *id)
            .collect();
        after.components.get_mut(id).unwrap().faces.extend(created);
    }
}

pub(super) fn validate(mesh: &Mesh) -> Result<(), String> {
    if mesh.intersection_allowances.len() > 8192 {
        return Err("intersection allowance budget exceeded (8192)".into());
    }
    for allowance in &mesh.intersection_allowances {
        if allowance.reason.trim().is_empty()
            || allowance.faces.is_some() == allowance.regions.is_some()
        {
            return Err(
                "intersection allowance requires a reason and exactly one face or region pair"
                    .into(),
            );
        }
        if allowance.faces.is_some_and(|ids| {
            ids[0] == ids[1] || ids.iter().any(|id| !mesh.faces.contains_key(id))
        }) || allowance.regions.is_some_and(|ids| {
            ids[0] == ids[1] || ids.iter().any(|id| !mesh.components.contains_key(id))
        }) {
            return Err(
                "intersection allowance references missing or identical faces/regions".into(),
            );
        }
    }
    let mut names = BTreeSet::new();
    for component in mesh.components.values() {
        if component.name.trim().is_empty()
            || component.name.chars().any(char::is_control)
            || !names.insert(&component.name)
        {
            return Err(
                "region names must be unique, nonempty and contain no control characters".into(),
            );
        }
        if component.faces.is_empty()
            || component
                .faces
                .iter()
                .any(|id| !mesh.faces.contains_key(id))
        {
            return Err("component references missing faces".into());
        }
        if !component.uv_cap_faces.is_subset(&component.faces) {
            return Err("generator UV caps must belong to the component".into());
        }
    }
    Ok(())
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    if request["report"] == "triangles" {
        return triangle_report::query(mesh, request, revision);
    }
    let region = request
        .get("region")
        .map(|v| {
            let name = v.as_str().ok_or("region must be a name")?;
            named(mesh, name)
        })
        .transpose()?;
    let object = request
        .get("object")
        .map(|value| value.as_u64().ok_or("object must be an ID"))
        .transpose()?;
    if object.is_some_and(|id| !mesh.objects.contains_key(&id)) {
        return Err("unknown object".into());
    }
    let items: Vec<_> = mesh.components.iter().filter_map(|(id, component)| {
        if region.is_some_and(|region|region!=*id){return None;}
        let vertices = vertices(mesh, &component.faces);
        let owners: BTreeSet<_> = mesh.objects.iter().filter(|(_, object)| !object.vertices.is_disjoint(&vertices)).map(|(id, _)| *id).collect();
        if object.is_some_and(|id| !owners.contains(&id)) { return None; }
        Some(json!({"id":id,"region":component.name,"objects":owners,"faces":component.faces,"vertices":vertices,"face_count":component.faces.len(),"vertex_count":vertices.len(),"bounds":bounds(mesh,&vertices),"recipe":component.recipe,"identity":"authored region; may contain several connected components after edits"}))
    }).collect();
    let mut result = crate::responses::slice(&json!(items), request)?;
    result["revision"] = json!(revision);
    Ok(result)
}

pub(super) fn remove(mesh: &mut Mesh, faces: &BTreeSet<u64>) {
    let candidates = vertices(mesh, faces);
    mesh.face_id_high_watermark = mesh
        .face_id_high_watermark
        .max(*mesh.faces.keys().next_back().unwrap_or(&0));
    mesh.vertex_id_high_watermark = mesh
        .vertex_id_high_watermark
        .max(*mesh.vertices.keys().next_back().unwrap_or(&0));
    mesh.faces.retain(|id, _| !faces.contains(id));
    let used: BTreeSet<u64> = mesh.faces.values().flatten().copied().collect();
    let removed: BTreeSet<_> = candidates.difference(&used).copied().collect();
    mesh.vertices.retain(|id, _| !removed.contains(id));
    mesh.selection.retain(|id| !removed.contains(id));
    for object in mesh.objects.values_mut() {
        object.vertices.retain(|id| !removed.contains(id));
    }
    mesh.flat_faces.retain(|id| !faces.contains(id));
    crate::uv::reconcile(mesh);
    crate::rig::reconcile(mesh);
}

pub(super) fn generate(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    let object = request
        .get("object")
        .map(|v| v.as_u64().ok_or("object must be an ID"))
        .transpose()?;
    let mode = request
        .get("mode")
        .map(|v| v.as_str().ok_or("mode must be append or replace_region"))
        .transpose()?;
    if object.is_some() && mode.is_none() {
        return Err("generating inside an object requires mode: append or replace_region".into());
    }
    if mode.is_some_and(|mode| !matches!(mode, "append" | "replace_region")) {
        return Err("mode must be append or replace_region".into());
    }
    if object.is_some_and(|id| !mesh.objects.contains_key(&id)) {
        return Err("unknown object".into());
    }
    if mesh.components.len() >= 4096 {
        return Err("component budget exceeded (4096)".into());
    }

    let existing_region = request
        .get("region")
        .map(|v| v.as_str().ok_or("region must be a name"))
        .transpose()?;
    let mut replaced = BTreeSet::new();
    let mut finish = None;
    let component_id = if mode == Some("replace_region") {
        let object = object.ok_or("replace_region requires object")?;
        let name = existing_region.ok_or("replace_region requires region")?;
        let id = named(mesh, name)?;
        let component = &mesh.components[&id];
        let selected = vertices(mesh, &component.faces);
        if !selected.is_subset(&mesh.objects[&object].vertices) {
            return Err("region lies outside the requested object".into());
        }
        let other_faces_touch = mesh.faces.iter().any(|(face, ids)| {
            !component.faces.contains(face) && ids.iter().any(|id| selected.contains(id))
        });
        if other_faces_touch {
            return Err("region shares vertices with surrounding surfaces; detach or use surface repair instead of replacing a component".into());
        }
        replaced = component.faces.clone();
        let prior = component.recipe.as_ref().and_then(|r| r["command"].as_str());
        if (prior == Some(command) || (prior == Some("object_create_box") && command == "object_add_box")) && !matches!(command, "solid_boolean" | "subdivision_surface") {
            finish = Some(ComponentFinish::read(mesh, &replaced));
        }
        remove(mesh, &replaced);
        id
    } else {
        if existing_region.is_some_and(|name| named(mesh, name).is_ok()) {
            return Err("region already exists; choose replace_region explicitly".into());
        }
        mesh.component_id_high_watermark = mesh
            .component_id_high_watermark
            .max(*mesh.components.keys().next_back().unwrap_or(&0));
        let id = mesh
            .component_id_high_watermark
            .checked_add(1)
            .ok_or("component IDs exhausted")?;
        mesh.component_id_high_watermark = id;
        id
    };
    let before_vertices: BTreeSet<_> = mesh.vertices.keys().copied().collect();
    let before_faces: BTreeSet<_> = mesh.faces.keys().copied().collect();
    let before_objects = mesh.objects.clone();
    let mut operation = match command {
        "object_create_box" | "object_add_box" => objects::edit(mesh, command, request)?,
        "object_profile" => crate::profiles::create(mesh, request)?,
        "revolve" => crate::shaping::revolve(mesh, request)?,
        "extrude_polygon" => crate::shaping::extrude_polygon(mesh, request)?,
        "sweep" => crate::sweep::create(mesh, request)?,
        "solid_boolean" => crate::solid::generate(mesh, request)?,
        "subdivision_surface" => crate::subdivision::generate(mesh, request)?,
        "loft" | "quad_strip" => crate::connected::loft(mesh, request, command == "quad_strip")?,
        _ => return Err("unknown construction generator".into()),
    };
    let created_vertices: BTreeSet<_> = mesh
        .vertices
        .keys()
        .filter(|id| !before_vertices.contains(id))
        .copied()
        .collect();
    let created_faces: BTreeSet<_> = mesh
        .faces
        .keys()
        .filter(|id| !before_faces.contains(id))
        .copied()
        .collect();
    let name = existing_region
        .map(str::to_string)
        .unwrap_or_else(|| format!("component-{component_id}"));
    if let Some(finish) = finish {
        operation["finish_transferred"] = json!(finish.apply(mesh, &created_faces));
    }
    let uv_cap_faces = if let Some(uv) = request.get("uv") {
        generator_uv::assign(mesh, command, request, uv, &mut operation, &created_faces)?
    } else {
        BTreeSet::new()
    };
    mesh.components.insert(
        component_id,
        Component {
            name: name.clone(),
            faces: created_faces.clone(),
            recipe: Some(request.clone()),
            uv_cap_faces,
        },
    );
    let region_vertices = vertices(mesh, &created_faces);
    let new_bounds = bounds(mesh, &region_vertices);
    let mut candidates = vec![];
    for (id, old_object) in &before_objects {
        if old_object.vertices.is_empty() {
            continue;
        }
        let old_bounds = bounds(mesh, &old_object.vertices);
        let overlap = (0..3).all(|axis| {
            new_bounds["min"][axis].as_f64().unwrap() < old_bounds["max"][axis].as_f64().unwrap()
                && new_bounds["max"][axis].as_f64().unwrap()
                    > old_bounds["min"][axis].as_f64().unwrap()
        });
        if overlap {
            candidates.push(json!({"object":id,"kind":"bounds_overlap_candidate","surface_intersection":"not_checked"}));
        }
    }
    topology::validate_topology(mesh)?;
    operation["component_id"] = json!(component_id);
    operation["region"] = json!(name);
    operation["mode"] = json!(mode.unwrap_or("append"));
    operation["created_vertices"] = json!(created_vertices);
    operation["created_face_ids"] = json!(created_faces);
    if operation.get("created_faces").is_none() {
        operation["created_faces"] = json!(created_faces);
    }
    operation["removed_faces"] = json!(replaced);
    operation["geometry_counts"] =
        json!({"vertices":created_vertices.len(),"faces":created_faces.len()});
    operation["bounds"] = new_bounds;
    operation["warnings"] = json!(candidates);
    Ok(operation)
}

struct FaceFinish {
    ring: Vec<u64>,
    uvs: Option<std::collections::BTreeMap<u64, [f64; 2]>>,
    texture: Option<String>,
    flat: bool,
}

struct ComponentFinish {
    faces: Vec<FaceFinish>,
    vertices: Vec<(u64, Option<[f64; 3]>, Option<Vec<crate::rig::Influence>>)>,
}

impl ComponentFinish {
    fn read(mesh: &Mesh, faces: &BTreeSet<u64>) -> Self {
        let vertices = vertices(mesh, faces)
            .into_iter()
            .map(|v| {
                let weights = mesh
                    .rig
                    .as_ref()
                    .and_then(|rig| rig.weights.get(&v))
                    .cloned();
                (v, mesh.vertex_colors.get(&v).copied(), weights)
            })
            .collect();
        let faces = faces
            .iter()
            .map(|id| FaceFinish {
                ring: mesh.faces[id].clone(),
                uvs: mesh.uvs.get(id).cloned(),
                texture: mesh.face_textures.get(id).cloned(),
                flat: mesh.flat_faces.contains(id),
            })
            .collect();
        Self { faces, vertices }
    }

    fn apply(self, mesh: &mut Mesh, faces: &BTreeSet<u64>) -> bool {
        let vertices = vertices(mesh, faces);
        if self.faces.len() != faces.len() || self.vertices.len() != vertices.len() {
            return false;
        }
        let remap: std::collections::BTreeMap<_, _> = self
            .vertices
            .iter()
            .zip(vertices)
            .map(|((v, _, _), new)| (*v, new))
            .collect();
        for (old, id) in self.faces.iter().zip(faces) {
            if !old
                .ring
                .iter()
                .map(|v| remap[v])
                .eq(mesh.faces[id].iter().copied())
            {
                return false;
            }
        }
        for (old, &id) in self.faces.into_iter().zip(faces) {
            if let Some(uvs) = old.uvs {
                mesh.uvs
                    .insert(id, uvs.into_iter().map(|(v, uv)| (remap[&v], uv)).collect());
            }
            if let Some(texture) = old.texture {
                mesh.face_textures.insert(id, texture);
            }
            if old.flat {
                mesh.flat_faces.insert(id);
            }
        }
        for (v, color, weights) in self.vertices {
            if let Some(color) = color {
                mesh.vertex_colors.insert(remap[&v], color);
            }
            if let Some(rig) = &mut mesh.rig
                && let Some(weights) = weights
            {
                rig.weights.insert(remap[&v], weights);
            }
        }
        true
    }
}
