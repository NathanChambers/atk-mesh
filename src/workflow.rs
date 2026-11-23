use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const REQUIREMENTS: [&str; 5] = [
    "intent",
    "reference",
    "primary_forms",
    "views",
    "export_validation",
];
const REFINED_REQUIREMENTS: [&str; 7] = [
    "intent", "reference", "primary_forms", "views",
    "placeholder_features", "placeholder_assembly", "placeholder_placement",
];

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Workflow {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub review_presets: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    part_plan: Value,
    #[serde(default)]
    records: BTreeMap<String, Value>,
    #[serde(default)]
    resources: BTreeMap<String, Value>,
    #[serde(default)]
    stage: String,
}

fn fingerprint(value: &Value) -> String {
    hash_bytes(value.to_string().as_bytes())
}

pub(crate) fn animation_fingerprint(mesh: &Value, clip: &str) -> String {
    let rig = &mesh["rig"];
    fingerprint(&json!({"bones":rig["bones"],"weights":rig["weights"],"clip":rig["clips"][clip]}))
}

pub(crate) fn review_rig_fingerprint(mesh: &Value, object: Option<u64>) -> String {
    let rig = &mesh["rig"];
    let Some(id) = object else {
        return fingerprint(rig);
    };
    let weights: BTreeMap<_, _> = mesh["objects"][id.to_string()]["vertices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|vertex| {
            let key = vertex.to_string();
            rig["weights"].get(&key).map(|weight| (key, weight))
        })
        .collect();
    if weights.is_empty() {
        return fingerprint(&Value::Null);
    }
    fingerprint(&json!({"bones":rig["bones"],"clips":rig["clips"],"weights":weights}))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn same_geometry(first: &Value, second: &Value, object: Option<u64>) -> bool {
    geometry(first, object) == geometry(second, object)
}

fn geometry(mesh: &Value, object: Option<u64>) -> Value {
    fn remove_stage(object: &mut Value) {
        if let Some(fields) = object.as_object_mut() {
            fields.remove("stage");
            fields.remove("stage_note");
            fields.remove("review");
        }
    }

    let Some(id) = object else {
        let mut objects = mesh["objects"].clone();
        if let Some(records) = objects.as_object_mut() {
            for record in records.values_mut() {
                remove_stage(record);
            }
        }
        return json!({"vertices":mesh["vertices"],"faces":mesh["faces"],"objects":objects,"flat_faces":mesh["flat_faces"],"uvs":mesh["uvs"],"face_textures":mesh["face_textures"],"textures":mesh["textures"],"vertex_colors":mesh["vertex_colors"]});
    };
    let key = id.to_string();
    let mut object = mesh["objects"][&key].clone();
    remove_stage(&mut object);
    let ids = object["vertices"].as_array().cloned().unwrap_or_default();
    let mut vertices = BTreeMap::new();
    for id in &ids {
        vertices.insert(id.to_string(), mesh["vertices"][id.to_string()].clone());
    }
    let faces: BTreeMap<_, _> = mesh["faces"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, face)| {
            face.as_array()
                .is_some_and(|face| face.iter().any(|id| ids.contains(id)))
        })
        .collect();
    let flat_faces: Vec<_> = mesh["flat_faces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|id| faces.contains_key(&id.to_string()))
        .collect();
    let scoped_map = |field: &str| -> BTreeMap<_, _> {
        mesh[field]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, _)| faces.contains_key(*id))
            .collect()
    };
    let uvs = scoped_map("uvs");
    let face_textures = scoped_map("face_textures");
    let textures: BTreeMap<_, _> = face_textures
        .values()
        .filter_map(|name| name.as_str())
        .map(|name| (name, &mesh["textures"][name]))
        .collect();
    let vertex_colors: BTreeMap<_, _> = mesh["vertex_colors"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, _)| vertices.contains_key(*id))
        .collect();
    json!({"object":object,"vertices":vertices,"faces":faces,"flat_faces":flat_faces,"uvs":uvs,"face_textures":face_textures,"textures":textures,"vertex_colors":vertex_colors})
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request[key]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 4096 && !s.contains("REPLACE"))
        .ok_or_else(|| {
            format!(
                "{key} requires nonempty text, at most 4096 bytes; replace template placeholders"
            )
        })
}

fn clearance_object(selector: &Value) -> Option<u64> {
    let fields = selector.as_object()?;
    if fields.len() != 1 {
        return None;
    }
    fields.get("object")?.as_u64()
}

fn scope(request: &Value, mesh: &Value) -> Result<Option<u64>, String> {
    if request.get("object").is_none() {
        return Ok(None);
    }
    let id = request["object"].as_u64().ok_or("object must be an ID")?;
    if mesh["objects"].get(id.to_string()).is_none() {
        return Err("unknown object".into());
    }
    Ok(Some(id))
}

fn key(requirement: &str, object: Option<u64>) -> String {
    object
        .map(|id| format!("{requirement}@{id}"))
        .unwrap_or_else(|| requirement.into())
}

