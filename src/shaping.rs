use crate::{Mesh, objects, topology, vector};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn number(request: &Value, key: &str, default: f64) -> Result<f64, String> {
    let value = match request.get(key) {
        Some(value) => value
            .as_f64()
            .ok_or_else(|| format!("{key} must be numeric"))?,
        None => default,
    };
    if !value.is_finite() {
        return Err(format!("{key} must be finite"));
    }
    Ok(value)
}

fn axis(request: &Value) -> Result<usize, String> {
    let value = match request.get("axis") {
        Some(value) => value.as_str().ok_or("axis must be X, Y or Z")?,
        None => "Y",
    };
    match value {
        "X" => Ok(0),
        "Y" => Ok(1),
        "Z" => Ok(2),
        _ => Err("axis must be X, Y or Z".into()),
    }
}

fn center(request: &Value) -> Result<[f64; 3], String> {
    request.get("center").map(vector).unwrap_or(Ok([0.0; 3]))
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    topology::validate_topology(mesh)?;
    let result = match command {
        "taper" => {
            let axis = axis(request)?;
            let center = center(request)?;
            let samples: Vec<[f64; 3]> = serde_json::from_value(request["profile"].clone())
                .map_err(
                    |_| "profile must contain [axial_position,first_scale,second_scale] samples",
                )?;
            let invalid = samples.len() < 2
                || samples.len() > 128
                || samples.iter().flatten().any(|v| !v.is_finite())
                || samples.iter().any(|v| v[1] <= 0.0 || v[2] <= 0.0)
                || samples.windows(2).any(|p| p[0][0] >= p[1][0]);
            if invalid {
                return Err(
                    "taper requires 2..128 increasing finite samples with positive scales".into(),
                );
            }
            if mesh.selection.is_empty() {
                return Err("selection is empty".into());
            }
            let mut changed = vec![];
            for id in &mesh.selection {
                let point = mesh.vertices.get_mut(id).unwrap();
                let coordinate = point[axis] - center[axis];
                let Some(pair) = samples
                    .windows(2)
                    .find(|p| coordinate >= p[0][0] && coordinate <= p[1][0])
                else {
                    continue;
                };
                let t = (coordinate - pair[0][0]) / (pair[1][0] - pair[0][0]);
                for (component, (&first, &second)) in
                    pair[0].iter().zip(&pair[1]).enumerate().skip(1)
                {
                    let radial = (axis + component) % 3;
                    let scale = first + t * (second - first);
                    point[radial] = center[radial] + (point[radial] - center[radial]) * scale;
                }
                changed.push(*id);
            }
            json!({"vertices":changed,"axis":request["axis"],"profile":samples})
        }
        "sweep" => crate::sweep::create(mesh, request)?,
        "revolve" => revolve(mesh, request)?,
        "extrude_polygon" => extrude_polygon(mesh, request)?,
        "bisect_plane" => {
            if let Some(offsets) = request.get("offsets") {
                let offsets: Vec<f64> = serde_json::from_value(offsets.clone())
                    .map_err(|_| "offsets must be a list of plane distances")?;
                if offsets.is_empty()
                    || offsets.len() > 32
                    || offsets.iter().any(|v| !v.is_finite())
                {
                    return Err("offsets requires 1..32 finite plane distances".into());
                }
                let mut cuts = vec![];
                for offset in offsets {
                    let mut cut = request.clone();
                    cut["offset"] = json!(offset);
                    cuts.push(bisect(mesh, &cut)?);
                }
                json!({"cuts":cuts})
            } else {
                bisect(mesh, request)?
            }
        }
        "triangulate" => triangulate(mesh, request)?,
        "select_loop" => {
            let result = inspect_loop(mesh, request)?;
            mesh.selection =
                serde_json::from_value(result["vertices"].clone()).map_err(|e| e.to_string())?;
            result
        }
        "radial_offset" | "radial_profile" => {
            let axis = axis(request)?;
            let center = center(request)?;
            let distance = number(request, "distance", 0.0)?;
            let function = if command == "radial_profile" {
                request["profile"]
                    .get("taper")
                    .map(RadialTaper::parse)
                    .transpose()?
            } else {
                None
            };
            let samples: Vec<[f64; 2]> = if command == "radial_profile" && function.is_none() {
                let samples: Vec<[f64; 2]> = serde_json::from_value(request["profile"].clone())
                    .map_err(|_| "profile must contain [axial_position,radial_offset] samples")?;
                if samples.len() < 2
                    || samples.len() > 128
                    || samples.iter().flatten().any(|n| !n.is_finite())
                    || samples.windows(2).any(|pair| pair[0][0] >= pair[1][0])
                {
                    return Err(
                        "profile requires 2..128 finite samples in increasing axial order".into(),
                    );
                }
                samples
            } else {
                vec![]
            };
            if mesh.selection.is_empty() {
                return Err("selection is empty".into());
            }
            let mut changed = vec![];
            for id in &mesh.selection {
                let point = mesh.vertices.get_mut(id).unwrap();
                let coordinate = point[axis] - center[axis];
                let distance = if let Some(function) = &function {
                    function.offset(coordinate)
                } else if samples.is_empty() {
                    distance
                } else {
                    samples
                        .windows(2)
                        .find(|pair| {
                            coordinate >= pair[0][0] - 1e-10 && coordinate <= pair[1][0] + 1e-10
                        })
                        .map(|pair| {
                            let t = ((coordinate - pair[0][0]) / (pair[1][0] - pair[0][0]))
                                .clamp(0.0, 1.0);
                            pair[0][1] + t * (pair[1][1] - pair[0][1])
                        })
                        .unwrap_or(0.0)
                };
                if distance.abs() < 1e-12 {
                    continue;
                }
                let a = (axis + 1) % 3;
                let b = (axis + 2) % 3;
                let radius = (point[a] - center[a]).hypot(point[b] - center[b]);
                if radius <= 1e-12 || radius + distance <= 1e-12 {
                    return Err("radial offset would cross the axis".into());
                }
                for i in [a, b] {
                    point[i] = center[i] + (point[i] - center[i]) * (radius + distance) / radius;
                }
                changed.push(*id);
            }
            if command == "radial_profile" {
                json!({"changed_vertices":changed,"profile":if function.is_some() { request["profile"].clone() } else { json!(samples) }})
            } else {
                json!({"changed_vertices":changed,"distance":distance})
            }
        }
        "shade_smooth" => {
            let id = request["object"].as_u64().ok_or("object is required")?;
            let angle = number(request, "angle", 60.0)?;
            if !(0.0..=180.0).contains(&angle) {
                return Err("angle must be between 0 and 180".into());
            }
            mesh.objects
                .get_mut(&id)
                .ok_or("unknown object")?
                .smooth_angle = Some(angle);
            json!({"object":id,"angle":angle})
        }
        _ => return Err("unknown shaping command".into()),
    };
    topology::validate_topology(mesh)?;
    Ok(result)
}

