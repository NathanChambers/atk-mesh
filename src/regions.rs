use crate::{Mesh, normals, shaping, topology};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn subtract(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| a[axis] - b[axis])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn number(request: &Value, key: &str, default: f64) -> Result<f64, String> {
    let value = request.get(key).map_or(Ok(default), |v| {
        v.as_f64().ok_or_else(|| format!("{key} must be numeric"))
    })?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{key} must be finite and nonnegative"));
    }
    Ok(value)
}

fn edges(mesh: &Mesh) -> BTreeMap<(u64, u64), Vec<u64>> {
    let mut edges = BTreeMap::<_, Vec<_>>::new();
    for (id, face) in &mesh.faces {
        for i in 0..face.len() {
            let a = face[i];
            let b = face[(i + 1) % face.len()];
            edges.entry((a.min(b), a.max(b))).or_default().push(*id);
        }
    }
    edges
}

pub(super) fn selection(mesh: &Mesh, request: &Value) -> Result<BTreeSet<u64>, String> {
    let mut faces: BTreeSet<u64> = request.get("faces").map_or(Ok(BTreeSet::new()), |v| {
        serde_json::from_value(v.clone()).map_err(|_| "faces must be an array of IDs")
    })?;
    if let Some(value) = request.get("face") {
        faces.insert(value.as_u64().ok_or("face must be an ID")?);
    }
    let object = request
        .get("object")
        .map(|value| {
            let id = value.as_u64().ok_or("object must be an ID")?;
            mesh.objects.get(&id).ok_or("unknown object")
        })
        .transpose()?;
    if faces.is_empty() {
        let object = object.ok_or("provide face, faces or object")?;
        faces.extend(
            mesh.faces
                .iter()
                .filter(|(_, face)| object.vertices.contains(&face[0]))
                .map(|(id, _)| *id),
        );
    }
    for id in &faces {
        let face = mesh.faces.get(id).ok_or("unknown region face")?;
        if object.is_some_and(|object| !object.vertices.contains(&face[0])) {
            return Err("region lies outside the requested object".into());
        }
    }
    if let Some(value) = request.get("coplanar") {
        let grow = value.as_bool().ok_or("coplanar must be boolean")?;
        if grow {
            if faces.len() != 1 {
                return Err("coplanar growth requires one seed face".into());
            }
            let seed = *faces.first().unwrap();
            let normal = normals::face(mesh, &mesh.faces[&seed]);
            let origin = mesh.vertices[&mesh.faces[&seed][0]];
            let tolerance = number(request, "tolerance", 1e-8)?;
            let edges = edges(mesh);
            let mut pending = vec![seed];
            while let Some(id) = pending.pop() {
                let face = &mesh.faces[&id];
                for i in 0..face.len() {
                    let a = face[i];
                    let b = face[(i + 1) % face.len()];
                    for neighbor in &edges[&(a.min(b), a.max(b))] {
                        if faces.contains(neighbor) {
                            continue;
                        }
                        let polygon = &mesh.faces[neighbor];
                        let aligned = dot(normal, normals::face(mesh, polygon)) > 1.0 - 1e-10;
                        let planar = polygon.iter().all(|v| {
                            dot(subtract(mesh.vertices[v], origin), normal).abs() <= tolerance
                        });
                        if aligned && planar {
                            faces.insert(*neighbor);
                            pending.push(*neighbor);
                        }
                    }
                }
            }
        }
    }
    Ok(faces)
}

fn quality(points: [[f64; 3]; 3]) -> f64 {
    let a = subtract(points[1], points[0]);
    let b = subtract(points[2], points[0]);
    let c = subtract(points[2], points[1]);
    let area = cross(a, b);
    let twice_area = dot(area, area).sqrt();
    2.0 * 3.0_f64.sqrt() * twice_area / (dot(a, a) + dot(b, b) + dot(c, c)).max(f64::MIN_POSITIVE)
}

