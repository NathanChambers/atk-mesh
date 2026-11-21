use crate::{Mesh, base64};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub rgba_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_cutoff: Option<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub maps: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<crate::painting::PaintStack>,
}

pub(super) fn faces(mesh: &Mesh, request: &Value) -> Result<Vec<u64>, String> {
    let object = request
        .get("object")
        .map(|value| {
            let id = value.as_u64().ok_or("object must be an ID")?;
            mesh.objects.get(&id).ok_or("unknown object")
        })
        .transpose()?;
    let requested: Option<BTreeSet<u64>> = request
        .get("faces")
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|_| "faces must be an array of IDs")
        })
        .transpose()?;
    if let Some(ids) = &requested {
        for id in ids {
            let face = mesh.faces.get(id).ok_or("unknown face")?;
            if object.is_some_and(|object| !object.vertices.contains(&face[0])) {
                return Err("face is outside the requested object".into());
            }
        }
    }
    let result: Vec<_> = mesh
        .faces
        .iter()
        .filter(|(id, face)| {
            requested.as_ref().is_none_or(|ids| ids.contains(id))
                && object.is_none_or(|object| object.vertices.contains(&face[0]))
        })
        .map(|(id, _)| *id)
        .collect();
    if result.is_empty() {
        return Err("no faces in the requested scope".into());
    }
    crate::appearance_tools::filter_faces(mesh, request, result)
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    if command == "texture_define" {
        if request.get("object").is_some() {
            return Err("texture definitions are project resources; omit object".into());
        }
        let name = request["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty() && name.len() <= 128)
            .ok_or("name must contain 1..128 bytes")?;
        let texture: Texture = serde_json::from_value(request.clone())
            .map_err(|_| "texture requires width, height, rgba_base64 and an optional numeric alpha_cutoff")?;
        pixels(&texture)?;
        let total: usize = mesh
            .textures
            .iter()
            .filter(|(id, _)| id.as_str() != name)
            .map(|(_, texture)| texture.width * texture.height)
            .sum();
        if total + texture.width * texture.height > 4_194_304 {
            return Err("project textures must not exceed 4194304 pixels in total".into());
        }
        mesh.textures.insert(name.into(), texture);
        return Ok(json!({"texture":name,"replaced_or_created":true}));
    }
    if command == "texture_remove" {
        if request.get("object").is_some() {
            return Err("texture definitions are project resources; omit object".into());
        }
        let name = request["texture"].as_str().ok_or("texture is required")?;
        if mesh.face_textures.values().any(|id| id == name) {
            return Err("texture is assigned to faces; clear their assignments first".into());
        }
        mesh.textures.remove(name).ok_or("unknown texture")?;
        return Ok(json!({"texture":name,"removed":true}));
    }

    let targets = faces(mesh, request)?;
    if command == "texture_assign" {
        let texture = match request.get("texture") {
            Some(Value::Null) => None,
            Some(Value::String(name)) if mesh.textures.contains_key(name) => Some(name),
            _ => {
                return Err(
                    "texture must name an existing texture, or null to clear assignments".into(),
                );
            }
        };
        for face in &targets {
            if let Some(name) = texture {
                mesh.face_textures.insert(*face, name.clone());
            } else {
                mesh.face_textures.remove(face);
            }
        }
        return Ok(json!({"faces":targets,"texture":texture}));
    }

    if command == "uv_set" {
        let values = request["values"]
            .as_array()
            .filter(|values| !values.is_empty() && values.len() <= 4096)
            .ok_or("values must contain 1..4096 {vertex,uv,face?} entries")?;
        let mut touched = BTreeSet::new();
        for value in values {
            let vertex = value["vertex"].as_u64().ok_or("vertex must be an ID")?;
            let uv: [f64; 2] = serde_json::from_value(value["uv"].clone())
                .map_err(|_| "uv must contain two finite numbers")?;
            let invalid_uv = uv
                .iter()
                .any(|value| !value.is_finite() || value.abs() > 1e6);
            if invalid_uv {
                return Err("UV coordinates must be finite and within -1000000..1000000".into());
            }
            let face = value
                .get("face")
                .map(|id| id.as_u64().ok_or("face must be an ID"))
                .transpose()?;
            if face.is_some_and(|id| !targets.contains(&id) || !mesh.faces[&id].contains(&vertex)) {
                return Err(
                    "face/vertex binding is outside the requested scope or not a face corner"
                        .into(),
                );
            }
            let mut count = 0;
            for id in &targets {
                if face.is_some_and(|face| face != *id) || !mesh.faces[id].contains(&vertex) {
                    continue;
                }
                let distinct = touched.insert((*id, vertex));
                if !distinct {
                    return Err(
                        "each face/vertex binding may be assigned only once per command".into(),
                    );
                }
                mesh.uvs.entry(*id).or_default().insert(vertex, uv);
                count += 1;
            }
            if count == 0 {
                return Err("vertex has no face corners in the requested scope".into());
            }
        }
        return Ok(json!({"assigned_corners":touched.len()}));
    }

    let vertices: Option<BTreeSet<u64>> = request
        .get("vertices")
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|_| "vertices must be an array of IDs")
        })
        .transpose()?;
    if let Some(vertices) = &vertices {
        let invalid_vertices = vertices.is_empty()
            || vertices
                .iter()
                .any(|vertex| !targets.iter().any(|face| mesh.faces[face].contains(vertex)));
        if invalid_vertices {
            return Err("vertices must contain existing corners in the requested scope".into());
        }
    }
    let mut count = 0;
    if command == "uv_clear" {
        for face in targets {
            if let Some(values) = mesh.uvs.get_mut(&face) {
                values.retain(|vertex, _| {
                    let remove = vertices.as_ref().is_none_or(|ids| ids.contains(vertex));
                    count += usize::from(remove);
                    !remove
                });
            }
        }
    } else if command == "uv_transform" {
        let pair = |name: &str, default: [f64; 2]| -> Result<[f64; 2], String> {
            let value: [f64; 2] = request
                .get(name)
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|_| format!("{name} must contain two numbers"))?
                .unwrap_or(default);
            if value.iter().any(|value| !value.is_finite()) {
                return Err(format!("{name} must contain finite numbers"));
            }
            Ok(value)
        };
        let offset = pair("offset", [0.0; 2])?;
        let scale = pair("scale", [1.0; 2])?;
        let center = pair("center", [0.0; 2])?;
        let angle = request
            .get("rotation")
            .map(|value| value.as_f64().ok_or("rotation must be finite degrees"))
            .transpose()?
            .unwrap_or(0.0)
            .to_radians();
        if !angle.is_finite() {
            return Err("rotation must be finite degrees".into());
        }
        for face in targets {
            if let Some(values) = mesh.uvs.get_mut(&face) {
                for (vertex, uv) in values {
                    if vertices.as_ref().is_some_and(|ids| !ids.contains(vertex)) {
                        continue;
                    }
                    let u = (uv[0] - center[0]) * scale[0];
                    let v = (uv[1] - center[1]) * scale[1];
                    *uv = [
                        center[0] + u * angle.cos() - v * angle.sin() + offset[0],
                        center[1] + u * angle.sin() + v * angle.cos() + offset[1],
                    ];
                    count += 1;
                }
            }
        }
    } else {
        return Err("unknown UV command".into());
    }
    Ok(json!({"changed_corners":count}))
}

