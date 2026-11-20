use crate::{Mesh, objects, vector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshInstance {
    pub master: u64,
    pub anchor: [f64; 3],
    pub rotation: [[f64; 3]; 3],
    pub vertices: BTreeMap<u64, u64>,
    pub faces: BTreeMap<u64, u64>,
}

fn rotated(rotation: &[[f64; 3]; 3], point: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|row| (0..3).map(|axis| rotation[row][axis] * point[axis]).sum())
}

fn rotate(point: [f64; 3], angles: [f64; 3]) -> [f64; 3] {
    let mut result = point;
    for (axis, angle) in angles.iter().enumerate() {
        let a = (axis + 1) % 3;
        let b = (axis + 2) % 3;
        let (sin, cos) = angle.to_radians().sin_cos();
        let first = result[a] * cos - result[b] * sin;
        result[b] = result[a] * sin + result[b] * cos;
        result[a] = first;
    }
    result
}

fn rotation_matrix(angles: [f64; 3]) -> [[f64; 3]; 3] {
    let columns: [[f64; 3]; 3] = std::array::from_fn(|column| {
        let unit = std::array::from_fn(|axis| if axis == column { 1.0 } else { 0.0 });
        rotate(unit, angles)
    });
    std::array::from_fn(|row| std::array::from_fn(|column| columns[column][row]))
}

fn multiply(left: [[f64; 3]; 3], right: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            (0..3)
                .map(|axis| left[row][axis] * right[axis][column])
                .sum()
        })
    })
}

pub(super) fn create(mesh: &mut Mesh, master: u64, request: &Value) -> Result<Value, String> {
    if let Some(link) = &mesh.objects[&master].instance {
        return Err(format!(
            "use master object {} to create an instance",
            link.master
        ));
    }
    let mut duplicate = request.clone();
    duplicate["command"] = json!("object_duplicate");
    let result = objects::edit(mesh, "object_duplicate", &duplicate)?;
    let id = result["object"].as_u64().unwrap();
    let vertices = serde_json::from_value(result["vertex_remap"].clone()).unwrap();
    let faces = serde_json::from_value(result["face_remap"].clone()).unwrap();
    let object = mesh.objects.get_mut(&id).unwrap();
    object.instance = Some(MeshInstance {
        master,
        anchor: object.origin,
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        vertices,
        faces,
    });
    Ok(json!({"object":id,"master_object":master,"linked":true}))
}

fn canonical_face(vertices: &[u64]) -> Vec<u64> {
    (0..vertices.len())
        .map(|offset| {
            vertices[offset..]
                .iter()
                .chain(&vertices[..offset])
                .copied()
                .collect::<Vec<_>>()
        })
        .min()
        .unwrap_or_default()
}

