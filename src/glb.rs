use crate::{Mesh, objects};
use serde_json::{Value, json};
use std::collections::BTreeSet;

type Matrix = [[f64; 4]; 4];
const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn integer(value: &Value) -> Result<usize, String> {
    let number = value.as_u64().ok_or("expected an unsigned GLB integer")?;
    usize::try_from(number).map_err(|_| "GLB integer is too large".into())
}

fn offset(value: &Value) -> Result<usize, String> {
    if value.is_null() {
        Ok(0)
    } else {
        integer(value)
    }
}

fn word(bytes: &[u8], start: usize) -> Result<u32, String> {
    let end = start.checked_add(4).ok_or("GLB offset overflow")?;
    let source = bytes.get(start..end).ok_or("truncated GLB")?;
    Ok(u32::from_le_bytes(source.try_into().unwrap()))
}

fn decode(bytes: &[u8]) -> Result<(Value, &[u8]), String> {
    if bytes.get(..4) != Some(b"glTF") || word(bytes, 4)? != 2 {
        return Err("expected a GLB version 2 file".into());
    }
    if word(bytes, 8)? as usize != bytes.len() {
        return Err("GLB length does not match header".into());
    }
    let mut cursor = 12;
    let mut document = None;
    let mut binary = None;
    while cursor < bytes.len() {
        let length = word(bytes, cursor)? as usize;
        let kind = word(bytes, cursor + 4)?;
        if length % 4 != 0 {
            return Err("GLB chunks must be aligned to four bytes".into());
        }
        let start = cursor.checked_add(8).ok_or("GLB offset overflow")?;
        let end = start.checked_add(length).ok_or("GLB offset overflow")?;
        let chunk = bytes.get(start..end).ok_or("truncated GLB chunk")?;
        match kind {
            0x4e4f534a if cursor == 12 => {
                document = Some(serde_json::from_slice::<Value>(chunk).map_err(|e| e.to_string())?);
            }
            0x004e4942 if document.is_some() && binary.is_none() => binary = Some(chunk),
            0x4e4f534a | 0x004e4942 => return Err("invalid GLB chunk order".into()),
            _ if document.is_some() => (),
            _ => return Err("GLB must start with JSON".into()),
        }
        cursor = end;
    }
    let document = document.ok_or("missing GLB JSON")?;
    let binary = binary.ok_or("missing GLB binary buffer")?;
    if document["asset"]["version"] != "2.0" {
        return Err("unsupported glTF asset version".into());
    }
    if document["extensionsRequired"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        return Err("required glTF extensions are not supported".into());
    }
    let buffers = document["buffers"]
        .as_array()
        .ok_or("missing GLB buffers")?;
    if buffers.len() != 1 || buffers[0].get("uri").is_some() {
        return Err("only a single embedded GLB buffer is supported".into());
    }
    let length = integer(&buffers[0]["byteLength"])?;
    let binary = binary.get(..length).ok_or("truncated GLB buffer")?;
    Ok((document, binary))
}

