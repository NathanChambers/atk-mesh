use crate::{Mesh, validate, vector};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    if command == "weld" {
        validate(mesh)?;
    } else {
        validate_topology(mesh)?;
    }
    // Legacy projects may predate ID allocation metadata; remember IDs before removing any.
    mesh.vertex_id_high_watermark = mesh
        .vertex_id_high_watermark
        .max(*mesh.vertices.keys().next_back().unwrap_or(&0));
    mesh.face_id_high_watermark = mesh
        .face_id_high_watermark
        .max(*mesh.faces.keys().next_back().unwrap_or(&0));
    let operation = match command {
        "weld" => weld(mesh, request)?,
        "split_edge" => split_edge(mesh, request)?,
        "split_vertices" => split_vertices(mesh, request)?,
        "split_face" => split_face(mesh, request)?,
        "inset" => inset(mesh, request)?,
        "bevel_vertex" => bevel_vertex(mesh, request)?,
        "bridge_faces" => bridge_faces(mesh, request)?,
        _ => return Err("unknown topology command".into()),
    };
    validate(mesh)?;
    validate_topology(mesh)?;
    Ok(operation)
}

fn weld(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let vertices = if request.get("vertices").is_some() {
        ids(&request["vertices"])?
    } else {
        mesh.selection.clone()
    };
    if vertices.len() < 2 {
        return Err("weld requires at least two distinct vertices".into());
    }
    if vertices.iter().any(|id| !mesh.vertices.contains_key(id)) {
        return Err("unknown vertex".into());
    }

    let target = match request.get("target") {
        Some(value) => value.as_u64().ok_or("target must be a vertex ID")?,
        None => *vertices.first().unwrap(),
    };
    if !vertices.contains(&target) {
        return Err("target must belong to the welded vertices".into());
    }

    let mut position = [0.0; 3];
    for id in &vertices {
        for (axis, component) in position.iter_mut().enumerate() {
            *component += mesh.vertices[id][axis] / vertices.len() as f64;
        }
    }
    if request.get("target").is_some() {
        position = mesh.vertices[&target];
    }
    if request.get("position").is_some() {
        position = vector(&request["position"])?;
    }

    let mut removed_faces = vec![];
    for (id, face) in &mut mesh.faces {
        let mut collapsed = vec![];
        for vertex in face.iter() {
            let mapped = if vertices.contains(vertex) {
                target
            } else {
                *vertex
            };
            if collapsed.last() != Some(&mapped) {
                collapsed.push(mapped);
            }
        }
        if collapsed.len() > 1 && collapsed.first() == collapsed.last() {
            collapsed.pop();
        }
        if collapsed.len() < 3 {
            removed_faces.push(*id);
        }
        *face = collapsed;
    }
    for id in &removed_faces {
        mesh.faces.remove(id);
    }
    for id in &vertices {
        if *id != target {
            mesh.vertices.remove(id);
        }
    }
    mesh.vertices.insert(target, position);
    mesh.selection = BTreeSet::from([target]);
    let remap: BTreeMap<_, _> = vertices.iter().map(|id| (*id, target)).collect();
    Ok(
        json!({"command":"weld", "vertex_remap":remap, "removed_faces":removed_faces, "target":target}),
    )
}

fn split_edge(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let [a, b] = pair(&request["edge"])?;
    let fraction = fraction(request, "fraction", 0.5)?;
    let first = *mesh.vertices.get(&a).ok_or("unknown edge vertex")?;
    let second = *mesh.vertices.get(&b).ok_or("unknown edge vertex")?;
    let affected: Vec<_> = mesh
        .faces
        .iter()
        .filter(|(_, face)| has_edge(face, a, b))
        .map(|(id, _)| *id)
        .collect();
    if affected.is_empty() {
        return Err("vertices do not form an edge".into());
    }

    let point = interpolate(first, second, fraction);
    if point == first || point == second {
        return Err("split is too close to an endpoint".into());
    }

    let vertex = mesh.add_vertex(point)?;
    for id in &affected {
        let face = mesh.faces.get_mut(id).unwrap();
        let index = (0..face.len())
            .find(|i| edge_matches(face[*i], face[(*i + 1) % face.len()], a, b))
            .unwrap();
        face.insert(index + 1, vertex);
    }
    mesh.selection = BTreeSet::from([vertex]);
    Ok(
        json!({"command":"split_edge", "edge":[a,b], "created_vertices":[vertex], "affected_faces":affected}),
    )
}