fn guidance(requirement: &str) -> &'static str {
    match requirement {
        "intent" => {
            "Record intended use, scale, geometry versus material detail, and uncertain features."
        }
        "reference" => {
            "Register an approved reference or assess a saved part plan as a shape_brief. Identify the authoritative geometry targets and record inferred features."
        }
        "primary_forms" => {
            "Read the reference and a matching capture. Assess silhouette, proportions, taper, and attachment alignment; repair discrepancies before detailing."
        }
        "views" => {
            "Read at least two distinct camera views. Check thickness, depth, clearances and hidden assumptions; use cross-sections when relevant."
        }
        "placeholder_features" => "List the visible features required by the brief and compare their presence and silhouette in the intended view.",
        "placeholder_assembly" => "Check how the major parts fit and read together. If the brief calls for motion, review representative poses.",
        "placeholder_placement" => "Verify scale, orientation and pivot in the intended camera and placement context or against a measured stand-in.",
        _ => {
            "Inspect current topology and shading. Compare clean topology and tessellation captures at critical panels and junctions. Record export target, origins, detail budget, and remaining limitations. Independent import verification follows export."
        }
    }
}

impl Workflow {
    fn design_fingerprint(&self) -> String {
        let alignments: BTreeMap<_,_> = self.resources.iter().filter(|(_,resource)| resource["kind"]=="reference").map(|(id,resource)| (id,json!({"view":resource["view"],"landmarks":resource["landmarks"],"contours":resource["contours"]}))).collect();
        fingerprint(
            &json!({"intent":self.records.get("intent"),"reference":self.records.get("reference"),"alignments":alignments}),
        )
    }

    pub fn reference(&self, id: &str, directory: &Path) -> Result<Value, String> {
        let resource = self.resources.get(id).ok_or("unknown reference resource")?;
        if resource["kind"] != "reference" {
            return Err("reference must name a registered reference resource".into());
        }
        if !self.current_resource(resource, &Value::Null, directory) {
            return Err("reference file is missing or modified; register the current source before aligning".into());
        }
        Ok(resource.clone())
    }

    pub fn update_reference(
        &mut self,
        id: &str,
        view: Option<Value>,
        landmarks: Option<&Value>,
    ) -> Result<Value, String> {
        let resource = self
            .resources
            .get_mut(id)
            .ok_or("unknown reference resource")?;
        if let Some(view) = view {
            resource["view"] = view;
        }
        if let Some(landmarks) = landmarks {
            resource["landmarks"] = landmarks["landmarks"].clone();
            resource["contours"] = landmarks
                .get("contours")
                .cloned()
                .unwrap_or_else(|| json!([]));
        }
        let version = resource["alignment_version"]
            .as_u64()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("alignment version exhausted")?;
        resource["alignment_version"] = json!(version);
        Ok(
            json!({"reference":id,"alignment_version":version,"landmarks":resource["landmarks"].as_array().map(|v|v.len()).unwrap_or(0),"view":resource["view"]}),
        )
    }

    pub fn open(directory: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let path = directory.join("workflow.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let value: Value = serde_json::from_slice(&fs::read(path)?)?;
        if value["version"] != 1 {
            return Err("unsupported workflow version".into());
        }
        Ok(serde_json::from_value(value["workflow"].clone())?)
    }

    pub fn save(&self, directory: &Path) -> Result<(), String> {
        let temporary = directory.join("workflow.tmp");
        let bytes =
            serde_json::to_vec(&json!({"version":1,"workflow":self})).map_err(|e| e.to_string())?;
        fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        fs::rename(temporary, directory.join("workflow.json")).map_err(|e| e.to_string())
    }

    fn current_resource(&self, resource: &Value, mesh: &Value, directory: &Path) -> bool {
        let path = directory.join(resource["path"].as_str().unwrap_or(""));
        let valid_file = fs::canonicalize(&path).is_ok_and(|path| {
            fs::canonicalize(directory).is_ok_and(|directory| path.starts_with(directory))
        }) && fs::read(path)
            .is_ok_and(|bytes| hash_bytes(&bytes) == resource["content_hash"]);
        let object = resource["object"].as_u64();
        let valid_geometry = resource["kind"] == "reference"
            || resource["geometry"] == fingerprint(&geometry(mesh, object));
        let source = &resource["source"];
        let valid_animation = source["animation"]["clip"].as_str().is_none_or(|clip| {
            source["animation_fingerprint"] == animation_fingerprint(mesh, clip)
        });
        valid_file && valid_geometry && valid_animation
    }

    pub(crate) fn object_review_evidence(
        &self, id: &str, object: u64, mesh: &Value, directory: &Path,
    ) -> Result<(bool, bool), String> {
        let resource = self.resources.get(id).ok_or_else(|| format!("unknown evidence {id}"))?;
        let capture = resource["kind"] == "capture";
        let report = resource["kind"] == "evidence";
        if !capture && !report {
            return Err(format!("{id} must be a capture or evidence resource"));
        }
        if resource["object"].as_u64().is_some_and(|scope| scope != object) {
            return Err(format!("{id} belongs to another object"));
        }
        if !self.current_resource(resource, mesh, directory) {
            return Err(format!("{id} is stale; register current evidence"));
        }
        let current_rig = review_rig_fingerprint(mesh, resource["object"].as_u64());
        let rig_current = resource["rig_fingerprint"] == current_rig
            || (resource["rig_fingerprint"].is_null() && current_rig == fingerprint(&Value::Null));
        if !rig_current {
            return Err(format!("{id} predates the current rig; register current evidence"));
        }
        Ok((capture, resource["object"].is_null()))
    }

    fn status(&self, record: &Value, mesh: &Value, directory: &Path) -> &'static str {
        if record["outcome"] == "needs_revision" {
            return "needs_revision";
        }
        if record["status"] == "waived" {
            return "waived";
        }
        if record["status"] != "satisfied" {
            return "pending";
        }
        if record["basis"] == "shape_brief" && record["brief"] != fingerprint(&self.shape_brief()) {
            return "pending";
        }
        let dependent = !matches!(record["requirement"].as_str(), Some("intent" | "reference"));
        if dependent
            && record["geometry"] != fingerprint(&geometry(mesh, record["object"].as_u64()))
        {
            return "pending";
        }
        if dependent && record["design"] != self.design_fingerprint() {
            return "pending";
        }
        let valid_resources = record["evidence"].as_array().is_some_and(|ids| {
            ids.iter().all(|id| {
                self.resources
                    .get(id.as_str().unwrap_or(""))
                    .is_some_and(|resource| self.current_resource(resource, mesh, directory))
            })
        });
        if !valid_resources {
            return "pending";
        }
        "satisfied"
    }

