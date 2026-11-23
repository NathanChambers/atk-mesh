use crate::{Mesh, geometry::*, normals};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
pub struct Allowance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<[u64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regions: Option<[u64; 2]>,
    pub reason: String,
}

#[derive(Clone)]
pub(super) struct Triangle {
    pub face: u64,
    pub object: u64,
    pub points: [[f64; 3]; 3],
    pub min: [f64; 3],
    pub max: [f64; 3],
}

pub(super) fn triangles(mesh: &Mesh, faces: &BTreeSet<u64>) -> Result<Vec<Triangle>, String> {
    let owners: BTreeMap<_, _> = mesh
        .objects
        .iter()
        .flat_map(|(id, o)| o.vertices.iter().map(move |v| (*v, *id)))
        .collect();
    let mut triangles = vec![];
    for id in faces {
        let face = mesh.faces.get(id).ok_or("unknown face")?;
        let normal = normals::face(mesh, face);
        let axis = (0..3)
            .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
            .unwrap();
        let points: Vec<_> = face
            .iter()
            .map(|v| {
                [
                    mesh.vertices[v][(axis + 1) % 3],
                    mesh.vertices[v][(axis + 2) % 3],
                ]
            })
            .collect();
        let indices = crate::shaping::polygon_triangles(&points)
            .map_err(|e| format!("cannot triangulate face {id}: {e}"))?;
        for indices in indices {
            let points = indices.map(|i| mesh.vertices[&face[i]]);
            let min =
                std::array::from_fn(|a| points.iter().map(|p| p[a]).fold(f64::INFINITY, f64::min));
            let max = std::array::from_fn(|a| {
                points
                    .iter()
                    .map(|p| p[a])
                    .fold(f64::NEG_INFINITY, f64::max)
            });
            triangles.push(Triangle {
                face: *id,
                object: owners[&face[0]],
                points,
                min,
                max,
            });
        }
    }
    Ok(triangles)
}

pub(super) fn faces(mesh: &Mesh, request: &Value) -> Result<BTreeSet<u64>, String> {
    if request.get("object").is_some() && request.get("objects").is_some() {
        return Err("use object or objects, not both".into());
    }
    if request.get("faces").is_some() && request.get("region").is_some() {
        return Err("use faces or region, not both".into());
    }
    let explicit: Option<BTreeSet<u64>> = request
        .get("faces")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "faces must be IDs"))
        .transpose()?;
    let objects: Option<BTreeSet<u64>> = if let Some(object) = request.get("object") {
        Some(BTreeSet::from([object
            .as_u64()
            .ok_or("object must be an ID")?]))
    } else {
        request
            .get("objects")
            .map(|v| serde_json::from_value(v.clone()).map_err(|_| "objects must be IDs"))
            .transpose()?
    };
    if objects
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|id| !mesh.objects.contains_key(id)))
    {
        return Err("unknown object".into());
    }
    let region = request
        .get("region")
        .map(|v| v.as_str().ok_or("region must be a name"))
        .transpose()?;
    let region_faces = region
        .map(|name| {
            crate::construction::named(mesh, name).map(|id| mesh.components[&id].faces.clone())
        })
        .transpose()?;
    let selected = explicit
        .or(region_faces)
        .unwrap_or_else(|| mesh.faces.keys().copied().collect());
    if selected.iter().any(|id| !mesh.faces.contains_key(id)) {
        return Err("unknown face".into());
    }
    Ok(selected
        .into_iter()
        .filter(|id| {
            objects.as_ref().is_none_or(|owners| {
                owners
                    .iter()
                    .any(|o| mesh.objects[o].vertices.contains(&mesh.faces[id][0]))
            })
        })
        .collect())
}

pub(super) fn components(mesh: &Mesh, object: u64) -> Vec<BTreeSet<u64>> {
    let mut adjacency: BTreeMap<_, BTreeSet<u64>> = mesh.objects[&object]
        .vertices
        .iter()
        .map(|id| (*id, BTreeSet::new()))
        .collect();
    for face in mesh
        .faces
        .values()
        .filter(|f| mesh.objects[&object].vertices.contains(&f[0]))
    {
        for i in 0..face.len() {
            adjacency
                .get_mut(&face[i])
                .unwrap()
                .insert(face[(i + 1) % face.len()]);
            adjacency
                .get_mut(&face[(i + 1) % face.len()])
                .unwrap()
                .insert(face[i]);
        }
    }
    connected_components(&adjacency)
}

