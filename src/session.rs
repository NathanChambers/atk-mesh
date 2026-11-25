use crate::{Document, base64, responses};
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
mod export_verify;
mod review_report;

pub struct Session {
    debug_session: bool,
    compress_recordings: bool,
    document: Document,
    root: PathBuf,
    id: String,
    events: Vec<Value>,
    state: Value,
    snapshot: usize,
    command_id: usize,
    workspace: Option<PathBuf>,
    workflow: crate::workflow::Workflow,
}

#[derive(Serialize)]
struct RecordedEvent<'a> {
    summary: &'a Value,
    input: &'a Value,
    output: &'a Value,
    full_output: &'a Value,
}

#[derive(Serialize)]
struct WorkspaceProject<'a> {
    version: u64,
    mesh: &'a Value,
    revision: &'a Value,
}

impl Session {
    pub fn new(root: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        Self::create(root, Document::default(), None, true, false)
    }

    pub fn open_workspace(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_with_recording_compression(path, false)
    }

    pub fn open_with_recording_compression(path: PathBuf, compress_recordings: bool) -> Result<Self, Box<dyn std::error::Error>> {
        Self::open_editor(path, true, compress_recordings)
    }

    pub fn open_editor(path: PathBuf, debug_session: bool, compress_recordings: bool) -> Result<Self, Box<dyn std::error::Error>> {
        fs::create_dir_all(&path)?;
        let workspace = fs::canonicalize(path)?;
        let project_path = workspace.join("project.json");
        let mut document = Document::empty();
        if project_path.exists() {
            let source = fs::read_to_string(&project_path)?;
            let project: Value = serde_json::from_str(source.trim_start_matches('\u{feff}'))?;
            let saved_revision = project["revision"]
                .as_u64()
                .filter(|revision| *revision < u64::MAX);
            document.execute(json!({"command":"load","project":project}))?;
            document.undo.clear();
            if let Some(revision) = saved_revision {
                document.revision = revision;
            }
        }
        let session = Self::create(workspace.join("sessions"), document, Some(workspace), debug_session, compress_recordings)?;
        session.write_workspace_project(project_path)?;
        Ok(session)
    }

    fn create(
        root: PathBuf,
        mut document: Document,
        workspace: Option<PathBuf>,
        debug_session: bool,
        compress_recordings: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let id = format!("{time}-{}", std::process::id());
        let state = document.execute(json!({"command":"inspect"}))?;
        fs::create_dir_all(workspace.as_ref().unwrap_or(&root))?;
        let workflow = crate::workflow::Workflow::open(workspace.as_ref().unwrap_or(&root))?;
        let session = Self {
            debug_session,
            compress_recordings,
            document,
            root,
            id,
            events: vec![],
            state,
            snapshot: 0,
            command_id: 0,
            workspace,
            workflow,
        };
        session.workflow.save(session.workflow_directory())?;
        if session.debug_session {
            session.initialize_recording()?;
            session.write("index.json", &session.index())?;
        }
        Ok(session)
    }

    fn initialize_recording(&self) -> std::io::Result<()> {
        fs::create_dir_all(self.directory().join("events"))?;
        fs::create_dir_all(self.directory().join("states"))?;
        self.write("states/0.json", &self.state)
    }

    fn directory(&self) -> PathBuf {
        self.root.join(&self.id)
    }

    fn workflow_directory(&self) -> &std::path::Path {
        self.workspace.as_deref().unwrap_or(&self.root)
    }

    pub fn normalize_saved_stages(mesh: &mut Value) {
        let Some(objects) = mesh["objects"].as_object_mut() else {
            return;
        };
        for object in objects.values_mut().filter(|object| object["stage"] == "production_ready") {
            let approved = serde_json::from_value(object["review"].clone())
                .is_ok_and(|checks: Vec<crate::object_review::Check>| crate::object_review::approved_plan(&checks));
            if !approved {
                object["stage"] = json!("detailed");
                object.as_object_mut().unwrap().remove("stage_note");
            }
        }
    }

    fn execute_object_review(&mut self, request: &Value) -> Result<Value, String> {
        let id = request["object"].as_u64().ok_or("object is required")?;
        let object = self.document.mesh.objects.get(&id).ok_or("unknown object")?;
        let refined = request["command"] == "object_set_stage" && request["stage"] == "refined";
        if refined {
            let missing = self.workflow.gate("refined", &self.state["mesh"], self.workflow_directory());
            let checks = crate::object_review::refined_blockers(&object.review);
            if missing.is_some() || !checks.is_empty() {
                let mut result = missing.unwrap_or_else(|| json!({"status":"requirements_missing","stage":"refined","requirements":[]}));
                result["object"] = json!(id);
                result["object_checks"] = json!(checks);
                result["review_command"] = json!({"command":"query_object_review","object":id});
                return Ok(result);
            }
        }
        let workspace_evidence = crate::object_review::verify(object, request, |resource| {
            self.workflow.object_review_evidence(resource, id, &self.state["mesh"], self.workflow_directory())
        })?;
        let mut request = request.clone();
        request["workspace_evidence"] = json!(workspace_evidence);
        self.document.execute_edit(request, true)
    }