fn accessor(
    document: &Value,
    binary: &[u8],
    index: &Value,
    positions: bool,
) -> Result<Vec<Vec<f64>>, String> {
    let index = integer(index)?;
    let source = document["accessors"].get(index).ok_or("unknown accessor")?;
    if source.get("sparse").is_some() || source["normalized"].as_bool().unwrap_or(false) {
        return Err("sparse and normalized GLB accessors are not supported".into());
    }
    let component = integer(&source["componentType"])?;
    let (width, components) = if positions {
        if component != 5126 || source["type"] != "VEC3" {
            return Err("GLB positions must be float VEC3".into());
        }
        (4, 3)
    } else {
        if source["type"] != "SCALAR" || !matches!(component, 5121 | 5123 | 5125) {
            return Err("GLB indices must be unsigned SCALAR".into());
        }
        (
            match component {
                5121 => 1,
                5123 => 2,
                _ => 4,
            },
            1,
        )
    };
    let view_index = integer(&source["bufferView"])?;
    let view = document["bufferViews"]
        .get(view_index)
        .ok_or("unknown buffer view")?;
    if integer(&view["buffer"])? != 0 {
        return Err("accessor must use the embedded buffer".into());
    }
    let start = offset(&view["byteOffset"])?;
    let end = start
        .checked_add(integer(&view["byteLength"])?)
        .ok_or("GLB offset overflow")?;
    let bytes = binary
        .get(start..end)
        .ok_or("buffer view exceeds GLB buffer")?;
    let start = offset(&source["byteOffset"])?;
    let stride = view
        .get("byteStride")
        .map(integer)
        .transpose()?
        .unwrap_or(width * components);
    let count = integer(&source["count"])?;
    if count == 0 || count > 2_000_000 || stride < width * components || stride % width != 0 {
        return Err("invalid or excessive GLB accessor count/stride".into());
    }
    let last = (count - 1)
        .checked_mul(stride)
        .and_then(|v| v.checked_add(start))
        .and_then(|v| v.checked_add(width * components))
        .ok_or("GLB offset overflow")?;
    if last > bytes.len() {
        return Err("accessor exceeds its buffer view".into());
    }
    let mut values = Vec::with_capacity(count);
    for item in 0..count {
        let mut value = Vec::with_capacity(components);
        for axis in 0..components {
            let cursor = start + item * stride + axis * width;
            let number = match component {
                5121 => bytes[cursor] as f64,
                5123 => u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().unwrap()) as f64,
                5125 => word(bytes, cursor)? as f64,
                _ => f32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as f64,
            };
            if !number.is_finite() {
                return Err("non-finite GLB position".into());
            }
            value.push(number);
        }
        values.push(value);
    }
    Ok(values)
}

fn multiply(parent: Matrix, local: Matrix) -> Matrix {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| {
            (0..4)
                .map(|axis| parent[row][axis] * local[axis][column])
                .sum()
        })
    })
}

fn transform(node: &Value) -> Result<Matrix, String> {
    if let Some(value) = node.get("matrix") {
        if ["translation", "rotation", "scale"]
            .iter()
            .any(|key| node.get(key).is_some())
        {
            return Err("node cannot contain both matrix and TRS".into());
        }
        let values: [f64; 16] =
            serde_json::from_value(value.clone()).map_err(|_| "invalid GLB matrix")?;
        let matrix: Matrix =
            std::array::from_fn(|row| std::array::from_fn(|column| values[column * 4 + row]));
        if matrix[3] != [0.0, 0.0, 0.0, 1.0] {
            return Err("GLB matrix must be affine".into());
        }
        return Ok(matrix);
    }
    let translation = node
        .get("translation")
        .map(crate::vector)
        .transpose()?
        .unwrap_or([0.0; 3]);
    let scale = node
        .get("scale")
        .map(crate::vector)
        .transpose()?
        .unwrap_or([1.0; 3]);
    let rotation: [f64; 4] = node
        .get("rotation")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| "invalid GLB quaternion")?
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let [x, y, z, w] = rotation;
    let length = x * x + y * y + z * z + w * w;
    if !length.is_finite() || (length - 1.0).abs() > 1e-5 {
        return Err("GLB quaternion must be normalized".into());
    }
    let mut matrix = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            translation[0],
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            translation[1],
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            translation[2],
        ],
        [0.0, 0.0, 0.0, 1.0],
    ];
    for row in matrix.iter_mut().take(3) {
        for axis in 0..3 {
            row[axis] *= scale[axis];
        }
    }
    Ok(matrix)
}

fn point(matrix: Matrix, position: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|row| {
        matrix[row][3]
            + (0..3)
                .map(|axis| matrix[row][axis] * position[axis])
                .sum::<f64>()
    })
}

struct Importer<'a> {
    document: &'a Value,
    binary: &'a [u8],
    mesh: &'a mut Mesh,
    visited: BTreeSet<usize>,
    objects: Vec<u64>,
    vertices: usize,
    faces: usize,
}

