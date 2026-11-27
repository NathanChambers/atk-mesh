use super::{inside, parse_camera, read_png, reference_coverage};
use crate::{Document, comparison, objects, projection::Camera, render};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Cursor, path::Path};

struct DepthView {
    camera: Camera,
    depths: Vec<Option<f64>>,
    maximum_jump: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Landmark {
    vertex: u64,
    pixel: [f64; 2],
}

pub(super) fn run(
    command: &str,
    request: &Value,
    obj_output: bool,
) -> Result<(String, Value), String> {
    let view = &request["view"];
    let coverage = reference_coverage(std::slice::from_ref(view))?;
    let depth = DepthView::read(view)?;
    let mut document = Document::empty();
    let mut result = if command == "depth_mesh" {
        build(&depth, &mut document)?
    } else {
        fit(&depth, request, &mut document)?
    };
    crate::validate(&document.mesh)?;

    let artifact = if obj_output {
        let visibility = render::CaptureVisibility::parse(&json!({}), &document.mesh)?;
        document.export_obj(&visibility, None)?
    } else {
        document.execute(json!({"command":"save"}))?.to_string()
    };
    result["reference_coverage"] = coverage;
    Ok((artifact, result))
}

impl DepthView {
    fn read(view: &Value) -> Result<Self, String> {
        let camera = parse_camera(&view["camera"])?;
        let pixels = camera.width * camera.height;
        if pixels > 262_144 {
            return Err("depth reconstruction supports up to 262144 pixels; resize maps and camera together".into());
        }
        let [near, far]: [f64; 2] = serde_json::from_value(view["distance_range"].clone())
            .map_err(|_| "distance_range must be [near, far]")?;
        if !near.is_finite() || !far.is_finite() || near >= far || !(far - near).is_finite() {
            return Err("distance_range must be finite and increasing".into());
        }
        if camera.perspective() && near < -camera.near_depth() {
            return Err("distance_range starts before the camera near plane".into());
        }
        let maximum_jump = positive(view, "maximum_jump", (far - near) * 0.05)?;
        let mask_path = view["mask"]
            .as_str()
            .ok_or("view requires a binary mask PNG path")?;
        let (width, height, mask) = comparison::decode(&read_png(Path::new(mask_path))?)?;
        if [width, height] != [camera.width, camera.height] {
            return Err("mask dimensions must match the camera".into());
        }

        let depth_path = view["depth"]
            .as_str()
            .ok_or("view requires a grayscale depth PNG path")?;
        let bytes = read_png(Path::new(depth_path))?;
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_limits(png::Limits {
            bytes: 8 * 1024 * 1024,
        });
        let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
        let info = reader.info();
        if [info.width as usize, info.height as usize] != [width, height] {
            return Err("depth dimensions must match the camera".into());
        }
        if info.color_type != png::ColorType::Grayscale {
            return Err("depth must be a grayscale PNG".into());
        }
        let bytes_per_sample = match info.bit_depth {
            png::BitDepth::Eight => 1,
            png::BitDepth::Sixteen => 2,
            _ => return Err("depth must have 8 or 16 bits per sample".into()),
        };
        let mut samples = vec![0; reader.output_buffer_size()];
        let output = reader
            .next_frame(&mut samples)
            .map_err(|error| error.to_string())?;
        let depths = samples[..output.buffer_size()]
            .chunks_exact(bytes_per_sample)
            .zip(mask.chunks_exact(3))
            .map(|(sample, mask)| {
                let fraction = match sample {
                    [high, low] => u16::from_be_bytes([*high, *low]) as f64 / 65535.0,
                    [value] => *value as f64 / 255.0,
                    _ => unreachable!(),
                };
                (mask[0] >= 128).then_some(near + fraction * (far - near))
            })
            .collect();
        Ok(Self {
            camera,
            depths,
            maximum_jump,
        })
    }

    fn sample(&self, pixel: [f64; 2]) -> Option<f64> {
        if !inside(&self.camera, pixel) {
            return None;
        }
        self.depths[pixel[1] as usize * self.camera.width + pixel[0] as usize]
    }

    fn point(&self, pixel: [f64; 2], distance: f64) -> [f64; 3] {
        let (origin, direction) = self.camera.ray(pixel);
        std::array::from_fn(|axis| origin[axis] + direction[axis] * distance)
    }
}

fn positive(options: &Value, name: &str, default: f64) -> Result<f64, String> {
    match options.get(name) {
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite() && *number > 0.0)
            .ok_or_else(|| format!("{name} must be positive and finite")),
        None => Ok(default),
    }
}

