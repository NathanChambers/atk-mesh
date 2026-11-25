use crate::{Mesh, diagnostics, geometry::*, normals};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Attributes {
    triangles: usize,
    corners: usize,
    uv_corners: usize,
    painted_corners: usize,
    world_area: f64,
    texel_area: f64,
}

pub(crate) fn constraints(
    mesh: &Mesh,
    object: Option<u64>,
    limits: &Value,
    metrics: &Value,
) -> Result<Vec<Value>, String> {
    let limits = limits
        .as_object()
        .ok_or("constraints must be a JSON object")?;
    let mut checks = Vec::new();
    for (name, limit) in limits {
        let measured = match name.as_str() {
            "thin_features" => {
                let mut check = crate::render::thin_feature_constraint(mesh, object, limit)?;
                check["metric"] = json!(name);
                check["object"] = json!(object);
                checks.push(check);
                continue;
            }
            "min_wall_thickness" | "clearance" | "protected_volume" => {
                let mut check = crate::physical_constraints::evaluate(mesh, object, name, limit)?;
                check["metric"] = json!(name);
                check["object"] = json!(object);
                checks.push(check);
                continue;
            }
            "max_triangles" | "max_corners" | "max_uv_corners" | "max_painted_corners" => {
                let field = name.trim_start_matches("max_");
                let count: u64 = metrics["objects"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(id, _)| object.is_none_or(|target| id.parse().ok() == Some(target)))
                    .filter_map(|(_, entry)| entry[field].as_u64())
                    .sum();
                json!(count)
            }
            "max_texture_size" => {
                let mut size = 0;
                for (face, name) in &mesh.face_textures {
                    if object.is_some_and(|id| {
                        !mesh.objects[&id].vertices.contains(&mesh.faces[face][0])
                    }) {
                        continue;
                    }
                    if let Some(image) = mesh.textures.get(name) {
                        size = size.max(image.width.max(image.height));
                    }
                }
                json!(size)
            }
            "closed_surfaces" => {
                let required = limit.as_bool().ok_or("closed_surfaces must be boolean")?;
                let open = metrics["shells"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|shell| {
                        object.is_none_or(|id| shell["object"] == id) && shell["closed"] == false
                    })
                    .count();
                checks.push(json!({"metric":name,"object":object,"target":limit,"open_shells":open,"passed":!required || open==0}));
                continue;
            }
            "pivot" => {
                let target = crate::vector(limit)?;
                let origins: Vec<_> = mesh
                    .objects
                    .iter()
                    .filter(|(id, _)| object.is_none_or(|o| o == **id))
                    .map(|(_, obj)| obj.origin)
                    .collect();
                let passed = origins
                    .iter()
                    .all(|origin| length(sub(*origin, target)) <= 1e-8);
                checks.push(json!({"metric":name,"object":object,"target":target,"measured":origins,"passed":passed}));
                continue;
            }
            _ => return Err(format!("unknown constraint {name}")),
        };
        let maximum = limit
            .as_u64()
            .ok_or_else(|| format!("{name} must be a nonnegative integer"))?;
        checks.push(json!({"metric":name,"object":object,"target":limit,"measured":measured,"passed":measured.as_u64().unwrap()<=maximum}));
    }
    Ok(checks)
}

impl Attributes {
    fn measure(&mut self, mesh: &Mesh, id: u64, face: &[u64]) {
        self.corners += face.len();
        let uvs = mesh.uvs.get(&id);
        self.uv_corners += uvs.map_or(0, |values| values.len());
        self.painted_corners += face
            .iter()
            .filter(|v| mesh.vertex_colors.contains_key(v))
            .count();
        let texture = mesh
            .face_textures
            .get(&id)
            .and_then(|name| mesh.textures.get(name));
        for corners in normals::triangles(mesh, face) {
            self.triangles += 1;
            let (Some(texture), Some(uvs)) = (texture, uvs) else {
                continue;
            };
            let ids = corners.map(|i| face[i]);
            let [Some(a), Some(b), Some(c)] = ids.map(|v| uvs.get(&v)) else {
                continue;
            };
            let pixel_area = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs()
                * (texture.width * texture.height) as f64;
            let [a, b, c] = ids.map(|v| mesh.vertices[&v]);
            self.world_area += length(cross(sub(b, a), sub(c, a))) * 0.5;
            self.texel_area += pixel_area * 0.5;
        }
    }

