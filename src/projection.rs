use crate::{render, vector};
use serde_json::{Value, json};

#[derive(Clone)]
pub(crate) struct Camera {
    pub width: usize,
    pub height: usize,
    pub pixels_per_unit: f64,
    pub center: [f64; 3],
    pub right: [f64; 3],
    pub up: [f64; 3],
    pub forward: [f64; 3],
    yaw: f64,
    pitch: f64,
    zoom: f64,
    fov: Option<f64>,
    near: f64,
    lens_shift: [f64; 2],
}

#[derive(Clone, Copy)]
pub(crate) struct ClipVertex {
    pub position: [f64; 3],
    pub weights: [f64; 3],
}

pub(crate) fn pair(value: &Value) -> Result<[f64; 2], String> {
    let pair: [f64; 2] =
        serde_json::from_value(value.clone()).map_err(|_| "expected two finite numbers")?;
    if pair.iter().any(|number| !number.is_finite()) {
        return Err("expected two finite numbers".into());
    }
    Ok(pair)
}

pub(crate) fn dot(first: [f64; 3], second: [f64; 3]) -> f64 {
    first.iter().zip(second).map(|(a, b)| a * b).sum()
}

impl Camera {
    pub fn parse(request: &Value) -> Result<Self, String> {
        let options = render::Options::parse(request)?;
        let number = |name: &str, default: f64| -> Result<f64, String> {
            request
                .get(name)
                .map(|value| {
                    value
                        .as_f64()
                        .ok_or_else(|| format!("{name} must be numeric"))
                })
                .unwrap_or(Ok(default))
        };
        let projection = request
            .get("projection")
            .map(|value| value.as_str().ok_or("projection must be text"))
            .transpose()?
            .unwrap_or("orthographic");
        if projection != "orthographic" && projection != "perspective" {
            return Err("projection must be orthographic or perspective".into());
        }
        let mut yaw = number("yaw", 35.0)?.to_radians();
        let mut pitch = number("pitch", 25.0)?.to_radians();
        let zoom = number("zoom", 100.0)?;
        let perspective = projection == "perspective";
        let center = if perspective {
            vector(
                request
                    .get("position")
                    .ok_or("perspective requires position:[x,y,z]")?,
            )?
        } else {
            request.get("center").map(vector).unwrap_or(Ok([0.0; 3]))?
        };
        if perspective && request.get("target").is_some() {
            let target = vector(&request["target"])?;
            let toward: [f64; 3] = std::array::from_fn(|axis| center[axis] - target[axis]);
            let horizontal = toward[0].hypot(toward[2]);
            if horizontal < 1e-10 {
                return Err("target must define a nonvertical camera direction".into());
            }
            yaw = toward[0].atan2(toward[2]);
            pitch = toward[1].atan2(horizontal);
        }
        let fov = if perspective {
            Some(number("fov", 60.0)?)
        } else {
            None
        };
        let near = number("near", 0.05)?;
        let lens_shift = request
            .get("lens_shift")
            .map(pair)
            .unwrap_or(Ok([0.0; 2]))?;
        if !perspective && lens_shift != [0.0; 2] {
            return Err("lens_shift is supported only for perspective cameras".into());
        }
        if fov.is_some_and(|value| !value.is_finite() || !(5.0..=150.0).contains(&value))
            || !near.is_finite()
            || near <= 0.0
            || lens_shift.iter().any(|value| value.abs() > 2.0)
        {
            return Err("fov must be 5..150 degrees, near positive, and lens_shift within +/-2 image dimensions".into());
        }
        let resolution_scale = (options.width as f64 / 800.0).min(options.height as f64 / 600.0);
        let pixels_per_unit = zoom * resolution_scale;
        let invalid = !yaw.is_finite()
            || !pitch.is_finite()
            || !zoom.is_finite()
            || zoom <= 0.0
            || !pixels_per_unit.is_finite()
            || pixels_per_unit <= 0.0;
        if invalid {
            return Err("invalid camera".into());
        }
        Ok(Self {
            width: options.width,
            height: options.height,
            pixels_per_unit,
            center,
            right: [yaw.cos(), 0.0, -yaw.sin()],
            up: [
                -pitch.sin() * yaw.sin(),
                pitch.cos(),
                -pitch.sin() * yaw.cos(),
            ],
            forward: [
                yaw.sin() * pitch.cos(),
                pitch.sin(),
                yaw.cos() * pitch.cos(),
            ],
            yaw,
            pitch,
            zoom,
            fov,
            near,
            lens_shift,
        })
    }

    pub fn perspective(&self) -> bool {
        self.fov.is_some()
    }

    pub fn near_depth(&self) -> f64 {
        -self.near
    }

    pub fn contains(&self, point: [f64; 3]) -> bool {
        !self.perspective() || -self.camera_depth(point) >= self.near * (1.0 - 1e-10)
    }

    pub fn camera_depth(&self, point: [f64; 3]) -> f64 {
        dot(
            std::array::from_fn(|axis| point[axis] - self.center[axis]),
            self.forward,
        )
    }

