use crate::{normals::unit, projection::dot, uv::Texture};
use std::collections::BTreeMap;

pub(super) struct Prepared {
    width: usize,
    height: usize,
    maps: BTreeMap<String, Vec<u8>>,
}

impl Prepared {
    pub fn new(texture: &Texture) -> Result<Self, String> {
        let mut maps = BTreeMap::new();
        for name in ["base_color", "normal", "roughness", "metallic", "specular"] {
            maps.insert(name.into(), crate::painting::map_pixels(texture, name)?);
        }
        Ok(Self {
            width: texture.width,
            height: texture.height,
            maps,
        })
    }

    pub fn sample(&self, name: &str, uv: [f64; 2]) -> [f64; 3] {
        let x = ((uv[0].rem_euclid(1.0) * self.width as f64).floor() as usize).min(self.width - 1);
        let row =
            ((uv[1].rem_euclid(1.0) * self.height as f64).floor() as usize).min(self.height - 1);
        let index = ((self.height - 1 - row) * self.width + x) * 4;
        let pixels = &self.maps[name];
        let alpha = if name == "base_color" {
            f64::from(pixels[index + 3]) / 255.0
        } else {
            1.0
        };
        std::array::from_fn(|axis| {
            (f64::from(pixels[index + axis]) * alpha + 128.0 * (1.0 - alpha)) / 255.0
        })
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
    paint: [f64; 3],
    normal: [f64; 3],
    frame: Option<[[f64; 3]; 2]>,
    view: [f64; 3],
) -> [u8; 3] {
    let sample = |name: &str, default: [f64; 3]| {
        material.map_or(default, |material| material.sample(name, uv))
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
    if dot(normal, view) < 0.0 {
        n = n.map(|value| -value);
    }
    let ambient = 0.20 + 0.15 * (n[1] * 0.5 + 0.5);
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
    std::array::from_fn(|axis| {
        let f0 = specular[axis] * (1.0 - metallic) + base[axis] * metallic;
        let linear = base[axis] * (ambient + (1.0 - metallic) * diffuse) + f0 * highlight;
        ((linear / (linear + 0.65)).powf(1.0 / 2.2) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}
