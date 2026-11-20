use crate::{Mesh, objects, topology};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn join(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    if request.get("object").is_some() {
        return Err("object_join uses target and objects; object scope is not supported".into());
    }

    let target = request["target"].as_u64().ok_or("target is required")?;
    let destination = mesh
        .objects
        .get(&target)
        .ok_or("unknown target object")?
        .clone();
    let sources: BTreeSet<u64> = serde_json::from_value(request["objects"].clone())
        .map_err(|_| "objects must be an array of source object IDs")?;
    if sources.is_empty() || sources.contains(&target) {
        return Err("provide source objects distinct from target".into());
    }

    let mut involved = sources.clone();
    involved.insert(target);
    for id in &involved {
        let object = mesh.objects.get(id).ok_or("unknown source object")?;
        if object.instance.is_some()
            || mesh.objects.values().any(|other| {
                other
                    .instance
                    .as_ref()
                    .is_some_and(|link| link.master == *id)
            })
        {
            return Err("make linked objects and their instances unique before joining".into());
        }

        if object.smooth_angle != destination.smooth_angle {
            return Err(
                "joined objects must have matching smooth_angle; use shade_smooth first".into(),
            );
        }
    }

    mesh.object_id_high_watermark = mesh
        .object_id_high_watermark
        .max(*mesh.objects.keys().next_back().unwrap_or(&0));
    let mut vertices = destination.vertices;
    for id in &sources {
        let source = mesh.objects.remove(id).unwrap();
        vertices.extend(source.vertices);
        mesh.primary_collections.remove(id);
    }
    mesh.objects.get_mut(&target).unwrap().vertices = vertices.clone();
    for collection in mesh.collections.values_mut() {
        let had_source = collection.objects.iter().any(|id| sources.contains(id));
        collection.objects.retain(|id| !sources.contains(id));
        if had_source {
            collection.objects.insert(target);
        }
    }
    mesh.selection = vertices;
    Ok(
        json!({"object":target,"removed_objects":sources,"positions_changed":0,"vertices_welded":0,"faces_removed":0}),
    )
}

pub(super) fn delete_faces(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let remove_orphans = request["remove_orphan_vertices"]
        .as_bool()
        .ok_or("remove_orphan_vertices must explicitly be true or false")?;
    let object_id = request["object"].as_u64().ok_or("object is required")?;
    let object = mesh.objects.get(&object_id).ok_or("unknown object")?;
    let faces: BTreeSet<u64> = serde_json::from_value(request["faces"].clone())
        .map_err(|_| "faces must be an array of face IDs")?;
    if faces.is_empty() {
        return Err("faces must not be empty".into());
    }

    let mut candidates = BTreeSet::new();
    for id in &faces {
        let face = mesh.faces.get(id).ok_or("unknown face")?;
        if !face.iter().all(|vertex| object.vertices.contains(vertex)) {
            return Err("face lies outside the requested object".into());
        }
        candidates.extend(face.iter().copied());
    }
    let has_remaining_faces = mesh
        .faces
        .iter()
        .any(|(id, face)| object.vertices.contains(&face[0]) && !faces.contains(id));
    if !has_remaining_faces && !mesh.defer_validation {
        return Err("use object_delete to remove an entire object's faces".into());
    }

    mesh.face_id_high_watermark = mesh
        .face_id_high_watermark
        .max(*mesh.faces.keys().next_back().unwrap_or(&0));
    mesh.vertex_id_high_watermark = mesh
        .vertex_id_high_watermark
        .max(*mesh.vertices.keys().next_back().unwrap_or(&0));
    mesh.faces.retain(|id, _| !faces.contains(id));
    let used: BTreeSet<u64> = mesh.faces.values().flatten().copied().collect();
    let removed_vertices: BTreeSet<u64> = if remove_orphans {
        candidates.difference(&used).copied().collect()
    } else {
        BTreeSet::new()
    };
    mesh.vertices.retain(|id, _| !removed_vertices.contains(id));
    mesh.selection.retain(|id| !removed_vertices.contains(id));
    Ok(
        json!({"object":object_id,"removed_faces":faces,"removed_vertices":removed_vertices,"positions_changed":0}),
    )
}

pub(super) fn weld_coincident(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let id = request["object"].as_u64().ok_or("object is required")?;
    let object = mesh.objects.get(&id).ok_or("unknown object")?;
    let mut positions = BTreeMap::new();
    let mut remap = BTreeMap::new();
    for vertex in &object.vertices {
        let position = mesh.vertices[vertex];
        let key = position.map(|value| if value == 0.0 { 0 } else { value.to_bits() });
        let target = *positions.entry(key).or_insert(*vertex);
        if target != *vertex {
            remap.insert(*vertex, target);
        }
    }
    let expected = request["expected_removed_vertices"]
        .as_u64()
        .ok_or("expected_removed_vertices is required")?;
    if remap.len() as u64 != expected {
        return Err(format!(
            "coincident weld would remove {} vertices; expected {expected}",
            remap.len()
        ));
    }
    if remap.is_empty() {
        return Err("no exactly coincident vertices in object".into());
    }
    mesh.vertex_id_high_watermark = mesh
        .vertex_id_high_watermark
        .max(*mesh.vertices.keys().next_back().unwrap_or(&0));
    for face in mesh.faces.values_mut() {
        for vertex in face {
            if let Some(target) = remap.get(vertex) {
                *vertex = *target;
            }
        }
    }
    for vertex in remap.keys() {
        mesh.vertices.remove(vertex);
    }
    mesh.selection = mesh
        .selection
        .iter()
        .map(|vertex| remap.get(vertex).copied().unwrap_or(*vertex))
        .collect();
    topology::validate_topology(mesh)?;
    Ok(
        json!({"object":id,"removed_vertices":remap.len(),"vertex_remap":remap,"positions_changed":0,"faces_removed":0}),
    )
}

