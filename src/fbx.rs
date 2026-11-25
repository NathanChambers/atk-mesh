//! Static binary FBX 7.4: Y-up, right-handed, one unit per meter.
use crate::Mesh;
use std::collections::{BTreeMap, BTreeSet};

struct Node {
    name: &'static str,
    properties: Vec<Vec<u8>>,
    children: Vec<Node>,
}

fn node(name: &'static str, properties: Vec<Vec<u8>>, children: Vec<Node>) -> Node {
    Node {
        name,
        properties,
        children,
    }
}

fn integer(value: i32) -> Vec<u8> {
    let mut bytes = vec![b'I'];
    bytes.extend(value.to_le_bytes());
    bytes
}

fn identifier(value: i64) -> Vec<u8> {
    let mut bytes = vec![b'L'];
    bytes.extend(value.to_le_bytes());
    bytes
}

fn number(value: f64) -> Vec<u8> {
    let mut bytes = vec![b'D'];
    bytes.extend(value.to_le_bytes());
    bytes
}

fn string(value: &str) -> Vec<u8> {
    let mut bytes = vec![b'S'];
    bytes.extend((value.len() as u32).to_le_bytes());
    bytes.extend(value.as_bytes());
    bytes
}

fn raw(value: &[u8]) -> Vec<u8> {
    let mut bytes = vec![b'R'];
    bytes.extend((value.len() as u32).to_le_bytes());
    bytes.extend(value);
    bytes
}

fn array(kind: u8, count: usize, values: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut bytes = vec![kind];
    bytes.extend(
        u32::try_from(count)
            .map_err(|_| "FBX array too large")?
            .to_le_bytes(),
    );
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(
        u32::try_from(values.len())
            .map_err(|_| "FBX array too large")?
            .to_le_bytes(),
    );
    bytes.extend(values);
    Ok(bytes)
}

fn property(name: &str, kind: &str, values: Vec<Vec<u8>>) -> Node {
    let label = match kind {
        "int" => "Integer",
        "double" => "Number",
        _ => "",
    };
    let flags = if kind.starts_with("Lcl ") { "A" } else { "" };
    let mut properties = vec![string(name), string(kind), string(label), string(flags)];
    properties.extend(values);
    node("P", properties, vec![])
}

fn vector_property(name: &str, value: [f64; 3]) -> Node {
    property(name, name, value.into_iter().map(number).collect())
}

fn write_node(node: &Node, bytes: &mut Vec<u8>) -> Result<(), String> {
    let start = bytes.len();
    bytes.extend([0; 13]);
    bytes.extend(node.name.as_bytes());
    let property_start = bytes.len();
    for property in &node.properties {
        bytes.extend(property);
    }
    let property_length =
        u32::try_from(bytes.len() - property_start).map_err(|_| "FBX node too large")?;
    for child in &node.children {
        write_node(child, bytes)?;
    }
    if !node.children.is_empty() || node.properties.is_empty() {
        bytes.extend([0; 13]);
    }
    let end = u32::try_from(bytes.len()).map_err(|_| "FBX 7.4 files must be smaller than 4 GB")?;
    bytes[start..start + 4].copy_from_slice(&end.to_le_bytes());
    bytes[start + 4..start + 8].copy_from_slice(&(node.properties.len() as u32).to_le_bytes());
    bytes[start + 8..start + 12].copy_from_slice(&property_length.to_le_bytes());
    bytes[start + 12] = node.name.len() as u8;
    Ok(())
}

