use atk_mesh::{Document, base64};
use serde_json::{Value, json};

#[test]
fn band_stretch_keeps_hardware_shape_placement_and_material_bindings() {
    let mut doc = Document::default();
    doc.execute(
        json!({"command":"object_create_box","name":"Mount","min":[1,0,0],"max":[1.25,0.25,0.25]}),
    )
    .unwrap();
    doc.execute(
        json!({"command":"object_instance","object":2,"name":"Linked mount","value":[-2.25,0,0]}),
    )
    .unwrap();
    doc.execute(json!({"command":"uv_project","object":1,"mode":"box"}))
        .unwrap();
    let before = doc.execute(json!({"command":"save"})).unwrap();
    let result = doc.execute(json!({"command":"stretch_band","axis":"X","span":[-0.2,0.2],"delta":0.3,"objects":[1],"rigid_groups":[[2],[3]]})).unwrap();
    let after = &result["mesh"];
    assert_eq!(after["uvs"], before["mesh"]["uvs"]);
    assert_eq!(after["faces"], before["mesh"]["faces"]);
    assert_eq!(
        result["operation"]["rigid_groups"][0]["translation"][0],
        0.15
    );
    assert_eq!(
        result["operation"]["rigid_groups"][1]["translation"][0],
        -0.15
    );
    for (object, delta) in [("2", 0.15), ("3", -0.15)] {
        for v in after["objects"][object]["vertices"].as_array().unwrap() {
            let id = v.to_string();
            let x = after["vertices"][&id][0].as_f64().unwrap();
            assert!(
                (x - before["mesh"]["vertices"][&id][0].as_f64().unwrap() - delta).abs() < 1e-10
            );
        }
    }
    assert_eq!(after["vertices"]["1"][0], -1.15);
    assert_eq!(after["vertices"]["2"][0], 1.15);
    doc.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(doc.execute(json!({"command":"save"})).unwrap(), before);
}

fn glb(result: &Value) -> (Value, Vec<u8>) {
    let bytes = base64::decode(result["glb_base64"].as_str().unwrap()).unwrap();
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let metadata = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    (metadata, bytes[28 + length..].to_vec())
}

fn pixels(document: &Value, binary: &[u8], binding: &Value) -> Vec<u8> {
    let image = &document["images"][binding["index"].as_u64().unwrap() as usize];
    let view = &document["bufferViews"][image["bufferView"].as_u64().unwrap() as usize];
    let offset = view["byteOffset"].as_u64().unwrap() as usize;
    let length = view["byteLength"].as_u64().unwrap() as usize;
    let mut reader = png::Decoder::new(&binary[offset..offset + length])
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).unwrap();
    pixels
}

#[test]
fn solid_operations_and_regeneration_keep_operands_and_transfer_face_bindings() {
    let mut doc = Document::empty();
    doc.execute(json!({"command":"object_create_box","region":"body","name":"Housing","min":[-1,-1,-1],"max":[1,1,1]})).unwrap();
    doc.execute(
        json!({"command":"object_create_box","name":"Vent","min":[-0.3,-2,-0.3],"max":[0.3,2,0.3]}),
    )
    .unwrap();
    doc.execute(json!({"command":"uv_project","object":1,"mode":"box"}))
        .unwrap();
    doc.execute(json!({"command":"texture_create","texture":"Paint","width":2,"height":2,"color":[1,0,0,1]})).unwrap();
    doc.execute(json!({"command":"texture_assign","object":1,"texture":"Paint"}))
        .unwrap();
    for (name, volume) in [
        ("union", 8.72),
        ("intersection", 0.72),
        ("difference", 7.28),
    ] {
        let result = doc.execute(json!({"command":"solid_boolean","operation":name,"base":1,"tools":[2],"name":name,"region":name})).unwrap();
        assert!((result["operation"]["volume"].as_f64().unwrap() - volume).abs() < 1e-8);
        assert_eq!(result["validation"]["boundary_edges"], 0);
        assert_eq!(result["validation"]["non_manifold_edges"], 0);
        let faces = result["operation"]["created_face_ids"].as_array().unwrap();
        assert!(
            faces
                .iter()
                .any(|f| result["mesh"]["face_textures"][f.to_string()] == "Paint")
        );
    }
    doc.execute(json!({"command":"regenerate_component","region":"body","object":1,"overrides":{"min":[-1.15,-1,-1],"max":[1.15,1,1]}})).unwrap();
    let updated = doc
        .execute(json!({"command":"regenerate_component","object":5,"region":"difference"}))
        .unwrap();
    assert!((updated["operation"]["volume"].as_f64().unwrap() - 8.48).abs() < 1e-8);
    let revision = doc.revision();
    assert!(doc.execute(json!({"command":"regenerate_component","object":5,"region":"difference","overrides":{"tools":[5]}})).is_err());
    assert_eq!(doc.revision(), revision);
    let (metadata, _) = glb(&doc
        .execute(json!({"command":"export","format":"glb","solo_objects":[5]}))
        .unwrap());
    assert_eq!(metadata["nodes"].as_array().unwrap().len(), 1);
    assert!(metadata["skins"].is_null());
    assert!(metadata["nodes"][0]["skin"].is_null());
    assert_eq!(metadata["materials"][0]["name"], "Paint");
    assert!(
        doc.execute(json!({"command":"export","format":"glb","solo_objects":[]}))
            .is_err()
    );
}

