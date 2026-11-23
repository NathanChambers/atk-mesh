use crate::{Mesh, geometry::*, material, normals};
use rstar::{AABB, RTree, RTreeObject};
use serde_json::{Value, json};

#[derive(Clone)]
struct Triangle {
    points: [[f64; 3]; 3],
    normals: [[f64; 3]; 3],
}

impl RTreeObject for Triangle {
    type Envelope = AABB<[f64; 3]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_points(self.points.iter())
    }
}

fn blend(tri: [[f64; 3]; 3], weights: [f64; 3]) -> [f64; 3] {
    add(
        add(mul(tri[0], weights[0]), mul(tri[1], weights[1])),
        mul(tri[2], weights[2]),
    )
}

fn project(
    tree: &RTree<Triangle>,
    p: [f64; 3],
    n: [f64; 3],
    reach: f64,
) -> Option<([f64; 3], f64)> {
    let offset = mul(n, reach);
    let start = sub(p, offset);
    let end = add(p, offset);
    let bounds = AABB::from_corners(start, end);
    let mut nearest = f64::INFINITY;
    let mut result = None;
    for tri in tree.locate_in_envelope_intersecting(bounds) {
        let Some(hit) = segment_triangle(start, end, tri.points, 1e-10) else {
            continue;
        };
        let distance = dot(sub(hit, p), n);
        if distance.abs() >= nearest {
            continue;
        }
        let [a, b, c] = tri.points;
        let ab = sub(b, a);
        let ac = sub(c, a);
        let face = cross(ab, ac);
        if dot(face, n) <= 0.0 {
            continue;
        }
        let delta = sub(hit, a);
        let u = dot(cross(delta, ac), face) / dot(face, face);
        let v = dot(cross(ab, delta), face) / dot(face, face);
        let normal = normals::unit(blend(tri.normals, [1.0 - u - v, u, v]));
        nearest = distance.abs();
        result = Some((normal, distance));
    }
    result
}

pub(super) fn export(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let size = request
        .get("resolution")
        .map_or(Some(1024), Value::as_u64)
        .filter(|v| (1..=4096).contains(v))
        .ok_or("resolution must be 1..4096")? as usize;
    let displacement = match request["channel"].as_str() {
        Some("normal") => false,
        Some("displacement") => true,
        _ => return Err("channel must be normal or displacement".into()),
    };
    let name = request
        .get("texture")
        .map(|v| v.as_str().ok_or("texture must be a name"))
        .transpose()?;
    let (pixels, mut result) = bake(mesh, [size, size], name, request, displacement)?;
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, size as u32, size as u32);
        encoder.set_color(if displacement {
            png::ColorType::Grayscale
        } else {
            png::ColorType::Rgba
        });
        encoder.set_depth(if displacement {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&pixels)
            .map_err(|e| e.to_string())?;
    }
    result["revision"] = json!(revision);
    result["channel"] = request["channel"].clone();
    result["resolution"] = json!(size);
    result["png_base64"] = json!(crate::base64::encode(&png));
    if displacement {
        result["zero_code"] = json!(32768);
        result["units_per_code"] = json!(request["max_distance"].as_f64().unwrap() / 32767.0);
    }
    Ok(result)
}

