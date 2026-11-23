use crate::{Mesh, geometry, normals, objects, uv};
use manifold_rust::{
    manifold::Manifold,
    types::{Error, MeshGL64, OpType},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

struct SourceTriangle {
    face: u64,
    vertices: [u64; 3],
}

fn operand(mesh: &Mesh, id: u64, sources: &mut Vec<SourceTriangle>) -> Result<Manifold, String> {
    let object = mesh
        .objects
        .get(&id)
        .ok_or_else(|| format!("unknown boolean operand {id}"))?;
    let mut input = MeshGL64 {
        num_prop: 3,
        ..Default::default()
    };
    let mut indices = BTreeMap::new();
    for vertex in &object.vertices {
        indices.insert(*vertex, indices.len() as u64);
        input.vert_properties.extend(mesh.vertices[vertex]);
    }
    for (&face_id, face) in &mesh.faces {
        if !object.vertices.contains(&face[0]) {
            continue;
        }
        for corners in normals::triangles(mesh, face) {
            let vertices = corners.map(|i| face[i]);
            input.tri_verts.extend(vertices.map(|id| indices[&id]));
            input.face_id.push(sources.len() as u64);
            sources.push(SourceTriangle {
                face: face_id,
                vertices,
            });
        }
    }
    let solid = Manifold::from_mesh_gl64(&input);
    if solid.status() != Error::NoError || solid.volume() <= 0.0 {
        return Err(format!(
            "operand {id} needs a closed, outward-wound manifold with positive volume ({:?})",
            solid.status()
        ));
    }
    Ok(solid)
}

fn interpolate_uv(mesh: &Mesh, source: &SourceTriangle, position: [f64; 3]) -> [f64; 2] {
    let [a, b, c] = source.vertices.map(|id| mesh.vertices[&id]);
    let ab = geometry::sub(b, a);
    let ac = geometry::sub(c, a);
    let ap = geometry::sub(position, a);
    let n = geometry::cross(ab, ac);
    let denominator = geometry::dot(n, n);
    let u = geometry::dot(geometry::cross(ap, ac), n) / denominator;
    let v = geometry::dot(geometry::cross(ab, ap), n) / denominator;
    let [a, b, c] = source.vertices.map(|id| mesh.uvs[&source.face][&id]);
    std::array::from_fn(|axis| a[axis] + u * (b[axis] - a[axis]) + v * (c[axis] - a[axis]))
}

pub(super) fn generate(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let base = request["base"]
        .as_u64()
        .ok_or("base must be an object ID")?;
    let tools: Vec<u64> = serde_json::from_value(request["tools"].clone())
        .map_err(|_| "tools must be an array of object IDs")?;
    let operation = match request["operation"].as_str() {
        Some("union") => OpType::Add,
        Some("difference") => OpType::Subtract,
        Some("intersection") => OpType::Intersect,
        _ => return Err("operation must be union, difference or intersection".into()),
    };
    if tools.is_empty() || tools.len() > 64 {
        return Err("solid_boolean requires 1..64 tool objects".into());
    }
    let inputs: BTreeSet<_> = std::iter::once(base).chain(tools.iter().copied()).collect();
    if inputs.len() != tools.len() + 1
        || request["object"]
            .as_u64()
            .is_some_and(|id| inputs.contains(&id))
    {
        return Err("base, tools and destination must be distinct objects".into());
    }

    let mut sources = Vec::new();
    let mut solid = operand(mesh, base, &mut sources)?;
    for &id in &tools {
        let tool = operand(mesh, id, &mut sources)?;
        solid = solid.boolean(&tool, operation);
        if solid.status() != Error::NoError || solid.volume() <= 0.0 {
            return Err(format!(
                "boolean with tool {id} produced an empty or invalid solid ({:?})",
                solid.status()
            ));
        }
    }

    let result = solid.get_mesh_gl64(-1);
    let object = objects::destination_object(mesh, request)?;
    let mut vertices = Vec::with_capacity(result.num_vert());
    // MeshGL can duplicate a vertex across property seams. Restore shared topology.
    let merges: BTreeMap<_, _> = result
        .merge_from_vert
        .iter()
        .copied()
        .zip(result.merge_to_vert.iter().copied())
        .collect();
    let mut canonical = BTreeMap::new();
    for index in 0..result.num_vert() {
        let root = merges.get(&(index as u64)).copied().unwrap_or(index as u64);
        let vertex = match canonical.get(&root) {
            Some(&id) => id,
            None => {
                let id = mesh.add_vertex(result.get_vert_pos(root as usize))?;
                canonical.insert(root, id);
                id
            }
        };
        vertices.push(vertex);
    }
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(canonical.values());
    for i in 0..result.num_tri() {
        let corners = result.get_tri_verts(i).map(|v| vertices[v as usize]);
        let source = result
            .face_id
            .get(i)
            .and_then(|id| sources.get(*id as usize))
            .ok_or("missing boolean face provenance")?;
        let face = mesh.add_face(corners.to_vec())?;
        if uv::complete(mesh, source.face) {
            let uvs = corners.map(|id| (id, interpolate_uv(mesh, source, mesh.vertices[&id])));
            mesh.uvs.insert(face, uvs.into_iter().collect());
        }
        if let Some(texture) = mesh.face_textures.get(&source.face).cloned() {
            mesh.face_textures.insert(face, texture);
        }
        if mesh.flat_faces.contains(&source.face) {
            mesh.flat_faces.insert(face);
        }
    }
    Ok(
        json!({"object":object,"base":base,"tools":tools,"volume":solid.volume(),"source_objects_preserved":true}),
    )
}
