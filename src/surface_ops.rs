use crate::{Mesh, normals, topology};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn edges(
    mesh: &Mesh,
    object: Option<u64>,
) -> BTreeMap<(u64, u64), Vec<(u64, u64, u64)>> {
    let mut edges = BTreeMap::<_, Vec<_>>::new();
    for (id, face) in &mesh.faces {
        if object.is_some_and(|id| !mesh.objects[&id].vertices.contains(&face[0])) {
            continue;
        }
        for i in 0..face.len() {
            let a = face[i];
            let b = face[(i + 1) % face.len()];
            edges
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((*id, a, b));
        }
    }
    edges
}

pub(super) fn boundaries(mesh: &Mesh, object: Option<u64>) -> (Vec<Vec<u64>>, Vec<u64>) {
    let edges = edges(mesh, object);
    let mut adjacency = BTreeMap::<u64, Vec<u64>>::new();
    let mut remaining = BTreeSet::new();
    for ((a, b), incident) in edges {
        if incident.len() == 1 {
            adjacency.entry(a).or_default().push(b);
            adjacency.entry(b).or_default().push(a);
            remaining.insert((a, b));
        }
    }
    let branching: Vec<_> = adjacency
        .iter()
        .filter(|(_, neighbors)| neighbors.len() != 2)
        .map(|(id, _)| *id)
        .collect();
    let mut loops = vec![];
    while let Some((a, b)) = remaining.first().copied() {
        remaining.remove(&(a, b));
        let mut ring = vec![a, b];
        let mut previous = a;
        let mut current = b;
        while current != a {
            let neighbors = &adjacency[&current];
            if neighbors.len() != 2 {
                break;
            }
            let next = if neighbors[0] == previous {
                neighbors[1]
            } else {
                neighbors[0]
            };
            if !remaining.remove(&(current.min(next), current.max(next))) {
                break;
            }
            previous = current;
            current = next;
            if current != a {
                ring.push(current);
            }
        }
        if current == a {
            loops.push(ring);
        }
    }
    (loops, branching)
}

pub(super) fn object(mesh: &Mesh, request: &Value) -> Result<u64, String> {
    let id = request["object"].as_u64().ok_or("object is required")?;
    if !mesh.objects.contains_key(&id) {
        return Err("unknown object".into());
    }
    Ok(id)
}

pub(super) fn query_boundaries(
    mesh: &Mesh,
    request: &Value,
    revision: u64,
) -> Result<Value, String> {
    let id = object(mesh, request)?;
    let (loops, branching) = boundaries(mesh, Some(id));
    let items: Vec<_> = loops
        .iter()
        .enumerate()
        .map(|(index, vertices)| json!({"index":index,"vertices":vertices,"closed":true}))
        .collect();
    let mut result = crate::responses::slice(&json!(items), request)?;
    result["revision"] = json!(revision);
    result["object"] = json!(id);
    result["branching_vertices"] = json!(branching);
    Ok(result)
}

fn read_loop(mesh: &Mesh, id: u64, value: &Value) -> Result<Vec<u64>, String> {
    let vertices: Vec<u64> = serde_json::from_value(value.clone())
        .map_err(|_| "loop must be an ordered array of vertex IDs")?;
    let unique: BTreeSet<_> = vertices.iter().copied().collect();
    if vertices.len() < 3
        || unique.len() != vertices.len()
        || vertices
            .iter()
            .any(|v| !mesh.objects[&id].vertices.contains(v))
    {
        return Err("loop requires at least three distinct vertices within object".into());
    }
    let incidence = edges(mesh, Some(id));
    for i in 0..vertices.len() {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        if incidence
            .get(&(a.min(b), a.max(b)))
            .is_none_or(|incident| incident.len() != 1)
        {
            return Err("loop edges must each have exactly one incident face".into());
        }
    }
    Ok(vertices)
}

fn orient(mesh: &Mesh, ring: &mut [u64], opposite: bool) -> Result<(), String> {
    let edges = edges(mesh, None);
    let a = ring[0];
    let b = ring[1];
    let same = edges[&(a.min(b), a.max(b))][0].1 == a;
    if same == opposite {
        ring.reverse();
    }
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        let same = edges[&(a.min(b), a.max(b))][0].1 == a;
        if same == opposite {
            return Err("boundary winding is inconsistent".into());
        }
    }
    Ok(())
}