pub(super) fn link_existing(mesh: &mut Mesh, id: u64, request: &Value) -> Result<Value, String> {
    let master = request["master"].as_u64().ok_or("master is required")?;
    if master == id {
        return Err("an object cannot instance itself".into());
    }
    let source = mesh.objects.get(&master).ok_or("unknown master")?.clone();
    let target = mesh.objects[&id].clone();
    if source.instance.is_some() || target.instance.is_some() {
        return Err(
            "source and target must be independent objects; use object_make_unique first".into(),
        );
    }
    if mesh.objects.values().any(|object| {
        object
            .instance
            .as_ref()
            .is_some_and(|link| link.master == id)
    }) {
        return Err("target is already a master; detach its followers before linking it".into());
    }
    if source.vertices.len() != target.vertices.len() || source.smooth_angle != target.smooth_angle
    {
        return Err("objects must have matching vertex counts and smoothing".into());
    }
    let angles = request
        .get("rotation")
        .map(vector)
        .transpose()?
        .unwrap_or([0.0; 3]);
    let rotation = rotation_matrix(angles);
    let center = |vertices: &BTreeSet<u64>| -> [f64; 3] {
        std::array::from_fn(|axis| {
            vertices
                .iter()
                .map(|vertex| mesh.vertices[vertex][axis] / vertices.len() as f64)
                .sum()
        })
    };
    let source_center = center(&source.vertices);
    let target_center = center(&target.vertices);
    let local = std::array::from_fn(|axis| source_center[axis] - source.origin[axis]);
    let offset = rotated(&rotation, local);
    let anchor = std::array::from_fn(|axis| target_center[axis] - offset[axis]);
    let position = |vertex: &u64| -> [f64; 3] {
        let local = std::array::from_fn(|axis| mesh.vertices[vertex][axis] - source.origin[axis]);
        let local = rotated(&rotation, local);
        std::array::from_fn(|axis| anchor[axis] + local[axis])
    };
    let mut vertices: BTreeMap<u64, u64> = match request.get("vertex_map") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "vertex_map must map master vertex IDs to target vertex IDs")?,
        None => BTreeMap::new(),
    };
    if request.get("vertex_map").is_none() {
        let mut available = target.vertices.clone();
        for vertex in &source.vertices {
            let expected = position(vertex);
            let matches: Vec<_> = available
                .iter()
                .filter(|id| {
                    (0..3).all(|axis| (mesh.vertices[id][axis] - expected[axis]).abs() <= 1e-7)
                })
                .copied()
                .collect();
            if matches.len() != 1 {
                return Err("geometry differs or vertex matching is ambiguous; provide an explicit vertex_map for coincident vertices".into());
            }
            vertices.insert(*vertex, matches[0]);
            available.remove(&matches[0]);
        }
    }
    let source_ids: BTreeSet<_> = vertices.keys().copied().collect();
    let target_ids: BTreeSet<_> = vertices.values().copied().collect();
    if source_ids != source.vertices
        || target_ids != target.vertices
        || vertices.len() != target_ids.len()
    {
        return Err("vertex_map must bijectively cover both objects".into());
    }
    let mut maximum_error = 0.0_f64;
    for (source, target) in &vertices {
        if mesh.vertex_colors.get(source) != mesh.vertex_colors.get(target) {
            return Err("objects must have identical vertex colors before linking".into());
        }
        let expected = position(source);
        for (axis, value) in expected.iter().enumerate() {
            maximum_error = maximum_error.max((mesh.vertices[target][axis] - value).abs());
        }
    }
    if maximum_error > 1e-7 {
        return Err("objects differ in shape; linking would change geometry".into());
    }
    let target_faces: BTreeMap<_, _> = mesh
        .faces
        .iter()
        .filter(|(_, face)| target.vertices.contains(&face[0]))
        .map(|(id, face)| (canonical_face(face), *id))
        .collect();
    let mut faces = BTreeMap::new();
    for (face, points) in &mesh.faces {
        if !source.vertices.contains(&points[0]) {
            continue;
        }
        let points: Vec<_> = points.iter().map(|vertex| vertices[vertex]).collect();
        let target_face = target_faces
            .get(&canonical_face(&points))
            .ok_or("objects must have identical topology and face winding")?;
        if mesh.flat_faces.contains(face) != mesh.flat_faces.contains(target_face) {
            return Err("objects must have identical face shading".into());
        }
        let expected = mesh.uvs.get(face).map(|uvs| {
            uvs.iter()
                .map(|(vertex, uv)| (vertices[vertex], *uv))
                .collect::<BTreeMap<_, _>>()
        });
        if mesh.uvs.get(target_face) != expected.as_ref()
            || mesh.face_textures.get(target_face) != mesh.face_textures.get(face)
        {
            return Err(
                "objects must have identical UVs and texture assignments before linking".into(),
            );
        }
        faces.insert(*face, *target_face);
    }
    if faces.len() != target_faces.len() {
        return Err("objects must have identical face counts".into());
    }
    mesh.objects.get_mut(&id).unwrap().instance = Some(MeshInstance {
        master,
        anchor,
        rotation,
        vertices,
        faces,
    });
    Ok(
        json!({"object":id,"master_object":master,"linked":true,"preserved_ids":true,"maximum_position_error":maximum_error}),
    )
}

