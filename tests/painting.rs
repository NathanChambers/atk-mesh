mod common;

use gpt_mesh::{Document, base64, session::Session};
use serde_json::{Value, json};

fn save(document: &mut Document) -> Value {
    document.execute(json!({"command":"save"})).unwrap()
}
fn define(document: &mut Document) {
    document.execute(json!({"command":"texture_create","texture":"paint","width":16,"height":16,"color":[0,0,1,1]})).unwrap();
    document
        .execute(json!({"command":"uv_project","mode":"box"}))
        .unwrap();
    document
        .execute(json!({"command":"texture_assign","texture":"paint"}))
        .unwrap();
}
fn bytes(project: &Value, channel: &str) -> Vec<u8> {
    let texture = &project["mesh"]["textures"]["paint"];
    let data = if channel == "base_color" {
        &texture["rgba_base64"]
    } else {
        &texture["maps"][channel]
    };
    base64::decode(data.as_str().unwrap()).unwrap()
}

#[test]
fn layered_composites_are_repeatable_editable_and_transactional() {
    let mut document = Document::default();
    define(&mut document);
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"red","color":[1,0,0,1],"opacity":0.5}})).unwrap();
    let before = save(&mut document);
    assert_eq!(before["version"], 4);
    assert_eq!(&bytes(&before, "base_color")[..4], &[128, 0, 128, 255]);
    document
        .execute(json!({"command":"texture_bake","texture":"paint"}))
        .unwrap();
    assert_eq!(save(&mut document), before);
    document.execute(json!({"command":"texture_layer_set","texture":"paint","layer":"red","changes":{"visible":false}})).unwrap();
    assert_eq!(
        &bytes(&save(&mut document), "base_color")[..4],
        &[0, 0, 255, 255]
    );
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(save(&mut document), before);
    for request in [
        json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"red"}}),
        json!({"command":"texture_layer_set","texture":"paint","layer":"red","changes":{"opacity":2}}),
        json!({"command":"texture_layer_paint","texture":"paint","layer":"red","strokes":[{"points":[[0,0]],"radius":0}]}),
        json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"bad","channel":"unsupported"}}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(save(&mut document), before);
    }
    let mut restored = Document::default();
    restored
        .execute(json!({"command":"load","project":before}))
        .unwrap();
    assert_eq!(save(&mut restored), before);
    let mut corrupt = before.clone();
    corrupt["mesh"]["textures"]["paint"]["rgba_base64"] =
        json!(base64::encode(&[255u8; 16 * 16 * 4]));
    assert!(
        restored
            .execute(json!({"command":"load","project":corrupt}))
            .is_err()
    );
    assert_eq!(save(&mut restored), before);
}

#[test]
fn masks_strokes_normals_and_layer_removal_have_defined_pixel_results() {
    let mut document = Document::default();
    define(&mut document);
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"rough","channel":"roughness","color":[0.1,0.1,0.1,1]}})).unwrap();
    let first = bytes(&save(&mut document), "roughness");
    document
        .execute(json!({"command":"texture_bake","texture":"paint"}))
        .unwrap();
    assert_eq!(bytes(&save(&mut document), "roughness"), first);
    document.execute(json!({"command":"texture_layer_set","texture":"paint","layer":"rough","changes":{"visible":false}})).unwrap();
    assert!(save(&mut document)["mesh"]["textures"]["paint"]["maps"].is_null());
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"stamp","color":[1,1,1,1],"mask":{"fill":0}}})).unwrap();
    document.execute(json!({"command":"texture_layer_paint","texture":"paint","layer":"stamp","strokes":[{"points":[[0.25,0.75]],"radius":0.15,"hardness":1}]})).unwrap();
    let image = bytes(&save(&mut document), "base_color");
    assert_eq!(
        &image[(3 * 16 + 3) * 4..(3 * 16 + 3) * 4 + 4],
        &[255, 255, 255, 255]
    );
    assert_eq!(
        &image[(12 * 16 + 12) * 4..(12 * 16 + 12) * 4 + 4],
        &[0, 0, 255, 255]
    );
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"height","channel":"height","color":[1,1,1,1],"mask":{"fill":0,"polygons":[[[0,0],[0.5,0],[0.5,1],[0,1]]]}}})).unwrap();
    document
        .execute(json!({"command":"texture_normal_from_height","texture":"paint","strength":2}))
        .unwrap();
    let normal = bytes(&save(&mut document), "normal");
    let index = (8 * 16 + 7) * 4;
    assert!(normal[index] > 200);
    assert!((i32::from(normal[index + 1]) - 128).abs() <= 1);
    assert!(normal[index + 2] > 128);
    let before = save(&mut document);
    document
        .execute(json!({"command":"texture_bake","texture":"paint"}))
        .unwrap();
    assert_eq!(save(&mut document), before);
    document
        .execute(json!({"command":"texture_normal_from_height","texture":"paint","strength":null}))
        .unwrap();
    assert!(save(&mut document)["mesh"]["textures"]["paint"]["maps"]["normal"].is_null());
}

