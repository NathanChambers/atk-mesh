use crate::{Mesh, instances, objects};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn edit(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let axis = match request["axis"].as_str() {
        Some("X") => 0,
        Some("Y") => 1,
        Some("Z") => 2,
        _ => return Err("axis must be X, Y or Z".into()),
    };
    let [min, max]: [f64; 2] = serde_json::from_value(request["span"].clone())
        .map_err(|_| "span must contain two coordinates")?;
    let delta = request["delta"]
        .as_f64()
        .ok_or("delta must be a signed length in mesh units")?;
    let width = max - min;
    if !width.is_finite() || width <= 0.0 || width + delta <= 0.0 {
        return Err("span must increase and its new width must stay positive".into());
    }
    let groups: Vec<Vec<u64>> = request
        .get("rigid_groups")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|_| "rigid_groups must contain arrays of object IDs")?
        .unwrap_or_default();
    let bodies: Vec<u64> = if let Some(id) = request.get("object") {
        if request.get("objects").is_some() || !groups.is_empty() {
            return Err("use objects when stretching a multi-part assembly".into());
        }
        vec![id.as_u64().ok_or("object must be an ID")?]
    } else {
        serde_json::from_value(request["objects"].clone())
            .map_err(|_| "objects must list the bodies to stretch")?
    };
    if bodies.is_empty() || groups.iter().any(Vec::is_empty) {
        return Err("supply at least one body and nonempty rigid groups".into());
    }
    let mut seen = BTreeSet::new();
    for id in bodies.iter().chain(groups.iter().flatten()) {
        if !mesh.objects.contains_key(id) || !seen.insert(*id) {
            return Err(format!("object {id} is unknown or appears more than once"));
        }
    }
    let offset = |v: f64| delta * ((v - min) / width).clamp(0.0, 1.0) - 0.5 * delta;
    for id in &bodies {
        let object = mesh.objects.get_mut(id).unwrap();
        if object.instance.is_some() {
            return Err("stretch the master or make a linked body unique; linked instances can move in rigid_groups".into());
        }
        for vertex in &object.vertices {
            let p = &mut mesh.vertices.get_mut(vertex).unwrap()[axis];
            *p += offset(*p);
        }
        object.origin[axis] += offset(object.origin[axis]);
    }
    let mut placements = Vec::new();
    for group in groups {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for vertex in group.iter().flat_map(|id| &mesh.objects[id].vertices) {
            let p = mesh.vertices[vertex][axis];
            low = low.min(p);
            high = high.max(p);
        }
        if !low.is_finite() {
            return Err("rigid group has no geometry".into());
        }
        let mut translation = [0.0; 3];
        translation[axis] = offset(low * 0.5 + high * 0.5);
        for id in &group {
            let movement = json!({"object":id,"value":translation});
            if mesh.objects[id].instance.is_some() {
                // Linked geometry is evaluated once, from its master, when this edit finishes.
                instances::placement(mesh, *id, "object_move", &movement)?;
                mesh.objects.get_mut(id).unwrap().origin[axis] += translation[axis];
            } else {
                objects::edit(mesh, "object_move", &movement)?;
            }
        }
        placements.push(json!({"objects":group,"translation":translation}));
    }
    Ok(
        json!({"objects":bodies,"axis":request["axis"],"old_span":width,"new_span":width+delta,"rigid_groups":placements}),
    )
}
