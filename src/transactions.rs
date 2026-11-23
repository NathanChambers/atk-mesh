use crate::{Document, Mesh, diagnostics, geometry::*};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_PREVIEW: AtomicU64 = AtomicU64::new(1);

pub(super) fn difference(before: &Mesh, after: &Mesh) -> Value {
    let added_vertices: Vec<_> = after
        .vertices
        .keys()
        .filter(|id| !before.vertices.contains_key(id))
        .copied()
        .collect();
    let removed_vertices: Vec<_> = before
        .vertices
        .keys()
        .filter(|id| !after.vertices.contains_key(id))
        .copied()
        .collect();
    let moved_vertices:Vec<_>=before.vertices.iter().filter_map(|(id,position)|after.vertices.get(id).filter(|p|*p!=position).map(|p|json!({"vertex":id,"before":position,"after":p,"distance":length(sub(*p,*position))}))).collect();
    let added_faces: Vec<_> = after
        .faces
        .keys()
        .filter(|id| !before.faces.contains_key(id))
        .copied()
        .collect();
    let removed_faces: Vec<_> = before
        .faces
        .keys()
        .filter(|id| !after.faces.contains_key(id))
        .copied()
        .collect();
    let changed_faces: Vec<_> = before
        .faces
        .iter()
        .filter(|(id, face)| after.faces.get(id).is_some_and(|other| other != *face))
        .map(|(id, _)| *id)
        .collect();
    json!({"added_vertices":added_vertices,"removed_vertices":removed_vertices,"moved_vertices":moved_vertices,"added_faces":added_faces,"removed_faces":removed_faces,"changed_faces":changed_faces,"objects_before":before.objects.len(),"objects_after":after.objects.len()})
}

fn changed_face_types(before: &Mesh, after: &Mesh) -> Value {
    let counts = |source: &Mesh, other: &Mesh| {
        let mut counts = [0; 3];
        for (id, face) in &source.faces {
            if other.faces.get(id) != Some(face) {
                let kind = match face.len() {
                    3 => 0,
                    4 => 1,
                    _ => 2,
                };
                counts[kind] += 1;
            }
        }
        json!({"triangles":counts[0],"quads":counts[1],"ngons":counts[2]})
    };
    json!({"added_or_rewritten":counts(after,before),"removed_or_rewritten":counts(before,after)})
}

fn boundary_changes(before: &Mesh, after: &Mesh, request: &Value) -> Result<Vec<Value>, String> {
    let names = |value: Option<&Value>| -> Result<BTreeSet<String>, String> {
        value.map_or(Ok(BTreeSet::new()), |value| {
            serde_json::from_value(value.clone()).map_err(|_| "boundary lists must contain region names".into())
        })
    };
    let watched = names(request.get("watch_boundaries"))?;
    let protected = names(request["guards"].get("protected_boundaries"))?;
    let mut reports = Vec::new();
    for name in watched.union(&protected) {
        let original = crate::patches::named_boundary(before, name)?;
        let current = crate::patches::named_boundary(after, name);
        let moved: Vec<_> = original.iter().filter(|id| {
            after.vertices.get(id).is_some_and(|position| position != &before.vertices[id])
        }).copied().collect();
        let removed: Vec<_> = original.iter().filter(|id| !after.vertices.contains_key(id)).copied().collect();
        let same_perimeter = current.as_ref().is_ok_and(|ids| ids == &original);
        if protected.contains(name) && (!same_perimeter || !moved.is_empty()) {
            return Err(format!("transaction changes protected boundary {name}"));
        }
        reports.push(json!({"region":name,"perimeter_unchanged":same_perimeter,"moved_vertices":moved,"removed_vertices":removed,"before":original,"after":current.ok()}));
    }
    Ok(reports)
}

