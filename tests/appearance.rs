mod common;

use gpt_mesh::{Document, base64, session::Session};
use serde_json::{Value, json};

fn front_uvs(document: &mut Document) {
    document.execute(json!({"command":"uv_set","faces":[2],"values":[{"vertex":5,"uv":[0,0]},{"vertex":6,"uv":[1,0]},{"vertex":7,"uv":[1,1]},{"vertex":8,"uv":[0,1]}]})).unwrap();
}

fn image(value: &Value) -> (usize, Vec<u8>) {
    let bytes = base64::decode(value["png_base64"].as_str().unwrap()).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    (info.width as usize, pixels)
}

#[test]
fn exact_corner_values_allow_seams_and_transactional_experimentation() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"uv_set","values":[{"vertex":5,"uv":[0.1,0.2]}]}))
        .unwrap();
    document
        .execute(json!({"command":"uv_set","values":[{"face":2,"vertex":5,"uv":[-0.25,1.25]}]}))
        .unwrap();
    let saved = document.execute(json!({"command":"save"})).unwrap();
    assert_eq!(saved["version"], 3);
    assert_eq!(saved["mesh"]["uvs"]["2"]["5"], json!([-0.25, 1.25]));
    assert_eq!(saved["mesh"]["uvs"]["3"]["5"], json!([0.1, 0.2]));
    let queried = document
        .execute(json!({"command":"query_uv","faces":[2],"limit":1}))
        .unwrap();
    assert_eq!(
        queried["items"][0]["corners"][0],
        json!({"vertex":5,"uv":[-0.25,1.25]})
    );
    assert_eq!(queried["items"][0]["complete"], false);
    document.execute(json!({"command":"undo"})).unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for request in [
        json!({"command":"uv_set","values":[{"face":2,"vertex":5,"uv":[0,0]},{"face":2,"vertex":1,"uv":[0,0]}]}),
        json!({"command":"uv_set","faces":[2],"values":[{"face":3,"vertex":5,"uv":[0,0]}]}),
        json!({"command":"uv_transform","scale":[1e308,1e308]}),
        json!({"command":"vertex_color_set","values":[{"vertex":1,"color":[1,0,0]},{"vertex":2,"color":[2,0,0]}]}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), saved);
    let mut restored = Document::default();
    restored
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    assert_eq!(restored.execute(json!({"command":"save"})).unwrap(), saved);
    let mut corrupt = saved.clone();
    corrupt["mesh"]["uvs"]["2"]["9999"] = json!([0, 0]);
    assert!(
        restored
            .execute(json!({"command":"load","project":corrupt}))
            .is_err()
    );
    assert_eq!(restored.execute(json!({"command":"save"})).unwrap(), saved);
    document
        .execute(json!({"command":"object_create_box","name":"Other","min":[3,0,0],"max":[4,1,1]}))
        .unwrap();
    assert!(
        document
            .execute(json!({"command":"uv_set","object":2,"values":[{"vertex":5,"uv":[0,0]}]}))
            .is_err()
    );
    assert!(
        document
            .execute(
                json!({"command":"vertex_color_set","object":2,"vertices":[5],"color":[1,0,0]})
            )
            .is_err()
    );
}

