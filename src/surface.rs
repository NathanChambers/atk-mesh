use crate::{Mesh, normals, topology};
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

type Point = [f64; 3];
type Edges = BTreeMap<(u64, u64), Vec<u64>>;

fn subtract(a: Point, b: Point) -> Point {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn dot(a: Point, b: Point) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: Point) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}

fn number(request: &Value, key: &str, default: f64, min: f64, max: f64) -> Result<f64, String> {
    let value = request
        .get(key)
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| format!("{key} must be numeric"))
        })
        .transpose()?
        .unwrap_or(default);
    if !value.is_finite() || value < min || value > max {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(value)
}

fn targets(mesh: &Mesh, request: &Value) -> Result<(BTreeSet<u64>, Vec<u64>), String> {
    let ids: BTreeSet<u64> = match (request.get("object"), request.get("objects")) {
        (Some(id), None) => BTreeSet::from([id.as_u64().ok_or("object must be an ID")?]),
        (None, Some(ids)) => {
            serde_json::from_value(ids.clone()).map_err(|_| "objects must be an array of IDs")?
        }
        _ => return Err("provide object or objects".into()),
    };
    if ids.is_empty() {
        return Err("objects cannot be empty".into());
    }
    let mut vertices = BTreeSet::new();
    for id in ids {
        let object = mesh.objects.get(&id).ok_or("unknown object")?;
        vertices.extend(&object.vertices);
    }
    let mut faces = vec![];
    for (id, face) in &mesh.faces {
        if !vertices.contains(&face[0]) {
            continue;
        }
        if face.len() != 3 {
            return Err("surface cleanup requires triangle faces in targeted objects".into());
        }
        faces.push(*id);
    }
    topology::validate_topology(mesh)?;
    Ok((vertices, faces))
}

fn edges(mesh: &Mesh, faces: &[u64]) -> Edges {
    let mut edges = Edges::new();
    for id in faces {
        let face = &mesh.faces[id];
        for index in 0..3 {
            let a = face[index];
            let b = face[(index + 1) % 3];
            edges.entry((a.min(b), a.max(b))).or_default().push(*id);
        }
    }
    edges
}

fn directions(request: &Value) -> Result<Vec<Point>, String> {
    let mut directions = vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    if let Some(views) = request.get("silhouette_views") {
        let views = views
            .as_array()
            .ok_or("silhouette_views must be an array of cameras")?;
        if views.len() > 16 {
            return Err("provide at most 16 silhouette views".into());
        }
        for view in views {
            let camera = crate::projection::Camera::parse(view)?;
            directions.push(camera.forward);
        }
    }
    Ok(directions)
}

fn contour(a: Point, b: Point, directions: &[Point]) -> bool {
    directions
        .iter()
        .any(|direction| dot(a, *direction) * dot(b, *direction) < 0.0)
}

pub(super) fn move_vertices(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let id = request["object"].as_u64().ok_or("object must be an ID")?;
    let object = mesh.objects.get(&id).ok_or("unknown object")?;
    let offsets = request["offsets"]
        .as_array()
        .filter(|offsets| (1..=16_384).contains(&offsets.len()))
        .ok_or("offsets must contain 1..16384 vertex offsets")?;

    let mut positions = BTreeMap::new();
    let mut maximum = 0.0_f64;
    let mut moved = 0;
    for entry in offsets {
        let vertex = entry["vertex"]
            .as_u64()
            .ok_or("offset vertex must be an ID")?;
        if !object.vertices.contains(&vertex) {
            return Err(format!("vertex {vertex} does not belong to object {id}"));
        }
        if positions.contains_key(&vertex) {
            return Err(format!("duplicate offset for vertex {vertex}"));
        }

        let delta = crate::vector(&entry["delta"])?;
        let before = mesh.vertices[&vertex];
        let after = crate::geometry::add(before, delta);
        if after.iter().any(|position| !position.is_finite()) {
            return Err(format!(
                "offset for vertex {vertex} produces a non-finite position"
            ));
        }
        let distance = length(subtract(after, before));
        if !distance.is_finite() {
            return Err(format!(
                "offset for vertex {vertex} produces a non-finite displacement"
            ));
        }
        positions.insert(vertex, after);
        maximum = maximum.max(distance);
        if before != after {
            moved += 1;
        }
    }

    mesh.vertices.extend(positions);
    Ok(json!({"object":id,"moved_vertices":moved,"max_displacement":maximum}))
}

