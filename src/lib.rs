use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

mod alignment;
mod animated_glb;
mod appearance_tools;
pub mod base64;
mod camera_fit;
mod cleanup;
mod collections;
mod comparison;
mod connected;
mod construction;
mod diagnostics;
mod fbx;
mod fbx_import;
mod geometry;
mod glb;
mod help;
mod inspection;
mod instances;
mod material;
mod modeling;
mod normals;
mod object_review;
mod objects;
mod paint_masks;
mod painting;
mod profiles;
mod projection;
mod regions;
mod render;
mod rig;
pub mod responses;
pub mod session;
mod shaping;
mod stl;
mod surface;
mod surface_ops;
mod surface_paint;
mod sweep;
mod topology;
mod transactions;
mod patches;
mod uv;
mod vertex_colors;
mod workflow;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Mesh {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rig: Option<rig::Rig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub constructions: BTreeMap<String, modeling::Construction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intersection_allowances: Vec<diagnostics::Allowance>,
    #[serde(skip)]
    defer_validation: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub components: BTreeMap<u64, construction::Component>,
    #[serde(default, skip_serializing_if = "crate::construction::is_zero")]
    component_id_high_watermark: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vertex_colors: BTreeMap<u64, [f64; 3]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub uvs: BTreeMap<u64, BTreeMap<u64, [f64; 2]>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub textures: BTreeMap<String, uv::Texture>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub face_textures: BTreeMap<u64, String>,
    pub vertices: BTreeMap<u64, [f64; 3]>,
    pub faces: BTreeMap<u64, Vec<u64>>,
    pub selection: BTreeSet<u64>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub flat_faces: BTreeSet<u64>,
    #[serde(default)]
    pub objects: BTreeMap<u64, objects::MeshObject>,
    #[serde(default)]
    object_id_high_watermark: u64,
    #[serde(default)]
    pub collections: BTreeMap<u64, collections::Collection>,
    #[serde(default)]
    pub primary_collections: BTreeMap<u64, u64>,
    #[serde(default)]
    collection_id_high_watermark: u64,
    #[serde(default)]
    vertex_id_high_watermark: u64,
    #[serde(default)]
    face_id_high_watermark: u64,
}

impl Mesh {
    fn add_vertex(&mut self, point: [f64; 3]) -> Result<u64, String> {
        let maximum = self.vertices.keys().next_back().copied().unwrap_or(0);
        let id = self
            .vertex_id_high_watermark
            .max(maximum)
            .checked_add(1)
            .ok_or("vertex IDs exhausted")?;
        self.vertex_id_high_watermark = id;
        self.vertices.insert(id, point);
        Ok(id)
    }

    fn add_face(&mut self, vertices: Vec<u64>) -> Result<u64, String> {
        let maximum = self.faces.keys().next_back().copied().unwrap_or(0);
        let id = self
            .face_id_high_watermark
            .max(maximum)
            .checked_add(1)
            .ok_or("face IDs exhausted")?;
        self.face_id_high_watermark = id;
        self.faces.insert(id, vertices);
        Ok(id)
    }
}

#[derive(Clone)]
struct View {
    revision: u64,
    points: BTreeMap<u64, [f64; 3]>,
    camera: projection::Camera,
    visibility: render::CaptureVisibility,
}

pub struct Document {
    mesh: Mesh,
    revision: u64,
    undo: Vec<Mesh>,
    redo: Vec<Mesh>,
    views: BTreeMap<u64, View>,
    next_view: u64,
    transaction_preview: Option<transactions::Prepared>,
}

impl Default for Document {
    fn default() -> Self {
        Self { mesh: cube(), ..Self::empty() }
    }
}