pub(super) fn placement(
    mesh: &mut Mesh,
    id: u64,
    command: &str,
    request: &Value,
) -> Result<(), String> {
    let origin = mesh.objects[&id].origin;
    let Some(link) = mesh.objects.get_mut(&id).unwrap().instance.as_mut() else {
        return Ok(());
    };
    match command {
        "object_move" => {
            let delta = vector(&request["value"])?;
            for (axis, value) in delta.iter().enumerate() {
                link.anchor[axis] += value;
            }
        }
        "object_rotate" => {
            let angles = vector(&request["value"])?;
            let local = std::array::from_fn(|axis| link.anchor[axis] - origin[axis]);
            let local = rotate(local, angles);
            link.anchor = std::array::from_fn(|axis| origin[axis] + local[axis]);
            let rotation = rotation_matrix(angles);
            link.rotation = multiply(rotation, link.rotation);
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn finish(before: &Mesh, mesh: &mut Mesh, request: &Value) -> Result<(), String> {
    let command = request["command"].as_str().unwrap_or("");
    if command == "cube" {
        return Ok(());
    }
    if command != "load" {
        for (id, object) in &before.objects {
            if object.instance.is_none() || !mesh.objects.contains_key(id) {
                continue;
            }
            if command == "object_make_unique" && request["object"].as_u64() == Some(*id) {
                continue;
            }
            let targeted = request["object"].as_u64() == Some(*id)
                || request["collection"].as_u64().is_some_and(|collection| {
                    before
                        .collections
                        .get(&collection)
                        .is_some_and(|group| group.objects.contains(id))
                });
            let placement = targeted && matches!(command, "object_move" | "object_rotate");
            let unchanged_vertices = object
                .vertices
                .iter()
                .all(|vertex| before.vertices.get(vertex) == mesh.vertices.get(vertex));
            let unchanged_colors = object
                .vertices
                .iter()
                .all(|vertex| before.vertex_colors.get(vertex) == mesh.vertex_colors.get(vertex));
            let unchanged_faces = before
                .faces
                .iter()
                .filter(|(_, face)| object.vertices.contains(&face[0]))
                .all(|(id, face)| {
                    mesh.faces.get(id) == Some(face)
                        && before.flat_faces.contains(id) == mesh.flat_faces.contains(id)
                        && before.uvs.get(id) == mesh.uvs.get(id)
                        && before.face_textures.get(id) == mesh.face_textures.get(id)
                });
            let after = &mesh.objects[id];
            let unchanged_shape = unchanged_vertices
                && unchanged_colors
                && unchanged_faces
                && object.vertices == after.vertices
                && object.smooth_angle == after.smooth_angle;
            if !placement && !unchanged_shape {
                return Err(format!(
                    "object {id} is an instance; edit master {} or use object_make_unique first",
                    object.instance.as_ref().unwrap().master
                ));
            }
        }
    }

    let ids: Vec<_> = mesh
        .objects
        .iter()
        .filter(|(_, object)| object.instance.is_some())
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        let mut link = mesh.objects[&id].instance.clone().unwrap();
        let source = mesh
            .objects
            .get(&link.master)
            .ok_or("master has instances; make them unique before deleting the master")?
            .clone();
        if source.instance.is_some() || link.master == id {
            return Err("instance master must be an independent object".into());
        }
        if link
            .anchor
            .iter()
            .chain(link.rotation.iter().flatten())
            .any(|value| !value.is_finite())
        {
            return Err("instance transform must be finite".into());
        }
        for row in 0..3 {
            for other in 0..3 {
                let dot: f64 = (0..3)
                    .map(|axis| link.rotation[row][axis] * link.rotation[other][axis])
                    .sum();
                if (dot - if row == other { 1.0 } else { 0.0 }).abs() > 1e-8 {
                    return Err("instance rotation must be orthonormal".into());
                }
            }
        }
        let rotation = link.rotation;
        let determinant = rotation[0][0]
            * (rotation[1][1] * rotation[2][2] - rotation[1][2] * rotation[2][1])
            - rotation[0][1] * (rotation[1][0] * rotation[2][2] - rotation[1][2] * rotation[2][0])
            + rotation[0][2] * (rotation[1][0] * rotation[2][1] - rotation[1][1] * rotation[2][0]);
        if (determinant - 1.0).abs() > 1e-8 {
            return Err("instance rotation must preserve winding".into());
        }
        let mapped: BTreeSet<_> = link.vertices.values().copied().collect();
        if mapped.len() != link.vertices.len() || mapped != mesh.objects[&id].vertices {
            return Err("invalid instance vertex mapping".into());
        }
        let owned_faces: BTreeSet<_> = mesh
            .faces
            .iter()
            .filter(|(_, face)| mapped.contains(&face[0]))
            .map(|(id, _)| *id)
            .collect();
        let mapped_faces: BTreeSet<_> = link.faces.values().copied().collect();
        if mapped_faces.len() != link.faces.len() || mapped_faces != owned_faces {
            return Err("invalid instance face mapping".into());
        }
        let master_rotated = command == "object_rotate"
            && (request["object"].as_u64() == Some(link.master)
                || request["collection"].as_u64().is_some_and(|collection| {
                    mesh.collections[&collection].objects.contains(&link.master)
                }));
        if master_rotated {
            let angles = vector(&request["value"])?;
            let rotation = rotation_matrix(angles);
            let inverse =
                std::array::from_fn(|row| std::array::from_fn(|column| rotation[column][row]));
            link.rotation = multiply(link.rotation, inverse);
        }
        if command == "object_set_origin" && request["object"].as_u64() == Some(link.master) {
            let previous = before.objects[&link.master].origin;
            let delta = std::array::from_fn(|axis| source.origin[axis] - previous[axis]);
            let delta = rotated(&link.rotation, delta);
            for (axis, value) in delta.iter().enumerate() {
                link.anchor[axis] += value;
            }
        }
        let source_faces: BTreeMap<_, _> = mesh
            .faces
            .iter()
            .filter(|(_, face)| source.vertices.contains(&face[0]))
            .map(|(id, face)| (*id, face.clone()))
            .collect();
        if command == "load"
            && (link.vertices.keys().copied().collect::<BTreeSet<_>>() != source.vertices
                || link.faces.keys().copied().collect::<BTreeSet<_>>()
                    != source_faces.keys().copied().collect())
        {
            return Err("incomplete instance mapping".into());
        }
        let removed: Vec<_> = link
            .vertices
            .keys()
            .filter(|vertex| !source.vertices.contains(vertex))
            .copied()
            .collect();
        for vertex in removed {
            let removed = link.vertices.remove(&vertex).unwrap();
            mesh.vertices.remove(&removed);
            mesh.selection.remove(&removed);
        }
        for vertex in &source.vertices {
            if !link.vertices.contains_key(vertex) {
                let created = mesh.add_vertex([0.0; 3])?;
                link.vertices.insert(*vertex, created);
            }
            let local =
                std::array::from_fn(|axis| mesh.vertices[vertex][axis] - source.origin[axis]);
            let local = rotated(&link.rotation, local);
            let point = std::array::from_fn(|axis| link.anchor[axis] + local[axis]);
            let destination = link.vertices[vertex];
            if command == "load"
                && (0..3).any(|axis| (mesh.vertices[&destination][axis] - point[axis]).abs() > 1e-8)
            {
                return Err("instance geometry does not match its master".into());
            }
            mesh.vertices.insert(destination, point);
            if command == "load"
                && mesh.vertex_colors.get(&destination) != mesh.vertex_colors.get(&vertex)
            {
                return Err("instance vertex colors do not match its master".into());
            }
            if let Some(color) = mesh.vertex_colors.get(&vertex).copied() {
                mesh.vertex_colors.insert(destination, color);
            } else {
                mesh.vertex_colors.remove(&destination);
            }
        }
        let removed: Vec<_> = link
            .faces
            .keys()
            .filter(|face| !source_faces.contains_key(face))
            .copied()
            .collect();
        for face in removed {
            let removed = link.faces.remove(&face).unwrap();
            mesh.faces.remove(&removed);
            mesh.flat_faces.remove(&removed);
        }
        for (face, vertices) in source_faces {
            let points: Vec<_> = vertices
                .iter()
                .map(|vertex| link.vertices[vertex])
                .collect();
            let destination = match link.faces.get(&face) {
                Some(id) => *id,
                None => {
                    let created = mesh.add_face(points.clone())?;
                    link.faces.insert(face, created);
                    created
                }
            };
            if command == "load"
                && (mesh.faces[&destination] != points
                    || mesh.flat_faces.contains(&destination) != mesh.flat_faces.contains(&face))
            {
                return Err("instance topology or shading does not match its master".into());
            }
            mesh.faces.insert(destination, points);
            if command == "load" {
                let expected = mesh.uvs.get(&face).map(|values| {
                    values
                        .iter()
                        .map(|(vertex, uv)| (link.vertices[vertex], *uv))
                        .collect::<BTreeMap<_, _>>()
                });
                if mesh.uvs.get(&destination) != expected.as_ref()
                    || mesh.face_textures.get(&destination) != mesh.face_textures.get(&face)
                {
                    return Err("instance UVs or textures do not match its master".into());
                }
            } else {
                crate::uv::copy_face(mesh, face, destination, &link.vertices);
            }
            if mesh.flat_faces.contains(&face) {
                mesh.flat_faces.insert(destination);
            } else {
                mesh.flat_faces.remove(&destination);
            }
        }
        let object = mesh.objects.get_mut(&id).unwrap();
        if command == "load" && object.smooth_angle != source.smooth_angle {
            return Err("instance smoothing does not match its master".into());
        }
        object.smooth_angle = source.smooth_angle;
        object.vertices = link.vertices.values().copied().collect();
        object.instance = Some(link);
    }
    Ok(())
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let id = request["object"].as_u64().ok_or("object is required")?;
    let object = mesh.objects.get(&id).ok_or("unknown object")?;
    let link = object
        .instance
        .as_ref()
        .ok_or("object is not an instance")?;
    let mut values = match request["kind"].as_str().unwrap_or("vertices") {
        "vertices" => json!(link.vertices),
        "faces" => json!(link.faces),
        _ => return Err("kind must be vertices or faces".into()),
    };
    if let Some(ids) = request.get("ids") {
        let ids: BTreeSet<u64> = serde_json::from_value(ids.clone())
            .map_err(|_| "ids must be instance vertex or face IDs")?;
        values
            .as_object_mut()
            .unwrap()
            .retain(|_, value| ids.contains(&value.as_u64().unwrap()));
    }
    let mut result = crate::responses::slice(&values, request)?;
    result["revision"] = json!(revision);
    result["object"] = json!(id);
    result["master_object"] = json!(link.master);
    result["mapping"] = json!("master ID to instance ID");
    Ok(result)
}