#[test]
fn atlas_and_static_glb_preserve_corner_topology_and_pack_pbr_channels() {
    let mut doc = Document::default();
    let atlas = doc
        .execute(
            json!({"command":"uv_unwrap","object":1,"seams":[[1,2]],"resolution":64,"padding":2}),
        )
        .unwrap();
    assert_eq!(atlas["operation"]["faces"], 6);
    assert!(atlas["operation"]["charts"].as_u64().unwrap() >= 2);
    let repacked = doc
        .execute(json!({"command":"uv_pack","object":1,"resolution":128}))
        .unwrap();
    assert_eq!(atlas["mesh"]["faces"], repacked["mesh"]["faces"]);
    assert_eq!(repacked["mesh"]["uvs"].as_object().unwrap().len(), 6);
    assert!(
        doc.execute(json!({"command":"uv_unwrap","object":1,"seams":[[1,7]]}))
            .is_err()
    );
    doc.execute(json!({"command":"texture_create","texture":"Finish","width":1,"height":1,"color":[1,0,0,1]})).unwrap();
    doc.execute(json!({"command":"texture_layer_add","texture":"Finish","layer":{"name":"finish","channels":{"normal":[0.5,0.25,1,1],"roughness":[0.25,0.25,0.25,1],"metallic":[0.75,0.75,0.75,1]}}})).unwrap();
    doc.execute(json!({"command":"texture_assign","object":1,"texture":"Finish"}))
        .unwrap();
    doc.execute(json!({"command":"object_set_origin","object":1,"origin":[2,0,0]}))
        .unwrap();
    let (metadata, binary) = glb(&doc
        .execute(json!({"command":"export","format":"glb"}))
        .unwrap());
    let material = &metadata["materials"][0];
    assert!(metadata["extensionsUsed"].is_null());
    assert_eq!(metadata["nodes"][0]["translation"], json!([2.0, 0.0, 0.0]));
    assert_eq!(
        pixels(
            &metadata,
            &binary,
            &material["pbrMetallicRoughness"]["baseColorTexture"]
        ),
        [255, 0, 0, 255]
    );
    assert_eq!(
        pixels(
            &metadata,
            &binary,
            &material["pbrMetallicRoughness"]["metallicRoughnessTexture"]
        ),
        [255, 64, 191, 255]
    );
    assert_eq!(
        pixels(&metadata, &binary, &material["normalTexture"]),
        [128, 191, 255, 255]
    );
}

#[test]
fn subdivision_retains_cage_uv_seams_weights_and_sharp_corners() {
    let mut doc = Document::default();
    doc.execute(json!({"command":"uv_unwrap","object":1,"resolution":32}))
        .unwrap();
    doc.execute(json!({"command":"rig_set","rig":{"bones":[{"name":"root","position":[0,0,0]}],"weights":{}}})).unwrap();
    doc.execute(json!({"command":"skin_weights_set","weights":{"1":[{"bone":"root","weight":1}]}}))
        .unwrap();
    let saved = doc.execute(json!({"command":"save"})).unwrap();
    let request = json!({"command":"subdivision_surface","name":"Limit","region":"skin","source":1,"levels":2,"creases":[[1,2],[1,4],[1,5]]});
    let result = doc.execute(request).unwrap();
    assert_eq!(result["operation"]["quads"], 96);
    assert_eq!(result["operation"]["geometry_counts"]["vertices"], 98);
    assert_eq!(
        result["mesh"]["objects"]["1"],
        saved["mesh"]["objects"]["1"]
    );
    assert_eq!(
        result["mesh"]["vertices"]["9"],
        saved["mesh"]["vertices"]["1"]
    );
    assert_eq!(
        result["mesh"]["rig"]["weights"]["9"],
        saved["mesh"]["rig"]["weights"]["1"]
    );
    assert_eq!(result["mesh"]["uvs"].as_object().unwrap().len(), 102);
    let result = doc.execute(json!({"command":"regenerate_component","object":2,"region":"skin","overrides":{"levels":1}})).unwrap();
    assert_eq!(result["operation"]["quads"], 24);
    doc.execute(json!({"command":"undo"})).unwrap();
    doc.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(doc.execute(json!({"command":"save"})).unwrap(), saved);
}

