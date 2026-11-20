use crate::{
    Mesh, base64,
    uv::{self, Texture},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn one() -> f64 {
    1.0
}
fn yes() -> bool {
    true
}
fn white() -> [f64; 4] {
    [1.0; 4]
}
fn normal_blend() -> String {
    "normal".into()
}
fn base_color() -> String {
    "base_color".into()
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba_base64: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<[f64; 2]>,
    pub radius: f64,
    #[serde(default)]
    pub hardness: f64,
    #[serde(default = "one")]
    pub opacity: f64,
    #[serde(default = "one")]
    pub value: f64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Noise {
    pub scale: f64,
    #[serde(default)]
    pub seed: u32,
    #[serde(default = "one")]
    pub amount: f64,
    pub threshold: Option<f64>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) surface: Option<crate::surface_paint::SurfaceMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<crate::paint_masks::Filter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<crate::paint_masks::Mark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<crate::paint_masks::Transform>,
    #[serde(default = "one")]
    pub fill: f64,
    #[serde(default)]
    pub polygons: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
    pub noise: Option<Noise>,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            surface: None,
            source: None,
            filters: vec![],
            marks: vec![],
            transform: None,
            fill: 1.0,
            polygons: vec![],
            strokes: vec![],
            noise: None,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, [f64; 4]>,
    #[serde(default = "base_color")]
    pub channel: String,
    #[serde(default = "white")]
    pub color: [f64; 4],
    #[serde(default = "one")]
    pub opacity: f64,
    #[serde(default = "normal_blend")]
    pub blend: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub mask: Mask,
    pub image: Option<Image>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintStack {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub masks: BTreeMap<String, Mask>,
    pub base: BTreeMap<String, String>,
    pub layers: Vec<Layer>,
    pub normal_from_height: Option<f64>,
}

const CHANNELS: [&str; 6] = [
    "base_color",
    "normal",
    "roughness",
    "metallic",
    "specular",
    "height",
];

fn channel(name: &str) -> Result<(), String> {
    if !CHANNELS.contains(&name) {
        return Err(
            "channel must be base_color, normal, roughness, metallic, specular or height".into(),
        );
    }
    Ok(())
}

fn decode(encoded: &str, count: usize) -> Result<Vec<u8>, String> {
    if encoded.len() > count.div_ceil(3) * 4 {
        return Err("image exceeds declared dimensions".into());
    }
    let bytes = base64::decode(encoded)?;
    if bytes.len() != count {
        return Err("image byte count does not match dimensions".into());
    }
    Ok(bytes)
}

pub(super) fn map_pixels(texture: &Texture, name: &str) -> Result<Vec<u8>, String> {
    channel(name)?;
    if name == "base_color" {
        return uv::pixels(texture);
    }
    if let Some(encoded) = texture.maps.get(name) {
        return decode(encoded, texture.width * texture.height * 4);
    }
    let color = match name {
        "normal" => [128, 128, 255, 255],
        "roughness" => [166, 166, 166, 255],
        "specular" => [10, 10, 10, 255],
        _ => [0, 0, 0, 255],
    };
    Ok(color.repeat(texture.width * texture.height))
}

fn stack(texture: &mut Texture) -> &mut PaintStack {
    texture.paint.get_or_insert_with(|| {
        let mut base = texture.maps.clone();
        base.insert("base_color".into(), texture.rgba_base64.clone());
        PaintStack {
            masks: BTreeMap::new(),
            base,
            layers: vec![],
            normal_from_height: None,
        }
    })
}

fn label(request: &Value, field: &str) -> Result<String, String> {
    request[field]
        .as_str()
        .filter(|name| !name.trim().is_empty() && name.len() <= 128)
        .map(str::to_owned)
        .ok_or_else(|| format!("{field} must contain 1..128 bytes"))
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    if request.get("object").is_some() && !matches!(command, "texture_layer_mask_faces" | "texture_mask_surface") {
        return Err(
            "texture resources are shared; omit object except when sampling a face mask".into(),
        );
    }
    let name = label(request, "texture")?;
    if command == "texture_create" {
        if mesh.textures.contains_key(&name) {
            return Err("texture already exists; use a new name".into());
        }
        let width = request["width"]
            .as_u64()
            .filter(|value| (1..=1024).contains(value))
            .ok_or("width must be 1..1024")? as usize;
        let height = request["height"]
            .as_u64()
            .filter(|value| (1..=1024).contains(value))
            .ok_or("height must be 1..1024")? as usize;
        let color: [f64; 4] = serde_json::from_value(request["color"].clone())
            .map_err(|_| "color must be normalized RGBA")?;
        if color.iter().any(|value| !normalized(*value)) {
            return Err("color must be normalized RGBA".into());
        }
        let pixel = color.map(|value| (value * 255.0).round() as u8);
        return uv::edit(
            mesh,
            "texture_define",
            &json!({"name":name,"width":width,"height":height,"rgba_base64":base64::encode(&pixel.repeat(width*height))}),
        );
    }
    let mut texture = mesh.textures.get(&name).ok_or("unknown texture")?.clone();
    if command.starts_with("texture_mask_") {
        let mask_name = label(request, "name")?;
        let paint = stack(&mut texture);
        match command {
            "texture_mask_surface" => {
                let mask = paint.masks.entry(mask_name).or_default();
                let surface = crate::surface_paint::SurfaceMask::set(mesh, request, mask.surface.as_ref())?;
                mask.surface = Some(surface);
            }
            "texture_mask_set" => {
                let mask: Mask = serde_json::from_value(request["mask"].clone())
                    .map_err(|error| format!("invalid mask: {error}"))?;
                paint.masks.insert(mask_name, mask);
            }
            "texture_mask_remove" => {
                paint.masks.remove(&mask_name).ok_or("unknown mask")?;
            }
            "texture_mask_transform" => {
                let transform = serde_json::from_value(request.clone())
                    .map_err(|error| format!("invalid mask transform: {error}"))?;
                paint
                    .masks
                    .get_mut(&mask_name)
                    .ok_or("unknown mask")?
                    .transform = Some(transform);
            }
            "texture_mask_paint" => {
                let strokes: Vec<Stroke> = serde_json::from_value(request["strokes"].clone())
                    .map_err(|error| format!("invalid strokes: {error}"))?;
                if strokes.is_empty() {
                    return Err("strokes must not be empty".into());
                }
                paint
                    .masks
                    .get_mut(&mask_name)
                    .ok_or("unknown mask")?
                    .marks
                    .extend(
                        strokes
                            .into_iter()
                            .map(|stroke| crate::paint_masks::Mark::Stroke { stroke }),
                    );
            }
            "texture_mask_stamp" => {
                let source = label(request, "source_texture")?;
                let source = mesh.textures.get(&source).ok_or("unknown source texture")?;
                let mut value = request.clone();
                value["image"] = json!({"width":source.width,"height":source.height,"rgba_base64":source.rgba_base64});
                let stamp = serde_json::from_value(value)
                    .map_err(|error| format!("invalid alpha stamp: {error}"))?;
                paint
                    .masks
                    .get_mut(&mask_name)
                    .ok_or("unknown mask")?
                    .marks
                    .push(crate::paint_masks::Mark::Stamp { stamp });
            }
            _ => return Err("unknown texture mask command".into()),
        }
    } else if command == "texture_map_set" {
        let target = label(request, "channel")?;
        channel(&target)?;
        let source = label(request, "source_texture")?;
        let source = mesh.textures.get(&source).ok_or("unknown source texture")?;
        if source.width != texture.width || source.height != texture.height {
            return Err("source map dimensions must match target texture".into());
        }
        stack(&mut texture)
            .base
            .insert(target, source.rgba_base64.clone());
    } else if command == "texture_normal_from_height" {
        let strength = if request.get("strength") == Some(&Value::Null) {
            None
        } else {
            Some(
                request["strength"]
                    .as_f64()
                    .filter(|value| value.is_finite() && (0.0..=64.0).contains(value))
                    .ok_or("strength must be in 0..64 or null to disable")?,
            )
        };
        stack(&mut texture).normal_from_height = strength;
    } else if command != "texture_bake" {
        if command == "texture_layer_add" {
            if request["layer"]
                .get("channels")
                .is_some_and(|value| value.as_object().is_some_and(|values| values.is_empty()))
            {
                return Err("channels must contain at least one channel".into());
            }
            if request["layer"].get("channels").is_some()
                && (request["layer"].get("channel").is_some()
                    || request["layer"].get("color").is_some())
            {
                return Err("use channels, or the legacy channel/color fields, not both".into());
            }
            let mut layer: Layer = serde_json::from_value(request["layer"].clone())
                .map_err(|error| format!("invalid layer: {error}"))?;
            if let Some(source) = request.get("source_texture") {
                let source = source
                    .as_str()
                    .and_then(|name| mesh.textures.get(name))
                    .ok_or("unknown source texture")?;
                layer.image = Some(Image {
                    width: source.width,
                    height: source.height,
                    rgba_base64: source.rgba_base64.clone(),
                });
            }
            let paint = stack(&mut texture);
            if paint
                .layers
                .iter()
                .any(|existing| existing.name == layer.name)
            {
                return Err("layer name already exists".into());
            }
            paint.layers.push(layer);
        } else {
            let layer_name = label(request, "layer")?;
            let paint = stack(&mut texture);
            let index = paint
                .layers
                .iter()
                .position(|layer| layer.name == layer_name)
                .ok_or("unknown layer")?;
            match command {
                "texture_layer_remove" => {
                    paint.layers.remove(index);
                }
                "texture_layer_move" => {
                    let destination = request["index"]
                        .as_u64()
                        .filter(|index| (*index as usize) < paint.layers.len())
                        .ok_or("index must be a valid zero-based layer position")?
                        as usize;
                    let layer = paint.layers.remove(index);
                    paint.layers.insert(destination, layer);
                }
                "texture_layer_set" => {
                    let patch = request["changes"]
                        .as_object()
                        .ok_or("changes must be a layer field object")?;
                    if patch.get("channels").is_some_and(|value| {
                        value.as_object().is_some_and(|values| values.is_empty())
                    }) {
                        return Err("channels must not be empty".into());
                    }
                    if patch.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "color" | "channels" | "opacity" | "blend" | "visible" | "mask"
                        )
                    }) {
                        return Err(
                            "changes accepts color, channels, opacity, blend, visible and mask"
                                .into(),
                        );
                    }
                    let mut layer = serde_json::to_value(&paint.layers[index]).unwrap();
                    for (key, value) in patch {
                        if key == "mask" {
                            let fields =
                                value.as_object().ok_or("mask changes must be an object")?;
                            if fields.keys().any(|key| {
                                !matches!(
                                    key.as_str(),
                                    "fill"
                                        | "polygons"
                                        | "strokes"
                                        | "noise"
                                        | "source"
                                        | "filters"
                                        | "marks"
                                        | "transform"
                                )
                            }) {
                                return Err("unknown mask field".into());
                            }
                            for (field, value) in fields {
                                layer["mask"][field] = value.clone();
                            }
                        } else {
                            layer[key] = value.clone();
                        }
                    }
                    paint.layers[index] = serde_json::from_value(layer)
                        .map_err(|error| format!("invalid layer changes: {error}"))?;
                    if !paint.layers[index].channels.is_empty() && patch.contains_key("color") {
                        return Err(
                            "edit channels on a material layer, not the legacy color".into()
                        );
                    }
                }
                "texture_layer_paint" => {
                    let strokes: Vec<Stroke> =
                        serde_json::from_value(request["strokes"].clone())
                            .map_err(|error| format!("invalid strokes: {error}"))?;
                    if strokes.is_empty() {
                        return Err("strokes must not be empty".into());
                    }
                    paint.layers[index].mask.strokes.extend(strokes);
                }
                "texture_layer_mask_faces" => {
                    let targets = crate::surface_paint::faces(mesh, request)?;
                    let mode = request["mode"].as_str().unwrap_or("faces");
                    if mode != "faces" && mode != "boundary" {
                        return Err("mask mode must be faces or boundary".into());
                    }
                    let mut mask = Mask {
                        fill: 0.0,
                        ..Mask::default()
                    };
                    let mut edges = BTreeMap::<(u64, u64), Vec<(u64, u64, u64)>>::new();
                    for face in targets {
                        if !uv::complete(mesh, face) {
                            return Err("face mask requires complete UVs".into());
                        }
                        let vertices = &mesh.faces[&face];
                        if mode == "faces" {
                            mask.polygons
                                .push(vertices.iter().map(|id| mesh.uvs[&face][id]).collect());
                        }
                        for index in 0..vertices.len() {
                            let a = vertices[index];
                            let b = vertices[(index + 1) % vertices.len()];
                            edges
                                .entry((a.min(b), a.max(b)))
                                .or_default()
                                .push((face, a, b));
                        }
                    }
                    if mode == "boundary" {
                        let radius = request["radius"]
                            .as_f64()
                            .ok_or("boundary mask requires radius in UV units")?;
                        let hardness = request
                            .get("hardness")
                            .map(|value| value.as_f64().ok_or("hardness must be numeric"))
                            .transpose()?
                            .unwrap_or(0.5);
                        for corners in edges.values().filter(|corners| corners.len() == 1) {
                            let (face, a, b) = corners[0];
                            mask.strokes.push(Stroke {
                                points: vec![mesh.uvs[&face][&a], mesh.uvs[&face][&b]],
                                radius,
                                hardness,
                                opacity: 1.0,
                                value: 1.0,
                            });
                        }
                    }
                    paint.layers[index].mask = mask;
                }
                _ => return Err("unknown texture layer command".into()),
            }
        }
    }
    validate_stack(&texture)?;
    bake(&mut texture)?;
    let flatten = request
        .get("flatten")
        .map(|value| value.as_bool().ok_or("flatten must be boolean"))
        .transpose()?
        .unwrap_or(false);
    if flatten {
        if command != "texture_bake" {
            return Err("flatten is only available with texture_bake".into());
        }
        texture.paint = None;
    }
    let layers = texture.paint.as_ref().map_or(0, |paint| paint.layers.len());
    mesh.textures.insert(name.clone(), texture);
    validate_budget(mesh)?;
    Ok(
        json!({"texture":name,"layers":layers,"baked":true,"channels":mesh.textures[&name].maps.keys().collect::<Vec<_>>(),"shared_resource":true}),
    )
}

