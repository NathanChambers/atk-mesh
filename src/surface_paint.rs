use crate::{
    Mesh,
    geometry::{cross, dot, mul, sub},
    normals,
    painting::Image,
    uv,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Shape {
    Band {
        center: [f64; 3],
        axis: [f64; 3],
        width: f64,
        #[serde(default)]
        feather: f64,
    },
    Gradient {
        start: [f64; 3],
        end: [f64; 3],
    },
    Decal {
        center: [f64; 3],
        right: [f64; 3],
        up: [f64; 3],
        size: [f64; 2],
        depth: f64,
    },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Triangle {
    positions: [[f64; 3]; 3],
    uv: [[f64; 2]; 3],
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct SurfaceMask {
    shape: Shape,
    triangles: Vec<Triangle>,
    pub(super) image: Option<Image>,
}

pub(super) fn faces(mesh: &Mesh, request: &Value) -> Result<Vec<u64>, String> {
    let candidates = uv::faces(mesh, request)?;
    let Some(expand) = request.get("expand") else {
        if request.get("seed_face").is_some() {
            return Err("seed_face requires expand".into());
        }
        return Ok(candidates);
    };
    let island = match expand.as_str() {
        Some("uv_island") => true,
        Some("connected") => false,
        _ => return Err("expand must be uv_island or connected".into()),
    };
    let seed = request["seed_face"]
        .as_u64()
        .ok_or("expand requires seed_face")?;
    if !candidates.contains(&seed) || (island && !uv::complete(mesh, seed)) {
        return Err("seed_face must be in scope and have complete UVs for uv_island".into());
    }

    let mut edges = BTreeMap::<_, Vec<_>>::new();
    let mut face_edges = BTreeMap::new();
    for face in candidates {
        if island && !uv::complete(mesh, face) {
            continue;
        }
        let vertices = &mesh.faces[&face];
        let mut keys = Vec::new();
        for index in 0..vertices.len() {
            let a = vertices[index];
            let b = vertices[(index + 1) % vertices.len()];
            let key = (a.min(b), a.max(b));
            keys.push(key);
            edges.entry(key).or_default().push(face);
        }
        face_edges.insert(face, keys);
    }

    let mut selected = BTreeSet::from([seed]);
    let mut pending = vec![seed];
    while let Some(face) = pending.pop() {
        for &(a, b) in &face_edges[&face] {
            for &neighbor in &edges[&(a, b)] {
                if selected.contains(&neighbor) {
                    continue;
                }
                let continuous = !island
                    || [a, b].into_iter().all(|id| {
                        let first = mesh.uvs[&face][&id];
                        let second = mesh.uvs[&neighbor][&id];
                        (0..2).all(|axis| (first[axis] - second[axis]).abs() <= 1e-9)
                    });
                if continuous && selected.insert(neighbor) {
                    pending.push(neighbor);
                }
            }
        }
    }
    Ok(selected.into_iter().collect())
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let faces = faces(mesh, request)?;
    let (offset, limit) = crate::responses::page(request)?;
    let total = faces.len();
    let items: Vec<_> = faces.into_iter().skip(offset).take(limit).collect();
    let next = offset.saturating_add(items.len());
    Ok(
        json!({"revision":revision,"faces":items,"total":total,"next_offset":(next < total).then_some(next)}),
    )
}

impl SurfaceMask {
    pub(super) fn set(
        mesh: &Mesh,
        request: &Value,
        previous: Option<&Self>,
    ) -> Result<Self, String> {
        let shape: Shape = serde_json::from_value(request["shape"].clone())
            .map_err(|error| format!("invalid surface mask shape: {error}"))?;
        let use_scope = ["object", "faces", "filter", "expand", "seed_face"]
            .iter()
            .any(|key| request.get(key).is_some());
        let triangles = match previous {
            Some(previous) if !use_scope => previous.triangles.clone(),
            _ => snapshot(mesh, request)?,
        };
        let image = if !matches!(shape, Shape::Decal { .. }) {
            if request.get("source_texture").is_some() {
                return Err("source_texture is only available for decals".into());
            }
            None
        } else if let Some(source) = request.get("source_texture") {
            let name = source
                .as_str()
                .ok_or("source_texture must be a texture name")?;
            let texture = mesh.textures.get(name).ok_or("unknown source texture")?;
            Some(Image {
                width: texture.width,
                height: texture.height,
                rgba_base64: texture.rgba_base64.clone(),
            })
        } else {
            previous.and_then(|mask| mask.image.clone())
        };
        let mask = Self {
            shape,
            triangles,
            image,
        };
        mask.validate()?;
        Ok(mask)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        let bounded = |value: f64| value.is_finite() && value.abs() <= 1e9;
        let positive = |value: f64| (1e-9..=1e9).contains(&value);
        let vector = |value: [f64; 3]| value.into_iter().all(bounded);
        let valid = match self.shape {
            Shape::Band {
                center,
                axis,
                width,
                feather,
            } => {
                vector(center)
                    && vector(axis)
                    && unit(axis).is_some()
                    && positive(width)
                    && bounded(feather)
                    && feather >= 0.0
            }
            Shape::Gradient { start, end } => {
                vector(start) && vector(end) && unit(sub(end, start)).is_some()
            }
            Shape::Decal {
                center,
                right,
                up,
                size,
                depth,
            } => {
                let basis = decal_basis(right, up);
                vector(center)
                    && vector(right)
                    && vector(up)
                    && basis.is_some()
                    && size.into_iter().all(positive)
                    && positive(depth)
                    && self.image.is_some()
            }
        };
        if !valid {
            return Err(
                "invalid band, gradient or decal parameters, or missing decal source_texture"
                    .into(),
            );
        }
        if self.triangles.is_empty() || self.triangles.len() > 4096 {
            return Err("surface mask requires 1..4096 triangles".into());
        }
        for triangle in &self.triangles {
            if !triangle.positions.into_iter().all(vector)
                || !triangle
                    .uv
                    .into_iter()
                    .flatten()
                    .all(|v| (0.0..=1.0).contains(&v))
            {
                return Err(
                    "surface mask requires finite positions and UVs inside the unit square".into(),
                );
            }
            if edge(triangle.uv[0], triangle.uv[1], triangle.uv[2]).abs() <= 1e-15 {
                return Err("surface mask requires nondegenerate UV triangles".into());
            }
        }
        if let Some(image) = &self.image {
            crate::paint_masks::alpha_pixels(image)?;
        }
        Ok(())
    }

    pub(super) fn raster(&self, width: usize, height: usize) -> Result<Vec<f64>, String> {
        let sample: Box<dyn Fn([f64; 3]) -> f64> = match self.shape {
            Shape::Band {
                center,
                axis,
                width,
                feather,
            } => {
                let axis = unit(axis).unwrap();
                Box::new(move |point| {
                    let distance = dot(sub(point, center), axis).abs() - width * 0.5;
                    if distance <= 0.0 {
                        1.0
                    } else if feather == 0.0 {
                        0.0
                    } else {
                        (1.0 - distance / feather).clamp(0.0, 1.0)
                    }
                })
            }
            Shape::Gradient { start, end } => {
                let delta = sub(end, start);
                let scale = dot(delta, delta);
                Box::new(move |point| (dot(sub(point, start), delta) / scale).clamp(0.0, 1.0))
            }
            Shape::Decal {
                center,
                right,
                up,
                size,
                depth,
            } => {
                let (right, up, normal) = decal_basis(right, up).unwrap();
                let image = self.image.as_ref().unwrap();
                let alpha = crate::paint_masks::alpha_pixels(image)?;
                Box::new(move |point| {
                    let delta = sub(point, center);
                    let uv = [
                        dot(delta, right) / size[0] + 0.5,
                        dot(delta, up) / size[1] + 0.5,
                    ];
                    if dot(delta, normal).abs() > depth * 0.5
                        || !uv.into_iter().all(|v| (0.0..=1.0).contains(&v))
                    {
                        return 0.0;
                    }
                    crate::paint_masks::sample(&alpha, uv, image.width, image.height)
                })
            }
        };
        let mut pixels = vec![0.0_f64; width * height];
        let mut work = 0;
        for triangle in &self.triangles {
            if let Shape::Decal { right, up, .. } = self.shape {
                let (_, _, direction) = decal_basis(right, up).unwrap();
                let [a, b, c] = triangle.positions;
                if dot(cross(sub(b, a), sub(c, a)), direction) <= 0.0 {
                    continue;
                }
            }
            let [a, b, c] = triangle
                .uv
                .map(|[u, v]| [u * width as f64, (1.0 - v) * height as f64]);
            let left = a[0].min(b[0]).min(c[0]).floor() as usize;
            let right = a[0].max(b[0]).max(c[0]).ceil() as usize;
            let top = a[1].min(b[1]).min(c[1]).floor() as usize;
            let bottom = a[1].max(b[1]).max(c[1]).ceil() as usize;
            work += (right - left) * (bottom - top);
            if work > 32 * 1024 * 1024 {
                return Err("surface mask exceeds raster work limit".into());
            }
            let area = edge(a, b, c);
            for y in top..bottom {
                for x in left..right {
                    let texel = [x as f64 + 0.5, y as f64 + 0.5];
                    let weights = [
                        edge(b, c, texel) / area,
                        edge(c, a, texel) / area,
                        edge(a, b, texel) / area,
                    ];
                    if weights.iter().any(|weight| *weight < -1e-9) {
                        continue;
                    }
                    let point = std::array::from_fn(|axis| {
                        (0..3)
                            .map(|i| weights[i] * triangle.positions[i][axis])
                            .sum()
                    });
                    let index = y * width + x;
                    pixels[index] = pixels[index].max(sample(point));
                }
            }
        }
        Ok(pixels)
    }
}

fn snapshot(mesh: &Mesh, request: &Value) -> Result<Vec<Triangle>, String> {
    let mut triangles = Vec::new();
    for face in faces(mesh, request)? {
        if !uv::complete(mesh, face) {
            return Err("surface mask requires complete UVs".into());
        }
        let vertices = &mesh.faces[&face];
        for indices in normals::triangles(mesh, vertices) {
            let ids = indices.map(|index| vertices[index]);
            triangles.push(Triangle {
                positions: ids.map(|id| mesh.vertices[&id]),
                uv: ids.map(|id| mesh.uvs[&face][&id]),
            });
            if triangles.len() > 4096 {
                return Err("surface mask requires at most 4096 triangles".into());
            }
        }
    }
    Ok(triangles)
}

fn unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(vector, vector).sqrt();
    (length.is_finite() && length > 1e-9).then(|| mul(vector, 1.0 / length))
}

fn decal_basis(right: [f64; 3], up: [f64; 3]) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
    let right = unit(right)?;
    let normal = unit(cross(right, unit(up)?))?;
    Some((right, cross(normal, right), normal))
}

fn edge(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
