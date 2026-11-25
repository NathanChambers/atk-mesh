use super::Component;
use crate::{Mesh, geometry::*, normals};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    side_region: [f64; 4],
    cap_region: [f64; 4],
    #[serde(default)]
    padding: f64,
}

pub(crate) fn expected_region(component: &Component, face: u64) -> Option<[f64; 4]> {
    let uv = component.recipe.as_ref()?.get("uv")?;
    let field = if component.uv_cap_faces.contains(&face) {
        "cap_region"
    } else {
        "side_region"
    };
    serde_json::from_value(uv.get(field)?.clone()).ok()
}

fn inset(mut rect: [f64; 4], padding: f64) -> Result<[f64; 4], String> {
    if !rect.iter().all(|n| (0.0..=1.0).contains(n)) || !padding.is_finite() || padding < 0.0 {
        return Err("UV regions must be normalized and padding must be nonnegative".into());
    }
    rect[0] += padding;
    rect[1] += padding;
    rect[2] -= padding;
    rect[3] -= padding;
    if rect[0] >= rect[2] || rect[1] >= rect[3] {
        return Err("UV region has no area after padding".into());
    }
    Ok(rect)
}

fn place(rect: [f64; 4], u: f64, v: f64) -> [f64; 2] {
    [
        rect[0] + (rect[2] - rect[0]) * u,
        rect[1] + (rect[3] - rect[1]) * v,
    ]
}

fn arc(points: &[[f64; 3]], closed: bool) -> Result<Vec<f64>, String> {
    let mut distances = vec![0.0];
    for pair in points.windows(2) {
        distances.push(distances.last().unwrap() + length(sub(pair[1], pair[0])));
    }
    let mut total = *distances.last().unwrap();
    if closed {
        total += length(sub(points[0], *points.last().unwrap()));
    }
    if total <= 0.0 {
        return Err("UV mapping requires a nonzero path and profile length".into());
    }
    Ok(distances.into_iter().map(|d| d / total).collect())
}

fn map_cap(mesh: &mut Mesh, faces: &[u64], rect: [f64; 4]) {
    let normal = normals::face(mesh, &mesh.faces[&faces[0]]);
    let axis = (0..3)
        .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
        .unwrap();
    let axes = [(axis + 1) % 3, (axis + 2) % 3];
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for vertex in faces.iter().flat_map(|id| &mesh.faces[id]) {
        for i in 0..2 {
            let coord = mesh.vertices[vertex][axes[i]];
            min[i] = min[i].min(coord);
            max[i] = max[i].max(coord);
        }
    }
    for face in faces {
        let corners = mesh.faces[face]
            .iter()
            .map(|id| {
                let coord = |i: usize| (mesh.vertices[id][axes[i]] - min[i]) / (max[i] - min[i]);
                (*id, place(rect, coord(0), coord(1)))
            })
            .collect();
        mesh.uvs.insert(*face, corners);
    }
}