fn normalized(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn validate_stack(texture: &Texture) -> Result<(), String> {
    let Some(paint) = &texture.paint else {
        return Ok(());
    };
    if paint.layers.len() > 64 || !paint.base.contains_key("base_color") {
        return Err("stack requires a base color and at most 64 layers".into());
    }
    if paint
        .normal_from_height
        .is_some_and(|value| !value.is_finite() || !(0.0..=64.0).contains(&value))
    {
        return Err("invalid height-to-normal strength".into());
    }
    for (name, data) in &paint.base {
        channel(name)?;
        decode(data, texture.width * texture.height * 4)?;
    }
    let mut names = std::collections::BTreeSet::new();
    for layer in &paint.layers {
        if layer.name.trim().is_empty() || layer.name.len() > 128 || !names.insert(&layer.name) {
            return Err("layer names must be distinct and contain 1..128 bytes".into());
        }
        channel(&layer.channel)?;
        for (name, color) in &layer.channels {
            channel(name)?;
            if color.iter().any(|value| !normalized(*value)) {
                return Err("channel colors must contain normalized RGBA".into());
            }
        }
        if !normalized(layer.opacity) || layer.color.iter().any(|value| !normalized(*value)) {
            return Err("layer color and opacity must be finite in 0..1".into());
        }
        if !matches!(
            layer.blend.as_str(),
            "normal" | "multiply" | "add" | "screen"
        ) {
            return Err("blend must be normal, multiply, add or screen".into());
        }
        validate_mask(&layer.mask)?;
        if let Some(image) = &layer.image {
            if image.width == 0 || image.height == 0 || image.width > 1024 || image.height > 1024 {
                return Err("layer image dimensions must be 1..1024".into());
            }
            decode(&image.rgba_base64, image.width * image.height * 4)?;
        }
    }
    crate::paint_masks::validate(paint)?;
    Ok(())
}

pub(super) fn validate_mask(mask: &Mask) -> Result<(), String> {
    if let Some(surface) = &mask.surface {
        surface.validate()?;
    }
    if !normalized(mask.fill) || mask.polygons.len() + mask.strokes.len() > 4096 {
        return Err("mask requires fill in 0..1 and at most 4096 shapes".into());
    }
    let mut points = 0;
    for polygon in &mask.polygons {
        if polygon.len() < 3 {
            return Err("mask polygons require at least three points".into());
        }
        points += polygon.len();
        for point in polygon {
            validate_point(*point)?;
        }
    }
    for stroke in &mask.strokes {
        if stroke.points.is_empty()
            || !stroke.radius.is_finite()
            || !(1e-6..=2.0).contains(&stroke.radius)
        {
            return Err("strokes require points and radius in 0.000001..2 UV units".into());
        }
        if !normalized(stroke.opacity) || !normalized(stroke.hardness) || !normalized(stroke.value)
        {
            return Err("stroke opacity, hardness and value must be in 0..1".into());
        }
        points += stroke.points.len();
        for point in &stroke.points {
            validate_point(*point)?;
        }
    }
    if points > 16384 {
        return Err("mask exceeds 16384 points".into());
    }
    if let Some(noise) = &mask.noise {
        if !noise.scale.is_finite()
            || !(1.0..=4096.0).contains(&noise.scale)
            || !normalized(noise.amount)
            || noise.threshold.is_some_and(|value| !normalized(value))
        {
            return Err("noise requires scale 1..4096, amount/threshold 0..1".into());
        }
    }
    Ok(())
}

fn validate_point(point: [f64; 2]) -> Result<(), String> {
    if point
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1e6)
    {
        return Err("UV points must be finite within +/-1000000".into());
    }
    Ok(())
}