pub(super) fn export(
    mesh: &Mesh,
    visibility: &crate::render::CaptureVisibility,
    merge: Option<&str>,
) -> Result<Vec<u8>, String> {
    let groups = crate::export_groups::groups(mesh, visibility, merge)?;
    // Deterministic metadata matching the fixed binary file ID and footer.
    let timestamp = node(
        "CreationTimeStamp",
        vec![],
        vec![
            node("Version", vec![integer(1000)], vec![]),
            node("Year", vec![integer(1970)], vec![]),
            node("Month", vec![integer(1)], vec![]),
            node("Day", vec![integer(1)], vec![]),
            node("Hour", vec![integer(10)], vec![]),
            node("Minute", vec![integer(0)], vec![]),
            node("Second", vec![integer(0)], vec![]),
            node("Millisecond", vec![integer(0)], vec![]),
        ],
    );
    let mut metadata = vec![node("Version", vec![integer(100)], vec![])];
    for name in [
        "Title", "Subject", "Author", "Keywords", "Revision", "Comment",
    ] {
        metadata.push(node(name, vec![string("")], vec![]));
    }
    let scene_info = node(
        "SceneInfo",
        vec![string("GlobalInfo\0\u{1}SceneInfo"), string("UserData")],
        vec![
            node("Type", vec![string("UserData")], vec![]),
            node("Version", vec![integer(100)], vec![]),
            node("MetaData", vec![], metadata),
            node("Properties70", vec![], vec![]),
        ],
    );
    let header = node(
        "FBXHeaderExtension",
        vec![],
        vec![
            node("FBXHeaderVersion", vec![integer(1003)], vec![]),
            node("FBXVersion", vec![integer(7400)], vec![]),
            node("EncryptionType", vec![integer(0)], vec![]),
            timestamp,
            node("Creator", vec![string("GPT Mesh")], vec![]),
            scene_info,
        ],
    );
    let mut settings = vec![];
    for (name, value) in [
        ("UpAxis", 1),
        ("UpAxisSign", 1),
        ("FrontAxis", 2),
        ("FrontAxisSign", 1),
        ("CoordAxis", 0),
        ("CoordAxisSign", 1),
        ("OriginalUpAxis", 1),
        ("OriginalUpAxisSign", 1),
    ] {
        settings.push(property(name, "int", vec![integer(value)]));
    }
    settings.push(property("TimeMode", "enum", vec![integer(0)]));
    settings.push(property("UnitScaleFactor", "double", vec![number(100.0)]));
    settings.push(property(
        "OriginalUnitScaleFactor",
        "double",
        vec![number(100.0)],
    ));
    let globals = node(
        "GlobalSettings",
        vec![],
        vec![
            node("Version", vec![integer(1000)], vec![]),
            node("Properties70", vec![], settings),
        ],
    );
    let document = node(
        "Document",
        vec![identifier(1), string("Scene"), string("Scene")],
        vec![
            node("Properties70", vec![], vec![]),
            node("RootNode", vec![identifier(0)], vec![]),
        ],
    );
    let documents = node(
        "Documents",
        vec![],
        vec![node("Count", vec![integer(1)], vec![]), document],
    );
    let count = i32::try_from(groups.len()).map_err(|_| "too many objects")?;
    let mut bindings = BTreeMap::new();
    if merge.is_some() && !mesh.face_textures.is_empty() {
        for (face, vertices) in &mesh.faces {
            if visibility.includes_vertex(&vertices[0]) {
                let key = mesh.face_textures.get(face).map(String::as_str);
                let id = 1000 + 2 * i64::from(count) + bindings.len() as i64;
                bindings.entry(key).or_insert(id);
            }
        }
    }
    let material_count = i32::try_from(bindings.len()).map_err(|_| "too many materials")?;
    let mut definitions = node(
        "Definitions",
        vec![],
        vec![
            node("Version", vec![integer(100)], vec![]),
            node(
                "Count",
                vec![integer(
                    count
                        .checked_mul(2)
                        .and_then(|n| n.checked_add(material_count))
                        .ok_or("too many objects")?,
                )],
                vec![],
            ),
            node(
                "ObjectType",
                vec![string("Model")],
                vec![node("Count", vec![integer(count)], vec![])],
            ),
            node(
                "ObjectType",
                vec![string("Geometry")],
                vec![node("Count", vec![integer(count)], vec![])],
            ),
        ],
    );
    let corner_normals = crate::normals::corners(mesh);
    let mut objects = vec![];
    let mut connections = vec![];
    if !bindings.is_empty() {
        definitions.children.push(node(
            "ObjectType",
            vec![string("Material")],
            vec![node("Count", vec![integer(material_count)], vec![])],
        ));
        for (binding, id) in &bindings {
            let name = binding.unwrap_or("Unassigned");
            objects.push(node(
                "Material",
                vec![
                    identifier(*id),
                    string(&format!("{name}\0\u{1}Material")),
                    string(""),
                ],
                vec![
                    node("Version", vec![integer(102)], vec![]),
                    node("ShadingModel", vec![string("lambert")], vec![]),
                ],
            ));
        }
    }
    for (index, group) in groups.iter().enumerate() {
        let vertices = group.vertices();
        let origin = group.members[0].1.origin;
        let model_id = 1000 + index as i64 * 2;
        let geometry_id = model_id + 1;
        let mut indices = BTreeMap::new();
        let mut positions = vec![];
        for (index, vertex) in vertices.iter().enumerate() {
            indices.insert(
                *vertex,
                i32::try_from(index).map_err(|_| "too many object vertices")?,
            );
            for axis in 0..3 {
                let local = mesh.vertices[vertex][axis] - origin[axis];
                if !local.is_finite() {
                    return Err("object coordinates exceed numeric range".into());
                }
                positions.extend(local.to_le_bytes());
            }
        }
        let mut polygons = vec![];
        let mut normals = vec![];
        let mut uv_values = vec![];
        let mut colors = vec![];
        let uv_vertices: BTreeSet<_> = group
            .members
            .iter()
            .filter(|(_, member)| {
                mesh.uvs
                    .keys()
                    .any(|id| member.vertices.contains(&mesh.faces[id][0]))
            })
            .flat_map(|(_, member)| member.vertices.iter().copied())
            .collect();
        let has_uvs = !uv_vertices.is_empty();
        let has_colors = vertices
            .iter()
            .any(|id| mesh.vertex_colors.contains_key(id));
        let mut corners = 0;
        let mut material_slots = BTreeMap::new();
        let mut polygon_materials = Vec::new();
        for (face_id, face) in mesh
            .faces
            .iter()
            .filter(|(_, face)| vertices.contains(&face[0]))
        {
            let mapped_source = uv_vertices.contains(&face[0]);
            if mapped_source && !crate::uv::complete(mesh, *face_id) {
                return Err("FBX UV export requires every corner of each UV-mapped object; inspect_uv to find missing assignments".into());
            }
            if !bindings.is_empty() {
                let binding = mesh.face_textures.get(face_id).map(String::as_str);
                let material_id = bindings[&binding];
                let next = material_slots.len() as i32;
                let slot = material_slots.entry(material_id).or_insert_with(|| {
                    connections.push(node(
                        "C",
                        vec![string("OO"), identifier(material_id), identifier(model_id)],
                        vec![],
                    ));
                    next
                });
                polygon_materials.extend(slot.to_le_bytes());
            }
            for (corner, vertex) in face.iter().enumerate() {
                if has_uvs {
                    let uv = if mapped_source {
                        mesh.uvs[face_id][vertex]
                    } else {
                        [0.0, 0.0]
                    };
                    for value in uv {
                        uv_values.extend(value.to_le_bytes());
                    }
                }
                if has_colors {
                    for value in mesh.vertex_colors.get(vertex).copied().unwrap_or([1.0; 3]) {
                        colors.extend(value.to_le_bytes());
                    }
                    colors.extend(1.0_f64.to_le_bytes());
                }
                let index = indices[vertex];
                let index = if corner + 1 == face.len() {
                    -index - 1
                } else {
                    index
                };
                polygons.extend(index.to_le_bytes());
                for value in corner_normals[face_id][corner] {
                    if !value.is_finite() {
                        return Err("cannot export invalid normals".into());
                    }
                    normals.extend(value.to_le_bytes());
                }
                corners += 1;
            }
        }
        let normal_layer = node(
            "LayerElementNormal",
            vec![integer(0)],
            vec![
                node("Version", vec![integer(101)], vec![]),
                node("Name", vec![string("")], vec![]),
                node(
                    "MappingInformationType",
                    vec![string("ByPolygonVertex")],
                    vec![],
                ),
                node("ReferenceInformationType", vec![string("Direct")], vec![]),
                node("Normals", vec![array(b'd', corners * 3, normals)?], vec![]),
            ],
        );
        let mut layer = node(
            "Layer",
            vec![integer(0)],
            vec![
                node("Version", vec![integer(100)], vec![]),
                node(
                    "LayerElement",
                    vec![],
                    vec![
                        node("Type", vec![string("LayerElementNormal")], vec![]),
                        node("TypedIndex", vec![integer(0)], vec![]),
                    ],
                ),
            ],
        );
        let mut appearance_layers = Vec::new();
        if !material_slots.is_empty() {
            appearance_layers.push(node(
                "LayerElementMaterial",
                vec![integer(0)],
                vec![
                    node("Version", vec![integer(101)], vec![]),
                    node("Name", vec![string("")], vec![]),
                    node("MappingInformationType", vec![string("ByPolygon")], vec![]),
                    node(
                        "ReferenceInformationType",
                        vec![string("IndexToDirect")],
                        vec![],
                    ),
                    node(
                        "Materials",
                        vec![array(b'i', polygon_materials.len() / 4, polygon_materials)?],
                        vec![],
                    ),
                ],
            ));
            layer.children.push(node(
                "LayerElement",
                vec![],
                vec![
                    node("Type", vec![string("LayerElementMaterial")], vec![]),
                    node("TypedIndex", vec![integer(0)], vec![]),
                ],
            ));
        }
        for (enabled, name, field, components, values) in [
            (has_uvs, "LayerElementUV", "UV", 2, uv_values),
            (has_colors, "LayerElementColor", "Colors", 4, colors),
        ] {
            if !enabled {
                continue;
            }
            appearance_layers.push(node(
                name,
                vec![integer(0)],
                vec![
                    node("Version", vec![integer(101)], vec![]),
                    node(
                        "Name",
                        vec![string(if name == "LayerElementUV" {
                            "UVMap"
                        } else {
                            "VertexColors"
                        })],
                        vec![],
                    ),
                    node(
                        "MappingInformationType",
                        vec![string("ByPolygonVertex")],
                        vec![],
                    ),
                    node("ReferenceInformationType", vec![string("Direct")], vec![]),
                    node(
                        field,
                        vec![array(b'd', corners * components, values)?],
                        vec![],
                    ),
                ],
            ));
            layer.children.push(node(
                "LayerElement",
                vec![],
                vec![
                    node("Type", vec![string(name)], vec![]),
                    node("TypedIndex", vec![integer(0)], vec![]),
                ],
            ));
        }
        let mut geometry = node(
            "Geometry",
            vec![
                identifier(geometry_id),
                string(&format!("{}\0\u{1}Geometry", group.name)),
                string("Mesh"),
            ],
            vec![
                node("GeometryVersion", vec![integer(124)], vec![]),
                node(
                    "Vertices",
                    vec![array(b'd', vertices.len() * 3, positions)?],
                    vec![],
                ),
                node(
                    "PolygonVertexIndex",
                    vec![array(b'i', corners, polygons)?],
                    vec![],
                ),
                normal_layer,
                layer,
            ],
        );
        geometry.children.extend(appearance_layers);
        objects.push(geometry);
        let mut properties = node(
            "Properties70",
            vec![],
            vec![
                vector_property("Lcl Translation", origin),
                vector_property("Lcl Rotation", [0.0; 3]),
                vector_property("Lcl Scaling", [1.0; 3]),
                property("RotationActive", "bool", vec![integer(1)]),
                property("InheritType", "enum", vec![integer(1)]),
            ],
        );
        if group.grouped {
            properties.children.push(node(
                "P",
                vec![
                    string("sourcePivots"),
                    string("KString"),
                    string(""),
                    string("U"),
                    string(&group.pivots().to_string()),
                ],
                vec![],
            ));
        }
        objects.push(node(
            "Model",
            vec![
                identifier(model_id),
                string(&format!("{}\0\u{1}Model", group.name)),
                string("Mesh"),
            ],
            vec![
                node("Version", vec![integer(232)], vec![]),
                properties,
                node("Shading", vec![vec![b'C', 1]], vec![]),
                node("Culling", vec![string("CullingOff")], vec![]),
            ],
        ));
        connections.push(node(
            "C",
            vec![string("OO"), identifier(geometry_id), identifier(model_id)],
            vec![],
        ));
        connections.push(node(
            "C",
            vec![string("OO"), identifier(model_id), identifier(0)],
            vec![],
        ));
    }
    let mut bytes = b"Kaydara FBX Binary  \0\x1a\0".to_vec();
    bytes.extend(7400_u32.to_le_bytes());
    for item in [
        header,
        node(
            "FileId",
            vec![raw(&[
                0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b,
                0xfc, 0xf1,
            ])],
            vec![],
        ),
        node(
            "CreationTime",
            vec![string("1970-01-01 10:00:00:000")],
            vec![],
        ),
        node("Creator", vec![string("GPT Mesh")], vec![]),
        globals,
        documents,
        node("References", vec![], vec![]),
        definitions,
        node("Objects", vec![], objects),
        node("Connections", vec![], connections),
        node(
            "Takes",
            vec![],
            vec![node("Current", vec![string("")], vec![])],
        ),
    ] {
        write_node(&item, &mut bytes)?;
    }
    bytes.extend([0; 13]);
    bytes.extend([
        0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26,
        0x7e,
    ]);
    bytes.extend([0; 4]);
    bytes.resize(bytes.len() + 16 - bytes.len() % 16, 0);
    bytes.extend(7400_u32.to_le_bytes());
    bytes.extend([0; 120]);
    bytes.extend([
        0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29,
        0x0b,
    ]);
    Ok(bytes)
}
