use crate::{Mesh, geometry::{add, sub}, normals, rig::Interpolation};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Buffer {
    fn accessor(
        &mut self,
        bytes: impl IntoIterator<Item = u8>,
        component: u32,
        shape: &str,
        count: usize,
        bounds: Option<(Value, Value)>,
    ) -> usize {
        let offset = self.bytes.len();
        self.bytes.extend(bytes);
        let size = self.bytes.len() - offset;
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
        let view = self.views.len();
        self.views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":size}));
        let mut accessor = json!({"bufferView":view,"componentType":component,"count":count,"type":shape});
        if let Some((min, max)) = bounds {
            accessor["min"] = min;
            accessor["max"] = max;
        }
        let id = self.accessors.len();
        self.accessors.push(accessor);
        id
    }

    fn floats<const N: usize>(
        &mut self,
        values: &[[f64; N]],
        shape: &str,
        bounded: bool,
    ) -> Result<usize, String> {
        let mut min = [f32::INFINITY; N];
        let mut max = [f32::NEG_INFINITY; N];
        let mut bytes = Vec::with_capacity(values.len() * N * 4);
        for value in values {
            for (axis, value) in value.iter().enumerate() {
                let value = *value as f32;
                if !value.is_finite() {
                    return Err("GLB coordinate exceeds float32 range".into());
                }
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
                bytes.extend(value.to_le_bytes());
            }
        }
        let bounds = bounded.then(|| (json!(min.as_slice()), json!(max.as_slice())));
        Ok(self.accessor(bytes, 5126, shape, values.len(), bounds))
    }
}

fn pack(document: &Value, binary: &[u8]) -> Result<Vec<u8>, String> {
    let mut metadata = serde_json::to_vec(document).map_err(|error| error.to_string())?;
    while metadata.len() % 4 != 0 {
        metadata.push(b' ');
    }
    let total = 28 + metadata.len() + binary.len();
    let total = u32::try_from(total).map_err(|_| "GLB exceeds the maximum file size")?;
    let mut output = Vec::with_capacity(total as usize);
    output.extend(b"glTF");
    output.extend(2u32.to_le_bytes());
    output.extend(total.to_le_bytes());
    output.extend((metadata.len() as u32).to_le_bytes());
    output.extend(b"JSON");
    output.extend(metadata);
    output.extend((binary.len() as u32).to_le_bytes());
    output.extend(b"BIN\0");
    output.extend(binary);
    Ok(output)
}

