use crate::{Mesh, objects};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn source(mesh: &Mesh, request: &Value) -> Result<(Vec<u64>, Vec<[f64; 3]>), String> {
    let object = request["source_object"]
        .as_u64()
        .ok_or("source_object is required")?;
    let object = mesh.objects.get(&object).ok_or("unknown source object")?;
    let vertices: Vec<u64> = serde_json::from_value(request["vertices"].clone())
        .map_err(|_| "vertices must be an ordered edge chain")?;
    let unique: BTreeSet<_> = vertices.iter().copied().collect();
    if vertices.len() < 2 || vertices.len() != unique.len() {
        return Err("profile requires at least two distinct vertices without repeats".into());
    }
    if vertices
        .iter()
        .any(|vertex| !object.vertices.contains(vertex))
    {
        return Err("profile vertices must belong to the source object".into());
    }
    let mut edges = BTreeSet::new();
    for face in mesh.faces.values() {
        for index in 0..face.len() {
            let a = face[index];
            let b = face[(index + 1) % face.len()];
            edges.insert((a.min(b), a.max(b)));
        }
    }
    for pair in vertices.windows(2) {
        if !edges.contains(&(pair[0].min(pair[1]), pair[0].max(pair[1]))) {
            return Err("consecutive profile vertices must share an existing edge".into());
        }
    }
    let points: Vec<_> = vertices
        .iter()
        .map(|vertex| mesh.vertices[vertex])
        .collect();
    let z = points[0][2];
    if points.iter().any(|point| (point[2] - z).abs() > 1e-8) {
        return Err("profile must lie in an XY plane".into());
    }
    Ok((vertices, points))
}

pub(super) fn inspect(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let (vertices, points) = source(mesh, request)?;
    let length: f64 = points
        .windows(2)
        .map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
        .sum();
    Ok(
        json!({"revision":revision,"source_object":request["source_object"],"vertices":vertices,"points":points,"length":length,"plane":"XY","offset_side":"right of ordered chain"}),
    )
}

fn offset(points: &[[f64; 3]], distance: f64) -> Result<Vec<[f64; 2]>, String> {
    let mut normals = vec![];
    for pair in points.windows(2) {
        let dx = pair[1][0] - pair[0][0];
        let dy = pair[1][1] - pair[0][1];
        let length = dx.hypot(dy);
        if length < 1e-8 {
            return Err("profile contains a zero-length projected edge".into());
        }
        normals.push([dy / length, -dx / length]);
    }
    let mut result = vec![];
    for (index, point) in points.iter().enumerate() {
        let before = normals[index.saturating_sub(1)];
        let after = normals[index.min(normals.len() - 1)];
        let sum = [before[0] + after[0], before[1] + after[1]];
        let divisor = sum[0] * after[0] + sum[1] * after[1];
        if divisor < 1e-6 {
            return Err("profile has a reversing or excessive miter".into());
        }
        result.push([
            point[0] + distance * sum[0] / divisor,
            point[1] + distance * sum[1] / divisor,
        ]);
    }
    for (index, pair) in result.windows(2).enumerate() {
        let dx = points[index + 1][0] - points[index][0];
        let dy = points[index + 1][1] - points[index][1];
        if (pair[1][0] - pair[0][0]) * dx + (pair[1][1] - pair[0][1]) * dy < 1e-10 {
            return Err("offset collapses or reverses a profile segment".into());
        }
    }
    Ok(result)
}

fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn intersects(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    if (0..2).any(|axis| {
        a[axis].max(b[axis]) < c[axis].min(d[axis]) - 1e-10
            || c[axis].max(d[axis]) < a[axis].min(b[axis]) - 1e-10
    }) {
        return false;
    }
    cross(a, b, c) * cross(a, b, d) <= 1e-20 && cross(c, d, a) * cross(c, d, b) <= 1e-20
}

pub(super) fn create(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let (source_vertices, points) = source(mesh, request)?;
    let distance = request["offset"].as_f64().ok_or("offset is required")?;
    let width = request["width"].as_f64().ok_or("width is required")?;
    let depth = request["depth"].as_f64().ok_or("depth is required")?;
    let z = match request.get("z") {
        Some(value) => value.as_f64().ok_or("z must be a number")?,
        None => points[0][2],
    };
    if !distance.is_finite()
        || !width.is_finite()
        || !depth.is_finite()
        || !z.is_finite()
        || width <= 1e-8
        || depth <= 1e-8
    {
        return Err("profile dimensions must be finite; width and depth must be positive".into());
    }
    let outer = offset(&points, distance)?;
    let inner = offset(&points, distance + width)?;
    let mut contour = outer.clone();
    contour.extend(inner.iter().rev());
    let count = contour.len();
    for first in 0..count {
        for second in first + 1..count {
            if second == first + 1 || (first == 0 && second == count - 1) {
                continue;
            }
            if intersects(
                contour[first],
                contour[(first + 1) % count],
                contour[second],
                contour[(second + 1) % count],
            ) {
                return Err("profile ribbon would self-intersect".into());
            }
        }
    }
    let object = objects::destination_object(mesh, request)?;
    let mut sections = vec![];
    let mut vertices = BTreeSet::new();
    for (outer, inner) in outer.iter().zip(&inner) {
        let section = [
            mesh.add_vertex([outer[0], outer[1], z - depth * 0.5])?,
            mesh.add_vertex([inner[0], inner[1], z - depth * 0.5])?,
            mesh.add_vertex([outer[0], outer[1], z + depth * 0.5])?,
            mesh.add_vertex([inner[0], inner[1], z + depth * 0.5])?,
        ];
        vertices.extend(section);
        sections.push(section);
    }
    let mut faces = vec![];
    for pair in sections.windows(2) {
        let [a, b, c, d] = pair[0];
        let [e, f, g, h] = pair[1];
        for face in [
            vec![c, d, h, g],
            vec![a, e, f, b],
            vec![a, c, g, e],
            vec![b, f, h, d],
        ] {
            faces.push(mesh.add_face(face)?);
        }
    }
    let [a, b, c, d] = sections[0];
    faces.push(mesh.add_face(vec![a, b, d, c])?);
    let [a, b, c, d] = *sections.last().unwrap();
    faces.push(mesh.add_face(vec![a, c, d, b])?);
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(&vertices);
    mesh.selection = vertices;
    Ok(
        json!({"object":object,"source_object":request["source_object"],"source_vertices":source_vertices,"sections":sections,"outer_profile":outer,"inner_profile":inner,"faces":faces,"offset":distance,"width":width,"depth":depth,"z":z}),
    )
}