    fn execute_workflow(&mut self, request: &Value) -> Result<Value, String> {
        let mut request = request.clone();
        if request["command"] == "workflow_resource" {
            request.as_object_mut().unwrap().remove("evidence_rig");
        }
        let review_report = request["kind"] == "evidence" && request["capture"] == "report";
        if request["command"] == "workflow_resource" && (request["kind"] == "capture" || review_report) {
            let session = request["session"]
                .as_str()
                .ok_or("capture requires trace session and event")?
                .to_string();
            let event = request["event"]
                .as_u64()
                .ok_or("capture requires event ID")?;
            let record = self.read(&format!("/sessions/{session}/events/{event}"))?;
            let valid_capture = record["output"]["ok"] == true
                && matches!(
                    record["input"]["command"].as_str(),
                    Some("capture" | "compare_reference" | "review_run" | "export_verify")
                );
            if !valid_capture || (record["input"]["command"]=="export_verify" && !review_report) {
                return Err("source event is not a successful capture or comparison".into());
            }
            let snapshot = record["summary"]["snapshot"]
                .as_u64()
                .ok_or("missing snapshot")?;
            let state = self.read(&format!("/sessions/{session}/states/{snapshot}"))?;
            request["evidence_rig"] = json!(crate::workflow::review_rig_fingerprint(
                &state["mesh"], request["object"].as_u64(),
            ));
            if !crate::workflow::same_geometry(
                &state["mesh"],
                &self.state["mesh"],
                request["object"].as_u64(),
            ) {
                return Err("capture geometry is stale; capture again".into());
            }
            if review_report {
                let review = &record["full_output"]["result"];
                let receipt = review.get("export_verification").unwrap_or(review);
                if !matches!(record["input"]["command"].as_str(), Some("review_run" | "export_verify")) || receipt["status"] == "failed" {
                    return Err("report evidence requires a successful review or verification".into());
                }
                request["path"] = record["output"]["result"]["artifacts"]["report"]["path"].clone();
            } else if record["input"]["command"] == "review_run" {
                let part = request["capture"]
                    .as_str()
                    .filter(|part| matches!(*part, "neutral" | "wireframe" | "silhouette_overlay" | "tessellation" | "front" | "side" | "top" | "three_quarter" | "intended"))
                    .ok_or(
                        "review capture requires capture: neutral, wireframe, silhouette_overlay or tessellation",
                    )?;
                let path = record["output"]["result"]["artifacts"][part]["path"].clone();
                let camera = record["full_output"]["result"]["views"][part]["camera"].clone();
                request["path"] = path;
                request["camera"] = camera;
            } else {
                request["path"] = record["output"]["result"]["artifacts"]["png"]["path"].clone();
                request["camera"] = record["full_output"]["result"]["camera"].clone();
            }
            request["source"] =
                json!({"session":session,"event":event,"command":record["input"]["command"]});
            let animation = &record["full_output"]["result"]["animation"];
            if animation.is_object() {
                let clip = animation["clip"].as_str().ok_or("capture clip is missing")?;
                let captured = crate::workflow::animation_fingerprint(&state["mesh"], clip);
                let current = crate::workflow::animation_fingerprint(&self.state["mesh"], clip);
                if captured != current {
                    return Err("capture animation is stale; capture again".into());
                }
                request["source"]["animation"] = animation.clone();
                request["source"]["animation_fingerprint"] = json!(captured);
            }
            if record["input"]["command"] == "review_run" {
                let review = &record["full_output"]["result"];
                request["source"]["review"] = json!({"name":review["name"],"preset_version":review["preset_version"],"capture":request["capture"]});
            }
        }
        let mut workflow = self.workflow.clone();
        let result = workflow.execute(&request, &self.state["mesh"], self.workflow_directory())?;
        let read_only = request["command"] == "workflow_status"
            || (request["command"] == "workflow_part_plan" && request.get("plan").is_none());
        if !read_only && result["status"] != "requirements_missing" {
            workflow.save(self.workflow_directory())?;
            self.workflow = workflow;
        }
        Ok(result)
    }

