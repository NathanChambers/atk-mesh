use crate::{Mesh, cube, vector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectStage {
    #[default]
    Blockout,
    Refined,
    Detailed,
    ProductionReady,
}

impl ObjectStage {
    fn is_blockout(&self) -> bool {
        *self == Self::Blockout
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshObject {
    pub name: String,
    pub origin: [f64; 3],
    pub vertices: BTreeSet<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth_angle: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<crate::instances::MeshInstance>,
    #[serde(default, skip_serializing_if = "ObjectStage::is_blockout")]
    pub stage: ObjectStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) review: Vec<crate::object_review::Check>,
}

pub(super) fn normalize_legacy_stages(mesh: &mut Mesh) {
    for object in mesh.objects.values_mut() {
        if object.stage == ObjectStage::ProductionReady && !crate::object_review::approved_plan(&object.review) {
            crate::object_review::reopen(object);
        }
    }
}

pub(super) fn refresh_stages(before: &Mesh, mesh: &mut Mesh) {
    let mut approved = mesh
        .objects
        .iter()
        .filter(|(_, object)| object.stage == ObjectStage::Refined || crate::object_review::has_results(object))
        .peekable();
    if approved.peek().is_none() {
        return;
    }
    let approved_vertices: BTreeMap<_, _> = approved
        .flat_map(|(id, object)| object.vertices.iter().map(move |vertex| (*vertex, *id)))
        .collect();

    let before_rig = before.rig.as_ref();
    let after_rig = mesh.rig.as_ref();
    let rig_changed = before_rig.map(|rig| (&rig.bones, &rig.clips))
        != after_rig.map(|rig| (&rig.bones, &rig.clips));
    let workspace_changed = mesh.objects.values().any(|object| {
        object.stage == ObjectStage::Refined || crate::object_review::has_workspace_evidence(object)
    })
        && (before.rig != mesh.rig
            || !crate::workflow::same_geometry(&json!(before), &json!(mesh), None));
    let mut changed = BTreeSet::new();
    for (vertex, id) in &approved_vertices {
        let before_weights = before_rig.and_then(|rig| rig.weights.get(vertex));
        let after_weights = after_rig.and_then(|rig| rig.weights.get(vertex));
        if before.vertices.get(vertex) != mesh.vertices.get(vertex)
            || before.vertex_colors.get(vertex) != mesh.vertex_colors.get(vertex)
            || before_weights != after_weights
            || (rig_changed && after_weights.is_some())
        {
            changed.insert(*id);
        }
    }
    let mut changed_textures = BTreeMap::new();
    for (face, vertices) in &mesh.faces {
        let Some(id) = approved_vertices.get(&vertices[0]) else {
            continue;
        };
        if before.faces.get(face) != Some(vertices)
            || before.flat_faces.contains(face) != mesh.flat_faces.contains(face)
            || before.uvs.get(face) != mesh.uvs.get(face)
            || before.face_textures.get(face) != mesh.face_textures.get(face)
            || mesh.face_textures.get(face).is_some_and(|name| {
                *changed_textures.entry(name).or_insert_with(|| {
                    before.textures.get(name) != mesh.textures.get(name)
                })
            })
        {
            changed.insert(*id);
        }
    }
    for (face, vertices) in &before.faces {
        if before.faces.get(face) != mesh.faces.get(face)
            && let Some(id) = approved_vertices.get(&vertices[0])
        {
            changed.insert(*id);
        }
    }
    for (id, object) in &mut mesh.objects {
        if crate::object_review::has_results(object)
            && before.objects.get(id).is_some_and(|previous| {
                changed.contains(id)
                    || object.vertices != previous.vertices
                    || object.origin != previous.origin
                    || object.smooth_angle != previous.smooth_angle
                    || object.instance.as_ref().map(|link| link.master)
                        != previous.instance.as_ref().map(|link| link.master)
            })
        {
            crate::object_review::reopen(object);
        } else if workspace_changed {
            crate::object_review::reopen_workspace_checks(object);
        }
    }
}

pub(super) fn region(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let Some(value) = request.get("object") else {
        return crate::inspection::region(mesh, request, revision);
    };
    let id = value.as_u64().ok_or("object must be an ID")?;
    let object = mesh.objects.get(&id).ok_or("unknown object")?;
    let mut scoped = request.clone();
    if request.get("faces").is_none() && request.get("vertices").is_none() {
        scoped["vertices"] = json!(object.vertices);
    }
    let mut result = crate::inspection::region(mesh, &scoped, revision)?;
    let vertices = result["vertices"].as_object().ok_or("invalid region")?;
    if vertices.keys().any(|key| {
        key.parse::<u64>()
            .map_or(true, |id| !object.vertices.contains(&id))
    }) {
        return Err("region lies outside the requested object".into());
    }
    result["object"] = json!({"id":id,"name":object.name,"origin":object.origin});
    Ok(result)
}

pub(super) fn initialize(mesh: &mut Mesh) {
    if mesh.objects.is_empty() && !mesh.vertices.is_empty() {
        mesh.objects.insert(
            1,
            MeshObject {
                name: "Mesh".into(),
                origin: [0.0; 3],
                vertices: mesh.vertices.keys().copied().collect(),
                smooth_angle: None,
                instance: None,
                stage: ObjectStage::default(),
                stage_note: None,
                review: Vec::new(),
            },
        );
        mesh.object_id_high_watermark = 1;
    }
}

pub(super) fn reconcile(mesh: &mut Mesh) -> Result<(), String> {
    let mut owners = BTreeMap::new();
    let mut names = BTreeSet::new();
    for (id, object) in &mut mesh.objects {
        if object.name.trim().is_empty()
            || object.name.chars().any(char::is_control)
            || !names.insert(object.name.clone())
        {
            return Err(
                "object names must be nonempty, unique and contain no control characters".into(),
            );
        }
        if object.origin.iter().any(|value| !value.is_finite()) {
            return Err("object origin must be finite".into());
        }
        if object
            .smooth_angle
            .is_some_and(|angle| !angle.is_finite() || !(0.0..=180.0).contains(&angle))
        {
            return Err("smooth_angle must be between 0 and 180 degrees".into());
        }
        object
            .vertices
            .retain(|vertex| mesh.vertices.contains_key(vertex));
        for vertex in &object.vertices {
            if owners.insert(*vertex, *id).is_some() {
                return Err("a vertex cannot belong to multiple objects".into());
            }
        }
    }
    loop {
        let mut changed = false;
        for face in mesh.faces.values() {
            let face_owners: BTreeSet<_> = face
                .iter()
                .filter_map(|vertex| owners.get(vertex).copied())
                .collect();
            if face_owners.len() > 1 {
                return Err("topology cannot connect separate objects".into());
            }
            if let Some(owner) = face_owners.first() {
                for vertex in face {
                    if owners.insert(*vertex, *owner).is_none() {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    if owners.len() != mesh.vertices.len() {
        return Err("every vertex must belong to an object".into());
    }
    for object in mesh.objects.values_mut() {
        object.vertices.clear();
    }
    for (vertex, owner) in owners {
        mesh.objects
            .get_mut(&owner)
            .unwrap()
            .vertices
            .insert(vertex);
    }
    if !mesh.defer_validation
        && mesh
            .objects
            .values()
            .any(|object| object.vertices.is_empty())
    {
        return Err("edit would remove an entire object".into());
    }
    Ok(())
}

pub(super) fn inherit(before: &Mesh, after: &mut Mesh) {
    for (id, object) in &before.objects {
        let mut created: BTreeSet<u64> = BTreeSet::new();
        for (face, vertices) in &before.faces {
            if !object.vertices.contains(&vertices[0]) {
                continue;
            }
            if let Some(vertices) = after.faces.get(face) {
                created.extend(
                    vertices
                        .iter()
                        .filter(|vertex| !before.vertices.contains_key(vertex)),
                );
            }
        }
        if let Some(object) = after.objects.get_mut(id) {
            object.vertices.extend(created);
        }
    }
}

pub(super) fn check_scope(before: &Mesh, after: &Mesh, request: &Value) -> Result<(), String> {
    let Some(value) = request.get("object") else {
        return Ok(());
    };
    let id = value.as_u64().ok_or("object must be an ID")?;
    let object = before.objects.get(&id).ok_or("unknown object")?;
    for (other_id, other) in &before.objects {
        if *other_id != id && after.objects.get(other_id) != Some(other) {
            return Err("edit would change another object's ownership or origin".into());
        }
    }
    for (vertex, point) in &before.vertices {
        if !object.vertices.contains(vertex)
            && before.vertex_colors.get(vertex) != after.vertex_colors.get(vertex)
        {
            return Err("edit would paint vertices outside the requested object".into());
        }
        if !object.vertices.contains(vertex) && after.vertices.get(vertex) != Some(point) {
            return Err("edit would change vertices outside the requested object".into());
        }
    }
    for (face, vertices) in &before.faces {
        let appearance_changed = before.uvs.get(face) != after.uvs.get(face)
            || before.face_textures.get(face) != after.face_textures.get(face);
        if !object.vertices.contains(&vertices[0]) && appearance_changed {
            return Err("edit would change UVs or textures outside the requested object".into());
        }
        if !object.vertices.contains(&vertices[0])
            && before.flat_faces.contains(face) != after.flat_faces.contains(face)
        {
            return Err("edit would change shading outside the requested object".into());
        }
        if !object.vertices.contains(&vertices[0]) && after.faces.get(face) != Some(vertices) {
            return Err("edit would change faces outside the requested object".into());
        }
    }
    let checks_selection = matches!(
        request["command"].as_str(),
        Some("select" | "select_box" | "select_image" | "select_object")
    );
    for vertex in &after.selection {
        if checks_selection && !after.objects[&id].vertices.contains(vertex) {
            return Err("selection contains vertices outside the requested object".into());
        }
    }
    Ok(())
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    if command == "object_create_box" || command == "object_add_box" {
        let min = vector(&request["min"])?;
        let max = vector(&request["max"])?;
        if (0..3).any(|axis| min[axis] >= max[axis]) {
            return Err("box min must be below max".into());
        }
        let origin = match request.get("origin") {
            Some(value) => vector(value)?,
            None => std::array::from_fn(|axis| (min[axis] + max[axis]) * 0.5),
        };
        let mut destination = request.clone();
        destination["origin"] = json!(origin);
        if command == "object_create_box" && request.get("object").is_some() {
            return Err("use object_add_box to append to an object".into());
        }
        if command == "object_add_box" && request.get("object").is_none() {
            return Err("object is required".into());
        }
        let id = destination_object(mesh, &destination)?;
        let source = cube();
        let mut remap = BTreeMap::new();
        for (vertex, point) in source.vertices {
            let position = std::array::from_fn(|axis| {
                if point[axis] < 0.0 {
                    min[axis]
                } else {
                    max[axis]
                }
            });
            remap.insert(vertex, mesh.add_vertex(position)?);
        }
        let mut faces = vec![];
        for face in source.faces.values() {
            faces.push(mesh.add_face(face.iter().map(|vertex| remap[vertex]).collect())?);
        }
        let vertices: BTreeSet<_> = remap.values().copied().collect();
        mesh.selection = vertices.clone();
        mesh.objects
            .get_mut(&id)
            .unwrap()
            .vertices
            .extend(&vertices);
        return Ok(json!({"object":id,"vertices":vertices,"faces":faces}));
    }
    let id = request["object"].as_u64().ok_or("object is required")?;
    let object = mesh.objects.get(&id).ok_or("unknown object")?.clone();
    crate::instances::placement(mesh, id, command, request)?;
    match command {
        "object_scale" => {
            if object.instance.is_none() {
                return Err("object_scale requires a linked instance".into());
            }
        }
        "object_review_plan" | "object_review_check" => {
            return crate::object_review::edit(mesh.objects.get_mut(&id).unwrap(), command, request);
        }
        "object_set_stage" => {
            let stage = serde_json::from_value(request["stage"].clone())
                .map_err(|_| "stage must be blockout, refined, detailed or production_ready")?;
            let note = request.get("note").map(|value| {
                let note = value.as_str().map(str::trim).ok_or("note must be a string")?;
                if note.is_empty() || note.len() > 1024 {
                    return Err("note must contain 1 to 1024 bytes");
                }
                Ok(note.to_owned())
            }).transpose()?;
            if stage == ObjectStage::ProductionReady && note.is_none() {
                return Err("production_ready requires a note describing the intended use and checks".into());
            }
            let object = mesh.objects.get_mut(&id).unwrap();
            object.stage = stage;
            object.stage_note = note;
            return Ok(json!({"object":id,"stage":stage,"stage_note":object.stage_note}));
        }
        "object_instance" => return crate::instances::create(mesh, id, request),
        "object_link_instance" => return crate::instances::link_existing(mesh, id, request),
        "object_make_unique" => {
            mesh.objects.get_mut(&id).unwrap().instance = None;
            return Ok(json!({"object":id,"independent_copy":true}));
        }
        "object_duplicate" => {
            let translation = request.get("value").map(vector).unwrap_or(Ok([0.0; 3]))?;
            let origin: [f64; 3] =
                std::array::from_fn(|axis| object.origin[axis] + translation[axis]);
            let destination =
                destination_object(mesh, &json!({"name":request["name"],"origin":origin}))?;
            let source_faces: Vec<_> = mesh
                .faces
                .iter()
                .filter(|(_, face)| object.vertices.contains(&face[0]))
                .map(|(face, vertices)| (*face, vertices.clone()))
                .collect();
            let mut vertices = BTreeMap::new();
            for vertex in &object.vertices {
                let point = mesh.vertices[vertex];
                let position = std::array::from_fn(|axis| point[axis] + translation[axis]);
                let created = mesh.add_vertex(position)?;
                vertices.insert(*vertex, created);
                if let Some(color) = mesh.vertex_colors.get(vertex).copied() {
                    mesh.vertex_colors.insert(created, color);
                }
            }

            let mut faces = BTreeMap::new();
            for (face, points) in source_faces {
                let created =
                    mesh.add_face(points.iter().map(|vertex| vertices[vertex]).collect())?;
                if mesh.flat_faces.contains(&face) {
                    mesh.flat_faces.insert(created);
                }
                crate::uv::copy_face(mesh, face, created, &vertices);
                faces.insert(face, created);
            }

            let copied = mesh.objects.get_mut(&destination).unwrap();
            copied.vertices = vertices.values().copied().collect();
            copied.smooth_angle = object.smooth_angle;
            mesh.selection = copied.vertices.clone();
            for collection in mesh.collections.values_mut() {
                if collection.objects.contains(&id) {
                    collection.objects.insert(destination);
                }
            }
            if let Some(primary) = mesh.primary_collections.get(&id).copied() {
                mesh.primary_collections.insert(destination, primary);
            }
            return Ok(
                json!({"object":destination,"source_object":id,"vertex_remap":vertices,"face_remap":faces,"translation":translation,"independent_copy":true}),
            );
        }
        "object_delete_components" => {
            let seeds: BTreeSet<u64> = serde_json::from_value(request["vertices"].clone())
                .map_err(|_| "vertices must be an array of component seed IDs")?;
            if seeds.is_empty() {
                return Err("component seeds are empty".into());
            }
            if seeds.iter().any(|vertex| !object.vertices.contains(vertex)) {
                return Err("component seed lies outside the requested object".into());
            }
            let expected = request["expected_vertices"]
                .as_u64()
                .ok_or("expected_vertices is required to guard component deletion")?;
            let mut adjacency = BTreeMap::<u64, Vec<u64>>::new();
            for face in mesh.faces.values() {
                if !object.vertices.contains(&face[0]) {
                    continue;
                }
                for index in 0..face.len() {
                    let first = face[index];
                    let second = face[(index + 1) % face.len()];
                    adjacency.entry(first).or_default().push(second);
                    adjacency.entry(second).or_default().push(first);
                }
            }
            let mut pending: Vec<u64> = seeds.into_iter().collect();
            let mut removed = BTreeSet::new();
            while let Some(vertex) = pending.pop() {
                if !removed.insert(vertex) {
                    continue;
                }
                if let Some(neighbors) = adjacency.get(&vertex) {
                    pending.extend(neighbors);
                }
            }
            if removed.len() as u64 != expected {
                return Err(format!(
                    "component deletion resolved {} vertices; expected {expected}",
                    removed.len()
                ));
            }
            if removed == object.vertices {
                return Err("use object_delete to remove the entire object".into());
            }
            let before_faces = mesh.faces.len();
            mesh.faces.retain(|_, face| !removed.contains(&face[0]));
            mesh.vertices.retain(|vertex, _| !removed.contains(vertex));
            mesh.selection.retain(|vertex| !removed.contains(vertex));
            return Ok(
                json!({"object":id,"removed_vertices":removed.len(),"removed_faces":before_faces - mesh.faces.len()}),
            );
        }
        "object_delete" => {
            mesh.faces
                .retain(|_, face| !face.iter().any(|vertex| object.vertices.contains(vertex)));
            mesh.vertices
                .retain(|vertex, _| !object.vertices.contains(vertex));
            mesh.selection
                .retain(|vertex| !object.vertices.contains(vertex));
            mesh.objects.remove(&id);
            for collection in mesh.collections.values_mut() {
                collection.objects.remove(&id);
            }
            mesh.primary_collections.remove(&id);
        }
        "object_rename" => {
            mesh.objects.get_mut(&id).unwrap().name = request["name"]
                .as_str()
                .ok_or("name is required")?
                .to_string()
        }
        "object_set_origin" => {
            mesh.objects.get_mut(&id).unwrap().origin = vector(&request["origin"])?
        }
        "select_object" => mesh.selection = object.vertices.clone(),
        "object_move" | "object_rotate" => {
            let value = vector(&request["value"])?;
            for vertex in &object.vertices {
                let point = mesh.vertices.get_mut(vertex).unwrap();
                if command == "object_move" {
                    for axis in 0..3 {
                        point[axis] += value[axis];
                    }
                } else {
                    let mut local: [f64; 3] =
                        std::array::from_fn(|axis| point[axis] - object.origin[axis]);
                    for (axis, angle) in value.iter().enumerate() {
                        let a = (axis + 1) % 3;
                        let b = (axis + 2) % 3;
                        let (sin, cos) = angle.to_radians().sin_cos();
                        let first = local[a] * cos - local[b] * sin;
                        local[b] = local[a] * sin + local[b] * cos;
                        local[a] = first;
                    }
                    for axis in 0..3 {
                        point[axis] = object.origin[axis] + local[axis];
                    }
                }
            }
            if command == "object_move" {
                for (axis, amount) in value.iter().enumerate() {
                    mesh.objects.get_mut(&id).unwrap().origin[axis] += amount;
                }
            }
            mesh.selection = object.vertices;
            if command == "object_move" && value != [0.0; 3] {
                let unchanged_instances = mesh.objects.values()
                    .filter(|object| object.instance.as_ref().is_some_and(|link| link.master == id))
                    .count();
                if unchanged_instances > 0 {
                    return Ok(json!({
                        "object":id,
                        "linked_instances_unchanged":unchanged_instances,
                        "shared_shape_edit":{
                            "instruction":"If the goal is to update every linked copy, restore the master to its intended placement first. Then select and move its vertices to change the shared shape.",
                            "select":{"command":"select_object","object":id},
                            "move":{"command":"move","object":id,"value":request["value"]}
                        }
                    }));
                }
            }
        }
        _ => return Err("unknown object command".into()),
    }
    Ok(json!({"object":id}))
}

pub(super) fn destination_object(mesh: &mut Mesh, request: &Value) -> Result<u64, String> {
    if let Some(value) = request.get("object") {
        let id = value.as_u64().ok_or("object must be an ID")?;
        if !mesh.objects.contains_key(&id) {
            return Err("unknown object".into());
        }
        return Ok(id);
    }
    let id = mesh
        .object_id_high_watermark
        .max(mesh.objects.keys().next_back().copied().unwrap_or(0))
        .checked_add(1)
        .ok_or("object IDs exhausted")?;
    let name = request["name"]
        .as_str()
        .ok_or("name is required")?
        .to_string();
    let origin = match request.get("origin") {
        Some(value) => vector(value)?,
        None => [0.0; 3],
    };
    mesh.objects.insert(
        id,
        MeshObject {
            name,
            origin,
            vertices: BTreeSet::new(),
            smooth_angle: None,
            instance: None,
            stage: ObjectStage::default(),
            stage_note: None,
            review: Vec::new(),
        },
    );
    mesh.object_id_high_watermark = id;
    Ok(id)
}
