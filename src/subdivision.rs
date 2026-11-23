use crate::{Mesh, geometry::add, geometry::mul, objects, rig::Influence};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

type Edge = [usize; 2];

fn edge(a: usize, b: usize) -> Edge {
    [a.min(b), a.max(b)]
}

#[derive(Clone, Default)]
struct Point {
    position: [f64; 3],
    color: Option<[f64; 3]>,
    weights: BTreeMap<String, f64>,
}

impl Point {
    fn blend<'a>(terms: impl IntoIterator<Item = (&'a Point, f64)>) -> Self {
        let mut result = Point::default();
        let mut color = [0.0; 3];
        let mut has_color = false;
        for (point, weight) in terms {
            result.position = add(result.position, mul(point.position, weight));
            color = add(color, mul(point.color.unwrap_or([1.0; 3]), weight));
            has_color |= point.color.is_some();
            for (bone, influence) in &point.weights {
                *result.weights.entry(bone.clone()).or_default() += weight * influence;
            }
        }
        result.color = has_color.then_some(color.map(|v| v.clamp(0.0, 1.0)));
        result
    }
}

struct Face {
    vertices: Vec<usize>,
    uvs: Option<Vec<[f64; 2]>>,
    source: u64,
}

struct Cage {
    points: Vec<Point>,
    faces: Vec<Face>,
    sharp: BTreeSet<Edge>,
}

impl Cage {
    fn refine(self) -> Result<Self, String> {
        let mut edges = BTreeMap::<Edge, Vec<usize>>::new();
        let mut centers = Vec::new();
        let mut faces_at = vec![Vec::new(); self.points.len()];
        for (index, face) in self.faces.iter().enumerate() {
            let ring = &face.vertices;
            let weight = 1.0 / ring.len() as f64;
            centers.push(Point::blend(
                ring.iter().map(|&v| (&self.points[v], weight)),
            ));
            for (i, &v) in ring.iter().enumerate() {
                faces_at[v].push(index);
                edges
                    .entry(edge(v, ring[(i + 1) % ring.len()]))
                    .or_default()
                    .push(index);
            }
        }
        if self.sharp.iter().any(|e| !edges.contains_key(e)) {
            return Err("each crease must be an edge of the source cage".into());
        }
        let mut neighbors = vec![Vec::new(); self.points.len()];
        let mut creases = vec![Vec::new(); self.points.len()];
        let mut points = self.points.clone();
        let mut midpoints = BTreeMap::new();
        let mut sharp = BTreeSet::new();
        for (&[a, b], incident) in &edges {
            neighbors[a].push(b);
            neighbors[b].push(a);
            let index = points.len();
            midpoints.insert([a, b], index);
            let p = &self.points[a];
            let q = &self.points[b];
            if incident.len() > 2 {
                return Err("subdivision requires manifold edges".into());
            }
            if incident.len() == 1 || self.sharp.contains(&[a, b]) {
                points.push(Point::blend([(p, 0.5), (q, 0.5)]));
                creases[a].push(b);
                creases[b].push(a);
                sharp.insert(edge(a, index));
                sharp.insert(edge(b, index));
            } else {
                points.push(Point::blend([
                    (p, 0.25),
                    (q, 0.25),
                    (&centers[incident[0]], 0.25),
                    (&centers[incident[1]], 0.25),
                ]));
            }
        }
        for (i, point) in self.points.iter().enumerate() {
            let n = neighbors[i].len() as f64;
            points[i] = match creases[i].as_slice() {
                [a, b] => Point::blend([
                    (point, 0.75),
                    (&self.points[*a], 0.125),
                    (&self.points[*b], 0.125),
                ]),
                [_, _, _, ..] => point.clone(),
                _ if n >= 2.0 && neighbors[i].len() == faces_at[i].len() => {
                    // (F + 2R + (n-3)P)/n, expanded into face centers and neighboring vertices.
                    let weight = 1.0 / (n * n);
                    let faces = faces_at[i].iter().map(|&f| (&centers[f], weight));
                    let adjacent = neighbors[i].iter().map(|&v| (&self.points[v], weight));
                    Point::blend(faces.chain(adjacent).chain([(point, (n - 2.0) / n)]))
                }
                _ => return Err("invalid boundary fan on subdivision cage".into()),
            };
        }
        let center_start = points.len();
        points.extend(centers);
        let mut faces = Vec::new();
        for (index, face) in self.faces.into_iter().enumerate() {
            let ring = face.vertices;
            let count = ring.len();
            let center_uv: Option<[f64; 2]> = face.uvs.as_ref().map(|uv| {
                std::array::from_fn(|axis| uv.iter().map(|p| p[axis]).sum::<f64>() / count as f64)
            });
            for (i, &vertex) in ring.iter().enumerate() {
                let next = (i + 1) % count;
                let prev = (i + count - 1) % count;
                let vertices = vec![
                    vertex,
                    midpoints[&edge(vertex, ring[next])],
                    center_start + index,
                    midpoints[&edge(vertex, ring[prev])],
                ];
                let uvs = face.uvs.as_ref().map(|uv| {
                    let mid =
                        |j: usize| std::array::from_fn(|axis| (uv[i][axis] + uv[j][axis]) * 0.5);
                    vec![uv[i], mid(next), center_uv.unwrap(), mid(prev)]
                });
                faces.push(Face {
                    vertices,
                    uvs,
                    source: face.source,
                });
            }
        }
        Ok(Self {
            points,
            faces,
            sharp,
        })
    }
}