#[test]
fn cpu_captures_show_texture_orientation_tiling_vertex_paint_and_missing_uvs() {
    let mut document = Document::default();
    front_uvs(&mut document);
    let rgba = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    document.execute(json!({"command":"texture_define","name":"quadrants","width":2,"height":2,"rgba_base64":base64::encode(&rgba)})).unwrap();
    document
        .execute(json!({"command":"texture_assign","faces":[2],"texture":"quadrants"}))
        .unwrap();
    let request = json!({"command":"capture","mode":"textured","yaw":0,"pitch":0,"zoom":200,"width":128,"height":128});
    let (width, pixels) = image(&document.execute(request.clone()).unwrap());
    let pixel = |x: usize, y: usize| &pixels[(y * width + x) * 3..(y * width + x) * 3 + 3];
    assert_eq!(pixel(56, 56), [255, 0, 0]);
    assert_eq!(pixel(72, 56), [0, 255, 0]);
    assert_eq!(pixel(56, 72), [0, 0, 255]);
    assert_eq!(pixel(72, 72), [255, 255, 0]);
    document
        .execute(json!({"command":"uv_transform","faces":[2],"offset":[-1,-1]}))
        .unwrap();
    assert_eq!(image(&document.execute(request.clone()).unwrap()).1, pixels);
    document
        .execute(json!({"command":"vertex_color_set","vertices":[5,6,7,8],"color":[0.5,1,1]}))
        .unwrap();
    let (_, painted) = image(&document.execute(request.clone()).unwrap());
    assert!((i32::from(painted[(56 * width + 56) * 3]) - 128).abs() <= 1);
    let colors = document
        .execute(json!({"command":"query_vertex_colors","ids":[5]}))
        .unwrap();
    assert_eq!(colors["items"][0]["color"], json!([0.5, 1.0, 1.0]));
    let mut paint_request = request.clone();
    paint_request["mode"] = json!("vertex_colors");
    let (_, paint_only) = image(&document.execute(paint_request).unwrap());
    assert_eq!(
        &paint_only[(56 * width + 56) * 3 + 1..(56 * width + 56) * 3 + 3],
        [255, 255]
    );
    document
        .execute(json!({"command":"uv_clear","faces":[2],"vertices":[5]}))
        .unwrap();
    let (_, missing) = image(&document.execute(request).unwrap());
    assert_eq!(
        &missing[(56 * width + 56) * 3..(56 * width + 56) * 3 + 3],
        [255, 0, 255]
    );
    assert_eq!(
        document
            .execute(json!({"command":"inspect_uv","faces":[2]}))
            .unwrap()["missing_uv_faces"],
        1
    );
    document
        .execute(json!({"command":"capture","mode":"shaded"}))
        .unwrap();
    let layout = document.execute(json!({"command":"capture_uv","faces":[2],"texture":"quadrants","bounds":[-1,-1,0,0],"size":128})).unwrap();
    assert_eq!(image(&layout).0, 128);
    assert_eq!(layout["report"]["missing_uv_faces"], 1);
    document.execute(json!({"command":"uv_set","faces":[2],"values":[{"vertex":5,"uv":[-1e-30,-1e-30]},{"vertex":6,"uv":[-1e-30,-1e-30]},{"vertex":7,"uv":[-1e-30,-1e-30]},{"vertex":8,"uv":[-1e-30,-1e-30]}]})).unwrap();
    document
        .execute(json!({"command":"capture","mode":"textured","width":64,"height":64}))
        .unwrap();
}

#[test]
fn opaque_preview_preserves_rgb_even_when_texture_alpha_is_partial() {
    let mut document = Document::default();
    front_uvs(&mut document);
    let capture = |document: &mut Document, alpha| {
        let pixels = base64::encode(&[240, 22, 8, alpha]);
        document.execute(json!({"command":"texture_define","name":"paint","width":1,"height":1,"rgba_base64":pixels})).unwrap();
        document.execute(json!({"command":"texture_assign","faces":[2],"texture":"paint"})).unwrap();
        ["textured", "material"].map(|mode| {
            let reply = document.execute(json!({"command":"capture","mode":mode,"width":64,"height":64,"yaw":0,"pitch":0,"zoom":20})).unwrap();
            image(&reply).1
        })
    };
    assert_eq!(capture(&mut document, 255), capture(&mut document, 64));
}