pub(super) fn separate(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    topology::validate_topology(mesh)?;
    let source = request["object"].as_u64().ok_or("object is required")?;
    let object = mesh.objects.get(&source).ok_or("unknown object")?.clone();
    if object.instance.is_some()
        || mesh.objects.values().any(|other| {
            other
                .instance
                .as_ref()
                .is_some_and(|link| link.master == source)
        })
    {
        return Err("make linked objects unique before separating faces".into());
    }
    let parts = request["parts"]
        .as_array()
        .ok_or("parts must be an array of named face sets")?;
    if parts.is_empty() || parts.len() > 64 {
        return Err("provide 1..64 parts".into());
    }
    let source_faces: BTreeSet<_> = mesh
        .faces
        .iter()
        .filter(|(_, face)| object.vertices.contains(&face[0]))
        .map(|(face, _)| *face)
        .collect();
    let mut assignments = BTreeMap::new();
    let mut destinations = vec![source];
    for part in parts {
        let faces: BTreeSet<u64> = serde_json::from_value(part["faces"].clone())
            .map_err(|_| "part faces must be an array of IDs")?;
        if faces.is_empty() {
            return Err("part must contain faces".into());
        }
        let origin = part
            .get("origin")
            .map(crate::vector)
            .transpose()?
            .unwrap_or(object.origin);
        let destination =
            objects::destination_object(mesh, &json!({"name":part["name"],"origin":origin}))?;
        destinations.push(destination);
        for face in faces {
            if !source_faces.contains(&face) || assignments.insert(face, destination).is_some() {
                return Err("part faces must belong to source and cannot overlap".into());
            }
        }
        mesh.objects.get_mut(&destination).unwrap().smooth_angle = object.smooth_angle;
        for collection in mesh.collections.values_mut() {
            if collection.objects.contains(&source) {
                collection.objects.insert(destination);
            }
        }
        if let Some(primary) = mesh.primary_collections.get(&source).copied() {
            mesh.primary_collections.insert(destination, primary);
        }
    }
    if assignments.len() == source_faces.len() {
        return Err("leave some faces on source object".into());
    }

    // Split each original vertex into edge-connected face fans within each new owner.
    let mut incident = BTreeMap::<(u64, u64), Vec<u64>>::new();
    for face_id in &source_faces {
        let owner = assignments.get(face_id).copied().unwrap_or(source);
        for vertex in &mesh.faces[face_id] {
            incident.entry((*vertex, owner)).or_default().push(*face_id);
        }
    }
    let original_faces = mesh.faces.clone();
    for id in &destinations {
        mesh.objects.get_mut(id).unwrap().vertices.clear();
    }
    let mut reused = BTreeSet::new();
    let mut duplicated = BTreeMap::<u64, Vec<u64>>::new();
    for ((vertex, owner), faces) in incident {
        let mut neighbors = BTreeMap::<u64, Vec<u64>>::new();
        for face_id in &faces {
            let face = &original_faces[face_id];
            let index = face.iter().position(|id| *id == vertex).unwrap();
            for adjacent in [
                face[(index + face.len() - 1) % face.len()],
                face[(index + 1) % face.len()],
            ] {
                neighbors.entry(adjacent).or_default().push(*face_id);
            }
        }
        let mut remaining: BTreeSet<_> = faces.into_iter().collect();
        while let Some(seed) = remaining.first().copied() {
            let mut pending = vec![seed];
            let mut fan = vec![];
            while let Some(face_id) = pending.pop() {
                if !remaining.remove(&face_id) {
                    continue;
                }
                fan.push(face_id);
                let face = &original_faces[&face_id];
                let index = face.iter().position(|id| *id == vertex).unwrap();
                for adjacent in [
                    face[(index + face.len() - 1) % face.len()],
                    face[(index + 1) % face.len()],
                ] {
                    pending.extend(&neighbors[&adjacent]);
                }
            }
            let target = if reused.insert(vertex) {
                vertex
            } else {
                let position = mesh.vertices[&vertex];
                let created = mesh.add_vertex(position)?;
                duplicated.entry(vertex).or_default().push(created);
                created
            };
            for face_id in fan {
                for id in mesh.faces.get_mut(&face_id).unwrap() {
                    if *id == vertex {
                        *id = target;
                    }
                }
            }
            mesh.objects
                .get_mut(&owner)
                .unwrap()
                .vertices
                .insert(target);
        }
    }
    for vertex in object.vertices.difference(&reused) {
        mesh.objects
            .get_mut(&source)
            .unwrap()
            .vertices
            .insert(*vertex);
    }
    topology::validate_topology(mesh)?;
    Ok(
        json!({"source_object":source,"objects":destinations,"duplicated_vertices":duplicated,"faces_reassigned":assignments.len(),"positions_changed":0,"faces_removed":0,"boundary_note":"Separated regions retain exact surface positions. Cuts through connected surfaces create matching open boundaries; no artificial caps are added."}),
    )
}