pub(super) fn sculpt_stroke(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let id = request["object"].as_u64().ok_or("object must be an ID")?;
    let (vertices, faces) = targets(mesh, request)?;
    let sign = match request["mode"].as_str() {
        Some("raise") => 1.0,
        Some("lower") => -1.0,
        _ => return Err("sculpt mode must be raise or lower".into()),
    };
    let radius = positive(request, "radius")?;
    let strength = positive(request, "strength")?;
    let direction = request
        .get("direction")
        .map(|value| {
            let direction = crate::vector(value)?;
            let scale = direction.iter().map(|axis| axis.abs()).fold(0.0, f64::max);
            if scale == 0.0 {
                return Err("direction must be a nonzero world-space vector".to_owned());
            }
            Ok(normals::unit(direction.map(|axis| axis / scale)))
        })
        .transpose()?;
    let limit = if request.get("max_displacement").is_some() {
        positive(request, "max_displacement")?
    } else {
        strength
    };
    let samples = request["samples"]
        .as_array()
        .filter(|samples| (1..=128).contains(&samples.len()))
        .ok_or("samples must contain 1..128 face anchors")?;
    let path_direction = match request.get("continuous") {
        None | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) if samples.len() < 2 => {
            return Err("continuous strokes require at least two ordered samples".into());
        }
        Some(Value::Bool(true)) => {
            Some(direction.ok_or("continuous strokes require a fixed direction")?)
        }
        _ => return Err("continuous must be a boolean".into()),
    };
    let reach = if path_direction.is_some() {
        radius * 2.0
    } else {
        radius
    };

    let mut pending = BinaryHeap::new();
    let mut path = Vec::new();
    for sample in samples {
        let face = sample["face"].as_u64().ok_or("sample face must be an ID")?;
        let face = mesh.faces.get(&face).ok_or("unknown sample face")?;
        if !vertices.contains(&face[0]) {
            return Err("sample face does not belong to the sculpt object".into());
        }
        let weights = crate::vector(&sample["barycentric"])?;
        let sum: f64 = weights.iter().sum();
        if (sum - 1.0).abs() > 1e-6 || weights.iter().any(|weight| !(0.0..=1.0).contains(weight)) {
            return Err("barycentric weights must be in [0,1] and sum to 1".into());
        }
        let center: Point = std::array::from_fn(|axis| {
            face.iter()
                .zip(weights)
                .map(|(vertex, weight)| mesh.vertices[vertex][axis] * (weight / sum))
                .sum()
        });
        if let Some(axis) = path_direction {
            let pressure = number(sample, "pressure", 1.0, 0.0, 1.0)?;
            path.push((project_onto_plane(center, axis), pressure));
        } else if sample.get("pressure").is_some() {
            return Err("sample pressure requires a continuous stroke".into());
        }
        for vertex in face {
            let distance = length(subtract(mesh.vertices[vertex], center));
            if distance < reach {
                // Nonnegative finite f64 values sort in bit order.
                pending.push(Reverse((distance.to_bits(), *vertex)));
            }
        }
    }
    if path
        .windows(2)
        .any(|pair| length(subtract(pair[1].0, pair[0].0)) > radius)
    {
        return Err(
            "continuous stroke samples must be at most one radius apart in the brush plane".into(),
        );
    }

    let mut adjacency = BTreeMap::<u64, Vec<(u64, f64)>>::new();
    let mut fixed = BTreeSet::new();
    for ((a, b), incident) in edges(mesh, &faces) {
        let distance = length(subtract(mesh.vertices[&a], mesh.vertices[&b]));
        adjacency.entry(a).or_default().push((b, distance));
        adjacency.entry(b).or_default().push((a, distance));
        if incident.len() != 2 {
            fixed.extend([a, b]);
        }
    }

    let mut distances = BTreeMap::new();
    while let Some(Reverse((distance, vertex))) = pending.pop() {
        if distances.contains_key(&vertex) {
            continue;
        }
        let distance = f64::from_bits(distance);
        distances.insert(vertex, distance);
        for (neighbor, edge_length) in &adjacency[&vertex] {
            let next = distance + edge_length;
            if next < reach && !distances.contains_key(neighbor) {
                pending.push(Reverse((next.to_bits(), *neighbor)));
            }
        }
    }

    let mut area_normals = BTreeMap::<u64, Point>::new();
    for face in faces.iter().map(|id| &mesh.faces[id]) {
        let area = triangle_area(face, &mesh.vertices);
        for vertex in face
            .iter()
            .filter(|id| distances.contains_key(id) && !fixed.contains(id))
        {
            let sum = area_normals.entry(*vertex).or_default();
            *sum = crate::geometry::add(*sum, area);
        }
    }

    let mut weights = BTreeMap::new();
    for &vertex in area_normals.keys() {
        let distance = distances[&vertex];
        let weight = if let Some(axis) = path_direction {
            let point = project_onto_plane(mesh.vertices[&vertex], axis);
            let (lateral, pressure) = path
                .windows(2)
                .map(|pair| {
                    let (distance, fraction) = segment_position(point, pair[0].0, pair[1].0);
                    let pressure = (1.0 - fraction) * pair[0].1 + fraction * pair[1].1;
                    (distance, pressure)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .expect("continuous strokes have at least two anchors");
            pressure
                * brush_falloff(1.0 - lateral / radius)
                * brush_falloff(4.0 - 2.0 * distance / radius)
        } else {
            brush_falloff(1.0 - distance / radius)
        };
        weights.insert(vertex, weight);
    }
    // Smooth the influence field to soften steps from edge routing and discrete anchors.
    let blend_passes = if path_direction.is_some() { 0 } else { 2 };
    for _ in 0..blend_passes {
        weights = weights
            .iter()
            .map(|(&vertex, &weight)| {
                let mut total = 0.0;
                let mut sum = 0.0;
                for &(neighbor, edge_length) in &adjacency[&vertex] {
                    let influence = 1.0 / (edge_length / radius).max(1e-6);
                    total += influence;
                    sum += influence * weights.get(&neighbor).copied().unwrap_or(0.0);
                }
                (vertex, 0.5 * (weight + sum / total))
            })
            .collect();
    }

    let mut positions = BTreeMap::new();
    let mut maximum = 0.0_f64;
    for (vertex, area) in area_normals {
        let before = mesh.vertices[&vertex];
        let amount = strength.min(limit) * weights[&vertex];
        let axis = direction.unwrap_or_else(|| normals::unit(area));
        let offset = axis.map(|component| component * sign * amount);
        let after = crate::geometry::add(before, offset);
        if after != before {
            maximum = maximum.max(length(subtract(after, before)));
            positions.insert(vertex, after);
        }
    }
    if positions.is_empty() {
        return Err(
            "stroke reached no movable vertices; use a larger radius or a denser patch".into(),
        );
    }

    for face in faces.iter().map(|id| &mesh.faces[id]) {
        if !face.iter().any(|id| positions.contains_key(id)) {
            continue;
        }
        let before = triangle_area(face, &mesh.vertices);
        let [a, b, c] = [face[0], face[1], face[2]]
            .map(|id| positions.get(&id).copied().unwrap_or(mesh.vertices[&id]));
        let after = cross(subtract(b, a), subtract(c, a));
        let area = length(after);
        if !area.is_finite()
            || area < length(before) * 0.2
            || dot(normals::unit(before), normals::unit(after)) <= 0.0
        {
            return Err(
                "stroke would collapse or flip a triangle; reduce strength or displacement".into(),
            );
        }
    }

    let moved = positions.len();
    mesh.vertices.extend(positions);
    Ok(
        json!({"object":id,"moved_vertices":moved,"max_displacement":maximum,"displacement_limit":limit}),
    )
}

fn project_onto_plane(point: Point, normal: Point) -> Point {
    let height = dot(point, normal);
    std::array::from_fn(|axis| point[axis] - height * normal[axis])
}

fn segment_position(point: Point, start: Point, end: Point) -> (f64, f64) {
    let delta = subtract(end, start);
    let size = length(delta);
    let from_start = subtract(point, start);
    if size == 0.0 {
        return (length(from_start), 0.5);
    }
    let unit = delta.map(|axis| axis / size);
    let along = dot(from_start, unit).clamp(0.0, size);
    let lateral = length(std::array::from_fn(|axis| {
        from_start[axis] - along * unit[axis]
    }));
    (lateral, along / size)
}

fn brush_falloff(radial: f64) -> f64 {
    let radial = radial.clamp(0.0, 1.0);
    radial * radial * (3.0 - 2.0 * radial)
}

fn positive(request: &Value, field: &str) -> Result<f64, String> {
    request[field]
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| format!("{field} must be a positive finite number"))
}

