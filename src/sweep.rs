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
    let mut profile = sweep_profile(&request["profile"])?;
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
    if request["path"]
        .as_object()
        .is_some_and(|path| path.len() != 1)
    {
        return Err("path must specify exactly one sweep path descriptor".into());
    }
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
    } else if let Some(line) = request["path"].get("line") {
        let start = vector(&line["start"])?;
        let end = vector(&line["end"])?;
        let segments = path_segments(&line["segments"], "segments", 4096)?;
        (0..=segments)
            .map(|i| lerp(start, end, i as f64 / segments as f64))
            .collect()
    } else if let Some(bezier) = request["path"].get("bezier") {
        bezier_path(bezier)?
    } else if let Some(spline) = request["path"].get("spline") {
        spline_path(spline)?
    } else {
        serde_json::from_value(request["path"].clone())
            .map_err(|_| "path must be 3D points or a line, bezier, spline or helix descriptor")?
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
    let scales: Vec<[f64; 2]> = match (request.get("scales"), request.get("scale_keys")) {
        (Some(_), Some(_)) => return Err("use either scales or scale_keys".into()),
        (Some(scales), None) => serde_json::from_value(scales.clone())
            .map_err(|_| "scales must contain [u,v] factors")?,
        (None, Some(keys)) => sample_scales(keys, path.len())?,
        (None, None) => vec![[1.0, 1.0]; path.len()],
    };
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

fn sweep_profile(value: &Value) -> Result<Vec<[f64; 2]>, String> {
    let Some(ellipse) = value.get("ellipse") else {
        return serde_json::from_value(value.clone())
            .map_err(|_| "profile must contain [u,v] points or an ellipse descriptor".into());
    };
    let radii: [f64; 2] = serde_json::from_value(ellipse["radii"].clone())
        .map_err(|_| "ellipse radii must contain two positive numbers")?;
    if radii
        .iter()
        .any(|radius| !radius.is_finite() || *radius <= 0.0)
    {
        return Err("ellipse radii must be finite and positive".into());
    }
    let sides = ellipse["sides"]
        .as_u64()
        .filter(|sides| (3..=64).contains(sides))
        .ok_or("ellipse sides must be 3..64")?;
    Ok((0..sides)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / sides as f64;
            [radii[0] * angle.cos(), radii[1] * angle.sin()]
        })
        .collect())
}

fn lerp<const N: usize>(start: [f64; N], end: [f64; N], t: f64) -> [f64; N] {
    std::array::from_fn(|i| start[i] * (1.0 - t) + end[i] * t)
}

fn path_segments(value: &Value, field: &str, max: usize) -> Result<usize, String> {
    value
        .as_u64()
        .filter(|segments| (1..=max as u64).contains(segments))
        .map(|segments| segments as usize)
        .ok_or_else(|| format!("{field} must be 1..{max}"))
}

fn bezier_path(spec: &Value) -> Result<Vec<[f64; 3]>, String> {
    let start = vector(&spec["start"])?;
    let end = vector(&spec["end"])?;
    let controls = spec["controls"]
        .as_array()
        .filter(|controls| (1..=2).contains(&controls.len()))
        .ok_or("bezier controls must contain one or two 3D points")?;
    let first = vector(&controls[0])?;
    let second = controls.get(1).map(vector).transpose()?;
    let segments = path_segments(&spec["segments"], "segments", 4096)?;

    Ok((0..=segments)
        .map(|i| {
            let t = i as f64 / segments as f64;
            let a = lerp(start, first, t);
            if let Some(second) = second {
                let b = lerp(first, second, t);
                let c = lerp(second, end, t);
                lerp(lerp(a, b, t), lerp(b, c, t), t)
            } else {
                lerp(a, lerp(first, end, t), t)
            }
        })
        .collect())
}

fn spline_path(spec: &Value) -> Result<Vec<[f64; 3]>, String> {
    let points = spec["points"]
        .as_array()
        .filter(|points| (2..=4097).contains(&points.len()))
        .ok_or("spline points must contain 2..4097 waypoints")?;
    let spans = points.len() - 1;
    let segments = path_segments(
        &spec["segments_per_span"],
        "segments_per_span",
        4096 / spans,
    )?;
    let points = points.iter().map(vector).collect::<Result<Vec<_>, _>>()?;
    if points.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("spline waypoints must not repeat consecutively".into());
    }

    let mut path = Vec::with_capacity(spans * segments + 1);
    for i in 0..spans {
        let previous = points[i.saturating_sub(1)];
        let start = points[i];
        let end = points[i + 1];
        let next = points[(i + 2).min(spans)];
        path.push(start);
        for j in 1..segments {
            let t = j as f64 / segments as f64;
            let t2 = t * t;
            let t3 = t2 * t;
            path.push(std::array::from_fn(|axis| {
                let incoming = (end[axis] - previous[axis]) * 0.5;
                let outgoing = (next[axis] - start[axis]) * 0.5;
                (2.0 * t3 - 3.0 * t2 + 1.0) * start[axis]
                    + (t3 - 2.0 * t2 + t) * incoming
                    + (-2.0 * t3 + 3.0 * t2) * end[axis]
                    + (t3 - t2) * outgoing
            }));
        }
    }
    path.push(points[spans]);
    Ok(path)
}

fn sample_scales(value: &Value, ring_count: usize) -> Result<Vec<[f64; 2]>, String> {
    let keys: Vec<[f64; 3]> = serde_json::from_value(value.clone())
        .map_err(|_| "scale_keys must contain [t,u,v] entries")?;
    let invalid_key = keys.iter().any(|[t, u, v]| {
        !(0.0..=1.0).contains(t) || !u.is_finite() || !v.is_finite() || *u <= 0.0 || *v <= 0.0
    });
    if keys.is_empty() || invalid_key || keys.windows(2).any(|pair| pair[0][0] >= pair[1][0]) {
        return Err("scale_keys require increasing t in 0..1 and finite positive factors".into());
    }

    let mut next = 1;
    let mut scales = Vec::with_capacity(ring_count);
    for i in 0..ring_count {
        let t = i as f64 / (ring_count - 1) as f64;
        while next < keys.len() && t > keys[next][0] {
            next += 1;
        }
        let [start, u, v] = keys[next - 1];
        let scale = match keys.get(next) {
            Some(&[end, end_u, end_v]) if t > start => {
                lerp([u, v], [end_u, end_v], (t - start) / (end - start))
            }
            _ => [u, v],
        };
        scales.push(scale);
    }
    Ok(scales)
}