struct RadialTaper {
    start: f64,
    span: f64,
    offsets: [f64; 2],
    amplitude: f64,
    cycles: u64,
}

impl RadialTaper {
    fn parse(spec: &Value) -> Result<Self, String> {
        let [start, end]: [f64; 2] = serde_json::from_value(spec["range"].clone())
            .map_err(|_| "taper range must contain two axial positions")?;
        let span = end - start;
        if !span.is_finite() || span <= 0.0 {
            return Err("taper range must be finite and increasing".into());
        }
        let offsets: [f64; 2] = serde_json::from_value(spec["offsets"].clone())
            .map_err(|_| "taper offsets must contain two radial offsets")?;
        let mut amplitude = 0.0;
        let mut cycles = 0;
        if let Some(ridge) = spec.get("ridge") {
            amplitude = ridge["amplitude"]
                .as_f64()
                .filter(|amplitude| amplitude.is_finite() && *amplitude >= 0.0)
                .ok_or("ridge amplitude must be finite and nonnegative")?;
            cycles = ridge["cycles"]
                .as_u64()
                .filter(|cycles| (1..=4096).contains(cycles))
                .ok_or("ridge cycles must be 1..4096")?;
        }
        if offsets
            .iter()
            .any(|offset| !offset.is_finite() || !(offset + amplitude).is_finite())
        {
            return Err("taper offsets and peak offsets must be finite".into());
        }
        Ok(Self {
            start,
            span,
            offsets,
            amplitude,
            cycles,
        })
    }

    fn offset(&self, coordinate: f64) -> f64 {
        let t = (coordinate - self.start) / self.span;
        if !(0.0..=1.0).contains(&t) {
            return 0.0;
        }
        let [from, to] = self.offsets;
        let phase = (t * self.cycles as f64).fract();
        let ridge = self.amplitude * (0.5 - 0.5 * (phase * std::f64::consts::TAU).cos());
        from * (1.0 - t) + to * t + ridge
    }
}