    fn reference_context(&self, request: &Value) -> Result<crate::alignment::Context, String> {
        if let Some(expected) = request.get("revision") {
            let expected = expected.as_u64().ok_or("revision must be an integer")?;
            if expected != self.document.revision() {
                return Err("stale document revision; run project_reference again".into());
            }
        }
        let id = request["reference"]
            .as_str()
            .ok_or("reference resource ID is required")?;
        let resource = self.workflow.reference(id, self.workflow_directory())?;
        if let Some(expected) = request.get("alignment_version") {
            let expected = expected
                .as_u64()
                .ok_or("alignment_version must be an integer")?;
            if expected != resource["alignment_version"].as_u64().unwrap_or(0) {
                return Err("stale reference alignment; run project_reference again".into());
            }
        }
        let path = self
            .workflow_directory()
            .join(resource["path"].as_str().ok_or("missing reference path")?);
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let directory = fs::canonicalize(self.workflow_directory()).map_err(|e| e.to_string())?;
        if !path.starts_with(directory) {
            return Err("reference must remain inside workspace".into());
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let mut view = resource["view"]
            .as_object()
            .cloned()
            .map(Value::Object)
            .unwrap_or_else(|| json!({}));
        let allow_overrides = matches!(
            request["command"].as_str(),
            Some("compare_reference" | "reference_view" | "fit_reference")
        );
        if allow_overrides {
            if request.get("target").is_none()
                && (request.get("yaw").is_some() || request.get("pitch").is_some())
            {
                view.as_object_mut().unwrap().remove("target");
            }
            for key in [
                "yaw",
                "pitch",
                "zoom",
                "center",
                "width",
                "height",
                "crop",
                "mode",
                "projection",
                "position",
                "target",
                "fov",
                "near",
                "lens_shift",
            ] {
                if let Some(value) = request.get(key) {
                    view[key] = value.clone();
                }
            }
        }
        let camera = crate::projection::Camera::parse(&view)?;
        let size = crate::comparison::dimensions(&bytes)?;
        let frame = crate::comparison::Frame::new(size, &view, camera.width, camera.height)?;
        Ok(crate::alignment::Context {
            resource,
            bytes,
            view,
            camera,
            frame,
        })
    }

    fn compare_reference(&mut self, request: &Value) -> Result<Value, String> {
        let context = self.reference_context(request)?;
        let visibility = crate::render::CaptureVisibility::parse(request, &self.document.mesh)?;
        let mut resource = context.resource.clone();
        let mut omitted = 0;
        if [
            "hide_objects",
            "solo_objects",
            "hide_collections",
            "solo_collections",
        ]
        .iter()
        .any(|key| request.get(key).is_some())
        {
            let mut landmarks = vec![];
            let mut names = std::collections::BTreeSet::new();
            for landmark in crate::alignment::landmarks(&resource)? {
                let vertices = crate::alignment::vertices(&self.document.mesh, &landmark)?;
                if vertices.iter().any(|id| !visibility.includes_vertex(id)) {
                    omitted += 1;
                    continue;
                }
                names.insert(landmark.name.clone());
                landmarks.push(landmark);
            }
            let contours: Vec<_> = crate::alignment::contours(&resource)?
                .into_iter()
                .filter(|contour| contour.landmarks.iter().all(|name| names.contains(name)))
                .collect();
            resource["landmarks"] = json!(landmarks);
            resource["contours"] = json!(contours);
        }
        let mut alignment = crate::alignment::project(
            &self.document.mesh,
            &resource,
            &context.camera,
            &context.frame,
            self.document.revision(),
        )?;
        alignment["contours"] = resource
            .get("contours")
            .cloned()
            .unwrap_or_else(|| json!([]));
        alignment.as_object_mut().unwrap().remove("move_template");
        alignment["omitted_hidden_landmarks"] = json!(omitted);
        if request
            .get("layout")
            .is_some_and(|layout| !layout.is_string())
        {
            return Err("layout must be side_by_side or overlay".into());
        }
        let layout = request["layout"].as_str().unwrap_or("side_by_side");
        if !matches!(layout, "side_by_side" | "overlay") {
            return Err("layout must be side_by_side or overlay".into());
        }
        let mut capture = context.view;
        capture["command"] = json!("capture");
        capture["reference"] = context.resource["id"].clone();
        capture["layout"] = json!(layout);
        for key in [
            "hide_objects",
            "solo_objects",
            "hide_collections",
            "solo_collections",
        ] {
            if let Some(value) = request.get(key) {
                capture[key] = value.clone();
            }
        }
        if layout == "overlay" {
            capture["mode"] = json!("silhouette");
            capture["clean"] = json!(true);
        }
        let result = self.document.execute(capture.clone())?;
        crate::comparison::compose(&context.bytes, &result, &capture, &alignment)
    }

    fn review_command(&mut self, request: &Value) -> Result<Value, String> {
        let command = request["command"].as_str().unwrap_or("");
        if command == "query_reviews" {
            return crate::responses::slice(&json!(self.workflow.review_presets), request);
        }
        let name = request["name"]
            .as_str()
            .filter(|name| {
                !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
            })
            .ok_or("name must contain 1..256 printable characters")?;
        if command == "review_define" || command == "review_update" {
            let previous = self.workflow.review_presets.get(name).cloned();
            if (command == "review_define" && previous.is_some())
                || (command == "review_update" && previous.is_none())
            {
                return Err(
                    "use review_define for a new preset or review_update for an existing preset"
                        .into(),
                );
            }
            let mut preset = previous.clone().unwrap_or_else(|| json!({}));
            for key in [
                "reference",
                "view",
                "crop",
                "dimensions",
                "clearances",
                "protected_volumes",
                "visibility",
                "tessellation",
                "wireframe",
                "defects",
                "standard_views",
                "export",
            ] {
                if let Some(value) = request.get(key) {
                    preset[key] = value.clone();
                }
            }
            for key in ["tessellation", "wireframe", "standard_views"] {
                if preset.get(key).is_some_and(|value| !value.is_boolean()) {
                    return Err(format!("{key} must be a boolean"));
                }
            }
            if let Some(export) = preset.get("export") && !matches!(export["format"].as_str(), Some("glb" | "fbx" | "obj" | "stl")) {
                return Err("export preset requires a supported format".into());
            }
            if preset.get("defects").is_some_and(|value| !value.is_boolean() && !value.is_object()) {
                return Err("defects must be a boolean or an inspect_defects options object".into());
            }
            let view = preset["view"]
                .as_object()
                .ok_or("view must explicitly contain fixed camera options")?;
            let mut comparison = Value::Object(view.clone());
            comparison["command"] = json!("compare_reference");
            comparison["reference"] = preset["reference"].clone();
            if let Some(crop) = preset.get("crop") {
                comparison["crop"] = crop.clone();
            }
            if preset["reference"].is_null() {
                if request.get("crop").is_some_and(|value| !value.is_null()) {
                    return Err("crop requires a reference image. Set the camera framing through view".into());
                }
                preset["view"] = crate::projection::Camera::parse(&comparison)?.metadata();
                preset.as_object_mut().unwrap().remove("crop");
                preset.as_object_mut().unwrap().remove("reference_hash");
            } else {
                let context = self.reference_context(&comparison)?;
                let mut frozen = context.camera.metadata();
                frozen["crop"] = json!(context.frame.crop);
                preset["view"] = frozen;
                preset["reference_hash"] = context.resource["content_hash"].clone();
            }
            preset["version"] = json!(
                previous
                    .as_ref()
                    .and_then(|p| p["version"].as_u64())
                    .unwrap_or(0)
                    + 1
            );
            for key in ["dimensions", "clearances", "protected_volumes"] {
                if preset
                    .get(key)
                    .is_some_and(|v| v.as_array().is_none_or(|v| v.len() > 64))
                {
                    return Err(format!("{key} must be an array with at most 64 checks"));
                }
            }
            let mut visibility = json!({});
            if let Some(value) = preset.get("visibility") {
                visibility = value.clone();
            }
            let visibility_fields = visibility
                .as_object()
                .ok_or("visibility must be an object")?;
            if visibility_fields.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "hide_objects" | "solo_objects" | "hide_collections" | "solo_collections"
                )
            }) {
                return Err("visibility only accepts hide/solo objects or collections; change camera through view".into());
            }
            crate::render::CaptureVisibility::parse(&visibility, &self.document.mesh)?;
            let mut workflow = self.workflow.clone();
            if workflow.review_presets.len() >= 64 && previous.is_none() {
                return Err("review preset budget exceeded (64)".into());
            }
            workflow
                .review_presets
                .insert(name.to_string(), preset.clone());
            workflow.save(self.workflow_directory())?;
            self.workflow = workflow;
            return Ok(
                json!({"name":name,"preset":preset,"camera_changes":"only review_update changes this preset; review_run rejects overrides"}),
            );
        }
        if command != "review_run" {
            return Err("unknown review command".into());
        }
        if request.as_object().unwrap().keys().any(|key| {
            !matches!(
                key.as_str(),
                "command" | "name" | "response" | "revision" | "preset_version" | "cli_request_id"
            )
        }) {
            return Err("review_run uses its fixed preset; change camera/crop explicitly with review_update".into());
        }
        if let Some(revision) = request.get("revision") {
            if revision.as_u64() != Some(self.document.revision()) {
                return Err("stale document revision".into());
            }
        }
        let preset = self
            .workflow
            .review_presets
            .get(name)
            .ok_or("unknown review preset")?
            .clone();
        if let Some(expected) = request.get("preset_version") {
            if expected.as_u64() != preset["version"].as_u64() {
                return Err(
                    "review preset changed; capture a new baseline before comparing".into(),
                );
            }
        }
        let has_reference = !preset["reference"].is_null();
        if has_reference {
            let id = preset["reference"].as_str().ok_or("preset reference is invalid")?;
            let reference = self.workflow.reference(id, self.workflow_directory())?;
            if reference["content_hash"] != preset["reference_hash"] {
                return Err("review reference changed; explicitly update the preset".into());
            }
        }
        let mut views = serde_json::Map::new();
        let mut base = preset["view"].clone();
        if let Some(visibility) = preset["visibility"].as_object() {
            for (key, value) in visibility {
                base[key] = value.clone();
            }
        }
        for (label, mode) in [("neutral", "shaded"), ("wireframe", "topology")] {
            if label == "wireframe" && preset["wireframe"] == false {
                continue;
            }
            let mut capture = base.clone();
            capture["command"] = json!("capture");
            capture["mode"] = json!(mode);
            capture["clean"] = json!(label == "neutral" || !has_reference);
            views.insert(label.into(), self.document.execute(capture)?);
        }
        if preset["tessellation"] == true {
            let mut capture = base.clone();
            capture["command"] = json!("capture");
            capture["mode"] = json!("tessellation");
            capture["clean"] = json!(true);
            views.insert("tessellation".into(), self.document.execute(capture)?);
        }
        if has_reference {
            let mut comparison = base.clone();
            comparison["command"] = json!("compare_reference");
            comparison["reference"] = preset["reference"].clone();
            comparison["layout"] = json!("overlay");
            views.insert("silhouette_overlay".into(), self.compare_reference(&comparison)?);
        }
        if preset["standard_views"] == true {
            for (label, angles) in [("front",Some((0.,0.))),("side",Some((90.,0.))),("top",Some((0.,90.))),("three_quarter",Some((45.,25.))),("intended",None)] {
                let mut capture = base.clone();
                capture["command"] = json!("capture");
                capture["mode"] = json!("material");
                capture["clean"] = json!(true);
                if let Some((yaw,pitch)) = angles {
                    capture["yaw"] = json!(yaw);
                    capture["pitch"] = json!(pitch);
                }
                views.insert(label.into(),self.document.execute(capture)?);
            }
        }
        let mut dimensions = vec![];
        if let Some(values) = preset["dimensions"].as_array() {
            for query in values {
                let faces = crate::diagnostics::faces(&self.document.mesh, query)?;
                let vertices = crate::construction::vertices(&self.document.mesh, &faces);
                let bounds = crate::construction::bounds(&self.document.mesh, &vertices);
                let size: Vec<_> = (0..3)
                    .map(|axis| {
                        bounds["max"][axis]
                            .as_f64()
                            .zip(bounds["min"][axis].as_f64())
                            .map(|(a, b)| a - b)
                    })
                    .collect();
                dimensions.push(json!({"name":query["name"],"bounds":bounds,"size_xyz":size}));
            }
        }
        let mut clearances = vec![];
        if let Some(values) = preset["clearances"].as_array() {
            for query in values {
                let mut result = crate::diagnostics::clearance(&self.document.mesh, query)?;
                result["name"] = query["name"].clone();
                if let Some(min) = query.get("minimum") {
                    let min = min
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.0)
                        .ok_or("clearance minimum must be finite and nonnegative")?;
                    result["passes"] = json!(result["distance"].as_f64().unwrap() >= min);
                }
                clearances.push(result);
            }
        }
        let mut volumes = vec![];
        if let Some(values) = preset["protected_volumes"].as_array() {
            for query in values {
                volumes.push(crate::diagnostics::volume(&self.document.mesh, query)?);
            }
        }
        let defects = if preset["defects"] == false {
            Value::Null
        } else {
            let options = preset.get("defects").filter(|value| value.is_object());
            let default = json!({"limit":256});
            crate::diagnostics::inspect(&self.document.mesh, options.unwrap_or(&default), self.document.revision())?
        };
        let mut report = json!({"name":name,"preset_version":preset["version"],"revision":self.document.revision(),"fixed_view":preset["view"],"reference":preset["reference"],"views":views,"dimensions":dimensions,"clearances":clearances,"protected_volumes":volumes,"defects":defects,"assessment":"Repeatable evidence only; read images and reports before assessing workflow requirements."});
        report["source_metrics"] = crate::source_metrics::inspect(&self.document.mesh, &json!({}))?;
        report["constraint_results"] = json!(self.workflow.constraint_results(&self.document.mesh, &report["source_metrics"])?);
        if let Some(options) = preset.get("export") {
            let (receipt, payload) = self.verify_export(&json!({"export":options}), &preset["version"])?;
            report["export_verification"] = receipt;
            if report["export_verification"]["status"]=="passed" && self.workflow.gate("export",&self.state["mesh"],self.workflow_directory()).is_none() {
                for (key,value) in payload.as_object().unwrap() {
                    if key=="obj" || key.ends_with("_base64") { report[key]=value.clone(); }
                }
            }
        }
        Ok(report)
    }

    fn reference_command(&mut self, request: &Value) -> Result<Value, String> {
        let context = self.reference_context(request)?;
        let id = context.resource["id"].as_str().unwrap();
        let command = request["command"].as_str().unwrap();
        if command == "reference_landmarks" && request.get("landmarks").is_none() {
            let kind = request["kind"].as_str().unwrap_or("landmarks");
            if !matches!(kind, "landmarks" | "contours") {
                return Err("kind must be landmarks or contours".into());
            }
            let offset = request
                .get("offset")
                .map(|value| value.as_u64().ok_or("offset must be an integer"))
                .transpose()?
                .unwrap_or(0) as usize;
            let limit = request
                .get("limit")
                .map(|value| {
                    value
                        .as_u64()
                        .filter(|v| (1..=64).contains(v))
                        .ok_or("limit must be 1..64")
                })
                .transpose()?
                .unwrap_or(8) as usize;
            let values = context.resource[kind]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let items: Vec<_> = values.iter().skip(offset).take(limit).cloned().collect();
            let next = offset.saturating_add(items.len());
            return Ok(
                json!({"reference":id,"alignment_version":context.resource["alignment_version"].as_u64().unwrap_or(0),"kind":kind,"items":items,"total":values.len(),"next_offset":if next<values.len() {json!(next)} else {Value::Null}}),
            );
        }
        if command == "project_reference" {
            let mut resource = context.resource.clone();
            if let Some(names) = request.get("names") {
                let names: Vec<String> = serde_json::from_value(names.clone())
                    .map_err(|_| "names must be landmark names")?;
                let points = crate::alignment::landmarks(&resource)?;
                if names
                    .iter()
                    .any(|name| !points.iter().any(|point| &point.name == name))
                {
                    return Err("unknown landmark name".into());
                }
                resource["landmarks"] = json!(
                    points
                        .into_iter()
                        .filter(|point| names.contains(&point.name))
                        .collect::<Vec<_>>()
                );
            }
            let mut report = crate::alignment::project(
                &self.document.mesh,
                &resource,
                &context.camera,
                &context.frame,
                self.document.revision(),
            )?;
            let offset = request
                .get("offset")
                .map(|v| v.as_u64().ok_or("offset must be an integer"))
                .transpose()?
                .unwrap_or(0) as usize;
            let limit = request
                .get("limit")
                .map(|v| {
                    v.as_u64()
                        .filter(|v| (1..=64).contains(v))
                        .ok_or("limit must be 1..64")
                })
                .transpose()?
                .unwrap_or(16) as usize;
            let entries = report["landmarks"].as_array().unwrap();
            let total = entries.len();
            let mut entries: Vec<_> = entries.iter().skip(offset).take(limit).cloned().collect();
            for entry in &mut entries {
                let entry = entry.as_object_mut().unwrap();
                entry.remove("target_canvas");
                entry.remove("projected_canvas");
            }
            let next = offset + entries.len();
            report["landmarks"] = json!(entries);
            report["total"] = json!(total);
            report["next_offset"] = if next < total {
                json!(next)
            } else {
                Value::Null
            };
            return Ok(report);
        }
        if command == "move_in_reference" {
            let revision = request["revision"]
                .as_u64()
                .ok_or("revision is required; run project_reference first")?;
            if revision != self.document.revision() {
                return Err("stale document revision; run project_reference again".into());
            }
            let version = request["alignment_version"]
                .as_u64()
                .ok_or("alignment_version is required; run project_reference first")?;
            if version != context.resource["alignment_version"].as_u64().unwrap_or(0) {
                return Err("stale reference alignment; run project_reference again".into());
            }
            let mut edit = json!({"command":"move_in_view","revision":revision,"camera":context.camera.metadata()});
            let delta = if let Some(name) = request.get("landmark") {
                if request.get("vertices").is_some() {
                    return Err("choose landmark or vertices, not both".into());
                }
                let points = crate::alignment::landmarks(&context.resource)?;
                let landmark = points
                    .iter()
                    .find(|landmark| Some(landmark.name.as_str()) == name.as_str())
                    .ok_or("unknown landmark")?;
                let anchor = crate::alignment::anchor(&self.document.mesh, landmark)?
                    .ok_or("landmark is unbound; register vertices or object")?;
                let projected = context.camera.project(anchor)?;
                let pixel = context.frame.to_image([projected[0], projected[1]]);
                edit["vertices"] =
                    json!(crate::alignment::vertices(&self.document.mesh, landmark)?);
                if let Some(object) = landmark.object {
                    edit["object"] = json!(object);
                }
                request
                    .get("delta_pixels")
                    .map(crate::projection::pair)
                    .unwrap_or(Ok([
                        landmark.pixel[0] - pixel[0],
                        landmark.pixel[1] - pixel[1],
                    ]))?
            } else {
                if let Some(vertices) = request.get("vertices") {
                    edit["vertices"] = vertices.clone();
                }
                crate::projection::pair(&request["delta_pixels"])?
            };
            if let Some(object) = request.get("object") {
                edit["object"] = object.clone();
            }
            edit["delta_pixels"] = json!([
                delta[0] * context.frame.scale,
                delta[1] * context.frame.scale
            ]);
            let mut result = self.document.execute(edit)?;
            result["operation"]["reference_delta_pixels"] = json!(delta);
            result["operation"]["reference"] = json!(id);
            return Ok(result);
        }
        let mut workflow = self.workflow.clone();
        let mut result = if command == "reference_landmarks" {
            let size = crate::comparison::dimensions(&context.bytes)?;
            crate::alignment::validate(request, &self.document.mesh, size)?;
            workflow.update_reference(id, None, Some(request))?
        } else if command == "reference_view" {
            workflow.update_reference(id, Some(context.view), None)?
        } else if command == "fit_reference" {
            let before = crate::alignment::project(
                &self.document.mesh,
                &context.resource,
                &context.camera,
                &context.frame,
                self.document.revision(),
            )?;
            let fitted = crate::alignment::fit(
                &self.document.mesh,
                &context.resource,
                request,
                &context.camera,
                &context.frame,
            )?;
            let camera = crate::projection::Camera::parse(&fitted)?;
            let after = crate::alignment::project(
                &self.document.mesh,
                &context.resource,
                &camera,
                &context.frame,
                self.document.revision(),
            )?;
            let mut view = context.view;
            view.as_object_mut().unwrap().remove("target");
            for key in [
                "yaw",
                "pitch",
                "zoom",
                "center",
                "width",
                "height",
                "projection",
                "position",
                "fov",
                "near",
                "lens_shift",
            ] {
                if let Some(value) = fitted.get(key) {
                    view[key] = value.clone();
                }
            }
            let mut result = workflow.update_reference(id, Some(view), None)?;
            result["camera_only"] = json!(true);
            result["before_rms_error_pixels"] = before["rms_error_pixels"].clone();
            result["after_rms_error_pixels"] = after["rms_error_pixels"].clone();
            result["fit_names"] = request["names"].clone();
            result["fit_diagnostics"] = fitted["fit_diagnostics"].clone();
            result["note"] = json!(if camera.perspective() {
                "Local perspective camera fit: position,yaw,pitch and vertical fov. Residuals cover all projectable bound landmarks; image agreement does not establish hidden geometry."
            } else {
                "Uniform scale and framing only; yaw/pitch remain explicit. Residuals cover all bound landmarks; image agreement does not establish depth."
            });
            result
        } else {
            return Err("unknown reference command".into());
        };
        workflow.save(self.workflow_directory())?;
        self.workflow = workflow;
        result["revision"] = json!(self.document.revision());
        Ok(result)
    }

    fn write(&self, path: &str, value: &impl Serialize) -> std::io::Result<()> {
        let mut destination = self.directory().join(path);
        if self.compress_recordings && (path.starts_with("states/") || path.starts_with("events/")) {
            destination.set_extension("json.gz");
        }
        Self::write_json(destination, value)
    }

    fn write_json(destination: PathBuf, value: &impl Serialize) -> std::io::Result<()> {
        let temporary = destination.with_extension("tmp");
        {
            let mut writer = BufWriter::new(fs::File::create(&temporary)?);
            if destination.extension().is_some_and(|ext| ext == "gz") {
                let mut gzip = flate2::write::GzEncoder::new(&mut writer, flate2::Compression::fast());
                {
                    let mut buffered = BufWriter::new(&mut gzip);
                    serde_json::to_writer(&mut buffered, value)?;
                    buffered.flush()?;
                }
                gzip.finish()?;
            } else {
                serde_json::to_writer(&mut writer, value)?;
            }
            writer.flush()?;
        }
        fs::rename(temporary, destination)
    }

    fn write_workspace_project(&self, path: PathBuf) -> std::io::Result<()> {
        let project = WorkspaceProject {
            version: self.document.project_version(),
            mesh: &self.state["mesh"],
            revision: &self.state["revision"],
        };
        Self::write_json(path, &project)
    }

    pub fn state(&self) -> Value {
        self.state.clone()
    }

    pub fn revision(&self) -> u64 {
        self.document.revision()
    }

    pub fn workspace(&self) -> Option<&std::path::Path> {
        self.workspace.as_deref()
    }

    fn index(&self) -> Value {
        json!({"id":self.id,"events":self.events,"initial_snapshot":0})
    }

    fn query_record(&self, request: &Value) -> Result<Value, String> {
        let session = request["session"].as_str().ok_or("session is required")?;
        let event = request["event"].as_u64().ok_or("event is required")?;
        let path = request["path"]
            .as_str()
            .ok_or("path must be a JSON pointer relative to the original result")?;
        let record = self.read(&format!("/sessions/{session}/events/{event}"))?;
        let value = record["full_output"]["result"]
            .pointer(path)
            .ok_or("result path not found")?;
        let mut result = responses::slice(value, request)?;
        result["source_revision"] = record["summary"]["revision_after"].clone();
        result["revision"] = self.state["revision"].clone();
        result["source"] = json!({"session":session,"event":event,"path":path});
        Ok(result)
    }

    fn artifacts(&self, result: &mut Value, command: &str, event: usize) -> Result<(), String> {
        let mut artifacts = serde_json::Map::new();
        let mut write = |kind: &str, extension: &str, bytes: Vec<u8>| -> Result<(), String> {
            let directory = self.directory().join("artifacts");
            fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            let path = directory.join(format!("{event}.{extension}"));
            fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
            artifacts.insert(kind.into(), json!({"path":path,"bytes":bytes.len()}));
            Ok(())
        };
        for (field, extension, encoded) in [
            ("png_base64", "png", true),
            ("fbx_base64", "fbx", true),
            ("glb_base64", "glb", true),
            ("stl_base64", "stl", true),
            ("svg", "svg", false),
            ("obj", "obj", false),
        ] {
            if let Some(value) = result.get(field).and_then(Value::as_str) {
                let bytes = if encoded {
                    base64::decode(value)?
                } else {
                    value.as_bytes().to_vec()
                };
                write(extension, extension, bytes)?;
            }
            result.as_object_mut().unwrap().remove(field);
        }
        for key in ["views", "preview_views"] {
            if let Some(views) = result.get_mut(key).and_then(Value::as_object_mut) {
                for (label, view) in views {
                    if let Some(encoded) = view["png_base64"].as_str() {
                        write(label, &format!("{label}.png"), base64::decode(encoded)?)?;
                    }
                    view.as_object_mut().unwrap().remove("png_base64");
                    view.as_object_mut().unwrap().remove("svg");
                }
            }
        }
        if matches!(command,"review_run" | "export_verify") {
            write(
                "report",
                "review.json",
                serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?,
            )?;
        }
        if command == "workflow_report" && let Some(markdown) = result["markdown"].as_str() {
            write("markdown","md",markdown.as_bytes().to_vec())?;
            result.as_object_mut().unwrap().remove("markdown");
        }
        if command == "save" {
            write(
                "project",
                "json",
                serde_json::to_vec(result).map_err(|e| e.to_string())?,
            )?;
            *result = json!({"version":result["version"]});
        }
        if !artifacts.is_empty() {
            result["artifacts"] = Value::Object(artifacts);
        }
        Ok(())
    }

    fn import_texture(&mut self, request: &Value) -> Result<Value, String> {
        let path = request["path"].as_str().ok_or("path is required")?;
        let directory =
            fs::canonicalize(self.workflow_directory()).map_err(|error| error.to_string())?;
        let path = fs::canonicalize(directory.join(path)).map_err(|error| error.to_string())?;
        if !path.starts_with(&directory) {
            return Err("texture PNG must remain inside workspace".into());
        }
        if fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len()
            > 8 * 1024 * 1024
        {
            return Err("texture PNG must not exceed 8 MiB".into());
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let texture = crate::uv::import_png(&bytes)?;
        let mut edit = serde_json::to_value(texture).map_err(|error| error.to_string())?;
        for property in ["filter", "double_sided"] {
            edit.as_object_mut().unwrap().remove(property);
            if let Some(value) = request.get(property) { edit[property] = value.clone(); }
        }
        if let Some(cutoff) = request.get("alpha_cutoff") {
            edit["alpha_cutoff"] = cutoff.clone();
        }
        edit["command"] = json!("texture_define");
        edit["name"] = request["name"].clone();
        if let Some(revision) = request.get("revision") {
            edit["revision"] = revision.clone();
        }
        if let Some(object) = request.get("object") {
            edit["object"] = object.clone();
        }
        self.document.execute(edit)
    }

    fn import_mesh(&mut self, request: &Value, format: &str) -> Result<Value, String> {
        let path = request["path"].as_str().ok_or("path is required")?;
        let directory = fs::canonicalize(self.workflow_directory()).map_err(|e| e.to_string())?;
        let path = fs::canonicalize(directory.join(path)).map_err(|e| e.to_string())?;
        if !path.starts_with(&directory) {
            return Err(format!("{format} must remain inside workspace"));
        }
        let mut edit = request.clone();
        edit["path"] = json!(path);
        self.document.execute(edit)
    }

    pub fn execute(&mut self, request: Value) -> Value {
        let started = Instant::now();
        let before_revision = self.state["revision"].clone();
        let result = if request
            .get("response")
            .is_some_and(|v| v != "compact" && v != "full")
        {
            Err("response must be compact or full".into())
        } else if request["command"] == "help" {
            crate::help::execute(&request)
        } else if request["command"] == "workflow_report" {
            self.workflow_markdown()
        } else if request["command"] == "export_verify" {
            if request.get("path").is_none() {Err("export_verify requires a workspace file path".into())}
            else {self.verify_export(&request,&Value::Null).map(|(receipt,_)|receipt)}
        } else if request["command"] == "texture_import" {
            self.import_texture(&request)
        } else if request["command"] == "import_glb" {
            self.import_mesh(&request, "GLB")
        } else if request["command"] == "import_fbx" {
            self.import_mesh(&request, "FBX")
        } else if request["command"] == "query_record" {
            self.query_record(&request)
        } else if request["command"] == "object_review_check"
            || (request["command"] == "object_set_stage" && matches!(request["stage"].as_str(), Some("refined" | "production_ready")))
        {
            self.execute_object_review(&request)
        } else if request["command"] == "compare_reference" {
            self.compare_reference(&request)
        } else if matches!(
            request["command"].as_str(),
            Some("review_define" | "review_update" | "review_run" | "query_reviews")
        ) {
            self.review_command(&request)
        } else if matches!(
            request["command"].as_str(),
            Some(
                "reference_landmarks"
                    | "reference_view"
                    | "project_reference"
                    | "fit_reference"
                    | "move_in_reference"
            )
        ) {
            self.reference_command(&request)
        } else if request["command"]
            .as_str()
            .is_some_and(|command| command.starts_with("workflow_"))
        {
            self.execute_workflow(&request)
        } else if request["command"] == "export" {
            match self.workflow.gate("export", &self.state["mesh"], self.workflow_directory()) {
                Some(missing) => Ok(missing),
                None => self.document.execute(request.clone()),
            }
        } else {
            self.document.execute(request.clone())
        };
        let duration = started.elapsed().as_secs_f64() * 1000.0;
        let state_changed = before_revision.as_u64() != Some(self.document.revision());
        let mut mesh_edited = false;
        let mut invalidated_review = None;
        let mut affected_clearance = None;
        let changed = if state_changed {
            // Ordinary edits already computed the full inspection. Transaction and workflow
            // replies can have different shapes and still need a fresh snapshot.
            let snapshot = result.as_ref().ok().and_then(|value| {
                if value["revision"].as_u64() != Some(self.document.revision()) {
                    return None;
                }
                let fields = self.state.as_object()?.keys();
                fields.map(|k| Some((k.clone(), value.get(k)?.clone()))).collect::<Option<serde_json::Map<_, _>>>()
            });
            let state = match snapshot {
                Some(fields) => Value::Object(fields),
                None => self.document.execute(json!({"command":"inspect"})).expect("inspect is read-only"),
            };
            let changed = self.state["mesh"] != state["mesh"];
            if changed {
                let previous = self.state["mesh"].as_object().expect("inspect returns a mesh");
                let current = state["mesh"].as_object().expect("inspect returns a mesh");
                let previous_fields = previous.iter().filter(|(key, _)| *key != "selection");
                let current_fields = current.iter().filter(|(key, _)| *key != "selection");
                mesh_edited = previous_fields.ne(current_fields);
                if mesh_edited {
                    invalidated_review = self.workflow.invalidated_saved_review(&self.state["mesh"], &state["mesh"], self.workflow_directory());
                    affected_clearance = self.workflow.affected_clearance(&self.state["mesh"], &state["mesh"]);
                }
            }
            self.state = state;
            changed
        } else {
            false
        };
        self.command_id += 1;
        let id = self.command_id;
        let mut full = match result {
            Ok(value) if value["status"] == "requirements_missing" => {
                json!({"ok":false,"error":"requirements_missing","result":value})
            }
            Ok(mut value) => {
                if request["command"] == "inspect" {
                    value["workflow"] = json!({"command":"workflow_status","policy":"Default requirements apply to every workspace; satisfy or explicitly waive before detail/complete checkpoints and export."});
                }
                let mut reply = json!({"ok":true});
                reply["result"] = value;
                reply
            }
            Err(error) => json!({"ok":false,"error":error}),
        };
        if mesh_edited && full["ok"] == true {
            full["turn_handoff"] = json!({
                "revision": self.state["revision"],
                "workflow_status_command": {"command":"workflow_status"},
                "prompt": "Use the latest receipt at turn end. Summarize changes, verified evidence and remaining uncertainty."
            });
            if let Some(mut review) = invalidated_review {
                review["command"]["revision"] = self.state["revision"].clone();
                full["turn_handoff"]["invalidated_review"] = review;
            }
            if let Some(mut clearance) = affected_clearance {
                clearance["command"]["revision"] = self.state["revision"].clone();
                full["turn_handoff"]["affected_clearance"] = clearance;
            }
        }
        let mut output = full.clone();
        if request["response"] != "full" && output["ok"] == true {
            let command = request["command"].as_str().unwrap_or("");
            let result = &mut output["result"];
            if let Err(error) = self.artifacts(result, command, id) {
                output["artifact_error"] = json!(error);
            }
            let paged_query = command == "project_reference"
                || command == "query_animation"
                || command == "animation_sample"
                || command == "query_uv"
                || command == "query_texture_layers"
                || command == "query_texture_masks"
                || command == "query_texture_faces"
                || command == "query_vertex_colors"
                || command == "inspect_uv"
                || command == "help"
                || command == "query_mesh"
                || command == "query_components"
                || command == "query_constructions"
                || command == "query_boundaries"
                || command == "inspect_defects"
                || command == "query_reviews"
                || command == "query_instance"
                || command == "query_record"
                || command.starts_with("workflow_");
            if !paged_query {
                let result = output["result"].take();
                output["result"] = responses::compact(result);
                if command=="review_run" && let Some(receipt)=full["result"].get("export_verification") {
                    output["result"]["export_verification"]=json!({"status":receipt["status"],"artifact_hash":receipt["artifact_hash"],"bytes":receipt["bytes"],"format":receipt["format"]});
                }
            }
        }
        let evidence = full["ok"] == true && (full["turn_handoff"].get("invalidated_review").is_some()
            || matches!(request["command"].as_str(), Some("capture" | "compare_reference" | "review_run" | "export_verify" | "export")));
        let mut project_saved = self.workspace.as_ref().filter(|_| state_changed).map(|_| false);
        let persistence = (|| -> std::io::Result<()> {
            if state_changed && let Some(workspace) = &self.workspace {
                self.write_workspace_project(workspace.join("project.json"))?;
                project_saved = Some(true);
            }
            if !self.debug_session && !evidence {
                return Ok(());
            }

            if !self.debug_session && self.events.is_empty() {
                self.initialize_recording()?;
            } else if state_changed || !self.debug_session {
                self.write(&format!("states/{id}.json"), &self.state)?;
                self.snapshot = id;
            }

            let trace = json!({"session":self.id,"event":id,"record":format!("/sessions/{}/events/{id}",self.id)});
            output["trace"] = trace;
            let mut summary = json!({"id":id,"command":request["command"],"ok":full["ok"],"timestamp_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),"duration_ms":duration,"request_bytes":request.to_string().len(),"response_bytes":output.to_string().len(),"full_response_bytes":full.to_string().len(),"revision_before":before_revision,"revision_after":self.state["revision"],"changed":changed,"snapshot":self.snapshot});
            if let Some(invalidated)=output["turn_handoff"].get("invalidated_review") {
                summary["invalidated_review"]=invalidated.clone();
            }
            let event = RecordedEvent {
                summary: &summary,
                input: &request,
                output: &output,
                full_output: &full,
            };
            self.write(&format!("events/{id}.json"), &event)?;
            self.events.push(summary);
            if let Err(error) = self.write("index.json", &self.index()) {
                self.events.pop();
                return Err(error);
            }
            Ok(())
        })();
        if let Err(error) = persistence {
            output.as_object_mut().expect("reply is an object").remove("turn_handoff");
            output.as_object_mut().unwrap().remove("trace");
            output["recording_error"] = json!(error.to_string());
            output["persistence"] = json!({
                "revision": self.state["revision"],
                "revision_changed_in_memory": state_changed,
                "project_saved": project_saved
            });
        }
        output
    }

    pub fn read(&self, route: &str) -> Result<Value, String> {
        let active = (self.debug_session || !self.events.is_empty()).then_some((self.id.as_str(), !self.events.is_empty()));
        Self::read_recordings(&self.root, self.workspace(), active, route)
    }

    pub fn read_saved_recordings(workspace: &Path, route: &str) -> Result<Value, String> {
        let root = workspace.join("sessions");
        let root = match fs::canonicalize(&root) {
            Ok(resolved) if resolved.starts_with(workspace) => resolved,
            Ok(_) => return Err("recordings directory resolves outside the workspace".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => root,
            Err(error) => return Err(error.to_string()),
        };
        Self::read_recordings(&root, Some(workspace), None, route)
    }

    fn read_recordings(
        root: &Path,
        workspace: Option<&Path>,
        active: Option<(&str, bool)>,
        route: &str,
    ) -> Result<Value, String> {
        if route == "/sessions/workspace" {
            let listing = Self::read_recordings(root, workspace, active, "/sessions")?;
            let mut events = vec![];
            let mut initial_session = listing["current"].as_str();
            for id in listing["sessions"].as_array().unwrap().iter().rev() {
                let id = id.as_str().unwrap();
                let index =
                    Self::read_recordings(root, workspace, active, &format!("/sessions/{id}"))?;
                for event in index["events"].as_array().ok_or("invalid session index")? {
                    if events.is_empty() {
                        initial_session = Some(id);
                    }
                    let mut event = event.clone();
                    event["session"] = json!(id);
                    event["source_event"] = event["id"].clone();
                    event["id"] = json!(events.len() + 1);
                    events.push(event);
                }
            }
            return Ok(json!({"id":"workspace","initial_session":initial_session,"events":events}));
        }
        if route == "/sessions" {
            let entries = match fs::read_dir(root) {
                Ok(entries) => Some(entries),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && active.is_none() => {
                    None
                }
                Err(error) => return Err(error.to_string()),
            };
            let mut ids: Vec<String> = entries
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().join("index.json").is_file())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|id| Self::valid_session_id(id))
                .collect();
            ids.sort();
            ids.reverse();
            let current = active
                .map(|(id, _)| id)
                .or_else(|| ids.first().map(String::as_str));
            let preferred = if !active.is_some_and(|(_, has_events)| has_events) {
                ids.iter()
                    .find(|id| {
                        let index = Self::read_recording_file(
                            root,
                            root.join(id).join("index.json"),
                            active.is_none(),
                        )
                        .ok();
                        index
                            .and_then(|value| {
                                value["events"].as_array().map(|events| !events.is_empty())
                            })
                            .unwrap_or(false)
                    })
                    .map(String::as_str)
                    .or(current)
            } else {
                current
            };
            return Ok(
                json!({"current":current,"preferred":preferred,"sessions":ids,"workspace":workspace,"timeline":current.map(|_| "workspace")}),
            );
        }
        let parts: Vec<_> = route.trim_start_matches('/').split('/').collect();
        let path = match parts.as_slice() {
            ["sessions", id] if Self::valid_session_id(id) => root.join(id).join("index.json"),
            ["sessions", id, kind, number]
                if (*kind == "events" || *kind == "states")
                    && Self::valid_session_id(id)
                    && number.parse::<usize>().is_ok() =>
            {
                root.join(id).join(kind).join(format!("{number}.json"))
            }
            _ => return Err("unknown session route".into()),
        };
        Self::read_recording_file(root, path, active.is_none())
    }

    fn read_recording_file(root: &Path, path: PathBuf, read_only: bool) -> Result<Value, String> {
        let compressed = path.with_extension("json.gz");
        let path = if !path.exists() && compressed.is_file() { compressed } else { path };
        let path = if read_only {
            let resolved = fs::canonicalize(path).map_err(|error| error.to_string())?;
            if !resolved.starts_with(root) {
                return Err("recording resolves outside the workspace recordings directory".into());
            }
            resolved
        } else {
            path
        };
        if path.extension().is_some_and(|ext| ext == "gz") {
            let file = fs::File::open(path).map_err(|e| e.to_string())?;
            let gzip = flate2::read::GzDecoder::new(file);
            return serde_json::from_reader(std::io::BufReader::new(gzip)).map_err(|e| e.to_string());
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    fn valid_session_id(id: &str) -> bool {
        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit() || byte == b'-')
    }
}