    pub fn report(&self, mesh: &Value, directory: &Path) -> Value {
        let mut records = self.records.clone();
        for requirement in REQUIREMENTS {
            records
                .entry(requirement.into())
                .or_insert_with(|| json!({"requirement":requirement,"status":"pending"}));
        }
        let records: Vec<_> = records
            .values()
            .map(|record| {
                let mut record = record.clone();
                record["status"] = json!(self.status(&record, mesh, directory));
                record["prompt"] = json!(guidance(record["requirement"].as_str().unwrap_or("")));
                record
            })
            .collect();
        let stage = if self.stage.is_empty() { "blockout" } else { &self.stage };
        let next_action = self.next_action(stage, &records, mesh, directory);
        let mut report = json!({"stage":stage,"requirements":records,"resources":self.resources,"next_action":next_action,"follow_ups":self.follow_ups(),"assessment":"Agent assessments are recorded claims, not automatic proof of visual accuracy."});
        report["part_review"] = self.part_review();
        if matches!(report["next_action"]["decision"].as_str(), Some("review_primary_forms" | "revise_primary_forms"))
            && self.records.get("reference").is_some_and(|record| record["basis"] == "shape_brief")
        {
            report["next_action"]["shape_brief"] = self.shape_brief();
        }
        if stage == "detail" || report["next_action"]["decision"] == "start_detail" {
            report["topology_review"] = self.topology_review();
        }
        if stage != "complete" {
            report["instancing_review"] = Self::instancing_review();
        }
        report
    }

    fn shape_brief(&self) -> Value {
        let mut brief = self.part_plan.clone();
        if let Some(parts) = brief.get_mut("parts").and_then(Value::as_array_mut) {
            for part in parts {
                if let Some(fields) = part.as_object_mut() {
                    fields.remove("objects");
                    fields.remove("collections");
                }
            }
        }
        brief
    }

    fn part_review(&self) -> Value {
        json!({
            "advisory":true,
            "recorded":!self.part_plan.is_null(),
            "command":{"command":"workflow_part_plan"},
            "record_command":{"command":"workflow_part_plan","plan":{"parts":[{"name":"REPLACE with part name","silhouette_views":["side"],"profiles":[{"view":"side","outline":"REPLACE with the shape and where it changes","basis":"observed","checkpoints":["REPLACE with a visible landmark or intended proportion"]}]}],"relationships":[]}},
            "prompts":[
                "Before blockout, list major parts and the views where each controls the silhouette. Identify which parts should follow a common outer form.",
                "For silhouette-driving parts, record the target outline in each key view. Include taper direction, main section changes, major proportions, and where edges should stay sharp or roll over.",
                "Mark each target profile observed or inferred. Check its landmarks in a matched view or section before accepting primary forms. Review profiles flagged by missing_profile_views.",
                "For each important pair, describe how the interface should read, how it will be built, and what is inferred from the reference.",
                "Use a shared mating profile for fitted separate parts. Bind planned parts to object and collection IDs as they are built. Check sections through the seam and preview one likely edit before detailing."
            ]
        })
    }