pub(super) fn inspect_loop(mesh: &Mesh, request: &Value) -> Result<Value, String> {
    let [start, next]: [u64; 2] = serde_json::from_value(request["edge"].clone())
        .map_err(|_| "edge must contain two vertex IDs")?;
    let mut neighbors = BTreeMap::<u64, BTreeSet<u64>>::new();
    let mut edge_faces = BTreeMap::<(u64, u64), BTreeSet<u64>>::new();
    for (id, face) in &mesh.faces {
        for i in 0..face.len() {
            let a = face[i];
            let b = face[(i + 1) % face.len()];
            neighbors.entry(a).or_default().insert(b);
            neighbors.entry(b).or_default().insert(a);
            edge_faces
                .entry((a.min(b), a.max(b)))
                .or_default()
                .insert(*id);
        }
    }
    if !edge_faces.contains_key(&(start.min(next), start.max(next))) {
        return Err("unknown edge".into());
    }
    let mut ordered = vec![start, next];
    loop {
        let previous = ordered[ordered.len() - 2];
        let current = *ordered.last().unwrap();
        let incoming = &edge_faces[&(previous.min(current), previous.max(current))];
        let candidates: Vec<_> = neighbors[&current]
            .iter()
            .copied()
            .filter(|candidate| {
                *candidate != previous
                    && incoming.is_disjoint(
                        &edge_faces[&(current.min(*candidate), current.max(*candidate))],
                    )
            })
            .collect();
        if candidates.len() != 1 {
            return Err("loop is ambiguous or ends at a pole, triangle or boundary; use a regular quad loop".into());
        }
        let candidate = candidates[0];
        if candidate == start {
            break;
        }
        if ordered.contains(&candidate) {
            return Err("loop intersects itself".into());
        }
        ordered.push(candidate);
    }
    Ok(json!({"vertices":ordered,"closed":true,"count":ordered.len()}))
}

fn sample_profile(request: &Value) -> Result<Vec<[f64; 2]>, String> {
    let input = request["profile"]
        .as_array()
        .ok_or("profile must be an array of [radius,height] points or arc descriptors")?;
    let mut points = vec![];
    for item in input {
        if item.is_array() {
            let point: [f64; 2] = serde_json::from_value(item.clone())
                .map_err(|_| "profile point must be [radius,height]")?;
            points.push(point);
        } else {
            let arc = &item["arc"];
            let center: [f64; 2] = serde_json::from_value(arc["center"].clone())
                .map_err(|_| "arc center must be [radius,height]")?;
            let radius = number(arc, "radius", 1.0)?;
            let start = number(arc, "start", 0.0)?.to_radians();
            let end = number(arc, "end", 90.0)?.to_radians();
            let steps = arc["steps"]
                .as_u64()
                .filter(|n| (1..=512).contains(n))
                .ok_or("arc steps must be from 1 to 512")?;
            if radius <= 0.0 || (end - start).abs() < 1e-12 {
                return Err("arc must have positive radius and nonzero sweep".into());
            }
            for i in 0..=steps {
                let angle = start + (end - start) * i as f64 / steps as f64;
                points.push([
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]);
            }
        }
    }
    for point in &mut points {
        if point[0].abs() < 1e-12 {
            point[0] = 0.0;
        }
    }
    points.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-10);
    if points.len() < 2
        || points.len() > 1024
        || points
            .iter()
            .any(|p| p[0] < 0.0 || p.iter().any(|n| !n.is_finite()))
    {
        return Err("profile requires 2..1024 finite points with nonnegative radii".into());
    }
    if points[1..points.len() - 1].iter().any(|p| p[0] == 0.0) {
        return Err("only profile endpoints may meet the axis".into());
    }
    Ok(points)
}