#[test]
fn cutout_pixels_reveal_geometry_behind_and_export_as_a_masked_glb_material() {
    let mut document = Document::default();
    front_uvs(&mut document);
    let pixels = base64::encode(&[255, 0, 0, 0, 0, 255, 0, 160]);
    document.execute(json!({"command":"texture_define","name":"cutout","width":2,"height":1,"rgba_base64":pixels,"alpha_cutoff":0.5})).unwrap();
    document.execute(json!({"command":"texture_assign","faces":[2],"texture":"cutout"})).unwrap();
    let request = json!({"command":"capture","mode":"textured","yaw":0,"pitch":0,"zoom":200,"width":128,"height":128});
    let (width, pixels) = image(&document.execute(request.clone()).unwrap());
    let pixel = |x| &pixels[(56 * width + x) * 3..(56 * width + x) * 3 + 3];
    assert_eq!(pixel(56_usize), [255, 0, 255], "missing UVs on the back face are visible through the cutout");
    assert_eq!(pixel(72_usize), [0, 255, 0], "retained pixels use their source RGB");

    let mut material = request;
    material["mode"] = json!("material");
    let (_, pixels) = image(&document.execute(material).unwrap());
    assert_eq!(&pixels[(56 * width + 56) * 3..(56 * width + 56) * 3 + 3], [255, 0, 255]);

    document.execute(json!({"command":"rig_set","rig":{"bones":[{"name":"root","position":[0,0,0]}],"weights":{}}})).unwrap();
    let exported = document.execute(json!({"command":"export","format":"glb"})).unwrap();
    let bytes = base64::decode(exported["glb_base64"].as_str().unwrap()).unwrap();
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let glb: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    assert_eq!(glb["materials"][0]["alphaMode"], "MASK");
    assert_eq!(glb["materials"][0]["alphaCutoff"], 0.5);
    let primitives = glb["meshes"][0]["primitives"].as_array().unwrap();
    assert_eq!(primitives.len(), 2);
    assert!(primitives[0]["material"].is_null());
    assert_eq!(primitives[1]["material"], 0);
    assert!(primitives[1]["attributes"]["TEXCOORD_0"].is_number());
    let view = glb["images"][0]["bufferView"].as_u64().unwrap() as usize;
    let offset = glb["bufferViews"][view]["byteOffset"].as_u64().unwrap() as usize;
    assert_eq!(&bytes[28 + length + offset..][..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn copies_instances_and_topology_edits_preserve_known_bindings_without_inventing_new_ones() {
    let mut document = Document::default();
    front_uvs(&mut document);
    document
        .execute(json!({"command":"vertex_color_set","vertices":[5],"color":[1,0,0]}))
        .unwrap();
    let duplicated = document
        .execute(json!({"command":"object_duplicate","object":1,"name":"Copy","value":[3,0,0]}))
        .unwrap();
    let operation = &duplicated["operation"];
    let vertex = operation["vertex_remap"]["5"].as_u64().unwrap();
    let face = operation["face_remap"]["2"].as_u64().unwrap();
    assert_eq!(
        duplicated["mesh"]["uvs"][face.to_string()][vertex.to_string()],
        json!([0.0, 0.0])
    );
    assert_eq!(
        duplicated["mesh"]["vertex_colors"][vertex.to_string()],
        json!([1.0, 0.0, 0.0])
    );
    document
        .execute(json!({"command":"object_instance","object":1,"name":"Linked","value":[-3,0,0]}))
        .unwrap();
    assert!(
        document
            .execute(
                json!({"command":"vertex_color_set","object":3,"vertices":[5],"color":[0,1,0]})
            )
            .is_err()
    );
    let mapping = document
        .execute(json!({"command":"query_instance","object":3,"kind":"vertices"}))
        .unwrap();
    let instance_vertex = mapping["items"]["5"].as_u64().unwrap();
    let mapping = document
        .execute(json!({"command":"query_instance","object":3,"kind":"faces"}))
        .unwrap();
    let instance_face = mapping["items"]["2"].as_u64().unwrap();
    assert!(document.execute(json!({"command":"uv_set","values":[{"face":instance_face,"vertex":instance_vertex,"uv":[0.4,0.5]}]})).is_err());
    assert!(
        document
            .execute(
                json!({"command":"vertex_color_set","vertices":[instance_vertex],"color":[0,1,0]})
            )
            .is_err()
    );
    document
        .execute(json!({"command":"uv_set","faces":[2],"values":[{"vertex":5,"uv":[0.2,0.3]}]}))
        .unwrap();
    let saved = document.execute(json!({"command":"save"})).unwrap();
    assert_eq!(
        saved["mesh"]["uvs"][instance_face.to_string()][instance_vertex.to_string()],
        json!([0.2, 0.3])
    );
    assert_eq!(
        saved["mesh"]["vertex_colors"][instance_vertex.to_string()],
        json!([1.0, 0.0, 0.0])
    );
    let mut restored = Document::default();
    restored
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    assert_eq!(restored.execute(json!({"command":"save"})).unwrap(), saved);
    document
        .execute(json!({"command":"split_edge","edge":[5,6]}))
        .unwrap();
    assert_eq!(
        document
            .execute(json!({"command":"inspect_uv","faces":[2]}))
            .unwrap()["missing_uv_faces"],
        1
    );
    let colors = document
        .execute(json!({"command":"query_vertex_colors","ids":[5]}))
        .unwrap();
    assert_eq!(colors["items"][0]["color"], json!([1.0, 0.0, 0.0]));
}

#[test]
fn texture_import_persistence_and_capture_evidence_follow_the_native_session() {
    let root = std::env::temp_dir().join(format!("mesh-appearance-test-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 64, 32, 255]).unwrap();
    }
    std::fs::write(root.join("test.png"), png).unwrap();
    assert_eq!(
        session.execute(json!({"command":"texture_import","name":"test","path":"test.png"}))["ok"],
        true
    );
    let before_import = session.state();
    let stale_import = session
        .execute(json!({"command":"texture_import","name":"stale","path":"test.png","revision":0}));
    assert_eq!(stale_import["ok"], false);
    assert_eq!(session.state(), before_import);
    assert_eq!(
        session.execute(json!({"command":"texture_import","name":"bad","path":"../outside.png"}))["ok"],
        false
    );
    let capture =
        session.execute(json!({"command":"capture","mode":"vertex_colors","width":64,"height":64}));
    let resource = session.execute(json!({"command":"workflow_resource","kind":"capture","session":capture["trace"]["session"],"event":capture["trace"]["event"],"description":"Unpainted cube","object":1}));
    assert_eq!(resource["ok"], true);
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"primary_forms","object":1,"outcome":"accepted","assessment":"Reviewed the unpainted cube.","evidence":[resource["result"]["id"]]}))["ok"],true);
    assert_eq!(
        session.execute(json!({"command":"vertex_color_set","vertices":[5],"color":[1,0,0]}))["ok"],
        true
    );
    let stale = session.execute(json!({"command":"workflow_resource","kind":"capture","session":capture["trace"]["session"],"event":capture["trace"]["event"],"description":"Old capture","object":1}));
    assert_eq!(stale["ok"], false);
    let before = session.state();
    let project: Value =
        serde_json::from_slice(&std::fs::read(root.join("project.json")).unwrap()).unwrap();
    assert_eq!(project["version"], 3);
    drop(session);
    std::fs::remove_file(root.join("test.png")).unwrap();
    let mut reopened = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(reopened.state()["mesh"], before["mesh"]);
    let texture = reopened.execute(json!({"command":"query_uv","kind":"textures"}));
    assert_eq!(texture["result"]["items"][0]["texture"], "test");
    let exported =
        reopened.execute(json!({"command":"texture_export","texture":"test","response":"full"}));
    assert_eq!(image(&exported["result"]).1, [255, 64, 32, 255]);
    let capture = reopened.execute(json!({"command":"capture_uv","size":64}));
    assert!(
        std::path::Path::new(
            capture["result"]["artifacts"]["png"]["path"]
                .as_str()
                .unwrap()
        )
        .is_file()
    );
    std::fs::remove_dir_all(root).unwrap();
}
