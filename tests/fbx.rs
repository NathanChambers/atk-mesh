use atk_mesh::{Document, base64};
use serde_json::json;

#[derive(Debug)]
enum Property {
    Integer(i64),
    Number(f64),
    Text(String),
    Array(Vec<u8>),
    Raw(Vec<u8>),
}

#[derive(Debug)]
struct Element {
    name: String,
    properties: Vec<Property>,
    children: Vec<Element>,
}

fn unsigned(bytes: &[u8], offset: usize) -> usize {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
}

fn read(bytes: &[u8], cursor: &mut usize) -> Element {
    let end = unsigned(bytes, *cursor);
    let count = unsigned(bytes, *cursor + 4);
    let length = unsigned(bytes, *cursor + 8);
    let name_length = bytes[*cursor + 12] as usize;
    *cursor += 13;
    let name = String::from_utf8(bytes[*cursor..*cursor + name_length].to_vec()).unwrap();
    *cursor += name_length;
    let property_end = *cursor + length;
    let mut properties = vec![];
    for _ in 0..count {
        let kind = bytes[*cursor];
        *cursor += 1;
        let property = match kind {
            b'I' => {
                let value = i32::from_le_bytes(bytes[*cursor..*cursor + 4].try_into().unwrap());
                *cursor += 4;
                Property::Integer(value as i64)
            }
            b'L' => {
                let value = i64::from_le_bytes(bytes[*cursor..*cursor + 8].try_into().unwrap());
                *cursor += 8;
                Property::Integer(value)
            }
            b'D' => {
                let value = f64::from_le_bytes(bytes[*cursor..*cursor + 8].try_into().unwrap());
                *cursor += 8;
                Property::Number(value)
            }
            b'C' => {
                let value = bytes[*cursor];
                *cursor += 1;
                Property::Integer(value as i64)
            }
            b'S' => {
                let length = unsigned(bytes, *cursor);
                *cursor += 4;
                let value = String::from_utf8(bytes[*cursor..*cursor + length].to_vec()).unwrap();
                *cursor += length;
                Property::Text(value)
            }
            b'd' | b'i' => {
                let count = unsigned(bytes, *cursor);
                assert_eq!(unsigned(bytes, *cursor + 4), 0);
                let length = unsigned(bytes, *cursor + 8);
                assert_eq!(length, count * if kind == b'd' { 8 } else { 4 });
                *cursor += 12;
                let values = bytes[*cursor..*cursor + length].to_vec();
                *cursor += length;
                Property::Array(values)
            }
            b'R' => {
                let length = unsigned(bytes, *cursor);
                *cursor += 4;
                let value = bytes[*cursor..*cursor + length].to_vec();
                *cursor += length;
                Property::Raw(value)
            }
            _ => panic!("unsupported test property {kind}"),
        };
        properties.push(property);
    }
    assert_eq!(*cursor, property_end);
    if count == 0 {
        assert!(
            end >= property_end + 13,
            "empty containers need a child-list terminator"
        );
    }
    let mut children = vec![];
    while *cursor < end {
        if bytes[*cursor..*cursor + 13] == [0; 13] {
            *cursor += 13;
            break;
        }
        children.push(read(bytes, cursor));
    }
    assert_eq!(*cursor, end);
    Element {
        name,
        properties,
        children,
    }
}

