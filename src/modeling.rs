use crate::{Mesh, construction, geometry::*};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
pub struct Construction {
    pub parameters: BTreeMap<String, f64>,
    pub steps: Vec<Value>,
}

pub(super) fn validate(mesh: &Mesh) -> Result<(), String> {
    if mesh.constructions.len() > 64 {
        return Err("construction budget exceeded".into());
    }
    for (name, plan) in &mesh.constructions {
        if name.trim().is_empty()
            || plan.steps.is_empty()
            || plan.steps.len() > 64
            || plan.parameters.len() > 256
            || plan.parameters.values().any(|v| !v.is_finite())
        {
            return Err("invalid saved construction".into());
        }
        let mut names = BTreeSet::new();
        for step in &plan.steps {
            if !construction::generator(step["command"].as_str().unwrap_or(""))
                || step["object"].as_u64().is_none()
                || step["mode"] != "append"
                || step["region"]
                    .as_str()
                    .is_none_or(|name| !names.insert(name))
            {
                return Err("invalid saved construction step".into());
            }
            resolve(step, &plan.parameters)?;
        }
    }
    Ok(())
}

pub(super) fn resolve_request(mesh: &Mesh, mut request: Value) -> Result<Value, String> {
    let command = request["command"].as_str().unwrap_or("");
    if construction::generator(command)
        || command.starts_with("region_")
        || command == "select_region"
        || matches!(
            command,
            "inspect_defects" | "query_components" | "measure_clearance" | "inspect_volume" | "regenerate_component"
        )
    {
        return Ok(request);
    }
    let uv_projection = command == "uv_project";
    let selector = request.get("mesh_region").or_else(|| {
        request
            .get("region")
            .filter(|value| !uv_projection || value.is_string())
    });
    if let Some(value) = selector {
        let name = value.as_str().ok_or("region must be a name")?;
        let id = construction::named(mesh, name)?;
        if !matches!(
            command,
            "delete_faces"
                | "dissolve_region"
                | "shade_region"
                | "inspect_region"
                | "retriangulate_region"
                | "inset_region"
                | "inspect_patch"
                | "replace_patch"
                | "uv_set"
                | "uv_clear"
                | "uv_transform"
                | "uv_project"
                | "uv_unwrap"
                | "uv_pack"
                | "query_uv"
                | "inspect_uv"
                | "capture_uv"
                | "texture_assign"
                | "texture_layer_mask_faces"
                | "texture_mask_surface"
                | "query_texture_faces"
                | "select_related"
        ) {
            return Err(
                "this command does not accept a region; use select_region before selection edits"
                    .into(),
            );
        }
        if request.get("faces").is_some() || request.get("vertices").is_some() {
            return Err("region cannot be combined with faces or vertices".into());
        }
        let faces = &mesh.components[&id].faces;
        let selected = construction::vertices(mesh, faces);
        if let Some(value) = request.get("object") {
            let id = value.as_u64().ok_or("object must be an ID")?;
            let object = mesh.objects.get(&id).ok_or("unknown object")?;
            if !selected.is_subset(&object.vertices) {
                return Err("region lies outside requested object".into());
            }
        }
        request["faces"] = json!(faces);
        if uv_projection && request["region"].is_string() {
            request.as_object_mut().unwrap().remove("region");
        }
    }
    Ok(request)
}

