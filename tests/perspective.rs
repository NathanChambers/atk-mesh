mod common;

use atk_mesh::{Document, base64, session::Session};
use serde_json::{Value, json};
use std::io::Cursor;

fn pixels(result: &Value) -> Vec<u8> {
    let bytes = base64::decode(result["png_base64"].as_str().unwrap()).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut output).unwrap();
    output.truncate(info.buffer_size());
    output
}

#[test]
fn perspective_clips_near_plane_and_supports_all_modes_and_image_selection() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"save"})).unwrap();
    for mode in [
        "shaded",
        "normals",
        "depth",
        "face_ids",
        "object_ids",
        "orientation",
        "silhouette",
        "topology",
    ] {
        let capture=document.execute(json!({"command":"capture","projection":"perspective","position":[0,0,0.5],"yaw":0,"pitch":0,"fov":80,"near":0.1,"mode":mode,"width":200,"height":160})).unwrap();
        assert_eq!(capture["camera"]["projection"], "perspective");
        let image = pixels(&capture);
        assert_eq!(image.len(), 200 * 160 * 3);
        if mode == "silhouette" {
            assert!(image.chunks(3).any(|pixel| pixel == [0, 0, 0]));
        }
        let selected=document.execute(json!({"command":"select_image","view_id":capture["view_id"],"rect":[0,0,200,160],"through":false})).unwrap();
        assert!(
            selected["mesh"]["selection"]
                .as_array()
                .unwrap()
                .iter()
                .all(|id| [1, 2, 3, 4].contains(&id.as_u64().unwrap()))
        );
    }
    assert_eq!(
        document.execute(json!({"command":"save"})).unwrap()["mesh"]["vertices"],
        before["mesh"]["vertices"]
    );
    let empty=document.execute(json!({"command":"capture","projection":"perspective","position":[0,0,-2],"yaw":0,"pitch":0,"width":100,"height":100,"mode":"silhouette"})).unwrap();
    assert!(pixels(&empty).iter().all(|value| *value == 255));
}

#[test]
fn perspective_screen_moves_preserve_depth_and_scale_with_anchor_distance() {
    let mut document = Document::default();
    let original = document.execute(json!({"command":"save"})).unwrap();
    let camera = json!({"projection":"perspective","position":[0,0,5],"yaw":0,"pitch":0,"fov":90,"width":200,"height":200});
    let front = document
        .execute(
            json!({"command":"move_in_view","camera":camera,"vertices":[5],"delta_pixels":[10,0]}),
        )
        .unwrap();
    let back = document
        .execute(
            json!({"command":"move_in_view","camera":camera,"vertices":[1],"delta_pixels":[10,0]}),
        )
        .unwrap();
    assert!((front["mesh"]["vertices"]["5"][0].as_f64().unwrap() + 0.6).abs() < 1e-10);
    assert!((back["mesh"]["vertices"]["1"][0].as_f64().unwrap() + 0.4).abs() < 1e-10);
    assert_eq!(
        back["mesh"]["vertices"]["1"][2],
        original["mesh"]["vertices"]["1"][2]
    );
    document.execute(json!({"command":"undo"})).unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(
        document.execute(json!({"command":"save"})).unwrap(),
        original
    );
}

