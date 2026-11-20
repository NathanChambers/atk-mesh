pub(super) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(super) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(super) fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|v| v * s)
}
pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
pub(super) fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    add(mul(a, 1.0 - t), mul(b, t))
}

pub(super) fn segment_triangle(
    a: [f64; 3],
    b: [f64; 3],
    t: [[f64; 3]; 3],
    eps: f64,
) -> Option<[f64; 3]> {
    let direction = sub(b, a);
    let e1 = sub(t[1], t[0]);
    let e2 = sub(t[2], t[0]);
    let p = cross(direction, e2);
    let det = dot(e1, p);
    if det.abs() <= eps * length(direction) * length(e1) * length(e2) {
        return None;
    }
    let s = sub(a, t[0]);
    let u = dot(s, p) / det;
    let q = cross(s, e1);
    let v = dot(direction, q) / det;
    let along = dot(e2, q) / det;
    if u >= -eps && v >= -eps && u + v <= 1.0 + eps && along >= -eps && along <= 1.0 + eps {
        Some(lerp(a, b, along.clamp(0.0, 1.0)))
    } else {
        None
    }
}

pub(super) fn closest_triangle(p: [f64; 3], t: [[f64; 3]; 3]) -> [f64; 3] {
    let [a, b, c] = t;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return lerp(a, b, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return lerp(a, c, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return lerp(b, c, (d4 - d3) / (d4 - d3 + d5 - d6));
    }
    let sum = va + vb + vc;
    if sum.abs() < 1e-30 {
        return a;
    }
    add(a, add(mul(ab, vb / sum), mul(ac, vc / sum)))
}

pub(super) fn closest_segments(
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
    d: [f64; 3],
) -> ([f64; 3], [f64; 3]) {
    let u = sub(b, a);
    let v = sub(d, c);
    let w = sub(a, c);
    let aa = dot(u, u);
    let bb = dot(u, v);
    let cc = dot(v, v);
    let dd = dot(u, w);
    let ee = dot(v, w);
    let denominator = aa * cc - bb * bb;
    let mut s = if denominator > 1e-30 {
        ((bb * ee - cc * dd) / denominator).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = if cc > 1e-30 {
        ((bb * s + ee) / cc).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if aa > 1e-30 {
        s = ((bb * t - dd) / aa).clamp(0.0, 1.0);
    }
    if cc > 1e-30 {
        t = ((bb * s + ee) / cc).clamp(0.0, 1.0);
    }
    (lerp(a, b, s), lerp(c, d, t))
}

pub(super) fn triangle_box(triangle: [[f64; 3]; 3], min: [f64; 3], max: [f64; 3]) -> bool {
    let center = lerp(min, max, 0.5);
    let half = mul(sub(max, min), 0.5);
    let points = triangle.map(|p| sub(p, center));
    let edges = [
        sub(points[1], points[0]),
        sub(points[2], points[1]),
        sub(points[0], points[2]),
    ];
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut tests = axes.to_vec();
    tests.push(cross(edges[0], edges[1]));
    for edge in edges {
        for axis in axes {
            tests.push(cross(edge, axis));
        }
    }
    for axis in tests {
        let lo = points
            .iter()
            .map(|p| dot(*p, axis))
            .fold(f64::INFINITY, f64::min);
        let hi = points
            .iter()
            .map(|p| dot(*p, axis))
            .fold(f64::NEG_INFINITY, f64::max);
        let radius: f64 = (0..3).map(|i| half[i] * axis[i].abs()).sum();
        if lo > radius || hi < -radius {
            return false;
        }
    }
    true
}

pub(super) fn triangle_box_location(
    triangle: [[f64; 3]; 3],
    min: [f64; 3],
    max: [f64; 3],
) -> Option<[f64; 3]> {
    let mut polygon = triangle.to_vec();
    for axis in 0..3 {
        for (plane, sign) in [(min[axis], 1.0), (max[axis], -1.0)] {
            let input = polygon;
            polygon = vec![];
            if input.is_empty() {
                return None;
            }
            let mut previous = *input.last().unwrap();
            let mut d0 = (previous[axis] - plane) * sign;
            for current in input {
                let d1 = (current[axis] - plane) * sign;
                if (d0 >= 0.0) != (d1 >= 0.0) {
                    polygon.push(lerp(previous, current, d0 / (d0 - d1)));
                }
                if d1 >= 0.0 {
                    polygon.push(current);
                }
                previous = current;
                d0 = d1;
            }
        }
    }
    if polygon.is_empty() {
        None
    } else {
        Some(std::array::from_fn(|axis| {
            polygon.iter().map(|p| p[axis]).sum::<f64>() / polygon.len() as f64
        }))
    }
}
