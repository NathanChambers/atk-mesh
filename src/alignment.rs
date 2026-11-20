use crate::{
    Mesh,
    projection::{Camera, dot},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) struct Context {
    pub resource: Value,
    pub bytes: Vec<u8>,
    pub view: Value,
    pub camera: Camera,
    pub frame: crate::comparison::Frame,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Landmark {
    pub name: String,
    pub pixel: [f64; 2],
    #[serde(default)]
    pub vertices: Vec<u64>,
    #[serde(default)]
    pub object: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Contour {
    pub landmarks: Vec<String>,
    #[serde(default)]
    pub closed: bool,
}

pub(crate) fn landmarks(value: &Value) -> Result<Vec<Landmark>, String> {
    serde_json::from_value(value.get("landmarks").cloned().unwrap_or_else(|| json!([])))
        .map_err(|e| format!("invalid landmarks: {e}"))
}

pub(crate) fn contours(value: &Value) -> Result<Vec<Contour>, String> {
    serde_json::from_value(value.get("contours").cloned().unwrap_or_else(|| json!([])))
        .map_err(|e| format!("invalid contours: {e}"))
}

pub(crate) fn vertices(mesh: &Mesh, landmark: &Landmark) -> Result<Vec<u64>, String> {
    let object = landmark
        .object
        .map(|id| {
            mesh.objects
                .get(&id)
                .ok_or_else(|| format!("landmark {} references a missing object", landmark.name))
        })
        .transpose()?;
    let vertices: Vec<_> = if landmark.vertices.is_empty() {
        object
            .map(|object| object.vertices.iter().copied().collect())
            .unwrap_or_default()
    } else {
        landmark.vertices.clone()
    };
    for id in &vertices {
        if !mesh.vertices.contains_key(id) {
            return Err(format!(
                "landmark {} references removed vertex {id}; rebind the landmark",
                landmark.name
            ));
        }
        if object.is_some_and(|object| !object.vertices.contains(id)) {
            return Err(format!(
                "landmark {} crosses its object scope",
                landmark.name
            ));
        }
    }
    Ok(vertices)
}

pub(crate) fn anchor(mesh: &Mesh, landmark: &Landmark) -> Result<Option<[f64; 3]>, String> {
    let vertices = vertices(mesh, landmark)?;
    if vertices.is_empty() {
        return Ok(None);
    }
    let point = std::array::from_fn(|axis| {
        vertices
            .iter()
            .map(|id| mesh.vertices[id][axis] / vertices.len() as f64)
            .sum()
    });
    Ok(Some(point))
}

pub(crate) fn validate(request: &Value, mesh: &Mesh, size: [usize; 2]) -> Result<(), String> {
    let landmarks = landmarks(request)?;
    if landmarks.is_empty() || landmarks.len() > 128 {
        return Err("landmarks requires 1..128 named points".into());
    }
    let mut names = BTreeSet::new();
    for landmark in &landmarks {
        let invalid_name = landmark.name.trim().is_empty()
            || landmark.name.len() > 64
            || !names.insert(landmark.name.clone());
        if invalid_name {
            return Err("landmark names must be unique nonempty text, at most 64 bytes".into());
        }
        let outside =
            landmark.pixel.iter().enumerate().any(|(axis, pixel)| {
                !pixel.is_finite() || *pixel < 0.0 || *pixel >= size[axis] as f64
            });
        if outside {
            return Err(format!(
                "landmark {} is outside the original reference image",
                landmark.name
            ));
        }
        let unique: BTreeSet<_> = landmark.vertices.iter().collect();
        if unique.len() != landmark.vertices.len() || unique.len() > 256 {
            return Err(
                "landmark vertices must be unique, at most 256; use object for an entire object"
                    .into(),
            );
        }
        vertices(mesh, landmark)?;
    }
    let contours = contours(request)?;
    if contours.len() > 32 {
        return Err("at most 32 contours are supported".into());
    }
    for contour in contours {
        let invalid = contour.landmarks.len() < 2
            || contour.landmarks.len() > 128
            || contour.landmarks.iter().any(|name| !names.contains(name));
        if invalid {
            return Err("contours require 2..128 existing landmark names".into());
        }
    }
    Ok(())
}

pub(crate) fn project(
    mesh: &Mesh,
    resource: &Value,
    camera: &Camera,
    frame: &crate::comparison::Frame,
    revision: u64,
) -> Result<Value, String> {
    let mut entries = vec![];
    let mut squared_error = 0.0;
    let mut maximum: f64 = 0.0;
    let mut bound = 0;
    for landmark in landmarks(resource)? {
        let target = frame.to_canvas(landmark.pixel);
        let mut entry = json!({"name":landmark.name,"target_pixel":landmark.pixel,"target_canvas":target,"object":landmark.object});
        if let Some(anchor) = anchor(mesh, &landmark)? {
            if !camera.contains(anchor) {
                entry["projectable"] = json!(false);
                entry["reason"] = json!("behind perspective near plane; excluded from residuals");
                entries.push(entry);
                continue;
            }
            let point = camera.project(anchor)?;
            let projected = frame.to_image([point[0], point[1]]);
            let delta = [
                landmark.pixel[0] - projected[0],
                landmark.pixel[1] - projected[1],
            ];
            let error = delta[0].hypot(delta[1]);
            squared_error += error * error;
            maximum = maximum.max(error);
            bound += 1;
            entry["projected_pixel"] = json!(projected);
            entry["projected_canvas"] = json!([point[0], point[1]]);
            entry["delta_pixels"] = json!(delta);
            entry["error_pixels"] = json!(error);
            entry["depth"] = json!(point[2]);
            entry["vertex_count"] = json!(vertices(mesh, &landmark)?.len());
        }
        entries.push(entry);
    }
    if !squared_error.is_finite() {
        return Err("alignment exceeds numeric range".into());
    }
    let rms = if bound == 0 {
        Value::Null
    } else {
        json!((squared_error / bound as f64).sqrt())
    };
    Ok(
        json!({"reference":resource["id"],"alignment_version":resource["alignment_version"].as_u64().unwrap_or(0),"revision":revision,"camera":camera.metadata(),"crop":frame.crop,"landmarks":entries,"bound_landmarks":bound,"rms_error_pixels":rms,"max_error_pixels":if bound==0 {Value::Null} else {json!(maximum)},"units":"original reference-image pixels; projected anchors are not visibility-tested","move_template":{"command":"move_in_reference","reference":resource["id"],"landmark":"CHOOSE a bound landmark name","revision":revision,"alignment_version":resource["alignment_version"].as_u64().unwrap_or(0)}}),
    )
}

pub(crate) fn fit(
    mesh: &Mesh,
    resource: &Value,
    request: &Value,
    camera: &Camera,
    frame: &crate::comparison::Frame,
) -> Result<Value, String> {
    let names: Option<Vec<String>> = request
        .get("names")
        .map(|names| {
            serde_json::from_value(names.clone()).map_err(|_| "names must be landmark names")
        })
        .transpose()?;
    let landmarks = landmarks(resource)?;
    if names.as_ref().is_some_and(|names| {
        names
            .iter()
            .any(|name| !landmarks.iter().any(|landmark| &landmark.name == name))
    }) {
        return Err("unknown fit landmark name".into());
    }
    if camera.perspective() {
        let mut anchors = vec![];
        for landmark in &landmarks {
            if names
                .as_ref()
                .is_some_and(|names| !names.contains(&landmark.name))
            {
                continue;
            }
            if let Some(anchor) = anchor(mesh, landmark)? {
                anchors.push((anchor, frame.to_canvas(landmark.pixel)));
            }
        }
        return crate::camera_fit::fit(camera, &anchors, request);
    }
    let mut pairs = vec![];
    for landmark in landmarks {
        if names
            .as_ref()
            .is_some_and(|names| !names.contains(&landmark.name))
        {
            continue;
        }
        if let Some(anchor) = anchor(mesh, &landmark)? {
            pairs.push((
                [dot(anchor, camera.right), -dot(anchor, camera.up)],
                frame.to_canvas(landmark.pixel),
            ));
        }
    }
    if pairs.len() < 2 {
        return Err("camera fitting requires at least two bound landmarks".into());
    }
    let mean = |index: usize| -> [f64; 2] {
        std::array::from_fn(|axis| {
            pairs
                .iter()
                .map(|pair| {
                    if index == 0 {
                        pair.0[axis]
                    } else {
                        pair.1[axis]
                    }
                })
                .sum::<f64>()
                / pairs.len() as f64
        })
    };
    let source = mean(0);
    let target = mean(1);
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (point, pixel) in &pairs {
        for axis in 0..2 {
            numerator += (point[axis] - source[axis]) * (pixel[axis] - target[axis]);
            denominator += (point[axis] - source[axis]).powi(2);
        }
    }
    let scale = numerator / denominator;
    let invalid = denominator <= 1e-24 || !scale.is_finite() || scale <= 0.0;
    if invalid {
        return Err(
            "degenerate or reversed fit; choose separated anchors and check yaw/pitch".into(),
        );
    }
    let translation = [target[0] - scale * source[0], target[1] - scale * source[1]];
    let right = (camera.width as f64 / 2.0 - translation[0]) / scale;
    let up = (translation[1] - camera.height as f64 / 2.0) / scale;
    let depth = dot(camera.center, camera.forward);
    let center: [f64; 3] = std::array::from_fn(|axis| {
        camera.right[axis] * right + camera.up[axis] * up + camera.forward[axis] * depth
    });
    let resolution_scale = (camera.width as f64 / 800.0).min(camera.height as f64 / 600.0);
    let mut fitted = camera.metadata();
    fitted["center"] = json!(center);
    fitted["zoom"] = json!(scale / resolution_scale);
    let camera = Camera::parse(&fitted)?;
    Ok(camera.metadata())
}