#[test]
fn binary_export_preserves_explicit_uvs_and_vertex_paint_in_polygon_corner_order() {
    let mut document = Document::default();
    let state = document.execute(json!({"command":"inspect"})).unwrap();
    let values: Vec<_> = (1..=8)
        .map(|vertex| json!({"vertex":vertex,"uv":[vertex as f64/10.0,vertex as f64/20.0]}))
        .collect();
    document
        .execute(json!({"command":"uv_set","values":values}))
        .unwrap();
    document
        .execute(json!({"command":"vertex_color_set","vertices":[1],"color":[0.25,0.5,0.75]}))
        .unwrap();
    let result = document
        .execute(json!({"command":"export","format":"fbx"}))
        .unwrap();
    let bytes = base64::decode(result["fbx_base64"].as_str().unwrap()).unwrap();
    let mut cursor = 27;
    let mut elements = Vec::new();
    while unsigned(&bytes, cursor) != 0 {
        elements.push(read(&bytes, &mut cursor));
    }
    let objects = elements.iter().find(|item| item.name == "Objects").unwrap();
    let geometry = objects
        .children
        .iter()
        .find(|item| item.name == "Geometry")
        .unwrap();
    for (layer, field, components) in [
        ("LayerElementUV", "UV", 2),
        ("LayerElementColor", "Colors", 4),
    ] {
        let layer = geometry
            .children
            .iter()
            .find(|item| item.name == layer)
            .unwrap();
        let mapping = layer
            .children
            .iter()
            .find(|item| item.name == "MappingInformationType")
            .unwrap();
        assert!(
            matches!(&mapping.properties[0],Property::Text(value) if value == "ByPolygonVertex")
        );
        let data = layer
            .children
            .iter()
            .find(|item| item.name == field)
            .unwrap();
        let Property::Array(data) = &data.properties[0] else {
            panic!("missing corner data")
        };
        let values: Vec<_> = data
            .chunks_exact(8)
            .map(|value| f64::from_le_bytes(value.try_into().unwrap()))
            .collect();
        let mut expected = Vec::new();
        for face in state["mesh"]["faces"].as_object().unwrap().values() {
            for vertex in face.as_array().unwrap() {
                let vertex = vertex.as_u64().unwrap();
                if components == 2 {
                    expected.extend([vertex as f64 / 10.0, vertex as f64 / 20.0]);
                } else {
                    expected.extend(if vertex == 1 {
                        [0.25, 0.5, 0.75, 1.0]
                    } else {
                        [1.0; 4]
                    });
                }
            }
        }
        assert_eq!(values, expected);
    }
    document
        .execute(json!({"command":"uv_clear","faces":[2],"vertices":[5]}))
        .unwrap();
    assert!(
        document
            .execute(json!({"command":"export","format":"fbx"}))
            .is_err()
    );
    let obj = document
        .execute(json!({"command":"export","format":"obj"}))
        .unwrap();
    let text = obj["obj"].as_str().unwrap();
    assert!(text.contains("v -1 -1 -1 0.25 0.5 0.75\n"));
    assert_eq!(
        text.lines().filter(|line| line.starts_with("vt ")).count(),
        23
    );
}

