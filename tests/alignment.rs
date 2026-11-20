mod common;

use gpt_mesh::{Document, session::Session};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture(name: &str) -> (PathBuf, Session) {
    let root = std::env::temp_dir().join(format!("mesh-alignment-{name}-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let capture = session.execute(
        json!({"command":"capture","yaw":0,"pitch":0,"zoom":160,"width":200,"height":200}),
    );
    let path = capture["result"]["artifacts"]["png"]["path"]
        .as_str()
        .unwrap();
    std::fs::copy(path, root.join("reference.png")).unwrap();
    let registered = session.execute(json!({"command":"workflow_resource","kind":"reference","path":"reference.png","description":"Synthetic calibration fixture","view":{"yaw":0,"pitch":0,"zoom":160,"width":200,"height":200}}));
    assert_eq!(registered["ok"], true);
    (root, session)
}

fn points() -> Value {
    json!([{"name":"a","pixel":[55,155],"vertices":[1]}, {"name":"b","pixel":[155,155],"vertices":[2]}, {"name":"c","pixel":[155,55],"vertices":[3]}])
}

fn register(session: &mut Session) {
    let result = session.execute(json!({"command":"reference_landmarks","reference":"resource-1","landmarks":points(),"contours":[{"landmarks":["a","b","c"],"closed":true}]}));
    assert_eq!(result["ok"], true, "{result}");
}

fn report(session: &mut Session) -> Value {
    let result = session.execute(json!({"command":"project_reference","reference":"resource-1"}));
    assert_eq!(result["ok"], true, "{result}");
    result["result"].clone()
}

#[test]
fn comparison_visibility_is_temporary_and_omits_hidden_landmarks_and_crossing_contours() {
    let (root, mut session) = fixture("capture-visibility");
    let object = session.execute(
        json!({"command":"object_create_box","name":"Cover","min":[-2,-2,2],"max":[2,2,3]}),
    );
    assert_eq!(object["ok"], true);
    let registered = session.execute(json!({"command":"reference_landmarks","reference":"resource-1","landmarks":[{"name":"a","pixel":[55,155],"vertices":[1]},{"name":"b","pixel":[155,155],"vertices":[2]},{"name":"cover","pixel":[100,100],"object":2},{"name":"guide","pixel":[80,80]}],"contours":[{"landmarks":["a","b"],"closed":false},{"landmarks":["a","cover"],"closed":false}]}));
    assert_eq!(registered["ok"], true);
    assert_eq!(
        session.execute(json!({"command":"collection_create","name":"Visible","objects":[1]}))["ok"],
        true
    );
    let before = session.state();
    for layout in ["side_by_side", "overlay"] {
        let compared = session.execute(json!({"command":"compare_reference","reference":"resource-1","solo_collections":[1],"layout":layout,"response":"full"}));
        assert_eq!(compared["ok"], true, "{compared}");
        assert_eq!(compared["result"]["visibility"]["solo_objects"], json!([1]));
        assert_eq!(compared["result"]["visibility"]["visible_object_count"], 1);
        assert_eq!(
            compared["result"]["alignment"]["omitted_hidden_landmarks"],
            1
        );
        assert_eq!(
            compared["result"]["alignment"]["landmarks"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            compared["result"]["alignment"]["contours"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(session.state(), before);
    }
    let complete = report(&mut session);
    assert_eq!(complete["landmarks"].as_array().unwrap().len(), 4);
    let all = session
        .execute(json!({"command":"compare_reference","reference":"resource-1","response":"full"}));
    assert_eq!(all["result"]["visibility"]["visible_object_count"], 2);
    assert_eq!(all["result"]["alignment"]["omitted_hidden_landmarks"], 0);
    assert_eq!(
        session.execute(
            json!({"command":"compare_reference","reference":"resource-1","hide_objects":[999]})
        )["ok"],
        false
    );
    assert_eq!(session.state(), before);
    drop(session);
    let mut reopened = Session::open_workspace(root.clone()).unwrap();
    let capture = reopened.execute(json!({"command":"capture"}));
    assert_eq!(capture["result"]["visibility"]["visible_object_count"], 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn fitting_changes_camera_only_and_landmarks_survive_restart() {
    let (root, mut session) = fixture("fit");
    register(&mut session);
    let before = session.state();
    let original = report(&mut session);
    assert!(original["rms_error_pixels"].as_f64().unwrap() > 5.0);
    let fit = session
        .execute(json!({"command":"fit_reference","reference":"resource-1","names":["a","b","c"]}));
    assert_eq!(fit["ok"], true, "{fit}");
    assert_eq!(fit["result"]["camera_only"], true);
    assert!(fit["result"]["after_rms_error_pixels"].as_f64().unwrap() < 1e-10);
    assert_eq!(session.state(), before);
    let fitted = report(&mut session);
    assert!((fitted["camera"]["zoom"].as_f64().unwrap() - 200.0).abs() < 1e-10);
    let geometry = session.state()["mesh"].clone();
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let reopened = report(&mut session);
    assert!(reopened["rms_error_pixels"].as_f64().unwrap() < 1e-10);
    assert_eq!(reopened["alignment_version"], fitted["alignment_version"]);
    assert_eq!(session.state()["mesh"], geometry);
    let page = session.execute(
        json!({"command":"project_reference","reference":"resource-1","limit":1,"offset":1}),
    );
    assert_eq!(page["result"]["landmarks"][0]["name"], "b");
    assert_eq!(page["result"]["next_offset"], 2);
    let filtered = session
        .execute(json!({"command":"project_reference","reference":"resource-1","names":["c"]}));
    assert_eq!(filtered["result"]["total"], 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn projection_uses_source_pixels_across_resolutions_crops_and_letterboxing() {
    let (root, mut session) = fixture("coordinates");
    register(&mut session);
    let before = report(&mut session);
    session.execute(
        json!({"command":"reference_view","reference":"resource-1","width":400,"height":400}),
    );
    let doubled = report(&mut session);
    for (a, b) in before["landmarks"]
        .as_array()
        .unwrap()
        .iter()
        .zip(doubled["landmarks"].as_array().unwrap())
    {
        assert_eq!(a["projected_pixel"], b["projected_pixel"]);
        assert_eq!(a["error_pixels"], b["error_pixels"]);
    }
    session.execute(json!({"command":"reference_view","reference":"resource-1","width":400,"height":200,"crop":[20,40,180,160]}));
    let cropped = report(&mut session);
    let pixel = &cropped["landmarks"][0]["projected_pixel"];
    assert!((pixel[0].as_f64().unwrap() - 68.4).abs() < 1e-10);
    assert!((pixel[1].as_f64().unwrap() - 132.0).abs() < 1e-10);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reference_moves_are_local_depth_preserving_atomic_and_undoable() {
    let (root, mut session) = fixture("moves");
    let other = session
        .execute(json!({"command":"object_create_box","name":"Other","min":[3,3,3],"max":[4,4,4]}));
    assert_eq!(other["ok"], true);
    session
        .execute(json!({"command":"reference_view","reference":"resource-1","yaw":37,"pitch":21}));
    let registered = session.execute(json!({"command":"reference_landmarks","reference":"resource-1","landmarks":[{"name":"center","pixel":[120,84],"object":1}]}));
    assert_eq!(registered["ok"], true);
    let projected = report(&mut session);
    let request = json!({"command":"move_in_reference","reference":"resource-1","landmark":"center","revision":projected["revision"],"alignment_version":projected["alignment_version"]});
    let before = session.state();
    let mut wrong_scope = request.clone();
    wrong_scope["object"] = json!(2);
    assert_eq!(session.execute(wrong_scope)["ok"], false);
    assert_eq!(session.state(), before);
    let result = session.execute(request.clone());
    assert_eq!(result["ok"], true, "{result}");
    let after = session.state();
    let displacement = &result["result"]["operation"]["displacement"];
    let yaw = 37.0f64.to_radians();
    let pitch = 21.0f64.to_radians();
    let normal = [
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    ];
    let depth: f64 = (0..3)
        .map(|axis| displacement[axis].as_f64().unwrap() * normal[axis])
        .sum();
    assert!(depth.abs() < 1e-12);
    for id in after["mesh"]["objects"]["2"]["vertices"]
        .as_array()
        .unwrap()
    {
        assert_eq!(
            after["mesh"]["vertices"][id.to_string()],
            before["mesh"]["vertices"][id.to_string()]
        );
    }
    assert_eq!(after["mesh"]["objects"], before["mesh"]["objects"]);
    assert_eq!(after["mesh"]["faces"], before["mesh"]["faces"]);
    assert!(report(&mut session)["rms_error_pixels"].as_f64().unwrap() < 1e-10);
    assert_eq!(session.execute(request)["ok"], false);
    assert_eq!(session.state(), after);
    session.execute(json!({"command":"undo"}));
    assert_eq!(session.state()["mesh"], before["mesh"]);
    let current = report(&mut session);
    session.execute(json!({"command":"reference_view","reference":"resource-1","zoom":170}));
    let stale_alignment = session.execute(json!({"command":"move_in_reference","reference":"resource-1","landmark":"center","revision":current["revision"],"alignment_version":current["alignment_version"]}));
    assert_eq!(stale_alignment["ok"], false);
    assert_eq!(session.state()["mesh"], before["mesh"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bad_landmarks_degenerate_fits_and_changed_sources_do_not_mutate_state() {
    let (root, mut session) = fixture("invalid");
    register(&mut session);
    let original = std::fs::read(root.join("workflow.json")).unwrap();
    let before = session.state();
    for landmarks in [
        json!([{"name":"a","pixel":[3,3],"vertices":[999]}]),
        json!([{"name":"a","pixel":[200,3],"vertices":[1]}]),
        json!([{"name":"a","pixel":[3,3]},{"name":"a","pixel":[4,4]}]),
    ] {
        assert_eq!(session.execute(json!({"command":"reference_landmarks","reference":"resource-1","landmarks":landmarks}))["ok"],false);
        assert_eq!(std::fs::read(root.join("workflow.json")).unwrap(), original);
        assert_eq!(session.state(), before);
    }
    assert_eq!(
        session.execute(json!({"command":"reference_view","reference":"resource-1","zoom":0}))["ok"],
        false
    );
    assert_eq!(
        session.execute(
            json!({"command":"fit_reference","reference":"resource-1","names":["missing"]})
        )["ok"],
        false
    );
    session.execute(json!({"command":"reference_landmarks","reference":"resource-1","landmarks":[{"name":"a","pixel":[55,155],"vertices":[1]},{"name":"b","pixel":[155,155],"vertices":[1]}]}));
    let degenerate = std::fs::read(root.join("workflow.json")).unwrap();
    assert_eq!(
        session.execute(json!({"command":"fit_reference","reference":"resource-1"}))["ok"],
        false
    );
    assert_eq!(
        std::fs::read(root.join("workflow.json")).unwrap(),
        degenerate
    );
    std::fs::write(root.join("reference.png"), b"changed").unwrap();
    assert_eq!(
        session.execute(json!({"command":"project_reference","reference":"resource-1"}))["ok"],
        false
    );
    assert_eq!(
        session.execute(
            json!({"command":"compare_reference","reference":"resource-1","layout":"overlay"})
        )["ok"],
        false
    );
    assert_eq!(session.state(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlays_are_read_only_png_artifacts_with_reference_and_mesh_annotations() {
    let (root, mut session) = fixture("overlay");
    register(&mut session);
    let before = session.state();
    let response = session.execute(
        json!({"command":"compare_reference","reference":"resource-1","layout":"overlay"}),
    );
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["result"]["width"], 200);
    assert_eq!(response["result"]["height"], 200);
    assert_eq!(session.state(), before);
    let path = response["result"]["artifacts"]["png"]["path"]
        .as_str()
        .unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(std::fs::read(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    for color in [[240, 30, 180], [0, 190, 255], [255, 200, 0]] {
        assert!(
            pixels[..info.buffer_size()]
                .chunks_exact(3)
                .any(|pixel| pixel == color)
        );
    }
    let session_id = response["trace"]["session"].as_str().unwrap();
    let event = response["trace"]["event"].as_u64().unwrap();
    let record = session
        .read(&format!("/sessions/{session_id}/events/{event}"))
        .unwrap();
    assert_eq!(record["input"]["layout"], "overlay");
    assert!(record["full_output"]["result"]["png_base64"].is_string());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_view_moves_reject_stale_captures_and_preserve_depth() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"select_object","object":1}))
        .unwrap();
    let view = document
        .execute(json!({"command":"capture","yaw":65,"pitch":-20,"width":100,"height":100}))
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let moved = document.execute(json!({"command":"move_in_view","view_id":view["view_id"],"delta_pixels":[3,-7],"object":1})).unwrap();
    assert_eq!(moved["mesh"]["faces"], before["mesh"]["faces"]);
    assert_eq!(moved["operation"]["depth_preserved"], true);
    assert!(
        document
            .execute(
                json!({"command":"move_in_view","view_id":view["view_id"],"delta_pixels":[1,1]})
            )
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
}

#[test]
fn deleted_bindings_fail_explicitly_and_can_be_rebound_without_geometry_edits() {
    let (root, mut session) = fixture("rebind");
    register(&mut session);
    session.execute(
        json!({"command":"object_create_box","name":"Replacement","min":[3,3,3],"max":[4,4,4]}),
    );
    session.execute(json!({"command":"object_delete","object":1}));
    let before = session.state();
    let failed = session.execute(json!({"command":"project_reference","reference":"resource-1"}));
    assert_eq!(failed["ok"], false);
    assert!(failed["error"].as_str().unwrap().contains("removed vertex"));
    let rebound = session.execute(json!({"command":"reference_landmarks","reference":"resource-1","landmarks":[{"name":"replacement","pixel":[100,100],"object":2}]}));
    assert_eq!(rebound["ok"], true);
    let projected = report(&mut session);
    assert_eq!(projected["total"], 1);
    assert_eq!(session.state(), before);
    let stored = std::fs::read(root.join("workflow.json")).unwrap();
    let query = session
        .execute(json!({"command":"reference_landmarks","reference":"resource-1","limit":1}));
    assert_eq!(query["ok"], true);
    assert_eq!(query["result"]["items"][0]["object"], 2);
    assert_eq!(std::fs::read(root.join("workflow.json")).unwrap(), stored);
    let stale_page = session.execute(
        json!({"command":"project_reference","reference":"resource-1","alignment_version":0}),
    );
    assert_eq!(stale_page["ok"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn dense_overlay_annotations_keep_default_artifact_receipts_and_camera_intact() {
    let (root, mut session) = fixture("dense");
    let points: Vec<_> = (0..64)
        .map(|i| json!({"name":format!("point-{i}"),"pixel":[40+i,80],"vertices":[1]}))
        .collect();
    let registered = session.execute(
        json!({"command":"reference_landmarks","reference":"resource-1","landmarks":points}),
    );
    assert_eq!(registered["ok"], true);
    let capture = session.execute(
        json!({"command":"compare_reference","reference":"resource-1","layout":"overlay"}),
    );
    assert_eq!(capture["ok"], true, "{capture}");
    assert!(capture["result"]["artifacts"]["png"]["path"].is_string());
    assert_eq!(capture["result"]["width"], 200);
    assert_eq!(capture["result"]["camera"]["zoom"], 160.0);
    assert_eq!(capture["result"]["alignment"]["landmark_count"], 64);
    let session_id = capture["trace"]["session"].as_str().unwrap();
    let event = capture["trace"]["event"].as_u64().unwrap();
    let record = session
        .read(&format!("/sessions/{session_id}/events/{event}"))
        .unwrap();
    assert_eq!(
        record["full_output"]["result"]["alignment"]["landmarks"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    std::fs::remove_dir_all(root).unwrap();
}