pub(super) fn complete(mesh: &Mesh, face: u64) -> bool {
    mesh.uvs.get(&face).is_some_and(|uvs| {
        mesh.faces[&face]
            .iter()
            .all(|vertex| uvs.contains_key(vertex))
    })
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let (offset, limit) = crate::responses::page(request)?;
    if request["kind"] == "textures" {
        let items: Vec<_> = mesh.textures.iter().skip(offset).take(limit).map(|(name, texture)| json!({"texture":name,"width":texture.width,"height":texture.height,"bytes":texture.width*texture.height*4,"channels":texture.maps.keys().collect::<Vec<_>>(),"layers":texture.paint.as_ref().map_or(0,|paint|paint.layers.len())})).collect();
        let next = offset.saturating_add(limit);
        return Ok(
            json!({"revision":revision,"items":items,"total":mesh.textures.len(),"offset":offset,"next_offset":if next < mesh.textures.len() {Some(next)} else {None}}),
        );
    }
    if request.get("kind").is_some_and(|value| value != "faces") {
        return Err("kind must be faces or textures".into());
    }
    let targets = faces(mesh, request)?;
    let items: Vec<_> = targets.iter().skip(offset).take(limit).map(|face| {
        let corners: Vec<_> = mesh.faces[face].iter().map(|vertex| json!({"vertex":vertex,"uv":mesh.uvs.get(face).and_then(|uvs| uvs.get(vertex))})).collect();
        json!({"face":face,"corners":corners,"complete":complete(mesh,*face),"texture":mesh.face_textures.get(face),"area_normal":crate::appearance_tools::normal(mesh,*face),"flat":mesh.flat_faces.contains(face)})
    }).collect();
    let next = offset.saturating_add(limit);
    Ok(
        json!({"revision":revision,"items":items,"total":targets.len(),"offset":offset,"next_offset":if next < targets.len() {Some(next)} else {None}}),
    )
}