#[test]
fn binary_export_preserves_local_geometry_pivots_units_and_document() {
    let mut document = Document::default();
    document.execute(json!({"command":"object_create_box","name":"Gate","min":[2,0,-0.1],"max":[3,2,0.1],"origin":[2,0,0]})).unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let result = document
        .execute(json!({"command":"export","format":"fbx"}))
        .unwrap();
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    let bytes = base64::decode(result["fbx_base64"].as_str().unwrap()).unwrap();
    assert_eq!(unsigned(&bytes, 23), 7400);
    let mut cursor = 27;
    let mut elements = vec![];
    while unsigned(&bytes, cursor) != 0 {
        elements.push(read(&bytes, &mut cursor));
    }
    let header = elements
        .iter()
        .find(|item| item.name == "FBXHeaderExtension")
        .unwrap();
    let timestamp = header
        .children
        .iter()
        .find(|item| item.name == "CreationTimeStamp")
        .unwrap();
    let year = timestamp
        .children
        .iter()
        .find(|item| item.name == "Year")
        .unwrap();
    assert!(matches!(year.properties[0], Property::Integer(1970)));
    assert!(header.children.iter().any(|item| item.name == "SceneInfo"));
    let file_id = elements.iter().find(|item| item.name == "FileId").unwrap();
    assert!(matches!(&file_id.properties[0], Property::Raw(value) if value.len() == 16));
    let creation = elements
        .iter()
        .find(|item| item.name == "CreationTime")
        .unwrap();
    assert!(
        matches!(&creation.properties[0], Property::Text(value) if value == "1970-01-01 10:00:00:000")
    );
    let globals = elements
        .iter()
        .find(|item| item.name == "GlobalSettings")
        .unwrap();
    let settings = &globals
        .children
        .iter()
        .find(|item| item.name == "Properties70")
        .unwrap()
        .children;
    let units = settings
        .iter()
        .find(
            |item| matches!(&item.properties[0], Property::Text(name) if name == "UnitScaleFactor"),
        )
        .unwrap();
    assert!(matches!(units.properties[4], Property::Number(100.0)));
    let objects = elements.iter().find(|item| item.name == "Objects").unwrap();
    assert_eq!(objects.children.len(), 4);
    let model = objects
        .children
        .iter()
        .find(|item| {
            item.name == "Model"
                && matches!(&item.properties[1], Property::Text(name) if name.starts_with("Gate\0"))
        })
        .unwrap();
    let properties = &model
        .children
        .iter()
        .find(|item| item.name == "Properties70")
        .unwrap()
        .children;
    let translation = properties
        .iter()
        .find(
            |item| matches!(&item.properties[0], Property::Text(name) if name == "Lcl Translation"),
        )
        .unwrap();
    assert!(matches!(translation.properties[4], Property::Number(2.0)));
    let geometry = objects
        .children
        .iter()
        .find(|item| {
            item.name == "Geometry"
                && matches!(&item.properties[1], Property::Text(name) if name.starts_with("Gate\0"))
        })
        .unwrap();
    let positions = geometry
        .children
        .iter()
        .find(|item| item.name == "Vertices")
        .unwrap();
    let Property::Array(values) = &positions.properties[0] else {
        panic!("expected positions");
    };
    assert_eq!(values.len(), 8 * 3 * 8);
    assert_eq!(f64::from_le_bytes(values[..8].try_into().unwrap()), 0.0);
    assert_eq!(f64::from_le_bytes(values[24..32].try_into().unwrap()), 1.0);
    let links = elements
        .iter()
        .find(|item| item.name == "Connections")
        .unwrap();
    assert_eq!(links.children.len(), 4);
    assert!(matches!(
        links.children[0].properties[1],
        Property::Integer(1001)
    ));
    assert!(matches!(
        links.children[0].properties[2],
        Property::Integer(1000)
    ));
}

#[test]
fn export_writes_smooth_corner_normals_as_unit_vectors() {
    check_normals(false);
    check_normals(true);
}