fn planar(mesh: &Mesh, ring: &[u64]) -> Result<(), String> {
    let normal = normals::face(mesh, ring);
    if normal == [0.0; 3] {
        return Err("boundary is degenerate".into());
    }
    let origin = mesh.vertices[&ring[0]];
    let drop = (0..3)
        .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
        .unwrap();
    let scale = ring
        .iter()
        .flat_map(|id| (0..3).map(move |a| (mesh.vertices[id][a] - origin[a]).abs()))
        .fold(0.0, f64::max);
    let mut points = vec![];
    for id in ring {
        let point = mesh.vertices[id];
        let distance: f64 = (0..3).map(|a| (point[a] - origin[a]) * normal[a]).sum();
        if distance.abs() > scale * 1e-8 {
            return Err("boundary must be planar".into());
        }
        points.push([point[(drop + 1) % 3], point[(drop + 2) % 3]]);
    }
    crate::shaping::polygon_triangles(&points)?;
    Ok(())
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    let object = object(mesh, request)?;
    if command == "dissolve_edges" {
        let requested: Vec<[u64; 2]> = serde_json::from_value(request["edges"].clone())
            .map_err(|_| "edges must contain vertex pairs")?;
        if requested.is_empty() {
            return Err("edges must not be empty".into());
        }
        let mut operations = vec![];
        for [a, b] in requested {
            let incidence = edges(mesh, Some(object));
            let owners = incidence
                .get(&(a.min(b), a.max(b)))
                .filter(|v| v.len() == 2)
                .ok_or("dissolve requires internal two-face edges")?;
            let faces: Vec<_> = owners.iter().map(|item| item.0).collect();
            let mut scoped = request.clone();
            scoped["faces"] = json!(faces);
            operations.push(dissolve(mesh, object, "dissolve_region", &scoped)?);
        }
        topology::validate_topology(mesh)?;
        return Ok(json!({"object":object,"operations":operations}));
    }
    let result = match command {
        "create_face" => {
            let mut ring = read_loop(mesh, object, &request["vertices"])?;
            orient(mesh, &mut ring, true)?;
            planar(mesh, &ring)?;
            let face = mesh.add_face(ring.clone())?;
            mesh.selection = ring.iter().copied().collect();
            json!({"object":object,"created_faces":[face],"boundary":ring,"appearance":"new face has no UV or texture binding; assign explicitly"})
        }
        "bridge_loops" => {
            let loops = request["loops"]
                .as_array()
                .filter(|v| v.len() == 2)
                .ok_or("loops must contain two ordered boundary loops")?;
            let mut a = read_loop(mesh, object, &loops[0])?;
            let mut b = read_loop(mesh, object, &loops[1])?;
            let count = a.len();
            if count != b.len() || a.iter().any(|id| b.contains(id)) {
                return Err("bridge loops must be disjoint and have equal vertex counts".into());
            }
            orient(mesh, &mut a, true)?;
            orient(mesh, &mut b, false)?;
            if request["reverse"].as_bool().unwrap_or(false) {
                b.reverse();
            }
            let offsets: Vec<usize> = if let Some(value) = request.get("offset") {
                vec![
                    value
                        .as_u64()
                        .filter(|v| *v < count as u64)
                        .ok_or("offset must be smaller than loop length")?
                        as usize,
                ]
            } else {
                (0..count).collect()
            };
            let mut best = None;
            let mut cost = f64::INFINITY;
            for offset in offsets {
                let aligned: Vec<_> = (0..count).map(|i| b[(i + offset) % count]).collect();
                let quads: Vec<_> = (0..count)
                    .map(|i| {
                        let j = (i + 1) % count;
                        vec![a[i], a[j], aligned[j], aligned[i]]
                    })
                    .collect();
                let score: f64 = a
                    .iter()
                    .zip(&aligned)
                    .map(|(a, b)| {
                        (0..3)
                            .map(|axis| (mesh.vertices[a][axis] - mesh.vertices[b][axis]).powi(2))
                            .sum::<f64>()
                    })
                    .sum();
                if score < cost && quads.iter().all(|q| topology::convex_face(mesh, q).is_ok()) {
                    cost = score;
                    best = Some((offset, aligned, quads));
                }
            }
            let (offset,aligned,quads)=best.ok_or("no planar convex quad bridge correspondence; choose another offset or intermediate loops")?;
            let mut created = vec![];
            for quad in quads {
                created.push(mesh.add_face(quad)?);
            }
            mesh.selection = a.iter().chain(&b).copied().collect();
            let pairs: Vec<_> = a.iter().zip(aligned).map(|(a, b)| [*a, b]).collect();
            json!({"object":object,"created_faces":created,"vertex_pairs":pairs,"offset":offset,"self_intersections":"not_checked"})
        }
        "dissolve_region" | "dissolve_edges" => dissolve(mesh, object, command, request)?,
        _ => return Err("unknown surface operation".into()),
    };
    topology::validate_topology(mesh)?;
    Ok(result)
}