fn split_vertices(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let faces = ids(&request["faces"])?;
    if faces.is_empty() {
        return Err("faces cannot be empty".into());
    }
    if faces.iter().any(|id| !mesh.faces.contains_key(id)) {
        return Err("unknown face".into());
    }

    let mut inside = BTreeSet::new();
    let mut outside = BTreeSet::new();
    for (id, face) in &mesh.faces {
        if faces.contains(id) {
            inside.extend(face);
        } else {
            outside.extend(face);
        }
    }
    let boundary: BTreeSet<u64> = inside.intersection(&outside).copied().collect();
    let vertices = if request.get("vertices").is_some() {
        ids(&request["vertices"])?
    } else {
        boundary.clone()
    };
    if vertices.is_empty() {
        return Err("no shared vertices to split".into());
    }
    if !vertices.is_subset(&boundary) {
        return Err("each split vertex must touch both chosen and remaining faces".into());
    }

    let mut remap = BTreeMap::new();
    for vertex in vertices {
        let point = mesh.vertices[&vertex];
        let duplicate = mesh.add_vertex(point)?;
        remap.insert(vertex, duplicate);
    }
    for id in &faces {
        for vertex in mesh.faces.get_mut(id).unwrap() {
            if let Some(duplicate) = remap.get(vertex) {
                *vertex = *duplicate;
            }
        }
    }
    mesh.selection = remap.values().copied().collect();
    Ok(
        json!({"command":"split_vertices", "vertex_remap":remap, "affected_faces":faces, "created_vertices":mesh.selection}),
    )
}

fn split_face(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let id = face_id(request)?;
    let face = mesh.faces.get(&id).ok_or("unknown face")?.clone();
    convex_face(mesh, &face)?;
    let [a, b] = pair(&request["vertices"])?;
    let mut first = face
        .iter()
        .position(|id| *id == a)
        .ok_or("vertex is not on the face")?;
    let mut second = face
        .iter()
        .position(|id| *id == b)
        .ok_or("vertex is not on the face")?;
    if first > second {
        std::mem::swap(&mut first, &mut second);
    }
    if second == first + 1 || (first == 0 && second == face.len() - 1) {
        return Err("face split requires non-adjacent vertices".into());
    }

    let first_half = face[first..=second].to_vec();
    let mut second_half = face[second..].to_vec();
    second_half.extend_from_slice(&face[..=first]);
    mesh.faces.insert(id, first_half);
    let created = mesh.add_face(second_half)?;
    mesh.selection = BTreeSet::from([a, b]);
    Ok(json!({"command":"split_face", "face":id, "created_faces":[created], "edge":[a,b]}))
}

fn inset(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let id = face_id(request)?;
    let face = mesh.faces.get(&id).ok_or("unknown face")?.clone();
    let (normal, scale) = convex_face(mesh, &face)?;
    let distance = request["distance"].as_f64().ok_or("distance is required")?;
    if !distance.is_finite() || distance <= 0.0 {
        return Err("distance must be finite and positive".into());
    }

    let points: Vec<_> = face.iter().map(|id| mesh.vertices[id]).collect();
    let mut inward = vec![];
    for i in 0..points.len() {
        let edge = subtract(points[(i + 1) % points.len()], points[i]);
        inward.push(cross(normal, normalize(edge)?));
    }
    let mut inner = vec![];
    for i in 0..points.len() {
        let previous = inward[(i + points.len() - 1) % points.len()];
        let current = inward[i];
        let denominator = 1.0 + dot(previous, current);
        if denominator < 1e-12 {
            return Err("inset corner is too sharp".into());
        }

        let mut point = points[i];
        for axis in 0..3 {
            point[axis] += (previous[axis] + current[axis]) * distance / denominator;
        }
        inner.push(point);
    }
    for i in 0..points.len() {
        for point in &inner {
            if dot(subtract(*point, points[i]), inward[i]) < distance - scale * 1e-8 {
                return Err("inset distance is too large for this face".into());
            }
        }
        let old_edge = subtract(points[(i + 1) % points.len()], points[i]);
        let new_edge = subtract(inner[(i + 1) % inner.len()], inner[i]);
        if dot(old_edge, new_edge) <= scale * scale * 1e-12 {
            return Err("inset would collapse or invert an edge".into());
        }
    }
    let mut cap = vec![];
    for point in inner {
        cap.push(mesh.add_vertex(point)?);
    }
    let mut created_faces = vec![];
    for i in 0..face.len() {
        let j = (i + 1) % face.len();
        created_faces.push(mesh.add_face(vec![face[i], face[j], cap[j], cap[i]])?);
    }
    mesh.faces.insert(id, cap.clone());
    mesh.selection = cap.iter().copied().collect();
    Ok(json!({"command":"inset", "face":id, "created_vertices":cap, "created_faces":created_faces}))
}

