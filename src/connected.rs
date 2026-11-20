use crate::{Mesh, geometry::*, normals, topology};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn loft(mesh: &mut Mesh, request: &Value, strip: bool) -> Result<Value, String> {
    if request.get("contours").is_some() == request.get("loops").is_some() {
        return Err(
            "provide exactly one of contours (3D points) or loops (existing vertex IDs)".into(),
        );
    }
    let existing: Option<Vec<Vec<u64>>> = request
        .get("loops")
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|_| "loops must be ordered arrays of existing vertex IDs")
        })
        .transpose()?;
    let contours: Vec<Vec<[f64; 3]>> = if let Some(loops) = &existing {
        let object = crate::surface_ops::object(mesh, request)?;
        let mut seen = BTreeSet::new();
        let mut contours = vec![];
        for ring in loops {
            let mut points = vec![];
            for id in ring {
                if !mesh.objects[&object].vertices.contains(id) || !seen.insert(*id) {
                    return Err("existing loops must have distinct vertices within object".into());
                }
                points.push(mesh.vertices[id]);
            }
            contours.push(points);
        }
        contours
    } else {
        serde_json::from_value(request["contours"].clone())
            .map_err(|_| "contours must be equally sized arrays of 3D points")?
    };
    let closed = request
        .get("closed")
        .map_or(Ok(!strip), |v| v.as_bool().ok_or("closed must be boolean"))?;
    let count = contours.first().map_or(0, Vec::len);
    if contours.len() < 2
        || contours.len() > 256
        || count < if closed { 3 } else { 2 }
        || count > 4096
        || contours.len() * count > 150000
        || contours
            .iter()
            .any(|c| c.len() != count || c.iter().flatten().any(|v| !v.is_finite()))
    {
        return Err("loft needs 2..256 compatible contours within 150000 vertices".into());
    }
    if strip && contours.len() != 2 {
        return Err("quad_strip requires exactly two paired contours".into());
    }
    let cap_start = request.get("cap_start").map_or(Ok(false), |v| {
        v.as_bool().ok_or("cap_start must be boolean")
    })?;
    let cap_end = request
        .get("cap_end")
        .map_or(Ok(false), |v| v.as_bool().ok_or("cap_end must be boolean"))?;
    if !closed && (cap_start || cap_end) {
        return Err("open contours cannot have end caps".into());
    }
    let object = crate::objects::destination_object(mesh, request)?;
    let mut rings = vec![];
    if let Some(existing) = existing {
        rings = existing;
    } else {
        for contour in contours {
            let mut ring = vec![];
            for point in contour {
                ring.push(mesh.add_vertex(point)?);
            }
            rings.push(ring);
        }
    }
    let mut faces = vec![];
    for pair in rings.windows(2) {
        for i in 0..if closed { count } else { count - 1 } {
            let j = (i + 1) % count;
            let quad = vec![pair[0][i], pair[0][j], pair[1][j], pair[1][i]];
            topology::convex_face(mesh, &quad)?;
            faces.push(mesh.add_face(quad)?);
        }
    }
    for (enabled, ring, reverse) in [
        (cap_start, &rings[0], true),
        (cap_end, rings.last().unwrap(), false),
    ] {
        if enabled {
            let mut cap = ring.clone();
            if reverse {
                cap.reverse();
            }
            let normal = normals::face(mesh, &cap);
            let origin = mesh.vertices[&cap[0]];
            let axis = (0..3)
                .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
                .unwrap();
            if cap
                .iter()
                .any(|id| dot(sub(mesh.vertices[id], origin), normal).abs() > 1e-8)
            {
                return Err("loft end cap must be planar".into());
            }
            let points: Vec<_> = cap
                .iter()
                .map(|id| {
                    [
                        mesh.vertices[id][(axis + 1) % 3],
                        mesh.vertices[id][(axis + 2) % 3],
                    ]
                })
                .collect();
            for triangle in crate::shaping::polygon_triangles(&points)? {
                faces.push(mesh.add_face(triangle.map(|i| cap[i]).to_vec())?);
            }
        }
    }
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(rings.iter().flatten());
    mesh.selection = rings.last().unwrap().iter().copied().collect();
    Ok(
        json!({"object":object,"rings":rings,"created_faces":faces,"closed":closed,"cap_start":cap_start,"cap_end":cap_end,"correspondence":"contour index order; no inferred rotation or reversal"}),
    )
}

