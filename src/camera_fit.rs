use crate::projection::Camera;
use serde_json::{Value, json};

fn solve(mut matrix: [[f64; 6]; 6], mut values: [f64; 6]) -> Option<[f64; 6]> {
    for column in 0..6 {
        let pivot = (column..6).max_by(|a, b| {
            matrix[*a][column]
                .abs()
                .total_cmp(&matrix[*b][column].abs())
        })?;
        if matrix[pivot][column].abs() < 1e-18 {
            return None;
        }
        matrix.swap(column, pivot);
        values.swap(column, pivot);
        let divisor = matrix[column][column];
        for entry in &mut matrix[column][column..] {
            *entry /= divisor;
        }
        values[column] /= divisor;
        for row in 0..6 {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            let pivot_values = matrix[column];
            for (entry, pivot) in matrix[row][column..]
                .iter_mut()
                .zip(&pivot_values[column..])
            {
                *entry -= factor * pivot;
            }
            values[row] -= factor * values[column];
        }
    }
    Some(values)
}

fn camera(base: &Value, parameters: [f64; 6]) -> Result<Camera, String> {
    let mut options = base.clone();
    options["position"] = json!(&parameters[..3]);
    options["yaw"] = json!(parameters[3]);
    options["pitch"] = json!(parameters[4]);
    options["fov"] = json!(parameters[5]);
    Camera::parse(&options)
}

fn residuals(
    base: &Value,
    parameters: [f64; 6],
    anchors: &[([f64; 3], [f64; 2])],
) -> Result<Vec<f64>, String> {
    let camera = camera(base, parameters)?;
    let mut values = vec![];
    for (point, target) in anchors {
        let projected = camera.project(*point)?;
        values.extend([projected[0] - target[0], projected[1] - target[1]]);
    }
    Ok(values)
}

pub(crate) fn fit(
    initial: &Camera,
    anchors: &[([f64; 3], [f64; 2])],
    request: &Value,
) -> Result<Value, String> {
    if anchors.len() < 6 {
        return Err("perspective fitting requires at least six bound landmarks spread across the image and scene depth".into());
    }
    let base = initial.metadata();
    let parameter_names = [
        "position_x",
        "position_y",
        "position_z",
        "yaw",
        "pitch",
        "fov",
    ];
    let selected: Vec<String> = request
        .get("fit_parameters")
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|_| "fit_parameters must contain parameter names")
        })
        .transpose()?
        .unwrap_or_else(|| {
            parameter_names
                .iter()
                .map(|name| name.to_string())
                .collect()
        });
    let unique: std::collections::BTreeSet<_> = selected.iter().collect();
    if selected.is_empty()
        || unique.len() != selected.len()
        || selected
            .iter()
            .any(|name| !parameter_names.contains(&name.as_str()))
    {
        return Err("fit_parameters must be distinct names from position_x,position_y,position_z,yaw,pitch,fov".into());
    }
    let mut bounds = [[f64::NEG_INFINITY, f64::INFINITY]; 6];
    bounds[4] = [-88.99, 88.99];
    bounds[5] = [5.01, 149.99];
    if let Some(value) = request.get("bounds") {
        for (name, value) in value
            .as_object()
            .ok_or("bounds must map parameter names to [min,max]")?
        {
            let index = parameter_names
                .iter()
                .position(|known| known == name)
                .ok_or("unknown bound parameter")?;
            let range = crate::projection::pair(value)?;
            if range[0] >= range[1] {
                return Err("fit bounds must be increasing".into());
            }
            bounds[index] = [
                range[0].max(bounds[index][0]),
                range[1].min(bounds[index][1]),
            ];
        }
    }
    let mut parameters = [
        initial.center[0],
        initial.center[1],
        initial.center[2],
        base["yaw"].as_f64().unwrap(),
        base["pitch"].as_f64().unwrap(),
        base["fov"].as_f64().unwrap(),
    ];
    if parameters
        .iter()
        .enumerate()
        .any(|(index, value)| *value < bounds[index][0] || *value > bounds[index][1])
    {
        return Err("initial camera must lie within fit bounds".into());
    }
    let extent = (0..3)
        .map(|axis| {
            anchors
                .iter()
                .map(|(p, _)| p[axis])
                .fold(f64::NEG_INFINITY, f64::max)
                - anchors
                    .iter()
                    .map(|(p, _)| p[axis])
                    .fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max);
    if extent < 1e-8 {
        return Err("degenerate fit: anchors must occupy distinct positions".into());
    }
    let mut values = residuals(&base, parameters, anchors)?;
    let mut cost = values.iter().map(|value| value * value).sum::<f64>();
    let mut damping = 0.001;
    let mut iterations = 0;
    for iteration in 0..100 {
        iterations = iteration + 1;
        let mut derivatives = vec![[0.0; 6]; values.len()];
        for column in 0..6 {
            if !selected.iter().any(|name| name == parameter_names[column]) {
                continue;
            }
            let step = if column < 3 { extent * 1e-6 } else { 1e-4 };
            let mut shifted = parameters;
            shifted[column] += step;
            let sampled = residuals(&base, shifted, anchors)?;
            for row in 0..values.len() {
                derivatives[row][column] = (sampled[row] - values[row]) / step;
            }
        }
        let mut normal = [[0.0; 6]; 6];
        let mut gradient = [0.0; 6];
        for (row, derivative) in derivatives.iter().enumerate() {
            for a in 0..6 {
                gradient[a] -= derivative[a] * values[row];
                for b in 0..6 {
                    normal[a][b] += derivative[a] * derivative[b];
                }
            }
        }
        for (axis, row) in normal.iter_mut().enumerate() {
            if !selected.iter().any(|name| name == parameter_names[axis]) {
                row[axis] = 1.0;
                continue;
            }
            row[axis] += damping * row[axis].max(1e-10);
        }
        let delta = solve(normal, gradient)
            .ok_or("degenerate perspective fit; choose anchors spread across image and depth")?;
        let candidate = std::array::from_fn(|axis| {
            (parameters[axis] + delta[axis]).clamp(bounds[axis][0], bounds[axis][1])
        });
        let sampled = if candidate[4].abs() < 89.0 && (5.01..149.99).contains(&candidate[5]) {
            residuals(&base, candidate, anchors).ok()
        } else {
            None
        };
        if let Some(sampled) = sampled {
            let next_cost = sampled.iter().map(|value| value * value).sum::<f64>();
            if next_cost < cost {
                let improvement = cost - next_cost;
                parameters = candidate;
                values = sampled;
                cost = next_cost;
                damping = (damping * 0.3).max(1e-12);
                if improvement < 1e-10 || cost < 1e-16 {
                    break;
                }
                continue;
            }
        }
        damping *= 10.0;
        if damping > 1e12 {
            break;
        }
    }
    let mut result = camera(&base, parameters)?.metadata();
    result["fit_diagnostics"] = json!({"iterations":iterations,"anchor_count":anchors.len(),"rms_canvas_pixels":(cost/anchors.len() as f64).sqrt(),"parameters":selected,"bounds":request["bounds"],"limitation":"local least-squares fit with zero roll and fixed lens shift; manually interpreted anchors and geometry may be inconsistent"});
    Ok(result)
}
