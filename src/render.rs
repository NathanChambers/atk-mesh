use crate::{Mesh, View, base64, visible};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Shaded,
    Topology,
    Tessellation,
    Normals,
    Depth,
    Silhouette,
    FaceIds,
    ObjectIds,
    CollectionIds,
    Orientation,
    UvChecker,
    Textured,
    Material,
    VertexColors,
}

pub(super) struct Options {
    pub width: usize,
    pub height: usize,
    mode: Mode,
    pub clean: bool,
    depth_range: Option<[f64; 2]>,
}

impl Options {
    pub fn parse(request: &Value) -> Result<Self, String> {
        let dimension = |key: &str, default: usize| -> Result<usize, String> {
            match request.get(key) {
                Some(value) => value
                    .as_u64()
                    .filter(|value| (64..=4096).contains(value))
                    .map(|value| value as usize)
                    .ok_or_else(|| format!("{key} must be an integer from 64 to 4096")),
                None => Ok(default),
            }
        };
        let width = dimension("width", 800)?;
        let height = dimension("height", 600)?;
        if width * height > 4_194_304 {
            return Err("capture must not exceed 4194304 pixels".into());
        }
        let mode = match request.get("mode").and_then(Value::as_str).unwrap_or("shaded") {
            "shaded" => Mode::Shaded,
            "topology" => Mode::Topology,
            "tessellation" => Mode::Tessellation,
            "normals" => Mode::Normals,
            "depth" => Mode::Depth,
            "silhouette" => Mode::Silhouette,
            "face_ids" => Mode::FaceIds,
            "object_ids" => Mode::ObjectIds,
            "collection_ids" => Mode::CollectionIds,
            "orientation" => Mode::Orientation,
            "uv_checker" => Mode::UvChecker,
            "textured" => Mode::Textured,
            "material" => Mode::Material,
            "vertex_colors" => Mode::VertexColors,
            _ => return Err("unknown render mode: use shaded, topology, tessellation, normals, depth, silhouette, face_ids, object_ids, collection_ids, orientation, uv_checker, textured, material or vertex_colors".into()),
        };
        if request.get("mode").is_some_and(|value| !value.is_string()) {
            return Err("mode must be a string".into());
        }

        let clean = match request.get("clean") {
            Some(value) => value.as_bool().ok_or("clean must be a boolean")?,
            None => mode != Mode::Topology,
        };
        if !clean && !matches!(mode, Mode::Shaded | Mode::Topology) {
            return Err("annotations are only available in shaded and topology modes".into());
        }

        let depth_range = match request.get("depth_range") {
            Some(value) => {
                let range: [f64; 2] = serde_json::from_value(value.clone())
                    .map_err(|_| "depth_range must be [far_z, near_z]")?;
                if range.iter().any(|v| !v.is_finite()) || range[0] >= range[1] {
                    return Err("depth_range must contain finite increasing values".into());
                }
                Some(range)
            }
            None => None,
        };
        Ok(Self {
            width,
            height,
            mode,
            clean,
            depth_range,
        })
    }

    pub fn name(&self) -> &'static str {
        match self.mode {
            Mode::Shaded => "shaded",
            Mode::Topology => "topology",
            Mode::Tessellation => "tessellation",
            Mode::Normals => "normals",
            Mode::Depth => "depth",
            Mode::Silhouette => "silhouette",
            Mode::FaceIds => "face_ids",
            Mode::ObjectIds => "object_ids",
            Mode::CollectionIds => "collection_ids",
            Mode::Orientation => "orientation",
            Mode::UvChecker => "uv_checker",
            Mode::Textured => "textured",
            Mode::Material => "material",
            Mode::VertexColors => "vertex_colors",
        }
    }
}

struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    depths: Vec<f64>,
    face_ids: Vec<u64>,
    occupied: Vec<bool>,
}

#[derive(Clone)]
pub(crate) struct CaptureVisibility {
    vertices: BTreeSet<u64>,
    solo: Option<BTreeSet<u64>>,
    hidden: BTreeSet<u64>,
    visible_count: usize,
    total_count: usize,
}