#[test]
fn material_captures_respond_to_detail_maps_without_altering_unlit_color() {
    let mut document = Document::default();
    define(&mut document);
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"base","color":[0.6,0.4,0.25,1]}})).unwrap();
    let request = json!({"command":"capture","mode":"material","width":64,"height":64,"yaw":0,"pitch":0,"zoom":150});
    let initial = document.execute(request.clone()).unwrap()["png_base64"].clone();
    let unlit_request = json!({"command":"capture","mode":"textured","width":64,"height":64,"yaw":0,"pitch":0,"zoom":150});
    let unlit = document.execute(unlit_request.clone()).unwrap()["png_base64"].clone();
    for (name, channel, color) in [
        ("normal", "normal", [1.0, 0.5, 0.5, 1.0]),
        ("rough", "roughness", [1.0, 1.0, 1.0, 1.0]),
        ("spec", "specular", [1.0, 1.0, 1.0, 1.0]),
        ("metal", "metallic", [1.0, 1.0, 1.0, 1.0]),
    ] {
        document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":name,"channel":channel,"color":color}})).unwrap();
        let changed = document.execute(request.clone()).unwrap()["png_base64"].clone();
        assert_ne!(changed, initial, "{channel} must affect lit output");
        assert_eq!(
            document.execute(unlit_request.clone()).unwrap()["png_base64"],
            unlit
        );
        document
            .execute(json!({"command":"texture_layer_remove","texture":"paint","layer":name}))
            .unwrap();
        assert_eq!(
            document.execute(request.clone()).unwrap()["png_base64"],
            initial
        );
    }
}

#[test]
fn tangent_normals_do_not_flip_across_a_front_facing_perspective_surface() {
    let mut document = Document::default();
    define(&mut document);
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"base","color":[0.6,0.4,0.25,1]}})).unwrap();
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"tilt","channel":"normal","color":[0.5,1,0.5,1]}})).unwrap();
    let request = json!({"command":"capture","mode":"material","width":128,"height":128,"projection":"perspective","position":[0,0,4],"target":[0,0,0],"fov":45});
    let decode = |result: Value| {
        let bytes = base64::decode(result["png_base64"].as_str().unwrap()).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut pixels).unwrap();
        pixels
    };
    let pixels = decode(document.execute(request.clone()).unwrap());
    let left = &pixels[(52 * 128 + 64) * 3..(52 * 128 + 64) * 3 + 3];
    let right = &pixels[(76 * 128 + 64) * 3..(76 * 128 + 64) * 3 + 3];
    for axis in 0..3 {
        assert!((i32::from(left[axis]) - i32::from(right[axis])).abs() < 4);
    }
    document
        .execute(json!({"command":"uv_transform","faces":[2],"scale":[1,-1]}))
        .unwrap();
    let mirrored = decode(document.execute(request).unwrap());
    let center = (64 * 128 + 64) * 3;
    assert!((i32::from(pixels[center]) - i32::from(mirrored[center])).abs() > 5);
}

