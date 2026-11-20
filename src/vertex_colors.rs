use crate::Mesh;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    let object = request
        .get("object")
        .map(|id| {
            let id = id.as_u64().ok_or("object must be an ID")?;
            mesh.objects.get(&id).ok_or("unknown object")
        })
        .transpose()?;
    let allowed = |id: u64| {
        mesh.vertices.contains_key(&id) && object.is_none_or(|object| object.vertices.contains(&id))
    };
    if command == "vertex_color_clear" {
        let vertices: Vec<u64> = request
            .get("vertices")
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()
            .map_err(|_| "vertices must be an array of IDs")?
            .unwrap_or_else(|| {
                mesh.vertices
                    .keys()
                    .filter(|id| allowed(**id))
                    .copied()
                    .collect()
            });
        if vertices.iter().any(|id| !allowed(*id)) {
            return Err("vertex is unknown or outside the requested object".into());
        }
        let mut changed = 0;
        for vertex in vertices {
            changed += usize::from(mesh.vertex_colors.remove(&vertex).is_some());
        }
        return Ok(json!({"cleared_vertices":changed}));
    }
    let values = if let Some(values) = request.get("values") {
        if request.get("vertices").is_some() || request.get("color").is_some() {
            return Err("use values, or vertices with one color, not both".into());
        }
        values
            .as_array()
            .ok_or("values must contain {vertex,color} entries")?
            .clone()
    } else {
        let vertices: Vec<u64> = serde_json::from_value(request["vertices"].clone())
            .map_err(|_| "vertices must be an array of IDs")?;
        vertices
            .into_iter()
            .map(|vertex| json!({"vertex":vertex,"color":request["color"]}))
            .collect()
    };
    if values.is_empty() || values.len() > 4096 {
        return Err("assign 1..4096 vertices per command".into());
    }
    let mut assigned = Vec::new();
    let mut ids = BTreeSet::new();
    for value in values {
        let vertex = value["vertex"].as_u64().ok_or("vertex must be an ID")?;
        let distinct = ids.insert(vertex);
        if !allowed(vertex) || !distinct {
            return Err(
                "vertices must be distinct, existing and inside the requested object".into(),
            );
        }
        let color: [f64; 3] = serde_json::from_value(value["color"].clone())
            .map_err(|_| "color must contain three normalized RGB numbers")?;
        let invalid = color
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value));
        if invalid {
            return Err("color must contain finite RGB values in 0..1".into());
        }
        assigned.push((vertex, color));
    }
    for (vertex, color) in &assigned {
        mesh.vertex_colors.insert(*vertex, *color);
    }
    Ok(json!({"painted_vertices":assigned.len()}))
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let object = request
        .get("object")
        .map(|id| {
            let id = id.as_u64().ok_or("object must be an ID")?;
            mesh.objects.get(&id).ok_or("unknown object")
        })
        .transpose()?;
    let ids: Option<BTreeSet<u64>> = request
        .get("ids")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| "ids must be an array of IDs")?;
    if let Some(ids) = &ids {
        let invalid = ids.iter().any(|id| {
            !mesh.vertices.contains_key(id)
                || object.is_some_and(|object| !object.vertices.contains(id))
        });
        if invalid {
            return Err("vertex is unknown or outside the requested object".into());
        }
    }
    let records: Vec<_> = mesh
        .vertices
        .keys()
        .filter(|id| {
            ids.as_ref().is_none_or(|ids| ids.contains(id))
                && object.is_none_or(|object| object.vertices.contains(id))
        })
        .collect();
    let (offset, limit) = crate::responses::page(request)?;
    let items: Vec<_> = records
        .iter()
        .skip(offset)
        .take(limit)
        .map(|id| json!({"vertex":id,"color":mesh.vertex_colors.get(id)}))
        .collect();
    let next = offset.saturating_add(limit);
    Ok(
        json!({"revision":revision,"items":items,"total":records.len(),"offset":offset,"next_offset":if next < records.len() {Some(next)} else {None}}),
    )
}

pub(super) fn validate(mesh: &Mesh) -> Result<(), String> {
    let invalid = mesh.vertex_colors.iter().any(|(id, color)| {
        !mesh.vertices.contains_key(id)
            || color
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    });
    if invalid {
        return Err("invalid vertex color or vertex binding".into());
    }
    Ok(())
}
