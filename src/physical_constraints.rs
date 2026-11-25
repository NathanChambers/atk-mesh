use crate::{Mesh, diagnostics, geometry::*, normals};
use rstar::{AABB, RTree, RTreeObject};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) fn evaluate(
    mesh: &Mesh,
    object: Option<u64>,
    name: &str,
    limit: &Value,
) -> Result<Value, String> {
    if object.is_some_and(|id| !mesh.objects.contains_key(&id)) {
        return Err("unknown constraint object".into());
    }
    match name {
        "min_wall_thickness" => {
            let minimum = limit
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.0)
                .ok_or("min_wall_thickness must be a positive distance in meters")?;
            wall_thickness(mesh, object, minimum)
        }
        "clearance" => {
            let minimum = limit["minimum"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or("clearance requires a nonnegative minimum in meters")?;
            let mut request = limit.clone();
            if let Some(id) = object {
                if request.get("a").is_none() {
                    request["a"] = json!({"object":id});
                }
                let faces = diagnostics::faces(mesh, &request["a"])?;
                if faces
                    .iter()
                    .any(|face| !mesh.objects[&id].vertices.contains(&mesh.faces[face][0]))
                {
                    return Err("clearance scope a must belong to the constraint object".into());
                }
            }
            let measured = diagnostics::clearance(mesh, &request)?;
            Ok(
                json!({"passed":measured["distance"].as_f64().unwrap() >= minimum,"measured":measured}),
            )
        }
        "protected_volume" => {
            let mut request = limit
                .as_object()
                .cloned()
                .ok_or("protected_volume must contain a min and max corner")?;
            if let Some(id) = object {
                if request
                    .get("object")
                    .is_some_and(|v| v.as_u64() != Some(id))
                    || request.contains_key("objects")
                {
                    return Err("protected_volume scope must match the constraint object".into());
                }
                request.insert("object".into(), json!(id));
            }
            let measured = diagnostics::volume(mesh, &Value::Object(request))?;
            Ok(json!({"passed":measured["detected"] == false,"measured":measured}))
        }
        _ => Err(format!("unknown physical constraint: {name}")),
    }
}

impl RTreeObject for diagnostics::Triangle {
    type Envelope = AABB<[f64; 3]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(self.min, self.max)
    }
}

fn wall_thickness(mesh: &Mesh, object: Option<u64>, minimum: f64) -> Result<Value, String> {
    let mut thickness = f64::INFINITY;
    let mut closest = Value::Null;
    let mut samples = 0;
    let mut missed = 0;
    let mut open = Vec::new();
    for (id, data) in &mesh.objects {
        if object.is_some_and(|scope| scope != *id) {
            continue;
        }
        let edges = crate::surface_ops::edges(mesh, Some(*id));
        if edges.is_empty()
            || edges
                .values()
                .any(|uses| uses.len() != 2 || uses[0].1 == uses[1].1)
        {
            open.push(*id);
            continue;
        }
        let faces: BTreeSet<_> = mesh
            .faces
            .iter()
            .filter(|(_, face)| data.vertices.contains(&face[0]))
            .map(|(id, _)| *id)
            .collect();
        let triangles = diagnostics::triangles(mesh, &faces)?;
        let tree = RTree::bulk_load(triangles);
        let bounds = tree.root().envelope();
        let reach = 2.0 * length(sub(bounds.upper(), bounds.lower()));
        for triangle in tree.iter() {
            let [a, b, c] = triangle.points;
            let direction = normals::unit(cross(sub(c, a), sub(b, a)));
            let center = mul(add(add(a, b), c), 1.0 / 3.0);
            for start in [
                center,
                lerp(center, a, 0.75),
                lerp(center, b, 0.75),
                lerp(center, c, 0.75),
            ] {
                let end = add(start, mul(direction, reach));
                let envelope = AABB::from_corners(start, end);
                let nearest = tree
                    .locate_in_envelope_intersecting(envelope)
                    .filter(|other| other.face != triangle.face)
                    .filter_map(|other| {
                        let point = segment_triangle(start, end, other.points, 1e-12)?;
                        let distance = dot(sub(point, start), direction);
                        (distance > reach * f64::EPSILON * 16.0).then_some((distance, other.face))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                samples += 1;
                let Some((distance, opposite)) = nearest else {
                    missed += 1;
                    continue;
                };
                if distance < thickness {
                    thickness = distance;
                    closest = json!({"object":id,"faces":[triangle.face,opposite],"locations":[start,add(start,mul(direction,thickness))]});
                }
            }
        }
    }
    let complete = open.is_empty() && missed == 0 && samples > 0;
    let measured = json!({"minimum":thickness.is_finite().then_some(thickness),"units":"meters","closest":closest,"samples":samples,"missed":missed,"incomplete_objects":open,"complete":complete,
        "method":"Nearest inward-normal surface hit at four interior points per tessellated triangle, measured within each object.",
        "limitation":"Sampled wall thickness. A narrow feature between sample points can be missed. Open surfaces and missed opposite surfaces prevent a pass."});
    Ok(json!({"passed":complete && thickness >= minimum,"measured":measured}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn thin_closed_wall_fails_and_open_sheet_is_incomplete() {
        let mut doc = Document::empty();
        doc.execute(
            json!({"command":"object_create_box","name":"Panel","min":[0,0,0],"max":[2,3,0.05]}),
        )
        .unwrap();
        let result = evaluate(&doc.mesh, Some(1), "min_wall_thickness", &json!(0.06)).unwrap();
        assert_eq!(result["passed"], false);
        assert_eq!(result["measured"]["complete"], true);
        assert!((result["measured"]["minimum"].as_f64().unwrap() - 0.05).abs() < 1e-12);
        assert_eq!(
            evaluate(&doc.mesh, None, "min_wall_thickness", &json!(0.04)).unwrap()["passed"],
            true
        );
        doc.execute(
            json!({"command":"delete_faces","object":1,"faces":[1],"remove_orphan_vertices":false}),
        )
        .unwrap();
        assert_eq!(
            evaluate(&doc.mesh, None, "min_wall_thickness", &json!(0.04)).unwrap()["measured"]["complete"],
            false
        );
    }

    #[test]
    fn clearance_and_protected_volume_use_actual_surfaces() {
        let mut doc = Document::empty();
        for (x, name) in [(0, "Left"), (2, "Right")] {
            doc.execute(
                json!({"command":"object_create_box","name":name,"min":[x,0,0],"max":[x+1,1,1]}),
            )
            .unwrap();
        }
        let gap = json!({"minimum":1,"b":{"object":2}});
        let result = evaluate(&doc.mesh, Some(1), "clearance", &gap).unwrap();
        assert_eq!(result["passed"], true);
        assert_eq!(result["measured"]["distance"], 1.0);
        let inside = json!({"min":[0.2,0.2,0.2],"max":[0.8,0.8,0.8]});
        assert_eq!(
            evaluate(&doc.mesh, Some(1), "protected_volume", &inside).unwrap()["passed"],
            false
        );
        assert_eq!(
            evaluate(&doc.mesh, Some(2), "protected_volume", &inside).unwrap()["passed"],
            true
        );
    }
}