    fn part_plan(&mut self, request: &Value, mesh: &Value) -> Result<Value, String> {
        if let Some(plan) = request.get("plan") {
            if request.get("object").is_some() || request.get("collection").is_some() {
                return Err("part plan filters are read-only".into());
            }
            let parts = plan["parts"].as_array().filter(|parts| !parts.is_empty() && parts.len() <= 64)
                .ok_or("part plan requires 1..64 parts")?;
            let relationships = plan["relationships"].as_array().filter(|items| items.len() <= 128)
                .ok_or("part plan requires a relationships array with at most 128 entries")?;
            if plan.to_string().len() > 65536 {
                return Err("part plan exceeds 64 KiB".into());
            }
            let mut names = BTreeSet::new();
            for part in parts {
                let name = text(part, "name")?;
                if !names.insert(name) {
                    return Err("part names must be unique".into());
                }
                let views = part["silhouette_views"].as_array()
                    .filter(|views| views.iter().all(|view| view.as_str().is_some_and(|s| !s.trim().is_empty())))
                    .ok_or("silhouette_views must be an array of view names")?;
                if let Some(profiles) = part.get("profiles") {
                    let profiles = profiles.as_array().ok_or("profiles must be an array")?;
                    let mut profiled = BTreeSet::new();
                    for profile in profiles {
                        let view = text(profile, "view")?;
                        if !views.iter().any(|name| name == view) || !profiled.insert(view) {
                            return Err("profile views must be unique entries in silhouette_views".into());
                        }
                        text(profile, "outline")?;
                        if !matches!(profile["basis"].as_str(), Some("observed" | "inferred")) {
                            return Err("profile basis must be observed or inferred".into());
                        }
                        profile["checkpoints"].as_array().filter(|items| !items.is_empty()
                            && items.iter().all(|item| item.as_str().is_some_and(|s| !s.trim().is_empty())))
                            .ok_or("profile checkpoints must be a nonempty array of text")?;
                        if let Some(reference) = profile.get("reference") {
                            let id = reference.as_str().ok_or("profile reference must be a resource ID")?;
                            if !self.resources.get(id).is_some_and(|entry| entry["kind"] == "reference") {
                                return Err(format!("unknown profile reference {id}"));
                            }
                        }
                    }
                }
                for (field, source) in [("objects", "objects"), ("collections", "collections")] {
                    if let Some(values) = part.get(field) {
                        let ids = values.as_array().ok_or("part bindings must be arrays of IDs")?;
                        let mut seen = BTreeSet::new();
                        for id in ids {
                            let id = id.as_u64().ok_or("part binding must be an integer ID")?;
                            if mesh[source][id.to_string()].is_null() || !seen.insert(id) {
                                return Err(format!("unknown or duplicate {field} binding {id}"));
                            }
                        }
                    }
                }
            }
            for relationship in relationships {
                let a = text(relationship, "a")?;
                let b = text(relationship, "b")?;
                if a == b || !names.contains(a) || !names.contains(b) {
                    return Err("a relationship requires two distinct planned parts".into());
                }
                text(relationship, "intent")?;
                text(relationship, "verification")?;
            }
            let mut plan = plan.clone();
            for part in plan["parts"].as_array_mut().unwrap() {
                let fields = part.as_object_mut().unwrap();
                for field in ["missing_profile_views", "missing_objects", "missing_collections"] {
                    fields.remove(field);
                }
            }
            self.part_plan = plan;
        }
        if self.part_plan.is_null() {
            return Ok(json!({"parts":[],"relationships":[],"part_review":self.part_review()}));
        }
        let object = request.get("object").map(|v| v.as_u64().ok_or("object must be an ID")).transpose()?;
        let collection = request.get("collection").map(|v| v.as_u64().ok_or("collection must be an ID")).transpose()?;
        let mut parts = self.part_plan["parts"].as_array().unwrap().clone();
        let mut relationships = self.part_plan["relationships"].as_array().unwrap().clone();
        if object.is_some() || collection.is_some() {
            let names: BTreeSet<_> = parts.iter().filter(|part| {
                let objects = part["objects"].as_array().into_iter().flatten();
                let collections = part["collections"].as_array().into_iter().flatten();
                objects.into_iter().any(|id| id.as_u64() == object || collection.is_some_and(|c| {
                    mesh["collections"][c.to_string()]["objects"].as_array().is_some_and(|members| members.contains(id))
                })) || collections.into_iter().any(|id| id.as_u64() == collection || object.is_some_and(|o| {
                    mesh["collections"][id.to_string()]["objects"].as_array().is_some_and(|members| members.contains(&json!(o)))
                }))
            }).filter_map(|part| part["name"].as_str().map(str::to_owned)).collect();
            relationships.retain(|item| ["a", "b"].iter().any(|key| names.contains(item[*key].as_str().unwrap())));
            let related: BTreeSet<_> = relationships.iter().flat_map(|item| [item["a"].as_str().unwrap(), item["b"].as_str().unwrap()]).collect();
            parts.retain(|part| {
                let name = part["name"].as_str().unwrap();
                names.contains(name) || related.contains(name)
            });
        }
        for part in &mut parts {
            let profiles = part["profiles"].as_array().into_iter().flatten();
            let covered: BTreeSet<_> = profiles.filter_map(|profile| profile["view"].as_str()).collect();
            let missing_profiles: Vec<_> = part["silhouette_views"].as_array().into_iter().flatten()
                .filter(|view| view.as_str().is_some_and(|name| !covered.contains(name))).collect();
            if !missing_profiles.is_empty() {
                part["missing_profile_views"] = json!(missing_profiles);
            }
            for field in ["objects", "collections"] {
                let missing: Vec<_> = part[field].as_array().into_iter().flatten()
                    .filter(|id| mesh[field][id.to_string()].is_null()).collect();
                if !missing.is_empty() {
                    part[format!("missing_{field}")] = json!(missing);
                }
            }
        }
        Ok(json!({"parts":parts,"relationships":relationships}))
    }

    fn instancing_review() -> Value {
        json!({
            "advisory":true,
            "query_command":{"command":"query_mesh","kind":"objects"},
            "instance_command":{"command":"object_instance","object":0,"value":[0,0,0],"name":"REPLACE with placement name"},
            "prompts":[
                "Before building the blockout, identify repeated shapes that should change together, including repeated parts within a larger assembly. Build one master per distinct shape and batch object_instance placements using the actual master ID, offsets and rotations.",
                "When a repeated layout contains linked parts, reuse its placement offsets across the larger assembly. Instance each part directly from its master because nested instance chains are unsupported. Review equivalent independent copies for object_link_instance and keep separate masters for geometry variants.",
                "Use the object query to check master_object, instance_count and instances. Check placements in fixed review views and preview one expected master edit in a transaction or a copy to verify it propagates."
            ]
        })
    }

    fn topology_review(&self) -> Value {
        let regions: Vec<_> = self.records.values().filter_map(|record| {
            let regions = record["critical_regions"].as_array()?;
            Some(json!({"object":record["object"],"regions":regions}))
        }).collect();
        json!({
            "help":{"command":"help","topic":"junction-review"},
            "critical_regions":regions,
            "prompts":[
                "Identify the shape, shading, motion, or future edit each critical region must support.",
                "Carry edge loops through bends and around openings. Redirect or terminate excess loops on flatter surfaces.",
                "For symmetric work, reuse the profile or find existing mirrored counterparts before editing.",
                "Try one expected edit in a transaction preview or a copy. Compare shaded, topology and tessellation views and check the relevant clearance and symmetry."
            ]
        })
    }

