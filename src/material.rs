use crate::{normals::unit, projection::dot, uv::Texture};
use std::collections::BTreeMap;

pub(super) struct Gradients {
    dx: [f64; 3],
    dy: [f64; 3],
}

impl Gradients {
    pub fn new(points: [[f64; 3]; 3], uv: [[f64; 2]; 3], perspective: bool) -> Self {
        let [a, b, c] = points;
        let det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        let weighted = std::array::from_fn::<_, 3, _>(|i| {
            let q = if perspective { points[i][2] } else { 1.0 };
            [uv[i][0] * q, uv[i][1] * q, q]
        });
        let dx = std::array::from_fn(|k| {
            ((weighted[1][k] - weighted[0][k]) * (c[1] - a[1])
                - (weighted[2][k] - weighted[0][k]) * (b[1] - a[1]))
                / det
        });
        let dy = std::array::from_fn(|k| {
            ((weighted[2][k] - weighted[0][k]) * (b[0] - a[0])
                - (weighted[1][k] - weighted[0][k]) * (c[0] - a[0]))
                / det
        });
        Self { dx, dy }
    }

    pub fn lod(&self, uv: [f64; 2], q: f64, dimensions: [f64; 2]) -> f64 {
        let length = |gradient: [f64; 3]| {
            let du = (gradient[0] - uv[0] * gradient[2]) * dimensions[0] / q;
            let dv = (gradient[1] - uv[1] * gradient[2]) * dimensions[1] / q;
            du.hypot(dv)
        };
        length(self.dx).max(length(self.dy)).max(1.0).log2()
    }
}

struct Level {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Level {
    fn downsample(&self) -> Self {
        let width = (self.width / 2).max(1);
        let height = (self.height / 2).max(1);
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0u32; 4];
                let mut count = 0;
                for sy in y * self.height / height..(y + 1) * self.height / height {
                    for sx in x * self.width / width..(x + 1) * self.width / width {
                        for (channel, value) in sum.iter_mut().enumerate() {
                            *value += self.pixels[4 * (sy * self.width + sx) + channel] as u32;
                        }
                        count += 1;
                    }
                }
                pixels.extend(sum.map(|value| ((value + count / 2) / count) as u8));
            }
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    fn bilinear(&self, uv: [f64; 2]) -> [f64; 4] {
        let x = uv[0].rem_euclid(1.0) * self.width as f64 - 0.5;
        let y = (1.0 - uv[1].rem_euclid(1.0)) * self.height as f64 - 0.5;
        let mut color = [0.0; 4];
        for (dy, wy) in [(0, 1.0 - (y - y.floor())), (1, y - y.floor())] {
            for (dx, wx) in [(0, 1.0 - (x - x.floor())), (1, x - x.floor())] {
                let sx = (x.floor() as isize + dx).rem_euclid(self.width as isize) as usize;
                let sy = (y.floor() as isize + dy).rem_euclid(self.height as isize) as usize;
                let offset = (sy * self.width + sx) * 4;
                for (channel, value) in color.iter_mut().enumerate() {
                    *value += wx * wy * f64::from(self.pixels[offset + channel]) / 255.0;
                }
            }
        }
        color
    }
}

pub(super) struct Prepared {
    pub dimensions: [f64; 2],
    pub double_sided: bool,
    linear: bool,
    alpha_cutoff: Option<f64>,
    maps: BTreeMap<String, Vec<Level>>,
}

impl Prepared {
    pub fn new(texture: &Texture) -> Result<Self, String> {
        let linear = texture.filter == Some(crate::uv::Filter::Linear);
        let mut maps = BTreeMap::new();
        for name in ["base_color", "normal", "roughness", "metallic", "specular"] {
            let pixels = crate::painting::map_pixels(texture, name)?;
            let mut levels = vec![Level {
                width: texture.width,
                height: texture.height,
                pixels,
            }];
            if linear {
                while levels
                    .last()
                    .is_some_and(|level| level.width > 1 || level.height > 1)
                {
                    let level = levels.last().unwrap().downsample();
                    levels.push(level);
                }
            }
            maps.insert(name.into(), levels);
        }
        Ok(Self {
            dimensions: [texture.width as f64, texture.height as f64],
            double_sided: texture.double_sided,
            alpha_cutoff: texture.alpha_cutoff,
            linear,
            maps,
        })
    }

    fn sample(&self, name: &str, uv: [f64; 2], lod: f64) -> [f64; 4] {
        let levels = &self.maps[name];
        let base = &levels[0];
        if !self.linear {
            let index = crate::uv::texel_offset(base.width, base.height, uv);
            return std::array::from_fn(|axis| f64::from(base.pixels[index + axis]) / 255.0);
        }
        let lod = lod.clamp(0.0, (levels.len() - 1) as f64);
        let a = levels[lod.floor() as usize].bilinear(uv);
        if lod.fract() == 0.0 {
            return a;
        }
        let b = levels[lod.ceil() as usize].bilinear(uv);
        std::array::from_fn(|axis| a[axis] + (b[axis] - a[axis]) * lod.fract())
    }