fn build(depth: &DepthView, document: &mut Document) -> Result<Value, String> {
    let width = depth.camera.width;
    let height = depth.camera.height;
    let mut vertices = vec![None; depth.depths.len()];
    for y in 0..height.saturating_sub(1) {
        for x in 0..width.saturating_sub(1) {
            let top_left = y * width + x;
            let corners = [
                top_left,
                top_left + 1,
                top_left + width,
                top_left + width + 1,
            ];
            let [a, b, c, d] = corners.map(|index| depth.depths[index]);
            let jump = |first: Option<f64>, second: Option<f64>| match (first, second) {
                (Some(first), Some(second)) => (first - second).abs(),
                _ => f64::INFINITY,
            };
            let triangles = if jump(a, d) < jump(b, c) {
                [[0, 2, 3], [0, 3, 1]]
            } else {
                [[0, 2, 1], [1, 2, 3]]
            };
            for triangle in triangles {
                let indices = triangle.map(|index| corners[index]);
                let [Some(a), Some(b), Some(c)] = indices.map(|index| depth.depths[index]) else {
                    continue;
                };
                if a.max(b).max(c) - a.min(b).min(c) > depth.maximum_jump {
                    continue;
                }

                let mut face = Vec::with_capacity(3);
                for (index, distance) in indices.into_iter().zip([a, b, c]) {
                    let id = match vertices[index] {
                        Some(id) => id,
                        None => {
                            let pixel =
                                [(index % width) as f64 + 0.5, (index / width) as f64 + 0.5];
                            let id = document.mesh.add_vertex(depth.point(pixel, distance))?;
                            vertices[index] = Some(id);
                            id
                        }
                    };
                    face.push(id);
                }
                document.mesh.add_face(face)?;
            }
        }
    }
    if document.mesh.faces.is_empty() {
        return Err("mask and depth map yielded no triangles within maximum_jump".into());
    }
    objects::initialize(&mut document.mesh);
    Ok(
        json!({"vertices":document.mesh.vertices.len(),"faces":document.mesh.faces.len(),"surface":"open"}),
    )
}

fn fit(depth: &DepthView, request: &Value, document: &mut Document) -> Result<Value, String> {
    let project_path = request["project"]
        .as_str()
        .ok_or("fit_depth requires a saved project JSON path")?;
    let project = read_png(Path::new(project_path))?;
    let project: Value = serde_json::from_slice(&project).map_err(|error| error.to_string())?;
    document.execute(json!({"command":"load","project":project}))?;
    let ids: BTreeSet<u64> = serde_json::from_value(request["vertices"].clone())
        .map_err(|_| "fit_depth requires explicit vertex IDs")?;
    if ids.is_empty() {
        return Err("select at least one vertex for fitting".into());
    }
    let strength = positive(request, "strength", 0.5)?;
    if strength > 1.0 {
        return Err("strength must be at most 1".into());
    }
    let maximum_move = positive(request, "maximum_move", depth.maximum_jump)?;
    let landmarks: Vec<Landmark> = request
        .get("landmarks")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|error| format!("invalid landmarks: {error}"))?
        .unwrap_or_default();
    let mut targets = std::collections::BTreeMap::new();
    for landmark in landmarks {
        if !ids.contains(&landmark.vertex)
            || targets.insert(landmark.vertex, landmark.pixel).is_some()
        {
            return Err("landmarks must target distinct selected vertices".into());
        }
        if depth.sample(landmark.pixel).is_none() {
            return Err("landmarks must target foreground pixels".into());
        }
    }

    let mut camera = depth.camera.metadata();
    camera["mode"] = json!("depth");
    let view = crate::View::project(&document.mesh, &camera, 0)?;
    let options = render::Options::parse(&camera)?;
    let visibility = render::capture_depths(&document.mesh, &view, &options)?;
    let visibility_tolerance = depth.maximum_jump * 0.01;

    let mut fitted = 0;
    let mut skipped = 0;
    let mut residual_before = 0.0;
    let mut residual_after = 0.0;
    for id in ids {
        let position = document
            .mesh
            .vertices
            .get_mut(&id)
            .ok_or_else(|| format!("unknown vertex {id}"))?;
        let Ok([x, y, z]) = depth.camera.project(*position) else {
            skipped += 1;
            continue;
        };
        if !inside(&depth.camera, [x, y]) {
            skipped += 1;
            continue;
        }
        let index = y as usize * depth.camera.width + x as usize;
        let visible_z = depth.camera.linear_depth(visibility[index]);
        let pixel = targets.get(&id).copied().unwrap_or([x, y]);
        let Some(distance) = depth.sample(pixel) else {
            skipped += 1;
            continue;
        };
        if visibility[index].is_finite() && visible_z - z > visibility_tolerance {
            skipped += 1;
            continue;
        }
        let target = depth.point(pixel, distance);
        let delta: [f64; 3] = std::array::from_fn(|axis| target[axis] - position[axis]);
        let length = delta[0].hypot(delta[1]).hypot(delta[2]);
        if !length.is_finite() {
            return Err("fit exceeds numeric range".into());
        }
        let fraction = strength.min(maximum_move / length);
        for (coordinate, delta) in position.iter_mut().zip(delta) {
            *coordinate += delta * fraction;
        }
        residual_before += length;
        residual_after += length * (1.0 - fraction);
        fitted += 1;
    }
    if fitted == 0 {
        return Err("no selected vertices were visible over foreground depth samples".into());
    }
    Ok(json!({
        "fitted_vertices":fitted,"skipped_vertices":skipped,
        "mean_target_distance_before":residual_before / fitted as f64,
        "mean_target_distance_after":residual_after / fitted as f64
    }))
}