impl Document {
    pub fn empty() -> Self {
        Self {
            mesh: Mesh::default(),
            revision: 0,
            undo: vec![],
            redo: vec![],
            views: BTreeMap::new(),
            next_view: 1,
            transaction_preview: None,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn project_version(&self) -> u64 {
        if self.mesh.textures.values().any(paint_masks::has_surface_masks) {
            return 8;
        }
        if self.mesh.rig.is_some() {
            return 7;
        }
        if !self.mesh.components.is_empty()
            || self.mesh.component_id_high_watermark != 0
            || !self.mesh.intersection_allowances.is_empty()
            || !self.mesh.constructions.is_empty()
        {
            return 6;
        }
        if self.mesh.textures.values().any(paint_masks::extended) {
            return 5;
        }
        if self
            .mesh
            .textures
            .values()
            .any(|texture| texture.paint.is_some() || !texture.maps.is_empty())
        {
            return 4;
        }
        let no_appearance = self.mesh.uvs.is_empty()
            && self.mesh.textures.is_empty()
            && self.mesh.face_textures.is_empty()
            && self.mesh.vertex_colors.is_empty();
        if no_appearance { 2 } else { 3 }
    }

    pub fn execute(&mut self, request: Value) -> Result<Value, String> {
        self.execute_edit(request, false)
    }

    pub(crate) fn execute_edit(&mut self, request: Value, reviewed: bool) -> Result<Value, String> {
        let request = modeling::resolve_request(&self.mesh, request)?;
        let request = self.collection_request(request)?;
        let command = request["command"].as_str().ok_or("command is required")?;
        let needs_review = command == "object_review_check"
            || (command == "object_set_stage" && matches!(request["stage"].as_str(), Some("refined" | "production_ready")));
        if needs_review && !reviewed {
            return Err("run review checks and production approval through the workspace CLI".into());
        }
        if let Some(expected) = request["revision"].as_u64()
            && expected != self.revision
        {
            return Err("stale document revision".into());
        }

        match command {
            "query_object_review" => {
                let id = request["object"].as_u64().ok_or("object is required")?;
                let object = self.mesh.objects.get(&id).ok_or("unknown object")?;
                Ok(json!({"object":id,"stage":object.stage,"checks":object.review}))
            }
            "transaction" => transactions::execute(self, &request),
            "inspect" => Ok(self.inspect()),
            "query_components" => construction::query(&self.mesh, &request, self.revision),
            "query_constructions" => responses::slice(&json!(self.mesh.constructions), &request),
            "query_boundaries" => {
                surface_ops::query_boundaries(&self.mesh, &request, self.revision)
            }
            "inspect_defects" => diagnostics::inspect(&self.mesh, &request, self.revision),
            "measure_clearance" => diagnostics::clearance(&self.mesh, &request),
            "inspect_volume" => diagnostics::volume(&self.mesh, &request),
            "query_uv" => uv::query(&self.mesh, &request, self.revision),
            "query_texture_faces" => surface_paint::query(&self.mesh, &request, self.revision),
            "query_texture_layers" => painting::query(&self.mesh, &request, self.revision),
            "query_texture_masks" => paint_masks::query(&self.mesh, &request, self.revision),
            "texture_mask_capture" => painting::capture(&self.mesh, &request),
            "texture_layer_capture" => painting::capture(&self.mesh, &request),
            "query_vertex_colors" => vertex_colors::query(&self.mesh, &request, self.revision),
            "inspect_uv" => uv::inspect(&self.mesh, &request, self.revision),
            "capture_uv" => uv::capture(&self.mesh, &request, self.revision),
            "texture_export" => uv::export_texture(&self.mesh, &request),
            "query_mesh" => responses::query_mesh(&self.mesh, &request, self.revision),
            "query_rig" => Ok(rig::query(&self.mesh, &request)),
            "query_animation" => rig::query_animation(&self.mesh, &request, self.revision),
            "animation_sample" => rig::sample_query(&self.mesh, &request, self.revision),
            "query_instance" => instances::query(&self.mesh, &request, self.revision),
            "inspect_region" => objects::region(&self.mesh, &request, self.revision),
            "inspect_profile" => profiles::inspect(&self.mesh, &request, self.revision),
            "inspect_loop" => shaping::inspect_loop(&self.mesh, &request),
            "inspect_shading" => regions::inspect(&self.mesh, &request, self.revision),
            "inspect_patch" => patches::inspect(&self.mesh, &request, self.revision),
            "capture" => self.capture(&request),
            "export" => {
                if self.mesh.faces.is_empty() {
                    return Err("cannot export an empty scene; create or import geometry first".into());
                }
                let format = request["format"].as_str().unwrap_or("obj");
                let mut result = match format {
                    "obj" => json!({"obj": self.export_obj()}),
                    "fbx" => {
                        json!({"fbx_base64": base64::encode(&fbx::export(&self.mesh)?),"format":"fbx","version":7400,"unit":"meter","up_axis":"Y"})
                    }
                    "glb" => {
                        json!({"glb_base64": base64::encode(&animated_glb::export(&self.mesh)?),"format":"glb","version":2,"unit":"meter","up_axis":"Y"})
                    }
                    "stl" => {
                        json!({"stl_base64": base64::encode(&stl::export(&self.mesh)?),"format":"stl","unit":"millimeter","up_axis":"Z"})
                    }
                    _ => return Err("unsupported export format".into()),
                };
                let mut warnings = Vec::new();
                if format == "stl" {
                    warnings.push("STL contains triangle geometry only. Import using millimeters.");
                } else if self.mesh.face_textures.iter().any(|(face, name)| {
                    format != "glb" || self.mesh.textures[name].alpha_cutoff.is_none() || !uv::complete(&self.mesh, *face)
                }) {
                    warnings.push("UV coordinates and vertex colors are exported. Texture image bindings remain in the native project; export images separately with texture_export and bind them in the consuming application.");
                }
                if self.mesh.rig.is_some() && format != "glb" {
                    warnings.push("Export contains rest geometry. Bones, skin weights and clips remain in the native project.");
                }
                if !warnings.is_empty() {
                    result["warnings"] = json!(warnings);
                }
                Ok(result)
            }
            "save" => Ok(json!({"version": self.project_version(), "mesh": self.mesh})),
            "undo" | "redo" => {
                let (source, destination) = if command == "undo" {
                    (&mut self.undo, &mut self.redo)
                } else {
                    (&mut self.redo, &mut self.undo)
                };
                let previous = source.pop().ok_or("history is empty")?;
                destination.push(self.mesh.clone());
                self.mesh = previous;
                self.transaction_preview = None;
                self.revision += 1;
                Ok(self.inspect())
            }
            _ => {
                let previous = self.mesh.clone();
                let operation = match self.edit(command, &request) {
                    Ok(operation) => operation,
                    Err(error) => {
                        self.mesh = previous;
                        return Err(error);
                    }
                };
                if !matches!(command, "load" | "cube" | "object_separate_faces") {
                    objects::inherit(&previous, &mut self.mesh);
                    self.mesh
                        .flat_faces
                        .retain(|id| self.mesh.faces.contains_key(id));
                }
                if command != "load" {
                    uv::reconcile(&mut self.mesh);
                    if command != "replace_patch" {
                        construction::inherit(&previous, &mut self.mesh);
                    }
                    construction::reconcile(&mut self.mesh);
                }
                let validation = objects::reconcile(&mut self.mesh)
                    .and_then(|()| objects::check_scope(&previous, &self.mesh, &request))
                    .and_then(|()| instances::finish(&previous, &mut self.mesh, &request))
                    .and_then(|()| {
                        if command != "load" {
                            uv::reconcile(&mut self.mesh);
                            rig::reconcile(&mut self.mesh);
                        }
                        Ok(())
                    })
                    .and_then(|()| objects::reconcile(&mut self.mesh))
                    .and_then(|()| collections::validate(&self.mesh))
                    .and_then(|()| construction::validate(&self.mesh))
                    .and_then(|()| modeling::validate(&self.mesh))
                    .and_then(|()| validate(&self.mesh));
                if let Err(error) = validation {
                    self.mesh = previous;
                    return Err(error);
                }
                if !matches!(command, "load" | "cube" | "object_set_stage" | "object_review_plan" | "object_review_check") {
                    objects::refresh_stages(&previous, &mut self.mesh);
                }
                self.undo.push(previous);
                if self.undo.len() > 100 {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.transaction_preview = None;
                self.revision += 1;
                // A transaction reports its final inspection and only consumes each step's operation.
                let mut result = if self.mesh.defer_validation { json!({}) } else { self.inspect() };
                if let Some(operation) = operation {
                    result["operation"] = operation;
                }
                Ok(result)
            }
        }
    }

    fn collection_request(&self, mut request: Value) -> Result<Value, String> {
        let Some(collection) = request.get("collection") else {
            return Ok(request);
        };
        let command = request["command"].as_str().unwrap_or("");
        if command.starts_with("collection_")
            || command == "select_collection"
            || command == "query_mesh"
            || matches!(command, "object_move" | "object_rotate" | "shade_smooth")
        {
            return Ok(request);
        }
        let members = collections::members(&self.mesh, collection)?;
        if !matches!(command, "inspect_region" | "select" | "move_in_view") {
            return Err("collection targeting is supported by query_mesh, inspect_region, select, move_in_view; use select_collection before selection-based edits".into());
        }
        if request.get("object").is_some()
            || request.get("vertices").is_some()
            || request.get("faces").is_some()
        {
            return Err("collection cannot be combined with object, vertices or faces".into());
        }
        let vertices: BTreeSet<u64> = members
            .iter()
            .flat_map(|id| self.mesh.objects[id].vertices.iter().copied())
            .collect();
        request["vertices"] = json!(vertices);
        Ok(request)
    }

    fn edit(&mut self, command: &str, request: &Value) -> Result<Option<Value>, String> {
        if matches!(command, "rig_set" | "rig_clear" | "rig_display_tails_set" | "skin_weights_set" | "animation_set" | "animation_update" | "animation_remove" | "animation_track_set" | "animation_track_remove" | "animation_keys_edit" | "animation_ik_bake") {
            return rig::edit(&mut self.mesh, command, request).map(Some);
        }
        if command != "load" && command != "cube" {
            self.mesh.vertex_id_high_watermark = self
                .mesh
                .vertex_id_high_watermark
                .max(*self.mesh.vertices.keys().next_back().unwrap_or(&0));
            self.mesh.face_id_high_watermark = self
                .mesh
                .face_id_high_watermark
                .max(*self.mesh.faces.keys().next_back().unwrap_or(&0));
            self.mesh.object_id_high_watermark = self
                .mesh
                .object_id_high_watermark
                .max(*self.mesh.objects.keys().next_back().unwrap_or(&0));
        }
        if matches!(
            command,
            "region_define"
                | "region_remove"
                | "select_region"
                | "select_related"
                | "construction_define"
                | "construction_apply"
        ) {
            return modeling::edit(&mut self.mesh, command, request).map(Some);
        }
        if construction::generator(command) {
            return construction::generate(&mut self.mesh, command, request).map(Some);
        }
        if command.starts_with("texture_layer_")
            || command.starts_with("texture_mask_")
            || matches!(
                command,
                "texture_create"
                    | "texture_map_set"
                    | "texture_bake"
                    | "texture_normal_from_height"
            )
        {
            return painting::edit(&mut self.mesh, command, request).map(Some);
        }
        if matches!(
            command,
            "uv_project" | "vertex_color_gradient" | "vertex_color_shade"
        ) {
            return appearance_tools::edit(&mut self.mesh, command, request).map(Some);
        }
        if matches!(command, "vertex_color_set" | "vertex_color_clear") {
            return vertex_colors::edit(&mut self.mesh, command, request).map(Some);
        }
        if matches!(
            command,
            "uv_set"
                | "uv_clear"
                | "uv_transform"
                | "texture_define"
                | "texture_assign"
                | "texture_remove"
        ) {
            return uv::edit(&mut self.mesh, command, request).map(Some);
        }
        if command.starts_with("collection_") || command == "select_collection" {
            return collections::edit(&mut self.mesh, command, request).map(Some);
        }
        if let Some(collection) = request.get("collection")
            && matches!(command, "object_move" | "object_rotate" | "shade_smooth")
        {
            if request.get("object").is_some() {
                return Err("use collection or object, not both".into());
            }
            let members = collections::members(&self.mesh, collection)?;
            if members.is_empty() {
                return Err("collection is empty".into());
            }
            for id in &members {
                let mut scoped = request.clone();
                scoped.as_object_mut().unwrap().remove("collection");
                scoped["object"] = json!(id);
                if command == "shade_smooth" {
                    shaping::edit(&mut self.mesh, command, &scoped)?;
                } else {
                    objects::edit(&mut self.mesh, command, &scoped)?;
                }
            }
            return Ok(Some(
                json!({"collection":collection,"object_count":members.len()}),
            ));
        }
        match command {
            "intersection_allow" => {
                if request.get("faces").is_some() == request.get("regions").is_some() {
                    return Err("provide exactly one of faces or regions".into());
                }
                let mut faces = None;
                let mut regions = None;
                if let Some(value) = request.get("faces") {
                    let mut pair: [u64; 2] = serde_json::from_value(value.clone())
                        .map_err(|_| "faces must contain two face IDs")?;
                    pair.sort();
                    if pair[0] == pair[1] || pair.iter().any(|id| !self.mesh.faces.contains_key(id))
                    {
                        return Err("provide two distinct existing faces".into());
                    }
                    faces = Some(pair);
                }
                if let Some(value) = request.get("regions") {
                    let names: [String; 2] = serde_json::from_value(value.clone())
                        .map_err(|_| "regions must contain two region names")?;
                    let mut pair = [
                        construction::named(&self.mesh, &names[0])?,
                        construction::named(&self.mesh, &names[1])?,
                    ];
                    pair.sort();
                    if pair[0] == pair[1] {
                        return Err("provide two distinct regions".into());
                    }
                    regions = Some(pair);
                }
                let reason = request["reason"]
                    .as_str()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or("specific reason is required")?
                    .to_string();
                self.mesh
                    .intersection_allowances
                    .retain(|allowance| allowance.faces != faces || allowance.regions != regions);
                self.mesh
                    .intersection_allowances
                    .push(diagnostics::Allowance {
                        faces,
                        regions,
                        reason,
                    });
                return Ok(Some(
                    json!({"faces":faces,"regions":regions,"intentional":true}),
                ));
            }
            "bridge_loops" | "create_face" | "dissolve_edges" | "dissolve_region" => {
                return surface_ops::edit(&mut self.mesh, command, request).map(Some);
            }
            "bevel_edges" => return connected::bevel(&mut self.mesh, request).map(Some),
            "relax_surface" => return surface::relax(&mut self.mesh, request).map(Some),
            "improve_triangulation" => return surface::improve(&mut self.mesh, request).map(Some),
            "weld_coincident" => {
                return cleanup::weld_coincident(&mut self.mesh, request).map(Some);
            }
            "object_separate_faces" => return cleanup::separate(&mut self.mesh, request).map(Some),
            "object_join" => return cleanup::join(&mut self.mesh, request).map(Some),
            "delete_faces" => return cleanup::delete_faces(&mut self.mesh, request).map(Some),
            "import_glb" => return glb::import(&mut self.mesh, request).map(Some),
            "import_fbx" => return fbx_import::import(&mut self.mesh, request).map(Some),
            "move_in_view" => {
                let camera = if let Some(id) = request.get("view_id") {
                    let view = self
                        .views
                        .get(&id.as_u64().ok_or("view_id must be an ID")?)
                        .ok_or("unknown or expired view")?;
                    if view.revision != self.revision {
                        return Err("stale view: capture again".into());
                    }
                    view.camera.clone()
                } else {
                    projection::Camera::parse(
                        request
                            .get("camera")
                            .ok_or("camera or view_id is required")?,
                    )?
                };
                let delta = projection::pair(&request["delta_pixels"])?;
                let vertices: BTreeSet<u64> = request
                    .get("vertices")
                    .map(|value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()))
                    .transpose()?
                    .unwrap_or_else(|| self.mesh.selection.clone());
                if vertices.is_empty() {
                    return Err("selection is empty".into());
                }
                if vertices
                    .iter()
                    .any(|id| !self.mesh.vertices.contains_key(id))
                {
                    return Err("unknown vertex".into());
                }
                let anchor = request
                    .get("anchor")
                    .map(vector)
                    .transpose()?
                    .unwrap_or_else(|| {
                        std::array::from_fn(|axis| {
                            vertices
                                .iter()
                                .map(|id| self.mesh.vertices[id][axis] / vertices.len() as f64)
                                .sum()
                        })
                    });
                let displacement = camera.displacement(delta, anchor)?;
                for id in &vertices {
                    let point = self.mesh.vertices.get_mut(id).unwrap();
                    for axis in 0..3 {
                        point[axis] += displacement[axis];
                    }
                }
                return Ok(Some(
                    json!({"vertices":vertices,"delta_pixels":delta,"displacement":displacement,"depth_preserved":true,"anchor":anchor}),
                ));
            }
            "cube" => self.mesh = cube(),
            "load" => {
                if request["project"]["version"] != 1
                    && request["project"]["version"] != 2
                    && request["project"]["version"] != 3
                    && request["project"]["version"] != 4
                    && request["project"]["version"] != 5
                    && request["project"]["version"] != 6
                    && request["project"]["version"] != 7
                    && request["project"]["version"] != 8
                {
                    return Err("unsupported project version".into());
                }
                self.mesh = serde_json::from_value(request["project"]["mesh"].clone())
                    .map_err(|e| e.to_string())?;
                objects::normalize_legacy_stages(&mut self.mesh);
                if request["project"]["version"] == 1 {
                    objects::initialize(&mut self.mesh);
                }
                uv::validate(&self.mesh)?;
                vertex_colors::validate(&self.mesh)?;
            }
            "select" => {
                let ids: BTreeSet<u64> = serde_json::from_value(request["vertices"].clone())
                    .map_err(|e| e.to_string())?;
                if ids.iter().any(|id| !self.mesh.vertices.contains_key(id)) {
                    return Err("unknown vertex".into());
                }
                self.mesh.selection = ids;
            }
            "select_box" => {
                let min = vector(&request["min"])?;
                let max = vector(&request["max"])?;
                self.mesh.selection = self
                    .mesh
                    .vertices
                    .iter()
                    .filter(|(_, p)| (0..3).all(|i| p[i] >= min[i] && p[i] <= max[i]))
                    .map(|(id, _)| *id)
                    .collect();
            }
            "select_image" => {
                let id = request["view_id"].as_u64().ok_or("view_id is required")?;
                let view = self.views.get(&id).ok_or("unknown or expired view")?;
                if view.revision != self.revision {
                    return Err("stale view: capture again".into());
                }
                let rect: [f64; 4] = serde_json::from_value(request["rect"].clone())
                    .map_err(|_| "rect must be [left, top, right, bottom]")?;
                if rect.iter().any(|v| !v.is_finite()) || rect[0] > rect[2] || rect[1] > rect[3] {
                    return Err("invalid rectangle".into());
                }
                let through = request["through"].as_bool().unwrap_or(false);
                self.mesh.selection = view
                    .points
                    .iter()
                    .filter(|(id, p)| {
                        p[0] >= rect[0]
                            && p[0] <= rect[2]
                            && p[1] >= rect[1]
                            && p[1] <= rect[3]
                            && (through || visible(**id, view, &self.mesh))
                    })
                    .map(|(id, _)| *id)
                    .collect();
            }
            "move" | "scale" | "rotate" => {
                if self.mesh.selection.is_empty() {
                    return Err("selection is empty".into());
                }
                let amount = vector(&request["value"])?;
                let mut center = [0.0; 3];
                for id in &self.mesh.selection {
                    for (i, value) in center.iter_mut().enumerate() {
                        *value += self.mesh.vertices[id][i] / self.mesh.selection.len() as f64;
                    }
                }
                for id in &self.mesh.selection {
                    let point = self.mesh.vertices.get_mut(id).unwrap();
                    if command == "move" {
                        for i in 0..3 {
                            point[i] += amount[i];
                        }
                    }
                    if command == "scale" {
                        for i in 0..3 {
                            point[i] = center[i] + (point[i] - center[i]) * amount[i];
                        }
                    }
                    if command == "rotate" {
                        let mut local = [
                            point[0] - center[0],
                            point[1] - center[1],
                            point[2] - center[2],
                        ];
                        for (axis, angle) in amount.iter().enumerate() {
                            let a = (axis + 1) % 3;
                            let b = (axis + 2) % 3;
                            let (sin, cos) = angle.to_radians().sin_cos();
                            let first = local[a] * cos - local[b] * sin;
                            local[b] = local[a] * sin + local[b] * cos;
                            local[a] = first;
                        }
                        for i in 0..3 {
                            point[i] = center[i] + local[i];
                        }
                    }
                }
            }
            "extrude" => {
                let face_id = request["face"].as_u64().ok_or("face is required")?;
                let face = self.mesh.faces.get(&face_id).ok_or("unknown face")?.clone();
                let offset = vector(&request["value"])?;
                let mut cap = vec![];
                for id in &face {
                    let p = self.mesh.vertices[id];
                    let next_vertex = self.mesh.add_vertex([
                        p[0] + offset[0],
                        p[1] + offset[1],
                        p[2] + offset[2],
                    ])?;
                    cap.push(next_vertex);
                }
                self.mesh.faces.insert(face_id, cap.clone());
                for i in 0..face.len() {
                    let j = (i + 1) % face.len();
                    self.mesh.add_face(vec![face[i], face[j], cap[j], cap[i]])?;
                }
                self.mesh.selection = cap.into_iter().collect();
            }
            "weld" | "split_edge" | "split_vertices" | "split_face" | "inset" | "bevel_vertex"
            | "bridge_faces" => {
                let operation = topology::edit(&mut self.mesh, command, request)?;
                return Ok(Some(operation));
            }
            "object_create_box"
            | "object_add_box"
            | "object_delete"
            | "object_delete_components"
            | "object_set_origin"
            | "object_set_stage"
            | "object_review_plan"
            | "object_review_check"
            | "object_move"
            | "object_rotate"
            | "object_rename"
            | "object_instance"
            | "object_link_instance"
            | "object_make_unique"
            | "object_duplicate"
            | "select_object" => {
                return objects::edit(&mut self.mesh, command, request).map(Some);
            }
            "object_profile" => return profiles::create(&mut self.mesh, request).map(Some),
            "replace_patch" => return patches::replace(&mut self.mesh, request).map(Some),
            "shade_region" | "retriangulate_region" | "inset_region" => {
                return regions::edit(&mut self.mesh, command, request).map(Some);
            }
            "taper" | "sweep" | "revolve" | "extrude_polygon" | "bisect_plane" | "triangulate"
            | "select_loop" | "radial_offset" | "radial_profile" | "shade_smooth" => {
                return shaping::edit(&mut self.mesh, command, request).map(Some);
            }
            _ => return Err(format!("unknown command: {command}")),
        }
        Ok(None)
    }

    fn inspect(&self) -> Value {
        let mut adjacency: BTreeMap<u64, BTreeSet<u64>> = self
            .mesh
            .vertices
            .keys()
            .map(|id| (*id, BTreeSet::new()))
            .collect();
        let mut edges = BTreeMap::<(u64, u64), usize>::new();
        for face in self.mesh.faces.values() {
            for i in 0..face.len() {
                let a = face[i];
                let b = face[(i + 1) % face.len()];
                adjacency.get_mut(&a).unwrap().insert(b);
                adjacency.get_mut(&b).unwrap().insert(a);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let mut topology = diagnostics::topology(&self.mesh);
        topology.as_object_mut().unwrap().remove("components");
        topology.as_object_mut().unwrap().remove("edge_findings");
        let branching_count = topology["branching_boundary_vertices"]
            .as_array()
            .unwrap()
            .len();
        topology["branching_boundary_vertex_count"] = json!(branching_count);
        topology
            .as_object_mut()
            .unwrap()
            .remove("branching_boundary_vertices");
        json!({"topology":topology,"geometry":{"self_intersections":"not_checked","inter_object_intersections":"not_checked","duplicate_surfaces":"not_checked","coplanar_overlaps":"not_checked"},"revision": self.revision, "mesh": self.mesh, "adjacency": adjacency, "edges": edges.keys().collect::<Vec<_>>(), "validation": {"boundary_edges": edges.values().filter(|n| **n == 1).count(), "non_manifold_edges": edges.values().filter(|n| **n > 2).count(), "self_intersections_checked": false}, "history": {"undo": self.undo.len(), "redo": self.redo.len()}})
    }

    fn render_view(&self, request: &Value) -> Result<(Value, View), String> {
        self.render_mesh_view(&self.mesh, request)
    }

    fn render_mesh_view(&self, mesh: &Mesh, request: &Value) -> Result<(Value, View), String> {
        let options = render::Options::parse(request)?;
        let camera = projection::Camera::parse(request)?;
        let visibility = render::CaptureVisibility::parse(request, mesh)?;
        let mut view = View {
            revision: self.revision,
            points: BTreeMap::new(),
            camera: camera.clone(),
            visibility,
        };
        for (id, p) in &mesh.vertices {
            if view.visibility.includes_vertex(id) && camera.contains(*p) {
                let point = camera.raster_point(*p)?;
                view.points.insert(*id, point);
            }
        }
        let mut result = render::capture(mesh, &view, &options)?;
        result["visibility"] = view.visibility.metadata();
        result["revision"] = json!(self.revision);
        result["width"] = json!(options.width);
        result["height"] = json!(options.height);
        result["camera"] = camera.metadata();
        result["objects"] = json!(mesh.objects);
        Ok((result, view))
    }

    fn capture(&mut self, request: &Value) -> Result<Value, String> {
        if request.get("clip").is_some() || request.get("time").is_some() {
            let name = request["clip"].as_str().ok_or("pose capture requires a clip name")?;
            let time = request["time"].as_f64().filter(|time| time.is_finite())
                .ok_or("pose capture requires a finite time in seconds")?;
            let (posed, sampled_time) = rig::sample_pose(&self.mesh, name, time)?;
            let (mut result, _) = self.render_mesh_view(&posed, request)?;
            result["animation"] = json!({"clip": name,"time": time,"sampled_time": sampled_time});
            return Ok(result);
        }

        let (mut result, view) = self.render_view(request)?;
        let id = self.next_view;
        self.next_view += 1;
        self.views.insert(id, view);
        if self.views.len() > 32 {
            let oldest = *self.views.keys().next().unwrap();
            self.views.remove(&oldest);
        }
        result["view_id"] = json!(id);
        Ok(result)
    }

    fn export_obj(&self) -> String {
        let mut output = String::new();
        let mut indices = BTreeMap::new();
        for (index, (id, p)) in self.mesh.vertices.iter().enumerate() {
            indices.insert(id, index + 1);
            output.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
            if let Some(color) = self.mesh.vertex_colors.get(id) {
                output.pop();
                output.push_str(&format!(" {} {} {}\n", color[0], color[1], color[2]));
            }
        }
        let mut uv_indices = BTreeMap::new();
        for (face, values) in &self.mesh.uvs {
            for (vertex, uv) in values {
                uv_indices.insert((*face, *vertex), uv_indices.len() + 1);
                output.push_str(&format!("vt {} {}\n", uv[0], uv[1]));
            }
        }
        for object in self.mesh.objects.values() {
            output.push_str(&format!(
                "o {}\n",
                object.name.replace(char::is_whitespace, "_")
            ));
            for face in self
                .mesh
                .faces
                .iter()
                .filter(|(_, face)| object.vertices.contains(&face[0]))
            {
                let (face_id, face) = face;
                output.push('f');
                for id in face {
                    output.push_str(&format!(" {}", indices[id]));
                    if let Some(uv) = uv_indices.get(&(*face_id, *id)) {
                        output.push_str(&format!("/{uv}"));
                    }
                }
                output.push('\n');
            }
        }
        output
    }
}

fn vector(value: &Value) -> Result<[f64; 3], String> {
    let result: [f64; 3] =
        serde_json::from_value(value.clone()).map_err(|_| "value must contain three numbers")?;
    if result.iter().any(|v| !v.is_finite()) {
        return Err("numbers must be finite".into());
    }
    Ok(result)
}

fn cube() -> Mesh {
    let points = [
        [-1., -1., -1.],
        [1., -1., -1.],
        [1., 1., -1.],
        [-1., 1., -1.],
        [-1., -1., 1.],
        [1., -1., 1.],
        [1., 1., 1.],
        [-1., 1., 1.],
    ];
    let faces = [
        vec![1, 4, 3, 2],
        vec![5, 6, 7, 8],
        vec![1, 2, 6, 5],
        vec![4, 8, 7, 3],
        vec![1, 5, 8, 4],
        vec![2, 3, 7, 6],
    ];
    let mut mesh = Mesh {
        rig: None,
        constructions: BTreeMap::new(),
        intersection_allowances: vec![],
        defer_validation: false,
        components: BTreeMap::new(),
        component_id_high_watermark: 0,
        vertex_colors: BTreeMap::new(),
        uvs: BTreeMap::new(),
        textures: BTreeMap::new(),
        face_textures: BTreeMap::new(),
        vertices: points
            .into_iter()
            .enumerate()
            .map(|(i, p)| (i as u64 + 1, p))
            .collect(),
        faces: faces
            .into_iter()
            .enumerate()
            .map(|(i, f)| (i as u64 + 1, f))
            .collect(),
        selection: BTreeSet::new(),
        flat_faces: BTreeSet::new(),
        objects: BTreeMap::new(),
        object_id_high_watermark: 0,
        collections: BTreeMap::new(),
        primary_collections: BTreeMap::new(),
        collection_id_high_watermark: 0,
        vertex_id_high_watermark: 8,
        face_id_high_watermark: 6,
    };
    objects::initialize(&mut mesh);
    mesh
}

fn validate(mesh: &Mesh) -> Result<(), String> {
    rig::validate(mesh)?;
    vertex_colors::validate(mesh)?;
    uv::validate(mesh)?;
    if mesh
        .flat_faces
        .iter()
        .any(|id| !mesh.faces.contains_key(id))
    {
        return Err("flat shading references an unknown face".into());
    }
    if !mesh.defer_validation && mesh.vertices.is_empty() != mesh.faces.is_empty() {
        return Err("a nonempty scene must contain vertices and faces".into());
    }
    if mesh.vertices.values().flatten().any(|v| !v.is_finite()) {
        return Err("non-finite vertex".into());
    }
    if mesh
        .selection
        .iter()
        .any(|id| !mesh.vertices.contains_key(id))
    {
        return Err("invalid selection".into());
    }
    for face in mesh.faces.values() {
        let unique: BTreeSet<_> = face.iter().collect();
        if face.len() < 3
            || unique.len() != face.len()
            || face.iter().any(|id| !mesh.vertices.contains_key(id))
        {
            return Err("invalid face references".into());
        }
        let origin = mesh.vertices[&face[0]];
        let mut area = [0.0; 3];
        let mut scale_squared = 0.0_f64;
        for vertex in face {
            let point = mesh.vertices[vertex];
            let distance_squared: f64 = (0..3)
                .map(|axis| (point[axis] - origin[axis]).powi(2))
                .sum();
            scale_squared = scale_squared.max(distance_squared);
        }
        for i in 1..face.len() - 1 {
            let a = mesh.vertices[&face[i]];
            let b = mesh.vertices[&face[i + 1]];
            for (axis, component) in area.iter_mut().enumerate() {
                let j = (axis + 1) % 3;
                let k = (axis + 2) % 3;
                *component += (a[j] - origin[j]) * (b[k] - origin[k])
                    - (a[k] - origin[k]) * (b[j] - origin[j]);
            }
        }
        let area_squared = area.iter().map(|v| v * v).sum::<f64>();
        if !area_squared.is_finite()
            || scale_squared == 0.0
            || area_squared <= scale_squared.powi(2) * 1e-24
        {
            return Err("edit would create a degenerate face".into());
        }
    }
    Ok(())
}

fn visible(id: u64, view: &View, mesh: &Mesh) -> bool {
    let p = view.points[&id];
    for face in mesh.faces.values() {
        if !view.visibility.includes_vertex(&face[0]) {
            continue;
        }
        for [i0, i1, i2] in normals::triangles(mesh, face) {
            let triangles = view.camera.clip_triangle([
                mesh.vertices[&face[i0]],
                mesh.vertices[&face[i1]],
                mesh.vertices[&face[i2]],
            ]);
            for triangle in triangles {
                let Ok(a) = view.camera.raster_point(triangle[0].position) else {
                    continue;
                };
                let Ok(b) = view.camera.raster_point(triangle[1].position) else {
                    continue;
                };
                let Ok(c) = view.camera.raster_point(triangle[2].position) else {
                    continue;
                };
                let denominator = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
                if denominator.abs() < 1e-10 {
                    continue;
                }
                let u =
                    ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / denominator;
                let v =
                    ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / denominator;
                let w = 1.0 - u - v;
                if u >= -1e-8
                    && v >= -1e-8
                    && w >= -1e-8
                    && u * a[2] + v * b[2] + w * c[2] > p[2] + 1e-7
                {
                    return false;
                }
            }
        }
    }
    true
}