fn protect(before: &Mesh, after: &Mesh, request: &Value) -> Result<(), String> {
    let guards = &request["guards"];
    if let Some(values) = guards.as_object() {
        for (key, value) in values {
            if !matches!(
                key.as_str(),
                "max_displacement"
                    | "protected_objects"
                    | "protected_regions"
                    | "protected_boundaries"
                    | "no_new_components"
                    | "protected_volumes"
                    | "no_new_defects"
            ) {
                return Err(format!("unknown transaction guard: {key}"));
            }
            if matches!(key.as_str(), "no_new_components" | "no_new_defects") && !value.is_boolean()
            {
                return Err(format!("{key} must be boolean"));
            }
        }
    } else if !guards.is_null() {
        return Err("guards must be an object".into());
    }
    if let Some(value) = guards.get("max_displacement") {
        let max = value
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or("max_displacement must be finite and nonnegative")?;
        if before.vertices.iter().any(|(id, p)| {
            after
                .vertices
                .get(id)
                .is_some_and(|q| length(sub(*q, *p)) > max)
        }) {
            return Err("transaction exceeds maximum displacement".into());
        }
    }
    let objects: BTreeSet<u64> = guards
        .get("protected_objects")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "protected_objects must be IDs"))
        .transpose()?
        .unwrap_or_default();
    let mut protected_faces = BTreeSet::new();
    let mut protected_vertices = BTreeSet::new();
    for id in objects {
        let object = before.objects.get(&id).ok_or("unknown protected object")?;
        if after.objects.get(&id) != Some(object) {
            return Err("transaction changes a protected object".into());
        }
        let old_faces: BTreeMap<_, _> = before
            .faces
            .iter()
            .filter(|(_, f)| object.vertices.contains(&f[0]))
            .collect();
        let new_faces: BTreeMap<_, _> = after
            .faces
            .iter()
            .filter(|(_, f)| object.vertices.contains(&f[0]))
            .collect();
        if old_faces != new_faces {
            return Err("transaction changes a protected object's surfaces".into());
        }
        protected_vertices.extend(&object.vertices);
        protected_faces.extend(
            before
                .faces
                .iter()
                .filter(|(_, face)| object.vertices.contains(&face[0]))
                .map(|(id, _)| *id),
        );
    }
    let regions: Vec<String> = guards
        .get("protected_regions")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "protected_regions must be names"))
        .transpose()?
        .unwrap_or_default();
    for name in regions {
        let id = crate::construction::named(before, &name)?;
        let component = &before.components[&id];
        protected_faces.extend(&component.faces);
        protected_vertices.extend(crate::construction::vertices(before, &component.faces));
    }
    if protected_vertices
        .iter()
        .any(|id| before.vertices.get(id) != after.vertices.get(id))
        || protected_faces
            .iter()
            .any(|id| before.faces.get(id) != after.faces.get(id))
    {
        return Err("transaction changes protected geometry".into());
    }
    if guards["no_new_components"].as_bool().unwrap_or(false) {
        let old: usize = before
            .objects
            .keys()
            .map(|id| diagnostics::components(before, *id).len())
            .sum();
        let new: usize = after
            .objects
            .keys()
            .map(|id| diagnostics::components(after, *id).len())
            .sum();
        if new > old {
            return Err("transaction adds disconnected components".into());
        }
        for id in before.objects.keys() {
            if after.objects.contains_key(id)
                && diagnostics::components(after, *id).len()
                    > diagnostics::components(before, *id).len()
            {
                return Err("transaction adds disconnected components within an object".into());
            }
        }
    }
    if let Some(volumes) = guards.get("protected_volumes") {
        for volume in volumes
            .as_array()
            .ok_or("protected_volumes must be an array of volume queries")?
        {
            if diagnostics::volume(after, volume)?["detected"] == true {
                return Err("transaction intrudes into a protected volume".into());
            }
        }
    }
    Ok(())
}