impl CaptureVisibility {
    pub fn parse(request: &Value, mesh: &Mesh) -> Result<Self, String> {
        let ids = |key: &str| -> Result<Option<BTreeSet<u64>>, String> {
            let Some(value) = request.get(key) else {
                return Ok(None);
            };
            let values: Vec<u64> = serde_json::from_value(value.clone())
                .map_err(|_| format!("{key} must be an array of object IDs"))?;
            for id in &values {
                if !mesh.objects.contains_key(id) {
                    return Err(format!("unknown capture object {id} in {key}"));
                }
            }
            Ok(Some(values.into_iter().collect()))
        };
        let mut solo = ids("solo_objects")?;
        let mut hidden = ids("hide_objects")?.unwrap_or_default();
        for (key, is_solo) in [("solo_collections", true), ("hide_collections", false)] {
            let Some(value) = request.get(key) else {
                continue;
            };
            let collections: Vec<u64> = serde_json::from_value(value.clone())
                .map_err(|_| format!("{key} must be an array of collection IDs"))?;
            let mut objects = BTreeSet::new();
            for id in collections {
                objects.extend(crate::collections::members(mesh, &json!(id))?);
            }
            if is_solo {
                solo = Some(match solo {
                    Some(existing) => existing.intersection(&objects).copied().collect(),
                    None => objects,
                });
            } else {
                hidden.extend(objects);
            }
        }
        let mut vertices = BTreeSet::new();
        let mut visible_count = 0;
        for (id, object) in &mesh.objects {
            let visible =
                !hidden.contains(id) && solo.as_ref().is_none_or(|objects| objects.contains(id));
            if visible {
                visible_count += 1;
            }
            let keep_vertices = if solo.is_some() { visible } else { !visible };
            if keep_vertices {
                vertices.extend(&object.vertices);
            }
        }
        Ok(Self {
            vertices,
            solo,
            hidden,
            visible_count,
            total_count: mesh.objects.len(),
        })
    }

    pub fn includes_vertex(&self, id: &u64) -> bool {
        if self.solo.is_some() {
            self.vertices.contains(id)
        } else {
            !self.vertices.contains(id)
        }
    }

    pub fn metadata(&self) -> Value {
        json!({"solo_objects":self.solo,"hide_objects":self.hidden,"visible_object_count":self.visible_count,"total_object_count":self.total_count})
    }
}