#[test]
fn workspace_reopens_layers_records_masks_and_invalidates_appearance_evidence() {
    let root = std::env::temp_dir().join(format!("mesh-painting-test-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(
        session.execute(json!({"command":"workflow_status"}))["ok"],
        true
    );
    assert_eq!(session.execute(json!({"command":"texture_create","texture":"paint","width":16,"height":16,"color":[0.4,0.3,0.5,1]}))["ok"],true);
    session.execute(json!({"command":"uv_project","mode":"box"}));
    session.execute(json!({"command":"texture_assign","texture":"paint"}));
    session.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"wear","color":[0.8,0.8,0.8,1]}}));
    let mask=session.execute(json!({"command":"texture_layer_mask_faces","texture":"paint","layer":"wear","faces":[2],"mode":"boundary","radius":0.1}));
    assert_eq!(mask["ok"], true);
    let capture = session
        .execute(json!({"command":"texture_layer_capture","texture":"paint","layer":"wear"}));
    assert!(capture["result"]["artifacts"]["png"]["path"].is_string());
    let before = session.state();
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(session.state()["mesh"], before["mesh"]);
    let capture =
        session.execute(json!({"command":"capture","mode":"material","width":64,"height":64}));
    session.execute(json!({"command":"texture_layer_set","texture":"paint","layer":"wear","changes":{"mask":{"noise":{"scale":20,"seed":2,"amount":1,"threshold":0.5}}}}));
    let resource=session.execute(json!({"command":"workflow_resource","kind":"capture","session":capture["trace"]["session"],"event":capture["trace"]["event"],"description":"old render"}));
    assert_eq!(resource["ok"], false);
    drop(session);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn one_damage_mask_updates_material_channels_and_derived_rim_together() {
    let mut document = Document::default();
    define(&mut document);
    document.execute(json!({"command":"texture_mask_set","texture":"paint","name":"damage","mask":{"fill":0,"polygons":[[[0.25,0.25],[0.5,0.25],[0.5,0.5],[0.25,0.5]]]}})).unwrap();
    document.execute(json!({"command":"texture_mask_set","texture":"paint","name":"coating","mask":{"source":"damage","filters":[{"op":"invert"}]}})).unwrap();
    document.execute(json!({"command":"texture_mask_set","texture":"paint","name":"rim","mask":{"source":"damage","filters":[{"op":"outline","side":"outer","radius":0.0625}]}})).unwrap();
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"metal","channels":{"base_color":[0.4,0.4,0.4,1],"roughness":[0.25,0.25,0.25,1],"metallic":[1,1,1,1],"height":[0.5,0.5,0.5,1]}}})).unwrap();
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"paint","channels":{"base_color":[0,0.5,1,1],"roughness":[0.8,0.8,0.8,1],"metallic":[0,0,0,1],"height":[0.52,0.52,0.52,1]},"mask":{"source":"coating"}}})).unwrap();
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"lifted-edge","channel":"height","color":[0.6,0.6,0.6,1],"mask":{"source":"rim"}}})).unwrap();
    document
        .execute(json!({"command":"texture_normal_from_height","texture":"paint","strength":2}))
        .unwrap();
    let original = save(&mut document);
    assert_eq!(original["version"], 5);
    let inside = (10 * 16 + 5) * 4;
    let outside = (10 * 16 + 10) * 4;
    let rim = (10 * 16 + 8) * 4;
    assert_eq!(
        &bytes(&original, "base_color")[inside..inside + 3],
        &[102, 102, 102]
    );
    assert_eq!(bytes(&original, "roughness")[inside], 64);
    assert_eq!(bytes(&original, "metallic")[inside], 255);
    assert_eq!(
        &bytes(&original, "base_color")[outside..outside + 3],
        &[0, 128, 255]
    );
    assert_eq!(bytes(&original, "roughness")[outside], 204);
    assert_eq!(bytes(&original, "metallic")[outside], 0);
    assert_eq!(bytes(&original, "height")[rim], 153);
    document.execute(json!({"command":"texture_mask_transform","texture":"paint","name":"damage","offset":[0.25,0]})).unwrap();
    let moved = save(&mut document);
    assert_eq!(
        &bytes(&moved, "base_color")[inside..inside + 3],
        &[0, 128, 255]
    );
    assert_eq!(
        &bytes(&moved, "base_color")[outside..outside + 3],
        &[102, 102, 102]
    );
    assert_eq!(bytes(&moved, "roughness")[inside], 204);
    assert_eq!(bytes(&moved, "metallic")[inside], 0);
    assert_eq!(bytes(&moved, "roughness")[outside], 64);
    assert_eq!(bytes(&moved, "metallic")[outside], 255);
    assert_ne!(bytes(&original, "normal"), bytes(&moved, "normal"));
    assert_eq!(
        original["mesh"]["textures"]["paint"]["paint"]["layers"],
        moved["mesh"]["textures"]["paint"]["paint"]["layers"]
    );
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(save(&mut document), original);
    document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(save(&mut document), moved);
    let revision = document.revision();
    for request in [
        json!({"command":"texture_mask_remove","texture":"paint","name":"damage"}),
        json!({"command":"texture_mask_set","texture":"paint","name":"damage","mask":{"source":"coating"}}),
        json!({"command":"texture_mask_set","texture":"paint","name":"bad","mask":{"source":"absent"}}),
        json!({"command":"texture_mask_paint","texture":"paint","name":"damage","strokes":[{"points":[],"radius":0.1}]}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(save(&mut document), moved);
        assert_eq!(document.revision(), revision);
    }
    let mut restored = Document::default();
    restored
        .execute(json!({"command":"load","project":moved}))
        .unwrap();
    assert_eq!(save(&mut restored), moved);
}

