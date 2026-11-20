use crate::{Mesh, uv};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn vector(value: &Value, name: &str) -> Result<[f64; 3], String> {
    let result: [f64; 3] = serde_json::from_value(value.clone())
        .map_err(|_| format!("{name} must contain three finite numbers"))?;
    if result.iter().any(|number| !number.is_finite()) {
        return Err(format!("{name} must contain three finite numbers"));
    }
    Ok(result)
}

pub(super) fn normal(mesh: &Mesh, face: u64) -> [f64; 3] {
    let vertices = &mesh.faces[&face];
    let mut result = [0.0; 3];
    for index in 0..vertices.len() {
        let a = mesh.vertices[&vertices[index]];
        let b = mesh.vertices[&vertices[(index + 1) % vertices.len()]];
        for axis in 0..3 {
            let next = (axis + 1) % 3;
            let last = (axis + 2) % 3;
            result[axis] += (a[next] - b[next]) * (a[last] + b[last]);
        }
    }
    result
}

fn unit(value: [f64; 3]) -> Result<[f64; 3], String> {
    let length = value[0].hypot(value[1]).hypot(value[2]);
    if !length.is_finite() || length <= 1e-15 {
        return Err("normal or direction has zero or nonfinite length".into());
    }
    Ok(value.map(|number| number / length))
}

fn bounds(value: &Value) -> Result<[[f64; 3]; 2], String> {
    if value.as_array().is_none_or(|values| values.len() != 2) {
        return Err("bounds must contain min and max points".into());
    }
    let result = [
        vector(&value[0], "bounds min")?,
        vector(&value[1], "bounds max")?,
    ];
    if (0..3).any(|axis| result[0][axis] > result[1][axis]) {
        return Err("bounds min must not exceed max".into());
    }
    Ok(result)
}

pub(super) fn filter_faces(
    mesh: &Mesh,
    request: &Value,
    targets: Vec<u64>,
) -> Result<Vec<u64>, String> {
    let Some(filter) = request.get("filter") else {
        return Ok(targets);
    };
    let object = filter.as_object().ok_or("filter must be an object")?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "bounds" | "normal" | "flat"))
    {
        return Err("face filter accepts bounds, normal and flat".into());
    }
    let region = filter.get("bounds").map(bounds).transpose()?;
    let flat = filter
        .get("flat")
        .map(|value| value.as_bool().ok_or("flat must be boolean"))
        .transpose()?;
    let normal_filter = filter
        .get("normal")
        .map(|value| -> Result<_, String> {
            let direction = unit(vector(&value["direction"], "normal direction")?)?;
            let range: [f64; 2] = serde_json::from_value(value["range"].clone())
                .map_err(|_| "normal range requires [min,max] dot products")?;
            if range
                .iter()
                .any(|number| !number.is_finite() || number.abs() > 1.0)
                || range[0] > range[1]
            {
                return Err("normal range must be ordered within -1..1".into());
            }
            let absolute = value
                .get("absolute")
                .map(|value| value.as_bool().ok_or("absolute must be boolean"))
                .transpose()?
                .unwrap_or(false);
            Ok((direction, range, absolute))
        })
        .transpose()?;
    let mut result = Vec::new();
    for face in targets {
        if flat.is_some_and(|flat| mesh.flat_faces.contains(&face) != flat) {
            continue;
        }
        if let Some(region) = region {
            let vertices = &mesh.faces[&face];
            let center: [f64; 3] = std::array::from_fn(|axis| {
                vertices
                    .iter()
                    .map(|id| mesh.vertices[id][axis])
                    .sum::<f64>()
                    / vertices.len() as f64
            });
            if (0..3).any(|axis| center[axis] < region[0][axis] || center[axis] > region[1][axis]) {
                continue;
            }
        }
        if let Some((direction, range, absolute)) = normal_filter {
            let n = unit(normal(mesh, face))?;
            let dot: f64 = (0..3).map(|axis| n[axis] * direction[axis]).sum();
            let dot = if absolute { dot.abs() } else { dot };
            if dot < range[0] || dot > range[1] {
                continue;
            }
        }
        result.push(face);
    }
    if result.is_empty() {
        return Err("no faces match the filter".into());
    }
    Ok(result)
}