fn name(request: &Value) -> Result<String, String> {
    let name = request["name"]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .ok_or("name must contain 1..256 printable characters")?;
    Ok(name.to_string())
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    match command {
        "regenerate_component" => {
            let region = request["region"].as_str().ok_or("region must name a generated component")?;
            let id = construction::named(mesh, region)?;
            let mut recipe = mesh.components[&id].recipe.clone().ok_or("region has no saved generator recipe")?;
            if let Some(overrides) = request.get("overrides") {
                for (key, value) in overrides.as_object().ok_or("overrides must be an object")? {
                    if matches!(key.as_str(), "command" | "object" | "region" | "name" | "mode") {
                        return Err(format!("cannot override generator identity field {key}"));
                    }
                    recipe[key] = value.clone();
                }
            }
            let command = match recipe["command"].as_str().ok_or("invalid generator recipe")? {
                "object_create_box" => "object_add_box".to_string(),
                command => command.to_string(),
            };
            recipe["command"] = json!(command);
            recipe["region"] = json!(region);
            recipe["object"] = json!(request["object"].as_u64().ok_or("object must identify the component owner")?);
            recipe["mode"] = json!("replace_region");
            construction::generate(mesh, &command, &recipe)
        }
        "region_define" => {
            let name = name(request)?;
            let faces = crate::diagnostics::faces(mesh, request)?;
            if faces.is_empty() || request.get("faces").is_none() {
                return Err("region_define requires explicit nonempty faces".into());
            }
            let explicit: BTreeSet<u64> = serde_json::from_value(request["faces"].clone())
                .map_err(|_| "faces must be IDs")?;
            if faces != explicit {
                return Err("region faces lie outside the requested object".into());
            }
            if construction::named(mesh, &name).is_ok() {
                return Err(
                    "region already exists; remove its label explicitly before redefining".into(),
                );
            }
            mesh.component_id_high_watermark = mesh
                .component_id_high_watermark
                .max(*mesh.components.keys().next_back().unwrap_or(&0));
            let id = mesh
                .component_id_high_watermark
                .checked_add(1)
                .ok_or("component IDs exhausted")?;
            mesh.component_id_high_watermark = id;
            mesh.components.insert(
                id,
                construction::Component {
                    name: name.clone(),
                    faces: faces.clone(),
                    recipe: None,
                },
            );
            Ok(json!({"component_id":id,"region":name,"faces":faces}))
        }
        "region_remove" => {
            let name = name(request)?;
            let id = construction::named(mesh, &name)?;
            mesh.components.remove(&id);
            Ok(json!({"component_id":id,"region":name,"geometry_changed":false}))
        }
        "select_region" => {
            let name = request["region"].as_str().ok_or("region is required")?;
            let id = construction::named(mesh, name)?;
            let faces = mesh.components[&id].faces.clone();
            let selected = construction::vertices(mesh, &faces);
            if let Some(object) = request.get("object") {
                let object = mesh
                    .objects
                    .get(&object.as_u64().ok_or("object must be an ID")?)
                    .ok_or("unknown object")?;
                if !selected.is_subset(&object.vertices) {
                    return Err("region lies outside requested object".into());
                }
            }
            mesh.selection = selected;
            Ok(json!({"component_id":id,"region":name,"faces":faces,"vertices":mesh.selection}))
        }
        "select_related" => related(mesh, request),
        "construction_define" => {
            let name = name(request)?;
            let parameters: BTreeMap<String, f64> =
                serde_json::from_value(request["parameters"].clone())
                    .map_err(|_| "parameters must map names to finite numbers")?;
            let steps: Vec<Value> = serde_json::from_value(request["steps"].clone())
                .map_err(|_| "steps must be generator commands")?;
            if parameters.values().any(|v| !v.is_finite())
                || steps.is_empty()
                || steps.len() > 64
                || mesh.constructions.len() >= 64
            {
                return Err("construction requires finite parameters and 1..64 steps within 64 construction budget".into());
            }
            let mut names = BTreeSet::new();
            for step in &steps {
                if !construction::generator(step["command"].as_str().unwrap_or(""))
                    || step["object"].as_u64().is_none()
                    || step["mode"] != "append"
                    || step["region"]
                        .as_str()
                        .is_none_or(|name| !names.insert(name))
                {
                    return Err("each construction step requires generator, object, mode:append and a distinct region name; apply replaces those regions on subsequent runs".into());
                }
                resolve(step, &parameters)?;
            }
            if mesh.constructions.contains_key(&name) {
                return Err("construction name already exists".into());
            }
            mesh.constructions
                .insert(name.clone(), Construction { parameters, steps });
            Ok(
                json!({"name":name,"update_policy":"append initially, replace named regions on subsequent apply; all steps atomic"}),
            )
        }
        "construction_apply" => {
            let name = name(request)?;
            let mut construction = mesh
                .constructions
                .get(&name)
                .ok_or("unknown construction")?
                .clone();
            let changes: BTreeMap<String, f64> = request
                .get("parameters")
                .map(|v| {
                    serde_json::from_value(v.clone())
                        .map_err(|_| "parameters must be finite numeric overrides")
                })
                .transpose()?
                .unwrap_or_default();
            for (key, value) in changes {
                if !value.is_finite() || !construction.parameters.contains_key(&key) {
                    return Err("unknown or nonfinite construction parameter".into());
                }
                construction.parameters.insert(key, value);
            }
            let mut operations = vec![];
            for step in &construction.steps {
                let mut resolved = resolve(step, &construction.parameters)?;
                let region = resolved["region"]
                    .as_str()
                    .ok_or("step region is required")?;
                resolved["mode"] = json!(if construction::named(mesh, region).is_ok() {
                    "replace_region"
                } else {
                    "append"
                });
                let command = resolved["command"].as_str().unwrap().to_string();
                operations.push(construction::generate(mesh, &command, &resolved)?);
            }
            let parameters = construction.parameters.clone();
            mesh.constructions.insert(name.clone(), construction);
            Ok(
                json!({"name":name,"parameters":parameters,"operations":operations,"policy":"all dependent steps regenerated in recorded order; manual edits within replaced regions are replaced"}),
            )
        }
        _ => Err("unknown modeling command".into()),
    }
}