fn dissolve(mesh: &mut Mesh, object: u64, command: &str, request: &Value) -> Result<Value, String> {
    let mut faces: BTreeSet<u64> = if command == "dissolve_region" {
        serde_json::from_value(request["faces"].clone())
            .map_err(|_| "faces must be an array of IDs")?
    } else {
        BTreeSet::new()
    };
    if command == "dissolve_edges" {
        let requested: Vec<[u64; 2]> = serde_json::from_value(request["edges"].clone())
            .map_err(|_| "edges must contain vertex pairs")?;
        let incidence = edges(mesh, Some(object));
        if requested.is_empty() {
            return Err("edges must not be empty".into());
        }
        for [a, b] in requested {
            let owners = incidence
                .get(&(a.min(b), a.max(b)))
                .filter(|v| v.len() == 2)
                .ok_or("dissolve requires internal two-face edges")?;
            faces.extend(owners.iter().map(|item| item.0));
        }
    }
    if faces.len() < 2
        || faces.iter().any(|id| {
            mesh.faces
                .get(id)
                .is_none_or(|face| !mesh.objects[&object].vertices.contains(&face[0]))
        })
    {
        return Err("provide at least two faces within the object".into());
    }
    let region = crate::regions::planar_region(mesh, &faces, request)?;
    let keep = *faces.first().unwrap();
    let texture = mesh.face_textures.get(&keep).cloned();
    let flat = mesh.flat_faces.contains(&keep);
    let has_uv = mesh.uvs.contains_key(&keep);
    let mut uv = BTreeMap::new();
    for face in &faces {
        if mesh.face_textures.get(face) != texture.as_ref()
            || mesh.flat_faces.contains(face) != flat
            || mesh.uvs.contains_key(face) != has_uv
        {
            return Err("dissolve would merge different texture, UV or shading assignments".into());
        }
        if let Some(values) = mesh.uvs.get(face) {
            for (id, value) in values {
                if uv
                    .insert(*id, *value)
                    .is_some_and(|previous| previous != *value)
                {
                    return Err("dissolve would remove a UV seam".into());
                }
            }
        }
    }
    let old_vertices = crate::construction::vertices(mesh, &faces);
    if has_uv {
        if old_vertices.iter().any(|id| !uv.contains_key(id)) {
            return Err(
                "dissolve requires complete affine UVs; clear or repair the mapping first".into(),
            );
        }
        let ids: Vec<_> = old_vertices.iter().copied().collect();
        let p0 = mesh.vertices[&ids[0]];
        let u0 = uv[&ids[0]];
        let mut basis = None;
        for i in 1..ids.len() {
            for j in i + 1..ids.len() {
                let e1 = crate::geometry::sub(mesh.vertices[&ids[i]], p0);
                let e2 = crate::geometry::sub(mesh.vertices[&ids[j]], p0);
                let a = crate::geometry::dot(e1, e1);
                let b = crate::geometry::dot(e1, e2);
                let c = crate::geometry::dot(e2, e2);
                let d = a * c - b * b;
                if d > 1e-20 * a * c {
                    basis = Some((ids[i], ids[j], e1, e2, a, b, c, d));
                    break;
                }
            }
            if basis.is_some() {
                break;
            }
        }
        let (i, j, e1, e2, a, b, c, d) = basis.ok_or("dissolve UV basis is degenerate")?;
        for id in &ids {
            let p = crate::geometry::sub(mesh.vertices[id], p0);
            let x = crate::geometry::dot(p, e1);
            let y = crate::geometry::dot(p, e2);
            let s = (c * x - b * y) / d;
            let t = (a * y - b * x) / d;
            for axis in 0..2 {
                let expected =
                    u0[axis] + s * (uv[&i][axis] - u0[axis]) + t * (uv[&j][axis] - u0[axis]);
                if (expected - uv[id][axis]).abs() > 1e-8 {
                    return Err("dissolve would change non-affine UV interpolation".into());
                }
            }
        }
    }
    for id in &faces {
        if *id != keep {
            mesh.faces.remove(id);
        }
    }
    mesh.faces.insert(keep, region.boundary.clone());
    if has_uv {
        uv.retain(|id, _| region.boundary.contains(id));
        mesh.uvs.insert(keep, uv);
    }
    let used: BTreeSet<_> = mesh.faces.values().flatten().copied().collect();
    let removed: BTreeSet<_> = old_vertices.difference(&used).copied().collect();
    mesh.vertices.retain(|id, _| !removed.contains(id));
    mesh.selection.retain(|id| !removed.contains(id));
    for component in mesh.components.values_mut() {
        if !component.faces.is_disjoint(&faces) {
            component.faces.insert(keep);
        }
    }
    mesh.flat_faces.retain(|id| mesh.faces.contains_key(id));
    crate::uv::reconcile(mesh);
    Ok(
        json!({"object":object,"retained_face":keep,"removed_faces":faces.iter().filter(|id|**id!=keep).collect::<Vec<_>>(),"removed_vertices":removed,"boundary":region.boundary,"positions_changed":0}),
    )
}