#[test]
fn mask_filters_have_predictable_pixel_results() {
    let mut document = Document::default();
    define(&mut document);
    document.execute(json!({"command":"texture_mask_set","texture":"paint","name":"square","mask":{"fill":0,"polygons":[[[0.25,0.25],[0.5,0.25],[0.5,0.5],[0.25,0.5]]]}})).unwrap();
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"filtered","color":[1,1,1,1],"mask":{"source":"square"}}})).unwrap();
    let render = |document: &mut Document, filters: Value| {
        document.execute(json!({"command":"texture_layer_set","texture":"paint","layer":"filtered","changes":{"mask":{"filters":filters}}})).unwrap();
        bytes(&save(document), "base_color")
    };
    let expanded = render(&mut document, json!([{"op":"dilate","radius":0.0625}]));
    assert_eq!(expanded[(9 * 16 + 8) * 4], 255);
    assert_eq!(expanded[(9 * 16 + 9) * 4], 0);
    let eroded = render(&mut document, json!([{"op":"erode","radius":0.0625}]));
    assert_eq!(eroded[(9 * 16 + 4) * 4], 0);
    assert_eq!(eroded[(9 * 16 + 5) * 4], 255);
    let outline = render(
        &mut document,
        json!([{"op":"outline","radius":0.0625,"side":"outer"}]),
    );
    assert_eq!(outline[(9 * 16 + 5) * 4], 0);
    assert_eq!(outline[(9 * 16 + 8) * 4], 255);
    let blurred = render(&mut document, json!([{"op":"blur","radius":0.0625}]));
    assert!((i32::from(blurred[(9 * 16 + 8) * 4]) - 85).abs() <= 1);
    let sharpened = render(
        &mut document,
        json!([{"op":"blur","radius":0.0625},{"op":"levels","low":0.5,"high":0.8}]),
    );
    assert_eq!(sharpened[(9 * 16 + 8) * 4], 0);
    let inverted = render(&mut document, json!([{"op":"invert"}]));
    assert_eq!(inverted[(9 * 16 + 5) * 4], 0);
    assert_eq!(inverted[(9 * 16 + 8) * 4], 255);
}