fn resolve(value: &Value, parameters: &BTreeMap<String, f64>) -> Result<Value, String> {
    if let Some(name) = value.get("parameter") {
        if value
            .as_object()
            .unwrap()
            .keys()
            .any(|key| !matches!(key.as_str(), "parameter" | "scale" | "offset"))
        {
            return Err("parameter descriptors only accept parameter, scale and offset".into());
        }
        let name = name.as_str().ok_or("parameter must be a name")?;
        let parameter = *parameters.get(name).ok_or("unknown parameter reference")?;
        let scale = value.get("scale").map_or(Ok(1.0), |v| {
            v.as_f64().ok_or("parameter scale must be numeric")
        })?;
        let offset = value.get("offset").map_or(Ok(0.0), |v| {
            v.as_f64().ok_or("parameter offset must be numeric")
        })?;
        let result = parameter * scale + offset;
        if !result.is_finite() {
            return Err("parameter expression is nonfinite".into());
        }
        return Ok(json!(result));
    }
    match value {
        Value::Array(values) => Ok(json!(
            values
                .iter()
                .map(|v| resolve(v, parameters))
                .collect::<Result<Vec<_>, _>>()?
        )),
        Value::Object(values) => {
            let mut result = serde_json::Map::new();
            for (key, value) in values {
                result.insert(key.clone(), resolve(value, parameters)?);
            }
            Ok(Value::Object(result))
        }
        _ => Ok(value.clone()),
    }
}

