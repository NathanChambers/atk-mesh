use crate::{Mesh, construction, topology};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

type PatchLayout = (Vec<[f64; 3]>, Vec<Vec<usize>>);

fn boundary<'a>(faces: impl IntoIterator<Item = &'a Vec<u64>>) -> Result<Vec<u64>, String> {
    let mut edges = BTreeMap::<_, Vec<_>>::new();
    let mut vertices = BTreeSet::new();
    let mut neighbors = Vec::<Vec<usize>>::new();
    for (index, face) in faces.into_iter().enumerate() {
        neighbors.push(vec![]);
        vertices.extend(face.iter().copied());
        for (i, &a) in face.iter().enumerate() {
            let b = face[(i + 1) % face.len()];
            edges
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((a, b, index));
        }
    }

    let mut next = BTreeMap::new();
    for uses in edges.values() {
        match uses.as_slice() {
            [(a, b, _)] => {
                if next.insert(*a, *b).is_some() {
                    return Err("patch boundary branches".into());
                }
            }
            [(a, b, first), (c, d, second)] if a == d && b == c => {
                neighbors[*first].push(*second);
                neighbors[*second].push(*first);
            }
            _ => {
                return Err("patch edges must have consistent winding and at most two faces".into());
            }
        }
    }
    let start = *next
        .keys()
        .next()
        .ok_or("patch requires an open perimeter")?;
    let mut ordered = vec![start];
    let mut cursor = next[&start];
    while cursor != start {
        if ordered.len() == next.len() {
            return Err("patch perimeter does not form a simple loop".into());
        }
        ordered.push(cursor);
        cursor = *next.get(&cursor).ok_or("patch perimeter is open")?;
    }
    if ordered.len() != next.len() || vertices.len() + neighbors.len() != edges.len() + 1 {
        return Err("patch must be a disk with exactly one boundary".into());
    }

    let mut visited = BTreeSet::new();
    let mut pending = vec![0];
    while let Some(index) = pending.pop() {
        if visited.insert(index) {
            pending.extend(neighbors[index].iter().copied());
        }
    }
    if visited.len() != neighbors.len() {
        return Err("patch faces must be connected by edges".into());
    }
    Ok(ordered)
}

fn source(mesh: &Mesh, request: &Value) -> Result<(u64, BTreeSet<u64>, Vec<u64>), String> {
    let object = crate::surface_ops::object(mesh, request)?;
    let faces = crate::regions::selection(mesh, request)?;
    let perimeter = boundary(faces.iter().map(|id| &mesh.faces[id]))?;
    Ok((object, faces, perimeter))
}

pub(super) fn inspect(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let (object, faces, perimeter) = source(mesh, request)?;
    let positions: Vec<_> = perimeter.iter().map(|id| mesh.vertices[id]).collect();
    Ok(
        json!({"revision":revision,"object":object,"faces":faces,"boundary":perimeter,"boundary_positions":positions}),
    )
}

pub(super) fn named_boundary(mesh: &Mesh, name: &str) -> Result<Vec<u64>, String> {
    let id = construction::named(mesh, name)?;
    boundary(
        mesh.components[&id]
            .faces
            .iter()
            .filter_map(|face| mesh.faces.get(face)),
    )
}

fn grid(mesh: &Mesh, perimeter: &[u64], corners: &Value) -> Result<PatchLayout, String> {
    let corners: [u64; 4] = serde_json::from_value(corners.clone())
        .map_err(|_| "grid_corners must contain four boundary vertex IDs")?;
    let mut indices = [0; 4];
    for (index, vertex) in corners.iter().enumerate() {
        indices[index] = perimeter
            .iter()
            .position(|id| id == vertex)
            .ok_or("grid corners must belong to the patch boundary")?;
    }
    let size = perimeter.len();
    let spans: [usize; 4] =
        std::array::from_fn(|i| (indices[(i + 1) % 4] + size - indices[i]) % size);
    let [columns, rows, opposite_columns, opposite_rows] = spans;
    if spans.contains(&0) || spans.iter().sum::<usize>() != size {
        return Err("grid corners must be distinct and follow boundary winding".into());
    }
    if columns != opposite_columns || rows != opposite_rows {
        return Err("opposing grid sides must have matching edge counts".into());
    }
    if columns.saturating_mul(rows) > 4096 {
        return Err("grid exceeds the patch face budget (4096)".into());
    }

    let at = |corner: usize, step: usize| (indices[corner] + step) % size;
    let point = |index: usize| mesh.vertices[&perimeter[index]];
    let corners = indices.map(point);
    let mut interior = Vec::new();
    let mut lattice = vec![vec![0; columns + 1]; rows + 1];
    for (row, column_indices) in lattice.iter_mut().enumerate() {
        for (column, index) in column_indices.iter_mut().enumerate() {
            *index = match (row, column) {
                (0, _) => at(0, column),
                (r, _) if r == rows => at(2, columns - column),
                (_, 0) => at(3, rows - row),
                (_, c) if c == columns => at(1, row),
                _ => {
                    let u = column as f64 / columns as f64;
                    let v = row as f64 / rows as f64;
                    let bottom = point(at(0, column));
                    let right = point(at(1, row));
                    let top = point(at(2, columns - column));
                    let left = point(at(3, rows - row));
                    let position = std::array::from_fn(|axis| {
                        let corner_blend = (1.0 - u)
                            * ((1.0 - v) * corners[0][axis] + v * corners[3][axis])
                            + u * ((1.0 - v) * corners[1][axis] + v * corners[2][axis]);
                        (1.0 - u) * left[axis]
                            + u * right[axis]
                            + (1.0 - v) * bottom[axis]
                            + v * top[axis]
                            - corner_blend
                    });
                    let index = size + interior.len();
                    interior.push(position);
                    index
                }
            };
        }
    }
    let mut polygons = Vec::new();
    for pair in lattice.windows(2) {
        for column in 0..columns {
            polygons.push(vec![
                pair[0][column],
                pair[0][column + 1],
                pair[1][column + 1],
                pair[1][column],
            ]);
        }
    }
    Ok((interior, polygons))
}