#[test]
fn consolidated_fbx_and_obj_preserve_appearance_and_source_instances() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"uv_project","mode":"box","object":1}))
        .unwrap();
    for (name, face) in [("Blue", 1), ("Red", 2)] {
        document.execute(json!({"command":"texture_create","texture":name,"width":1,"height":1,"color":[1,0,0,1]})).unwrap();
        document
            .execute(json!({"command":"texture_assign","texture":name,"faces":[face]}))
            .unwrap();
    }
    document
        .execute(json!({"command":"object_instance","object":1,"name":"Other","value":[5,0,0]}))
        .unwrap();
    document
        .execute(json!({"command":"object_set_origin","object":1,"origin":[1,1,0]}))
        .unwrap();
    document
        .execute(
            json!({"command":"collection_create","name":"Pair","objects":[1,2],"primary":true}),
        )
        .unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let baseline = document
        .execute(json!({"command":"export","format":"fbx"}))
        .unwrap();
    let baseline_bytes = base64::decode(baseline["fbx_base64"].as_str().unwrap()).unwrap();
    let baseline_scene = ufbx::load_memory(&baseline_bytes, ufbx::LoadOpts::default()).unwrap();
    let reference_normals: Vec<_> = baseline_scene
        .meshes
        .iter()
        .flat_map(|mesh| {
            mesh.vertex_normal
                .indices
                .iter()
                .map(|index| mesh.vertex_normal.values[*index as usize])
        })
        .collect();
    for mode in ["all", "collection"] {
        let response = document
            .execute(json!({"command":"export","format":"fbx","merge":mode}))
            .unwrap();
        let bytes = base64::decode(response["fbx_base64"].as_str().unwrap()).unwrap();
        let scene = ufbx::load_memory(&bytes, ufbx::LoadOpts::default()).unwrap();
        assert_eq!(scene.meshes.len(), 1);
        let merged = &scene.meshes[0];
        assert_eq!(merged.num_vertices, 16);
        assert_eq!(merged.num_faces, 12);
        assert!(merged.vertex_normal.exists && merged.vertex_uv.exists);
        let node = scene.nodes.iter().find(|node| node.mesh.is_some()).unwrap();
        for (index, point) in merged.vertices.iter().enumerate() {
            let world = ufbx::transform_position(&node.geometry_to_world, *point);
            let expected: &serde_json::Value = &before["mesh"]["vertices"][(index + 1).to_string()];
            for (axis, value) in [world.x, world.y, world.z].iter().enumerate() {
                assert!((value - expected[axis].as_f64().unwrap()).abs() < 1e-10);
            }
        }
        for (index, expected) in merged.vertex_normal.indices.iter().zip(&reference_normals) {
            let normal = merged.vertex_normal.values[*index as usize];
            assert!(
                (normal.x - expected.x).abs()
                    + (normal.y - expected.y).abs()
                    + (normal.z - expected.z).abs()
                    < 1e-10
            );
        }
        assert_eq!(merged.materials.len(), 3);
        assert_eq!(merged.face_material.len(), 12);
        assert_eq!(merged.face_material[0], merged.face_material[6]);
        assert_ne!(merged.face_material[0], merged.face_material[1]);
        let mut cursor = 27;
        let mut nodes = Vec::new();
        while unsigned(&bytes, cursor) != 0 {
            nodes.push(read(&bytes, &mut cursor));
        }
        let model = nodes
            .iter()
            .find(|n| n.name == "Objects")
            .unwrap()
            .children
            .iter()
            .find(|n| n.name == "Model")
            .unwrap();
        let property = model
            .children
            .iter()
            .find(|n| n.name == "Properties70")
            .unwrap()
            .children
            .iter()
            .find(|n| matches!(&n.properties[0],Property::Text(name) if name=="sourcePivots"))
            .unwrap();
        let Property::Text(json) = &property.properties[4] else {
            panic!("missing source pivot metadata")
        };
        let pivots: serde_json::Value = serde_json::from_str(json).unwrap();
        for (index, id) in ["1", "2"].into_iter().enumerate() {
            assert_eq!(
                pivots[index]["origin"],
                before["mesh"]["objects"][id]["origin"]
            );
        }
        let response = document
            .execute(json!({"command":"export","format":"obj","merge":mode}))
            .unwrap();
        let obj = response["obj"].as_str().unwrap();
        assert_eq!(obj.lines().filter(|l| l.starts_with("o ")).count(), 1);
        assert_eq!(obj.lines().filter(|l| l.starts_with("vn ")).count(), 48);
        assert!(obj.contains("usemtl Red\n") && obj.contains("usemtl Blue\n"));
        assert!(obj.contains(&format!("# sourcePivots {pivots}")));
        if mode == "collection" {
            assert!(obj.contains("o Pair\n"));
        }
    }
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
}