pub(super) fn assign(
    mesh: &mut Mesh,
    command: &str,
    request: &Value,
    uv: &Value,
    operation: &mut Value,
    faces: &BTreeSet<u64>,
) -> Result<BTreeSet<u64>, String> {
    if !matches!(command, "loft" | "sweep" | "revolve" | "extrude_polygon") {
        return Err("generator UVs require a sweep, loft, revolve, or extrude_polygon".into());
    }
    let layout: Layout = serde_json::from_value(uv.clone())
        .map_err(|e| format!("invalid generator UV layout: {e}"))?;
    let side = inset(layout.side_region, layout.padding)?;
    inset(layout.cap_region, layout.padding)?;
    let rings: Vec<Vec<u64>> = operation["rings"]
        .as_array()
        .ok_or("missing generated rings")?
        .iter()
        .map(|ring| {
            if let (Some(first), Some(count)) =
                (ring["first_vertex"].as_u64(), ring["count"].as_u64())
            {
                return Ok((first..first + count).collect());
            }
            serde_json::from_value(ring.clone()).map_err(|_| "invalid generated ring")
        })
        .collect::<Result<_, _>>()?;
    let mut caps = [Vec::new(), Vec::new()];
    let locations: BTreeMap<_, _> = rings
        .iter()
        .enumerate()
        .flat_map(|(i, ring)| ring.iter().enumerate().map(move |(j, id)| (*id, (i, j))))
        .collect();
    let revolved = command == "revolve";
    let closed = request["closed_profile"]
        .as_bool()
        .unwrap_or(command != "revolve");
    let wrap = if command == "loft" {
        request["closed"].as_bool().unwrap_or(true)
    } else {
        closed
    };
    let points = |ring: &[u64]| ring.iter().map(|id| mesh.vertices[id]).collect::<Vec<_>>();
    let profile = if revolved {
        rings.iter().map(|r| mesh.vertices[&r[0]]).collect()
    } else {
        points(&rings[0])
    };
    let u = arc(&profile, wrap)?;
    let mut v = vec![];
    if !revolved {
        let centers: Vec<_> = rings
            .iter()
            .map(|ring| {
                mul(
                    points(ring).into_iter().fold([0.; 3], add),
                    1. / ring.len() as f64,
                )
            })
            .collect();
        if command == "sweep" {
            let path: Vec<[f64; 3]> = serde_json::from_value(operation["uv_path"].take())
                .map_err(|_| "sweep UV path missing")?;
            operation.as_object_mut().unwrap().remove("uv_path");
            v = arc(&path, false)?;
        } else {
            let mut lengths = vec![0.0];
            for (index, pair) in centers.windows(2).enumerate() {
                let mut distance = length(sub(pair[1], pair[0]));
                if distance <= 1e-12 {
                    let section = &rings[index];
                    distance = section
                        .iter()
                        .zip(&rings[index + 1])
                        .map(|(a, b)| length(sub(mesh.vertices[a], mesh.vertices[b])))
                        .sum::<f64>()
                        / section.len() as f64;
                }
                lengths.push(lengths[index] + distance);
            }
            let total = *lengths.last().unwrap();
            if total <= 0.0 {
                return Err("UV mapping requires a nonzero section path".into());
            }
            v = lengths.into_iter().map(|s| s / total).collect();
        }
    }
    let segments = operation["segments"].as_u64().unwrap_or(0) as usize;
    let band_count = operation["bands"].as_array().map_or(0, Vec::len);
    for (ordinal, face) in faces.iter().enumerate() {
        let vertices = &mesh.faces[face];
        let corners: Vec<_> = vertices.iter().map(|id| locations[id]).collect();
        if revolved && ordinal >= segments * band_count {
            let end = usize::from(corners.iter().any(|(_, j)| *j == segments));
            caps[end].push(*face);
            continue;
        }
        if !revolved && corners.iter().all(|(i, _)| *i == corners[0].0) {
            caps[usize::from(corners[0].0 != 0)].push(*face);
            continue;
        }
        let indices: Vec<_> = corners
            .iter()
            .map(|(i, j)| if revolved { *i } else { *j })
            .collect();
        let seam = wrap && indices.contains(&0) && indices.contains(&(u.len() - 1));
        let mut mapped = BTreeMap::new();
        for ((id, (ring, column)), index) in vertices.iter().zip(corners).zip(indices) {
            let longitude = if seam && index == 0 { 1.0 } else { u[index] };
            let latitude = if revolved {
                let segment = ordinal % segments;
                let position = if rings[ring].len() == 1 {
                    segment as f64 + 0.5
                } else if column == 0 && segment + 1 == segments {
                    segments as f64
                } else {
                    column as f64
                };
                position / segments as f64
            } else {
                v[ring]
            };
            mapped.insert(*id, place(side, longitude, latitude));
        }
        mesh.uvs.insert(*face, mapped);
    }
    let both_caps = caps.iter().all(|f| !f.is_empty());
    for (index, faces) in caps
        .iter()
        .enumerate()
        .filter(|(_, faces)| !faces.is_empty())
    {
        let mut rect = layout.cap_region;
        if both_caps {
            let middle = (rect[0] + rect[2]) * 0.5;
            rect[if index == 0 { 2 } else { 0 }] = middle;
        }
        map_cap(mesh, faces, inset(rect, layout.padding)?);
    }
    Ok(caps.into_iter().flatten().collect())
}