    pub fn masked(&self, uv: [f64; 2], lod: f64) -> bool {
        self.alpha_cutoff
            .is_some_and(|cutoff| self.sample("base_color", uv, lod)[3] < cutoff)
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn frame(points: [[f64; 3]; 3], uvs: [[f64; 2]; 3]) -> Option<[[f64; 3]; 2]> {
    let a: [f64; 3] = std::array::from_fn(|axis| points[1][axis] - points[0][axis]);
    let b: [f64; 3] = std::array::from_fn(|axis| points[2][axis] - points[0][axis]);
    let u = [uvs[1][0] - uvs[0][0], uvs[2][0] - uvs[0][0]];
    let v = [uvs[1][1] - uvs[0][1], uvs[2][1] - uvs[0][1]];
    let determinant = u[0] * v[1] - u[1] * v[0];
    if determinant.abs() < 1e-12 {
        return None;
    }
    Some([
        std::array::from_fn(|axis| (a[axis] * v[1] - b[axis] * v[0]) / determinant),
        std::array::from_fn(|axis| (b[axis] * u[0] - a[axis] * u[1]) / determinant),
    ])
}

pub(super) fn shade(
    material: Option<&Prepared>,
    uv: [f64; 2],
    lod: f64,
    paint: [f64; 3],
    normal: [f64; 3],
    frame: Option<[[f64; 3]; 2]>,
    view: [f64; 3],
    back_facing: bool,
) -> [u8; 3] {
    let sample = |name: &str, default: [f64; 3]| {
        material.map_or(default, |material| {
            let rgba = material.sample(name, uv, lod);
            [rgba[0], rgba[1], rgba[2]]
        })
    };
    let base = sample("base_color", [0.5; 3]);
    let base: [f64; 3] = std::array::from_fn(|axis| (base[axis] * paint[axis]).powf(2.2));
    let roughness = sample("roughness", [0.65; 3])[0].clamp(0.04, 1.0);
    let metallic = sample("metallic", [0.0; 3])[0];
    let specular = sample("specular", [0.04; 3]);
    let mut n = normal;
    if let Some([t, b]) = frame {
        let tangent = unit(std::array::from_fn(|axis| {
            t[axis] - normal[axis] * dot(t, normal)
        }));
        let hand = if dot(cross(normal, tangent), b) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let bitangent = cross(normal, tangent).map(|value| value * hand);
        let mapped = unit(sample("normal", [0.5, 0.5, 1.0]).map(|value| value * 2.0 - 1.0));
        n = unit(std::array::from_fn(|axis| {
            tangent[axis] * mapped[0] + bitangent[axis] * mapped[1] + normal[axis] * mapped[2]
        }));
    }
    let view = unit(view);
    if back_facing {
        n = n.map(|value| -value);
    }
    let ambient = 0.06 + 0.18 * (n[1] * 0.5 + 0.5);
    let overhead = (2.0 * dot(n, view) * n[1] - view[1]).max(0.0);
    let environment = 0.04 + 0.35 * (overhead * overhead * (1.0 - roughness) + 0.35 * roughness);
    let mut diffuse = 0.0;
    let mut highlight = 0.0;
    let exponent = (2.0 / (roughness * roughness) - 2.0).clamp(2.0, 512.0);
    for (direction, weight) in [
        ([-3.0, 5.0, 4.0], 0.8),
        ([4.0, 2.0, 1.0], 0.27),
        ([1.0, 3.0, -4.0], 0.38),
    ] {
        let light = unit(direction);
        let facing = dot(n, light).max(0.0);
        diffuse += facing * weight;
        if facing > 0.0 {
            let halfway = unit(std::array::from_fn(|axis| light[axis] + view[axis]));
            highlight +=
                dot(n, halfway).max(0.0).powf(exponent) * weight * (0.5 + 1.5 * (1.0 - roughness));
        }
    }
    let linear: [f64; 3] = std::array::from_fn(|axis| {
        let f0 = specular[axis] * (1.0 - metallic) + base[axis] * metallic;
        base[axis] * (1.0 - metallic) * (ambient + diffuse) + f0 * (highlight + environment)
    });
    let shoulder = (linear.into_iter().fold(0.0, f64::max) - 0.65).max(0.0);
    let scale = 1.0 + shoulder * shoulder / (shoulder + 0.35);
    linear.map(|channel| {
        ((channel / scale).powf(1.0 / 2.2) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}
