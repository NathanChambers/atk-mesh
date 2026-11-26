use crate::{Mesh, face_is_degenerate, objects};
use std::collections::BTreeSet;
use serde_json::{Value, json};
use std::fs::File;
use std::io::Read;

const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ELEMENTS: usize = 1_000_000;
const MAX_LOADER_BYTES: usize = 256 * 1024 * 1024;

fn coordinates(point: ufbx::Vec3) -> [f64; 3] {
    [point.x, point.y, point.z]
}

pub(super) fn import(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let obj = request["command"] == "import_obj";
    let format = if obj { "OBJ" } else { "FBX" };
    let element_limit = if obj { 2 * MAX_ELEMENTS } else { MAX_ELEMENTS };
    let loader_limit = if obj { 2 * MAX_LOADER_BYTES } else { MAX_LOADER_BYTES };
    let path = request["path"].as_str().ok_or("path is required")?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(format!("{format} exceeds 512 MiB import limit"));
    }

    let scene = ufbx::load_memory(
        &bytes,
        ufbx::LoadOpts {
            file_format: if obj { ufbx::FileFormat::Obj } else { ufbx::FileFormat::Fbx },
            obj_unit_meters: 1.0,
            obj_axes: ufbx::CoordinateAxes::right_handed_y_up(),
            ignore_animation: true,
            ignore_embedded: true,
            load_external_files: false,
            index_error_handling: ufbx::IndexErrorHandling::AbortLoading,
            skip_skin_vertices: true,
            node_depth_limit: 256,
            target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
            target_unit_meters: 1.0,
            result_allocator: ufbx::AllocatorOpts {
                memory_limit: loader_limit,
                ..Default::default()
            },
            temp_allocator: ufbx::AllocatorOpts {
                memory_limit: loader_limit,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .map_err(|error| format!("cannot read {format}: {}", error.description))?;

    let mut imported = Vec::new();
    let mut vertex_count = 0;
    let mut face_count = 0;
    let mut corner_count = 0;
    let mut skipped_faces = 0;
    for node in &scene.nodes {
        let Some(source) = &node.mesh else { continue };
        let face_total = source.num_faces
            - source.num_empty_faces
            - source.num_point_faces
            - source.num_line_faces;
        if face_total == 0 {
            continue;
        }

        vertex_count += source.num_vertices;
        face_count += face_total;
        corner_count += source.num_indices;
        if vertex_count > element_limit
            || face_count > element_limit
            || corner_count > 4 * element_limit
        {
            return Err(format!("{format} exceeds import limits for vertices, faces or face corners"));
        }

        let matrix = &node.geometry_to_world;
        let determinant = ufbx::matrix_determinant(matrix);
        if !determinant.is_finite() || determinant == 0.0 {
            return Err(format!("singular or non-finite {format} mesh transform"));
        }

        let base = node.element.name.as_ref();
        let base = if base.trim().is_empty() {
            "Imported Mesh"
        } else {
            base
        };
        let mut name = base.to_string();
        let mut suffix = 2;
        while mesh.objects.values().any(|object| object.name == name) {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        let pivot = ufbx::transform_position(&node.node_to_world, ufbx::Vec3::default());
        let id =
            objects::destination_object(mesh, &json!({"name":name,"origin":coordinates(pivot)}))?;

        let mut vertices = Vec::with_capacity(source.num_vertices);
        for point in &source.vertices {
            let position = ufbx::transform_position(matrix, *point);
            vertices.push(mesh.add_vertex(coordinates(position))?);
        }

        for polygon in &source.faces {
            if polygon.num_indices < 3 {
                continue;
            }

            let start = polygon.index_begin as usize;
            let end = start + polygon.num_indices as usize;
            let mut face = Vec::with_capacity(end - start);
            for corner in start..end {
                let index = source.vertex_indices[corner] as usize;
                face.push(vertices[index]);
            }
            if determinant < 0.0 {
                face[1..].reverse();
            }
            let unique: BTreeSet<_> = face.iter().collect();
            if unique.len() != face.len() || face_is_degenerate(mesh, &face) {
                skipped_faces += 1;
                continue;
            }
            mesh.add_face(face)?;
        }

        mesh.objects.get_mut(&id).unwrap().vertices.extend(vertices);
        imported.push(id);
    }
    if face_count == skipped_faces {
        return Err(format!("{format} scene contains no polygon meshes"));
    }

    Ok(json!({
        "objects": imported,
        "vertices": vertex_count,
        "faces": face_count - skipped_faces,
        "skipped_invalid_faces": skipped_faces,
        "warnings": [
            "Geometry only: UVs, materials, textures, colors, source normals, rigs and animations are not retained.",
            "Mesh transforms are baked into vertices in right-handed Y-up coordinates and meters."
        ]
    }))
}