fn svg(before: &Mesh, after: &Mesh, request: &Value) -> Result<String, String> {
    let camera = crate::projection::Camera::parse(&request["camera"])?;
    let options = crate::render::Options::parse(&request["camera"])?;
    let mut output = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\"><rect width=\"100%\" height=\"100%\" fill=\"#eeeeee\"/>",
        options.width, options.height
    );
    for (mesh, removed) in [(after, false), (before, true)] {
        for (id, face) in &mesh.faces {
            let changed = before.faces.get(id) != after.faces.get(id);
            if removed && !changed {
                continue;
            }
            let color = if removed {
                "#d52b32"
            } else if changed {
                "#198544"
            } else {
                "#aaaaaa"
            };
            for i in 0..face.len() {
                let a = camera.raster_point(mesh.vertices[&face[i]])?;
                let b = camera.raster_point(mesh.vertices[&face[(i + 1) % face.len()]])?;
                output.push_str(&format!(
                    "<path d=\"M {} {} L {} {}\" stroke=\"{color}\" stroke-width=\"{}\"/>",
                    a[0],
                    a[1],
                    b[0],
                    b[1],
                    if changed { 2 } else { 1 }
                ));
            }
        }
    }
    for (id, p) in &before.vertices {
        if let Some(q) = after.vertices.get(id) {
            if p != q {
                let a = camera.raster_point(*p)?;
                let b = camera.raster_point(*q)?;
                output.push_str(&format!(
                    "<path d=\"M {} {} L {} {}\" stroke=\"#ce8b00\" stroke-width=\"3\"/>",
                    a[0], a[1], b[0], b[1]
                ));
            }
        }
    }
    output.push_str("<text x=\"12\" y=\"24\" font-family=\"sans-serif\" font-size=\"14\">Green: added/changed faces. Red: removed/old faces. Amber: moved vertices. Wire overlay; no occlusion.</text></svg>");
    Ok(output)
}

pub(super) struct Prepared {
    id: String,
    revision: u64,
    mesh: Box<Mesh>,
}

fn commit_mesh(document: &mut Document, mesh: Mesh) {
    let previous = std::mem::replace(&mut document.mesh, mesh);
    document.undo.push(previous);
    if document.undo.len() > 100 {
        document.undo.remove(0);
    }
    document.redo.clear();
    document.transaction_preview = None;
    document.revision += 1;
}