pub(super) fn validate(texture: &Texture) -> Result<(), String> {
    for (name, data) in &texture.maps {
        channel(name)?;
        if name == "base_color" {
            return Err("base_color is stored in rgba_base64".into());
        }
        decode(data, texture.width * texture.height * 4)?;
    }
    validate_stack(texture)?;
    if texture.paint.is_some() {
        let mut expected = texture.clone();
        bake(&mut expected)?;
        if texture.rgba_base64 != expected.rgba_base64 || texture.maps != expected.maps {
            return Err("baked texture does not match its editable stack".into());
        }
    }
    Ok(())
}

pub(super) fn validate_budget(mesh: &Mesh) -> Result<(), String> {
    let mut encoded = 0usize;
    for texture in mesh.textures.values() {
        encoded += texture.rgba_base64.len();
        encoded += texture.maps.values().map(String::len).sum::<usize>();
        if let Some(paint) = &texture.paint {
            encoded += paint.base.values().map(String::len).sum::<usize>();
            encoded += paint
                .layers
                .iter()
                .filter_map(|layer| layer.image.as_ref())
                .map(|image| image.rgba_base64.len())
                .sum::<usize>();
            encoded += paint
                .masks
                .values()
                .chain(paint.layers.iter().map(|layer| &layer.mask))
                .filter_map(|mask| mask.surface.as_ref()?.image.as_ref())
                .map(|image| image.rgba_base64.len())
                .sum::<usize>();
            encoded += paint
                .masks
                .values()
                .chain(paint.layers.iter().map(|layer| &layer.mask))
                .flat_map(|mask| &mask.marks)
                .filter_map(|mark| match mark {
                    crate::paint_masks::Mark::Stamp { stamp } => {
                        Some(stamp.image.rgba_base64.len())
                    }
                    _ => None,
                })
                .sum::<usize>();
        }
    }
    if encoded > 44_739_264 {
        return Err("project texture images and layer sources exceed 32 MiB decoded budget".into());
    }
    Ok(())
}

