use crate::Mesh;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn region(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let mut faces = read_ids(request.get("faces"))?;
    let vertices = read_ids(request.get("vertices"))?;
    if faces.is_empty() && vertices.is_empty() {
        return Err("inspect_region requires face or vertex IDs".into());
    }
    if faces.iter().any(|id| !mesh.faces.contains_key(id)) {
        return Err("unknown region face".into());
    }
    if vertices.iter().any(|id| !mesh.vertices.contains_key(id)) {
        return Err("unknown region vertex".into());
    }

    for (id, face) in &mesh.faces {
        if face.iter().any(|id| vertices.contains(id)) {
            faces.insert(*id);
        }
    }
    let selected_faces: BTreeMap<_, _> = faces
        .iter()
        .map(|id| (*id, mesh.faces[id].clone()))
        .collect();
    let mut core_vertices = vertices;
    for face in selected_faces.values() {
        core_vertices.extend(face);
    }
    let mut neighbor_faces = BTreeMap::new();
    let mut adjacency: BTreeMap<u64, BTreeSet<u64>> = core_vertices
        .iter()
        .map(|id| (*id, BTreeSet::new()))
        .collect();
    let mut edge_incidence = BTreeMap::<(u64, u64), Vec<u64>>::new();
    let mut region_edges = BTreeMap::<(u64, u64), usize>::new();
    for (id, face) in &mesh.faces {
        let touches_region = face.iter().any(|id| core_vertices.contains(id));
        if touches_region && !faces.contains(id) {
            neighbor_faces.insert(*id, face.clone());
        }

        for i in 0..face.len() {
            let a = face[i];
            let b = face[(i + 1) % face.len()];
            let edge = (a.min(b), a.max(b));
            if faces.contains(id) {
                *region_edges.entry(edge).or_default() += 1;
            }
            edge_incidence.entry(edge).or_default().push(*id);
            if let Some(neighbors) = adjacency.get_mut(&a) {
                neighbors.insert(b);
            }
            if let Some(neighbors) = adjacency.get_mut(&b) {
                neighbors.insert(a);
            }
        }
    }
    let mut context_ids = core_vertices.clone();
    for face in neighbor_faces.values() {
        context_ids.extend(face);
    }
    let positions: BTreeMap<_, _> = core_vertices
        .iter()
        .map(|id| (*id, mesh.vertices[id]))
        .collect();
    let context: BTreeMap<_, _> = context_ids
        .iter()
        .map(|id| (*id, mesh.vertices[id]))
        .collect();
    let minimum: [f64; 3] = std::array::from_fn(|axis| {
        positions
            .values()
            .map(|point| point[axis])
            .fold(f64::INFINITY, f64::min)
    });
    let maximum: [f64; 3] = std::array::from_fn(|axis| {
        positions
            .values()
            .map(|point| point[axis])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    let boundary: Vec<_> = region_edges
        .iter()
        .filter(|(_, count)| **count == 1)
        .map(|((a, b), _)| [*a, *b])
        .collect();
    let incidence: Vec<_> = region_edges
        .keys()
        .map(|edge| json!({"edge":[edge.0,edge.1],"faces":edge_incidence[edge]}))
        .collect();
    let mut region_adjacency: BTreeMap<u64, BTreeSet<u64>> = core_vertices
        .iter().map(|id| (*id, BTreeSet::new())).collect();
    for &(a, b) in region_edges.keys() {
        region_adjacency.get_mut(&a).unwrap().insert(b);
        region_adjacency.get_mut(&b).unwrap().insert(a);
    }
    let components = crate::diagnostics::connected_components(&region_adjacency);
    Ok(
        json!({"revision":revision,"faces":selected_faces,"vertices":positions,"neighbor_faces":neighbor_faces,"context_vertices":context,"adjacency":adjacency,"bounds":{"min":minimum,"max":maximum},"boundary_edges":boundary,"edge_incidence":incidence,"topology":{"disconnected_components":components.len()}}),
    )
}

fn read_ids(value: Option<&Value>) -> Result<BTreeSet<u64>, String> {
    match value {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "region IDs must be arrays of unsigned integers".into()),
        None => Ok(BTreeSet::new()),
    }
}
