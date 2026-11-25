use crate::{Mesh, geometry::{add, sub, mul, cross, dot}, normals, rig::Interpolation};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Buffer {
    fn view(&mut self, bytes: impl IntoIterator<Item = u8>) -> usize {
        let offset = self.bytes.len();
        self.bytes.extend(bytes);
        let size = self.bytes.len() - offset;
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
        let view = self.views.len();
        self.views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":size}));
        view
    }

    fn accessor(
        &mut self,
        bytes: impl IntoIterator<Item = u8>,
        component: u32,
        shape: &str,
        count: usize,
        bounds: Option<(Value, Value)>,
    ) -> usize {
        let view = self.view(bytes);
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

fn material(
    name: &str,
    texture: &crate::uv::Texture,
    images: &mut Vec<Value>,
    buffer: &mut Buffer,
) -> Result<Value, String> {
    let mut embed = |label: &str, pixels: &[u8]| -> Result<Value, String> {
        let png = crate::uv::encode_png(texture, pixels)?;
        let index = images.len();
        images.push(json!({"name":format!("{name} {label}"),"mimeType":"image/png","bufferView":buffer.view(png)}));
        Ok(json!({"index":index}))
    };
    let mut base = crate::uv::pixels(texture)?;
    if texture.alpha_cutoff.is_none() {
        for pixel in base.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    let base_color = embed("base color", &base)?;
    let mut material = json!({"name":name,"doubleSided":texture.double_sided,"pbrMetallicRoughness":{"baseColorTexture":base_color}});
    if let Some(cutoff) = texture.alpha_cutoff {
        material["alphaMode"] = json!("MASK");
        material["alphaCutoff"] = json!(cutoff);
        material["extensions"] = json!({"KHR_materials_unlit":{}});
        return Ok(material);
    }
    if texture.maps.contains_key("normal") {
        let mut pixels = crate::painting::map_pixels(texture, "normal")?;
        // The exported UVs reverse V, so the generated bitangent reverses too.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[1] = 255 - pixel[1];
        }
        material["normalTexture"] = embed("normal", &pixels)?;
    }
    let pbr = &mut material["pbrMetallicRoughness"];
    if texture.maps.contains_key("metallic") || texture.maps.contains_key("roughness") {
        let metallic = crate::painting::map_pixels(texture, "metallic")?;
        let mut packed = crate::painting::map_pixels(texture, "roughness")?;
        for (pixel, metal) in packed.chunks_exact_mut(4).zip(metallic.chunks_exact(4)) {
            pixel.copy_from_slice(&[255, pixel[0], metal[0], 255]);
        }
        pbr["metallicRoughnessTexture"] = embed("metallic roughness", &packed)?;
    } else {
        pbr["metallicFactor"] = json!(0);
        pbr["roughnessFactor"] = json!(166.0 / 255.0);
    }
    Ok(material)
}

fn tangents(
    positions: &[[f64; 3]],
    normals: &[[f64; 3]],
    uvs: &[[f64; 2]],
    indices: &[u32],
) -> Vec<[f64; 4]> {
    let mut frames = vec![[[0.0; 3]; 2]; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let ids = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let Some([t, b]) = crate::material::frame(ids.map(|i| positions[i]), ids.map(|i| uvs[i]))
        else {
            continue;
        };
        for i in ids {
            frames[i][0] = add(frames[i][0], t);
            frames[i][1] = add(frames[i][1], b);
        }
    }
    frames
        .into_iter()
        .zip(normals)
        .map(|([t, b], &n)| {
            let mut tangent = normals::unit(sub(t, mul(n, dot(t, n))));
            if tangent == [0.0; 3] {
                let axis = if n[0].abs() < 0.8 {
                    [1.0, 0.0, 0.0]
                } else {
                    [0.0, 1.0, 0.0]
                };
                tangent = normals::unit(cross(n, axis));
            }
            let hand = if dot(cross(n, tangent), b) < 0.0 {
                -1.0
            } else {
                1.0
            };
            [tangent[0], tangent[1], tangent[2], hand]
        })
        .collect()
}

pub(super) fn export(mesh: &Mesh, visibility: &crate::render::CaptureVisibility, merge: Option<&str>) -> Result<Vec<u8>, String> {
    let rig = mesh.rig.as_ref();
    let bones = rig.map_or(&[][..], |rig| rig.bones.as_slice());
    let mut buffer = Buffer::default();
    let indices: BTreeMap<_, _> = bones.iter().enumerate()
        .map(|(index, bone)| (bone.name.as_str(), index)).collect();
    let mut nodes = Vec::new();
    let mut scene_nodes = Vec::new();
    if rig.is_some() {
        nodes.push(json!({"name":"Skeleton","children":[]}));
        scene_nodes.push(0);
    }
    let mut offsets = Vec::new();
    let mut inverse = Vec::new();
    for bone in bones {
        let parent = bone.parent.as_deref().map(|name| indices[name]);
        let parent_position = parent.map_or([0.0; 3], |index| bones[index].position);
        let offset = sub(bone.position, parent_position);
        let child = nodes.len();
        let parent_node = parent.map_or(0, |index| index + 1);
        nodes[parent_node]["children"].as_array_mut().unwrap().push(json!(child));
        nodes.push(json!({"name":bone.name,"translation":offset,"children":[]}));
        offsets.push(offset);
        let [x, y, z] = bone.position;
        inverse.push([1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, -x,-y,-z,1.0]);
    }
    let static_index = bones.len();
    if rig.is_some_and(|rig| mesh.vertices.keys().any(|v| visibility.includes_vertex(v) && !rig.weights.contains_key(v))) {
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
    let skin = if rig.is_some() {
        Some(json!({"joints":joints,"skeleton":0,"inverseBindMatrices":buffer.floats(&inverse, "MAT4", false)?}))
    } else {
        None
    };
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    let mut image_textures = Vec::new();
    let mut texture_samplers = Vec::new();
    let mut texture_materials = BTreeMap::new();
    let normals = normals::corners(mesh);
    for group in crate::export_groups::groups(mesh,visibility,merge)? {
        let object = group.members[0].1;
        let name = group.name;
        let vertices = group.vertices();
        let mut groups = BTreeMap::<Option<&str>, Vec<_>>::new();
        for (face_id, face) in mesh.faces.iter().filter(|(_, face)| vertices.contains(&face[0])) {
            let name = mesh.face_textures.get(face_id).map(String::as_str);
            let binding = name.filter(|_| crate::uv::complete(mesh, *face_id));
            groups.entry(binding).or_default().push((face_id, face));
        }
        let mut primitives = Vec::new();
        for (texture, faces) in groups {
            let mut positions = Vec::new();
            let mut shading = Vec::new();
            let mut uv = Vec::new();
            let mut colors = Vec::new();
            let mut triangles = Vec::new();
            let mut bindings = Vec::<[u16; 4]>::new();
            let mut weights = Vec::new();
            let mut has_uv = false;
            let mut has_color = false;
            for (face_id, face) in faces {
                let base = u32::try_from(positions.len()).map_err(|_| "too many GLB vertices")?;
                for triangle in normals::triangles(mesh, face) {
                    triangles.extend(triangle.map(|corner| base + corner as u32));
                }
                for (corner, id) in face.iter().enumerate() {
                    let point = mesh.vertices[id];
                    positions.push(if rig.is_some() { point } else { sub(point, object.origin) });
                    let normal = normals[face_id][corner];
                    shading.push(if normal == [0.0; 3] { [0.0, 1.0, 0.0] } else { normal });
                    let texel = mesh.uvs.get(face_id).and_then(|uv| uv.get(id));
                    has_uv |= texel.is_some();
                    let [u, v] = texel.copied().unwrap_or([0.0; 2]);
                    uv.push([u, 1.0 - v]);
                    let color = mesh.vertex_colors.get(id);
                    has_color |= color.is_some();
                    colors.push(color.copied().unwrap_or([1.0; 3]));
                    let Some(rig) = rig else { continue };
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
            let mut attributes = json!({"POSITION":position,"NORMAL":normal});
            if texture.is_some_and(|name| mesh.textures[name].alpha_cutoff.is_none() && mesh.textures[name].maps.contains_key("normal")) {
                let tangents = tangents(&positions, &shading, &uv, &triangles);
                attributes["TANGENT"] = json!(buffer.floats(&tangents, "VEC4", false)?);
            }
            let triangles = buffer.accessor(triangles.iter().flat_map(|value| value.to_le_bytes()), 5125, "SCALAR", triangles.len(), None);
            if rig.is_some() {
                attributes["WEIGHTS_0"] = json!(buffer.floats(&weights, "VEC4", false)?);
                attributes["JOINTS_0"] = json!(buffer.accessor(bindings.iter().flatten().flat_map(|value| value.to_le_bytes()), 5123, "VEC4", bindings.len(), None));
            }
            if has_uv {
                attributes["TEXCOORD_0"] = json!(buffer.floats(&uv, "VEC2", false)?);
            }
            if has_color {
                attributes["COLOR_0"] = json!(buffer.floats(&colors, "VEC3", false)?);
            }
            let mut primitive = json!({"attributes":attributes,"indices":triangles});
            let element_view = buffer.accessors[triangles]["bufferView"].as_u64().unwrap() as usize;
            buffer.views[element_view]["target"] = json!(34963);
            for accessor in primitive["attributes"].as_object().unwrap().values() {
                let index = accessor.as_u64().unwrap() as usize;
                let view = buffer.accessors[index]["bufferView"].as_u64().unwrap() as usize;
                buffer.views[view]["target"] = json!(34962);
            }
            if let Some(name) = texture {
                let material = if let Some(&material) = texture_materials.get(name) {
                    material
                } else {
                    let index = materials.len();
                    let texture = &mesh.textures[name];
                    let first = images.len();
                    materials.push(material(name, texture, &mut images, &mut buffer)?);
                    let sampler = match texture.filter.unwrap_or_default() {
                        crate::uv::Filter::Nearest => json!({"magFilter":9728,"minFilter":9728}),
                        crate::uv::Filter::Linear => json!({"magFilter":9729,"minFilter":9987}),
                    };
                    let sampler_index = texture_samplers.iter().position(|entry| entry == &sampler).unwrap_or_else(|| {
                        texture_samplers.push(sampler);
                        texture_samplers.len() - 1
                    });
                    image_textures.extend((first..images.len()).map(|source| json!({"sampler":sampler_index,"source":source})));
                    texture_materials.insert(name, index);
                    index
                };
                primitive["material"] = json!(material);
            }
            primitives.push(primitive);
        }
        if primitives.is_empty() {
            continue;
        }
        scene_nodes.push(nodes.len());
        let mut node = json!({"name":name,"mesh":meshes.len()});
        if group.grouped {
            node["extras"] = json!({"sourcePivots":group.pivots()});
        }
        if rig.is_some() {
            node["skin"] = json!(0);
        } else {
            node["translation"] = json!(object.origin);
        }
        nodes.push(node);
        meshes.push(json!({"name":name,"primitives":primitives}));
    }

    let mut animations = Vec::new();
    for (name, clip) in rig.into_iter().flat_map(|rig| &rig.clips) {
        let mut channels = Vec::new();
        let mut samplers = Vec::new();
        for (bone, keys) in &clip.tracks {
            let index = indices[bone.as_str()];
            let mode = clip.interpolation(bone);
            let mut times = vec![0.0];
            times.extend(keys.iter().map(|key| key.time).filter(|time| *time > 0.0 && *time < clip.duration));
            times.push(clip.duration);
            let bake_limit = mode != Interpolation::Step && bones[index].rotation_limit.is_some();
            if mode == Interpolation::Cubic || bake_limit {
                let mut baked = Vec::new();
                for interval in times.windows(2) {
                    let span = interval[1] - interval[0];
                    let steps = (span * 60.0).ceil().max(1.0) as usize;
                    if baked.len().saturating_add(steps) > 200_000 {
                        return Err("GLB track baking exceeds 200,000 samples on one track".into());
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
                let (rotation, translation) = bones[index].sample(clip, time);
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

    if meshes.is_empty() {
        return Err("no visible faces to export".into());
    }
    let mut document = json!({
        "asset":{"version":"2.0","generator":"gpt-mesh"},"scene":0,"scenes":[{"nodes":scene_nodes}],
        "nodes":nodes,"meshes":meshes,
        "accessors":buffer.accessors,"bufferViews":buffer.views,"buffers":[{"byteLength":buffer.bytes.len()}]
    });
    if let Some(skin) = skin {
        document["skins"] = json!([skin]);
    }
    if !animations.is_empty() {
        document["animations"] = json!(animations);
    }
    if !materials.is_empty() {
        if materials.iter().any(|material| material.get("extensions").is_some()) {
            document["extensionsUsed"] = json!(["KHR_materials_unlit"]);
        }
        document["textures"] = json!(image_textures);
        document["samplers"] = json!(texture_samplers);
        document["images"] = json!(images);
        document["materials"] = json!(materials);
    }
    pack(&document, &buffer.bytes)
}
