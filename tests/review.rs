mod common;

use gpt_mesh::session::Session;
use serde_json::{Value, json};

fn send(session: &mut Session, request: Value) -> Value {
    let reply = session.execute(request);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

#[test]
fn saved_reviews_freeze_camera_emit_evidence_and_survive_reopening() {
    let root = std::env::temp_dir().join(format!("mesh-review-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    send(&mut session, json!({"command":"workflow_status"}));
    let capture = send(
        &mut session,
        json!({"command":"capture","width":200,"height":200,"yaw":0,"pitch":0,"zoom":60}),
    );
    std::fs::copy(
        capture["result"]["artifacts"]["png"]["path"]
            .as_str()
            .unwrap(),
        root.join("reference.png"),
    )
    .unwrap();
    let registered = send(
        &mut session,
        json!({"command":"workflow_resource","kind":"reference","path":"reference.png","description":"Synthetic camera calibration","view":{"width":200,"height":200,"yaw":0,"pitch":0,"zoom":60}}),
    );
    let reference = registered["result"]["id"].clone();
    send(
        &mut session,
        json!({"command":"review_define","name":"cab-side","reference":reference,"view":{"width":200,"height":200,"yaw":0,"pitch":0,"zoom":60},"dimensions":[{"name":"panel","object":1}],"tessellation":true}),
    );
    let project_before = std::fs::read(root.join("project.json")).unwrap();
    let first = send(
        &mut session,
        json!({"command":"review_run","name":"cab-side"}),
    );
    assert_eq!(
        first["result"]["dimensions"][0]["size_xyz"],
        json!([2.0, 2.0, 2.0])
    );
    let mut images = vec![];
    for part in ["neutral", "wireframe", "silhouette_overlay", "tessellation"] {
        let path = first["result"]["artifacts"][part]["path"].as_str().unwrap();
        images.push((part, std::fs::read(path).unwrap()));
        assert!(!images.last().unwrap().1.is_empty());
    }
    let evidence = send(
        &mut session,
        json!({"command":"workflow_resource","kind":"capture","session":first["trace"]["session"],"event":first["trace"]["event"],"capture":"neutral","description":"Inspected saved review neutral view"}),
    );
    assert!(evidence["result"]["id"].is_string());
    let evidence_id = evidence["result"]["id"].clone();
    for requirement in ["intent", "reference", "views"] {
        send(&mut session, json!({"command":"workflow_waive","requirement":requirement,"reason":"Isolate the saved review variant test."}));
    }
    send(&mut session, json!({"command":"workflow_stage","stage":"primary"}));
    send(&mut session, json!({"command":"workflow_assess","requirement":"primary_forms","outcome":"needs_revision","assessment":"Check a shallower panel variant.","evidence":[evidence_id]}));
    let status = send(&mut session, json!({"command":"workflow_status"}));
    let baseline = &status["result"]["next_action"]["baseline_reviews"][0];
    assert_eq!(baseline["evidence"], evidence_id);
    assert_eq!(baseline["source"]["event"], first["trace"]["event"]);
    let repeat = baseline["command"].clone();
    assert_eq!(repeat, json!({"command":"review_run","name":"cab-side","preset_version":1}));
    assert_eq!(status["result"]["next_action"]["recheck_command"], repeat);
    assert_eq!(
        std::fs::read(root.join("project.json")).unwrap(),
        project_before
    );
    assert_eq!(
        session.execute(json!({"command":"review_run","name":"cab-side","yaw":45}))["ok"],
        false
    );
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let reopened = send(
        &mut session,
        json!({"command":"review_run","name":"cab-side"}),
    );
    assert_eq!(
        first["result"]["fixed_view"],
        reopened["result"]["fixed_view"]
    );
    for (part, bytes) in images {
        assert_eq!(
            std::fs::read(
                reopened["result"]["artifacts"][part]["path"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
            bytes
        );
    }
    send(
        &mut session,
        json!({"command":"review_update","name":"cab-side","view":{"width":200,"height":200,"yaw":20,"pitch":0,"zoom":60}}),
    );
    let updated = send(
        &mut session,
        json!({"command":"review_run","name":"cab-side"}),
    );
    assert_eq!(updated["result"]["preset_version"], 2);
    assert_ne!(
        updated["result"]["fixed_view"],
        first["result"]["fixed_view"]
    );
    assert_eq!(session.execute(repeat)["ok"], false);
    let status = send(&mut session, json!({"command":"workflow_status"}));
    let baseline = &status["result"]["next_action"]["baseline_reviews"][0];
    assert_eq!(baseline["current_preset_version"], 2);
    assert_eq!(baseline["command"], Value::Null);
    assert_eq!(status["result"]["next_action"]["recheck_command"], Value::Null);
    send(&mut session, json!({"command":"workflow_assess","requirement":"primary_forms","outcome":"accepted","assessment":"Accept the panel dimensions.","follow_up":"Finish the seam in detail.","evidence":[evidence_id]}));
    let detail = send(&mut session, json!({"command":"workflow_stage","stage":"detail"}));
    assert_eq!(detail["result"]["follow_ups"][0]["note"], "Finish the seam in detail.");
    std::fs::remove_dir_all(root).unwrap();
}