pub(super) fn inspect(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let targets = faces(mesh, request)?;
    let mut missing = 0;
    let mut degenerate = 0;
    let mut mirrored = 0;
    let mut outside = 0;
    let mut issues = Vec::new();
    for id in &targets {
        let face = &mesh.faces[id];
        if !complete(mesh, *id) {
            missing += 1;
            issues.push(json!({"face":id,"issue":"missing_uv"}));
            continue;
        }
        let uv = &mesh.uvs[id];
        let out = face
            .iter()
            .any(|vertex| uv[vertex].iter().any(|value| *value < 0.0 || *value > 1.0));
        let mut flat = false;
        let mut flipped = false;
        for i in 1..face.len() - 1 {
            let a = uv[&face[0]];
            let b = uv[&face[i]];
            let c = uv[&face[i + 1]];
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            flat |= area.abs() < 1e-12;
            flipped |= area < -1e-12;
        }
        degenerate += usize::from(flat);
        mirrored += usize::from(flipped);
        outside += usize::from(out);
        if flat || flipped || out {
            issues.push(
                json!({"face":id,"degenerate":flat,"mirrored":flipped,"outside_unit_square":out}),
            );
        }
    }
    let (offset, limit) = crate::responses::page(request)?;
    let next = offset.saturating_add(limit);
    let page: Vec<_> = issues.iter().skip(offset).take(limit).collect();
    Ok(
        json!({"revision":revision,"faces":targets.len(),"missing_uv_faces":missing,"degenerate_uv_faces":degenerate,"mirrored_uv_faces":mirrored,"outside_unit_square_faces":outside,"issues":page,"total":issues.len(),"offset":offset,"next_offset":if next < issues.len() {Some(next)} else {None},"notes":"Mirroring, tiling and overlapping UVs may be intentional. Overlap and distortion are not automatically classified. No UV values are repaired or generated."}),
    )
}

pub(super) fn reconcile(mesh: &mut Mesh) {
    mesh.vertex_colors
        .retain(|id, _| mesh.vertices.contains_key(id));
    mesh.uvs.retain(|face, values| {
        let Some(vertices) = mesh.faces.get(face) else {
            return false;
        };
        values.retain(|vertex, _| vertices.contains(vertex));
        !values.is_empty()
    });
    mesh.face_textures
        .retain(|face, _| mesh.faces.contains_key(face));
}

pub(super) fn copy_face(
    mesh: &mut Mesh,
    source: u64,
    destination: u64,
    vertices: &BTreeMap<u64, u64>,
) {
    if let Some(uvs) = mesh.uvs.get(&source) {
        let copied = uvs
            .iter()
            .filter_map(|(vertex, uv)| vertices.get(vertex).map(|mapped| (*mapped, *uv)))
            .collect();
        mesh.uvs.insert(destination, copied);
    } else {
        mesh.uvs.remove(&destination);
    }
    if let Some(texture) = mesh.face_textures.get(&source).cloned() {
        mesh.face_textures.insert(destination, texture);
    } else {
        mesh.face_textures.remove(&destination);
    }
}

pub(super) fn validate(mesh: &Mesh) -> Result<(), String> {
    for (face, values) in &mesh.uvs {
        let vertices = mesh
            .faces
            .get(face)
            .ok_or("UVs reference an unknown face")?;
        for (vertex, uv) in values {
            let invalid_binding = !vertices.contains(vertex)
                || uv
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > 1e6);
            if invalid_binding {
                return Err("invalid UV corner binding or coordinate".into());
            }
        }
    }
    for (face, texture) in &mesh.face_textures {
        if !mesh.faces.contains_key(face) || !mesh.textures.contains_key(texture) {
            return Err("invalid face texture binding".into());
        }
    }
    let mut total = 0;
    let invalid_names = mesh
        .textures
        .keys()
        .any(|name| name.trim().is_empty() || name.len() > 128);
    if invalid_names {
        return Err("texture names must contain 1..128 bytes".into());
    }
    for texture in mesh.textures.values() {
        pixels(texture)?;
        crate::painting::validate(texture)?;
        total += texture.width * texture.height;
    }
    if total > 4_194_304 {
        return Err("project textures must not exceed 4194304 pixels in total".into());
    }
    crate::painting::validate_budget(mesh)?;
    Ok(())
}