pub(super) fn revolve(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let points = sample_profile(request)?;
    let axis = axis(request)?;
    let center = center(request)?;
    let segments = request["segments"]
        .as_u64()
        .filter(|n| (8..=512).contains(n))
        .ok_or("segments must be from 8 to 512")? as usize;
    let sweep = number(request, "sweep", 360.0)?;
    let start = number(request, "start", 0.0)?.to_radians();
    if sweep <= 0.0 || sweep > 360.0 {
        return Err("sweep must be greater than 0 and at most 360".into());
    }
    if points.len() * segments > 150_000 {
        return Err("revolve exceeds the 150000 vertex budget".into());
    }
    let closed = sweep == 360.0;
    let closed_profile = match request.get("closed_profile") {
        Some(value) => value.as_bool().ok_or("closed_profile must be boolean")?,
        None => false,
    };
    if closed_profile && points.iter().any(|p| p[0] <= 0.0) {
        return Err("closed profiles must stay away from the revolution axis".into());
    }
    if !closed_profile && (points[0][0] != 0.0 || points.last().unwrap()[0] != 0.0) {
        return Err("profile endpoints must lie on the axis for a closed solid".into());
    }
    polygon_triangles(&points)?;
    let area: f64 = (0..points.len())
        .map(|i| {
            points[i][0] * points[(i + 1) % points.len()][1]
                - points[(i + 1) % points.len()][0] * points[i][1]
        })
        .sum();
    if area <= 0.0 {
        return Err("revolve profile must be counterclockwise in radius/height coordinates".into());
    }
    let object = objects::destination_object(mesh, request)?;
    let mut rings = vec![];
    let mut created = BTreeSet::new();
    let mut bands = vec![];
    let a = (axis + 1) % 3;
    let b = (axis + 2) % 3;
    for [radius, height] in &points {
        let mut ring = vec![];
        for j in 0..if *radius == 0.0 {
            1
        } else {
            segments + usize::from(!closed)
        } {
            let angle = start + sweep.to_radians() * j as f64 / segments as f64;
            let mut point = center;
            point[axis] += height;
            point[a] += radius * angle.cos();
            point[b] += radius * angle.sin();
            let id = mesh.add_vertex(point)?;
            created.insert(id);
            ring.push(id);
        }
        rings.push(ring);
    }
    for i in 0..rings.len() - usize::from(!closed_profile) {
        let pair = [&rings[i], &rings[(i + 1) % rings.len()]];
        let first = mesh
            .faces
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .max(mesh.face_id_high_watermark)
            + 1;
        for j in 0..segments {
            let next = if closed { (j + 1) % segments } else { j + 1 };
            let at = |ring: &Vec<u64>, index| ring[if ring.len() == 1 { 0 } else { index }];
            let mut face = vec![
                at(pair[0], j),
                at(pair[0], next),
                at(pair[1], next),
                at(pair[1], j),
            ];
            face.dedup();
            if face.first() == face.last() {
                face.pop();
            }
            mesh.add_face(face)?;
        }
        bands.push(json!({"first_face":first,"count":segments}));
    }
    if !closed {
        for end in [0, segments] {
            let ids: Vec<_> = rings
                .iter()
                .map(|ring| ring[if ring.len() == 1 { 0 } else { end }])
                .collect();
            for triangle in polygon_triangles(&points)? {
                let mut face: Vec<_> = triangle.iter().map(|i| ids[*i]).collect();
                if end == segments {
                    face.reverse();
                }
                mesh.add_face(face)?;
            }
        }
    }
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(&created);
    mesh.selection = created;
    let rings: Vec<_> = rings
        .iter()
        .map(|ring| json!({"first_vertex":ring[0],"count":ring.len()}))
        .collect();
    Ok(
        json!({"object":object,"rings":rings,"bands":bands,"segments":segments,"profile_points":points.len()}),
    )
}