fn bevel_vertex(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let vertex = request["vertex"].as_u64().ok_or("vertex is required")?;
    let point = *mesh.vertices.get(&vertex).ok_or("unknown vertex")?;
    let fraction = fraction(request, "fraction", 0.1)?;
    let affected: Vec<_> = mesh
        .faces
        .iter()
        .filter(|(_, face)| face.contains(&vertex))
        .map(|(id, _)| *id)
        .collect();
    let mut links = BTreeMap::new();
    let mut neighbors = BTreeSet::new();
    for id in &affected {
        let face = &mesh.faces[id];
        convex_face(mesh, face)?;
        let index = face.iter().position(|id| *id == vertex).unwrap();
        let previous = face[(index + face.len() - 1) % face.len()];
        let next = face[(index + 1) % face.len()];
        if links.insert(next, previous).is_some() {
            return Err("vertex bevel requires a closed manifold fan".into());
        }
        neighbors.insert(previous);
        neighbors.insert(next);
    }
    if neighbors.len() < 3 || neighbors.len() != links.len() {
        return Err("vertex bevel requires a closed fan with at least three edges".into());
    }

    let first = *neighbors.first().unwrap();
    let mut order = vec![first];
    let mut current = first;
    loop {
        current = *links
            .get(&current)
            .ok_or("vertex bevel requires a closed fan")?;
        if current == first {
            break;
        }
        if order.contains(&current) {
            return Err("vertex has disconnected face fans".into());
        }
        order.push(current);
    }
    if order.len() != neighbors.len() {
        return Err("vertex has disconnected face fans".into());
    }

    let mut direction = [0.0; 3];
    for neighbor in &neighbors {
        let edge = subtract(mesh.vertices[neighbor], point);
        let unit = normalize(edge)?;
        for axis in 0..3 {
            direction[axis] += unit[axis];
        }
    }
    let direction = normalize(direction)?;
    let mut minimum_projection = f64::INFINITY;
    for neighbor in &neighbors {
        let edge = subtract(mesh.vertices[neighbor], point);
        let projection = dot(edge, direction);
        if projection <= 0.0 {
            return Err(
                "bevel requires a convex corner with all edges crossing the cut plane".into(),
            );
        }
        minimum_projection = minimum_projection.min(projection);
    }
    let distance = minimum_projection * fraction;
    let mut remap = BTreeMap::new();
    for neighbor in &neighbors {
        let end = mesh.vertices[neighbor];
        let edge = subtract(end, point);
        let cut_fraction = distance / dot(edge, direction);
        let cut = interpolate(point, end, cut_fraction);
        if cut == point || cut == end {
            return Err("bevel is too close to an endpoint".into());
        }
        remap.insert(*neighbor, mesh.add_vertex(cut)?);
    }
    for id in &affected {
        let face = mesh.faces.get_mut(id).unwrap();
        let index = face.iter().position(|id| *id == vertex).unwrap();
        let previous = face[(index + face.len() - 1) % face.len()];
        let next = face[(index + 1) % face.len()];
        face.splice(index..=index, [remap[&previous], remap[&next]]);
    }
    let cap: Vec<_> = order.iter().map(|id| remap[id]).collect();
    convex_face(mesh, &cap)?;
    let created_face = mesh.add_face(cap.clone())?;
    mesh.vertices.remove(&vertex);
    mesh.selection = cap.iter().copied().collect();
    Ok(
        json!({"command":"bevel_vertex", "removed_vertices":[vertex], "created_vertices":cap, "created_faces":[created_face], "edge_vertices":remap, "affected_faces":affected, "cut_plane":{"normal":direction, "distance_from_vertex":distance}}),
    )
}

