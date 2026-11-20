use serde_json::{Value, json};
use std::io::Cursor;

fn decode(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_limits(png::Limits {
        bytes: 64 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    if info.width > 8192
        || info.height > 8192
        || u64::from(info.width) * u64::from(info.height) > 16_777_216
    {
        return Err("reference PNG exceeds 16 megapixels or 8192 pixels per dimension".into());
    }
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let channels = info.color_type.samples();
    let mut rgb = Vec::with_capacity(info.width as usize * info.height as usize * 3);
    for pixel in buffer[..info.buffer_size()].chunks_exact(channels) {
        let color = match info.color_type {
            png::ColorType::Rgb | png::ColorType::Rgba => [pixel[0], pixel[1], pixel[2]],
            png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha => [pixel[0]; 3],
            _ => return Err("unsupported PNG color format".into()),
        };
        let alpha = match info.color_type {
            png::ColorType::Rgba => u32::from(pixel[3]),
            png::ColorType::GrayscaleAlpha => u32::from(pixel[1]),
            _ => 255,
        };
        for channel in color {
            rgb.push(((u32::from(channel) * alpha + 235 * (255 - alpha)) / 255) as u8);
        }
    }
    Ok((info.width as usize, info.height as usize, rgb))
}

pub(crate) fn dimensions(bytes: &[u8]) -> Result<[usize; 2], String> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = [reader.info().width as usize, reader.info().height as usize];
    let invalid = size[0] > 8192 || size[1] > 8192 || size[0] * size[1] > 16_777_216;
    if invalid {
        return Err("reference PNG exceeds 16 megapixels or 8192 pixels per dimension".into());
    }
    Ok(size)
}

pub(crate) struct Frame {
    pub crop: [usize; 4],
    pub scale: f64,
    width: usize,
    height: usize,
    fitted: [usize; 2],
    offset: [usize; 2],
}

impl Frame {
    pub fn new(
        size: [usize; 2],
        request: &Value,
        width: usize,
        height: usize,
    ) -> Result<Self, String> {
        let crop: [usize; 4] = request
            .get("crop")
            .map(|crop| {
                serde_json::from_value(crop.clone())
                    .map_err(|_| "crop must be [left,top,right,bottom] integer pixels")
            })
            .unwrap_or(Ok([0, 0, size[0], size[1]]))?;
        let invalid =
            crop[0] >= crop[2] || crop[1] >= crop[3] || crop[2] > size[0] || crop[3] > size[1];
        if invalid {
            return Err("crop is outside reference image".into());
        }
        let scale = (width as f64 / (crop[2] - crop[0]) as f64)
            .min(height as f64 / (crop[3] - crop[1]) as f64);
        let fitted = [
            ((crop[2] - crop[0]) as f64 * scale).round() as usize,
            ((crop[3] - crop[1]) as f64 * scale).round() as usize,
        ];
        Ok(Self {
            crop,
            scale,
            width,
            height,
            fitted,
            offset: [(width - fitted[0]) / 2, (height - fitted[1]) / 2],
        })
    }

    pub fn to_canvas(&self, pixel: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|axis| {
            self.offset[axis] as f64 + (pixel[axis] - self.crop[axis] as f64) * self.scale
        })
    }

    pub fn to_image(&self, pixel: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|axis| {
            self.crop[axis] as f64 + (pixel[axis] - self.offset[axis] as f64) / self.scale
        })
    }

    fn background(&self, source: &[u8], source_width: usize) -> Vec<u8> {
        let mut output = vec![235; self.width * self.height * 3];
        for y in 0..self.fitted[1] {
            for x in 0..self.fitted[0] {
                let sx = (self.crop[0] + (x as f64 / self.scale) as usize).min(self.crop[2] - 1);
                let sy = (self.crop[1] + (y as f64 / self.scale) as usize).min(self.crop[3] - 1);
                let target = ((self.offset[1] + y) * self.width + self.offset[0] + x) * 3;
                let offset = (sy * source_width + sx) * 3;
                output[target..target + 3].copy_from_slice(&source[offset..offset + 3]);
            }
        }
        output
    }
}

fn pixel(output: &mut [u8], width: usize, x: i64, y: i64, color: [u8; 3]) {
    let height = output.len() / 3 / width;
    if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
        return;
    }
    let offset = (y as usize * width + x as usize) * 3;
    output[offset..offset + 3].copy_from_slice(&color);
}