fn target_vertices(mesh: &Mesh, request: &Value) -> Result<BTreeSet<u64>, String> {
    let faces = uv::faces(mesh, request)?;
    let mut vertices: BTreeSet<_> = faces
        .iter()
        .flat_map(|face| mesh.faces[face].iter().copied())
        .collect();
    if let Some(ids) = request.get("vertices") {
        let ids: BTreeSet<u64> =
            serde_json::from_value(ids.clone()).map_err(|_| "vertices must be an array of IDs")?;
        if ids.is_empty() || !ids.is_subset(&vertices) {
            return Err("vertices must be existing corners inside the requested scope".into());
        }
        vertices = ids;
    }
    Ok(vertices)
}

fn mesh_bounds(mesh: &Mesh, vertices: &BTreeSet<u64>) -> [[f64; 3]; 2] {
    let mut result = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for id in vertices {
        for axis in 0..3 {
            result[0][axis] = result[0][axis].min(mesh.vertices[id][axis]);
            result[1][axis] = result[1][axis].max(mesh.vertices[id][axis]);
        }
    }
    result
}

fn number(request: &Value, name: &str, default: f64) -> Result<f64, String> {
    let number = request
        .get(name)
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| format!("{name} must be finite"))
        })
        .transpose()?
        .unwrap_or(default);
    if !number.is_finite() {
        return Err(format!("{name} must be finite"));
    }
    Ok(number)
}