pub(super) fn capture(mesh: &Mesh, view: &View, options: &Options) -> Result<Value, String> {
    let width = options.width;
    let height = options.height;
    let background = match options.mode {
        Mode::Depth | Mode::FaceIds | Mode::ObjectIds | Mode::CollectionIds => [0, 0, 0],
        Mode::Normals => [32, 32, 32],
        Mode::Silhouette => [255, 255, 255],
        _ => [232, 232, 232],
    };
    let mut canvas = Canvas {
        width,
        height,
        pixels: background.repeat(width * height),
        depths: vec![f64::NEG_INFINITY; width * height],
        face_ids: vec![0; width * height],
        occupied: vec![false; width * height],
    };
    let minimum = view
        .points
        .values()
        .map(|p| view.camera.linear_depth(p[2]))
        .fold(f64::INFINITY, f64::min);
    let mut maximum = view
        .points
        .values()
        .map(|p| view.camera.linear_depth(p[2]))
        .fold(f64::NEG_INFINITY, f64::max);
    if view.camera.perspective()
        && mesh
            .faces
            .values()
            .filter(|face| view.visibility.includes_vertex(&face[0]))
            .any(|face| {
                (0..face.len()).any(|index| {
                    view.camera.contains(mesh.vertices[&face[index]])
                        != view
                            .camera
                            .contains(mesh.vertices[&face[(index + 1) % face.len()]])
                })
            })
    {
        maximum = view.camera.near_depth();
    }
    let range = options.depth_range.unwrap_or(if minimum.is_finite() {
        [minimum, maximum]
    } else {
        [0.0, 0.0]
    });
    let mut palette = BTreeMap::new();
    let mut used_colors = BTreeSet::new();
    let mut object_palette = BTreeMap::new();
    let mut object_colors = BTreeMap::new();
    let mut owners = BTreeMap::new();
    if matches!(options.mode, Mode::ObjectIds | Mode::CollectionIds) {
        let mut used = BTreeSet::new();
        for (index, (id, object)) in mesh.objects.iter().enumerate() {
            let color = categorical_color(index, &mut used)?;
            object_colors.insert(*id, color);
            object_palette.insert(
                format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]),
                *id,
            );
            for vertex in &object.vertices {
                owners.insert(*vertex, *id);
            }
        }
    }
    let collection_palette = if options.mode == Mode::CollectionIds {
        Some(crate::collections::colors(mesh)?)
    } else {
        None
    };
    let corner_normals = crate::normals::corners(mesh);
    let materials: BTreeMap<_, _> = if options.mode == Mode::Material {
        mesh.textures
            .iter()
            .map(|(name, texture)| Ok((name.clone(), crate::material::Prepared::new(texture)?)))
            .collect::<Result<_, String>>()?
    } else {
        BTreeMap::new()
    };
    let textures: BTreeMap<_, _> = if options.mode == Mode::Textured {
        mesh.textures
            .iter()
            .map(|(name, texture)| Ok((name.clone(), crate::uv::pixels(texture)?)))
            .collect::<Result<_, String>>()?
    } else {
        BTreeMap::new()
    };
    let mut edges = BTreeMap::<(u64, u64), Vec<u64>>::new();
    let mut triangle_count = 0;
    let mut nonplanar_count = 0;
    let mut nonplanar_sample = Vec::new();
    for (index, (id, face)) in mesh.faces.iter().enumerate() {
        let complete_uv = crate::uv::complete(mesh, *id);
        let texture = if options.mode == Mode::Textured {
            mesh.face_textures
                .get(id)
                .map(|name| (&mesh.textures[name], &textures[name]))
        } else {
            None
        };
        let normal = face_normal(mesh, face);
        let material = mesh
            .face_textures
            .get(id)
            .and_then(|name| materials.get(name));
        let center = std::array::from_fn(|axis| {
            face.iter()
                .map(|id| mesh.vertices[id][axis] / face.len() as f64)
                .sum()
        });
        let toward_camera = view.camera.toward(center);
        let face_color = if options.mode == Mode::CollectionIds {
            mesh.primary_collections
                .get(&owners[&face[0]])
                .map(|id| collection_palette.as_ref().unwrap().colors[id])
                .unwrap_or([128, 128, 128])
        } else if options.mode == Mode::ObjectIds {
            object_colors[&owners[&face[0]]]
        } else {
            categorical_color(index, &mut used_colors)?
        };
        if options.mode == Mode::FaceIds {
            palette.insert(
                format!(
                    "#{:02x}{:02x}{:02x}",
                    face_color[0], face_color[1], face_color[2]
                ),
                *id,
            );
        }
        if !view.visibility.includes_vertex(&face[0]) {
            continue;
        }

        if options.mode == Mode::Tessellation && face.len() > 3 {
            let origin = mesh.vertices[&face[0]];
            let mut scale: f64 = 0.0;
            let mut deviation: f64 = 0.0;
            for vertex in face {
                let position = mesh.vertices[vertex];
                let offset = std::array::from_fn(|axis| position[axis] - origin[axis]);
                scale = scale.max(offset[0].hypot(offset[1]).hypot(offset[2]));
                deviation = deviation.max(dot(offset, normal).abs());
            }
            if deviation > scale * 1e-6 {
                nonplanar_count += 1;
                if nonplanar_sample.len() < 8 {
                    nonplanar_sample.push(*id);
                }
            }
        }
        for [i0, i1, i2] in crate::normals::triangles(mesh, face) {
            if options.mode == Mode::Tessellation {
                triangle_count += 1;
                let triangle_edges = [(i0, i1), (i1, i2), (i2, i0)];
                for (start, end) in triangle_edges {
                    let a = face[start];
                    let b = face[end];
                    let incident = edges.entry((a.min(b), a.max(b))).or_default();
                    if incident.last() != Some(id) {
                        incident.push(*id);
                    }
                }
            }
            let tangent_frame = if options.mode == Mode::Material && complete_uv {
                crate::material::frame(
                    [
                        mesh.vertices[&face[i0]],
                        mesh.vertices[&face[i1]],
                        mesh.vertices[&face[i2]],
                    ],
                    [
                        mesh.uvs[id][&face[i0]],
                        mesh.uvs[id][&face[i1]],
                        mesh.uvs[id][&face[i2]],
                    ],
                )
            } else {
                None
            };
            let triangles = view.camera.clip_triangle([
                mesh.vertices[&face[i0]],
                mesh.vertices[&face[i1]],
                mesh.vertices[&face[i2]],
            ]);
            let normals = [
                corner_normals[id][i0],
                corner_normals[id][i1],
                corner_normals[id][i2],
            ];
            for triangle in triangles {
                let points = [
                    view.camera.raster_point(triangle[0].position)?,
                    view.camera.raster_point(triangle[1].position)?,
                    view.camera.raster_point(triangle[2].position)?,
                ];
                canvas.triangle(points, *id, |depth, mut weights| {
                    if view.camera.perspective() {
                        weights =
                            std::array::from_fn(|index| weights[index] * points[index][2] / depth);
                    }
                    let original_weights: [f64; 3] = std::array::from_fn(|axis| {
                        (0..3)
                            .map(|index| weights[index] * triangle[index].weights[axis])
                            .sum()
                    });
                    if matches!(
                        options.mode,
                        Mode::UvChecker | Mode::Textured | Mode::VertexColors | Mode::Material
                    ) {
                        let vertices = [face[i0], face[i1], face[i2]];
                        let paint: [f64; 3] = std::array::from_fn(|axis| {
                            (0..3)
                                .map(|corner| {
                                    mesh.vertex_colors
                                        .get(&vertices[corner])
                                        .unwrap_or(&[1.0; 3])[axis]
                                        * original_weights[corner]
                                })
                                .sum()
                        });
                        if options.mode == Mode::VertexColors {
                            return Some(paint
                                .map(|channel| (channel * 255.0).round().clamp(0.0, 255.0) as u8));
                        }
                        if !complete_uv {
                            return Some([255, 0, 255]);
                        }
                        let uv = std::array::from_fn(|axis| {
                            (0..3)
                                .map(|corner| {
                                    mesh.uvs[id][&vertices[corner]][axis] * original_weights[corner]
                                })
                                .sum()
                        });
                        if options.mode == Mode::Material {
                            if material.is_some_and(|material| material.masked(uv)) {
                                return None;
                            }
                            let smooth = crate::normals::unit(std::array::from_fn(|axis| {
                                (0..3)
                                    .map(|corner| normals[corner][axis] * original_weights[corner])
                                    .sum()
                            }));
                            let position = std::array::from_fn(|axis| {
                                (0..3)
                                    .map(|corner| {
                                        mesh.vertices[&vertices[corner]][axis]
                                            * original_weights[corner]
                                    })
                                    .sum()
                            });
                            let toward = view.camera.toward(position);
                            return Some(crate::material::shade(
                                material,
                                uv,
                                paint,
                                smooth,
                                tangent_frame,
                                toward,
                            ));
                        }
                        return Some(if options.mode == Mode::UvChecker {
                            crate::uv::checker(uv)
                        } else if let Some((texture, data)) = texture {
                            let index = crate::uv::texel_offset(texture.width, texture.height, uv);
                            if crate::uv::below_cutoff(texture.alpha_cutoff, data[index + 3]) {
                                return None;
                            }
                            let color = crate::uv::sample(texture, data, uv, false);
                            std::array::from_fn(|axis| {
                                (f64::from(color[axis]) * paint[axis])
                                    .round()
                                    .clamp(0.0, 255.0) as u8
                            })
                        } else {
                            [128; 3]
                        });
                    }
                    let smooth = crate::normals::unit(std::array::from_fn(|axis| {
                        (0..3).map(|i| normals[i][axis] * original_weights[i]).sum()
                    }));
                    let normal = if options.mode == Mode::Orientation {
                        normal
                    } else {
                        smooth
                    };
                    Some(shade(
                        normal,
                        view.camera.linear_depth(depth),
                        toward_camera,
                        options.mode,
                        range,
                        face_color,
                    ))
                });
            }
        }
    }
    if options.mode == Mode::Topology || !options.clean {
        for (id, face) in &mesh.faces {
            if !view.visibility.includes_vertex(&face[0]) {
                continue;
            }
            for i in 0..face.len() {
                let a = face[i];
                let b = face[(i + 1) % face.len()];
                edges.entry((a.min(b), a.max(b))).or_default().push(*id);
            }
        }
    }
    for ((a, b), faces) in edges {
        if let Some([first, second]) = view
            .camera
            .raster_segment(mesh.vertices[&a], mesh.vertices[&b])?
        {
            canvas.edge(first, second, &faces);
        }
    }
    if !options.clean {
        for (id, point) in &view.points {
            let on_screen = point[0] >= 0.0
                && point[0] < width as f64
                && point[1] >= 0.0
                && point[1] < height as f64;
            if !on_screen || !visible(*id, view, mesh) {
                continue;
            }

            let color = if mesh.selection.contains(id) {
                [204, 112, 16]
            } else {
                [35, 35, 35]
            };
            canvas.marker(*point, color);
            canvas.label(
                point[0] as i32 + 6,
                point[1] as i32 - 11,
                &id.to_string(),
                color,
            );
        }
        for (id, face) in &mesh.faces {
            if !view.visibility.includes_vertex(&face[0]) {
                continue;
            }
            let position: [f64; 3] = std::array::from_fn(|axis| {
                face.iter()
                    .map(|id| mesh.vertices[id][axis] / face.len() as f64)
                    .sum()
            });
            if !view.camera.contains(position) {
                continue;
            }
            let center = view.camera.raster_point(position)?;
            if canvas.is_visible(center, Some(*id)) {
                canvas.label(
                    center[0] as i32,
                    center[1] as i32,
                    &format!("f{id}"),
                    [45, 45, 45],
                );
            }
        }
    }
    let mut png = vec![];
    {
        let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&canvas.pixels)
            .map_err(|error| error.to_string())?;
    }
    let encoded = base64::encode(&png);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><image width=\"{width}\" height=\"{height}\" href=\"data:image/png;base64,{encoded}\"/></svg>"
    );
    let mut result = json!({"svg":svg, "png_base64":encoded, "mode":options.name(), "clean":options.clean, "background_rgb":background, "depth_range":range, "depth_encoding":"linear camera-space z: far=32, near=255; background=0; larger z is nearer", "normal_space":"world", "normal_encoding":"RGB = round(255 * (normal + 1) / 2); no lighting or gamma", "face_id_colors":palette, "object_id_colors":object_palette, "collection_id_colors":if options.mode == Mode::CollectionIds {json!(collection_palette.as_ref().unwrap().legend)} else {json!({})}, "unassigned_collection_rgb":[128,128,128], "projection":if view.camera.perspective() {"perspective"} else {"orthographic"}});
    if options.mode == Mode::Tessellation {
        result["tessellation"] = json!({"triangle_count":triangle_count,"nonplanar_face_count":nonplanar_count,"nonplanar_face_sample":nonplanar_sample});
    }
    Ok(result)
}