fn related(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let object = crate::surface_ops::object(mesh, request)?;
    let kind = request["kind"].as_str().ok_or("kind is required")?;
    let mut faces: BTreeSet<u64> = request
        .get("faces")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "faces must be IDs"))
        .transpose()?
        .unwrap_or_default();
    if faces.iter().any(|id| {
        mesh.faces
            .get(id)
            .is_none_or(|f| !mesh.objects[&object].vertices.contains(&f[0]))
    }) {
        return Err("face outside requested object".into());
    }
    let mut selected: BTreeSet<u64> = request
        .get("vertices")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "vertices must be IDs"))
        .transpose()?
        .unwrap_or_else(|| construction::vertices(mesh, &faces));
    if !selected.is_subset(&mesh.objects[&object].vertices) {
        return Err("vertex outside requested object".into());
    }
    let mut pairs = vec![];
    match kind {
        "component" => {
            if selected.is_empty() {
                return Err("component selection requires seed vertices or faces".into());
            }
            for group in crate::diagnostics::components(mesh, object) {
                if !selected.is_disjoint(&group) {
                    selected.extend(group);
                }
            }
            faces = mesh
                .faces
                .iter()
                .filter(|(_, f)| selected.contains(&f[0]))
                .map(|(id, _)| *id)
                .collect();
        }
        "boundary_loop" => {
            let (loops, branching) = crate::surface_ops::boundaries(mesh, Some(object));
            if !branching.is_empty() {
                return Err("boundary branches; repair or select explicit vertices".into());
            }
            let index = request["index"]
                .as_u64()
                .ok_or("boundary index is required")? as usize;
            selected = loops
                .get(index)
                .ok_or("unknown boundary index")?
                .iter()
                .copied()
                .collect();
            faces.clear();
        }
        "adjacent_faces" => {
            if faces.is_empty() {
                return Err("adjacent selection requires faces".into());
            }
            let rings = request.get("rings").map_or(Ok(1), |v| {
                v.as_u64()
                    .filter(|v| *v > 0 && *v <= 64)
                    .ok_or("rings must be 1..64")
            })?;
            let edges = crate::surface_ops::edges(mesh, Some(object));
            for _ in 0..rings {
                let previous = faces.clone();
                for incident in edges.values() {
                    if incident.iter().any(|item| previous.contains(&item.0)) {
                        faces.extend(incident.iter().map(|item| item.0));
                    }
                }
            }
            selected = construction::vertices(mesh, &faces);
        }
        "coplanar_patch" => {
            let mut scoped = request.clone();
            scoped["coplanar"] = json!(true);
            faces = crate::regions::selection(mesh, &scoped)?;
            selected = construction::vertices(mesh, &faces);
        }
        "mirrored" => {
            if selected.is_empty() {
                return Err("mirroring requires selected faces or vertices".into());
            }
            let axis = match request["axis"].as_str() {
                Some("X") => 0,
                Some("Y") => 1,
                Some("Z") => 2,
                _ => return Err("axis must be X, Y or Z".into()),
            };
            let center = request.get("plane").map_or(Ok(0.0), |v| {
                v.as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or("plane must be finite")
            })?;
            let tolerance = request.get("tolerance").map_or(Ok(1e-7), |v| {
                v.as_f64()
                    .filter(|v| v.is_finite() && *v > 0.0)
                    .ok_or("tolerance must be positive")
            })?;
            let mut map = BTreeMap::new();
            for id in &selected {
                let mut p = mesh.vertices[id];
                p[axis] = 2.0 * center - p[axis];
                let matches: Vec<_> = mesh.objects[&object]
                    .vertices
                    .iter()
                    .filter(|other| length(sub(mesh.vertices[other], p)) <= tolerance)
                    .copied()
                    .collect();
                if matches.len() != 1 {
                    return Err("mirrored vertex correspondence is missing or ambiguous".into());
                }
                map.insert(*id, matches[0]);
                pairs.push([*id, matches[0]]);
            }
            let mut mirrored_faces = BTreeSet::new();
            for id in &faces {
                let target: BTreeSet<_> = mesh.faces[id].iter().map(|v| map[v]).collect();
                let matches: Vec<_> = mesh
                    .faces
                    .iter()
                    .filter(|(_, f)| {
                        f.len() == target.len() && f.iter().all(|v| target.contains(v))
                    })
                    .map(|(id, _)| *id)
                    .collect();
                if matches.len() != 1 {
                    return Err("mirrored face correspondence is missing or ambiguous".into());
                }
                mirrored_faces.insert(matches[0]);
            }
            faces = mirrored_faces;
            selected = map.into_values().collect();
        }
        _ => {
            return Err(
                "kind must be component, boundary_loop, adjacent_faces, coplanar_patch or mirrored"
                    .into(),
            );
        }
    }
    mesh.selection = selected.clone();
    Ok(json!({"object":object,"kind":kind,"vertices":selected,"faces":faces,"vertex_pairs":pairs}))
}
