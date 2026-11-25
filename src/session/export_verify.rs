use super::*;
use crate::Mesh;
use std::collections::BTreeMap;

fn geometry_summary(mesh: &Mesh, visibility: Option<&crate::render::CaptureVisibility>) -> Value {
    let visible = |id: &u64| visibility.is_none_or(|view| view.includes_vertex(id));
    let ids: std::collections::BTreeSet<_> = mesh
        .vertices
        .keys()
        .filter(|id| visible(id))
        .copied()
        .collect();
    let pivots: Vec<_> = mesh
        .objects
        .values()
        .filter(|object| object.vertices.iter().any(visible))
        .map(|object| json!({"name":object.name,"origin":object.origin}))
        .collect();
    let mut faces = 0;
    let mut triangles = 0;
    let mut mapped_corners = 0;
    let mut maps = std::collections::BTreeSet::new();
    for (id, face) in &mesh.faces {
        if face.first().is_none_or(|vertex| !visible(vertex)) {
            continue;
        }
        faces += 1;
        triangles += crate::normals::triangles(mesh, face).len();
        mapped_corners += mesh.uvs.get(id).map_or(0, |corners| corners.len());
        maps.extend(mesh.face_textures.get(id));
    }
    json!({"objects":pivots.len(),"vertices":ids.len(),"faces":faces,"triangles":triangles,
        "bounds":crate::construction::bounds(mesh,&ids),"pivots":pivots,"mapped_corners":mapped_corners,"assigned_maps":maps.len()})
}

fn optional_array<'a>(document: &'a Value, key: &str) -> Result<&'a [Value], String> {
    match document.get(key) {
        None => Ok(&[]),
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{key} must be an array")),
    }
}

fn indexed<'a>(array: &'a [Value], index: &Value, label: &str) -> Result<&'a Value, String> {
    let index = index
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("invalid {label} index"))?;
    array
        .get(index)
        .ok_or_else(|| format!("{label} index out of range"))
}

fn accessor_hash(values: &[Vec<f64>]) -> String {
    crate::workflow::hash_byte_sequence(
        values
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes()),
    )
}

