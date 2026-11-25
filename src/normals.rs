use crate::Mesh;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = v[0].hypot(v[1]).hypot(v[2]);
    if length > 0.0 {
        v.map(|x| x / length)
    } else {
        [0.0; 3]
    }
}

pub(super) fn face(mesh: &Mesh, vertices: &[u64]) -> [f64; 3] {
    let mut n = [0.0; 3];
    let origin = mesh.vertices[&vertices[0]];
    for i in 0..vertices.len() {
        let a = mesh.vertices[&vertices[i]];
        let b = mesh.vertices[&vertices[(i + 1) % vertices.len()]];
        for (axis, value) in n.iter_mut().enumerate() {
            let j = (axis + 1) % 3;
            let k = (axis + 2) % 3;
            *value +=
                (a[j] - origin[j]) * (b[k] - origin[k]) - (a[k] - origin[k]) * (b[j] - origin[j]);
        }
    }
    unit(n)
}

pub(super) fn triangles(mesh: &Mesh, vertices: &[u64]) -> Vec<[usize; 3]> {
    let fan: Vec<_> = (1..vertices.len() - 1).map(|i| [0, i, i + 1]).collect();
    if vertices.len() <= 3 {
        return fan;
    }
    let normal = face(mesh, vertices);
    let axis = (0..3)
        .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
        .unwrap();
    let origin = mesh.vertices[&vertices[0]];
    let points: Vec<_> = vertices
        .iter()
        .map(|id| {
            [
                mesh.vertices[id][(axis + 1) % 3] - origin[(axis + 1) % 3],
                mesh.vertices[id][(axis + 2) % 3] - origin[(axis + 2) % 3],
            ]
        })
        .collect();
    let mut positive = false;
    let mut negative = false;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let c = points[(i + 2) % points.len()];
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        positive |= cross > 0.0;
        negative |= cross < 0.0;
    }
    if positive && negative {
        crate::shaping::polygon_triangles(&points).unwrap_or(fan)
    } else {
        fan
    }
}

pub(super) fn corners(mesh: &Mesh) -> BTreeMap<u64, Vec<[f64; 3]>> {
    let normals: BTreeMap<_, _> = mesh
        .faces
        .iter()
        .map(|(id, f)| (*id, face(mesh, f)))
        .collect();
    let owners: BTreeMap<_, _> = mesh
        .objects
        .values()
        .flat_map(|object| {
            object
                .vertices
                .iter()
                .map(move |id| (*id, object.smooth_angle))
        })
        .collect();
    let mut edges = BTreeMap::<(u64, u64), Vec<u64>>::new();
    for (id, f) in &mesh.faces {
        for i in 0..f.len() {
            let a = f[i];
            let b = f[(i + 1) % f.len()];
            edges.entry((a.min(b), a.max(b))).or_default().push(*id);
        }
    }
    let mut output = BTreeMap::new();
    for (id, f) in &mesh.faces {
        let mut result = vec![];
        for vertex in f {
            if mesh.flat_faces.contains(id) {
                result.push(normals[id]);
                continue;
            }
            let Some(angle) = owners.get(vertex).copied().flatten() else {
                result.push(normals[id]);
                continue;
            };
            if angle == 0.0 {
                result.push(normals[id]);
                continue;
            }
            let cosine = angle.to_radians().cos();
            let mut seen = BTreeSet::new();
            let mut pending = vec![*id];
            let mut sum = [0.0; 3];
            while let Some(current) = pending.pop() {
                if !seen.insert(current) {
                    continue;
                }
                let polygon = &mesh.faces[&current];
                let i = polygon.iter().position(|v| v == vertex).unwrap();
                let p = mesh.vertices[vertex];
                let before = mesh.vertices[&polygon[(i + polygon.len() - 1) % polygon.len()]];
                let after = mesh.vertices[&polygon[(i + 1) % polygon.len()]];
                let a = unit(std::array::from_fn(|axis| before[axis] - p[axis]));
                let b = unit(std::array::from_fn(|axis| after[axis] - p[axis]));
                let weight = a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    .clamp(-1.0, 1.0)
                    .acos();
                for (axis, value) in sum.iter_mut().enumerate() {
                    *value += normals[&current][axis] * weight;
                }
                for neighbor in [
                    polygon[(i + polygon.len() - 1) % polygon.len()],
                    polygon[(i + 1) % polygon.len()],
                ] {
                    for adjacent in &edges[&((*vertex).min(neighbor), (*vertex).max(neighbor))] {
                        let dot: f64 = normals[&current]
                            .iter()
                            .zip(normals[adjacent])
                            .map(|(a, b)| a * b)
                            .sum();
                        if dot >= cosine - 1e-10 {
                            pending.push(*adjacent);
                        }
                    }
                }
            }
            let normal = unit(sum);
            result.push(if normal == [0.0; 3] {
                normals[id]
            } else {
                normal
            });
        }
        output.insert(*id, result);
    }
    for link in mesh.objects.values().filter_map(|object| object.instance.as_ref()) {
        if link.scale == [1.0; 3] {
            continue;
        }
        for (master, instance) in &link.faces {
            let transformed = output[master].iter()
                .map(|normal| unit(link.transform_direction(*normal, true)))
                .collect();
            output.insert(*instance, transformed);
        }
    }
    output
}