fn triangle_area(face: &[u64], vertices: &BTreeMap<u64, Point>) -> Point {
    let [a, b, c] = [face[0], face[1], face[2]].map(|vertex| vertices[&vertex]);
    cross(subtract(b, a), subtract(c, a))
}

pub(super) fn relax(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let (vertices, faces) = targets(mesh, request)?;
    let limit = number(request, "max_displacement", 0.003, f64::MIN_POSITIVE, 1.0)?;
    let factor = number(request, "factor", 0.2, f64::MIN_POSITIVE, 0.5)?;
    let angle = number(request, "feature_angle", 35.0, 5.0, 80.0)?;
    let iterations = request
        .get("iterations")
        .map(|value| {
            value
                .as_u64()
                .filter(|value| (1..=10).contains(value))
                .ok_or("iterations must be 1..10")
        })
        .transpose()?
        .unwrap_or(4);
    let original = mesh.vertices.clone();
    let source_normals: BTreeMap<_, _> = faces
        .iter()
        .map(|id| (*id, normals::face(mesh, &mesh.faces[id])))
        .collect();
    let edge_faces = edges(mesh, &faces);
    let directions = directions(request)?;
    let mut fixed = BTreeSet::new();
    let mut adjacency = BTreeMap::<u64, BTreeSet<u64>>::new();
    for ((a, b), incident) in &edge_faces {
        adjacency.entry(*a).or_default().insert(*b);
        adjacency.entry(*b).or_default().insert(*a);
        let boundary = incident.len() != 2;
        let feature = !boundary
            && dot(source_normals[&incident[0]], source_normals[&incident[1]])
                < angle.to_radians().cos();
        let silhouette = !boundary
            && contour(
                source_normals[&incident[0]],
                source_normals[&incident[1]],
                &directions,
            );
        if boundary || feature || silhouette {
            fixed.extend([*a, *b]);
        }
    }
    for object in mesh.objects.values() {
        if !object.vertices.iter().any(|id| vertices.contains(id)) {
            continue;
        }
        for axis in 0..3 {
            let min = object
                .vertices
                .iter()
                .map(|id| original[id][axis])
                .fold(f64::INFINITY, f64::min);
            let max = object
                .vertices
                .iter()
                .map(|id| original[id][axis])
                .fold(f64::NEG_INFINITY, f64::max);
            fixed.extend(
                object
                    .vertices
                    .iter()
                    .filter(|id| original[id][axis] == min || original[id][axis] == max),
            );
        }
    }
    let movable: Vec<_> = vertices
        .difference(&fixed)
        .filter(|id| adjacency.contains_key(id))
        .copied()
        .collect();
    let source_areas: BTreeMap<_, _> = faces
        .iter()
        .map(|id| {
            let f = &mesh.faces[id];
            let area = cross(
                subtract(original[&f[1]], original[&f[0]]),
                subtract(original[&f[2]], original[&f[0]]),
            );
            (*id, length(area))
        })
        .collect();
    let cosine = 15.0_f64.to_radians().cos();
    let mut accepted = 0;
    for _ in 0..iterations {
        let previous = mesh.vertices.clone();
        let mut candidates = BTreeMap::new();
        for id in &movable {
            let mut centroid = [0.0; 3];
            let mut total = 0.0;
            for neighbor in &adjacency[id] {
                let weight = 1.0 / length(subtract(previous[neighbor], previous[id])).max(1e-12);
                for axis in 0..3 {
                    centroid[axis] += previous[neighbor][axis] * weight;
                }
                total += weight;
            }
            let candidate: Point = std::array::from_fn(|axis| {
                previous[id][axis] + factor * (centroid[axis] / total - previous[id][axis])
            });
            let delta = subtract(candidate, original[id]);
            let distance = length(delta);
            let candidate = if distance > limit {
                std::array::from_fn(|axis| original[id][axis] + delta[axis] * limit / distance)
            } else {
                candidate
            };
            candidates.insert(*id, candidate);
        }
        let mut step = 1.0;
        let mut valid = false;
        for _ in 0..12 {
            for (id, candidate) in &candidates {
                let point = std::array::from_fn(|axis| {
                    previous[id][axis] + step * (candidate[axis] - previous[id][axis])
                });
                mesh.vertices.insert(*id, point);
            }
            valid = faces.iter().all(|id| {
                let f = &mesh.faces[id];
                let area = cross(
                    subtract(mesh.vertices[&f[1]], mesh.vertices[&f[0]]),
                    subtract(mesh.vertices[&f[2]], mesh.vertices[&f[0]]),
                );
                let size = length(area);
                size >= source_areas[id] * 0.2
                    && dot(normals::unit(area), source_normals[id]) >= cosine
            });
            if valid {
                break;
            }
            step *= 0.5;
        }
        if !valid {
            mesh.vertices = previous;
            break;
        }
        accepted += 1;
    }
    topology::validate_topology(mesh)?;
    let moved: Vec<_> = movable
        .iter()
        .filter(|id| original[id] != mesh.vertices[id])
        .copied()
        .collect();
    let maximum = moved
        .iter()
        .map(|id| length(subtract(original[id], mesh.vertices[id])))
        .fold(0.0_f64, f64::max);
    Ok(
        json!({"moved_vertices":moved.len(),"fixed_vertices":fixed.len(),"iterations":accepted,"max_displacement":maximum,"displacement_limit":limit,"feature_angle":angle,"faces_changed":0,"silhouette_directions":directions,"note":"Movement bound is relative to this command input. Boundaries, creases, silhouette vertices and object extrema are fixed. Face normals stay within 15 degrees and areas above 20 percent of input."}),
    )
}