pub(super) fn export(mesh: &Mesh) -> Result<Vec<u8>, String> {
    let rig = mesh.rig.as_ref().ok_or("animated GLB export requires a rig")?;
    let mut buffer = Buffer::default();
    let indices: BTreeMap<_, _> = rig.bones.iter().enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index)).collect();
    let mut nodes = vec![json!({"name":"Skeleton","children":[]})];
    let mut offsets = Vec::new();
    let mut inverse = Vec::new();
    for bone in &rig.bones {
        let parent = bone.parent.as_deref().map(|name| indices[name]);
        let parent_position = parent.map_or([0.0; 3], |index| rig.bones[index].position);
        let offset = sub(bone.position, parent_position);
        let child = nodes.len();
        let parent_node = parent.map_or(0, |index| index + 1);
        nodes[parent_node]["children"].as_array_mut().unwrap().push(json!(child));
        nodes.push(json!({"name":bone.name,"translation":offset,"children":[]}));
        offsets.push(offset);
        let [x, y, z] = bone.position;
        inverse.push([1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, -x,-y,-z,1.0]);
    }
    let static_index = rig.bones.len();
    if mesh.vertices.keys().any(|vertex| !rig.weights.contains_key(vertex)) {
        let child = nodes.len();
        nodes[0]["children"].as_array_mut().unwrap().push(json!(child));
        nodes.push(json!({"name":"Unweighted vertices"}));
        inverse.push([1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0]);
    }
    for node in &mut nodes {
        if node["children"].as_array().is_some_and(Vec::is_empty) {
            node.as_object_mut().unwrap().remove("children");
        }
    }
    let joints: Vec<_> = (1..nodes.len()).collect();
    let inverse_accessor = buffer.floats(&inverse, "MAT4", false)?;
    let mut scene_nodes = vec![0];
    let mut meshes = Vec::new();
    let normals = normals::corners(mesh);
    for object in mesh.objects.values() {
        let mut positions = Vec::new();
        let mut shading = Vec::new();
        let mut uv = Vec::new();
        let mut colors = Vec::new();
        let mut triangles = Vec::new();
        let mut bindings = Vec::<[u16; 4]>::new();
        let mut weights = Vec::new();
        let mut has_uv = false;
        let mut has_color = false;
        for (face_id, face) in mesh.faces.iter().filter(|(_, face)| object.vertices.contains(&face[0])) {
            let base = u32::try_from(positions.len()).map_err(|_| "too many GLB vertices")?;
            for triangle in normals::triangles(mesh, face) {
                triangles.extend(triangle.map(|corner| base + corner as u32));
            }
            for (corner, id) in face.iter().enumerate() {
                positions.push(mesh.vertices[id]);
                let normal = normals[face_id][corner];
                shading.push(if normal == [0.0; 3] { [0.0, 1.0, 0.0] } else { normal });
                let texel = mesh.uvs.get(face_id).and_then(|uv| uv.get(id));
                has_uv |= texel.is_some();
                let [u, v] = texel.copied().unwrap_or([0.0; 2]);
                uv.push([u, 1.0 - v]);
                let color = mesh.vertex_colors.get(id);
                has_color |= color.is_some();
                colors.push(color.copied().unwrap_or([1.0; 3]));
                let mut joint = [0; 4];
                let mut weight = [0.0; 4];
                if let Some(influences) = rig.weights.get(id) {
                    let sum: f64 = influences.iter().map(|influence| influence.weight).sum();
                    for (index, influence) in influences.iter().enumerate() {
                        joint[index] = indices[influence.bone.as_str()] as u16;
                        weight[index] = influence.weight / sum;
                    }
                } else {
                    joint[0] = static_index as u16;
                    weight[0] = 1.0;
                }
                bindings.push(joint);
                weights.push(weight);
            }
        }
        if triangles.is_empty() {
            continue;
        }
        let position = buffer.floats(&positions, "VEC3", true)?;
        let normal = buffer.floats(&shading, "VEC3", false)?;
        let weight = buffer.floats(&weights, "VEC4", false)?;
        let joint = buffer.accessor(bindings.iter().flatten().flat_map(|value| value.to_le_bytes()), 5123, "VEC4", bindings.len(), None);
        let triangles = buffer.accessor(triangles.iter().flat_map(|value| value.to_le_bytes()), 5125, "SCALAR", triangles.len(), None);
        let mut attributes = json!({"POSITION":position,"NORMAL":normal,"JOINTS_0":joint,"WEIGHTS_0":weight});
        if has_uv {
            attributes["TEXCOORD_0"] = json!(buffer.floats(&uv, "VEC2", false)?);
        }
        if has_color {
            attributes["COLOR_0"] = json!(buffer.floats(&colors, "VEC3", false)?);
        }
        scene_nodes.push(nodes.len());
        nodes.push(json!({"name":object.name,"mesh":meshes.len(),"skin":0}));
        meshes.push(json!({"name":object.name,"primitives":[{"attributes":attributes,"indices":triangles}]}));
    }

    let mut animations = Vec::new();
    for (name, clip) in &rig.clips {
        let mut channels = Vec::new();
        let mut samplers = Vec::new();
        for (bone, keys) in &clip.tracks {
            let index = indices[bone.as_str()];
            let mode = clip.interpolation(bone);
            let mut times = vec![0.0];
            times.extend(keys.iter().map(|key| key.time).filter(|time| *time > 0.0 && *time < clip.duration));
            times.push(clip.duration);
            if mode == Interpolation::Cubic {
                let mut baked = Vec::new();
                for interval in times.windows(2) {
                    let span = interval[1] - interval[0];
                    let steps = (span * 60.0).ceil().max(1.0) as usize;
                    if baked.len().saturating_add(steps) > 200_000 {
                        return Err("cubic GLB export exceeds 200,000 samples on one track".into());
                    }
                    baked.extend((0..steps).map(|step| interval[0] + span * step as f64 / steps as f64));
                }
                baked.push(clip.duration);
                times = baked;
            }
            if times.windows(2).any(|pair| (pair[0] as f32) >= (pair[1] as f32)) {
                return Err(format!("clip {name} has sample times too close for GLB float32 precision"));
            }
            let input_times: Vec<_> = times.iter().map(|&time| [time]).collect();
            let input = buffer.floats(&input_times, "SCALAR", true)?;
            let mut rotations = Vec::<[f64; 4]>::new();
            let mut translations = Vec::new();
            for time in times {
                let (rotation, translation) = clip.track(bone, time);
                let mut quaternion = rotation.0;
                if rotations.last().is_some_and(|previous| previous.iter().zip(quaternion).map(|(a,b)| a*b).sum::<f64>() < 0.0) {
                    quaternion = quaternion.map(|value| -value);
                }
                rotations.push(quaternion);
                translations.push(add(offsets[index], translation));
            }
            let rotation = buffer.floats(&rotations, "VEC4", false)?;
            let translation = buffer.floats(&translations, "VEC3", false)?;
            let interpolation = if mode == Interpolation::Step { "STEP" } else { "LINEAR" };
            for (path, output) in [("rotation", rotation), ("translation", translation)] {
                channels.push(json!({"sampler":samplers.len(),"target":{"node":index+1,"path":path}}));
                samplers.push(json!({"input":input,"output":output,"interpolation":interpolation}));
            }
        }
        animations.push(json!({"name":name,"channels":channels,"samplers":samplers,"extras":{"looping":clip.looping,"duration":clip.duration}}));
    }

    let mut document = json!({
        "asset":{"version":"2.0","generator":"gpt-mesh"},"scene":0,"scenes":[{"nodes":scene_nodes}],
        "nodes":nodes,"meshes":meshes,"skins":[{"joints":joints,"skeleton":0,"inverseBindMatrices":inverse_accessor}],
        "accessors":buffer.accessors,"bufferViews":buffer.views,"buffers":[{"byteLength":buffer.bytes.len()}]
    });
    if !animations.is_empty() {
        document["animations"] = json!(animations);
    }
    pack(&document, &buffer.bytes)
}