pub(super) fn categorical_color(index: usize, used: &mut BTreeSet<u32>) -> Result<[u8; 3], String> {
    if used.len() >= 0xffffff {
        return Err("too many faces for unique 24-bit face colors".into());
    }

    let hue = (index as f64 * 0.618033988749895).fract() * 6.0;
    let sector = hue.floor() as usize;
    let fraction = hue - hue.floor();
    let value = 0.95;
    let p = value * (1.0 - 0.78);
    let q = value * (1.0 - 0.78 * fraction);
    let t = value * (1.0 - 0.78 * (1.0 - fraction));
    let rgb = match sector {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    };
    let rgb = rgb.map(|channel| (channel * 255.0).round() as u32);
    let mut code = (rgb[0] << 16) | (rgb[1] << 8) | rgb[2];
    if used.contains(&code) {
        code = ((index as u32 + 1).wrapping_mul(0x9e3779)) & 0xffffff;
    }
    while code == 0 || !used.insert(code) {
        code = (code + 1) & 0xffffff;
    }
    Ok([(code >> 16) as u8, (code >> 8) as u8, code as u8])
}

fn shade(
    mut normal: [f64; 3],
    depth: f64,
    toward_camera: [f64; 3],
    mode: Mode,
    range: [f64; 2],
    face_color: [u8; 3],
) -> [u8; 3] {
    let front = dot(normal, toward_camera) >= 0.0;
    match mode {
        Mode::Normals => {
            normal.map(|value| ((value + 1.0) * 127.5).round().clamp(0.0, 255.0) as u8)
        }
        Mode::Depth => {
            let fraction = if range[1] > range[0] {
                (depth - range[0]) / (range[1] - range[0])
            } else {
                0.5
            };
            let value = (32.0 + 223.0 * fraction.clamp(0.0, 1.0)).round() as u8;
            [value; 3]
        }
        Mode::Silhouette => [0, 0, 0],
        Mode::UvChecker | Mode::Textured | Mode::VertexColors | Mode::Material => [255, 0, 255],
        Mode::FaceIds | Mode::ObjectIds | Mode::CollectionIds => face_color,
        Mode::Orientation => {
            if front {
                [180; 3]
            } else {
                [220, 55, 40]
            }
        }
        Mode::Shaded | Mode::Topology | Mode::Tessellation => {
            if !front {
                normal = normal.map(|value| -value);
            }
            let key = dot(normal, normalize([-3.0, 5.0, 4.0])).max(0.0);
            let fill = dot(normal, normalize([4.0, 2.0, 1.0])).max(0.0);
            let rim = dot(normal, normalize([1.0, 3.0, -4.0])).max(0.0);
            let ambient = 0.12 + 0.20 * (normal[1] * 0.5 + 0.5);
            let light = ambient + key * 0.8 + fill * 0.27 + rim * 0.38;
            let linear = light * 0.45;
            let value = (255.0 * (linear / (linear + 0.65)).powf(1.0 / 2.2))
                .round()
                .clamp(0.0, 255.0) as u8;
            [value; 3]
        }
    }
}

