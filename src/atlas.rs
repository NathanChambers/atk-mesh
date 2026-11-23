use crate::{Mesh, normals, uv};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uvunwrap::{Density, MeshInput, Options};

pub(super) fn edit(mesh: &mut Mesh, request: &Value, repack: bool) -> Result<Value, String> {
    let faces = uv::faces(mesh, request)?;
    let resolution = request
        .get("resolution")
        .map_or(Some(1024), Value::as_u64)
        .filter(|v| (16..=4096).contains(v))
        .ok_or("resolution must be 16..4096")? as u32;
    let padding = request
        .get("padding")
        .map_or(Some(2), Value::as_u64)
        .filter(|v| *v < u64::from(resolution / 2))
        .ok_or("padding must be below half the resolution")? as u32;
    let seams: Vec<[u64; 2]> = request
        .get("seams")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|_| "seams must contain vertex ID pairs")?
        .unwrap_or_default();
    if repack && !seams.is_empty() {
        return Err("uv_pack uses existing UV islands; explicit seams apply to uv_unwrap".into());
    }
    let edge = |a: u64, b: u64| [a.min(b), a.max(b)];
    let seams: BTreeSet<_> = seams.into_iter().map(|[a, b]| edge(a, b)).collect();
    let mut found = BTreeSet::new();
    let mut positions = Vec::new();
    let mut old_uvs = Vec::new();
    let mut indices = Vec::new();
    let mut points = BTreeMap::new();
    let mut corners = Vec::new();
    let mut polygons = Vec::new();
    let mut materials = Vec::new();
    let mut splits = Vec::new();
    let mut material_ids = BTreeMap::new();
    let owners: BTreeMap<_, _> = mesh
        .objects
        .iter()
        .flat_map(|(id, obj)| obj.vertices.iter().map(move |v| (*v, *id)))
        .collect();
    for (polygon, &face_id) in faces.iter().enumerate() {
        let face = &mesh.faces[&face_id];
        if repack && !uv::complete(mesh, face_id) {
            return Err(format!("face {face_id} needs complete UVs before packing"));
        }
        let key = (owners[&face[0]], mesh.face_textures.get(&face_id));
        let next_id = material_ids.len() as u32;
        let material = *material_ids.entry(key).or_insert(next_id);
        for i in 0..face.len() {
            let pair = edge(face[i], face[(i + 1) % face.len()]);
            if seams.contains(&pair) {
                found.insert(pair);
            }
        }
        for tri in normals::triangles(mesh, face) {
            polygons.push(polygon as u32);
            materials.push(material);
            let ids = tri.map(|i| face[i]);
            splits.push(std::array::from_fn(|i| {
                seams.contains(&edge(ids[i], ids[(i + 1) % 3]))
            }));
            for id in ids {
                let old = if repack {
                    mesh.uvs[&face_id][&id].map(|v| v as f32)
                } else {
                    [0.0; 2]
                };
                let key = (id, old.map(f32::to_bits));
                let index = points.entry(key).or_insert_with(|| {
                    let index = positions.len() as u32;
                    positions.push(mesh.vertices[&id].map(|v| v as f32));
                    old_uvs.push(old);
                    index
                });
                indices.push(*index);
                corners.push((face_id, id));
            }
        }
    }
    if found != seams {
        return Err("each seam must be a polygon edge inside the selected faces".into());
    }
    let input = MeshInput {
        polygons: Some(&polygons),
        seams: Some(&splits),
        face_materials: Some(&materials),
        existing_uvs: repack.then_some(&old_uvs),
        ..MeshInput::new(&positions, &indices)
    };
    let options = Options {
        density: Density::Auto {
            page_size: resolution,
        },
        padding,
        ..Options::default()
    };
    let atlas = uvunwrap::generate(&[input], &options).map_err(|e| format!("UV atlas: {e}"))?;
    if atlas.page_count != 1 {
        return Err(
            "atlas needs multiple pages; reduce padding, increase resolution or select fewer faces"
                .into(),
        );
    }
    let out = &atlas.meshes[0];
    let mut generated = BTreeMap::<u64, BTreeMap<u64, [f64; 2]>>::new();
    for ((face, id), index) in corners.into_iter().zip(&out.indices) {
        let point = out.vertices[*index as usize].uv;
        // Preserve square texels when the packer crops the atlas height.
        let uv = [
            f64::from(point[0]) * f64::from(atlas.width) / f64::from(resolution),
            f64::from(point[1]) * f64::from(atlas.height) / f64::from(resolution),
        ];
        if generated
            .entry(face)
            .or_default()
            .insert(id, uv)
            .is_some_and(|old| old != uv)
        {
            return Err(format!(
                "face {face} needs an internal UV seam; split or triangulate it first"
            ));
        }
    }
    mesh.uvs.extend(generated);
    Ok(
        json!({"faces":faces.len(),"charts":atlas.chart_count,"resolution":resolution,"padding":padding,"texels_per_unit":atlas.texels_per_unit}),
    )
}