fn bridge_faces(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let faces: [u64; 2] = serde_json::from_value(request["faces"].clone())
        .map_err(|_| "faces must contain two face IDs")?;
    if faces[0] == faces[1] {
        return Err("bridge requires two distinct faces".into());
    }

    let first = mesh
        .faces
        .get(&faces[0])
        .ok_or("unknown bridge face")?
        .clone();
    let second = mesh
        .faces
        .get(&faces[1])
        .ok_or("unknown bridge face")?
        .clone();
    if first.len() != second.len() {
        return Err(
            "bridge faces must have equal vertex counts; split edges to match them first".into(),
        );
    }
    if first.iter().any(|id| second.contains(id)) {
        return Err("bridge faces must not share vertices".into());
    }

    let (first_normal, first_scale) = convex_face(mesh, &first)?;
    let (second_normal, second_scale) = convex_face(mesh, &second)?;
    let centroid = |face: &[u64]| -> [f64; 3] {
        std::array::from_fn(|axis| {
            face.iter()
                .map(|id| mesh.vertices[id][axis] / face.len() as f64)
                .sum()
        })
    };
    let first_center = centroid(&first);
    let second_center = centroid(&second);
    let separation = subtract(second_center, first_center);
    let tolerance = first_scale.max(second_scale) * 1e-9;
    let faces_away =
        dot(first_normal, separation) <= tolerance || dot(second_normal, separation) >= -tolerance;
    if faces_away {
        return Err("bridge end faces must face the gap between them".into());
    }

    for face in [&first, &second] {
        for index in 0..face.len() {
            let a = face[index];
            let b = face[(index + 1) % face.len()];
            let incident = mesh
                .faces
                .values()
                .filter(|other| has_edge(other, a, b))
                .count();
            if incident != 2 {
                return Err(
                    "each bridge end edge must have one neighboring face besides its cap".into(),
                );
            }
        }
    }

    let count = first.len();
    let offsets: Vec<usize> = match request.get("offset") {
        Some(value) => {
            let offset = value
                .as_u64()
                .ok_or("offset must be a non-negative integer")?;
            if offset >= count as u64 {
                return Err("offset must be smaller than the loop's vertex count".into());
            }
            vec![offset as usize]
        }
        None => (0..count).collect(),
    };
    let mut best = None;
    let mut best_cost = f64::INFINITY;
    for offset in offsets {
        let aligned: Vec<_> = (0..count)
            .map(|i| second[(offset + count - i) % count])
            .collect();
        let mut quads = vec![];
        let mut cost = 0.0;
        for i in 0..count {
            let j = (i + 1) % count;
            quads.push(vec![first[i], first[j], aligned[j], aligned[i]]);
            let connector = subtract(mesh.vertices[&first[i]], mesh.vertices[&aligned[i]]);
            cost += dot(connector, connector);
        }
        let valid_quads = quads.iter().all(|quad| convex_face(mesh, quad).is_ok());
        if valid_quads && cost.is_finite() && cost < best_cost {
            best_cost = cost;
            best = Some((offset, aligned, quads));
        }
    }
    let (offset, aligned, quads) = best
        .ok_or("no valid bridge correspondence: side faces must be planar, convex and untwisted")?;
    let mut created = vec![];
    for quad in quads {
        created.push(mesh.add_face(quad)?);
    }
    for id in faces {
        mesh.faces.remove(&id);
    }
    mesh.selection = first.iter().chain(&second).copied().collect();
    let pairs: Vec<_> = first.iter().zip(&aligned).map(|(a, b)| [*a, *b]).collect();
    Ok(
        json!({"command":"bridge_faces", "removed_faces":faces, "created_faces":created, "vertex_pairs":pairs, "offset":offset}),
    )
}

fn ids(value: &Value) -> Result<BTreeSet<u64>, String> {
    let values: Vec<u64> = serde_json::from_value(value.clone())
        .map_err(|_| "expected an array of vertex or face IDs")?;
    let unique: BTreeSet<_> = values.iter().copied().collect();
    if unique.len() != values.len() {
        return Err("IDs must be distinct".into());
    }
    Ok(unique)
}

fn pair(value: &Value) -> Result<[u64; 2], String> {
    let pair: [u64; 2] =
        serde_json::from_value(value.clone()).map_err(|_| "expected two vertex IDs")?;
    if pair[0] == pair[1] {
        return Err("vertices must be distinct".into());
    }
    Ok(pair)
}

fn fraction(request: &Value, field: &str, default: f64) -> Result<f64, String> {
    let value = match request.get(field) {
        Some(value) => value.as_f64().ok_or("fraction must be a number")?,
        None => default,
    };
    if !value.is_finite() || value <= 0.0 || value >= 1.0 {
        return Err("fraction must lie strictly between zero and one".into());
    }
    Ok(value)
}