#[test]
fn baking_projects_high_normals_into_the_target_tangent_frame() {
    let mut doc = Document::default();
    doc.execute(json!({"command":"object_duplicate","object":1,"name":"High"}))
        .unwrap();
    doc.execute(json!({"command":"object_rotate","object":2,"value":[0,10,0]}))
        .unwrap();
    doc.execute(
        json!({"command":"uv_project","object":1,"faces":[2],"mode":"planar","axes":[0,1]}),
    )
    .unwrap();
    doc.execute(json!({"command":"texture_create","texture":"Panel","width":16,"height":16,"color":[1,1,1,1]})).unwrap();
    doc.execute(json!({"command":"texture_assign","object":1,"faces":[2],"texture":"Panel"}))
        .unwrap();
    let bake = json!({"command":"texture_bake_normals","texture":"Panel","source":2,"target":1,"max_distance":0.4});
    let result = doc.execute(bake.clone()).unwrap();
    assert!(result["operation"]["projection"]["hits"].as_u64().unwrap() > 180);
    let encoded = result["mesh"]["textures"]["Panel"]["maps"]["normal"]
        .as_str()
        .unwrap();
    let map = base64::decode(encoded).unwrap();
    let center = (8 * 16 + 8) * 4;
    assert!((148..=152).contains(&map[center]));
    assert!(map[center + 2] > 250);
    let revision = doc.revision();
    let height = doc.execute(json!({"command":"bake_surface_map","channel":"displacement","source":2,"target":1,"texture":"Panel","max_distance":0.4,"resolution":32})).unwrap();
    assert_eq!(doc.revision(), revision);
    assert_eq!(height["zero_code"], 32768);
    let png_bytes = base64::decode(height["png_base64"].as_str().unwrap()).unwrap();
    let mut reader = png::Decoder::new(png_bytes.as_slice()).read_info().unwrap();
    assert_eq!(reader.info().bit_depth, png::BitDepth::Sixteen);
    assert_eq!(reader.info().color_type, png::ColorType::Grayscale);
    let mut height = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut height).unwrap();
    let codes: Vec<_> = height
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    assert!(codes.iter().any(|&v| v < 30000));
    assert!(codes.iter().any(|&v| v > 35000));
    doc.execute(json!({"command":"object_move","object":2,"value":[6,0,0]}))
        .unwrap();
    let revision = doc.revision();
    assert!(doc.execute(bake).is_err());
    assert_eq!(doc.revision(), revision);
}

#[test]
fn compressed_events_replay_through_the_editor_and_saved_viewer() {
    use atk_mesh::session::Session;
    let root = std::env::temp_dir().join(format!("mesh-asset-compression-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let mut session = Session::open_with_recording_compression(root.clone(), true).unwrap();
    let reply = session
        .execute(json!({"command":"object_create_box","name":"Case","min":[0,0,0],"max":[1,1,1]}));
    assert_eq!(reply["ok"], true);
    let route = reply["trace"]["record"].as_str().unwrap();
    let id = reply["trace"]["session"].as_str().unwrap();
    assert!(
        root.join("sessions")
            .join(id)
            .join("states/1.json.gz")
            .is_file()
    );
    let event = session.read(route).unwrap();
    assert_eq!(
        event["full_output"]["result"]["mesh"],
        session.state()["mesh"]
    );
    drop(session);
    let workspace = std::fs::canonicalize(&root).unwrap();
    assert_eq!(
        Session::read_saved_recordings(&workspace, route).unwrap(),
        event
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rounded_cap_rim_meets_cleanly_and_rejects_excessive_width_atomically() {
    let mut doc = Document::default();
    let mut rim = json!({"command":"bevel_edges","corner_mode":"rim","object":1,"edges":[[5,6],[7,6],[8,7],[5,8]],"width":1.1,"segments":4});
    assert!(doc.execute(rim.clone()).is_err());
    assert_eq!(doc.revision(), 0);
    rim["width"] = json!(0.2);
    let result = doc.execute(rim).unwrap();
    assert_eq!(
        result["operation"]["created_faces"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
    assert_eq!(result["validation"]["boundary_edges"], 0);
    assert_eq!(result["validation"]["non_manifold_edges"], 0);
}