pub(super) fn inspect(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let faces = selection(mesh, request)?;
    let threshold = number(request, "normal_threshold", 1.0)?;
    let quality_threshold = number(request, "quality_threshold", 0.1)?;
    if quality_threshold > 1.0 || threshold > 180.0 {
        return Err("quality_threshold must be 0..1 and normal_threshold must be 0..180".into());
    }
    let limit = request.get("limit").map_or(Ok(16), |v| {
        v.as_u64()
            .filter(|n| *n <= 256)
            .ok_or("limit must be 0..256")
    })? as usize;
    let corner_normals = normals::corners(mesh);
    let mut issues = vec![];
    let mut maximum_deviation = 0.0_f64;
    let mut maximum_planarity = 0.0_f64;
    let mut minimum_quality = 1.0_f64;
    let mut normal_count = 0;
    let mut quality_count = 0;
    for id in &faces {
        let face = &mesh.faces[id];
        let normal = normals::face(mesh, face);
        let origin = mesh.vertices[&face[0]];
        let planarity = face
            .iter()
            .map(|v| dot(subtract(mesh.vertices[v], origin), normal).abs())
            .fold(0.0_f64, f64::max);
        let deviation = corner_normals[id]
            .iter()
            .map(|n| dot(normal, *n).clamp(-1.0, 1.0).acos().to_degrees())
            .fold(0.0_f64, f64::max);
        let triangle_quality = (1..face.len() - 1)
            .map(|i| quality([origin, mesh.vertices[&face[i]], mesh.vertices[&face[i + 1]]]))
            .fold(1.0_f64, f64::min);
        maximum_deviation = maximum_deviation.max(deviation);
        maximum_planarity = maximum_planarity.max(planarity);
        minimum_quality = minimum_quality.min(triangle_quality);
        normal_count += usize::from(deviation > threshold);
        quality_count += usize::from(triangle_quality < quality_threshold);
        if deviation > threshold || triangle_quality < quality_threshold || planarity > 1e-8 {
            issues.push(json!({"face":id,"vertices":face,"normal_deviation_degrees":deviation,"triangle_quality":triangle_quality,"planarity_error":planarity}));
        }
    }
    issues.sort_by(|a, b| {
        b["normal_deviation_degrees"]
            .as_f64()
            .unwrap()
            .total_cmp(&a["normal_deviation_degrees"].as_f64().unwrap())
            .then_with(|| {
                a["triangle_quality"]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&b["triangle_quality"].as_f64().unwrap())
            })
    });
    let issue_count = issues.len();
    issues.truncate(limit);
    Ok(
        json!({"revision":revision,"face_count":faces.len(),"max_normal_deviation_degrees":maximum_deviation,"max_face_planarity_error":maximum_planarity,"min_triangle_quality":minimum_quality,"normal_threshold":threshold,"quality_threshold":quality_threshold,"normal_warning_faces":normal_count,"low_quality_faces":quality_count,"issue_count":issue_count,"issues":issues,"omitted_issues":issue_count.saturating_sub(limit),"note":"Normal deviation is expected on curved smooth surfaces; inspect planar caps for unwanted bevel influence. Triangle quality ranges from 0 (degenerate) to 1 (equilateral)."}),
    )
}

pub(super) struct PlanarRegion {
    pub boundary: Vec<u64>,
    points: Vec<[f64; 2]>,
    origin: [f64; 3],
    normal: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
}