pub(super) fn pixels(texture: &Texture) -> Result<Vec<u8>, String> {
    if texture.alpha_cutoff.is_some_and(|cutoff| !(0.0..=1.0).contains(&cutoff)) {
        return Err("alpha_cutoff must be between 0 and 1".into());
    }
    if texture.width == 0 || texture.height == 0 || texture.width > 1024 || texture.height > 1024 {
        return Err(format!("texture is {}×{} pixels; each dimension must be 1..1024", texture.width, texture.height));
    }
    if texture.rgba_base64.len() > (texture.width * texture.height * 4).div_ceil(3) * 4 {
        return Err("texture pixel data exceeds dimensions".into());
    }
    let bytes = base64::decode(&texture.rgba_base64)?;
    if bytes.len() != texture.width * texture.height * 4 {
        return Err("RGBA byte count must equal width * height * 4".into());
    }
    Ok(bytes)
}

pub(super) fn import_png(bytes: &[u8]) -> Result<Texture, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("texture PNG must not exceed 8 MiB".into());
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let width = reader.info().width as usize;
    let height = reader.info().height as usize;
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return Err(format!("texture is {width}×{height} pixels; each dimension must be 1..1024"));
    }
    if reader.info().animation_control.is_some() {
        return Err("animated PNG textures are not supported".into());
    }
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    let channels = info.color_type.samples();
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in buffer[..info.buffer_size()].chunks_exact(channels) {
        let value = match info.color_type {
            png::ColorType::Rgb => [pixel[0], pixel[1], pixel[2], 255],
            png::ColorType::Rgba => [pixel[0], pixel[1], pixel[2], pixel[3]],
            png::ColorType::Grayscale => [pixel[0], pixel[0], pixel[0], 255],
            png::ColorType::GrayscaleAlpha => [pixel[0], pixel[0], pixel[0], pixel[1]],
            _ => return Err("unsupported PNG texture encoding".into()),
        };
        rgba.extend(value);
    }
    Ok(Texture {
        width,
        height,
        rgba_base64: base64::encode(&rgba),
        alpha_cutoff: None,
        maps: BTreeMap::new(),
        paint: None,
    })
}

pub(super) fn sample(texture: &Texture, bytes: &[u8], uv: [f64; 2], composite_alpha: bool) -> [u8; 3] {
    let index = texel_offset(texture.width, texture.height, uv);
    let alpha = if composite_alpha { f64::from(bytes[index + 3]) / 255.0 } else { 1.0 };
    std::array::from_fn(|channel| {
        (f64::from(bytes[index + channel]) * alpha + 128.0 * (1.0 - alpha)).round() as u8
    })
}

pub(super) fn below_cutoff(cutoff: Option<f64>, alpha: u8) -> bool {
    cutoff.is_some_and(|cutoff| f64::from(alpha) / 255.0 < cutoff)
}

pub(super) fn texel_offset(width: usize, height: usize, uv: [f64; 2]) -> usize {
    let x = ((uv[0].rem_euclid(1.0) * width as f64).floor() as usize).min(width - 1);
    let row = ((uv[1].rem_euclid(1.0) * height as f64).floor() as usize).min(height - 1);
    ((height - 1 - row) * width + x) * 4
}

pub(super) fn checker(uv: [f64; 2]) -> [u8; 3] {
    let odd = ((uv[0] * 8.0).floor() as i64 + (uv[1] * 8.0).floor() as i64).rem_euclid(2) != 0;
    if odd { [48; 3] } else { [224; 3] }
}