impl Importer<'_> {
    fn node(&mut self, index: usize, parent: Matrix, depth: usize) -> Result<(), String> {
        if depth > 128 || !self.visited.insert(index) {
            return Err(
                "GLB node hierarchy contains a cycle, shared node or excessive depth".into(),
            );
        }
        let node = self.document["nodes"]
            .get(index)
            .ok_or("unknown GLB node")?;
        if node.get("skin").is_some() {
            return Err("skinned GLB nodes are not supported".into());
        }
        let local = transform(node)?;
        let world = multiply(parent, local);
        let determinant = world[0][0] * (world[1][1] * world[2][2] - world[1][2] * world[2][1])
            - world[0][1] * (world[1][0] * world[2][2] - world[1][2] * world[2][0])
            + world[0][2] * (world[1][0] * world[2][1] - world[1][1] * world[2][0]);
        if !determinant.is_finite() || determinant == 0.0 {
            return Err("singular or non-finite GLB node transform".into());
        }
        if let Some(mesh_index) = node.get("mesh") {
            let source_index = integer(mesh_index)?;
            let source = self.document["meshes"]
                .get(source_index)
                .ok_or("unknown GLB mesh")?;
            let base = node["name"]
                .as_str()
                .or(source["name"].as_str())
                .unwrap_or("Imported Mesh");
            let mut name = base.to_string();
            let mut suffix = 2;
            while self.mesh.objects.values().any(|object| object.name == name) {
                name = format!("{base} {suffix}");
                suffix += 1;
            }
            let origin = point(world, [0.0; 3]);
            let id = objects::destination_object(self.mesh, &json!({"name":name,"origin":origin}))?;
            let primitives = source["primitives"]
                .as_array()
                .ok_or("missing GLB primitives")?;
            for primitive in primitives {
                if primitive.get("mode").is_some_and(|mode| mode != 4)
                    || primitive.get("targets").is_some()
                {
                    return Err("only static GLB TRIANGLES primitives are supported".into());
                }
                let positions = accessor(
                    self.document,
                    self.binary,
                    &primitive["attributes"]["POSITION"],
                    true,
                )?;
                let indices: Vec<usize> = match primitive.get("indices") {
                    Some(index) => accessor(self.document, self.binary, index, false)?
                        .iter()
                        .map(|value| value[0] as usize)
                        .collect(),
                    None => (0..positions.len()).collect(),
                };
                if indices.is_empty()
                    || indices.len() % 3 != 0
                    || indices.iter().any(|index| *index >= positions.len())
                {
                    return Err("invalid GLB triangle indices".into());
                }
                self.vertices += positions.len();
                self.faces += indices.len() / 3;
                if self.vertices > 1_000_000 || self.faces > 1_000_000 {
                    return Err("GLB exceeds one million vertices or faces".into());
                }
                let mut vertices = Vec::with_capacity(positions.len());
                for position in positions {
                    let position = point(world, [position[0], position[1], position[2]]);
                    vertices.push(self.mesh.add_vertex(position)?);
                }
                for triangle in indices.chunks_exact(3) {
                    let mut face = vec![
                        vertices[triangle[0]],
                        vertices[triangle[1]],
                        vertices[triangle[2]],
                    ];
                    if determinant < 0.0 {
                        face.swap(1, 2);
                    }
                    self.mesh.add_face(face)?;
                }
                self.mesh
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .vertices
                    .extend(vertices);
            }
            self.objects.push(id);
        }
        if let Some(children) = node.get("children") {
            for child in children.as_array().ok_or("invalid GLB children")? {
                self.node(integer(child)?, world, depth + 1)?;
            }
        }
        Ok(())
    }
}

pub(super) fn import(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let path = request["path"].as_str().ok_or("path is required")?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    use std::io::Read;
    let mut bytes = vec![];
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("GLB exceeds 64 MiB import limit".into());
    }
    let (document, binary) = decode(&bytes)?;
    if document["animations"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        return Err("animated GLB files are not supported".into());
    }
    let scene = document.get("scene").map(integer).transpose()?.unwrap_or(0);
    let roots = document["scenes"]
        .get(scene)
        .and_then(|scene| scene["nodes"].as_array())
        .ok_or("missing GLB scene nodes")?;
    let mut importer = Importer {
        document: &document,
        binary,
        mesh,
        visited: BTreeSet::new(),
        objects: vec![],
        vertices: 0,
        faces: 0,
    };
    for root in roots {
        importer.node(integer(root)?, IDENTITY, 0)?;
    }
    if importer.objects.is_empty() {
        return Err("GLB scene contains no mesh objects".into());
    }
    Ok(
        json!({"objects":importer.objects,"vertices":importer.vertices,"faces":importer.faces,"warnings":["Geometry only: UVs, materials, textures, colors and source normals are not retained. Shading is recomputed by the mesh tool.","Node transforms are baked into vertices; mesh nodes become independent editable objects. Vertices are not welded or simplified."]}),
    )
}