pub(super) fn polygon_triangles(points: &[[f64; 2]]) -> Result<Vec<[usize; 3]>, String> {
    if points.len() < 3 || points.len() > 1024 || points.iter().flatten().any(|n| !n.is_finite()) {
        return Err("polygon requires 3..1024 finite points".into());
    }
    let cross = |a: usize, b: usize, c: usize| {
        (points[b][0] - points[a][0]) * (points[c][1] - points[a][1])
            - (points[b][1] - points[a][1]) * (points[c][0] - points[a][0])
    };
    let area: f64 = (0..points.len())
        .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let sign = area.signum();
    if area.abs() < 1e-12 {
        return Err("polygon area is zero".into());
    }
    for i in 0..points.len() {
        let j = (i + 1) % points.len();
        if (points[i][0] - points[j][0]).hypot(points[i][1] - points[j][1]) < 1e-10 {
            return Err("polygon has a zero-length edge".into());
        }
        for k in i + 1..points.len() {
            let l = (k + 1) % points.len();
            if j == k || l == i {
                continue;
            }
            let crosses =
                cross(i, j, k) * cross(i, j, l) <= 0.0 && cross(k, l, i) * cross(k, l, j) <= 0.0;
            let overlaps = (0..2).all(|axis| {
                points[i][axis].min(points[j][axis]) <= points[k][axis].max(points[l][axis])
                    && points[k][axis].min(points[l][axis]) <= points[i][axis].max(points[j][axis])
            });
            if crosses && overlaps {
                return Err(format!(
                    "polygon intersects itself at outline edges {i}–{j} and {k}–{l} (zero-based point indices)"
                ));
            }
        }
    }
    let mut remaining: Vec<_> = (0..points.len()).collect();
    let mut triangles = vec![];
    while remaining.len() > 3 {
        let mut ear = None;
        for i in 0..remaining.len() {
            let a = remaining[(i + remaining.len() - 1) % remaining.len()];
            let b = remaining[i];
            let c = remaining[(i + 1) % remaining.len()];
            if cross(a, b, c) * sign <= 1e-14 {
                continue;
            }
            let inside = remaining.iter().any(|p| {
                *p != a
                    && *p != b
                    && *p != c
                    && cross(a, b, *p) * sign >= -1e-14
                    && cross(b, c, *p) * sign >= -1e-14
                    && cross(c, a, *p) * sign >= -1e-14
            });
            if !inside {
                ear = Some((i, [a, b, c]));
                break;
            }
        }
        let (i, triangle) = ear.ok_or("profile cap is degenerate or self-intersecting")?;
        triangles.push(triangle);
        remaining.remove(i);
    }
    if cross(remaining[0], remaining[1], remaining[2]).abs() < 1e-14 {
        return Err("profile cap is degenerate".into());
    }
    triangles.push([remaining[0], remaining[1], remaining[2]]);
    Ok(triangles)
}