#[test]
fn alpha_stamps_snapshot_images_and_support_later_erase_strokes() {
    let mut document = Document::default();
    define(&mut document);
    document.execute(json!({"command":"texture_create","texture":"alpha","width":2,"height":2,"color":[1,1,1,1]})).unwrap();
    document.execute(json!({"command":"texture_mask_set","texture":"paint","name":"stamps","mask":{"fill":0}})).unwrap();
    document.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"stamped","color":[1,0,0,1],"mask":{"source":"stamps"}}})).unwrap();
    document.execute(json!({"command":"texture_mask_stamp","texture":"paint","name":"stamps","source_texture":"alpha","center":[0.5,0.5],"size":[0.5,0.25],"rotation":90})).unwrap();
    let stamped = save(&mut document);
    let pixels = bytes(&stamped, "base_color");
    assert_eq!(pixels[(7 * 16 + 7) * 4], 255);
    assert_eq!(pixels[(7 * 16 + 11) * 4], 0);
    assert!(pixels[(5 * 16 + 7) * 4] > 0);
    document
        .execute(json!({"command":"texture_remove","texture":"alpha"}))
        .unwrap();
    assert_eq!(bytes(&save(&mut document), "base_color"), pixels);
    document.execute(json!({"command":"texture_mask_paint","texture":"paint","name":"stamps","strokes":[{"points":[[0.5,0.5]],"radius":0.1,"hardness":1,"value":0}]})).unwrap();
    let erased = bytes(&save(&mut document), "base_color");
    assert_eq!(erased[(7 * 16 + 7) * 4], 0);
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(bytes(&save(&mut document), "base_color"), pixels);
    let query = document
        .execute(json!({"command":"query_texture_masks","texture":"paint","limit":1}))
        .unwrap();
    assert!(query["items"][0]["mask"]["marks"][0]["image"]["rgba_base64"].is_null());
}

#[test]
fn shared_masks_and_material_bundles_survive_workspace_reopening() {
    let root = std::env::temp_dir().join(format!("mesh-shared-masks-{}", std::process::id()));
    let mut session = Session::open_workspace(root.clone()).unwrap();
    for request in [
        json!({"command":"texture_create","texture":"paint","width":16,"height":16,"color":[0,0,1,1]}),
        json!({"command":"texture_mask_set","texture":"paint","name":"damage","mask":{"fill":0,"polygons":[[[0.2,0.2],[0.6,0.2],[0.6,0.6],[0.2,0.6]]]}}),
        json!({"command":"texture_mask_set","texture":"paint","name":"coat","mask":{"source":"damage","filters":[{"op":"invert"}]}}),
        json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"coat","channels":{"base_color":[1,0,0,1],"roughness":[0.7,0.7,0.7,1]},"mask":{"source":"coat"}}}),
        json!({"command":"texture_mask_transform","texture":"paint","name":"damage","offset":[0.125,0]}),
    ] {
        let result = session.execute(request);
        assert_eq!(result["ok"], true, "{result}");
    }
    let before = session.state();
    let capture =
        session.execute(json!({"command":"texture_mask_capture","texture":"paint","name":"coat"}));
    assert!(capture["result"]["artifacts"]["png"]["path"].is_string());
    let query =
        session.execute(json!({"command":"query_texture_masks","texture":"paint","limit":1}));
    assert_eq!(query["result"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(query["result"]["next_offset"], 1);
    let project: Value =
        serde_json::from_slice(&std::fs::read(root.join("project.json")).unwrap()).unwrap();
    assert_eq!(project["version"], 5);
    drop(session);
    let mut reopened = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(reopened.state()["mesh"], before["mesh"]);
    assert_eq!(
        reopened
            .execute(json!({"command":"texture_mask_remove","texture":"paint","name":"damage"}))["ok"],
        false
    );
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}