pub(super) fn connected_components(adjacency: &BTreeMap<u64, BTreeSet<u64>>) -> Vec<BTreeSet<u64>> {
    let mut remaining: BTreeSet<_> = adjacency.keys().copied().collect();
    let mut groups = vec![];
    while let Some(seed) = remaining.first().copied() {
        let mut pending = vec![seed];
        let mut group = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if remaining.remove(&id) {
                group.insert(id);
                pending.extend(&adjacency[&id]);
            }
        }
        groups.push(group);
    }
    groups
}

pub(super) fn topology(mesh: &Mesh) -> Value {
    let edges = crate::surface_ops::edges(mesh, None);
    let (loops, branching) = crate::surface_ops::boundaries(mesh, None);
    let edge_findings:Vec<_>=edges.iter().filter(|(_,owners)|owners.len()>2 || (owners.len()==2 && owners[0].1==owners[1].1)).map(|((a,b),owners)|{
        let objects:BTreeSet<_>=mesh.objects.iter().filter(|(_,object)|object.vertices.contains(a)).map(|(id,_)|*id).collect();
        json!({"kind":if owners.len()>2{"non_manifold_edge"}else{"inconsistent_winding"},"vertices":[a,b],"faces":owners.iter().map(|item|item.0).collect::<Vec<_>>(),"objects":objects,"location":lerp(mesh.vertices[a],mesh.vertices[b],0.5),"limitation":"edge incidence; vertex fan validation is separate"})
    }).collect();
    let groups:Vec<_>=mesh.objects.keys().flat_map(|id|components(mesh,*id).into_iter().map(move |vertices|{let faces:Vec<_>=mesh.faces.iter().filter(|(_,face)|vertices.contains(&face[0])).map(|(id,_)|*id).collect();json!({"object":id,"seed_vertex":vertices.first(),"vertex_count":vertices.len(),"face_count":faces.len(),"faces":faces,"bounds":crate::construction::bounds(mesh,&vertices)})})).collect();
    json!({"non_manifold_edges":edges.values().filter(|v|v.len()>2).count(),"inconsistent_winding_edges":edges.values().filter(|v|v.len()==2 && v[0].1==v[1].1).count(),"edge_findings":edge_findings,"vertex_fans":"not_checked","boundary_edges":edges.values().filter(|v|v.len()==1).count(),"boundary_loops":loops.len(),"branching_boundary_vertices":branching,"disconnected_components":groups.len(),"components":groups,"self_intersections":"not_checked"})
}

fn coplanar_overlap(
    a: [[f64; 3]; 3],
    b: [[f64; 3]; 3],
    normal: [f64; 3],
    epsilon: f64,
) -> Option<[f64; 3]> {
    let axis = (0..3)
        .max_by(|a, b| normal[*a].abs().total_cmp(&normal[*b].abs()))
        .unwrap();
    let project = |p: [f64; 3]| [p[(axis + 1) % 3], p[(axis + 2) % 3]];
    let clip = b.map(project);
    let mut polygon: Vec<_> = a.iter().copied().map(project).collect();
    let cross2 = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let sign = if cross2(clip[0], clip[1], clip[2]) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    for i in 0..3 {
        let p = clip[i];
        let q = clip[(i + 1) % 3];
        let input = polygon;
        polygon = vec![];
        if input.is_empty() {
            break;
        }
        let mut previous = *input.last().unwrap();
        let mut d0 = cross2(p, q, previous) * sign;
        for current in input {
            let d1 = cross2(p, q, current) * sign;
            if (d0 >= 0.0) != (d1 >= 0.0) {
                let t = d0 / (d0 - d1);
                polygon.push([
                    previous[0] + t * (current[0] - previous[0]),
                    previous[1] + t * (current[1] - previous[1]),
                ]);
            }
            if d1 >= 0.0 {
                polygon.push(current);
            }
            previous = current;
            d0 = d1;
        }
    }
    if polygon.len() < 3 {
        return None;
    }
    let area: f64 = (0..polygon.len())
        .map(|i| {
            polygon[i][0] * polygon[(i + 1) % polygon.len()][1]
                - polygon[(i + 1) % polygon.len()][0] * polygon[i][1]
        })
        .sum();
    if area.abs() <= epsilon * epsilon {
        return None;
    }
    let p: [f64; 2] = std::array::from_fn(|axis| {
        polygon.iter().map(|p| p[axis]).sum::<f64>() / polygon.len() as f64
    });
    let mut result = a[0];
    result[(axis + 1) % 3] = p[0];
    result[(axis + 2) % 3] = p[1];
    result[axis] = a[0][axis]
        - ((result[(axis + 1) % 3] - a[0][(axis + 1) % 3]) * normal[(axis + 1) % 3]
            + (result[(axis + 2) % 3] - a[0][(axis + 2) % 3]) * normal[(axis + 2) % 3])
            / normal[axis];
    Some(result)
}