#[test]
fn instances_export_resolved_geometry_with_independent_pivots() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_instance","object":1,"name":"Linked","value":[4,2,0]}))
        .unwrap();
    document
        .execute(json!({"command":"object_set_origin","object":2,"origin":[3,0,0]}))
        .unwrap();
    document
        .execute(json!({"command":"object_rotate","object":2,"value":[20,65,-15]}))
        .unwrap();
    let before = document
        .execute(json!({"command":"extrude","object":1,"face":4,"value":[0,1,0]}))
        .unwrap();
    let result = document
        .execute(json!({"command":"export","format":"fbx"}))
        .unwrap();
    let bytes = base64::decode(result["fbx_base64"].as_str().unwrap()).unwrap();
    let mut cursor = 27;
    let mut elements = vec![];
    while unsigned(&bytes, cursor) != 0 {
        elements.push(read(&bytes, &mut cursor));
    }
    let objects = elements.iter().find(|item| item.name == "Objects").unwrap();
    assert_eq!(objects.children.len(), 4);
    for object in before["mesh"]["objects"].as_object().unwrap().values() {
        let prefix = format!("{}\0", object["name"].as_str().unwrap());
        let geometry = objects.children.iter().find(|item| item.name == "Geometry" && matches!(&item.properties[1], Property::Text(name) if name.starts_with(&prefix))).unwrap();
        let model = objects.children.iter().find(|item| item.name == "Model" && matches!(&item.properties[1], Property::Text(name) if name.starts_with(&prefix))).unwrap();
        let properties = model
            .children
            .iter()
            .find(|item| item.name == "Properties70")
            .unwrap();
        let translation = properties.children.iter().find(|item| matches!(&item.properties[0], Property::Text(name) if name == "Lcl Translation")).unwrap();
        let origin: [f64; 3] = serde_json::from_value(object["origin"].clone()).unwrap();
        for (axis, expected) in origin.iter().enumerate() {
            assert!(
                matches!(translation.properties[4 + axis], Property::Number(value) if (value - expected).abs() < 1e-10)
            );
        }
        let positions = geometry
            .children
            .iter()
            .find(|item| item.name == "Vertices")
            .unwrap();
        let Property::Array(values) = &positions.properties[0] else {
            panic!("expected positions");
        };
        let vertices = object["vertices"].as_array().unwrap();
        assert_eq!(values.len(), vertices.len() * 3 * 8);
        for (index, id) in vertices.iter().enumerate() {
            for (axis, offset) in origin.iter().enumerate() {
                let start = (index * 3 + axis) * 8;
                let local = f64::from_le_bytes(values[start..start + 8].try_into().unwrap());
                let expected = before["mesh"]["vertices"][id.to_string()][axis]
                    .as_f64()
                    .unwrap();
                assert!((local + offset - expected).abs() < 1e-10);
            }
        }
    }
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap()["mesh"],
        before["mesh"]
    );
}

fn check_normals(flat_front: bool) {
    let mut document = Document::default();
    document
        .execute(json!({"command":"shade_smooth","object":1,"angle":180}))
        .unwrap();
    if flat_front {
        document
            .execute(json!({"command":"shade_region","face":1,"mode":"flat"}))
            .unwrap();
    }
    let result = document
        .execute(json!({"command":"export","format":"fbx"}))
        .unwrap();
    let bytes = base64::decode(result["fbx_base64"].as_str().unwrap()).unwrap();
    let mut cursor = 27;
    let mut elements = vec![];
    while unsigned(&bytes, cursor) != 0 {
        elements.push(read(&bytes, &mut cursor));
    }
    let objects = elements.iter().find(|n| n.name == "Objects").unwrap();
    let geometry = objects
        .children
        .iter()
        .find(|n| n.name == "Geometry")
        .unwrap();
    let layer = geometry
        .children
        .iter()
        .find(|n| n.name == "LayerElementNormal")
        .unwrap();
    let normals = layer.children.iter().find(|n| n.name == "Normals").unwrap();
    let Property::Array(data) = &normals.properties[0] else {
        panic!("missing normals")
    };
    for (index, corner) in data.chunks_exact(24).enumerate() {
        let n: Vec<_> = corner
            .chunks_exact(8)
            .map(|value| f64::from_le_bytes(value.try_into().unwrap()))
            .collect();
        assert!((n.iter().map(|n| n * n).sum::<f64>() - 1.0).abs() < 1e-10);
        if flat_front && index < 4 {
            assert_eq!(n, vec![0.0, 0.0, -1.0]);
            continue;
        }
        assert!(
            n.iter()
                .all(|n| (n.abs() - 1.0_f64 / 3.0_f64.sqrt()).abs() < 1e-10)
        );
    }
}