    fn report(&self) -> Value {
        let density = (self.world_area > 0.).then(|| (self.texel_area / self.world_area).sqrt());
        json!({"triangles":self.triangles,"corners":self.corners,"uv_corners":self.uv_corners,
            "painted_corners":self.painted_corners,"texels_per_meter":density})
    }
}

pub(crate) fn inspect(mesh: &Mesh, request: &Value) -> Result<Value, String> {
    let selected = diagnostics::faces(mesh, request)?;
    let mut objects = BTreeMap::new();
    let mut materials = BTreeMap::<Option<String>, Attributes>::new();
    let mut shells = vec![];
    let mut shell_of = BTreeMap::new();
    let mut folds = vec![];
    let mut confirmed = vec![];
    let mut region_violations = vec![];
    let mut total_triangles = 0;
    let mut closed_shells = BTreeSet::new();
    let owners: BTreeMap<_, _> = mesh.objects.iter().flat_map(|(&id, object)| {
        object.vertices.iter().map(move |&vertex| (vertex,id))
    }).collect();
    let mut object_faces = BTreeMap::<u64, Vec<u64>>::new();
    for &face in &selected {
        if let Some(&id) = owners.get(&mesh.faces[&face][0]) {
            object_faces.entry(id).or_default().push(face);
        }
    }
    for (object_id, object) in &mesh.objects {
        let mut totals = Attributes::default();
        let mut edges = BTreeMap::<(u64, u64), Vec<(u64, bool)>>::new();
        let mut neighbors = BTreeMap::<u64, BTreeSet<u64>>::new();
        for &id in object_faces.get(object_id).into_iter().flatten() {
            let face = &mesh.faces[&id];
            neighbors.entry(id).or_default();
            totals.measure(mesh, id, face);
            materials
                .entry(mesh.face_textures.get(&id).cloned())
                .or_default()
                .measure(mesh, id, face);
            for (&a, &b) in face.iter().zip(face.iter().cycle().skip(1)) {
                edges
                    .entry((a.min(b), a.max(b)))
                    .or_default()
                    .push((id, a < b));
            }
        }
        if neighbors.is_empty() {
            continue;
        }
        for adjacent in edges.values() {
            for &(face, _) in adjacent.iter().skip(1) {
                neighbors.get_mut(&adjacent[0].0).unwrap().insert(face);
                neighbors.get_mut(&face).unwrap().insert(adjacent[0].0);
            }
            if let [(a, _), (b, _)] = adjacent.as_slice() {
                let cosine = dot(
                    normals::face(mesh, &mesh.faces[a]),
                    normals::face(mesh, &mesh.faces[b]),
                );
                if cosine < 170_f64.to_radians().cos() {
                    folds.push(json!({"object":object_id,"faces":[a,b],"angle":cosine.clamp(-1.,1.).acos().to_degrees()}));
                }
            }
        }
        let mut oriented_shells = Vec::new();
        for faces in diagnostics::connected_components(&neighbors) {
            let shell = shells.len();
            let mut boundary = 0;
            let mut nonmanifold = 0;
            let mut inconsistent = 0;
            for (face, edge) in faces.iter().flat_map(|id| {
                let polygon = &mesh.faces[id];
                polygon.iter().zip(polygon.iter().cycle().skip(1))
                    .map(move |(&a,&b)| (*id, (a.min(b),a.max(b))))
            }) {
                let adjacent = &edges[&edge];
                if adjacent[0].0 != face { continue }
                match adjacent.as_slice() {
                    [_] => boundary += 1,
                    [a, b] => inconsistent += usize::from(a.1 == b.1),
                    _ => nonmanifold += 1,
                }
            }
            let closed = boundary == 0 && nonmanifold == 0;
            if closed {
                closed_shells.insert(shell);
                if inconsistent > 0 {
                    confirmed.push(
                        json!({"shell":shell,"kind":"inconsistent_winding","edges":inconsistent}),
                    );
                } else {
                    let first = mesh.vertices[&mesh.faces[faces.first().unwrap()][0]];
                    let mut volume = 0.;
                    for id in &faces {
                        let face = &mesh.faces[id];
                        for corners in normals::triangles(mesh, face) {
                            let [a, b, c] = corners.map(|i| sub(mesh.vertices[&face[i]], first));
                            volume += dot(a, cross(b, c));
                        }
                    }
                    oriented_shells.push((shell, first, volume / 6., faces.clone()));
                }
            }
            for &face in &faces {
                shell_of.insert(face, shell);
            }
            shells.push(json!({"id":shell,"object":object_id,"faces":faces.len(),"closed":closed,"boundary_edges":boundary,"nonmanifold_edges":nonmanifold}));
        }
        for (shell, point, volume, _) in &oriented_shells {
            if *volume >= 0. { continue }
            // An inward shell nested inside an outer boundary can bound a cavity.
            let mut enclosing = 0;
            for (other, _, _, faces) in &oriented_shells {
                if other == shell { continue }
                let mut angle = 0.;
                for face in faces {
                    let vertices = &mesh.faces[face];
                    for corners in normals::triangles(mesh,vertices) {
                        let [a,b,c] = corners.map(|i| sub(mesh.vertices[&vertices[i]],*point));
                        let [la,lb,lc] = [a,b,c].map(length);
                        angle += 2. * dot(a,cross(b,c)).atan2(la*lb*lc + dot(a,b)*lc + dot(b,c)*la + dot(c,a)*lb);
                    }
                }
                if angle.abs() > std::f64::consts::TAU { enclosing += 1; }
            }
            if enclosing % 2 == 0 {
                confirmed.push(json!({"shell":shell,"kind":"inverted_shell","signed_volume":volume}));
            }
        }
        total_triangles += totals.triangles;
        let mut object_report = totals.report();
        object_report["vertices"] = json!(object.vertices.len());
        objects.insert(*object_id, object_report);
    }
    let mut regions = mesh.projection_regions.clone();
    for component in mesh.components.values() {
        for &face in component.faces.intersection(&selected) {
            let Some(rect) = crate::construction::generator_uv::expected_region(component, face)
            else {
                continue;
            };
            regions.entry(face).or_insert(rect);
        }
    }
    for (face, rect) in regions {
        if selected.contains(&face) {
            if let Some(corners) = mesh.uvs.get(&face) {
                for (vertex, &[u, v]) in corners {
                    if u < rect[0] - 1e-9
                        || v < rect[1] - 1e-9
                        || u > rect[2] + 1e-9
                        || v > rect[3] + 1e-9
                    {
                        region_violations
                            .push(json!({"face":face,"vertex":vertex,"region":rect,"uv":[u,v]}));
                    }
                }
            }
        }
    }
    let report = match diagnostics::full_report(mesh, request, 0) {
        Ok(report) => report,
        Err(error) if error.starts_with("cannot triangulate face ") => {
            json!({"complete":false,"items":[],"error":error})
        }
        Err(error) => return Err(error),
    };
    for finding in report["items"].as_array().into_iter().flatten() {
        let a = finding["faces"][0].as_u64().unwrap();
        let b = finding["faces"][1].as_u64().unwrap();
        if let Some(shell) = shell_of.get(&a) {
            if shell_of.get(&b) == Some(shell)
                && closed_shells.contains(shell)
                && finding["intentional"] == false
            {
                confirmed.push(finding.clone());
            }
        }
    }
    let materials: Vec<_> = materials
        .into_iter()
        .map(|(name, attributes)| json!({"material":name,"attributes":attributes.report()}))
        .collect();
    Ok(
        json!({"triangles":total_triangles,"objects":objects,"materials":materials,"shells":shells,
        "fold_candidates":folds,"confirmed_defects":confirmed,"overlap_scan_complete":report["complete"],
        "tested_triangle_pairs":report["tested_triangle_pairs"],"intersection_findings":report["items"],
        "overlap_scan_error":report["error"],
        "region_violations":region_violations}),
    )
}