fn contact(a: &Triangle, b: &Triangle, epsilon: f64) -> Option<(&'static str, [f64; 3])> {
    if (0..3).any(|i| a.min[i] > b.max[i] + epsilon || b.min[i] > a.max[i] + epsilon) {
        return None;
    }
    let na = crate::normals::unit(cross(
        sub(a.points[1], a.points[0]),
        sub(a.points[2], a.points[0]),
    ));
    let nb = crate::normals::unit(cross(
        sub(b.points[1], b.points[0]),
        sub(b.points[2], b.points[0]),
    ));
    if length(cross(na, nb)) < 1e-9
        && b.points
            .iter()
            .all(|p| dot(sub(*p, a.points[0]), na).abs() <= epsilon)
    {
        return coplanar_overlap(a.points, b.points, na, epsilon)
            .map(|point| ("coplanar_overlap", point));
    }
    for (first, second) in [(a, b), (b, a)] {
        for i in 0..3 {
            if let Some(point) = segment_triangle(
                first.points[i],
                first.points[(i + 1) % 3],
                second.points,
                1e-10,
            ) {
                return Some(("surface_intersection", point));
            }
        }
    }
    None
}

pub(super) fn inspect(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    report(mesh, request, revision, false)
}

pub(super) fn full_report(mesh: &Mesh, request: &Value, revision: u64) -> Result<Value, String> {
    report(mesh, request, revision, true)
}