    fn follow_ups(&self) -> Vec<Value> {
        self.records.values().filter_map(|record| {
            let note = record["follow_up"].as_str()?;
            Some(json!({"requirement":record["requirement"],"object":record["object"],"note":note}))
        }).collect()
    }

    fn baseline_reviews(&self, evidence: &Value) -> Vec<Value> {
        let mut seen = BTreeSet::new();
        evidence.as_array().into_iter().flatten().filter_map(|id| {
            let resource = self.resources.get(id.as_str()?)?;
            if resource["kind"] != "capture" || resource["source"]["command"] != "review_run" {
                return None;
            }
            let review = &resource["source"]["review"];
            let name = review["name"].as_str()?;
            let version = review["preset_version"].as_u64()?;
            if !seen.insert((name, version)) {
                return None;
            }
            let current = self.review_presets.get(name).and_then(|preset| preset["version"].as_u64());
            let command = (current == Some(version))
                .then(|| json!({"command":"review_run","name":name,"preset_version":version}));
            Some(json!({"evidence":id,"source":resource["source"],"current_preset_version":current,"command":command}))
        }).collect()
    }

    pub(crate) fn invalidated_saved_review(&self, before: &Value, after: &Value, directory: &Path) -> Option<Value> {
        for record in self.records.values() {
            let Some(requirement) = record["requirement"].as_str() else {
                continue;
            };
            if record["status"] != "satisfied" || matches!(requirement, "intent" | "reference" | "export_validation") {
                continue;
            }
            let reviews = self.baseline_reviews(&record["evidence"]);
            let Some(review) = reviews.iter().find(|review| review["command"].is_object()) else {
                continue;
            };
            let object = record["object"].as_u64();
            if !same_geometry(before, after, object) && self.status(record, before, directory) == "satisfied" {
                return Some(json!({"requirement":requirement,"object":object,"evidence":review["evidence"],"command":review["command"]}));
            }
        }
        None
    }

    pub(crate) fn affected_clearance(&self, before: &Value, after: &Value) -> Option<Value> {
        let mut surfaces = BTreeMap::new();
        for (name, preset) in &self.review_presets {
            for check in preset["clearances"].as_array().into_iter().flatten() {
                let Some(a) = clearance_object(&check["a"]) else { continue };
                let Some(b) = clearance_object(&check["b"]) else { continue };
                if a == b || after["objects"].get(a.to_string()).is_none() || after["objects"].get(b.to_string()).is_none() {
                    continue;
                }

                let mut surface = |object| *surfaces.entry(object).or_insert_with(|| {
                    let previous = geometry(before, Some(object));
                    let current = geometry(after, Some(object));
                    let changed = ["vertices", "faces"].iter().any(|key| previous[*key] != current[*key])
                        || previous["object"]["origin"] != current["object"]["origin"];
                    let triangles = current["faces"].as_object().into_iter().flatten()
                        .filter_map(|(_, face)| face.as_array())
                        .map(|face| face.len().saturating_sub(2)).sum::<usize>();
                    (changed, triangles)
                });
                let (a_changed, a_triangles) = surface(a);
                let (b_changed, b_triangles) = surface(b);
                let triangle_pairs = a_triangles.saturating_mul(b_triangles);
                if !(a_changed || b_changed) || !(1..=2_000_000).contains(&triangle_pairs) {
                    continue;
                }

                let mut cue = json!({"preset":name,"check":check["name"],"pose":"current",
                    "command":{"command":"measure_clearance","a":{"object":a},"b":{"object":b}}});
                if let Some(minimum) = check["minimum"].as_f64().filter(|minimum| *minimum >= 0.0) {
                    cue["minimum"] = json!(minimum);
                }
                return Some(cue);
            }
        }
        None
    }