fn face_id(request: &Value) -> Result<u64, String> {
    request["face"].as_u64().ok_or("face is required".into())
}
fn subtract(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn interpolate(a: [f64; 3], b: [f64; 3], fraction: f64) -> [f64; 3] {
    std::array::from_fn(|axis| a[axis] * (1.0 - fraction) + b[axis] * fraction)
}

fn normalize(vector: [f64; 3]) -> Result<[f64; 3], String> {
    let length = dot(vector, vector).sqrt();
    if !length.is_finite() || length <= 0.0 {
        return Err("zero-length or unrepresentable geometry".into());
    }
    Ok(vector.map(|value| value / length))
}

fn edge_matches(first: u64, second: u64, a: u64, b: u64) -> bool {
    (first == a && second == b) || (first == b && second == a)
}
fn has_edge(face: &[u64], a: u64, b: u64) -> bool {
    (0..face.len()).any(|i| edge_matches(face[i], face[(i + 1) % face.len()], a, b))
}

pub(super) fn convex_face(mesh: &Mesh, face: &[u64]) -> Result<([f64; 3], f64), String> {
    let origin = mesh.vertices[&face[0]];
    let points: Vec<_> = face
        .iter()
        .map(|id| subtract(mesh.vertices[id], origin))
        .collect();
    let mut area = [0.0; 3];
    let mut scale: f64 = 0.0;
    for i in 0..points.len() {
        let contribution = cross(points[i], points[(i + 1) % points.len()]);
        for axis in 0..3 {
            area[axis] += contribution[axis];
        }
        scale = scale.max(dot(points[i], points[i]).sqrt());
    }
    let normal = normalize(area)?;
    for i in 0..points.len() {
        if dot(points[i], normal).abs() > scale * 1e-7 {
            return Err("operation requires a planar face or bevel cap".into());
        }

        let edge = subtract(points[(i + 1) % points.len()], points[i]);
        let inward = cross(normal, normalize(edge)?);
        for point in &points {
            if dot(subtract(*point, points[i]), inward) < -scale * 1e-9 {
                return Err("operation requires a convex face or bevel cap".into());
            }
        }
    }
    Ok((normal, scale))
}

pub(super) fn validate_topology(mesh: &Mesh) -> Result<(), String> {
    if mesh.defer_validation {
        return Ok(());
    }
    validate(mesh)?;
    let mut edges = BTreeMap::<(u64, u64), Vec<(u64, u64)>>::new();
    let mut faces = BTreeSet::new();
    let mut links = BTreeMap::<u64, BTreeMap<u64, Vec<u64>>>::new();
    for face in mesh.faces.values() {
        let start = face.iter().enumerate().min_by_key(|(_, id)| *id).unwrap().0;
        let forward: Vec<_> = (0..face.len())
            .map(|i| face[(start + i) % face.len()])
            .collect();
        let backward: Vec<_> = (0..face.len())
            .map(|i| face[(start + face.len() - i) % face.len()])
            .collect();
        if !faces.insert(forward.min(backward)) {
            return Err("operation would leave duplicate faces".into());
        }

        for i in 0..face.len() {
            let previous = face[(i + face.len() - 1) % face.len()];
            let vertex = face[i];
            let next = face[(i + 1) % face.len()];
            if mesh.vertices[&vertex] == mesh.vertices[&next] {
                return Err("operation would leave a zero-length edge".into());
            }

            edges
                .entry((vertex.min(next), vertex.max(next)))
                .or_default()
                .push((vertex, next));
            let link = links.entry(vertex).or_default();
            link.entry(previous).or_default().push(next);
            link.entry(next).or_default().push(previous);
        }
    }
    for incident in edges.values() {
        if incident.len() > 2 {
            return Err("operation would leave a non-manifold edge".into());
        }
        if incident.len() == 2 && incident[0] == incident[1] {
            return Err("operation would leave inconsistent face winding".into());
        }
    }
    for link in links.values() {
        if link.values().any(|neighbors| neighbors.len() > 2) {
            return Err("operation would leave a non-manifold vertex".into());
        }

        let mut visited = BTreeSet::new();
        let mut pending = vec![*link.keys().next().unwrap()];
        while let Some(vertex) = pending.pop() {
            if visited.insert(vertex) {
                pending.extend(&link[&vertex]);
            }
        }
        if visited.len() != link.len() {
            return Err("operation would leave disconnected face fans at a vertex".into());
        }
    }
    Ok(())
}