fn report(mesh: &Mesh, request: &Value, revision: u64, all: bool) -> Result<Value, String> {
    let selected = faces(mesh, request)?;
    let mut triangles = triangles(mesh, &selected)?;
    let epsilon = request.get("tolerance").map_or(Ok(1e-8), |v| {
        v.as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or("tolerance must be finite and positive")
    })?;
    let budget = request.get("max_pairs").map_or(Ok(250000), |v| {
        v.as_u64()
            .filter(|v| *v > 0 && *v <= 2000000)
            .ok_or("max_pairs must be 1..2000000")
    })?;
    triangles.sort_by(|a, b| a.min[0].total_cmp(&b.min[0]));
    let mut findings = BTreeMap::new();
    let mut tested = 0;
    let mut complete = true;
    'outer: for i in 0..triangles.len() {
        let a = &triangles[i];
        for b in &triangles[i + 1..] {
            if b.min[0] > a.max[0] + epsilon {
                break;
            }
            if a.face == b.face
                || (1..3).any(|axis| {
                    a.min[axis] > b.max[axis] + epsilon || b.min[axis] > a.max[axis] + epsilon
                })
            {
                continue;
            }
            if tested >= budget {
                complete = false;
                break 'outer;
            }
            tested += 1;
            let pair = [a.face.min(b.face), a.face.max(b.face)];
            if let Some((kind, mut point)) = contact(a, b, epsilon) {
                if kind == "surface_intersection" && a.object == b.object {
                    let shared: Vec<_> = mesh.faces[&a.face]
                        .iter()
                        .filter(|v| mesh.faces[&b.face].contains(v))
                        .map(|v| mesh.vertices[v])
                        .collect();
                    let expected = |point: [f64; 3]| {
                        shared.iter().any(|p| length(sub(*p, point)) <= epsilon)
                            || (shared.len() == 2
                                && length(sub(
                                    closest_segments(shared[0], shared[1], point, point).0,
                                    point,
                                )) <= epsilon)
                    };
                    let mut unexpected = None;
                    for (first, second) in [(a, b), (b, a)] {
                        for i in 0..3 {
                            if let Some(p) = segment_triangle(
                                first.points[i],
                                first.points[(i + 1) % 3],
                                second.points,
                                1e-10,
                            ) {
                                if !expected(p) {
                                    unexpected = Some(p);
                                }
                            }
                        }
                    }
                    if let Some(p) = unexpected {
                        point = p;
                    } else {
                        continue;
                    }
                }
                let duplicate = kind == "coplanar_overlap"
                    && mesh.faces[&a.face].len() == mesh.faces[&b.face].len()
                    && mesh.faces[&a.face].iter().all(|id| {
                        mesh.faces[&b.face].iter().any(|other| {
                            length(sub(mesh.vertices[id], mesh.vertices[other])) <= epsilon
                        })
                    });
                let kind = if duplicate { "duplicate_surface" } else { kind };
                let allowance = mesh.intersection_allowances.iter().find(|allowance| {
                    allowance.faces == Some(pair)
                        || allowance.regions.is_some_and(|[a, b]| {
                            let a = &mesh.components[&a].faces;
                            let b = &mesh.components[&b].faces;
                            (a.contains(&pair[0]) && b.contains(&pair[1]))
                                || (a.contains(&pair[1]) && b.contains(&pair[0]))
                        })
                });
                let objects = if a.face <= b.face {
                    [a.object, b.object]
                } else {
                    [b.object, a.object]
                };
                findings.insert((pair,kind),json!({"kind":kind,"objects":objects,"faces":pair,"location":point,"intentional":allowance.is_some(),"reason":allowance.map(|a|a.reason.as_str()),"limitation":"surface test on triangulated polygons; no solid-volume containment or material displacement"}));
                if findings.len() >= 8192 {
                    complete = false;
                    break 'outer;
                }
            }
        }
    }
    let values: Vec<_> = findings.into_values().collect();
    let mut geometry = serde_json::Map::new();
    for (name, predicate) in [
        ("duplicate_surfaces", 0),
        ("coplanar_overlaps", 1),
        ("self_intersections", 2),
        ("inter_object_intersections", 3),
    ] {
        let detected = values.iter().any(|v| match predicate {
            0 => v["kind"] == "duplicate_surface",
            1 => v["kind"] == "coplanar_overlap",
            2 => v["kind"] == "surface_intersection" && v["objects"][0] == v["objects"][1],
            _ => v["objects"][0] != v["objects"][1],
        });
        geometry.insert(
            name.into(),
            json!(if detected {
                "detected"
            } else if complete {
                "not_detected"
            } else {
                "incomplete"
            }),
        );
    }
    let mut result = if all {
        json!({"items":values,"total":values.len(),"offset":0,"next_offset":null})
    } else {
        crate::responses::slice(&json!(values), request)?
    };
    result["revision"] = json!(revision);
    result["topology"] = topology(mesh);
    if request["response"] != "full" {
        for key in ["components", "edge_findings", "branching_boundary_vertices"] {
            let bounded = crate::responses::compact(json!({"details":result["topology"][key]}));
            result["topology"][key] = bounded["details"].clone();
        }
    }
    result["geometry"] = json!(geometry);
    result["complete"] = json!(complete);
    result["tested_triangle_pairs"] = json!(tested);
    result["scope"] = json!({"faces":selected,"tolerance":epsilon,"volume_containment":"not_checked","near_coplanar_flicker":"coplanar only within tolerance"});
    Ok(result)
}