pub(super) fn extrude_polygon(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let mut points: Vec<[f64; 2]> = serde_json::from_value(request["points"].clone())
        .map_err(|_| "points must be a simple polygon of [u,v] coordinates")?;
    let closed = request.get("closed_profile").map_or(Ok(true), |v| {
        v.as_bool().ok_or("closed_profile must be boolean")
    })?;
    let cap_start = request.get("cap_start").map_or(Ok(closed), |v| {
        v.as_bool().ok_or("cap_start must be boolean")
    })?;
    let cap_end = request
        .get("cap_end")
        .map_or(Ok(closed), |v| v.as_bool().ok_or("cap_end must be boolean"))?;
    if !closed && (cap_start || cap_end) {
        return Err("an open profile cannot have polygon end caps".into());
    }
    if closed {
        polygon_triangles(&points)?;
    } else if points.len() < 2
        || points.len() > 4096
        || points.iter().flatten().any(|v| !v.is_finite())
        || points.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err("open profile requires 2..4096 finite distinct consecutive points".into());
    }
    let area: f64 = (0..points.len())
        .map(|i| {
            points[i][0] * points[(i + 1) % points.len()][1]
                - points[(i + 1) % points.len()][0] * points[i][1]
        })
        .sum();
    if closed && area < 0.0 {
        points.reverse();
    }
    let axis = axis(request)?;
    let center = center(request)?;
    let depth = number(request, "depth", 0.1)?;
    if depth <= 1e-10 {
        return Err("depth must be positive".into());
    }
    let bevel = number(request, "bevel", 0.0)?;
    if !closed && bevel != 0.0 {
        return Err(
            "open profile extrusion requires bevel 0; bevel selected surface edges separately"
                .into(),
        );
    }
    if bevel < 0.0 || bevel * 2.0 >= depth {
        return Err("bevel must be nonnegative and less than half the extrusion depth".into());
    }
    let segments = match request.get("bevel_segments") {
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=16).contains(n))
            .ok_or("bevel_segments must be from 1 to 16")? as usize,
        None => 3,
    };
    if let Some(holes) = request.get("holes") {
        let holes: Vec<Vec<[f64; 2]>> = serde_json::from_value(holes.clone())
            .map_err(|_| "holes must be a list of [u,v] outlines")?;
        if !holes.is_empty() {
            if !closed || bevel != 0.0 {
                return Err("polygon holes require a closed profile and bevel 0".into());
            }
            return extrude_with_holes(mesh, request, points, holes, depth, [cap_start, cap_end]);
        }
    }
    let mut stages = vec![];
    if bevel == 0.0 {
        stages.extend([(0.0, 0.0), (depth, 0.0)]);
    } else {
        for i in 0..=segments {
            let angle = std::f64::consts::FRAC_PI_2 * i as f64 / segments as f64;
            stages.push((bevel * (1.0 - angle.cos()), bevel * (1.0 - angle.sin())));
        }
        for i in 0..=segments {
            let angle = std::f64::consts::FRAC_PI_2 * i as f64 / segments as f64;
            stages.push((
                depth - bevel + bevel * angle.sin(),
                bevel * (1.0 - angle.cos()),
            ));
        }
    }
    let mut outlines = vec![];
    for (_, offset) in &stages {
        if !closed {
            outlines.push(points.clone());
            continue;
        }
        let mut outline = vec![];
        for i in 0..points.len() {
            let previous = points[(i + points.len() - 1) % points.len()];
            let p = points[i];
            let next = points[(i + 1) % points.len()];
            let a = [p[0] - previous[0], p[1] - previous[1]];
            let b = [next[0] - p[0], next[1] - p[1]];
            let la = a[0].hypot(a[1]);
            let lb = b[0].hypot(b[1]);
            let na = [-a[1] / la, a[0] / la];
            let nb = [-b[1] / lb, b[0] / lb];
            let denominator = 1.0 + na[0] * nb[0] + na[1] * nb[1];
            if denominator < 1e-8 {
                return Err("bevel encounters a reversing outline corner".into());
            }
            outline.push([
                p[0] + offset * (na[0] + nb[0]) / denominator,
                p[1] + offset * (na[1] + nb[1]) / denominator,
            ]);
        }
        for i in 0..points.len() {
            let j = (i + 1) % points.len();
            let dot: f64 = (0..2)
                .map(|axis| {
                    (outline[j][axis] - outline[i][axis]) * (points[j][axis] - points[i][axis])
                })
                .sum();
            if dot <= 0.0 {
                return Err(format!(
                    "bevel collapses an outline edge between points {i} and {j} (zero-based) at offset {offset}; reduce bevel width"
                ));
            }
        }
        polygon_triangles(&outline)?;
        outlines.push(outline);
    }
    let triangles = if closed {
        polygon_triangles(&outlines[0])?
    } else {
        vec![]
    };
    let object = objects::destination_object(mesh, request)?;
    let mut rings = vec![];
    for (stage, outline) in stages.iter().zip(&outlines) {
        let mut ring = vec![];
        for p in outline {
            let mut point = center;
            point[axis] += stage.0;
            point[(axis + 1) % 3] += p[0];
            point[(axis + 2) % 3] += p[1];
            ring.push(mesh.add_vertex(point)?);
        }
        rings.push(ring);
    }
    let last = rings.len() - 1;
    for triangle in triangles {
        if cap_start {
            mesh.add_face(triangle.iter().rev().map(|i| rings[0][*i]).collect())?;
        }
        if cap_end {
            mesh.add_face(triangle.iter().map(|i| rings[last][*i]).collect())?;
        }
    }
    for pair in rings.windows(2) {
        for i in 0..if closed {
            points.len()
        } else {
            points.len() - 1
        } {
            let j = (i + 1) % points.len();
            mesh.add_face(vec![pair[0][i], pair[0][j], pair[1][j], pair[1][i]])?;
        }
    }
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(rings.iter().flatten());
    mesh.selection = rings[last].iter().copied().collect();
    let mut operation = json!({"object":object,"front_vertices":rings[last],"back_vertices":rings[0],"bevel":bevel,"bevel_segments":segments});
    if request.get("uv").is_some() {
        operation["rings"] = json!(rings);
    }
    Ok(operation)
}

fn outline_edges(points: &[[f64; 2]]) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
    points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
}

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn outline_area(points: &[[f64; 2]]) -> f64 {
    let origin = points[0];
    outline_edges(points)
        .map(|(a, b)| orient(origin, a, b))
        .sum::<f64>()
        * 0.5
}

fn outline_contains(outline: &[[f64; 2]], [x, y]: [f64; 2]) -> bool {
    outline_edges(outline)
        .filter(|&([ax, ay], [bx, by])| {
            (ay > y) != (by > y) && x < ax + (bx - ax) * (y - ay) / (by - ay)
        })
        .count()
        % 2
        == 1
}

fn outlines_touch(first: &[[f64; 2]], second: &[[f64; 2]]) -> bool {
    outline_edges(first).any(|(a, b)| {
        outline_edges(second).any(|(c, d)| {
            let overlapping_bounds = (0..2).all(|axis| {
                a[axis].min(b[axis]) <= c[axis].max(d[axis])
                    && c[axis].min(d[axis]) <= a[axis].max(b[axis])
            });
            overlapping_bounds
                && orient(a, b, c) * orient(a, b, d) <= 0.0
                && orient(c, d, a) * orient(c, d, b) <= 0.0
        })
    })
}