fn axis(value: &Value) -> Result<usize, String> {
    let axis = value
        .as_u64()
        .filter(|axis| *axis < 3)
        .ok_or("axis must be 0, 1 or 2")?;
    Ok(axis as usize)
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    if command == "uv_project" {
        return project(mesh, request);
    }
    let vertices = target_vertices(mesh, request)?;
    let blend = request
        .get("blend")
        .map(|value| value.as_str().ok_or("blend must be replace or multiply"))
        .transpose()?
        .unwrap_or("replace");
    let multiply = match blend {
        "replace" => false,
        "multiply" => true,
        _ => return Err("blend must be replace or multiply".into()),
    };
    let mut colors = BTreeMap::new();
    if command == "vertex_color_gradient" {
        let start = vector(&request["start"], "start")?;
        let end = vector(&request["end"], "end")?;
        let from = color(&request["from"])?;
        let to = color(&request["to"])?;
        let direction: [f64; 3] = std::array::from_fn(|axis| end[axis] - start[axis]);
        let length: f64 = direction.iter().map(|value| value * value).sum();
        if !length.is_finite() || length <= 1e-24 {
            return Err("gradient start and end must be distinct finite points".into());
        }
        for id in &vertices {
            let dot: f64 = (0..3)
                .map(|axis| (mesh.vertices[id][axis] - start[axis]) * direction[axis])
                .sum();
            let fraction = (dot / length).clamp(0.0, 1.0);
            colors.insert(
                *id,
                std::array::from_fn(|axis| from[axis] + fraction * (to[axis] - from[axis])),
            );
        }
    } else {
        let direction = vector(&request["direction"], "direction")?;
        let tint = request
            .get("color")
            .map(color)
            .transpose()?
            .unwrap_or([1.0; 3]);
        let ambient = number(request, "ambient", 0.5)?;
        let strength = number(request, "strength", 0.5)?;
        let absolute: BTreeSet<usize> = request
            .get("absolute_axes")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(|_| "absolute_axes must contain axis indices")?
            .unwrap_or_default();
        if absolute.iter().any(|axis| *axis > 2) {
            return Err("absolute_axes must contain 0, 1 or 2".into());
        }
        let mut normals = BTreeMap::new();
        for (face, corners) in &mesh.faces {
            let n = normal(mesh, *face);
            for id in corners {
                if vertices.contains(id) {
                    let sum = normals.entry(*id).or_insert([0.0; 3]);
                    for axis in 0..3 {
                        sum[axis] += n[axis];
                    }
                }
            }
        }
        for id in &vertices {
            let mut n = unit(normals[id])?;
            for axis in &absolute {
                n[*axis] = n[*axis].abs();
            }
            let dot: f64 = (0..3).map(|axis| n[axis] * direction[axis]).sum();
            let factor = (ambient + strength * dot.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            colors.insert(*id, tint.map(|value| value * factor));
        }
    }
    for (id, mut color) in colors {
        if multiply {
            let previous = mesh.vertex_colors.get(&id).copied().unwrap_or([1.0; 3]);
            for axis in 0..3 {
                color[axis] *= previous[axis];
            }
        }
        mesh.vertex_colors.insert(id, color);
    }
    Ok(json!({"painted_vertices":vertices.len(),"blend":if multiply {"multiply"} else {"replace"}}))
}

fn color(value: &Value) -> Result<[f64; 3], String> {
    let result = vector(value, "color")?;
    if result.iter().any(|number| !(0.0..=1.0).contains(number)) {
        return Err("color must contain RGB in 0..1".into());
    }
    Ok(result)
}

fn project(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let masters_only = request.get("masters_only")
        .map(|value| value.as_bool().ok_or("masters_only must be boolean"))
        .transpose()?.unwrap_or(false);
    let mut scoped = request.clone();
    if masters_only {
        if request.get("object").is_some() || request.get("faces").is_some() {
            return Err("masters_only requires project-wide UV projection; omit object and faces".into());
        }
        let instance_vertices: BTreeSet<_> = mesh.objects.values()
            .filter(|object| object.instance.is_some())
            .flat_map(|object| object.vertices.iter().copied())
            .collect();
        let faces: Vec<_> = mesh.faces.iter()
            .filter(|(_, vertices)| !instance_vertices.contains(&vertices[0]))
            .map(|(id, _)| *id)
            .collect();
        scoped["faces"] = json!(faces);
    }
    let request = &scoped;
    let mode = request["mode"]
        .as_str()
        .ok_or("mode must be planar, box or cylindrical")?;
    if !matches!(mode, "planar" | "box" | "cylindrical") {
        return Err("mode must be planar, box or cylindrical".into());
    }
    if request.get("vertices").is_some() {
        return Err("uv_project targets whole faces; use object, faces and filter".into());
    }
    let targets = uv::faces(mesh, request)?;
    // Fit to the unfiltered object/face scope so a face subset keeps consistent atlas scaling.
    let mut unfiltered = request.clone();
    unfiltered.as_object_mut().unwrap().remove("filter");
    if request.get("object").is_some() {
        unfiltered.as_object_mut().unwrap().remove("faces");
    }
    let scope = uv::faces(mesh, &unfiltered)?;
    let vertices = scope
        .iter()
        .flat_map(|face| mesh.faces[face].iter().copied())
        .collect();
    let fitted = mesh_bounds(mesh, &vertices);
    let bounds = request
        .get("bounds")
        .map(bounds)
        .transpose()?
        .unwrap_or(fitted);
    let region: [f64; 4] = request
        .get("region")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| "region must be [u_min,v_min,u_max,v_max]")?
        .unwrap_or([0.0, 0.0, 1.0, 1.0]);
    if region.iter().any(|value| !value.is_finite())
        || region[0] >= region[2]
        || region[1] >= region[3]
    {
        return Err("region requires finite, increasing UV bounds".into());
    }
    let axes: [usize; 2] = request
        .get("axes")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| "axes must contain two distinct axis indices")?
        .unwrap_or([0, 1]);
    if axes[0] > 2 || axes[1] > 2 || axes[0] == axes[1] {
        return Err("axes must contain two distinct indices in 0..2".into());
    }
    let cylinder_axis = request.get("axis").map(axis).transpose()?.unwrap_or(1);
    let radial = [(cylinder_axis + 1) % 3, (cylinder_axis + 2) % 3];
    let phase = number(request, "phase", 0.0)?.rem_euclid(1.0);
    let fit_sections = request
        .get("fit_sections")
        .map(|value| value.as_bool().ok_or("fit_sections must be boolean"))
        .transpose()?
        .unwrap_or(false);
    let tolerance = number(request, "section_tolerance", 0.0)?;
    if tolerance < 0.0 {
        return Err("section_tolerance must be nonnegative".into());
    }
    let mut section_bounds = BTreeMap::new();
    let mut section_count = 0;
    if mode == "cylindrical" && fit_sections {
        let mut ordered: Vec<_> = vertices.iter().copied().collect();
        ordered.sort_by(|a, b| {
            mesh.vertices[a][cylinder_axis].total_cmp(&mesh.vertices[b][cylinder_axis])
        });
        let mut offset = 0;
        while offset < ordered.len() {
            let height = mesh.vertices[&ordered[offset]][cylinder_axis];
            let mut end = offset + 1;
            while end < ordered.len()
                && mesh.vertices[&ordered[end]][cylinder_axis] - height <= tolerance
            {
                end += 1;
            }
            let ring = ordered[offset..end].iter().copied().collect();
            let fitted = mesh_bounds(mesh, &ring);
            for id in &ordered[offset..end] {
                section_bounds.insert(*id, fitted);
            }
            section_count += 1;
            offset = end;
        }
    }
    let fraction = |point: [f64; 3], axis: usize| -> Result<f64, String> {
        let extent = bounds[1][axis] - bounds[0][axis];
        if !extent.is_finite() || extent <= 1e-12 {
            return Err("projection bounds have zero or nonfinite extent on a mapped axis".into());
        }
        Ok((point[axis] - bounds[0][axis]) / extent)
    };
    let mut count = 0;
    for face in &targets {
        let mapped_axes = if mode == "box" {
            let n = normal(mesh, *face);
            let mut dominant = 0;
            for axis in 1..3 {
                if n[axis].abs() > n[dominant].abs() {
                    dominant = axis;
                }
            }
            match dominant {
                0 => [2, 1],
                1 => [0, 2],
                _ => [0, 1],
            }
        } else {
            axes
        };
        let mut coordinates = Vec::new();
        for vertex in &mesh.faces[face] {
            let point = mesh.vertices[vertex];
            let uv = if mode == "cylindrical" {
                let radial_bounds = if fit_sections {
                    section_bounds[vertex]
                } else {
                    bounds
                };
                let mut position = [0.0; 2];
                for index in 0..2 {
                    let axis = radial[index];
                    let radius = (radial_bounds[1][axis] - radial_bounds[0][axis]) / 2.0;
                    if !radius.is_finite() || radius <= 1e-12 {
                        return Err("cylindrical sections need nonzero radial extents; use box/planar for caps".into());
                    }
                    let center = radial_bounds[0][axis] + radius;
                    position[index] = (point[axis] - center) / radius;
                    if position[index].abs() < 1e-12 {
                        position[index] = 0.0;
                    }
                }
                let angle = position[1].atan2(position[0]) / std::f64::consts::TAU;
                let u = (phase + angle).rem_euclid(1.0);
                [
                    if u < 1e-12 { 1.0 } else { u },
                    fraction(point, cylinder_axis)?,
                ]
            } else {
                [
                    fraction(point, mapped_axes[0])?,
                    fraction(point, mapped_axes[1])?,
                ]
            };
            coordinates.push((*vertex, uv));
        }
        if mode == "cylindrical" {
            let min = coordinates
                .iter()
                .map(|(_, uv)| uv[0])
                .fold(f64::INFINITY, f64::min);
            let max = coordinates
                .iter()
                .map(|(_, uv)| uv[0])
                .fold(f64::NEG_INFINITY, f64::max);
            if max - min > 0.5 {
                for (_, uv) in &mut coordinates {
                    if uv[0] > 0.5 {
                        uv[0] -= 1.0;
                    }
                }
            }
        }
        for (vertex, uv) in coordinates {
            let mapped = [
                region[0] + uv[0] * (region[2] - region[0]),
                region[1] + uv[1] * (region[3] - region[1]),
            ];
            mesh.uvs.entry(*face).or_default().insert(vertex, mapped);
            count += 1;
        }
    }
    Ok(
        json!({"assigned_corners":count,"face_count":targets.len(),"mode":mode,"bounds":bounds,"region":region,"section_count":section_count}),
    )
}