fn hash(x: i64, y: i64, seed: u32) -> f64 {
    let mut value = (x as u32).wrapping_mul(374761393) ^ (y as u32).wrapping_mul(668265263) ^ seed;
    value = (value ^ (value >> 13)).wrapping_mul(1274126177);
    f64::from(value ^ (value >> 16)) / f64::from(u32::MAX)
}

fn noise(point: [f64; 2], settings: &Noise) -> f64 {
    let mut total = 0.0;
    for octave in 0..3 {
        let scale = settings.scale * (1u32 << octave) as f64;
        let x = point[0] * scale;
        let y = point[1] * scale;
        let ix = x.floor() as i64;
        let iy = y.floor() as i64;
        let u = x - x.floor();
        let v = y - y.floor();
        let u = u * u * (3.0 - 2.0 * u);
        let v = v * v * (3.0 - 2.0 * v);
        let seed = settings.seed.wrapping_add(octave * 73);
        let a = hash(ix, iy, seed) * (1.0 - u) + hash(ix + 1, iy, seed) * u;
        let b = hash(ix, iy + 1, seed) * (1.0 - u) + hash(ix + 1, iy + 1, seed) * u;
        total += (a * (1.0 - v) + b * v) / (1u32 << octave) as f64;
    }
    let value = total / 1.75;
    settings.threshold.map_or(
        value,
        |threshold| if value >= threshold { 1.0 } else { 0.0 },
    )
}