impl Canvas {
    fn triangle(
        &mut self,
        points: [[f64; 3]; 3],
        face_id: u64,
        color: impl Fn(f64, [f64; 3]) -> Option<[u8; 3]>,
    ) {
        let [a, b, c] = points;
        let denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if !denominator.is_finite() || denominator.abs() < 1e-12 {
            return;
        }

        let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
        let max_x = a[0].max(b[0]).max(c[0]).ceil().min(self.width as f64) as usize;
        let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
        let max_y = a[1].max(b[1]).max(c[1]).ceil().min(self.height as f64) as usize;
        for y in min_y..max_y {
            for x in min_x..max_x {
                let px = x as f64 + 0.5;
                let py = y as f64 + 0.5;
                let u = ((b[1] - c[1]) * (px - c[0]) + (c[0] - b[0]) * (py - c[1])) / denominator;
                let v = ((c[1] - a[1]) * (px - c[0]) + (a[0] - c[0]) * (py - c[1])) / denominator;
                let w = 1.0 - u - v;
                if u < -1e-10 || v < -1e-10 || w < -1e-10 {
                    continue;
                }

                let depth = u * a[2] + v * b[2] + w * c[2];
                let index = y * self.width + x;
                if depth <= self.depths[index] {
                    continue;
                }
                let Some(color) = color(depth, [u, v, w]) else { continue };

                self.depths[index] = depth;
                self.face_ids[index] = face_id;
                self.occupied[index] = true;
                self.pixels[index * 3..index * 3 + 3].copy_from_slice(&color);
            }
        }
    }