pub(super) fn replace(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    topology::validate_topology(mesh)?;
    let (object, faces, perimeter) = source(mesh, request)?;
    if request["boundary"] != json!(perimeter) {
        return Err("boundary must match the ordered IDs from inspect_patch".into());
    }
    let (interior, polygons): PatchLayout = if let Some(corners) = request.get("grid_corners") {
        if request.get("polygons").is_some() || request.get("interior").is_some() {
            return Err("grid_corners cannot be combined with polygons or interior".into());
        }
        grid(mesh, &perimeter, corners)?
    } else {
        let points = request.get("interior").map_or(Ok(vec![]), |value| {
            serde_json::from_value(value.clone())
                .map_err(|_| "interior must be an array of 3D points")
        })?;
        let polygons = serde_json::from_value(request["polygons"].clone())
            .map_err(|_| "polygons must contain zero-based corner indices")?;
        (points, polygons)
    };
    if interior.len() > 4096 || polygons.is_empty() || polygons.len() > 4096 {
        return Err("patch requires 1..4096 polygons and at most 4096 interior points".into());
    }

    let flat = mesh.flat_faces.contains(faces.first().unwrap());
    if faces.iter().any(|id| {
        mesh.uvs.contains_key(id)
            || mesh.face_textures.contains_key(id)
            || mesh.flat_faces.contains(id) != flat
    }) {
        return Err("patch requires unmapped faces with uniform shading".into());
    }
    let boundary_vertices: BTreeSet<_> = perimeter.iter().copied().collect();
    let old_vertices = construction::vertices(mesh, &faces);
    let retired_vertices: BTreeSet<_> = old_vertices
        .difference(&boundary_vertices)
        .copied()
        .collect();
    if retired_vertices
        .iter()
        .any(|id| mesh.vertex_colors.contains_key(id))
    {
        return Err("patch would remove interior vertex colors".into());
    }
    if !interior.is_empty()
        && old_vertices
            .iter()
            .any(|id| mesh.vertex_colors.contains_key(id))
    {
        return Err("patch would add unpainted vertices to a painted region".into());
    }
    if mesh
        .components
        .values()
        .any(|component| !faces.is_disjoint(&component.faces) && !faces.is_subset(&component.faces))
    {
        return Err("replacement crosses a named region boundary".into());
    }
    if mesh.faces.iter().any(|(id, polygon)| {
        !faces.contains(id) && polygon.iter().any(|v| retired_vertices.contains(v))
    }) {
        return Err("patch interior is shared by faces outside the patch".into());
    }

    let mut corners = perimeter.clone();
    for point in interior {
        corners.push(mesh.add_vertex(point)?);
    }
    let mut replacement = Vec::new();
    let mut used = BTreeSet::new();
    for polygon in polygons {
        if !(3..=64).contains(&polygon.len()) {
            return Err("each replacement polygon requires 3..64 corners".into());
        }
        let mut face = Vec::new();
        for index in polygon {
            let vertex = *corners
                .get(index)
                .ok_or("patch corner index is out of range")?;
            face.push(vertex);
        }
        topology::convex_face(mesh, &face)?;
        used.extend(face.iter().copied());
        replacement.push(face);
    }
    if used.len() != corners.len() || boundary(&replacement)? != perimeter {
        return Err("replacement must use every point and preserve the directed boundary".into());
    }

    // Allocate before removing the originals so legacy projects keep the same ID guarantees.
    let mut created = BTreeSet::new();
    for face in replacement {
        let id = mesh.add_face(face)?;
        if flat {
            mesh.flat_faces.insert(id);
        }
        created.insert(id);
    }
    for component in mesh.components.values_mut() {
        if faces.is_subset(&component.faces) {
            component.faces.extend(&created);
        }
    }
    mesh.faces.retain(|id, _| !faces.contains(id));
    mesh.vertices.retain(|id, _| !retired_vertices.contains(id));
    let owner = mesh.objects.get_mut(&object).unwrap();
    owner.vertices.retain(|id| !retired_vertices.contains(id));
    owner.vertices.extend(&corners[perimeter.len()..]);
    mesh.selection.retain(|id| !retired_vertices.contains(id));
    topology::validate_topology(mesh)?;
    Ok(
        json!({"object":object,"boundary":perimeter,"boundary_vertices_moved":0,"created_faces":created,"removed_faces":faces,"created_vertices":&corners[perimeter.len()..],"removed_vertices":retired_vertices}),
    )
}