    fn next_action(&self, stage: &str, records: &[Value], mesh: &Value, directory: &Path) -> Value {
        let active = |record: &&Value| stage == "export" || record["requirement"] != "export_validation";
        let revision = records.iter().filter(active).find(|record| {
            record["requirement"] == "primary_forms" && record["status"] == "needs_revision"
        });
        let pending = REQUIREMENTS.iter().find(|requirement| {
            records.iter().filter(active).any(|record| {
                record["requirement"].as_str() == Some(**requirement)
                    && record["object"].is_null()
                    && record["status"] == "pending"
            })
        });
        let (decision, prompt, topic, command) = match (stage, pending.copied()) {
            (_, Some("intent")) => (
                "define_asset",
                "Record intended use, fixed dimensions, moving parts if required, and the visual traits that must survive simplification.",
                "reference-preparation",
                json!({"command":"workflow_assess","requirement":"intent","assessment":"REPLACE with the asset brief","evidence":[]}),
            ),
            (_, Some("reference")) => (
                "prepare_references",
                "Register and assess a reference, or record a part plan and assess it with basis: shape_brief and an empty evidence array.",
                "reference-preparation",
                json!({"command":"help","topic":"reference-preparation"}),
            ),
            ("blockout", _) => (
                "start_primary",
                "Enter primary and build the smallest set of masses needed to judge silhouette, scale, and fit.",
                "blockout-review",
                json!({"command":"workflow_stage","stage":"primary"}),
            ),
            (_, _) if revision.is_some() || pending == Some(&"primary_forms") => {
                let reference = self.records.get("reference")
                    .filter(|record| record["status"] == "satisfied" && record["basis"] != "shape_brief")
                    .and_then(|record| record["evidence"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|id| self.resources.get(id.as_str()?))
                    .find(|resource| resource["kind"] == "reference"
                        && resource["object"].is_null()
                        && self.current_resource(resource, mesh, directory));
                let comparison = reference
                    .map(|resource| json!({"command":"compare_reference","reference":resource["id"]}))
                    .unwrap_or_else(|| json!({"command":"capture","mode":"shaded","yaw":35,"pitch":20}));
                if let Some(review) = revision {
                    let baselines = self.baseline_reviews(&review["evidence"]);
                    let recheck = if baselines.is_empty() {
                        comparison
                    } else {
                        baselines.iter().find_map(|baseline| {
                            baseline["command"].as_object().map(|_| baseline["command"].clone())
                        }).unwrap_or(Value::Null)
                    };
                    return json!({
                        "decision":"revise_primary_forms",
                        "prompt":"Address the recorded findings. If the right change is uncertain, compare a controlled variant using the same views. A changed preset requires a new baseline.",
                        "object":review["object"],
                        "findings":review["assessment"],
                        "previous_evidence":review["evidence"],
                        "baseline_reviews":baselines,
                        "help":{"command":"help","topic":"variant-review"},
                        "recheck_command":recheck
                    });
                }
                (
                    "review_primary_forms",
                    "Compare the main masses as soon as they exist. Inspect silhouette, relative dimensions and attachment placement before adding detail. Refresh evidence after relevant changes.",
                    "blockout-review",
                    comparison,
                )
            }
            (_, Some("views")) => (
                "review_additional_views",
                "Choose views that resolve thickness, depth, or fit. Inspect and register at least two distinct directions before assessing views.",
                "blockout-review",
                json!({"command":"help","topic":"blockout-review"}),
            ),
            ("primary", _) => (
                "start_detail",
                "Advance to detail using the recorded primary-form and view findings.",
                "modeling-requirements",
                json!({"command":"workflow_stage","stage":"detail"}),
            ),
            _ => return Value::Null,
        };
        json!({"decision":decision,"prompt":prompt,"help":{"command":"help","topic":topic},"command":command})
    }

    pub fn gate(&self, stage: &str, mesh: &Value, directory: &Path) -> Option<Value> {
        let needed: &[&str] = match stage {
            "blockout" => &[],
            "primary" => &REQUIREMENTS[..2],
            "refined" => &REFINED_REQUIREMENTS,
            "detail" | "complete" => &REQUIREMENTS[..4],
            _ => &REQUIREMENTS,
        };
        let report = self.report(mesh, directory);
        let mut records = report["requirements"].as_array().unwrap().clone();
        if stage == "refined" {
            for record in &mut records {
                let assembly_view = record["object"].is_null()
                    && matches!(record["requirement"].as_str(), Some("primary_forms" | "views"));
                if assembly_view && record["status"] == "satisfied"
                    && record["evidence"].as_array().is_some_and(|ids| ids.iter().any(|id| {
                        self.resources.get(id.as_str().unwrap_or("")).is_some_and(|resource| resource["object"].is_number())
                    }))
                {
                    record["status"] = json!("pending");
                }
            }
            for requirement in &REFINED_REQUIREMENTS[4..] {
                if !records.iter().any(|record| record["requirement"] == *requirement) {
                    records.push(json!({"requirement":requirement,"status":"pending"}));
                }
            }
        }
        let missing: Vec<_> = records.iter().filter(|record| {
            needed.contains(&record["requirement"].as_str().unwrap_or(""))
                && (stage != "refined" || record["object"].is_null())
                && (matches!(record["status"].as_str(), Some("pending" | "needs_revision"))
                    || (stage == "refined" && record["status"] == "waived"))
        }).map(|record| {
            let requirement = record["requirement"].as_str().unwrap();
            let mut satisfy = json!({"command":"workflow_assess","requirement":requirement,"assessment":"REPLACE with findings after reading resources","evidence":[]});
            let mut waive = json!({"command":"workflow_waive","requirement":requirement,"reason":"REPLACE with why this requirement does not apply"});
            if requirement == "primary_forms" || requirement.starts_with("placeholder_") {
                satisfy["outcome"] = json!("accepted");
            }
            if !record["object"].is_null() {
                satisfy["object"] = record["object"].clone();
                waive["object"] = record["object"].clone();
            }
            let reference = self.resources.values().find(|resource| resource["kind"] == "reference");
            let capture = reference.map(|resource| json!({"command":"compare_reference","reference":resource["id"]})).unwrap_or_else(|| json!({"command":"capture","mode":"shaded","yaw":0,"pitch":0}));
            let mut missing = json!({"requirement":requirement,"object":record["object"],"status":record["status"],"prompt":guidance(requirement),"resource_command":{"command":"workflow_resource","kind":if requirement == "reference" {"reference"} else {"evidence"},"path":"REPLACE with workspace-relative resource path","description":"REPLACE with resource purpose"},"capture_command":capture,"register_capture_command":{"command":"workflow_resource","kind":"capture","session":"REPLACE with capture trace.session","event":0,"description":"REPLACE with view purpose"},"satisfy_command":satisfy});
            if stage != "refined" {
                missing["waive_command"] = waive;
            }
            if record["status"] == "needs_revision" {
                missing["assessment"] = record["assessment"].clone();
                missing["evidence"] = record["evidence"].clone();
            }
            missing
        }).collect();
        if missing.is_empty() {
            return None;
        }
        let (next_action, prompt) = if stage == "refined" {
            let blocker = missing.iter().find(|record| record["requirement"] == "primary_forms").unwrap_or(&missing[0]);
            (json!({"decision":"review_placeholder","requirement":blocker["requirement"],"command":blocker["satisfy_command"],"help":{"command":"help","topic":"object-detail-stages"}}),
             "Review the current evidence and resolve each workspace and object blocker before promoting.")
        } else {
            (self.next_action(stage, &missing, mesh, directory),
             "Read the requested resources, fix discrepancies and record findings, or explicitly waive each inapplicable requirement with a reason. Commands containing REPLACE are templates.")
        };
        Some(
            json!({"status":"requirements_missing","stage":stage,"requirements":missing,"next_action":next_action,"prompt":prompt}),
        )
    }

    pub fn execute(
        &mut self,
        request: &Value,
        mesh: &Value,
        directory: &Path,
    ) -> Result<Value, String> {
        let command = request["command"].as_str().unwrap_or("");
        if command == "workflow_part_plan" {
            return self.part_plan(request, mesh);
        }
        if command == "workflow_status" {
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
                .unwrap_or(8) as usize;
            let mut report = self.report(mesh, directory);
            let mut resources: Vec<_> = self
                .resources
                .values()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            if request["response"] != "full" {
                for resource in &mut resources {
                    for key in ["landmarks", "contours"] {
                        if let Some(values) = resource.as_object_mut().unwrap().remove(key) {
                            resource[format!("{key}_count")] =
                                json!(values.as_array().map(|v| v.len()).unwrap_or(0));
                        }
                    }
                }
            }
            let next = offset + resources.len();
            report["resources"] = json!(resources);
            report["resource_count"] = json!(self.resources.len());
            report["next_offset"] = if next < self.resources.len() {
                json!(next)
            } else {
                Value::Null
            };
            return Ok(report);
        }
        if command == "workflow_resource" {
            let kind = text(request, "kind")?;
            if !matches!(kind, "reference" | "capture" | "evidence") {
                return Err("kind must be reference, capture or evidence".into());
            }
            let path = directory.join(text(request, "path")?);
            let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
            let directory = fs::canonicalize(directory).map_err(|e| e.to_string())?;
            let relative = path
                .strip_prefix(&directory)
                .map_err(|_| "resources must be inside this workspace")?;
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            let object = scope(request, mesh)?;
            let id = format!("resource-{}", self.resources.len() + 1);
            let rig_fingerprint = request["evidence_rig"].as_str().map(str::to_owned)
                .unwrap_or_else(|| review_rig_fingerprint(mesh, object));
            let resource = json!({"id":id,"kind":kind,"path":relative,"description":text(request,"description")?,"content_hash":hash_bytes(&bytes),"object":object,"geometry":fingerprint(&geometry(mesh,object)),"rig_fingerprint":rig_fingerprint,"camera":request["camera"],"view":request["view"],"source":request["source"]});
            self.resources.insert(id.clone(), resource.clone());
            return Ok(resource);
        }
        if command == "workflow_stage" {
            let stage = text(request, "stage")?;
            if !matches!(stage, "blockout" | "primary" | "detail" | "complete") {
                return Err("stage must be blockout, primary, detail or complete".into());
            }
            if let Some(blocked) = self.gate(stage, mesh, directory) {
                return Ok(blocked);
            }
            self.stage = stage.into();
            let mut result = json!({"stage":stage,"follow_ups":self.follow_ups()});
            if matches!(stage, "primary" | "detail") {
                result["part_review"] = self.part_review();
            }
            if stage == "detail" {
                result["topology_review"] = self.topology_review();
            }
            if stage != "complete" {
                result["instancing_review"] = Self::instancing_review();
            }
            return Ok(result);
        }
        let requirement = text(request, "requirement")?;
        if !REQUIREMENTS.contains(&requirement) && !REFINED_REQUIREMENTS.contains(&requirement) {
            return Err("unknown requirement; use workflow_status".into());
        }
        let placeholder = requirement.starts_with("placeholder_");
        if placeholder && (request.get("object").is_some() || command == "workflow_waive") {
            return Err("placeholder assessments apply to the whole workspace and require a review".into());
        }
        let existing_scope = request["object"]
            .as_u64()
            .filter(|id| self.records.contains_key(&key(requirement, Some(*id))));
        let object = if matches!(command, "workflow_waive" | "workflow_reopen")
            && existing_scope.is_some()
        {
            existing_scope
        } else {
            scope(request, mesh)?
        };
        let record = match command {
            "workflow_waive" => {
                json!({"requirement":requirement,"object":object,"status":"waived","reason":text(request,"reason")?})
            }
            "workflow_assess" => {
                let assessment = text(request, "assessment")?;
                let outcome = if requirement == "primary_forms" || placeholder {
                    Some(request["outcome"].as_str()
                        .filter(|outcome| matches!(*outcome, "accepted" | "needs_revision"))
                        .ok_or("assessment requires outcome: accepted or needs_revision")?)
                } else {
                    None
                };
                let critical_regions = if let Some(regions) = request.get("critical_regions") {
                    if requirement != "primary_forms" || outcome != Some("accepted") {
                        return Err("critical_regions requires an accepted primary_forms assessment".into());
                    }
                    let regions = regions.as_array().filter(|regions| regions.len() <= 8)
                        .ok_or("critical_regions must be an array of at most eight region labels")?;
                    for region in regions {
                        if !region.as_str().is_some_and(|label| {
                            !label.trim().is_empty() && label.len() <= 256 && !label.contains("REPLACE")
                        }) {
                            return Err("each critical region label must contain 1..256 bytes of text".into());
                        }
                    }
                    Some(regions)
                } else {
                    None
                };
                let follow_up = if request.get("follow_up").is_some() {
                    if requirement != "primary_forms" || outcome != Some("accepted") {
                        return Err("follow_up requires an accepted primary_forms assessment".into());
                    }
                    Some(text(request, "follow_up")?)
                } else {
                    None
                };
                let evidence = request["evidence"]
                    .as_array()
                    .ok_or("evidence must be an array of resource IDs")?;
                let mut kinds = vec![];
                let mut cameras: Vec<[f64; 3]> = vec![];
                for id in evidence {
                    let resource = self
                        .resources
                        .get(id.as_str().ok_or("invalid resource ID")?)
                        .ok_or("unknown resource")?;
                    if !self.current_resource(resource, mesh, directory) {
                        return Err("resource is missing, modified, or stale; produce and register current evidence".into());
                    }
                    if placeholder && resource["object"].is_number() {
                        return Err("placeholder assessments require workspace-scoped evidence".into());
                    }
                    if object.is_some()
                        && resource["object"].as_u64().is_some()
                        && object != resource["object"].as_u64()
                    {
                        return Err("evidence belongs to a different object".into());
                    }
                    kinds.push(resource["kind"].as_str().unwrap_or(""));
                    if resource["kind"] == "capture" && resource["camera"].is_object() {
                        let yaw = resource["camera"]["yaw"]
                            .as_f64()
                            .unwrap_or(0.0)
                            .to_radians();
                        let pitch = resource["camera"]["pitch"]
                            .as_f64()
                            .unwrap_or(0.0)
                            .to_radians();
                        let direction = [
                            yaw.sin() * pitch.cos(),
                            pitch.sin(),
                            yaw.cos() * pitch.cos(),
                        ];
                        let duplicate = cameras.iter().any(|previous| {
                            previous
                                .iter()
                                .zip(direction)
                                .all(|(a, b)| (a - b).abs() < 1e-6)
                        });
                        if !duplicate {
                            cameras.push(direction);
                        }
                    }
                }
                let shape_brief = request["basis"] == "shape_brief";
                if request.get("basis").is_some() && (!shape_brief || requirement != "reference") {
                    return Err("basis: shape_brief applies only to the reference assessment".into());
                }
                let brief = self.shape_brief();
                if shape_brief {
                    let parts = brief["parts"].as_array().ok_or("record a part plan before assessing a shape brief")?;
                    let views = parts.iter().flat_map(|part| part["silhouette_views"].as_array().into_iter().flatten());
                    let has_views = views.count() > 0;
                    let has_uncovered_views = parts.iter().any(|part| {
                        part["silhouette_views"].as_array().into_iter().flatten().any(|view| {
                            !part["profiles"].as_array().into_iter().flatten().any(|profile| &profile["view"] == view)
                        })
                    });
                    if !has_views || has_uncovered_views {
                        return Err("a shape brief needs target profiles for every recorded silhouette view".into());
                    }
                }
                let sufficient = match requirement {
                    "intent" => true,
                    "reference" => shape_brief || kinds.contains(&"reference"),
                    "primary_forms" => kinds.contains(&"capture"),
                    "views" => cameras.len() >= 2,
                    "placeholder_features" | "placeholder_assembly" => kinds.contains(&"capture"),
                    "placeholder_placement" => kinds.contains(&"capture") || kinds.contains(&"evidence"),
                    _ => kinds.contains(&"evidence"),
                };
                if !sufficient {
                    let required = match requirement {
                        "reference" => "a reference resource ID or basis:\"shape_brief\" with a complete part plan",
                        "primary_forms" => "a registered capture resource ID",
                        "views" => "registered capture resource IDs for two distinct camera views",
                        "placeholder_features" | "placeholder_assembly" => "a workspace-scoped capture",
                        "placeholder_placement" => "a workspace-scoped capture or evidence report",
                        _ => "a resource ID registered with workflow_resource and kind:\"evidence\"",
                    };
                    return Err(format!("{requirement} requires {required}; pass resource IDs in evidence"));
                }
                let status = if outcome == Some("needs_revision") { "needs_revision" } else { "satisfied" };
                let mut record = json!({"requirement":requirement,"object":object,"status":status,"assessment":assessment,"evidence":evidence,"geometry":fingerprint(&geometry(mesh,object)),"design":self.design_fingerprint()});
                if shape_brief {
                    record["basis"] = json!("shape_brief");
                    record["brief"] = json!(fingerprint(&brief));
                }
                if let Some(outcome) = outcome {
                    record["outcome"] = json!(outcome);
                }
                if let Some(note) = follow_up {
                    record["follow_up"] = json!(note);
                }
                if let Some(regions) = critical_regions {
                    record["critical_regions"] = json!(regions);
                }
                record
            }
            "workflow_reopen" => {
                json!({"requirement":requirement,"object":object,"status":"pending","reason":text(request,"reason")?})
            }
            _ => return Err("unknown workflow command".into()),
        };
        self.records
            .insert(key(requirement, object), record.clone());
        Ok(record)
    }
}