    fn is_visible(&self, point: [f64; 3], face_id: Option<u64>) -> bool {
        let x = point[0].floor() as i32;
        let y = point[1].floor() as i32;
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return false;
        }

        let index = y as usize * self.width + x as usize;
        if let Some(face) = face_id {
            return self.occupied[index] && self.face_ids[index] == face;
        }

        let tolerance = point[2].abs().max(self.depths[index].abs()).max(1.0) * 1e-7;
        !self.occupied[index] || point[2] >= self.depths[index] - tolerance
    }

    fn edge(&mut self, a: [f64; 3], b: [f64; 3], incident_faces: &[u64]) {
        let mut low: f64 = 0.0;
        let mut high: f64 = 1.0;
        for (axis, maximum) in [(0, self.width as f64 - 1.0), (1, self.height as f64 - 1.0)] {
            let delta = b[axis] - a[axis];
            if delta.abs() < 1e-12 {
                if a[axis] < 0.0 || a[axis] > maximum {
                    return;
                }
                continue;
            }
            let start = -a[axis] / delta;
            let end = (maximum - a[axis]) / delta;
            low = low.max(start.min(end));
            high = high.min(start.max(end));
        }
        if low > high {
            return;
        }

        let length = (b[0] - a[0]).abs().max((b[1] - a[1]).abs()) * (high - low);
        let steps = length
            .ceil()
            .clamp(1.0, (self.width + self.height) as f64 * 2.0) as usize;
        for step in 0..=steps {
            let fraction = low + (high - low) * step as f64 / steps as f64;
            let point = std::array::from_fn(|axis| a[axis] * (1.0 - fraction) + b[axis] * fraction);
            let x = point[0].floor() as i32;
            let y = point[1].floor() as i32;
            if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
                continue;
            }
            let index = y as usize * self.width + x as usize;
            if incident_faces.contains(&self.face_ids[index]) || self.is_visible(point, None) {
                self.pixel(point[0].floor() as i32, point[1].floor() as i32, [45; 3]);
            }
        }
    }

    fn pixel(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }

        let index = (y as usize * self.width + x as usize) * 3;
        self.pixels[index..index + 3].copy_from_slice(&color);
    }

    fn marker(&mut self, point: [f64; 3], color: [u8; 3]) {
        for dy in -3..=3 {
            for dx in -3..=3 {
                if dx * dx + dy * dy <= 9 {
                    self.pixel(point[0] as i32 + dx, point[1] as i32 + dy, color);
                }
            }
        }
    }

    fn label(&mut self, x: i32, y: i32, text: &str, color: [u8; 3]) {
        const DIGITS: [u16; 10] = [
            0b111_101_101_101_111,
            0b010_110_010_010_111,
            0b111_001_111_100_111,
            0b111_001_111_001_111,
            0b101_101_111_001_001,
            0b111_100_111_001_111,
            0b111_100_111_101_111,
            0b111_001_001_001_001,
            0b111_101_111_101_111,
            0b111_101_111_001_111,
        ];
        for (index, character) in text.chars().enumerate() {
            let glyph = if character == 'f' {
                0b011_010_111_010_010
            } else if let Some(digit) = character.to_digit(10) {
                DIGITS[digit as usize]
            } else {
                continue;
            };
            for row in 0..5 {
                for column in 0..3 {
                    if glyph & (1 << (14 - row * 3 - column)) == 0 {
                        continue;
                    }
                    for dy in 0..2 {
                        for dx in 0..2 {
                            self.pixel(
                                x + index as i32 * 8 + column * 2 + dx,
                                y + row * 2 + dy,
                                color,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn normalize(vector: [f64; 3]) -> [f64; 3] {
    let length = dot(vector, vector).sqrt();
    if length > 0.0 {
        vector.map(|value| value / length)
    } else {
        [0.0; 3]
    }
}
fn face_normal(mesh: &Mesh, face: &[u64]) -> [f64; 3] {
    let origin = mesh.vertices[&face[0]];
    let mut normal = [0.0; 3];
    for i in 1..face.len() - 1 {
        let a: [f64; 3] = std::array::from_fn(|axis| mesh.vertices[&face[i]][axis] - origin[axis]);
        let b: [f64; 3] =
            std::array::from_fn(|axis| mesh.vertices[&face[i + 1]][axis] - origin[axis]);
        for (axis, value) in normal.iter_mut().enumerate() {
            let j = (axis + 1) % 3;
            let k = (axis + 2) % 3;
            *value += a[j] * b[k] - a[k] * b[j];
        }
    }
    normalize(normal)
}