fn region(points: &[[f64; 2]], radius: f64, width: usize, height: usize) -> [usize; 4] {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for p in points {
        for a in 0..2 {
            low[a] = low[a].min(p[a] - radius);
            high[a] = high[a].max(p[a] + radius);
        }
    }
    [
        (low[0] * width as f64).floor().clamp(0.0, width as f64) as usize,
        ((1.0 - high[1]) * height as f64)
            .floor()
            .clamp(0.0, height as f64) as usize,
        (high[0] * width as f64).ceil().clamp(0.0, width as f64) as usize,
        ((1.0 - low[1]) * height as f64)
            .ceil()
            .clamp(0.0, height as f64) as usize,
    ]
}

fn texel(x: usize, y: usize, width: usize, height: usize) -> [f64; 2] {
    [
        (x as f64 + 0.5) / width as f64,
        1.0 - (y as f64 + 0.5) / height as f64,
    ]
}

fn distance(point: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let delta = [b[0] - a[0], b[1] - a[1]];
    let length = delta[0] * delta[0] + delta[1] * delta[1];
    let fraction = if length > 0.0 {
        ((point[0] - a[0]) * delta[0] + (point[1] - a[1]) * delta[1]) / length
    } else {
        0.0
    };
    let fraction = fraction.clamp(0.0, 1.0);
    (point[0] - a[0] - fraction * delta[0]).hypot(point[1] - a[1] - fraction * delta[1])
}

