use crate::{
    Mesh, base64,
    painting::{Image, Mask, PaintStack},
    uv::Texture,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn one() -> f64 {
    1.0
}
fn unit_scale() -> [f64; 2] {
    [1.0; 2]
}
fn center() -> [f64; 2] {
    [0.5; 2]
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Filter {
    Invert,
    Levels {
        low: f64,
        high: f64,
        #[serde(default = "one")]
        gamma: f64,
    },
    Blur {
        radius: f64,
    },
    Dilate {
        radius: f64,
    },
    Erode {
        radius: f64,
    },
    Outline {
        radius: f64,
        side: String,
    },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    #[serde(default)]
    pub offset: [f64; 2],
    #[serde(default = "unit_scale")]
    pub scale: [f64; 2],
    #[serde(default = "center")]
    pub center: [f64; 2],
    #[serde(default)]
    pub rotation: f64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Stamp {
    pub image: Image,
    pub center: [f64; 2],
    pub size: [f64; 2],
    #[serde(default)]
    pub rotation: f64,
    #[serde(default = "one")]
    pub opacity: f64,
    #[serde(default = "one")]
    pub value: f64,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Mark {
    Stroke {
        #[serde(flatten)]
        stroke: crate::painting::Stroke,
    },
    Stamp {
        #[serde(flatten)]
        stamp: Stamp,
    },
}

pub(super) fn extended(texture: &Texture) -> bool {
    texture.paint.as_ref().is_some_and(|paint| {
        !paint.masks.is_empty()
            || paint
                .layers
                .iter()
                .any(|layer| !layer.channels.is_empty() || mask_extended(&layer.mask))
    })
}

pub(super) fn has_surface_masks(texture: &Texture) -> bool {
    texture.paint.as_ref().is_some_and(|paint| {
        paint.masks.values().chain(paint.layers.iter().map(|layer| &layer.mask))
            .any(|mask| mask.surface.is_some())
    })
}

fn mask_extended(mask: &Mask) -> bool {
    mask.source.is_some()
        || !mask.filters.is_empty()
        || !mask.marks.is_empty()
        || mask.transform.is_some()
}
fn normalized(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

pub(super) fn validate(paint: &PaintStack) -> Result<(), String> {
    if paint.masks.len() > 32 {
        return Err("a texture supports at most 32 named masks".into());
    }
    for mask in paint.masks.values() {
        crate::painting::validate_mask(mask)?;
    }
    for name in paint.masks.keys() {
        if name.trim().is_empty() || name.len() > 128 {
            return Err("mask names must contain 1..128 bytes".into());
        }
        visit(name, paint, &mut BTreeSet::new(), &mut BTreeSet::new())?;
    }
    for mask in paint
        .masks
        .values()
        .chain(paint.layers.iter().map(|layer| &layer.mask))
    {
        if let Some(name) = &mask.source {
            if !paint.masks.contains_key(name) {
                return Err(format!("unknown mask source: {name}"));
            }
        }
        if mask.filters.len() > 16 || mask.marks.len() > 256 {
            return Err("mask supports at most 16 filters and 256 ordered marks".into());
        }
        for filter in &mask.filters {
            match filter {
                Filter::Invert => {}
                Filter::Levels { low, high, gamma } => {
                    if !normalized(*low)
                        || !normalized(*high)
                        || low >= high
                        || !gamma.is_finite()
                        || !(0.01..=16.0).contains(gamma)
                    {
                        return Err(
                            "levels require 0 <= low < high <= 1 and gamma in 0.01..16".into()
                        );
                    }
                }
                Filter::Blur { radius }
                | Filter::Dilate { radius }
                | Filter::Erode { radius }
                | Filter::Outline { radius, .. } => {
                    if !radius.is_finite() || !(0.0..=0.0625).contains(radius) {
                        return Err("filter radius must be in 0..0.0625 UV units".into());
                    }
                }
            }
            if let Filter::Outline { side, .. } = filter {
                if !matches!(side.as_str(), "inner" | "outer" | "both") {
                    return Err("outline side must be inner, outer or both".into());
                }
            }
        }
        if let Some(transform) = &mask.transform {
            let invalid = transform
                .offset
                .iter()
                .chain(&transform.center)
                .any(|value| !value.is_finite() || value.abs() > 1e6);
            if invalid
                || !transform.rotation.is_finite()
                || transform
                    .scale
                    .iter()
                    .any(|value| !value.is_finite() || !(1e-6..=1000.0).contains(&value.abs()))
            {
                return Err("mask transform requires finite coordinates/rotation and nonzero scale magnitudes in 0.000001..1000".into());
            }
        }
        let mut points = mask.polygons.iter().map(Vec::len).sum::<usize>()
            + mask
                .strokes
                .iter()
                .map(|stroke| stroke.points.len())
                .sum::<usize>();
        for mark in &mask.marks {
            let stamp = match mark {
                Mark::Stroke { stroke } => {
                    crate::painting::validate_mask(&Mask {
                        strokes: vec![stroke.clone()],
                        ..Mask::default()
                    })?;
                    points += stroke.points.len();
                    continue;
                }
                Mark::Stamp { stamp } => stamp,
            };
            let invalid = stamp
                .center
                .iter()
                .any(|value| !value.is_finite() || value.abs() > 1e6);
            if invalid
                || stamp
                    .size
                    .iter()
                    .any(|value| !value.is_finite() || !(1e-6..=2.0).contains(value))
                || !stamp.rotation.is_finite()
                || !normalized(stamp.opacity)
                || !normalized(stamp.value)
            {
                return Err("invalid stamp center, size, rotation, opacity or value".into());
            }
            decode_alpha_image(&stamp.image)?;
        }
        if points > 16384 {
            return Err("mask exceeds 16384 points including ordered strokes".into());
        }
    }
    Ok(())
}

fn visit(
    name: &str,
    paint: &PaintStack,
    active: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> Result<(), String> {
    if done.contains(name) {
        return Ok(());
    }
    let mask = paint
        .masks
        .get(name)
        .ok_or_else(|| format!("unknown mask source: {name}"))?;
    if !active.insert(name.into()) {
        return Err(format!("mask reference cycle at {name}"));
    }
    if let Some(source) = &mask.source {
        visit(source, paint, active, done)?;
    }
    active.remove(name);
    done.insert(name.into());
    Ok(())
}

pub(super) fn resolve(
    mask: &Mask,
    paint: &PaintStack,
    width: usize,
    height: usize,
    cache: &mut BTreeMap<String, Vec<f64>>,
) -> Result<Vec<f64>, String> {
    let mut pixels = crate::painting::raster_mask(mask, width, height);
    if let Some(surface) = &mask.surface {
        for (value, coverage) in pixels.iter_mut().zip(surface.raster(width, height)?) {
            *value *= coverage;
        }
    }
    for mark in &mask.marks {
        match mark {
            Mark::Stamp { stamp } => stamp_pixels(&mut pixels, stamp, width, height)?,
            Mark::Stroke { stroke } => {
                crate::painting::raster_stroke(&mut pixels, stroke, width, height)
            }
        }
    }
    if let Some(source) = &mask.source {
        if !cache.contains_key(source) {
            let definition = paint.masks.get(source).ok_or("unknown mask source")?;
            let resolved = resolve(definition, paint, width, height, cache)?;
            cache.insert(source.clone(), resolved);
        }
        for (value, source) in pixels.iter_mut().zip(&cache[source]) {
            *value *= source;
        }
    }
    if let Some(transform) = &mask.transform {
        pixels = transform_pixels(&pixels, transform, width, height);
    }
    for filter in &mask.filters {
        pixels = filter_pixels(pixels, filter, width, height);
        for value in &mut pixels {
            *value = value.clamp(0.0, 1.0);
        }
    }
    Ok(pixels)
}

pub(super) fn sample(pixels: &[f64], uv: [f64; 2], width: usize, height: usize) -> f64 {
    let x = uv[0] * width as f64 - 0.5;
    let y = (1.0 - uv[1]) * height as f64 - 0.5;
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let point = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
            0.0
        } else {
            pixels[y as usize * width + x as usize]
        }
    };
    let top = point(ix, iy) * (1.0 - fx) + point(ix + 1, iy) * fx;
    let bottom = point(ix, iy + 1) * (1.0 - fx) + point(ix + 1, iy + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

fn decode_alpha_image(image: &Image) -> Result<Vec<u8>, String> {
    if image.width == 0 || image.height == 0 || image.width > 1024 || image.height > 1024 {
        return Err("alpha image dimensions must be 1..1024".into());
    }
    let size = image.width * image.height * 4;
    if image.rgba_base64.len() > size.div_ceil(3) * 4 {
        return Err("alpha image exceeds dimensions".into());
    }
    let bytes = base64::decode(&image.rgba_base64)?;
    if bytes.len() != size {
        return Err("alpha image byte count does not match dimensions".into());
    }
    Ok(bytes)
}

pub(super) fn alpha_pixels(image: &Image) -> Result<Vec<f64>, String> {
    let bytes = decode_alpha_image(image)?;
    Ok(bytes.chunks_exact(4)
        .map(|pixel| f64::from(pixel[0]) * f64::from(pixel[3]) / (255.0 * 255.0))
        .collect())
}

fn stamp_pixels(
    pixels: &mut [f64],
    stamp: &Stamp,
    width: usize,
    height: usize,
) -> Result<(), String> {
    let alpha = alpha_pixels(&stamp.image)?;
    let (sin, cos) = stamp.rotation.rem_euclid(360.0).to_radians().sin_cos();
    let extent = [
        (cos.abs() * stamp.size[0] + sin.abs() * stamp.size[1]) * 0.5,
        (sin.abs() * stamp.size[0] + cos.abs() * stamp.size[1]) * 0.5,
    ];
    let left = ((stamp.center[0] - extent[0]) * width as f64)
        .floor()
        .clamp(0.0, width as f64) as usize;
    let right = ((stamp.center[0] + extent[0]) * width as f64)
        .ceil()
        .clamp(0.0, width as f64) as usize;
    let top = ((1.0 - stamp.center[1] - extent[1]) * height as f64)
        .floor()
        .clamp(0.0, height as f64) as usize;
    let bottom = ((1.0 - stamp.center[1] + extent[1]) * height as f64)
        .ceil()
        .clamp(0.0, height as f64) as usize;
    for y in top..bottom {
        for x in left..right {
            let dx = (x as f64 + 0.5) / width as f64 - stamp.center[0];
            let dy = 1.0 - (y as f64 + 0.5) / height as f64 - stamp.center[1];
            let uv = [
                (dx * cos + dy * sin) / stamp.size[0] + 0.5,
                (-dx * sin + dy * cos) / stamp.size[1] + 0.5,
            ];
            if uv.iter().any(|value| *value < 0.0 || *value > 1.0) {
                continue;
            }
            let coverage =
                sample(&alpha, uv, stamp.image.width, stamp.image.height) * stamp.opacity;
            let index = y * width + x;
            pixels[index] = pixels[index] * (1.0 - coverage) + stamp.value * coverage;
        }
    }
    Ok(())
}

fn transform_pixels(
    pixels: &[f64],
    transform: &Transform,
    width: usize,
    height: usize,
) -> Vec<f64> {
    let (sin, cos) = transform.rotation.rem_euclid(360.0).to_radians().sin_cos();
    let mut result = vec![0.0; pixels.len()];
    for y in 0..height {
        for x in 0..width {
            let dx = (x as f64 + 0.5) / width as f64 - transform.center[0] - transform.offset[0];
            let dy =
                1.0 - (y as f64 + 0.5) / height as f64 - transform.center[1] - transform.offset[1];
            let uv = [
                transform.center[0] + (dx * cos + dy * sin) / transform.scale[0],
                transform.center[1] + (-dx * sin + dy * cos) / transform.scale[1],
            ];
            result[y * width + x] = sample(pixels, uv, width, height);
        }
    }
    result
}

fn sweep(values: &[f64], radius: usize, maximum: bool) -> Vec<f64> {
    let mut queue = VecDeque::<(usize, f64)>::new();
    let mut output = vec![0.0; values.len()];
    for index in 0..values.len() + 2 * radius {
        let value = if index >= radius && index - radius < values.len() {
            values[index - radius]
        } else {
            0.0
        };
        while queue.back().is_some_and(|(_, last)| {
            if maximum {
                *last <= value
            } else {
                *last >= value
            }
        }) {
            queue.pop_back();
        }
        queue.push_back((index, value));
        while queue
            .front()
            .is_some_and(|(first, _)| first + 2 * radius < index)
        {
            queue.pop_front();
        }
        if index >= 2 * radius {
            output[index - 2 * radius] = queue.front().unwrap().1;
        }
    }
    output
}

fn blur(values: &[f64], radius: usize) -> Vec<f64> {
    let mut prefix = vec![0.0; values.len() + 1];
    for (index, value) in values.iter().enumerate() {
        prefix[index + 1] = prefix[index] + value;
    }
    (0..values.len())
        .map(|index| {
            let low = index.saturating_sub(radius);
            let high = (index + radius + 1).min(values.len());
            (prefix[high] - prefix[low]) / (2 * radius + 1) as f64
        })
        .collect()
}

fn neighborhood(
    pixels: &[f64],
    width: usize,
    height: usize,
    radius: usize,
    maximum: Option<bool>,
) -> Vec<f64> {
    if radius == 0 {
        return pixels.to_vec();
    }
    let line = |values: &[f64]| {
        if let Some(maximum) = maximum {
            sweep(values, radius, maximum)
        } else {
            blur(values, radius)
        }
    };
    let mut horizontal = Vec::with_capacity(pixels.len());
    for row in pixels.chunks_exact(width) {
        horizontal.extend(line(row));
    }
    let mut output = vec![0.0; pixels.len()];
    for x in 0..width {
        let column: Vec<_> = (0..height).map(|y| horizontal[y * width + x]).collect();
        let filtered = line(&column);
        for y in 0..height {
            output[y * width + x] = filtered[y];
        }
    }
    output
}

fn filter_pixels(mut pixels: Vec<f64>, filter: &Filter, width: usize, height: usize) -> Vec<f64> {
    let radius = |uv: f64| (uv * width.min(height) as f64).round() as usize;
    match filter {
        Filter::Invert => {
            for value in &mut pixels {
                *value = 1.0 - *value;
            }
            pixels
        }
        Filter::Levels { low, high, gamma } => {
            for value in &mut pixels {
                *value = ((*value - low) / (high - low))
                    .clamp(0.0, 1.0)
                    .powf(1.0 / gamma);
            }
            pixels
        }
        Filter::Blur { radius: uv } => neighborhood(&pixels, width, height, radius(*uv), None),
        Filter::Dilate { radius: uv } => {
            neighborhood(&pixels, width, height, radius(*uv), Some(true))
        }
        Filter::Erode { radius: uv } => {
            neighborhood(&pixels, width, height, radius(*uv), Some(false))
        }
        Filter::Outline { radius: uv, side } => {
            let expanded = if side == "inner" {
                pixels.clone()
            } else {
                neighborhood(&pixels, width, height, radius(*uv), Some(true))
            };
            let contracted = if side == "outer" {
                pixels
            } else {
                neighborhood(&pixels, width, height, radius(*uv), Some(false))
            };
            expanded
                .into_iter()
                .zip(contracted)
                .map(|(a, b)| (a - b).clamp(0.0, 1.0))
                .collect()
        }
    }
}

pub(super) fn metadata(mask: &Mask) -> Value {
    let mut result = serde_json::to_value(mask).unwrap();
    if let Some(surface) = result.get_mut("surface").and_then(Value::as_object_mut) {
        let triangle_count = surface.remove("triangles").and_then(|value| value.as_array().map(Vec::len));
        surface.insert("triangle_count".into(), json!(triangle_count));
        if let Some(image) = surface.get_mut("image").and_then(Value::as_object_mut) {
            image.remove("rgba_base64");
        }
    }
    if let Some(marks) = result["marks"].as_array_mut() {
        for mark in marks {
            if let Some(image) = mark["image"].as_object_mut() {
                image.remove("rgba_base64");
            }
        }
    }
    result
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let name = request["texture"].as_str().ok_or("texture is required")?;
    let texture = mesh.textures.get(name).ok_or("unknown texture")?;
    let masks = texture.paint.as_ref().map(|paint| &paint.masks);
    let (offset, limit) = crate::responses::page(request)?;
    let items: Vec<_> = masks
        .into_iter()
        .flatten()
        .skip(offset)
        .take(limit)
        .map(|(name, mask)| json!({"name":name,"mask":metadata(mask)}))
        .collect();
    let total = masks.map_or(0, |masks| masks.len());
    let next = offset.saturating_add(limit);
    Ok(
        json!({"revision":revision,"texture":name,"items":items,"offset":offset,"total":total,"next_offset":if next<total{Some(next)}else{None}}),
    )
}