pub(super) fn bake(
    mesh: &Mesh,
    size: [usize; 2],
    name: Option<&str>,
    request: &Value,
    displacement: bool,
) -> Result<(Vec<u8>, Value), String> {
    let source = request["source"]
        .as_u64()
        .ok_or("source must be the high-resolution object ID")?;
    let target = request["target"]
        .as_u64()
        .ok_or("target must be the UV-mapped object ID")?;
    if source == target {
        return Err("source and target must be different objects".into());
    }
    let high = mesh.objects.get(&source).ok_or("unknown source object")?;
    let low = mesh.objects.get(&target).ok_or("unknown target object")?;
    let reach = request["max_distance"]
        .as_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or("max_distance must be positive in mesh units")?;
    let padding = request
        .get("padding")
        .map_or(Some(2), Value::as_u64)
        .filter(|v| *v <= 16)
        .ok_or("padding must be 0..16 pixels")? as u8;
    let corners = normals::corners(mesh);
    let mut triangles = Vec::new();
    for (id, face) in mesh
        .faces
        .iter()
        .filter(|(_, f)| high.vertices.contains(&f[0]))
    {
        for tri in normals::triangles(mesh, face) {
            triangles.push(Triangle {
                points: tri.map(|i| mesh.vertices[&face[i]]),
                normals: tri.map(|i| corners[id][i]),
            });
        }
    }
    let tree = RTree::bulk_load(triangles);
    let [width, height] = size;
    let neutral: &[u8] = if displacement {
        &[128, 0]
    } else {
        &[128, 128, 255, 255]
    };
    let stride = neutral.len();
    let mut pixels = neutral.repeat(width * height);
    let mut coverage = vec![0u8; width * height];
    let mut borders = Vec::new();
    let (mut covered, mut hits) = (0, 0);
    for (&id, face) in mesh.faces.iter().filter(|(id, f)| {
        low.vertices.contains(&f[0])
            && name.is_none_or(|name| mesh.face_textures.get(id).is_some_and(|s| s == name))
    }) {
        if !crate::uv::complete(mesh, id)
            || mesh.uvs[&id]
                .values()
                .flatten()
                .any(|v| !(0.0..=1.0).contains(v))
        {
            return Err(format!("face {id} needs complete UVs in the unit square"));
        }
        for tri in normals::triangles(mesh, face) {
            let uv = tri.map(|i| mesh.uvs[&id][&face[i]]);
            let points = tri.map(|i| mesh.vertices[&face[i]]);
            let shading = tri.map(|i| corners[&id][i]);
            let frame = material::frame(points, uv)
                .ok_or_else(|| format!("face {id} has degenerate UVs"))?;
            let texels = uv.map(|[u, v]| [u * width as f64, (1.0 - v) * height as f64]);
            let orient = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
                (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
            };
            let [a, b, c] = texels;
            let area = orient(a, b, c);
            let range = |axis: usize, size: usize| {
                let min = texels.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
                let max = texels
                    .iter()
                    .map(|p| p[axis])
                    .fold(f64::NEG_INFINITY, f64::max);
                min.floor() as usize..(max.ceil() as usize).min(size)
            };
            for y in range(1, height) {
                for x in range(0, width) {
                    let sample = [x as f64 + 0.5, y as f64 + 0.5];
                    let w = [
                        orient(b, c, sample) / area,
                        orient(c, a, sample) / area,
                        orient(a, b, sample) / area,
                    ];
                    if w.iter().any(|v| *v < -1e-10) {
                        continue;
                    }
                    let index = y * width + x;
                    let interior = w.iter().all(|v| *v > 1e-10);
                    if coverage[index] != 0 {
                        if interior && coverage[index] == 2 {
                            return Err(format!(
                                "target UVs overlap at texel [{x},{y}]; unwrap or pack the target"
                            ));
                        }
                        continue;
                    }
                    coverage[index] = if interior { 2 } else { 1 };
                    covered += 1;
                    let n = normals::unit(blend(shading, w));
                    let Some((high_normal, distance)) = project(&tree, blend(points, w), n, reach)
                    else {
                        continue;
                    };
                    if displacement {
                        let code = (32768.0 + 32767.0 * (distance / reach).clamp(-1.0, 1.0)).round()
                            as u16;
                        pixels[index * 2..index * 2 + 2].copy_from_slice(&code.to_be_bytes());
                    } else {
                        let tangent = normals::unit(sub(frame[0], mul(n, dot(frame[0], n))));
                        let mut bitangent = cross(n, tangent);
                        if dot(bitangent, frame[1]) < 0.0 {
                            bitangent = mul(bitangent, -1.0);
                        }
                        let local = [tangent, bitangent, n].map(|axis| {
                            ((dot(axis, high_normal) * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round()
                                as u8
                        });
                        pixels[index * 4..index * 4 + 3].copy_from_slice(&local);
                    }
                    if padding > 0 {
                        borders.push(index);
                    }
                    hits += 1;
                }
            }
        }
    }
    if hits == 0 {
        return Err(format!(
            "no normal bake hits across {covered} covered texels; check target assignment, UVs, winding and max_distance"
        ));
    }
    let mut padded = 0;
    for _ in 0..padding {
        let mut next = Vec::new();
        for index in borders {
            let x = index % width;
            let y = index / width;
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx >= width || ny >= height {
                    continue;
                }
                let dest = ny * width + nx;
                if coverage[dest] == 0 {
                    pixels.copy_within(index * stride..index * stride + stride, dest * stride);
                    coverage[dest] = 3;
                    next.push(dest);
                }
            }
        }
        padded += next.len();
        borders = next;
    }
    Ok((
        pixels,
        json!({"source":source,"target":target,"covered_texels":covered,"hits":hits,"misses":covered-hits,"padded_texels":padded}),
    ))
}