pub(super) fn raster_mask(mask: &Mask, width: usize, height: usize) -> Vec<f64> {
    let mut pixels = vec![mask.fill; width * height];
    for polygon in &mask.polygons {
        let [left, top, right, bottom] = region(polygon, 0.0, width, height);
        for y in top..bottom {
            for x in left..right {
                let p = texel(x, y, width, height);
                let mut inside = false;
                for i in 0..polygon.len() {
                    let a = polygon[i];
                    let b = polygon[(i + 1) % polygon.len()];
                    if (a[1] > p[1]) != (b[1] > p[1])
                        && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                if inside {
                    pixels[y * width + x] = 1.0;
                }
            }
        }
    }
    for stroke in &mask.strokes {
        raster_stroke(&mut pixels, stroke, width, height);
    }
    if let Some(settings) = &mask.noise {
        for y in 0..height {
            for x in 0..width {
                let variation = noise(texel(x, y, width, height), settings);
                pixels[y * width + x] *= 1.0 - settings.amount + settings.amount * variation;
            }
        }
    }
    pixels
}

pub(super) fn raster_stroke(pixels: &mut [f64], stroke: &Stroke, width: usize, height: usize) {
    let [left, top, right, bottom] = region(&stroke.points, stroke.radius, width, height);
    for y in top..bottom {
        for x in left..right {
            let p = texel(x, y, width, height);
            let mut nearest = f64::INFINITY;
            for i in 0..stroke.points.len().max(2) - 1 {
                nearest = nearest.min(distance(
                    p,
                    stroke.points[i.min(stroke.points.len() - 1)],
                    stroke.points[(i + 1).min(stroke.points.len() - 1)],
                ));
            }
            let coverage = if nearest <= stroke.radius * stroke.hardness {
                1.0
            } else {
                ((stroke.radius - nearest) / (stroke.radius * (1.0 - stroke.hardness)).max(1e-12))
                    .clamp(0.0, 1.0)
            };
            let alpha = coverage * stroke.opacity;
            let index = y * width + x;
            pixels[index] = pixels[index] * (1.0 - alpha) + stroke.value * alpha;
        }
    }
}

fn bake(texture: &mut Texture) -> Result<(), String> {
    let Some(paint) = &texture.paint else {
        return Ok(());
    };
    let neutral = Texture {
        width: texture.width,
        height: texture.height,
        rgba_base64: texture.rgba_base64.clone(),
        maps: BTreeMap::new(),
        paint: None,
    };
    let mut channels = BTreeMap::new();
    let mut resolved_masks = BTreeMap::new();
    for (name, data) in &paint.base {
        channels.insert(
            name.clone(),
            decode(data, texture.width * texture.height * 4)?,
        );
    }
    for layer in paint.layers.iter().filter(|layer| layer.visible) {
        let mask = crate::paint_masks::resolve(
            &layer.mask,
            paint,
            texture.width,
            texture.height,
            &mut resolved_masks,
        )?;
        let outputs = if layer.channels.is_empty() {
            BTreeMap::from([(layer.channel.clone(), layer.color)])
        } else {
            layer.channels.clone()
        };
        let image = layer
            .image
            .as_ref()
            .map(|image| decode(&image.rgba_base64, image.width * image.height * 4))
            .transpose()?;
        for (channel, color) in outputs {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                channels.entry(channel.clone())
            {
                entry.insert(map_pixels(&neutral, &channel)?);
            }
            let pixels = channels.get_mut(&channel).unwrap();
            for y in 0..texture.height {
                for x in 0..texture.width {
                    let index = y * texture.width + x;
                    let offset = index * 4;
                    let mut source = color;
                    if let (Some(image), Some(bytes)) = (&layer.image, &image) {
                        let sx = x * image.width / texture.width;
                        let sy = y * image.height / texture.height;
                        let start = (sy * image.width + sx) * 4;
                        for axis in 0..4 {
                            source[axis] *= f64::from(bytes[start + axis]) / 255.0;
                        }
                    }
                    let alpha = source[3] * layer.opacity * mask[index];
                    let previous_alpha = f64::from(pixels[offset + 3]) / 255.0;
                    let out_alpha = alpha + previous_alpha * (1.0 - alpha);
                    for axis in 0..3 {
                        let previous = f64::from(pixels[offset + axis]) / 255.0;
                        let blend = match layer.blend.as_str() {
                            "multiply" => previous * source[axis],
                            "add" => (previous + source[axis]).min(1.0),
                            "screen" => 1.0 - (1.0 - previous) * (1.0 - source[axis]),
                            _ => source[axis],
                        };
                        let value = if out_alpha > 0.0 {
                            (blend * alpha + previous * previous_alpha * (1.0 - alpha)) / out_alpha
                        } else {
                            0.0
                        };
                        pixels[offset + axis] = (value * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                    pixels[offset + 3] = (out_alpha * 255.0).round() as u8;
                }
            }
        }
    }
    if let Some(strength) = paint.normal_from_height {
        let height = channels
            .entry("height".into())
            .or_insert(map_pixels(&neutral, "height")?)
            .clone();
        let mut normal = vec![0; texture.width * texture.height * 4];
        let sample = |x: usize, y: usize| f64::from(height[(y * texture.width + x) * 4]) / 255.0;
        for y in 0..texture.height {
            for x in 0..texture.width {
                let dx = (sample((x + 1).min(texture.width - 1), y)
                    - sample(x.saturating_sub(1), y))
                    * strength;
                let dy = (sample(x, y.saturating_sub(1))
                    - sample(x, (y + 1).min(texture.height - 1)))
                    * strength;
                let n = crate::normals::unit([-dx, -dy, 1.0]);
                let index = (y * texture.width + x) * 4;
                for axis in 0..3 {
                    normal[index + axis] = ((n[axis] * 0.5 + 0.5) * 255.0).round() as u8;
                }
                normal[index + 3] = 255;
            }
        }
        channels.insert("normal".into(), normal);
    }
    texture.rgba_base64 = base64::encode(&channels.remove("base_color").unwrap());
    texture.maps = channels
        .into_iter()
        .map(|(name, bytes)| (name, base64::encode(&bytes)))
        .collect();
    Ok(())
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let name = label(request, "texture")?;
    let texture = mesh.textures.get(&name).ok_or("unknown texture")?;
    let (offset, limit) = crate::responses::page(request)?;
    let layers = texture
        .paint
        .as_ref()
        .map(|paint| paint.layers.as_slice())
        .unwrap_or(&[]);
    let items:Vec<_>=layers.iter().enumerate().skip(offset).take(limit).map(|(index,layer)|json!({"index":index,"name":layer.name,"channel":layer.channel,"channels":layer.channels,"color":layer.color,"opacity":layer.opacity,"blend":layer.blend,"visible":layer.visible,"mask":crate::paint_masks::metadata(&layer.mask),"image":layer.image.as_ref().map(|image|json!({"width":image.width,"height":image.height}))})).collect();
    let next = offset.saturating_add(limit);
    Ok(
        json!({"revision":revision,"texture":name,"items":items,"total":layers.len(),"offset":offset,"next_offset":if next<layers.len(){Some(next)}else{None},"normal_from_height":texture.paint.as_ref().and_then(|paint|paint.normal_from_height),"baked_channels":texture.maps.keys().collect::<Vec<_>>()}),
    )
}

pub(super) fn capture(mesh: &Mesh, request: &Value) -> Result<Value, String> {
    let name = label(request, "texture")?;
    let texture = mesh.textures.get(&name).ok_or("unknown texture")?;
    let paint = texture.paint.as_ref().ok_or("texture has no paint stack")?;
    let layer_name = if request["command"] == "texture_mask_capture" {
        label(request, "name")?
    } else {
        label(request, "layer")?
    };
    let definition = if request["command"] == "texture_mask_capture" {
        paint.masks.get(&layer_name).ok_or("unknown mask")?
    } else {
        &paint
            .layers
            .iter()
            .find(|layer| layer.name == layer_name)
            .ok_or("unknown layer")?
            .mask
    };
    let mask = crate::paint_masks::resolve(
        definition,
        paint,
        texture.width,
        texture.height,
        &mut BTreeMap::new(),
    )?;
    let mut bytes = Vec::with_capacity(mask.len() * 4);
    for value in mask {
        let value = (value * 255.0).round() as u8;
        bytes.extend([value, value, value, 255]);
    }
    let mut png = vec![];
    {
        let mut encoder = png::Encoder::new(&mut png, texture.width as u32, texture.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&bytes)
            .map_err(|error| error.to_string())?;
    }
    Ok(json!({"texture":name,"layer":layer_name,"kind":"mask","png_base64":base64::encode(&png)}))
}
