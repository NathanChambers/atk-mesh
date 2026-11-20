use crate::{Mesh, normals, topology};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

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