#[test]
fn perspective_camera_fit_persists_without_mesh_mutation_and_reference_move_is_exact() {
    let root = std::env::temp_dir().join(format!("mesh-perspective-fit-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let capture=session.execute(json!({"command":"capture","projection":"perspective","position":[0,0,5],"yaw":0,"pitch":0,"fov":60,"width":400,"height":400}));
    assert_eq!(capture["ok"], true, "{capture}");
    std::fs::copy(
        capture["result"]["artifacts"]["png"]["path"]
            .as_str()
            .unwrap(),
        root.join("reference.png"),
    )
    .unwrap();
    session.execute(json!({"command":"workflow_resource","kind":"reference","path":"reference.png","description":"Synthetic perspective","view":{"projection":"perspective","position":[0.2,0.15,5.4],"yaw":2,"pitch":-1,"fov":63,"width":400,"height":400}}));
    let state = session.state();
    let focal = 200.0 / (30_f64.to_radians().tan());
    let landmarks: Vec<_>=state["mesh"]["vertices"].as_object().unwrap().iter().map(|(id,p)| {
        let distance=5.0-p[2].as_f64().unwrap();
        json!({"name":format!("v{id}"),"vertices":[id.parse::<u64>().unwrap()],"pixel":[200.0+focal*p[0].as_f64().unwrap()/distance,200.0-focal*p[1].as_f64().unwrap()/distance]})
    }).collect();
    assert_eq!(
        session.execute(
            json!({"command":"reference_landmarks","reference":"resource-1","landmarks":landmarks})
        )["ok"],
        true
    );
    let bounded = session.execute(json!({"command":"fit_reference","reference":"resource-1","response":"full","fit_parameters":["fov"],"bounds":{"fov":[61,64]}}));
    assert_eq!(bounded["ok"], true, "{bounded}");
    assert_eq!(
        bounded["result"]["view"]["position"],
        json!([0.2, 0.15, 5.4])
    );
    assert_eq!(bounded["result"]["view"]["yaw"].as_f64(), Some(2.0));
    let bounded_fov = bounded["result"]["view"]["fov"].as_f64().unwrap();
    assert!((61.0..=64.0).contains(&bounded_fov));
    assert_eq!(
        session.execute(
            json!({"command":"fit_reference","reference":"resource-1","bounds":{"fov":[70,60]}})
        )["ok"],
        false
    );
    assert_eq!(
        session
            .execute(json!({"command":"reference_view","reference":"resource-1","target":[0,0,0]}))
            ["ok"],
        true
    );
    let explicit = session
        .execute(json!({"command":"reference_view","reference":"resource-1","yaw":2,"pitch":-1}));
    assert_eq!(explicit["ok"], true, "{explicit}");
    let fitted = session.execute(json!({"command":"fit_reference","reference":"resource-1"}));
    assert_eq!(fitted["ok"], true, "{fitted}");
    assert!(
        fitted["result"]["after_rms_error_pixels"].as_f64().unwrap() < 1e-5,
        "{fitted}"
    );
    assert_eq!(session.state(), state);
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let report = session.execute(json!({"command":"project_reference","reference":"resource-1"}));
    assert!(report["result"]["rms_error_pixels"].as_f64().unwrap() < 1e-5);
    let moved=session.execute(json!({"command":"move_in_reference","reference":"resource-1","landmark":"v5","revision":report["result"]["revision"],"alignment_version":report["result"]["alignment_version"],"delta_pixels":[8,0]}));
    assert_eq!(moved["ok"], true, "{moved}");
    let after = session
        .execute(json!({"command":"project_reference","reference":"resource-1","names":["v5"]}));
    assert!(
        (after["result"]["landmarks"][0]["delta_pixels"][0]
            .as_f64()
            .unwrap()
            + 8.0)
            .abs()
            < 1e-5
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_perspective_requests_are_atomic() {
    let mut document = Document::default();
    let original = document.execute(json!({"command":"save"})).unwrap();
    for request in [
        json!({"command":"capture","projection":"bad"}),
        json!({"command":"capture","projection":"perspective"}),
        json!({"command":"capture","projection":"perspective","position":[0,0,3],"fov":0}),
        json!({"command":"capture","projection":"perspective","position":[0,0,3],"near":-1}),
        json!({"command":"move_in_view","vertices":[5],"delta_pixels":[1,0],"camera":{"projection":"perspective","position":[0,0,-2],"yaw":0,"pitch":0}}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"save"})).unwrap(),
            original
        );
    }
}

#[test]
fn reciprocal_depth_resolves_crossing_faces_and_depth_mode_remains_linear() {
    let mut document = Document::default();
    document.execute(json!({"command":"load","project":{"version":1,"mesh":{"vertices":{"1":[-1,-1,0],"2":[1,-1,2],"3":[1,1,2],"4":[-1,1,0],"5":[-1,-1,1],"6":[1,-1,1],"7":[1,1,1],"8":[-1,1,1]},"faces":{"1":[1,2,3,4],"2":[5,6,7,8]},"selection":[]}}})).unwrap();
    let mut request = json!({"command":"capture","projection":"perspective","position":[0,0,5],"yaw":0,"pitch":0,"fov":90,"width":400,"height":400,"mode":"face_ids"});
    let capture = document.execute(request.clone()).unwrap();
    let image = pixels(&capture);
    let color_at = |x: usize| {
        format!(
            "#{:02x}{:02x}{:02x}",
            image[(200 * 400 + x) * 3],
            image[(200 * 400 + x) * 3 + 1],
            image[(200 * 400 + x) * 3 + 2]
        )
    };
    assert_eq!(capture["face_id_colors"][color_at(195)], 2);
    assert_eq!(capture["face_id_colors"][color_at(205)], 1);
    request["mode"] = json!("depth");
    request["depth_range"] = json!([-5, -3]);
    let depth = pixels(&document.execute(request).unwrap());
    assert!((depth[(200 * 400 + 195) * 3] as i32 - 144).abs() <= 1);
}