fn extrude_with_holes(
    mesh: &mut Mesh,
    request: &Value,
    outer: Vec<[f64; 2]>,
    holes: Vec<Vec<[f64; 2]>>,
    depth: f64,
    caps: [bool; 2],
) -> Result<Value, String> {
    let total = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
    if total > 1024 || holes.len() > 32 {
        return Err("polygon holes support at most 32 holes and 1024 total points".into());
    }
    let mut contours = vec![outer];
    for mut hole in holes {
        polygon_triangles(&hole)?;
        if !outline_contains(&contours[0], hole[0]) || outlines_touch(&contours[0], &hole) {
            return Err("each hole must lie strictly inside the outer outline".into());
        }
        for other in &contours[1..] {
            if outlines_touch(other, &hole)
                || outline_contains(other, hole[0])
                || outline_contains(&hole, other[0])
            {
                return Err("holes may not touch, overlap or contain one another".into());
            }
        }
        if outline_area(&hole) > 0.0 {
            hole.reverse();
        }
        contours.push(hole);
    }
    let points: Vec<_> = contours.iter().flatten().copied().collect();
    let mut triangles = Vec::new();
    if caps.into_iter().any(|cap| cap) {
        let polygons = contours
            .iter()
            .map(|contour| {
                contour
                    .iter()
                    .map(|&[x, y]| manifold_rust::linalg::Vec2::new(x, y))
                    .collect()
            })
            .collect();
        let raw = manifold_rust::polygon::triangulate(&polygons, 0.0, false);
        let expected_area = contours
            .iter()
            .map(|contour| outline_area(contour))
            .sum::<f64>();
        let mut area = 0.0;
        for triangle in raw {
            let indices = [
                triangle.x as usize,
                triangle.y as usize,
                triangle.z as usize,
            ];
            if indices.iter().any(|&index| index >= points.len()) {
                return Err("polygon cap tessellation produced an invalid vertex".into());
            }
            let [a, b, c] = indices.map(|index| points[index]);
            let triangle_area = orient(a, b, c) * 0.5;
            if !triangle_area.is_finite() || triangle_area <= 0.0 {
                return Err(
                    "polygon cap tessellation produced a degenerate or reversed triangle".into(),
                );
            }
            area += triangle_area;
            triangles.push(indices);
        }
        if !expected_area.is_finite()
            || (area - expected_area).abs() > expected_area * 1e-8
            || triangles.len() != total + 2 * (contours.len() - 1) - 2
        {
            return Err("polygon caps could not be tessellated with the requested holes".into());
        }
    }
    let axis = axis(request)?;
    let center = center(request)?;
    let object = objects::destination_object(mesh, request)?;
    let mut rings = Vec::new();
    for height in [0.0, depth] {
        let mut ring = Vec::with_capacity(total);
        for &[u, v] in &points {
            let mut point = center;
            point[axis] += height;
            point[(axis + 1) % 3] += u;
            point[(axis + 2) % 3] += v;
            ring.push(mesh.add_vertex(point)?);
        }
        rings.push(ring);
    }
    for [a, b, c] in triangles {
        if caps[0] {
            mesh.add_face(vec![rings[0][c], rings[0][b], rings[0][a]])?;
        }
        if caps[1] {
            mesh.add_face(vec![rings[1][a], rings[1][b], rings[1][c]])?;
        }
    }
    let mut start = 0;
    let lengths: Vec<_> = contours.iter().map(Vec::len).collect();
    for &count in &lengths {
        for index in 0..count {
            let a = start + index;
            let b = start + (index + 1) % count;
            mesh.add_face(vec![rings[0][a], rings[0][b], rings[1][b], rings[1][a]])?;
        }
        start += count;
    }
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(rings.iter().flatten().copied());
    mesh.selection = rings[1].iter().copied().collect();
    let mut operation = json!({"object":object,"front_vertices":rings[1],"back_vertices":rings[0],
        "bevel":0,"hole_count":contours.len()-1});
    if request.get("uv").is_some() {
        operation["rings"] = json!(rings);
        operation["uv_contour_lengths"] = json!(lengths);
    }
    Ok(operation)
}

