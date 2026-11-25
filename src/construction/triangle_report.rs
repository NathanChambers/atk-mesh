use crate::{Mesh, diagnostics, normals, responses};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Default)]
struct Cost {
    triangles: usize,
    bounds: Option<([f64; 3], [f64; 3])>,
}

impl Cost {
    fn measure(
        mesh: &Mesh,
        faces: &BTreeSet<u64>,
        counts: &std::collections::BTreeMap<u64, usize>,
    ) -> Self {
        let mut cost = Self::default();
        for face in faces {
            cost.triangles += counts[face];
            for id in &mesh.faces[face] {
                let vertex = mesh.vertices[id];
                let (min, max) = cost.bounds.get_or_insert((vertex, vertex));
                for axis in 0..3 {
                    min[axis] = min[axis].min(vertex[axis]);
                    max[axis] = max[axis].max(vertex[axis]);
                }
            }
        }
        cost
    }

    fn report(&self, asset: usize) -> Value {
        let dimensions = self.bounds.map(|(min, max)| crate::geometry::sub(max, min));
        let percent = if asset == 0 {
            0.0
        } else {
            100.0 * self.triangles as f64 / asset as f64
        };
        json!({"triangles":self.triangles,"asset_percent":percent,"dimensions":dimensions})
    }
}

pub(super) fn query(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    let mut scope = diagnostics::faces(mesh, request)?;
    if let Some(value) = request.get("selection") {
        let selected = value.as_bool().ok_or("selection must be boolean")?;
        if selected {
            scope.retain(|id| {
                mesh.faces[id]
                    .iter()
                    .all(|vertex| mesh.selection.contains(vertex))
            });
        }
    }

    let counts = mesh
        .faces
        .iter()
        .map(|(&id, face)| (id, normals::triangles(mesh, face).len()))
        .collect();
    let scope_cost = Cost::measure(mesh, &scope, &counts);
    let asset = counts.values().sum();
    let mut unassigned = scope.clone();
    let mut rows = Vec::new();
    for (&id, component) in &mesh.components {
        let faces: BTreeSet<_> = component.faces.intersection(&scope).copied().collect();
        if faces.is_empty() {
            continue;
        }
        for face in &faces {
            unassigned.remove(face);
        }
        let cost = Cost::measure(mesh, &faces, &counts);
        let mut row = cost.report(asset);
        row["id"] = json!(id);
        row["region"] = json!(component.name);
        rows.push((cost.triangles, row));
    }
    // Stable sorting preserves component ID order for equal costs.
    rows.sort_by_key(|(triangles, _)| std::cmp::Reverse(*triangles));
    let items = Value::Array(rows.into_iter().map(|(_, row)| row).collect());
    let mut result = responses::slice(&items, request)?;
    result["revision"] = json!(revision);
    result["asset_triangles"] = json!(asset);
    result["scope"] = scope_cost.report(asset);
    result["unassigned"] = Cost::measure(mesh, &unassigned, &counts).report(asset);
    Ok(result)
}