fn glb_summary(bytes: &[u8]) -> Result<Value, String> {
    let (document, binary) = crate::glb::decode(bytes)?;
    let mut vertices = 0;
    let mut triangles = 0;
    let mut primitives = 0;
    let mut attributes = BTreeMap::<String, usize>::new();
    let mut primitive_hashes = Vec::new();
    let mut uv_min = [f64::INFINITY; 2];
    let mut uv_max = [f64::NEG_INFINITY; 2];
    let materials = optional_array(&document, "materials")?;
    let textures = optional_array(&document, "textures")?;
    let images = optional_array(&document, "images")?;
    let samplers = optional_array(&document, "samplers")?;
    let buffer_views = optional_array(&document, "bufferViews")?;
    for sampler in samplers {
        for (key, legal) in [
            ("magFilter", &[9728, 9729][..]),
            ("minFilter", &[9728, 9729, 9984, 9985, 9986, 9987]),
            ("wrapS", &[33071, 33648, 10497]),
            ("wrapT", &[33071, 33648, 10497]),
        ] {
            if let Some(value) = sampler.get(key)
                && !value.as_u64().is_some_and(|v| legal.contains(&v))
            {
                return Err(format!("invalid sampler {key}"));
            }
        }
    }
    for texture in textures {
        indexed(images, &texture["source"], "image")?;
        if let Some(index) = texture.get("sampler") {
            indexed(samplers, index, "sampler")?;
        }
    }
    let mut maps = vec![];
    for image in images {
        let view = indexed(buffer_views, &image["bufferView"], "embedded image")?;
        let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let length = view["byteLength"]
            .as_u64()
            .ok_or("missing image byte length")? as usize;
        let end = start.checked_add(length).ok_or("image range overflow")?;
        let data = binary
            .get(start..end)
            .ok_or("embedded image exceeds binary buffer")?;
        if image["mimeType"] == "image/png" {
            let decoder = png::Decoder::new(std::io::Cursor::new(data));
            let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
            let size = reader.output_buffer_size();
            if size > 64 * 1024 * 1024 {
                return Err("embedded PNG exceeds 64 MiB decoded image budget".into());
            }
            reader
                .next_frame(&mut vec![0; size])
                .map_err(|error| error.to_string())?;
            maps.push(json!({"name":image["name"],"mime_type":image["mimeType"],"width":reader.info().width,"height":reader.info().height,"hash":crate::workflow::hash_bytes(data)}));
        } else {
            maps.push(
                json!({"mime_type":image["mimeType"],"hash":crate::workflow::hash_bytes(data)}),
            );
        }
    }
    for material in materials {
        let pbr = &material["pbrMetallicRoughness"];
        for binding in [
            &pbr["baseColorTexture"],
            &pbr["metallicRoughnessTexture"],
            &material["normalTexture"],
            &material["occlusionTexture"],
            &material["emissiveTexture"],
        ] {
            if !binding.is_null() {
                indexed(textures, &binding["index"], "material texture")?;
            }
        }
    }
    for mesh in optional_array(&document, "meshes")? {
        for primitive in mesh["primitives"]
            .as_array()
            .ok_or("missing GLB primitives")?
        {
            let values = primitive["attributes"]
                .as_object()
                .ok_or("invalid GLB attributes")?;
            let positions = crate::glb::accessor(
                &document,
                binary,
                &primitive["attributes"]["POSITION"],
                "VEC3",
            )?;
            let count = positions.len();
            let mut hashes = json!({"POSITION":accessor_hash(&positions)});
            for (key, index) in values {
                let entry = indexed(
                    optional_array(&document, "accessors")?,
                    index,
                    "attribute accessor",
                )?;
                if entry["count"].as_u64() != Some(count as u64) {
                    return Err(format!("{key} attribute count mismatch"));
                }
                if key == "NORMAL" {
                    let normals = crate::glb::accessor(&document, binary, index, "VEC3")?;
                    hashes[key] = json!(accessor_hash(&normals));
                }
                if key == "TEXCOORD_0" {
                    let coordinates = crate::glb::accessor(&document, binary, index, "VEC2")?;
                    hashes[key] = json!(accessor_hash(&coordinates));
                    for uv in coordinates {
                        for axis in 0..2 {
                            uv_min[axis] = uv_min[axis].min(uv[axis]);
                            uv_max[axis] = uv_max[axis].max(uv[axis]);
                        }
                    }
                }
                *attributes.entry(key.clone()).or_default() += count;
            }
            if let Some(index) = primitive.get("material") {
                indexed(materials, index, "primitive material")?;
            }
            if primitive.get("mode").is_some_and(|value| value != 4) {
                return Err("verification supports triangle primitives".into());
            }
            let index_count = if let Some(index) = primitive.get("indices") {
                let indices = crate::glb::accessor(&document, binary, index, "SCALAR")?;
                hashes["indices"] = json!(accessor_hash(&indices));
                if indices.iter().any(|item| item[0] as usize >= count) {
                    return Err("triangle index out of range".into());
                }
                indices.len()
            } else {
                count
            };
            if index_count % 3 != 0 {
                return Err("incomplete GLB triangle".into());
            }
            primitives += 1;
            primitive_hashes.push(hashes);
            vertices += count;
            triangles += index_count / 3;
        }
    }
    let mut scratch = Mesh::default();
    let reimport = if !optional_array(&document, "animations")?.is_empty()
        || !optional_array(&document, "extensionsRequired")?.is_empty()
        || !optional_array(&document, "skins")?.is_empty()
    {
        json!({"status":"not_run","reason":"geometry importer supports static meshes without required extensions"})
    } else {
        crate::glb::import_document(&mut scratch, &document, binary)?;
        json!({"status":"passed","geometry":geometry_summary(&scratch,None)})
    };
    Ok(
        json!({"triangles":triangles,"vertices":vertices,"primitives":primitives,"nodes":optional_array(&document,"nodes")?.len(),
        "meshes":optional_array(&document,"meshes")?.len(),"attributes":attributes,"primitive_hashes":primitive_hashes,"uv_bounds":if uv_min[0].is_finite(){json!({"min":uv_min,"max":uv_max})}else{Value::Null},"materials":materials,"maps":maps,
        "textures":textures,"samplers":samplers,"geometry_reimport":reimport}),
    )
}