fn triangulate(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let object = request["object"].as_u64().ok_or("object is required")?;
    let vertices = mesh
        .objects
        .get(&object)
        .ok_or("unknown object")?
        .vertices
        .clone();
    let faces: Vec<_> = mesh
        .faces
        .iter()
        .filter(|(_, face)| vertices.contains(&face[0]) && face.len() > 3)
        .map(|(id, face)| (*id, face.clone()))
        .collect();
    let mut count = 0;
    for (id, face) in faces {
        // Revolved strips and pole fans are convex; reject other polygons rather than guessing.
        topology::convex_face(mesh, &face)?;
        mesh.faces.insert(id, vec![face[0], face[1], face[2]]);
        for i in 2..face.len() - 1 {
            mesh.add_face(vec![face[0], face[i], face[i + 1]])?;
            count += 1;
        }
    }
    Ok(json!({"object":object,"created_face_count":count}))
}

fn bisect(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let object = request["object"].as_u64().ok_or("object is required")?;
    let owned = mesh
        .objects
        .get(&object)
        .ok_or("unknown object")?
        .vertices
        .clone();
    let normal = vector(&request["normal"])?;
    let length = normal[0].hypot(normal[1]).hypot(normal[2]);
    if !length.is_finite() || length < 1e-12 {
        return Err("plane normal must not be zero".into());
    }
    let normal = normal.map(|n| n / length);
    let offset = number(request, "offset", 0.0)?;
    let distance = |p: [f64; 3]| p.iter().zip(normal).map(|(a, b)| a * b).sum::<f64>() - offset;
    let tolerance = number(request, "snap_tolerance", 0.0)?;
    if !(0.0..=0.001).contains(&tolerance) {
        return Err("snap_tolerance must be from 0 to 0.001 world units".into());
    }
    let mut snapped = vec![];
    if tolerance > 0.0 {
        for vertex in &owned {
            let point = mesh.vertices.get_mut(vertex).unwrap();
            let d = distance(*point);
            if d.abs() > 1e-12 && d.abs() <= tolerance {
                for axis in 0..3 {
                    point[axis] -= normal[axis] * d;
                }
                snapped.push(*vertex);
            }
        }
    }
    let faces: Vec<_> = mesh
        .faces
        .iter()
        .filter(|(_, f)| owned.contains(&f[0]))
        .map(|(id, f)| (*id, f.clone()))
        .collect();
    let mut cuts = BTreeMap::new();
    let mut selected = BTreeSet::new();
    let mut splits = BTreeMap::new();
    for (id, face) in faces {
        let distances: Vec<_> = face.iter().map(|v| distance(mesh.vertices[v])).collect();
        for (v, d) in face.iter().zip(&distances) {
            if d.abs() < 1e-9 {
                selected.insert(*v);
            }
        }
        if !distances.iter().any(|d| *d < -1e-9) || !distances.iter().any(|d| *d > 1e-9) {
            continue;
        }
        topology::convex_face(mesh, &face)?;
        let mut halves = [vec![], vec![]];
        for i in 0..face.len() {
            let j = (i + 1) % face.len();
            let a = face[i];
            let b = face[j];
            let da = distances[i];
            let db = distances[j];
            if da >= -1e-9 {
                halves[0].push(a);
            }
            if da <= 1e-9 {
                halves[1].push(a);
            }
            if (da < -1e-9 && db > 1e-9) || (da > 1e-9 && db < -1e-9) {
                let key = (a.min(b), a.max(b));
                let v = if let Some(v) = cuts.get(&key) {
                    *v
                } else {
                    let pa = mesh.vertices[&a];
                    let pb = mesh.vertices[&b];
                    let t = da / (da - db);
                    let v = mesh.add_vertex(std::array::from_fn(|axis| {
                        pa[axis] + t * (pb[axis] - pa[axis])
                    }))?;
                    cuts.insert(key, v);
                    v
                };
                selected.insert(v);
                halves[0].push(v);
                halves[1].push(v);
            }
        }
        let mut ids = vec![];
        for half in halves {
            for i in 1..half.len() - 1 {
                let triangle = vec![half[0], half[i], half[i + 1]];
                if ids.is_empty() {
                    mesh.faces.insert(id, triangle);
                    ids.push(id);
                } else {
                    ids.push(mesh.add_face(triangle)?);
                }
            }
        }
        splits.insert(id, ids);
    }
    // A plane passing through an existing vertex must split crossing edges on both incident faces.
    mesh.objects
        .get_mut(&object)
        .unwrap()
        .vertices
        .extend(cuts.values());
    mesh.selection = selected;
    Ok(
        json!({"cut_vertices":mesh.selection,"face_splits":splits,"snapped_vertices":snapped,"snap_tolerance":tolerance,"plane":{"normal":normal,"offset":offset}}),
    )
}