pub(super) fn clearance(mesh: &Mesh, request: &Value) -> Result<Value, String> {
    for side in ["a", "b"] {
        if request[side].as_object().is_none_or(|v| {
            !v.keys()
                .any(|k| matches!(k.as_str(), "object" | "objects" | "faces" | "region"))
        }) {
            return Err("clearance requires explicit a and b surface scopes".into());
        }
    }
    let a = triangles(mesh, &faces(mesh, &request["a"])?)?;
    let b = triangles(mesh, &faces(mesh, &request["b"])?)?;
    if a.is_empty() || b.is_empty() {
        return Err("clearance requires two nonempty surface regions".into());
    }
    if a.len().saturating_mul(b.len()) > 2000000 {
        return Err("clearance pair budget exceeded; narrow the regions".into());
    }
    let mut distance = f64::INFINITY;
    let mut closest = Value::Null;
    for a in &a {
        for b in &b {
            let mut candidates = vec![];
            if let Some((_, p)) = contact(a, b, 1e-9) {
                candidates.push((p, p));
            }
            for p in a.points {
                candidates.push((p, closest_triangle(p, b.points)));
            }
            for p in b.points {
                candidates.push((closest_triangle(p, a.points), p));
            }
            for i in 0..3 {
                for j in 0..3 {
                    candidates.push(closest_segments(
                        a.points[i],
                        a.points[(i + 1) % 3],
                        b.points[j],
                        b.points[(j + 1) % 3],
                    ));
                }
            }
            for (p, q) in candidates {
                let value = length(sub(p, q));
                if value < distance {
                    distance = value;
                    closest = json!({"objects":[a.object,b.object],"faces":[a.face,b.face],"locations":[p,q]});
                }
            }
        }
    }
    Ok(
        json!({"distance":distance,"closest":closest,"units":"world units (meters)","limitation":"unsigned triangle-surface distance; overlapping or nested solid volumes require separate interpretation"}),
    )
}

pub(super) fn volume(mesh: &Mesh, request: &Value) -> Result<Value, String> {
    let min = crate::vector(&request["min"])?;
    let max = crate::vector(&request["max"])?;
    if (0..3).any(|i| min[i] >= max[i]) {
        return Err("volume min must be below max".into());
    }
    let selected = faces(mesh, request)?;
    let triangles = triangles(mesh, &selected)?;
    let mut findings = BTreeMap::new();
    for triangle in &triangles {
        if triangle_box(triangle.points, min, max) {
            if let Some(location) = triangle_box_location(triangle.points, min, max) {
                findings.insert(triangle.face,json!({"object":triangle.object,"face":triangle.face,"location":location,"kind":"protected_volume_surface_intrusion"}));
            }
        }
    }
    let center = lerp(min, max, 0.5);
    let mut containment = vec![];
    let mut open_objects = vec![];
    let objects: BTreeSet<_> = triangles.iter().map(|t| t.object).collect();
    for object in objects {
        let complete_object = mesh
            .faces
            .iter()
            .filter(|(_, face)| mesh.objects[&object].vertices.contains(&face[0]))
            .all(|(id, _)| selected.contains(id));
        let closed = crate::surface_ops::edges(mesh, Some(object))
            .values()
            .all(|owners| owners.len() == 2);
        if !closed || !complete_object {
            open_objects.push(object);
            continue;
        }
        if findings.values().any(|v| v["object"] == object) {
            continue;
        }
        let scale = triangles
            .iter()
            .filter(|t| t.object == object)
            .flat_map(|t| t.points)
            .map(|p| length(sub(p, center)))
            .fold(1.0, f64::max)
            * 4.0;
        let far = add(
            center,
            mul(normals::unit([1.0, 0.371390676, 0.529174]), scale),
        );
        let mut crossings: Vec<_> = triangles
            .iter()
            .filter(|t| t.object == object)
            .filter_map(|t| segment_triangle(center, far, t.points, 1e-10))
            .map(|p| length(sub(p, center)))
            .collect();
        crossings.sort_by(f64::total_cmp);
        crossings.dedup_by(|a, b| (*a - *b).abs() <= scale * 1e-9);
        if crossings.len() % 2 == 1 {
            containment.push(json!({"object":object,"location":center,"kind":"protected_volume_inside_closed_object","faces":triangles.iter().filter(|t|t.object==object).map(|t|t.face).collect::<BTreeSet<_>>()}));
        }
    }
    Ok(
        json!({"detected":!findings.is_empty() || !containment.is_empty(),"findings":findings.into_values().collect::<Vec<_>>(),"containment":containment,"min":min,"max":max,"containment_not_checked_objects":open_objects,"limitation":"triangle/axis-aligned-box clipping includes touching; closed whole objects additionally use center-point parity containment. Open or partial regions have no solid-inside interpretation."}),
    )
}
