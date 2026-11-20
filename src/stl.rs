//! Binary STL with Z-up coordinates in millimeters.
use crate::{Mesh, geometry, normals};

pub(super) fn export(mesh: &Mesh) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0; 84];
    let mut triangle_count = 0_u32;
    for (id, face) in &mesh.faces {
        for triangle in normals::triangles(mesh, face) {
            let vertices = triangle.map(|corner| {
                let [x, y, z] = mesh.vertices[&face[corner]];
                [x, -z, y].map(|value| (value * 1000.0) as f32)
            });
            if vertices.iter().flatten().any(|value| !value.is_finite()) {
                return Err(format!("face {id} has coordinates outside STL's range"));
            }

            // Calculate the facet normal from the positions the file actually stores.
            let [a, b, c] = vertices.map(|vertex| vertex.map(f64::from));
            let ab = geometry::sub(b, a);
            let ac = geometry::sub(c, a);
            let normal = normals::unit(geometry::cross(ab, ac));
            if normal == [0.0; 3] {
                return Err(format!("face {id} produces a degenerate STL triangle"));
            }

            triangle_count = triangle_count
                .checked_add(1)
                .ok_or("too many STL triangles")?;
            for vector in [
                normal.map(|value| value as f32),
                vertices[0],
                vertices[1],
                vertices[2],
            ] {
                for value in vector {
                    bytes.extend(value.to_le_bytes());
                }
            }
            bytes.extend(0_u16.to_le_bytes());
        }
    }
    bytes[80..84].copy_from_slice(&triangle_count.to_le_bytes());
    Ok(bytes)
}