fn line(output: &mut [u8], width: usize, first: [f64; 2], second: [f64; 2], color: [u8; 3]) {
    let bounds = [width as f64 - 1.0, (output.len() / 3 / width) as f64 - 1.0];
    let delta = [second[0] - first[0], second[1] - first[1]];
    if delta.iter().any(|value| !value.is_finite()) {
        return;
    }
    let mut low: f64 = 0.0;
    let mut high: f64 = 1.0;
    for axis in 0..2 {
        if delta[axis].abs() < 1e-12 {
            if first[axis] < 0.0 || first[axis] > bounds[axis] {
                return;
            }
        } else {
            let a = -first[axis] / delta[axis];
            let b = (bounds[axis] - first[axis]) / delta[axis];
            low = low.max(a.min(b));
            high = high.min(a.max(b));
        }
    }
    if low > high {
        return;
    }
    let start: [f64; 2] = std::array::from_fn(|axis| first[axis] + delta[axis] * low);
    let end: [f64; 2] = std::array::from_fn(|axis| first[axis] + delta[axis] * high);
    let steps = ((end[0] - start[0])
        .abs()
        .max((end[1] - start[1]).abs())
        .ceil() as usize)
        .max(1);
    for step in 0..=steps {
        let t = step as f64 / steps as f64;
        pixel(
            output,
            width,
            (start[0] + (end[0] - start[0]) * t).round() as i64,
            (start[1] + (end[1] - start[1]) * t).round() as i64,
            color,
        );
    }
}

fn annotate(output: &mut [u8], width: usize, alignment: &Value) {
    let entries = alignment["landmarks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for contour in alignment["contours"].as_array().into_iter().flatten() {
        let Some(names) = contour["landmarks"].as_array() else {
            continue;
        };
        let mut points: Vec<_> = names
            .iter()
            .filter_map(|name| entries.iter().find(|entry| entry["name"] == *name))
            .collect();
        if contour["closed"] == true && !points.is_empty() {
            points.push(points[0]);
        }
        for pair in points.windows(2) {
            for (field, color) in [
                ("target_canvas", [240, 30, 180]),
                ("projected_canvas", [0, 190, 255]),
            ] {
                if let (Ok(a), Ok(b)) = (
                    crate::projection::pair(&pair[0][field]),
                    crate::projection::pair(&pair[1][field]),
                ) {
                    line(output, width, a, b, color);
                }
            }
        }
    }
    for entry in entries {
        let Ok(target) = crate::projection::pair(&entry["target_canvas"]) else {
            continue;
        };
        if let Ok(projected) = crate::projection::pair(&entry["projected_canvas"]) {
            line(output, width, target, projected, [255, 200, 0]);
        }
        for (field, color) in [
            ("target_canvas", [240, 30, 180]),
            ("projected_canvas", [0, 190, 255]),
        ] {
            if let Ok(point) = crate::projection::pair(&entry[field]) {
                line(
                    output,
                    width,
                    [point[0] - 4.0, point[1]],
                    [point[0] + 4.0, point[1]],
                    color,
                );
                line(
                    output,
                    width,
                    [point[0], point[1] - 4.0],
                    [point[0], point[1] + 4.0],
                    color,
                );
            }
        }
    }
}

pub(crate) fn compose(
    reference: &[u8],
    capture: &Value,
    request: &Value,
    alignment: &Value,
) -> Result<Value, String> {
    let (source_width, source_height, source) = decode(reference)?;
    let encoded = capture["png_base64"].as_str().ok_or("missing capture")?;
    let bytes = crate::base64::decode(encoded)?;
    let (width, height, rendered) = decode(&bytes)?;
    let frame = Frame::new([source_width, source_height], request, width, height)?;
    let background = frame.background(&source, source_width);
    let overlay = match request
        .get("layout")
        .and_then(Value::as_str)
        .unwrap_or("side_by_side")
    {
        "side_by_side" => false,
        "overlay" => true,
        _ => return Err("layout must be side_by_side or overlay".into()),
    };
    let output_width = if overlay { width } else { width * 2 };
    let mut output = if overlay {
        background.clone()
    } else {
        vec![235u8; output_width * height * 3]
    };
    if overlay {
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let inside = rendered[index * 3] < 128;
                let boundary = x == 0
                    || y == 0
                    || x + 1 == width
                    || y + 1 == height
                    || rendered[(index - 1) * 3] >= 128
                    || rendered[(index + 1) * 3] >= 128
                    || rendered[(index - width) * 3] >= 128
                    || rendered[(index + width) * 3] >= 128;
                if inside && boundary {
                    pixel(&mut output, width, x as i64, y as i64, [0, 190, 255]);
                }
            }
        }
        annotate(&mut output, width, alignment);
    } else {
        for y in 0..height {
            let target = y * output_width * 3;
            let source = y * width * 3;
            output[target..target + width * 3]
                .copy_from_slice(&background[source..source + width * 3]);
            output[target + width * 3..target + width * 6]
                .copy_from_slice(&rendered[source..source + width * 3]);
        }
    }
    let mut png = vec![];
    {
        let mut encoder = png::Encoder::new(&mut png, output_width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&output)
            .map_err(|e| e.to_string())?;
    }
    Ok(
        json!({"png_base64":crate::base64::encode(&png),"camera":capture["camera"],"visibility":capture["visibility"],"revision":capture["revision"],"reference":request["reference"],"crop":frame.crop,"layout":if overlay {"overlay"} else {"side_by_side"},"legend":{"reference_landmarks":"magenta","projected_mesh":"cyan","landmark_residual":"yellow"},"alignment":alignment,"width":output_width,"height":height}),
    )
}