fn polygon_summary(bytes: &[u8], format: &str) -> Result<Value, String> {
    let file_format = if format == "fbx" {
        ufbx::FileFormat::Fbx
    } else {
        ufbx::FileFormat::Obj
    };
    let scene = ufbx::load_memory(
        bytes,
        ufbx::LoadOpts {
            file_format,
            load_external_files: false,
            ignore_animation: true,
            ignore_embedded: true,
            index_error_handling: ufbx::IndexErrorHandling::AbortLoading,
            target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
            target_unit_meters: 1.0,
            temp_allocator: ufbx::AllocatorOpts {
                memory_limit: 256 * 1024 * 1024,
                ..Default::default()
            },
            result_allocator: ufbx::AllocatorOpts {
                memory_limit: 256 * 1024 * 1024,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .map_err(|error| error.description.to_string())?;
    let mut triangles = 0;
    let mut vertices = 0;
    let mut uv = 0;
    let mut normals = 0;
    let mut pivots = vec![];
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for node in &scene.nodes {
        let Some(mesh) = &node.mesh else { continue };
        for vertex in &mesh.vertices {
            let point = ufbx::transform_position(&node.geometry_to_world, *vertex);
            for (axis, value) in [point.x, point.y, point.z].into_iter().enumerate() {
                if !value.is_finite() {
                    return Err("nonfinite vertex".into());
                }
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        let point = ufbx::transform_position(&node.node_to_world, ufbx::Vec3::default());
        pivots.push(json!({"name":node.element.name.as_ref(),"origin":[point.x,point.y,point.z]}));
        vertices += mesh.num_vertices;
        triangles += mesh.num_triangles;
        if mesh.vertex_uv.exists {
            uv += mesh.num_indices;
        }
        if mesh.vertex_normal.exists {
            normals += mesh.num_indices;
        }
    }
    if triangles == 0 {
        return Err("file has no polygon mesh".into());
    }
    Ok(
        json!({"triangles":triangles,"vertices":vertices,"meshes":scene.meshes.len(),"bounds":{"min":min,"max":max},
        "pivots":if format=="fbx"{json!(pivots)}else{json!({"status":"format_limitation"})},
        "attributes":{"normal_corners":normals,"uv_corners":uv},"materials":scene.materials.len(),"maps":scene.textures.len(),
        "geometry_reimport":{"status":"passed","reader":"ufbx"}}),
    )
}

fn summarize(bytes: &[u8], format: &str) -> Result<Value, String> {
    match format {
        "glb" => glb_summary(bytes),
        "fbx" | "obj" => polygon_summary(bytes, format),
        "stl" => {
            let count = bytes.get(80..84).ok_or("missing binary STL header")?;
            let triangles = u32::from_le_bytes(count.try_into().unwrap()) as usize;
            if bytes.len() != 84 + triangles * 50 {
                return Err("invalid binary STL triangle count".into());
            }
            let mut minimum = [f64::INFINITY; 3];
            let mut maximum = [f64::NEG_INFINITY; 3];
            for triangle in bytes[84..].chunks_exact(50) {
                if triangle[..48]
                    .chunks_exact(4)
                    .any(|n| !f32::from_le_bytes(n.try_into().unwrap()).is_finite())
                {
                    return Err("nonfinite binary STL coordinate".into());
                }
                for vertex in triangle[12..48].chunks_exact(12) {
                    let read = |index: usize| {
                        f32::from_le_bytes(vertex[index..index + 4].try_into().unwrap()) as f64
                    };
                    let world = [read(0) * 0.001, read(8) * 0.001, -read(4) * 0.001];
                    for axis in 0..3 {
                        minimum[axis] = minimum[axis].min(world[axis]);
                        maximum[axis] = maximum[axis].max(world[axis]);
                    }
                }
            }
            Ok(
                json!({"triangles":triangles,"vertices":triangles*3,"bounds":{"min":minimum,"max":maximum},"bounds_convention":"Y-up meters assuming Workbench Z-up millimeter export",
                    "attributes":{"facet_normals":triangles,"uvs":{"status":"format_limitation"}},"pivots":{"status":"format_limitation"},"materials":{"status":"format_limitation"},"geometry_reimport":{"status":"not_run"}}),
            )
        }
        _ => Err("format must be glb, fbx, obj or stl".into()),
    }
}

impl Session {
    fn export_payload(&mut self, options: &Value) -> Result<(Vec<u8>, Value), String> {
        let mut export = options.clone();
        let format = options["format"]
            .as_str()
            .ok_or("export options require format")?;
        export["command"] = json!("export");
        let result = self.document.execute(export)?;
        let bytes = if format == "obj" {
            result["obj"]
                .as_str()
                .ok_or("missing OBJ")?
                .as_bytes()
                .to_vec()
        } else {
            base64::decode(
                result[format!("{format}_base64")]
                    .as_str()
                    .ok_or("missing export bytes")?,
            )?
        };
        Ok((bytes, result))
    }

    fn verification_path(&self, value: &Value) -> Result<PathBuf, String> {
        let root = self
            .workflow_directory()
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let path = root
            .join(value.as_str().ok_or("workspace file path is required")?)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !path.starts_with(root) || !path.is_file() {
            return Err("file must be inside workspace".into());
        }
        Ok(path)
    }

    pub(super) fn verify_export(
        &mut self,
        request: &Value,
        preset_version: &Value,
    ) -> Result<(Value, Value), String> {
        let from_file = request.get("path").is_some();
        let mut format = request["format"].as_str().unwrap_or("glb").to_owned();
        let mut options = request.get("export").cloned().unwrap_or_else(|| json!({}));
        if !options.is_object() {
            return Err("export must be an options object".into());
        }
        let file = if from_file {
            Some(self.verification_path(&request["path"])?)
        } else {
            None
        };
        if let Some(path) = &file
            && request.get("format").is_none()
        {
            format = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("")
                .to_lowercase();
        }
        if let Some(value) = options.get("format") {
            format = value.as_str().ok_or("format must be a string")?.into();
        }
        if !matches!(format.as_str(), "glb" | "obj" | "fbx" | "stl") {
            return Err("unsupported export format".into());
        }
        options["format"] = json!(format);
        let (bytes, payload) = if let Some(path) = &file {
            use std::io::Read;
            let mut bytes = Vec::new();
            fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 64 * 1024 * 1024 {
                return Err("verification file exceeds 64 MiB".into());
            }
            (bytes, Value::Null)
        } else {
            self.export_payload(&options)?
        };
        let mut findings = vec![];
        let observed = match summarize(&bytes, &format) {
            Ok(value) => {
                findings.push(json!({"check":"structure","status":"passed"}));
                value
            }
            Err(error) => {
                findings.push(json!({"check":"structure","status":"failed","detail":error}));
                Value::Null
            }
        };
        let baseline = request.get("baseline");
        let mut source = Value::Null;
        let expected = match baseline {
            None => None,
            Some(value) if value == "source" => {
                let visibility =
                    crate::render::CaptureVisibility::parse(&options, &self.document.mesh)?;
                source = geometry_summary(&self.document.mesh, Some(&visibility));
                let (expected, _) = self.export_payload(&options)?;
                Some(summarize(&expected, &format)?)
            }
            Some(value) => {
                let path = self.verification_path(value)?;
                if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
                    return Err("baseline receipt exceeds 8 MiB".into());
                }
                let report: Value =
                    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                let receipt = report.get("export_verification").unwrap_or(&report);
                if receipt["format"] != format
                    || receipt["status"] != "passed"
                    || !receipt["observed"].is_object()
                {
                    return Err("baseline must be a passed receipt for this format".into());
                }
                Some(receipt["observed"].clone())
            }
        };
        if let Some(expected) = expected {
            for (key, value) in expected.as_object().ok_or("invalid baseline")? {
                if &observed[key] != value {
                    findings.push(json!({"check":key,"status":"failed","expected":value,"observed":observed[key]}));
                }
            }
        } else if !from_file {
            let visibility =
                crate::render::CaptureVisibility::parse(&options, &self.document.mesh)?;
            source = geometry_summary(&self.document.mesh, Some(&visibility));
        } else {
            findings.push(json!({"check":"source_comparison","status":"not_run","reason":"standalone artifact without baseline"}));
        }
        if source.is_object() && observed.is_object() {
            let status = if source["triangles"] == observed["triangles"] {
                "passed"
            } else {
                "failed"
            };
            findings.push(json!({"check":"source_triangles","status":status,"source":source["triangles"],"observed":observed["triangles"]}));
        }
        let status = if findings.iter().any(|finding| finding["status"] == "failed") {
            "failed"
        } else {
            "passed"
        };
        let revision = if !from_file || baseline.is_some_and(|v| v == "source") {
            json!(self.document.revision())
        } else {
            Value::Null
        };
        let receipt = json!({"format":format,"source_revision":revision,"preset_version":preset_version,"export_options":options,
            "artifact_hash":crate::workflow::hash_bytes(&bytes),"artifact_hash_algorithm":"fnv1a64","bytes":bytes.len(),"artifact_path":file,"source":source,"observed":observed,
            "baseline":baseline,"findings":findings,"status":status});
        Ok((receipt, payload))
    }
}
