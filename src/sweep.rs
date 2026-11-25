use crate::{Mesh, normals, objects, shaping, vector};
use serde_json::{Value, json};

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn create(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let quads = match request["side_faces"].as_str().unwrap_or("triangles") {
        "triangles" => false,
        "quads" => true,
        _ => return Err("side_faces must be triangles or quads".into()),
    };
    let cap_start = request
        .get("cap_start")
        .map_or(Ok(true), |v| v.as_bool().ok_or("cap_start must be boolean"))?;
    let cap_end = request
        .get("cap_end")
        .map_or(Ok(true), |v| v.as_bool().ok_or("cap_end must be boolean"))?;
    let mut profile: Vec<[f64; 2]> = serde_json::from_value(request["profile"].clone())
        .map_err(|_| "profile must contain [u,v] points")?;
    if profile.len() > 64 {
        return Err("sweep profile supports at most 64 points".into());
    }
    shaping::polygon_triangles(&profile)?;
    let area: f64 = (0..profile.len())
        .map(|i| {
            profile[i][0] * profile[(i + 1) % profile.len()][1]
                - profile[(i + 1) % profile.len()][0] * profile[i][1]
        })
        .sum();
    if area < 0.0 {
        profile.reverse();
    }
    let mut radial = vec![];
    let path: Vec<[f64; 3]> = if let Some(helix) = request["path"].get("helix") {
        let radius: [f64; 2] = serde_json::from_value(helix["radius"].clone())
            .map_err(|_| "helix radius must contain two positive radii")?;
        let center = helix.get("center").map(vector).unwrap_or(Ok([0.0; 3]))?;
        let height = helix["height"].as_f64().ok_or("helix height is required")?;
        let turns = helix["turns"].as_f64().ok_or("helix turns is required")?;
        let start = helix
            .get("start")
            .map_or(Ok(0.0), |v| v.as_f64().ok_or("helix start must be numeric"))?;
        let steps = helix["steps"]
            .as_u64()
            .filter(|v| (8..=4096).contains(v))
            .ok_or("helix steps must be 8..4096")? as usize;
        if radius.iter().any(|v| !v.is_finite() || *v <= 0.0)
            || !height.is_finite()
            || height.abs() < 1e-10
            || !turns.is_finite()
            || turns.abs() < 0.01
            || turns.abs() > 64.0
            || !start.is_finite()
            || steps as f64 / turns.abs() < 12.0
        {
            return Err("helix requires finite positive radii, nonzero height, 0.01..64 turns and at least 12 steps per turn".into());
        }
        let axis_name = request
            .get("axis")
            .map_or(Ok("Y"), |v| v.as_str().ok_or("axis must be X, Y or Z"))?;
        let axis = match axis_name {
            "X" => 0,
            "Y" => 1,
            "Z" => 2,
            _ => return Err("axis must be X, Y or Z".into()),
        };
        let a = (axis + 1) % 3;
        let b = (axis + 2) % 3;
        let mut points = vec![];
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let angle = start.to_radians() + t * turns * std::f64::consts::TAU;
            let mut p = center;
            p[axis] += height * t;
            p[a] += radius[0] * angle.cos();
            p[b] += radius[1] * angle.sin();
            points.push(p);
            let mut outward = [0.0; 3];
            outward[a] = angle.cos() / radius[0];
            outward[b] = angle.sin() / radius[1];
            radial.push(normals::unit(outward));
        }
        points
    } else {
        serde_json::from_value(request["path"].clone())
            .map_err(|_| "path must be 3D points or a helix descriptor")?
    };
    if path.len() < 2
        || path.len() > 4097
        || path.len() * profile.len() > 150000
        || path.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(
            "sweep requires 2..4097 finite path points within the 150000 vertex budget".into(),
        );
    }
    let scales: Vec<[f64; 2]> = request
        .get("scales")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "scales must contain [u,v] factors"))
        .transpose()?
        .unwrap_or_else(|| vec![[1.0, 1.0]; path.len()]);
    if scales.len() != path.len() || scales.iter().flatten().any(|v| !v.is_finite() || *v <= 0.0) {
        return Err("scales must provide two finite positive factors per path point".into());
    }
    let mut up = request
        .get("up")
        .map(vector)
        .unwrap_or(Ok([1.0, 0.0, 0.0]))?;
    let mut frames = vec![];
    let mut tangents = vec![];
    for i in 0..path.len() {
        let previous = path[i.saturating_sub(1)];
        let next = path[(i + 1).min(path.len() - 1)];
        let direction = std::array::from_fn(|a| next[a] - previous[a]);
        let tangent = normals::unit(direction);
        if tangent == [0.0; 3] || (i > 0 && path[i] == path[i - 1]) {
            return Err("sweep path repeats or reverses at a point".into());
        }
        if !radial.is_empty() {
            up = radial[i];
        }
        let projection = dot(up, tangent);
        let u = normals::unit(std::array::from_fn(|a| up[a] - projection * tangent[a]));
        if dot(u, u) < 0.5 {
            return Err("sweep up vector is parallel to the path; choose a different up".into());
        }
        let v = cross(tangent, u);
        up = u;
        frames.push((u, v));
        tangents.push(tangent);
    }
    for pair in tangents.windows(2) {
        if dot(pair[0], pair[1]) < -0.99 {
            return Err("sweep path turns back on itself".into());
        }
    }
    let object = objects::destination_object(mesh, request)?;
    let mut rings = vec![];
    for (i, point) in path.iter().enumerate() {
        let (u, v) = frames[i];
        let mut ring = vec![];
        for p in &profile {
            ring.push(mesh.add_vertex(std::array::from_fn(|a| {
                point[a] + u[a] * p[0] * scales[i][0] + v[a] * p[1] * scales[i][1]
            }))?);
        }
        rings.push(ring);
    }
    let mut created_faces = 0;
    for (section, pair) in rings.windows(2).enumerate() {
        let left = scales[section];
        let right = scales[section + 1];
        let anisotropic_taper = left[0] * right[1] != left[1] * right[0];
        for i in 0..profile.len() {
            let j = (i + 1) % profile.len();
            if quads {
                let face = vec![pair[0][i], pair[0][j], pair[1][j], pair[1][i]];
                crate::topology::convex_face(mesh, &face)?;
                mesh.add_face(face)?;
                created_faces += 1;
            } else {
                let [a, b, c, d] = [pair[0][i], pair[0][j], pair[1][j], pair[1][i]];
                let distance = |first: u64, second: u64| {
                    crate::geometry::length(crate::geometry::sub(
                        mesh.vertices[&first],
                        mesh.vertices[&second],
                    ))
                };
                // On narrow concave sections the long diagonal can pass through the opposite wall.
                if mesh.modern_material_defaults
                    && anisotropic_taper
                    && distance(b, d) < distance(a, c)
                {
                    mesh.add_face(vec![a, b, d])?;
                    mesh.add_face(vec![b, c, d])?;
                } else {
                    mesh.add_face(vec![a, b, c])?;
                    mesh.add_face(vec![a, c, d])?;
                }
                created_faces += 2;
            }
        }
    }
    let last = rings.len() - 1;
    for t in shaping::polygon_triangles(&profile)? {
        if cap_start {
            mesh.add_face(t.iter().rev().map(|i| rings[0][*i]).collect())?;
            created_faces += 1;
        }
        if cap_end {
            mesh.add_face(t.iter().map(|i| rings[last][*i]).collect())?;
            created_faces += 1;
        }
    }
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(rings.iter().flatten());
    mesh.selection = rings[last].iter().copied().collect();
    let rings: Vec<_> = rings
        .iter()
        .map(|r| json!({"first_vertex":r[0],"count":r.len()}))
        .collect();
    let mut operation = json!({"object":object,"rings":rings,"created_faces":created_faces,"path_points":path.len(),"profile_points":profile.len(),"capped":cap_start && cap_end,"cap_start":cap_start,"cap_end":cap_end,"side_faces":if quads{"quads"}else{"triangles"},"self_intersections_checked":false});
    if request.get("uv").is_some() {
        operation["uv_path"] = json!(path);
    }
    Ok(operation)
}