pub(super) fn capture(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let targets = faces(mesh, request)?;
    let size = request
        .get("size")
        .map(|value| {
            value
                .as_u64()
                .filter(|value| (64..=2048).contains(value))
                .ok_or("size must be 64..2048")
        })
        .transpose()?
        .unwrap_or(512) as usize;
    let bounds: [f64; 4] = request
        .get("bounds")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| "bounds must be [min_u,min_v,max_u,max_v]")?
        .unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let invalid_bounds = bounds
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1e6)
        || bounds[2] - bounds[0] < 1e-12
        || bounds[3] - bounds[1] < 1e-12;
    if invalid_bounds {
        return Err("UV bounds must be finite and increasing within -1000000..1000000".into());
    }
    let texture = request
        .get("texture")
        .map(|value| {
            value
                .as_str()
                .and_then(|name| mesh.textures.get(name))
                .ok_or("unknown texture")
        })
        .transpose()?;
    let data = texture.map(pixels).transpose()?;
    let mut rgb = vec![0; size * size * 3];
    for y in 0..size {
        for x in 0..size {
            let uv = [
                bounds[0] + (x as f64 + 0.5) / size as f64 * (bounds[2] - bounds[0]),
                bounds[3] - (y as f64 + 0.5) / size as f64 * (bounds[3] - bounds[1]),
            ];
            let color = if let Some(texture) = texture {
                sample(texture, data.as_ref().unwrap(), uv, true)
            } else {
                checker(uv)
            };
            rgb[(y * size + x) * 3..(y * size + x) * 3 + 3].copy_from_slice(&color);
        }
    }
    let point = |uv: [f64; 2]| -> [f64; 2] {
        [
            (uv[0] - bounds[0]) / (bounds[2] - bounds[0]) * (size - 1) as f64,
            (bounds[3] - uv[1]) / (bounds[3] - bounds[1]) * (size - 1) as f64,
        ]
    };
    for id in &targets {
        let face = &mesh.faces[id];
        for index in 0..face.len() {
            let a = mesh.uvs.get(id).and_then(|values| values.get(&face[index]));
            let b = mesh
                .uvs
                .get(id)
                .and_then(|values| values.get(&face[(index + 1) % face.len()]));
            if let (Some(a), Some(b)) = (a, b) {
                line(&mut rgb, size, point(*a), point(*b));
            }
        }
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, size as u32, size as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&rgb)
            .map_err(|error| error.to_string())?;
    }
    let encoded = base64::encode(&png);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\"><image width=\"{size}\" height=\"{size}\" href=\"data:image/png;base64,{encoded}\"/></svg>"
    );
    Ok(
        json!({"png_base64":encoded,"svg":svg,"revision":revision,"mode":"uv_layout","size":size,"bounds":bounds,"faces":targets.len(),"query":{"command":"query_uv","faces":targets},"report":inspect(mesh,request,revision)?}),
    )
}

pub(super) fn export_texture(mesh: &Mesh, request: &Value) -> Result<Value, String> {
    let name = request["texture"].as_str().ok_or("texture is required")?;
    let texture = mesh.textures.get(name).ok_or("unknown texture")?;
    let pixels =
        crate::painting::map_pixels(texture, request["channel"].as_str().unwrap_or("base_color"))?;
    let png = encode_png(texture, &pixels)?;
    Ok(
        json!({"texture":name,"channel":request["channel"].as_str().unwrap_or("base_color"),"width":texture.width,"height":texture.height,"png_base64":base64::encode(&png)}),
    )
}

pub(super) fn encode_png(texture: &Texture, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, texture.width as u32, texture.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(pixels)
            .map_err(|error| error.to_string())?;
    }
    Ok(png)
}

fn line(rgb: &mut [u8], size: usize, a: [f64; 2], b: [f64; 2]) {
    // Clip before rasterizing so tiling UVs cannot create unbounded line work.
    let delta = [b[0] - a[0], b[1] - a[1]];
    let mut low: f64 = 0.0;
    let mut high: f64 = 1.0;
    for axis in 0..2 {
        if delta[axis].abs() < 1e-12 {
            if a[axis] < 0.0 || a[axis] > (size - 1) as f64 {
                return;
            }
        } else {
            let first = -a[axis] / delta[axis];
            let second = ((size - 1) as f64 - a[axis]) / delta[axis];
            low = low.max(first.min(second));
            high = high.min(first.max(second));
        }
    }
    if low > high {
        return;
    }
    let first: [f64; 2] = std::array::from_fn(|axis| a[axis] + delta[axis] * low);
    let last: [f64; 2] = std::array::from_fn(|axis| a[axis] + delta[axis] * high);
    let steps = (last[0] - first[0])
        .abs()
        .max((last[1] - first[1]).abs())
        .ceil() as usize;
    for step in 0..=steps {
        let t = step as f64 / steps.max(1) as f64;
        let x = (first[0] + (last[0] - first[0]) * t)
            .round()
            .clamp(0.0, (size - 1) as f64) as usize;
        let y = (first[1] + (last[1] - first[1]) * t)
            .round()
            .clamp(0.0, (size - 1) as f64) as usize;
        rgb[(y * size + x) * 3..(y * size + x) * 3 + 3].copy_from_slice(&[255, 32, 128]);
    }
}
