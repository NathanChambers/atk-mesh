use crate::Mesh;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

const SAMPLE_SIZE: usize = 8;
const NODE_BUDGET: usize = 160;

fn pointer(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn bounded(value: &Value, path: &str, remaining: &mut usize, depth: usize) -> Value {
    let count = match value {
        Value::Array(values) => Some(values.len()),
        Value::Object(values) => Some(values.len()),
        Value::String(value) => Some(value.len()),
        _ => None,
    };
    if *remaining == 0 || depth > 6 {
        return json!({"truncated":true,"count":count,"path":path});
    }
    *remaining -= 1;
    match value {
        Value::Array(values) => {
            let mut sample = vec![];
            for (index, value) in values.iter().take(SAMPLE_SIZE).enumerate() {
                if *remaining == 0 {
                    break;
                }
                sample.push(bounded(
                    value,
                    &pointer(path, &index.to_string()),
                    remaining,
                    depth + 1,
                ));
            }
            if sample.len() == values.len() {
                return Value::Array(sample);
            }
            json!({"count":values.len(),"sample":sample,"truncated":true,"path":path})
        }
        Value::Object(values) => {
            let mut sample = Map::new();
            for (key, value) in values.iter().take(SAMPLE_SIZE) {
                if *remaining == 0 {
                    break;
                }
                sample.insert(
                    key.clone(),
                    bounded(value, &pointer(path, key), remaining, depth + 1),
                );
            }
            if sample.len() == values.len() {
                return Value::Object(sample);
            }
            json!({"count":values.len(),"sample":sample,"truncated":true,"path":path})
        }
        Value::String(value) if value.len() > 512 => {
            json!({"bytes":value.len(),"truncated":true,"path":path})
        }
        _ => value.clone(),
    }
}

/// Project a recorded result without changing the underlying mesh or recording.
pub fn compact(mut result: Value) -> Value {
    if let Some(alignment) = result.get_mut("alignment") {
        let count = alignment["landmarks"]
            .as_array()
            .map(|v| v.len())
            .unwrap_or(0);
        *alignment = json!({"reference":alignment["reference"],"alignment_version":alignment["alignment_version"],"landmark_count":count,"bound_landmarks":alignment["bound_landmarks"],"rms_error_pixels":alignment["rms_error_pixels"],"max_error_pixels":alignment["max_error_pixels"],"units":alignment["units"],"query":{"command":"project_reference","reference":alignment["reference"]}});
    }
    let selection_path = if result.get("mesh").is_some() {
        "/mesh/selection"
    } else {
        "/selection"
    };
    if let Some(object) = result.as_object_mut()
        && let Some(mesh) = object.remove("mesh")
    {
        object.remove("adjacency");
        object.remove("edges");
        object.insert("counts".into(), json!({"vertices":mesh["vertices"].as_object().map(|v|v.len()),"faces":mesh["faces"].as_object().map(|v|v.len()),"objects":mesh["objects"].as_object().map(|v|v.len()),"collections":mesh["collections"].as_object().map(|v|v.len())}));
        object.insert("selection".into(), mesh["selection"].clone());
    }
    let mut remaining = NODE_BUDGET;
    if let Some(operation) = result.get_mut("operation").and_then(Value::as_object_mut) {
        for (key, value) in operation {
            if value.is_number()
                || value.is_boolean()
                || value.is_null()
                || value.as_str().is_some_and(|text| text.len() <= 512)
            {
                continue;
            }
            *value = bounded(value, &pointer("/operation", key), &mut remaining, 0);
        }
    }
    // Keep top-level metadata keys; bound the contents independently of field ordering.
    if let Some(object) = result.as_object_mut() {
        for (key, value) in object {
            if matches!(
                key.as_str(),
                "revision"
                    | "base_revision"
                    | "preview_id"
                    | "committed"
                    | "new_defects_complete"
                    | "counts"
                    | "validation"
                    | "history"
                    | "artifacts"
                    | "camera"
                    | "operation"
                    | "topology"
                    | "geometry"
            ) {
                continue;
            }
            let path = if key == "selection" {
                selection_path.to_string()
            } else {
                pointer("", key)
            };
            *value = bounded(value, &path, &mut remaining, 0);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_receipt_survives_a_full_response_budget() {
        let items = vec![json!({"faces":[1,2,3],"location":[0,0,0]}); 100];
        let reply = compact(json!({
            "revision":12,
            "defects_after":{"items":items},
            "defects_before":{"items":items},
            "diff":{"items":items},
            "preview_id":"prepared",
            "committed":false,
            "new_defects_complete":true
        }));
        assert_eq!(reply["preview_id"], "prepared");
        assert_eq!(reply["revision"], 12);
        assert_eq!(reply["new_defects_complete"], true);
    }
}

pub(super) fn page(request: &Value) -> Result<(usize, usize), String> {
    let offset = request.get("offset").map_or(Ok(0), |v| {
        v.as_u64().ok_or("offset must be an unsigned integer")
    })?;
    let limit = request.get("limit").map_or(Ok(32), |v| {
        v.as_u64()
            .filter(|v| (1..=256).contains(v))
            .ok_or("limit must be 1..256")
    })?;
    let offset = usize::try_from(offset).map_err(|_| "offset is too large")?;
    Ok((offset, limit as usize))
}

pub(super) fn slice(value: &Value, request: &Value) -> Result<Value, String> {
    let (offset, limit) = page(request)?;
    let full = request["response"] == "full";
    let path = request["path"].as_str().unwrap_or("");
    let mut remaining = NODE_BUDGET;
    let mut project = |key: &str, value: &Value| {
        if full {
            value.clone()
        } else {
            bounded(value, &pointer(path, key), &mut remaining, 0)
        }
    };
    let (total, items) = match value {
        Value::Array(values) => (
            values.len(),
            Value::Array(
                values
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(limit)
                    .map(|(i, v)| project(&i.to_string(), v))
                    .collect(),
            ),
        ),
        Value::Object(values) => (
            values.len(),
            Value::Object(
                values
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|(k, v)| (k.clone(), project(k, v)))
                    .collect(),
            ),
        ),
        _ => {
            let value = if full {
                value.clone()
            } else {
                bounded(value, path, &mut remaining, 0)
            };
            return Ok(json!({"value":value,"total":1,"offset":0,"next_offset":null}));
        }
    };
    let next = offset.saturating_add(limit);
    Ok(
        json!({"items":items,"total":total,"offset":offset,"next_offset":if next < total {Some(next)} else {None}}),
    )
}

pub(super) fn query_mesh(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let (offset, limit) = page(request)?;
    let members = request
        .get("collection")
        .map(|v| crate::collections::members(mesh, v))
        .transpose()?;
    let scoped_vertices: BTreeSet<u64> = members
        .as_ref()
        .into_iter()
        .flatten()
        .flat_map(|id| mesh.objects[id].vertices.iter().copied())
        .collect();
    let object = request
        .get("object")
        .map(|v| {
            let id = v.as_u64().ok_or("object must be an ID")?;
            mesh.objects.get(&id).ok_or("unknown object")
        })
        .transpose()?;
    let ids: Option<BTreeSet<u64>> = request
        .get("ids")
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|_| "ids must be an array of unsigned integers")
        })
        .transpose()?;
    let kind = request["kind"]
        .as_str()
        .ok_or("kind must be vertices, faces, objects or selection")?;
    let mut records = vec![];
    let mut total = 0;
    let mut found = BTreeSet::new();
    let mut add = |id: u64, value: &dyn Fn() -> Value| {
        if ids.as_ref().is_some_and(|ids| !ids.contains(&id)) {
            return;
        }
        if ids.is_some() {
            found.insert(id);
        }
        if total >= offset && records.len() < limit {
            records.push(value());
        }
        total += 1;
    };
    match kind {
        "vertices" => {
            for (id, point) in &mesh.vertices {
                if object.is_none_or(|o| o.vertices.contains(id))
                    && members
                        .as_ref()
                        .is_none_or(|_| scoped_vertices.contains(id))
                {
                    add(*id, &|| json!({"id":id,"position":point}));
                }
            }
        }
        "faces" => {
            for (id, face) in &mesh.faces {
                if object.is_none_or(|o| o.vertices.contains(&face[0]))
                    && members
                        .as_ref()
                        .is_none_or(|_| scoped_vertices.contains(&face[0]))
                {
                    add(*id, &|| json!({"id":id,"vertices":face}));
                }
            }
        }
        "selection" => {
            for id in &mesh.selection {
                if object.is_none_or(|o| o.vertices.contains(id))
                    && members
                        .as_ref()
                        .is_none_or(|_| scoped_vertices.contains(id))
                {
                    add(*id, &|| json!(id));
                }
            }
        }
        "objects" => {
            if request.get("object").is_some() {
                return Err("use ids to filter object records".into());
            }
            for (id, object) in &mesh.objects {
                if members
                    .as_ref()
                    .is_some_and(|members| !members.contains(id))
                {
                    continue;
                }
                add(
                    *id,
                    &|| {
                        let instances: Vec<_> = mesh.objects.iter().filter_map(|(instance_id, instance)| {
                            let link = instance.instance.as_ref()?;
                            (link.master == *id).then(|| json!({"id":instance_id,"name":instance.name}))
                        }).collect();
                        json!({"id":id,"name":object.name,"origin":object.origin,"vertex_count":object.vertices.len(),"stage":object.stage,"stage_note":object.stage_note,"smooth_angle":object.smooth_angle,"master_object":object.instance.as_ref().map(|link| link.master),"instance_count":instances.len(),"instances":instances,"primary_collection":mesh.primary_collections.get(id),"collections":mesh.collections.iter().filter(|(_,c)| c.objects.contains(id)).map(|(id,_)| *id).collect::<Vec<_>>()})
                    },
                );
            }
        }
        "collections" => {
            for (id, collection) in &mesh.collections {
                add(
                    *id,
                    &|| json!({"id":id,"name":collection.name,"object_count":collection.objects.len(),"query":{"command":"query_mesh","kind":"objects","collection":id}}),
                );
            }
        }
        "components" => return Err("use query_components to query named components".into()),
        _ => return Err("kind must be vertices, faces, objects, collections or selection".into()),
    }
    if ids.is_some_and(|ids| ids != found) {
        return Err("unknown or out-of-scope query IDs".into());
    }
    let next = offset.saturating_add(limit);
    Ok(
        json!({"revision":revision,"kind":kind,"items":records,"total":total,"offset":offset,"next_offset":if next < total {Some(next)} else {None}}),
    )
}