pub(super) fn execute(document: &mut Document, request: &Value) -> Result<Value, String> {
    if document.mesh.defer_validation {
        return Err("nested transactions are not supported".into());
    }
    let commit = request["commit"]
        .as_bool()
        .ok_or("commit must explicitly be true or false")?;
    if let Some(value) = request.get("preview_id") {
        let supplied_options = request.as_object().unwrap().keys().any(|key| {
            !matches!(key.as_str(), "command" | "commit" | "preview_id" | "revision" | "response" | "cli_request_id")
        });
        if !commit || supplied_options {
            return Err("preview_id commits a prepared transaction. Omit commands, guards and camera options".into());
        }
        let id = value.as_str().ok_or("preview_id must be a string")?;
        let revision = request["revision"].as_u64().ok_or("preview reuse requires the preview revision")?;
        let candidate = document.transaction_preview.as_ref().ok_or("no prepared transaction preview in this editor")?;
        if candidate.id != id || revision != candidate.revision || revision != document.revision {
            return Err("transaction preview is stale or was replaced. Preview the edit again".into());
        }
        let candidate = document.transaction_preview.take().unwrap();
        commit_mesh(document, *candidate.mesh);
        return Ok(json!({"committed":true,"preview_id":id,"base_revision":revision,"revision":document.revision}));
    }
    let steps = request["commands"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or("commands must contain 1..128 document edits")?;
    let preview = request.get("preview");
    if preview.is_some_and(|options| !options.is_object()) {
        return Err("preview must be a capture options object".into());
    }
    let mut temporary = Document {
        mesh: document.mesh.clone(),
        revision: document.revision,
        undo: vec![],
        redo: vec![],
        views: document.views.clone(),
        next_view: document.next_view,
        transaction_preview: None,
    };
    temporary.mesh.defer_validation = true;
    let mut operations = vec![];
    for (index, step) in steps.iter().enumerate() {
        let command = step["command"]
            .as_str()
            .ok_or("each step requires command")?;
        if matches!(
            command,
            "load"
                | "cube"
                | "undo"
                | "redo"
                | "transaction"
                | "export"
                | "bake_surface_map"
                | "save"
                | "capture"
                | "measure_clearance"
        ) || command.starts_with("workflow_")
            || command.starts_with("query_")
            || command.starts_with("inspect")
            || command.starts_with("review_")
        {
            return Err("transactions accept document edits only; reset, history, queries, workflow, exports and captures are excluded".into());
        }
        let result = temporary
            .execute(step.clone())
            .map_err(|error| format!("transaction step {index} ({command}): {error}"))?;
        operations.push(
            result
                .get("operation")
                .cloned()
                .unwrap_or_else(|| json!({"command":command})),
        );
        temporary.undo.clear();
        temporary.redo.clear();
    }
    temporary.mesh.defer_validation = false;
    crate::objects::reconcile(&mut temporary.mesh)?;
    crate::topology::validate_topology(&temporary.mesh)?;
    crate::construction::validate(&temporary.mesh)?;
    protect(&document.mesh, &temporary.mesh, request)?;
    let boundaries = boundary_changes(&document.mesh, &temporary.mesh, request)?;
    let before_report = diagnostics::full_report(
        &document.mesh,
        &json!({"response":"full","limit":256}),
        document.revision,
    )?;
    let after_report = diagnostics::full_report(
        &temporary.mesh,
        &json!({"response":"full","limit":256}),
        document.revision,
    )?;
    let old = before_report["items"].as_array().unwrap();
    let new_defects: Vec<_> = after_report["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| {
            !old.iter().any(|previous| {
                previous["kind"] == item["kind"] && previous["faces"] == item["faces"]
            })
        })
        .cloned()
        .collect();
    if request["guards"]["no_new_defects"]
        .as_bool()
        .unwrap_or(false)
    {
        if before_report["complete"] != true
            || after_report["complete"] != true
            || before_report["next_offset"].is_number()
            || after_report["next_offset"].is_number()
        {
            return Err("no_new_defects guard requires complete untruncated diagnostics; narrow or repair the model first".into());
        }
        let introduced = new_defects.iter().any(|item| item["intentional"] != true);
        if introduced {
            return Err("transaction adds surface defects".into());
        }
    }
    let diff = difference(&document.mesh, &temporary.mesh);
    let reports_complete = [&before_report, &after_report].iter().all(|report| {
        report["complete"] == true && report["next_offset"].is_null()
    });
    let face_layout = changed_face_types(&document.mesh, &temporary.mesh);
    let mut result = json!({"committed":commit,"revision":document.revision,"diff":diff,"face_layout":face_layout,"boundary_changes":boundaries,"operations":operations,"new_defects":new_defects,"new_defects_complete":reports_complete,"defects_before":before_report,"defects_after":after_report});
    if request.get("camera").is_some() {
        result["svg"] = json!(svg(&document.mesh, &temporary.mesh, request)?);
    }
    if let Some(options) = preview {
        let (before, _) = document.render_view(options)?;
        let (after, _) = temporary.render_view(options)?;
        result["preview_views"] = json!({
            "before":{"png_base64":before["png_base64"],"camera":before["camera"],"mode":before["mode"]},
            "after":{"png_base64":after["png_base64"],"camera":after["camera"],"mode":after["mode"]}
        });
    }
    if commit {
        commit_mesh(document, temporary.mesh);
        result["revision"] = json!(document.revision);
    } else {
        let index = NEXT_PREVIEW.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
        let id = format!("{time}-{}-{index}", std::process::id());
        result["preview_id"] = json!(id);
        document.transaction_preview = Some(Prepared {
            id,
            revision: document.revision,
            mesh: Box::new(temporary.mesh),
        });
    }
    Ok(result)
}