pub(super) fn bevel(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let object = crate::surface_ops::object(mesh, request)?;
    let edges: Vec<[u64; 2]> = serde_json::from_value(request["edges"].clone())
        .map_err(|_| "edges must contain selected vertex pairs")?;
    let width = request["width"]
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or("width must be finite and positive")?;
    let segments = request.get("segments").map_or(Ok(1), |v| {
        v.as_u64()
            .filter(|v| *v > 0 && *v <= 32)
            .ok_or("segments must be 1..32")
    })? as usize;
    if request["corner_mode"].as_str() != Some("isolated") {
        return Err("corner_mode must explicitly be isolated; shared selected-edge corners require separate patch construction".into());
    }
    if edges.is_empty() || edges.len() > 128 {
        return Err("provide 1..128 isolated edges".into());
    }
    let mut touched = BTreeSet::new();
    for edge in &edges {
        for id in edge {
            if !mesh.objects[&object].vertices.contains(id) || !touched.insert(*id) {
                return Err("selected edges must be vertex-disjoint within object".into());
            }
        }
    }
    let mut created_faces = vec![];
    let mut created_vertices = vec![];
    for [a, b] in edges {
        let incidence = crate::surface_ops::edges(mesh, Some(object));
        let incident = incidence
            .get(&(a.min(b), a.max(b)))
            .filter(|v| v.len() == 2)
            .ok_or("bevel requires a two-face manifold edge")?;
        let f = incident[0].0;
        let g = incident[1].0;
        let mut rings = vec![];
        let mut third_faces = vec![];
        let direction = normals::unit(sub(mesh.vertices[&b], mesh.vertices[&a]));
        let mut edge_neighbors = BTreeMap::new();
        for vertex in [a, b] {
            let containing: Vec<_> = mesh
                .faces
                .iter()
                .filter(|(_, face)| face.contains(&vertex))
                .map(|(id, _)| *id)
                .collect();
            if containing.len() != 3 {
                return Err("isolated bevel endpoints must each have three incident faces".into());
            }
            let third = *containing.iter().find(|id| **id != f && **id != g).unwrap();
            third_faces.push(third);
            let origin = mesh.vertices[&vertex];
            let mut rays = vec![];
            for face_id in [f, g] {
                let face = &mesh.faces[&face_id];
                topology::convex_face(mesh, face)?;
                let index = face.iter().position(|id| *id == vertex).unwrap();
                let previous = face[(index + face.len() - 1) % face.len()];
                let next = face[(index + 1) % face.len()];
                let other = if [a, b].contains(&previous) {
                    next
                } else {
                    previous
                };
                edge_neighbors.insert((vertex, face_id), other);
                let toward = sub(mesh.vertices[&other], origin);
                let radial = sub(toward, mul(direction, dot(toward, direction)));
                let radial_length = length(radial);
                if radial_length <= width {
                    return Err("bevel width reaches an adjacent edge endpoint".into());
                }
                rays.push(mul(radial, 1.0 / radial_length));
            }
            let cosine = dot(rays[0], rays[1]);
            if cosine <= -0.999999 || cosine >= 0.999999 {
                return Err("bevel requires a nonflat convex edge".into());
            }
            let n_f = normals::face(mesh, &mesh.faces[&f]);
            if dot(n_f, rays[1]) >= -1e-9 {
                return Err("bevel supports convex edges only".into());
            }
            let center = mul(add(rays[0], rays[1]), width / (1.0 + cosine));
            let u = sub(mul(rays[0], width), center);
            let v = sub(mul(rays[1], width), center);
            let radius = length(u);
            let angle = (dot(u, v) / (radius * radius)).clamp(-1.0, 1.0).acos();
            let normal = normals::face(mesh, &mesh.faces[&third]);
            let denominator = dot(direction, normal);
            if denominator.abs() < 1e-8 {
                return Err("bevel endpoint plane is parallel to edge".into());
            }
            let mut ring = vec![];
            for segment in 0..=segments {
                let t = segment as f64 / segments as f64;
                let radial = if segments == 1 {
                    lerp(mul(rays[0], width), mul(rays[1], width), t)
                } else {
                    add(
                        center,
                        add(
                            mul(u, ((1.0 - t) * angle).sin() / angle.sin()),
                            mul(v, (t * angle).sin() / angle.sin()),
                        ),
                    )
                };
                let offset = sub(radial, mul(direction, dot(radial, normal) / denominator));
                let id = mesh.add_vertex(add(origin, offset))?;
                ring.push(id);
                created_vertices.push(id);
                if let Some(color) = mesh.vertex_colors.get(&vertex).copied() {
                    mesh.vertex_colors.insert(id, color);
                }
            }
            rings.push(ring);
        }
        for (endpoint, vertex) in [a, b].into_iter().enumerate() {
            for face_id in [f, g] {
                let index = mesh.faces[&face_id]
                    .iter()
                    .position(|id| *id == vertex)
                    .unwrap();
                let target = if face_id == f {
                    rings[endpoint][0]
                } else {
                    rings[endpoint][segments]
                };
                mesh.faces.get_mut(&face_id).unwrap()[index] = target;
                if let Some(values) = mesh.uvs.get_mut(&face_id) {
                    if let Some(uv) = values.remove(&vertex) {
                        values.insert(target, uv);
                    }
                }
            }
            let third = third_faces[endpoint];
            let face = mesh.faces[&third].clone();
            let index = face.iter().position(|id| *id == vertex).unwrap();
            let previous = face[(index + face.len() - 1) % face.len()];
            let mut replacement = rings[endpoint].clone();
            if previous != edge_neighbors[&(vertex, f)] {
                replacement.reverse();
            }
            let mut updated = face[..index].to_vec();
            updated.extend(&replacement);
            updated.extend(&face[index + 1..]);
            mesh.faces.insert(third, updated);
            if let Some(values) = mesh.uvs.get_mut(&third) {
                if let Some(uv) = values.remove(&vertex) {
                    for id in replacement {
                        values.insert(id, uv);
                    }
                }
            }
        }
        let f_direction = incident[0].1 == a;
        for i in 0..segments {
            let mut quad = vec![rings[0][i], rings[1][i], rings[1][i + 1], rings[0][i + 1]];
            if f_direction {
                quad.reverse();
            }
            topology::convex_face(mesh, &quad)?;
            let id = mesh.add_face(quad)?;
            created_faces.push(id);
        }
        for vertex in [a, b] {
            mesh.vertices.remove(&vertex);
            mesh.selection.remove(&vertex);
        }
        mesh.objects
            .get_mut(&object)
            .unwrap()
            .vertices
            .extend(rings.iter().flatten());
        for component in mesh.components.values_mut() {
            if component.faces.contains(&f) && component.faces.contains(&g) {
                component.faces.extend(&created_faces);
            }
        }
    }
    mesh.selection = created_vertices.iter().copied().collect();
    crate::uv::reconcile(mesh);
    topology::validate_topology(mesh)?;
    Ok(
        json!({"object":object,"created_faces":created_faces,"created_vertices":created_vertices,"width":width,"segments":segments,"corner_mode":"isolated","appearance":"new bevel strips have no UV or texture binding; existing affected UV corners copy endpoint values and should be reviewed"}),
    )
}