    fn focal_length(&self) -> f64 {
        self.height as f64 / (2.0 * (self.fov.unwrap_or(60.0).to_radians() * 0.5).tan())
    }

    pub fn project(&self, point: [f64; 3]) -> Result<[f64; 3], String> {
        let local = std::array::from_fn(|axis| point[axis] - self.center[axis]);
        if !self.contains(point) {
            return Err("point is behind the perspective near plane".into());
        }
        let scale = if self.perspective() {
            self.focal_length() / -dot(local, self.forward)
        } else {
            self.pixels_per_unit
        };
        let projected = [
            self.width as f64 * (0.5 + self.lens_shift[0]) + dot(local, self.right) * scale,
            self.height as f64 * (0.5 + self.lens_shift[1]) - dot(local, self.up) * scale,
            dot(local, self.forward),
        ];
        if projected.iter().any(|value| !value.is_finite()) {
            return Err("camera projection exceeds numeric range".into());
        }
        Ok(projected)
    }

    pub fn raster_point(&self, point: [f64; 3]) -> Result<[f64; 3], String> {
        let mut projected = self.project(point)?;
        if self.perspective() {
            projected[2] = -1.0 / projected[2];
        }
        Ok(projected)
    }

    pub fn linear_depth(&self, raster_depth: f64) -> f64 {
        if self.perspective() {
            -1.0 / raster_depth
        } else {
            raster_depth
        }
    }

    pub fn toward(&self, point: [f64; 3]) -> [f64; 3] {
        if self.perspective() {
            std::array::from_fn(|axis| self.center[axis] - point[axis])
        } else {
            self.forward
        }
    }

    pub fn displacement(&self, pixels: [f64; 2], anchor: [f64; 3]) -> Result<[f64; 3], String> {
        if !self.contains(anchor) {
            return Err("move anchor is behind the perspective near plane".into());
        }
        let scale = if self.perspective() {
            self.focal_length() / -self.camera_depth(anchor)
        } else {
            self.pixels_per_unit
        };
        Ok(std::array::from_fn(|axis| {
            (self.right[axis] * pixels[0] - self.up[axis] * pixels[1]) / scale
        }))
    }

    pub fn clip_triangle(&self, points: [[f64; 3]; 3]) -> Vec<[ClipVertex; 3]> {
        let polygon: Vec<_> = points
            .iter()
            .enumerate()
            .map(|(index, point)| ClipVertex {
                position: *point,
                weights: std::array::from_fn(|axis| if axis == index { 1.0 } else { 0.0 }),
            })
            .collect();
        let mut clipped = vec![];
        for index in 0..polygon.len() {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            let inside_a = self.contains(a.position);
            let inside_b = self.contains(b.position);
            if inside_a {
                clipped.push(a);
            }
            if inside_a != inside_b {
                let first = -self.camera_depth(a.position);
                let second = -self.camera_depth(b.position);
                let fraction = (self.near - first) / (second - first);
                clipped.push(ClipVertex {
                    position: std::array::from_fn(|axis| {
                        a.position[axis] + fraction * (b.position[axis] - a.position[axis])
                    }),
                    weights: std::array::from_fn(|axis| {
                        a.weights[axis] + fraction * (b.weights[axis] - a.weights[axis])
                    }),
                });
            }
        }
        (1..clipped.len().saturating_sub(1))
            .map(|index| [clipped[0], clipped[index], clipped[index + 1]])
            .collect()
    }

    pub fn raster_segment(
        &self,
        mut a: [f64; 3],
        mut b: [f64; 3],
    ) -> Result<Option<[[f64; 3]; 2]>, String> {
        let inside_a = self.contains(a);
        let inside_b = self.contains(b);
        if !inside_a && !inside_b {
            return Ok(None);
        }
        if inside_a != inside_b {
            let first = -self.camera_depth(a);
            let second = -self.camera_depth(b);
            let fraction = (self.near - first) / (second - first);
            let intersection = std::array::from_fn(|axis| a[axis] + fraction * (b[axis] - a[axis]));
            if inside_a {
                b = intersection;
            } else {
                a = intersection;
            }
        }
        Ok(Some([self.raster_point(a)?, self.raster_point(b)?]))
    }

    pub fn metadata(&self) -> Value {
        if let Some(fov) = self.fov {
            json!({"projection":"perspective","position":self.center,"yaw":self.yaw.to_degrees(),"pitch":self.pitch.to_degrees(),"fov":fov,"near":self.near,"lens_shift":self.lens_shift,"width":self.width,"height":self.height,"focal_length_pixels":self.focal_length()})
        } else {
            json!({"yaw":self.yaw.to_degrees(),"pitch":self.pitch.to_degrees(),"zoom":self.zoom,"center":self.center,"width":self.width,"height":self.height,"pixels_per_unit":self.pixels_per_unit,"projection":"orthographic"})
        }
    }
}