pub(super) fn generate(mesh: &mut Mesh, request: &Value) -> Result<Value, String> {
    let id = request["source"]
        .as_u64()
        .ok_or("source must be a cage object ID")?;
    if request["object"].as_u64() == Some(id) {
        return Err("subdivision destination must be separate from the source cage".into());
    }
    let source = mesh.objects.get(&id).ok_or("unknown cage object")?;
    let levels = request
        .get("levels")
        .map_or(Some(1), Value::as_u64)
        .filter(|n| (1..=6).contains(n))
        .ok_or("levels must be 1..6")?;
    let mut points = Vec::new();
    let mut indices = BTreeMap::new();
    let mut faces = Vec::new();
    for (&face_id, ring) in &mesh.faces {
        if !source.vertices.contains(&ring[0]) {
            continue;
        }
        let mut vertices = Vec::new();
        for v in ring {
            let local = indices.entry(*v).or_insert_with(|| {
                let index = points.len();
                let influences = mesh.rig.as_ref().and_then(|rig| rig.weights.get(v));
                let weights = influences
                    .into_iter()
                    .flatten()
                    .map(|i| (i.bone.clone(), i.weight))
                    .collect();
                points.push(Point {
                    position: mesh.vertices[v],
                    color: mesh.vertex_colors.get(v).copied(),
                    weights,
                });
                index
            });
            vertices.push(*local);
        }
        let uvs = mesh
            .uvs
            .get(&face_id)
            .and_then(|uv| ring.iter().map(|v| uv.get(v).copied()).collect());
        faces.push(Face {
            vertices,
            uvs,
            source: face_id,
        });
    }
    let first_level: usize = faces.iter().map(|f| f.vertices.len()).sum();
    let count = first_level.saturating_mul(4usize.pow(levels as u32 - 1));
    if count == 0 || count > 1_000_000 {
        return Err(format!(
            "subdivision would produce {count} quads; choose a cage and level within 1..1000000"
        ));
    }
    let creases: Vec<[u64; 2]> = request
        .get("creases")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|_| "creases must be edge ID pairs")?
        .unwrap_or_default();
    let mut sharp = BTreeSet::new();
    for [a, b] in creases {
        sharp.insert(edge(
            *indices.get(&a).ok_or("unknown crease vertex")?,
            *indices.get(&b).ok_or("unknown crease vertex")?,
        ));
    }
    let mut cage = Cage {
        points,
        faces,
        sharp,
    };
    for _ in 0..levels {
        cage = cage.refine()?;
    }
    let object = objects::destination_object(mesh, request)?;
    let mut ids = Vec::with_capacity(cage.points.len());
    for point in cage.points {
        let vertex = mesh.add_vertex(point.position)?;
        ids.push(vertex);
        if let Some(color) = point.color {
            mesh.vertex_colors.insert(vertex, color);
        }
        if let Some(rig) = &mut mesh.rig {
            let mut weights: Vec<_> = point
                .weights
                .into_iter()
                .filter(|(_, w)| *w > 0.0)
                .map(|(bone, weight)| Influence { bone, weight })
                .collect();
            if !weights.is_empty() {
                weights.sort_by(|a, b| b.weight.total_cmp(&a.weight));
                weights.truncate(4);
                let sum: f64 = weights.iter().map(|i| i.weight).sum();
                for influence in &mut weights {
                    influence.weight /= sum;
                }
                rig.weights.insert(vertex, weights);
            }
        }
    }
    mesh.objects.get_mut(&object).unwrap().vertices.extend(&ids);
    for face in cage.faces {
        let ring: Vec<_> = face.vertices.into_iter().map(|i| ids[i]).collect();
        let id = mesh.add_face(ring.clone())?;
        if let Some(uvs) = face.uvs {
            mesh.uvs.insert(id, ring.into_iter().zip(uvs).collect());
        }
        if let Some(texture) = mesh.face_textures.get(&face.source).cloned() {
            mesh.face_textures.insert(id, texture);
        }
        if mesh.flat_faces.contains(&face.source) {
            mesh.flat_faces.insert(id);
        }
    }
    if request.get("object").is_none() {
        mesh.objects.get_mut(&object).unwrap().smooth_angle =
            mesh.objects[&id].smooth_angle.or(Some(60.0));
    }
    Ok(
        json!({"object":object,"source":id,"levels":levels,"quads":count,"source_cage_preserved":true}),
    )
}
