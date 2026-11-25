use crate::{Mesh, geometry::{add, cross, dot, length, lerp, mul, sub}, responses};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Bone {
    pub(super) name: String,
    pub(super) parent: Option<String>,
    pub(super) position: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tail: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) rotation_limit: Option<RotationLimit>,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct RotationLimit {
    axis: [f64; 3],
    max_swing: f64,
    twist: [f64; 2],
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Influence {
    pub(super) bone: String,
    pub(super) weight: f64,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Keyframe {
    pub(super) time: f64,
    #[serde(default)]
    rotation: [f64; 3],
    #[serde(default)]
    translation: [f64; 3],
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Interpolation {
    Step,
    #[default]
    Linear,
    Cubic,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Clip {
    pub(super) duration: f64,
    #[serde(default)]
    pub(super) looping: bool,
    pub(super) tracks: BTreeMap<String, Vec<Keyframe>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    interpolation: BTreeMap<String, Interpolation>,
}

impl Clip {
    pub(super) fn interpolation(&self, bone: &str) -> Interpolation {
        self.interpolation.get(bone).copied().unwrap_or_default()
    }

    pub(super) fn track(&self, bone: &str, time: f64) -> (Rotation, [f64; 3]) {
        self.tracks.get(bone).map_or((Rotation::IDENTITY, [0.0; 3]), |keys| {
            sample_track(keys, self.interpolation(bone), time)
        })
    }
}

impl Bone {
    pub(super) fn sample(&self, clip: &Clip, time: f64) -> (Rotation, [f64; 3]) {
        let (rotation, translation) = clip.track(&self.name, time);
        let rotation = self.rotation_limit.map_or(rotation, |limit| limit.apply(rotation));
        (rotation, translation)
    }
}

impl RotationLimit {
    fn apply(self, rotation: Rotation) -> Rotation {
        let axis = mul(self.axis, 1.0 / length(self.axis));
        let [x, y, z, w] = rotation.0;
        let sign = if w < 0.0 { -1.0 } else { 1.0 };
        let projection = sign * dot(axis, [x, y, z]);
        let scalar = sign * w;
        let twist_angle = if projection.hypot(scalar) < 1e-12 {
            0.0
        } else {
            2.0 * projection.atan2(scalar)
        };
        let twist = |angle: f64| {
            let (sine, cosine) = (angle * 0.5).sin_cos();
            Rotation([axis[0] * sine, axis[1] * sine, axis[2] * sine, cosine])
        };

        let swing = rotation.compose(twist(twist_angle).inverse());
        let [x, y, z, w] = swing.0;
        let swing_angle = 2.0 * length([x, y, z]).atan2(w.abs());
        let swing_limit = self.max_swing.to_radians();
        let twist_limit = twist_angle.clamp(self.twist[0].to_radians(), self.twist[1].to_radians());
        if swing_angle <= swing_limit && twist_angle == twist_limit {
            return rotation;
        }

        let swing = if swing_angle > swing_limit {
            Rotation::IDENTITY.interpolate(swing, swing_limit / swing_angle)
        } else {
            swing
        };
        swing.compose(twist(twist_limit))
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Rig {
    pub(super) bones: Vec<Bone>,
    #[serde(default)]
    pub(super) weights: BTreeMap<u64, Vec<Influence>>,
    #[serde(default)]
    pub(super) clips: BTreeMap<String, Clip>,
}

#[derive(Clone, Copy)]
pub(super) struct Rotation(pub(super) [f64; 4]);

impl Rotation {
    const IDENTITY: Self = Self([0.0, 0.0, 0.0, 1.0]);

    fn xyz(degrees: [f64; 3]) -> Self {
        let [x, y, z] = degrees.map(|value| (value % 360.0).to_radians() * 0.5);
        let (sx, cx) = x.sin_cos();
        let (sy, cy) = y.sin_cos();
        let (sz, cz) = z.sin_cos();
        Self([0.0, 0.0, sz, cz])
            .compose(Self([0.0, sy, 0.0, cy]))
            .compose(Self([sx, 0.0, 0.0, cx]))
    }

    fn compose(self, other: Self) -> Self {
        let [ax, ay, az, aw] = self.0;
        let [bx, by, bz, bw] = other.0;
        Self([
            aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz,
        ])
    }

    fn rotate(self, point: [f64; 3]) -> [f64; 3] {
        let [x, y, z, w] = self.0;
        let axis = [x, y, z];
        let tangent = mul(cross(axis, point), 2.0);
        add(add(point, mul(tangent, w)), cross(axis, tangent))
    }

    fn inverse(self) -> Self {
        let [x, y, z, w] = self.0;
        Self([-x, -y, -z, w])
    }

    fn from_to(start: [f64; 3], end: [f64; 3], pole: [f64; 3]) -> Self {
        let a = mul(start, 1.0 / length(start));
        let b = mul(end, 1.0 / length(end));
        let cosine = dot(a, b);
        if cosine >= 1.0 - 1e-12 {
            return Self::IDENTITY;
        }
        if cosine <= -1.0 + 1e-12 {
            let mut axis = cross(a, pole);
            if length(axis) <= 1e-9 {
                let reference = if a[0].abs() < 0.8 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
                axis = cross(a, reference);
            }
            let [x, y, z] = mul(axis, 1.0 / length(axis));
            return Self([x, y, z, 0.0]);
        }
        let [x, y, z] = cross(a, b);
        let w = 1.0 + cosine;
        let scale = (x*x + y*y + z*z + w*w).sqrt();
        Self([x / scale, y / scale, z / scale, w / scale])
    }

    fn euler_xyz(self) -> [f64; 3] {
        let [x, y, z, w] = self.0;
        let sine = 2.0 * (w * y - z * x);
        if sine.abs() >= 1.0 - 1e-12 {
            return [0.0, sine.signum() * 90.0, (2.0 * (w * z - x * y)).atan2(1.0 - 2.0 * (x*x + z*z)).to_degrees()];
        }
        [
            (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x*x + y*y)),
            sine.asin(),
            (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y*y + z*z)),
        ].map(f64::to_degrees)
    }

    fn interpolate(self, other: Self, fraction: f64) -> Self {
        let dot: f64 = self.0.iter().zip(other.0).map(|(a, b)| a * b).sum();
        let sign = if dot < 0.0 { -1.0 } else { 1.0 };
        if dot.abs() > 0.9995 {
            let blended: [f64; 4] = std::array::from_fn(|i| {
                self.0[i] + (sign * other.0[i] - self.0[i]) * fraction
            });
            let length = blended.iter().map(|value| value * value).sum::<f64>().sqrt();
            return Self(blended.map(|value| value / length));
        }

        let angle = dot.abs().acos();
        let start = ((1.0 - fraction) * angle).sin() / angle.sin();
        let end = sign * (fraction * angle).sin() / angle.sin();
        Self(std::array::from_fn(|i| start * self.0[i] + end * other.0[i]))
    }
}

fn sample_track(keys: &[Keyframe], interpolation: Interpolation, time: f64) -> (Rotation, [f64; 3]) {
    let index = keys.partition_point(|key| key.time <= time);
    let start = &keys[index.saturating_sub(1)];
    let start_rotation = Rotation::xyz(start.rotation);
    if index == 0 || index == keys.len() || interpolation == Interpolation::Step {
        return (start_rotation, start.translation);
    }

    let end = &keys[index];
    let mut fraction = (time - start.time) / (end.time - start.time);
    if interpolation == Interpolation::Cubic {
        fraction *= fraction * (3.0 - 2.0 * fraction);
    }
    let rotation = start_rotation.interpolate(Rotation::xyz(end.rotation), fraction);
    (rotation, lerp(start.translation, end.translation, fraction))
}

#[derive(Clone, Copy)]
struct Joint {
    rest: [f64; 3],
    origin: [f64; 3],
    rotation: Rotation,
}

fn sample_joints<'a>(rig: &'a Rig, clip: &Clip, time: f64) -> BTreeMap<&'a str, Joint> {
    let world = Joint { rest: [0.0; 3], origin: [0.0; 3], rotation: Rotation::IDENTITY };
    let mut joints = BTreeMap::new();
    for bone in &rig.bones {
        let parent = bone.parent.as_deref().map_or(&world, |name| &joints[name]);
        let (rotation, translation) = bone.sample(clip, time);
        let offset = add(sub(bone.position, parent.rest), translation);
        let origin = add(parent.origin, parent.rotation.rotate(offset));
        joints.insert(bone.name.as_str(), Joint {
            rest: bone.position,
            origin,
            rotation: parent.rotation.compose(rotation),
        });
    }
    joints
}

fn pose_vertices(mesh: &Mesh, rig: &Rig, joints: &BTreeMap<&str, Joint>) -> Result<BTreeMap<u64, [f64; 3]>, String> {
    if joints.values().any(|joint| joint.origin.iter().any(|value| !value.is_finite())) {
        return Err("pose produces non-finite bone positions".into());
    }
    let mut vertices = mesh.vertices.clone();
    for (id, influences) in &rig.weights {
        let point = mesh.vertices[id];
        let position = influences.iter().fold([0.0; 3], |sum, influence| {
            let joint = &joints[influence.bone.as_str()];
            let transformed = add(joint.origin, joint.rotation.rotate(sub(point, joint.rest)));
            add(sum, mul(transformed, influence.weight))
        });
        if position.iter().any(|value| !value.is_finite()) {
            return Err(format!("pose produces non-finite coordinates at vertex {id}"));
        }
        vertices.insert(*id, position);
    }
    Ok(vertices)
}

fn find_sample<'a>(mesh: &'a Mesh, name: &str, time: f64) -> Result<(&'a Rig, &'a Clip, f64), String> {
    let rig = mesh.rig.as_ref().ok_or("define a rig first")?;
    let clip = rig.clips.get(name).ok_or_else(|| format!("unknown clip {name}"))?;
    let time = if clip.looping { time.rem_euclid(clip.duration) } else { time.clamp(0.0, clip.duration) };
    Ok((rig, clip, time))
}

pub(super) fn sample_pose(mesh: &Mesh, name: &str, time: f64) -> Result<(Mesh, f64), String> {
    let (rig, clip, time) = find_sample(mesh, name, time)?;
    let joints = sample_joints(rig, clip, time);
    let mut posed = mesh.clone();
    posed.vertices = pose_vertices(mesh, rig, &joints)?;
    Ok((posed, time))
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 128
}

pub(super) fn validate(mesh: &Mesh) -> Result<(), String> {
    let Some(rig) = &mesh.rig else { return Ok(()); };
    if !(1..=256).contains(&rig.bones.len()) {
        return Err("a rig requires 1 to 256 bones".into());
    }
    let mut names = BTreeSet::new();
    for bone in &rig.bones {
        if !valid_name(&bone.name) || names.contains(bone.name.as_str()) {
            return Err("bone names must be nonempty and unique".into());
        }
        if bone.parent.as_deref().is_some_and(|parent| !names.contains(parent)) {
            return Err(format!("parent of {} must precede it", bone.name));
        }
        if bone.position.iter().any(|value| !value.is_finite()) {
            return Err("bone positions must be finite".into());
        }
        if let Some(tail) = bone.tail {
            let distance = length(sub(tail, bone.position));
            if !distance.is_finite() || distance <= 0.0 {
                return Err(format!("display tail of {} must be finite and distinct from its pivot", bone.name));
            }
        }
        if let Some(limit) = bone.rotation_limit {
            let axis_length = length(limit.axis);
            let [minimum, maximum] = limit.twist;
            let valid_axis = axis_length.is_finite() && axis_length > 1e-9;
            let valid_twist = (-180.0..=0.0).contains(&minimum) && (0.0..=180.0).contains(&maximum);
            if !valid_axis || !valid_twist || !(0.0..=180.0).contains(&limit.max_swing) {
                return Err(format!("invalid rotation limit for {}: require a nonzero axis, swing in 0..180 and twist bounds spanning zero in -180..180", bone.name));
            }
        }
        names.insert(bone.name.as_str());
    }
    for (vertex, influences) in &rig.weights {
        if !mesh.vertices.contains_key(vertex) || !(1..=4).contains(&influences.len()) {
            return Err(format!("vertex {vertex} must exist and have 1 to 4 influences"));
        }
        let mut bones = BTreeSet::new();
        let mut total = 0.0;
        for influence in influences {
            if !names.contains(influence.bone.as_str())
                || !bones.insert(&influence.bone)
                || !influence.weight.is_finite()
                || influence.weight <= 0.0
            {
                return Err(format!("vertex {vertex} requires unique bones with positive finite weights"));
            }
            total += influence.weight;
        }
        if (total - 1.0).abs() > 1e-6 {
            return Err(format!("weights for vertex {vertex} must sum to one"));
        }
    }
    for (name, clip) in &rig.clips {
        if !valid_name(name) || !clip.duration.is_finite() || clip.duration <= 0.0 || clip.tracks.is_empty() {
            return Err("a clip requires a name, positive finite duration and tracks".into());
        }
        for (bone, keys) in &clip.tracks {
            if !names.contains(bone.as_str()) || keys.is_empty() {
                return Err(format!("clip {name} needs an existing bone and at least one key per track"));
            }
            let mut previous = None;
            for key in keys {
                if !key.time.is_finite() || key.time < 0.0 || key.time > clip.duration
                    || previous.is_some_and(|time| time >= key.time)
                    || key.rotation.iter().any(|value| !value.is_finite())
                    || key.translation.iter().any(|value| !value.is_finite())
                {
                    return Err(format!("clip {name} requires ordered finite keyframes within its duration"));
                }
                previous = Some(key.time);
            }
        }
        if clip.interpolation.keys().any(|bone| !clip.tracks.contains_key(bone)) {
            return Err(format!("clip {name} interpolation references a missing track"));
        }
    }
    Ok(())
}

pub(super) fn reconcile(mesh: &mut Mesh) {
    if let Some(rig) = &mut mesh.rig {
        rig.weights.retain(|vertex, _| mesh.vertices.contains_key(vertex));
    }
}

pub(super) fn sample_query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let name = request["clip"].as_str().ok_or("clip name is required")?;
    let time = request["time"].as_f64().ok_or("time in seconds is required")?;
    let (rig, clip, sampled_time) = find_sample(mesh, name, time)?;
    let joints = sample_joints(rig, clip, sampled_time);
    let vertices = pose_vertices(mesh, rig, &joints)?;
    let (offset, limit) = responses::page(request)?;
    let kind = request.get("kind").map_or(Some("bones"), Value::as_str);
    let (total, items) = match kind {
        Some("bones") => {
            let names = request.get("bones").map(|value| {
                serde_json::from_value::<BTreeSet<String>>(value.clone()).map_err(|_| "bones must contain names")
            }).transpose()?;
            if let Some(names) = &names && names.iter().any(|name| !joints.contains_key(name.as_str())) {
                return Err("unknown bone in pose query".into());
            }
            let items = rig.bones.iter()
                .filter(|bone| names.as_ref().is_none_or(|names| names.contains(&bone.name)));
            let page: Vec<_> = items.clone().skip(offset).take(limit).map(|bone| {
                let joint = joints[bone.name.as_str()];
                let (rotation, translation) = bone.sample(clip, sampled_time);
                let parent_rest = bone.parent.as_deref().map_or([0.0; 3], |name| joints[name].rest);
                let local = add(sub(bone.position, parent_rest), translation);
                json!({"name":bone.name,"parent":bone.parent,"position":joint.origin,
                    "rotation":joint.rotation.0,"local_rotation":rotation.0,"local_translation":local,
                    "translation_offset":translation})
            }).collect();
            (items.count(), page)
        }
        Some("vertices") => {
            let ids = request.get("vertices").map(|value| {
                serde_json::from_value::<BTreeSet<u64>>(value.clone()).map_err(|_| "vertices must contain IDs")
            }).transpose()?;
            if let Some(ids) = &ids && ids.iter().any(|id| !vertices.contains_key(id)) {
                return Err("unknown vertex in pose query".into());
            }
            let items = vertices.iter().filter(|(id, _)| ids.as_ref().is_none_or(|ids| ids.contains(id)));
            let page: Vec<_> = items.clone().skip(offset).take(limit)
                .map(|(id, position)| json!({"id":id,"position":position})).collect();
            (items.count(), page)
        }
        _ => return Err("pose kind must be bones or vertices".into()),
    };
    let bounds = if vertices.is_empty() {
        Value::Null
    } else {
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for point in vertices.values() {
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        json!({"min":min,"max":max})
    };
    let next = offset.saturating_add(limit);
    Ok(json!({"revision":revision,"clip":name,"time":time,"sampled_time":sampled_time,
        "kind":kind,"bounds":bounds,"items":items,"total":total,"offset":offset,
        "next_offset":(next < total).then_some(next)}))
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum KeyEdit {
    Set { bone: String, key: Keyframe },
    Remove { bone: String, time: f64 },
    Retime { bones: Vec<String>, start: f64, end: f64, #[serde(default = "one")] scale: f64, #[serde(default)] shift: f64 },
}

fn one() -> f64 { 1.0 }

fn edit_keys(clip: &mut Clip, request: &Value) -> Result<(), String> {
    let edits: Vec<KeyEdit> = serde_json::from_value(request["edits"].clone()).map_err(|error| error.to_string())?;
    if edits.is_empty() || edits.len() > 4096 {
        return Err("provide 1 to 4096 key edits".into());
    }
    for edit in edits {
        match edit {
            KeyEdit::Set { bone, key } => {
                let keys = clip.tracks.entry(bone).or_default();
                match keys.binary_search_by(|existing| existing.time.total_cmp(&key.time)) {
                    Ok(index) => keys[index] = key,
                    Err(index) => keys.insert(index, key),
                }
            }
            KeyEdit::Remove { bone, time } => {
                let keys = clip.tracks.get_mut(&bone).ok_or_else(|| format!("no track for {bone}"))?;
                let index = keys.iter().position(|key| key.time == time).ok_or_else(|| format!("no key for {bone} at {time}"))?;
                keys.remove(index);
            }
            KeyEdit::Retime { bones, start, end, scale, shift } => {
                if bones.is_empty() || start < 0.0 || start > end || end > clip.duration || scale <= 0.0 {
                    return Err("retime requires bones, a range within the clip and positive scale".into());
                }
                let mut seen = BTreeSet::new();
                for bone in bones {
                    if !seen.insert(bone.clone()) {
                        return Err("retime bones must be unique".into());
                    }
                    let keys = clip.tracks.get_mut(&bone).ok_or_else(|| format!("no track for {bone}"))?;
                    for key in keys.iter_mut().filter(|key| key.time >= start && key.time <= end) {
                        key.time = start + (key.time - start) * scale + shift;
                    }
                    keys.sort_by(|a, b| a.time.total_cmp(&b.time));
                }
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct IkBake {
    chain: [String; 3],
    target: [f64; 3],
    pole: [f64; 3],
    start: f64,
    end: f64,
    #[serde(default = "default_bake_rate")]
    fps: u32,
    tip_rotation: Option<[f64; 3]>,
}

fn default_bake_rate() -> u32 { 30 }

fn bake_ik(rig: &mut Rig, name: &str, request: &Value) -> Result<Value, String> {
    let input: IkBake = serde_json::from_value(request.clone()).map_err(|error| error.to_string())?;
    let [root, middle, tip] = input.chain.each_ref().map(String::as_str);
    let clip = rig.clips.get(name).ok_or_else(|| format!("unknown clip {name}"))?;
    if input.start < 0.0 || input.start > input.end || input.end > clip.duration || !(1..=240).contains(&input.fps) {
        return Err("IK requires a range within the clip and fps between 1 and 240".into());
    }
    let root_bone = rig.bones.iter().find(|bone| bone.name == root).ok_or("unknown chain root")?;
    let middle_bone = rig.bones.iter().find(|bone| bone.name == middle).ok_or("unknown middle bone")?;
    let tip_bone = rig.bones.iter().find(|bone| bone.name == tip).ok_or("unknown tip bone")?;
    if middle_bone.parent.as_deref() != Some(root) || tip_bone.parent.as_deref() != Some(middle) {
        return Err("IK chain must be three directly connected bones in parent order".into());
    }
    let steps = ((input.end - input.start) * f64::from(input.fps)).ceil() as usize;
    if steps > 10_000 {
        return Err("IK bake exceeds 10,001 samples; use a shorter range or lower fps".into());
    }
    let mut baked = [Vec::new(), Vec::new(), Vec::new()];
    let mut max_error: f64 = 0.0;
    let mut clamped = 0;
    for index in 0..=steps {
        let time = if index == steps { input.end } else {
            input.start + (input.end - input.start) * index as f64 / steps as f64
        };
        let joints = sample_joints(rig, clip, time);
        let parent = root_bone.parent.as_deref().map_or(Rotation::IDENTITY, |bone| joints[bone].rotation);
        let pivot = joints[root].origin;
        let target = sub(input.target, pivot);
        let distance = length(target);
        let (middle_local, middle_translation) = middle_bone.sample(clip, time);
        let (_, tip_translation) = clip.track(tip, time);
        let upper = add(sub(middle_bone.position, root_bone.position), middle_translation);
        let lower = add(sub(tip_bone.position, middle_bone.position), tip_translation);
        let upper_length = length(upper);
        let lower_length = length(lower);
        if [upper_length, lower_length, distance].iter().any(|value| !value.is_finite() || *value <= 1e-8) {
            return Err("IK requires nonzero segment lengths and target distance".into());
        }
        let direction = mul(target, 1.0 / distance);
        let mut bend = sub(input.pole, pivot);
        bend = sub(bend, mul(direction, dot(bend, direction)));
        let bend_length = length(bend);
        if bend_length <= 1e-8 || !bend_length.is_finite() {
            return Err("IK pole must be off the root-to-target axis".into());
        }
        let reach = distance.clamp((upper_length - lower_length).abs().max(1e-9), upper_length + lower_length);
        let along = (upper_length * upper_length - lower_length * lower_length + reach * reach) / (2.0 * reach);
        let across = (upper_length * upper_length - along * along).max(0.0).sqrt();
        let elbow = add(mul(direction, along), mul(bend, across / bend_length));
        let forearm = sub(mul(direction, reach), elbow);
        let root_world = Rotation::from_to(joints[root].rotation.rotate(upper), elbow, bend).compose(joints[root].rotation);
        let middle_guess = root_world.compose(middle_local);
        let middle_world = Rotation::from_to(middle_guess.rotate(lower), forearm, bend).compose(middle_guess);
        let tip_world = input.tip_rotation.map_or(joints[tip].rotation, Rotation::xyz);
        let rotations = [parent.inverse().compose(root_world), root_world.inverse().compose(middle_world), middle_world.inverse().compose(tip_world)];
        let posed_root = parent.compose(root_bone.rotation_limit.map_or(rotations[0], |limit| limit.apply(rotations[0])));
        let posed_middle = posed_root.compose(middle_bone.rotation_limit.map_or(rotations[1], |limit| limit.apply(rotations[1])));
        let posed_target = add(posed_root.rotate(upper), posed_middle.rotate(lower));
        let error = length(sub(posed_target, target));
        max_error = max_error.max(error);
        clamped += usize::from(error > 1e-8);
        for (index, bone) in [root, middle, tip].into_iter().enumerate() {
            baked[index].push(Keyframe {
                time,
                rotation: rotations[index].euler_xyz(),
                translation: clip.track(bone, time).1,
            });
        }
    }
    let frame = 1.0 / f64::from(input.fps);
    for (index, bone) in [root, middle, tip].into_iter().enumerate() {
        for time in [
            (input.start > 0.0).then_some((input.start - frame).max(0.0)),
            (input.end < clip.duration).then_some((input.end + frame).min(clip.duration)),
        ].into_iter().flatten() {
            if time == input.start || time == input.end
                || clip.tracks.get(bone).is_some_and(|keys| keys.iter().any(|key| key.time == time))
            {
                continue;
            }
            let (rotation, translation) = clip.track(bone, time);
            baked[index].push(Keyframe { time, rotation: rotation.euler_xyz(), translation });
        }
    }
    let clip = rig.clips.get_mut(name).unwrap();
    for (bone, baked) in input.chain.into_iter().zip(baked) {
        let keys = clip.tracks.entry(bone).or_default();
        keys.retain(|key| key.time < input.start || key.time > input.end);
        keys.extend(baked);
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    }
    Ok(json!({"name":name,"samples":steps + 1,"clamped_samples":clamped,"max_target_error":max_error}))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WeightEasing {
    Linear,
    Smoothstep,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WeightRamp {
    bones: [String; 2],
    start: [f64; 3],
    end: [f64; 3],
    easing: WeightEasing,
}

fn weight_ramp(mesh: &Mesh, request: &Value) -> Result<BTreeMap<u64, Vec<Influence>>, String> {
    let ramp: WeightRamp = serde_json::from_value(request["ramp"].clone())
        .map_err(|error| error.to_string())?;
    if ramp.bones[0] == ramp.bones[1] {
        return Err("weight ramp requires two distinct bones".into());
    }
    let rig = mesh.rig.as_ref().ok_or("define a rig first")?;
    for name in &ramp.bones {
        if !rig.bones.iter().any(|bone| bone.name == *name) {
            return Err(format!("unknown weight ramp bone {name}"));
        }
    }
    let span = sub(ramp.end, ramp.start);
    let span_length = length(span);
    if !span_length.is_finite() || span_length <= 1e-12 {
        return Err("weight ramp start and end must define a finite nonzero span".into());
    }
    let axis = mul(span, 1.0 / span_length);
    let vertices: BTreeSet<u64> = match (request.get("object"), request.get("vertices")) {
        (Some(_), Some(_)) => return Err("scope weight ramp by object or vertices".into()),
        (Some(object), None) => {
            let object = object.as_u64().and_then(|id| mesh.objects.get(&id))
                .ok_or("unknown weight ramp object")?;
            object.vertices.clone()
        }
        (None, Some(vertices)) => serde_json::from_value(vertices.clone())
            .map_err(|_| "vertices must contain vertex IDs")?,
        (None, None) => mesh.selection.clone(),
    };
    if vertices.is_empty() {
        return Err("weight ramp scope is empty".into());
    }

    let mut weights = BTreeMap::new();
    for vertex in vertices {
        let point = mesh.vertices.get(&vertex).ok_or_else(|| format!("unknown vertex {vertex}"))?;
        let mut t = dot(sub(*point, ramp.start), axis) / span_length;
        if !t.is_finite() {
            return Err("weight ramp projection must be finite".into());
        }
        t = t.clamp(0.0, 1.0);
        if matches!(ramp.easing, WeightEasing::Smoothstep) {
            t = t * t * (3.0 - 2.0 * t);
        }
        let influences = ramp.bones.iter().zip([1.0 - t, t])
            .filter(|(_, weight)| *weight > 0.0)
            .map(|(bone, weight)| Influence { bone: bone.clone(), weight })
            .collect();
        weights.insert(vertex, influences);
    }
    Ok(weights)
}

fn rigid_weights(mesh: &Mesh, request: &Value, bone: &Value) -> Result<BTreeMap<u64, Vec<Influence>>, String> {
    let rig = mesh.rig.as_ref().ok_or("define a rig first")?;
    let bone = bone.as_str().ok_or("bone must be a name")?;
    if !rig.bones.iter().any(|candidate| candidate.name == bone) {
        return Err(format!("unknown bone {bone}"));
    }
    if request.get("vertices").is_some() {
        return Err("rigid binding uses object scope".into());
    }
    let object = request["object"].as_u64().and_then(|id| mesh.objects.get(&id))
        .ok_or("rigid binding requires an existing object")?;
    let influences = vec![Influence { bone: bone.to_owned(), weight: 1.0 }];
    Ok(object.vertices.iter().map(|&id| (id, influences.clone())).collect())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurveChannel {
    #[serde(default)]
    rotation: [f64; 3],
    #[serde(default)]
    translation: [f64; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedCurve {
    keys: Vec<[f64; 2]>,
    channels: BTreeMap<String, CurveChannel>,
}

fn animation_clip(request: &Value) -> Result<Clip, String> {
    let mut input = request["clip"].clone();
    let Some(curve) = request.get("shared_curve") else {
        return serde_json::from_value(input).map_err(|error| error.to_string());
    };
    let curve: SharedCurve = serde_json::from_value(curve.clone()).map_err(|error| error.to_string())?;
    if curve.keys.is_empty() || curve.channels.is_empty() {
        return Err("shared_curve requires keys and channels".into());
    }
    if let Some(clip) = input.as_object_mut() {
        clip.entry("tracks").or_insert_with(|| json!({}));
    }
    let mut clip: Clip = serde_json::from_value(input).map_err(|error| error.to_string())?;
    for (bone, channel) in curve.channels {
        if clip.tracks.contains_key(&bone) {
            return Err(format!("shared curve channel overlaps explicit track {bone}"));
        }
        let keys = curve.keys.iter().map(|&[time, value]| Keyframe {
            time,
            rotation: mul(channel.rotation, value),
            translation: mul(channel.translation, value),
        }).collect();
        clip.tracks.insert(bone, keys);
    }
    Ok(clip)
}

pub(super) fn edit(mesh: &mut Mesh, command: &str, request: &Value) -> Result<Value, String> {
    let mut operation = None;
    match command {
        "rig_set" => {
            let rig: Rig = serde_json::from_value(request["rig"].clone()).map_err(|error| error.to_string())?;
            mesh.rig = Some(rig);
        }
        "rig_clear" => mesh.rig = None,
        "rig_display_tails_set" => {
            let rig = mesh.rig.as_mut().ok_or("define a rig first")?;
            let tails: BTreeMap<String, Option<[f64; 3]>> =
                serde_json::from_value(request["tails"].clone()).map_err(|error| error.to_string())?;
            for (name, tail) in tails {
                let bone = rig.bones.iter_mut().find(|bone| bone.name == name)
                    .ok_or_else(|| format!("unknown bone {name}"))?;
                bone.tail = tail;
            }
        }
        "rig_rotation_limits_set" => {
            let rig = mesh.rig.as_mut().ok_or("define a rig first")?;
            let limits: BTreeMap<String, Option<RotationLimit>> =
                serde_json::from_value(request["limits"].clone()).map_err(|error| error.to_string())?;
            for (name, limit) in limits {
                let bone = rig.bones.iter_mut().find(|bone| bone.name == name)
                    .ok_or_else(|| format!("unknown bone {name}"))?;
                bone.rotation_limit = limit;
            }
        }
        "skin_weights_set" => {
            let weights: BTreeMap<u64, Vec<Influence>> = match (request.get("weights"), request.get("ramp"), request.get("bone")) {
                (Some(weights), None, None) => serde_json::from_value(weights.clone())
                    .map_err(|error| error.to_string())?,
                (None, Some(_), None) => weight_ramp(mesh, request)?,
                (None, None, Some(bone)) => rigid_weights(mesh, request, bone)?,
                _ => return Err("provide exactly one of weights, ramp or bone".into()),
            };
            let rig = mesh.rig.as_mut().ok_or("define a rig first")?;
            for (id, influences) in weights {
                if !mesh.vertices.contains_key(&id) {
                    return Err(format!("unknown vertex {id}"));
                }
                if influences.is_empty() {
                    rig.weights.remove(&id);
                } else {
                    rig.weights.insert(id, influences);
                }
            }
        }
        "animation_set" | "animation_update" | "animation_remove" | "animation_track_set" | "animation_track_remove" | "animation_keys_edit" | "animation_ik_bake" => {
            let rig = mesh.rig.as_mut().ok_or("define a rig first")?;
            let name = request["name"].as_str().filter(|name| valid_name(name)).ok_or("clip name is required")?;
            match command {
                "animation_set" => {
                    let clip = animation_clip(request)?;
                    rig.clips.insert(name.into(), clip);
                }
                "animation_remove" => {
                    if rig.clips.remove(name).is_none() {
                        return Err(format!("unknown clip {name}"));
                    }
                }
                "animation_update" => {
                    let clip = rig.clips.get_mut(name).ok_or_else(|| format!("unknown clip {name}"))?;
                    let duration = request.get("duration").map(|value| value.as_f64().ok_or("duration must be a number")).transpose()?;
                    let looping = request.get("looping").map(|value| value.as_bool().ok_or("looping must be a boolean")).transpose()?;
                    if duration.is_none() && looping.is_none() {
                        return Err("provide duration or looping".into());
                    }
                    clip.duration = duration.unwrap_or(clip.duration);
                    clip.looping = looping.unwrap_or(clip.looping);
                }
                "animation_keys_edit" => {
                    let clip = rig.clips.get_mut(name).ok_or_else(|| format!("unknown clip {name}"))?;
                    edit_keys(clip, request)?;
                }
                "animation_ik_bake" => operation = Some(bake_ik(rig, name, request)?),
                _ => {
                    let clip = rig.clips.get_mut(name).ok_or_else(|| format!("unknown clip {name}"))?;
                    let bone = request["bone"].as_str().filter(|bone| valid_name(bone)).ok_or("bone name is required")?;
                    if command == "animation_track_set" {
                        if let Some(keys) = request.get("keys") {
                            let keys = serde_json::from_value(keys.clone()).map_err(|error| error.to_string())?;
                            clip.tracks.insert(bone.into(), keys);
                        } else if request.get("interpolation").is_none() {
                            return Err("provide keys or interpolation".into());
                        }
                        if let Some(mode) = request.get("interpolation") {
                            let mode = serde_json::from_value(mode.clone()).map_err(|error| error.to_string())?;
                            clip.interpolation.insert(bone.into(), mode);
                        }
                    } else {
                        if clip.tracks.remove(bone).is_none() {
                            return Err(format!("clip {name} has no track for {bone}"));
                        }
                        clip.interpolation.remove(bone);
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    validate(mesh)?;
    Ok(operation.unwrap_or_else(|| summary(mesh)))
}

pub(super) fn query(mesh: &Mesh, request: &Value) -> Value {
    if request["response"] == "full" {
        json!({"rig":mesh.rig})
    } else {
        summary(mesh)
    }
}

pub(super) fn query_animation(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let (offset, limit) = responses::page(request)?;
    let name = request.get("clip").map(|clip| clip.as_str().ok_or("clip must be a name")).transpose()?;
    let bone = request.get("bone").map(|bone| bone.as_str().ok_or("bone must be a name")).transpose()?;
    let clips = mesh.rig.as_ref().map(|rig| &rig.clips);
    let (kind, metadata, total, items) = match (name, bone) {
        (None, None) => {
            let total = clips.map_or(0, BTreeMap::len);
            let items: Vec<_> = clips.into_iter().flatten().skip(offset).take(limit)
                .map(|(name, clip)| clip_summary(name, clip)).collect();
            ("clips", Value::Null, total, json!(items))
        }
        (None, Some(_)) => return Err("a bone query requires a clip name".into()),
        (Some(name), bone) => {
            let clip = clips.and_then(|clips| clips.get(name)).ok_or_else(|| format!("unknown clip {name}"))?;
            let metadata = clip_summary(name, clip);
            if let Some(bone) = bone {
                let keys = clip.tracks.get(bone).ok_or_else(|| format!("clip {name} has no track for {bone}"))?;
                let page = &keys[offset.min(keys.len())..offset.saturating_add(limit).min(keys.len())];
                ("keys", metadata, keys.len(), json!(page))
            } else {
                let items: Vec<_> = clip.tracks.iter().skip(offset).take(limit).map(|(bone, keys)| {
                    json!({"bone":bone,"keyframes":keys.len(),"first_time":keys.first().map(|key| key.time),"last_time":keys.last().map(|key| key.time),"interpolation":clip.interpolation(bone)})
                }).collect();
                ("tracks", metadata, clip.tracks.len(), json!(items))
            }
        }
    };
    let next = offset.saturating_add(limit);
    let mut result = json!({
        "kind":kind,"items":items,"total":total,"offset":offset,
        "next_offset":(next < total).then_some(next),"revision":revision
    });
    if name.is_some() {
        result["clip"] = metadata;
    }
    if let Some(bone) = bone {
        result["bone"] = json!(bone);
    }
    Ok(result)
}

fn clip_summary(name: &str, clip: &Clip) -> Value {
    json!({"name":name,"duration":clip.duration,"looping":clip.looping,"tracks":clip.tracks.len()})
}

fn summary(mesh: &Mesh) -> Value {
    let Some(rig) = &mesh.rig else { return json!({"bones":[],"weighted_vertices":0,"clips":[]}); };
    let clips: Vec<_> = rig.clips.iter().map(|(name, clip)| clip_summary(name, clip)).collect();
    json!({"bones":rig.bones,"weighted_vertices":rig.weights.len(),"clips":clips})
}