pub(super) fn planar_region(
    mesh: &Mesh,
    faces: &BTreeSet<u64>,
    request: &Value,
) -> Result<PlanarRegion, String> {
    let first = &mesh.faces[faces.first().ok_or("empty region")?];
    let normal = normals::face(mesh, first);
    let origin = mesh.vertices[&first[0]];
    let u = normals::unit(subtract(mesh.vertices[&first[1]], origin));
    let v = cross(normal, u);
    let tolerance = number(request, "tolerance", 1e-8)?;
    let mut incidence = BTreeMap::<_, Vec<_>>::new();
    for id in faces {
        let face = &mesh.faces[id];
        if dot(normal, normals::face(mesh, face)) < 1.0 - 1e-10
            || face
                .iter()
                .any(|id| dot(subtract(mesh.vertices[id], origin), normal).abs() > tolerance)
        {
            return Err("region must be planar and consistently wound".into());
        }
        for i in 0..face.len() {
            let a = face[i];
            let b = face[(i + 1) % face.len()];
            incidence
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push((a, b));
        }
    }
    let mut next = BTreeMap::new();
    for owners in incidence.values() {
        if owners.len() == 1 && next.insert(owners[0].0, owners[0].1).is_some() {
            return Err("region has a branching boundary".into());
        }
        if owners.len() > 2 {
            return Err("region is non-manifold".into());
        }
    }
    let start = *next.keys().next().ok_or("region has no boundary")?;
    let mut boundary = vec![start];
    let mut cursor = next[&start];
    while cursor != start {
        if boundary.contains(&cursor) || boundary.len() >= next.len() {
            return Err("region boundary is not a simple loop".into());
        }
        boundary.push(cursor);
        cursor = *next.get(&cursor).ok_or("region boundary is open")?;
    }
    if boundary.len() != next.len() {
        return Err(
            "region must have one boundary; holes and disconnected patches are unsupported".into(),
        );
    }
    let points: Vec<_> = boundary
        .iter()
        .map(|id| {
            let p = subtract(mesh.vertices[id], origin);
            [dot(p, u), dot(p, v)]
        })
        .collect();
    shaping::polygon_triangles(&points)?;
    Ok(PlanarRegion {
        boundary,
        points,
        origin,
        normal,
        u,
        v,
    })
}