fn min_angle(mesh: &Mesh, face: &[u64]) -> f64 {
    (0..3)
        .map(|index| {
            let a = normals::unit(subtract(
                mesh.vertices[&face[(index + 1) % 3]],
                mesh.vertices[&face[index]],
            ));
            let b = normals::unit(subtract(
                mesh.vertices[&face[(index + 2) % 3]],
                mesh.vertices[&face[index]],
            ));
            dot(a, b).clamp(-1.0, 1.0).acos().to_degrees()
        })
        .fold(180.0_f64, f64::min)
}

struct Flip {
    edge: (u64, u64),
    diagonal: (u64, u64),
    faces: [u64; 2],
    loops: [Vec<u64>; 2],
    gain: f64,
    deviation: f64,
}

pub(super) fn improve(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let (_, faces) = targets(mesh, request)?;
    let limit = number(request, "max_deviation", 0.0005, 0.0, 1.0)?;
    let gain = number(request, "min_angle_gain", 3.0, 0.1, 30.0)?;
    let angle = number(request, "feature_angle", 10.0, 0.1, 30.0)?;
    let edge_faces = edges(mesh, &faces);
    let source_normals: BTreeMap<_, _> = faces
        .iter()
        .map(|id| (*id, normals::face(mesh, &mesh.faces[id])))
        .collect();
    let directions = directions(request)?;
    let mut candidates = vec![];
    for ((first, second), incident) in &edge_faces {
        if incident.len() != 2 {
            continue;
        }
        let [f1, f2] = [incident[0], incident[1]];
        if mesh.flat_faces.contains(&f1) != mesh.flat_faces.contains(&f2) {
            continue;
        }
        let n1 = source_normals[&f1];
        let n2 = source_normals[&f2];
        if dot(n1, n2) < angle.to_radians().cos() || contour(n1, n2, &directions) {
            continue;
        }
        let f = &mesh.faces[&f1];
        let index = f.iter().position(|id| *id == *first).unwrap();
        let (a, b) = if f[(index + 1) % 3] == *second {
            (*first, *second)
        } else {
            (*second, *first)
        };
        let c = *f.iter().find(|id| **id != a && **id != b).unwrap();
        let d = *mesh.faces[&f2]
            .iter()
            .find(|id| **id != a && **id != b)
            .unwrap();
        let diagonal = (c.min(d), c.max(d));
        if c == d || edge_faces.contains_key(&diagonal) {
            continue;
        }
        let normal = normals::unit(std::array::from_fn(|axis| n1[axis] + n2[axis]));
        let project = |p: Point| {
            let height = dot(p, normal);
            std::array::from_fn(|axis| p[axis] - normal[axis] * height)
        };
        let u = project(subtract(mesh.vertices[&b], mesh.vertices[&a]));
        let v = project(subtract(mesh.vertices[&d], mesh.vertices[&c]));
        let r = project(subtract(mesh.vertices[&c], mesh.vertices[&a]));
        let (uu, uv, vv, ru, rv) = (dot(u, u), dot(u, v), dot(v, v), dot(r, u), dot(r, v));
        let determinant = uu * vv - uv * uv;
        if determinant <= uu * vv * 1e-12 {
            continue;
        }
        let t = (ru * vv - rv * uv) / determinant;
        let s = (ru * uv - rv * uu) / determinant;
        if !(1e-6..1.0 - 1e-6).contains(&t) || !(1e-6..1.0 - 1e-6).contains(&s) {
            continue;
        }
        let old: Point = std::array::from_fn(|axis| {
            mesh.vertices[&a][axis] + t * (mesh.vertices[&b][axis] - mesh.vertices[&a][axis])
        });
        let new: Point = std::array::from_fn(|axis| {
            mesh.vertices[&c][axis] + s * (mesh.vertices[&d][axis] - mesh.vertices[&c][axis])
        });
        let deviation = length(subtract(old, new));
        if deviation > limit {
            continue;
        }
        let loops = [vec![c, d, b], vec![d, c, a]];
        let next_normals = [
            normals::face(mesh, &loops[0]),
            normals::face(mesh, &loops[1]),
        ];
        let sharp = next_normals
            .iter()
            .any(|next| dot(*next, normal) < 15.0_f64.to_radians().cos());
        let crossing = contour(next_normals[0], next_normals[1], &directions)
            || next_normals.iter().any(|next| {
                directions
                    .iter()
                    .any(|direction| dot(*next, *direction) * dot(n1, *direction) < 0.0)
            });
        if sharp || crossing {
            continue;
        }
        let before = min_angle(mesh, &mesh.faces[&f1]).min(min_angle(mesh, &mesh.faces[&f2]));
        let after = min_angle(mesh, &loops[0]).min(min_angle(mesh, &loops[1]));
        if after < before + gain {
            continue;
        }
        candidates.push(Flip {
            edge: (a.min(b), a.max(b)),
            diagonal,
            faces: [f1, f2],
            loops,
            gain: after - before,
            deviation,
        });
    }
    candidates.sort_by(|a, b| b.gain.total_cmp(&a.gain));
    let mut used = BTreeSet::new();
    let mut present: BTreeSet<_> = edge_faces.keys().copied().collect();
    let mut changes = vec![];
    let mut maximum = 0.0_f64;
    for candidate in candidates {
        if candidate.faces.iter().any(|id| used.contains(id))
            || present.contains(&candidate.diagonal)
        {
            continue;
        }
        for index in 0..2 {
            mesh.faces
                .insert(candidate.faces[index], candidate.loops[index].clone());
            used.insert(candidate.faces[index]);
        }
        present.remove(&candidate.edge);
        present.insert(candidate.diagonal);
        maximum = maximum.max(candidate.deviation);
        changes.push(json!({"faces":candidate.faces,"old_edge":candidate.edge,"new_edge":candidate.diagonal,"minimum_angle_gain":candidate.gain,"deviation":candidate.deviation}));
    }
    topology::validate_topology(mesh)?;
    Ok(
        json!({"flipped_edges":changes.len(),"changes":changes,"max_patch_deviation":maximum,"deviation_limit":limit,"positions_changed":0,"faces_removed":0,"note":"Each face participates at most once. Convex projected patches preserve boundary edges; diagonal intersection separation bounds the patch surface deviation. Bounds apply to this command input."}),
    )
}