fn improve(mesh: &Mesh, triangles: &mut [[u64; 3]], normal: [f64; 3]) -> usize {
    let mut flips = 0;
    for _ in 0..64 {
        let mut incidence = BTreeMap::<_, Vec<_>>::new();
        for (index, triangle) in triangles.iter().enumerate() {
            for i in 0..3 {
                let a = triangle[i];
                let b = triangle[(i + 1) % 3];
                incidence.entry((a.min(b), a.max(b))).or_default().push((
                    index,
                    a,
                    b,
                    triangle[(i + 2) % 3],
                ));
            }
        }
        let mut changed = false;
        let mut touched = BTreeSet::new();
        for owners in incidence.values().filter(|v| v.len() == 2) {
            let (i, a, b, c) = owners[0];
            let (j, _, _, d) = owners[1];
            if touched.contains(&i)
                || touched.contains(&j)
                || c == d
                || incidence.contains_key(&(c.min(d), c.max(d)))
            {
                continue;
            }
            let replacements = [[c, a, d], [c, d, b]];
            let valid = replacements.iter().all(|t| {
                dot(
                    cross(
                        subtract(mesh.vertices[&t[1]], mesh.vertices[&t[0]]),
                        subtract(mesh.vertices[&t[2]], mesh.vertices[&t[0]]),
                    ),
                    normal,
                ) > 0.0
            });
            let score = |t: [u64; 3]| quality(t.map(|id| mesh.vertices[&id]));
            let old = score(triangles[i]).min(score(triangles[j]));
            let new = score(replacements[0]).min(score(replacements[1]));
            if valid && new > old + 1e-10 {
                triangles[i] = replacements[0];
                triangles[j] = replacements[1];
                touched.extend([i, j]);
                flips += 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    flips
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    let faces = selection(mesh, request)?;
    if command == "shade_region" {
        let flat = match request["mode"].as_str() {
            Some("flat") => true,
            Some("smooth") => false,
            _ => return Err("mode must be flat or smooth".into()),
        };
        for id in &faces {
            if flat {
                mesh.flat_faces.insert(*id);
            } else {
                mesh.flat_faces.remove(id);
            }
        }
        return Ok(json!({"faces":faces,"mode":request["mode"],"geometry_changed":false}));
    }
    topology::validate_topology(mesh)?;
    let region = planar_region(mesh, &faces, request)?;
    let uniform_flat = faces.iter().all(|id| mesh.flat_faces.contains(id));
    if !uniform_flat && faces.iter().any(|id| mesh.flat_faces.contains(id)) {
        return Err("set a uniform shading mode before rebuilding region topology".into());
    }
    let mut created_vertices = vec![];
    let mut borders = vec![];
    let mut triangles = vec![];
    if command == "inset_region" {
        let vertices: BTreeSet<_> = faces
            .iter()
            .flat_map(|id| mesh.faces[id].iter().copied())
            .collect();
        if vertices.len() != region.boundary.len() {
            return Err(
                "support inset currently requires a region without interior vertices".into(),
            );
        }
        let distance = number(request, "distance", 0.0)?;
        if distance <= 0.0 {
            return Err("distance must be positive".into());
        }
        let mut points = vec![];
        for i in 0..region.points.len() {
            let previous = region.points[(i + region.points.len() - 1) % region.points.len()];
            let p = region.points[i];
            let next = region.points[(i + 1) % region.points.len()];
            let a = normals::unit([p[0] - previous[0], p[1] - previous[1], 0.0]);
            let b = normals::unit([next[0] - p[0], next[1] - p[1], 0.0]);
            let denominator = 1.0 + dot(a, b);
            if denominator < 1e-8 {
                return Err("inset corner reverses direction".into());
            }
            points.push([
                p[0] - distance * (a[1] + b[1]) / denominator,
                p[1] + distance * (a[0] + b[0]) / denominator,
            ]);
        }
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            let forward = (0..2)
                .map(|axis| {
                    (points[j][axis] - points[i][axis])
                        * (region.points[j][axis] - region.points[i][axis])
                })
                .sum::<f64>();
            if forward <= 0.0 {
                return Err("inset collapses an edge; reduce distance".into());
            }
        }
        let cap = shaping::polygon_triangles(&points)?;
        for point in &points {
            created_vertices.push(mesh.add_vertex(std::array::from_fn(|axis| {
                region.origin[axis] + region.u[axis] * point[0] + region.v[axis] * point[1]
            }))?);
        }
        triangles.extend(cap.iter().map(|t| t.map(|i| created_vertices[i])));
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            let border = vec![
                region.boundary[i],
                region.boundary[j],
                created_vertices[j],
                created_vertices[i],
            ];
            topology::convex_face(mesh, &border)?;
            borders.push(border);
        }
    } else {
        for id in &faces {
            let face = &mesh.faces[id];
            let points: Vec<_> = face
                .iter()
                .map(|v| {
                    let p = subtract(mesh.vertices[v], region.origin);
                    [dot(p, region.u), dot(p, region.v)]
                })
                .collect();
            triangles.extend(
                shaping::polygon_triangles(&points)?
                    .iter()
                    .map(|t| t.map(|i| face[i])),
            );
        }
    }
    let flips = improve(mesh, &mut triangles, region.normal);
    let mut replacement: Vec<Vec<u64>> = triangles.into_iter().map(|t| t.to_vec()).collect();
    replacement.extend(borders);
    let old: Vec<_> = faces.iter().copied().collect();
    let mut resulting_faces = vec![];
    let mut created_faces = vec![];
    for (i, face) in replacement.into_iter().enumerate() {
        let id = if let Some(id) = old.get(i) {
            mesh.faces.insert(*id, face);
            *id
        } else {
            let id = mesh.add_face(face)?;
            created_faces.push(id);
            id
        };
        if uniform_flat {
            mesh.flat_faces.insert(id);
        }
        resulting_faces.push(id);
    }
    let removed_faces: Vec<_> = old.iter().skip(resulting_faces.len()).copied().collect();
    for id in &removed_faces {
        mesh.faces.remove(id);
        mesh.flat_faces.remove(id);
    }
    topology::validate_topology(mesh)?;
    Ok(
        json!({"source_faces":faces,"faces":resulting_faces,"created_faces":created_faces,"removed_faces":removed_faces,"created_vertices":created_vertices,"boundary_vertices":region.boundary,"edge_flips":flips,"existing_vertices_moved":0}),
    )
}
