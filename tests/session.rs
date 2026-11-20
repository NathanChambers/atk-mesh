mod common;

use gpt_mesh::session::Session;
use serde_json::json;

#[test]
fn pose_captures_render_samples_without_mutating_geometry_or_selection_views() {
    let root = std::env::temp_dir().join(format!("mesh-pose-capture-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root).unwrap();
    let weights: serde_json::Value = (1..=8).map(|id| {
        (id.to_string(), json!([{"bone":"root","weight":1}]))
    }).collect::<serde_json::Map<_, _>>().into();
    let mut clip = json!({"duration":2,"looping":true,"tracks":{"root":[
        {"time":0},
        {"time":1,"translation":[0.6,0,0],"rotation":[0,45,0]},
        {"time":2}
    ]}});
    assert_eq!(session.execute(json!({"command":"rig_set","rig":{
        "bones":[{"name":"root","position":[0,0,0]}],"weights":weights
    }}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"animation_set","name":"Turn","clip":clip}))["ok"], true);
    let initial = session.state();
    let capture = |session: &mut Session, time: f64| {
        session.execute(json!({"command":"capture","clip":"Turn","time":time,"width":180,"height":160,"zoom":100,"response":"full"}))
    };
    let start = capture(&mut session, 0.0);
    let sample = capture(&mut session, 0.5);
    let wrapped = capture(&mut session, 2.5);
    assert_eq!(sample["ok"], true);
    assert_eq!(wrapped["result"]["animation"]["sampled_time"], 0.5);
    assert!(sample["result"]["view_id"].is_null());
    assert_ne!(sample["result"]["png_base64"], start["result"]["png_base64"]);
    assert_eq!(sample["result"]["png_base64"], wrapped["result"]["png_base64"]);
    assert_eq!(session.state(), initial);
    assert_eq!(session.execute(json!({"command":"capture","width":180,"height":160}))["result"]["view_id"], 1);

    let artifact = session.execute(json!({"command":"capture","clip":"Turn","time":0.5,"width":180,"height":160}));
    assert!(artifact["result"]["artifacts"]["png"]["path"].is_string());
    let register = json!({"command":"workflow_resource","kind":"capture",
        "session":artifact["trace"]["session"],"event":artifact["trace"]["event"],"description":"Turn pose"});
    let registered = session.execute(register.clone());
    assert_eq!(registered["ok"], true, "{registered}");
    clip["looping"] = json!(false);
    assert_eq!(session.execute(json!({"command":"animation_set","name":"Turn","clip":clip}))["ok"], true);
    assert_eq!(session.execute(register)["error"], "capture animation is stale; capture again");
    assert_eq!(capture(&mut session, 2.5)["result"]["animation"]["sampled_time"], 2.0);
}

#[test]
fn animation_queries_page_exact_keys_and_track_edits_are_atomic() {
    let root = std::env::temp_dir().join(format!("mesh-animation-query-{}", std::process::id()));
    let mut session = Session::new(root).unwrap();
    assert_eq!(session.execute(json!({"command":"query_animation"}))["result"]["items"], json!([]));
    assert_eq!(session.execute(json!({"command":"rig_set","rig":{"bones":[
        {"name":"root","position":[0,0,0]},
        {"name":"arm","parent":"root","position":[1,0,0]}
    ]}}))["ok"], true);
    let keys: Vec<_> = (0..16).map(|i| json!({"time":i,"rotation":[0, i, 0]})).collect();
    assert_eq!(session.execute(json!({"command":"animation_set","name":"Turn","clip":{
        "duration":15,"looping":true,"tracks":{"root":keys}
    }}))["ok"], true);
    let clips = session.execute(json!({"command":"query_animation"}));
    assert_eq!(clips["result"]["items"][0]["duration"], 15.0);
    let revision = &clips["result"]["revision"];
    let tracks = session.execute(json!({"command":"query_animation","clip":"Turn","revision":revision}));
    assert_eq!(tracks["result"]["items"][0]["keyframes"], 16);
    let keys = session.execute(json!({"command":"query_animation","clip":"Turn","bone":"root","limit":12,"revision":revision}));
    assert_eq!(keys["result"]["items"].as_array().unwrap().len(), 12);
    assert_eq!(keys["result"]["next_offset"], 12);
    let tail = session.execute(json!({"command":"query_animation","clip":"Turn","bone":"root","offset":12,"revision":revision}));
    assert_eq!(tail["result"]["items"][3]["rotation"], json!([0.0,15.0,0.0]));
    assert!(tail["result"]["next_offset"].is_null());
    let before = session.state();
    assert_eq!(session.execute(json!({"command":"animation_update","name":"Turn","duration":14}))["ok"], false);
    assert_eq!(session.execute(json!({"command":"animation_track_remove","name":"Turn","bone":"root"}))["ok"], false);
    assert_eq!(session.state(), before);
    assert_eq!(session.execute(json!({"command":"animation_track_set","name":"Turn","bone":"arm","keys":[{"time":2}],"revision":revision}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"query_animation","revision":revision}))["error"], "stale document revision");
    assert_eq!(session.execute(json!({"command":"animation_track_remove","name":"Turn","bone":"root"}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"animation_update","name":"Turn","duration":3,"looping":false}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"animation_remove","name":"Turn"}))["ok"], true);
}

#[test]
fn recordings_preserve_full_results_failures_and_read_only_snapshots() {
    let root = std::env::temp_dir().join(format!("mesh-session-test-{}", std::process::id()));
    let mut session = Session::new(root.clone()).unwrap();
    let initial = session.state();
    let response = session.execute(json!({"command":"select","vertices":[1]}));
    assert_eq!(response["ok"], true);
    assert!(response["result"].get("mesh").is_none());
    assert_eq!(response["result"]["selection"], json!([1]));
    let selected = session.state();
    let failed = session.execute(json!({"command":"not_a_command"}));
    assert_eq!(failed["ok"], false);
    assert_eq!(session.state(), selected);
    session.execute(json!({"command":"undo"}));
    assert_eq!(session.state()["mesh"], initial["mesh"]);
    session.execute(json!({"command":"redo"}));
    assert_eq!(session.state()["mesh"], selected["mesh"]);

    let listing = session.read("/sessions").unwrap();
    let id = listing["current"].as_str().unwrap();
    let index = session.read(&format!("/sessions/{id}")).unwrap();
    let events = index["events"].as_array().unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[1]["snapshot"], 1);
    assert_eq!(events[1]["ok"], false);
    assert!(
        events[0]["response_bytes"].as_u64().unwrap()
            < events[0]["full_response_bytes"].as_u64().unwrap()
    );
    let detail = session.read(&format!("/sessions/{id}/events/1")).unwrap();
    assert_eq!(detail["input"]["command"], "select");
    assert_eq!(detail["full_output"]["result"]["mesh"], selected["mesh"]);
    assert_eq!(
        session.read(&format!("/sessions/{id}/states/0")).unwrap(),
        initial
    );
    assert_eq!(
        session.read(&format!("/sessions/{id}/states/1")).unwrap(),
        selected
    );
    assert!(session.read("/sessions/../states/0").is_err());
    let before = session.state();
    let second = Session::new(root.clone()).unwrap();
    assert_eq!(
        second.read(&format!("/sessions/{id}/events/1")).unwrap(),
        detail
    );
    assert_eq!(session.state(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn replies_default_to_bounded_and_full_queries_remain_available() {
    let root = std::env::temp_dir().join(format!("mesh-response-test-{}", std::process::id()));
    let mut session = Session::new(root.clone()).unwrap();
    for command in ["cube", "undo", "redo"] {
        let response = session.execute(json!({"command":command}));
        assert_eq!(response["ok"], true);
        assert!(response["result"].get("mesh").is_none());
        assert_eq!(response["result"]["counts"]["vertices"], 8);
    }
    let full = session.execute(json!({"command":"select","vertices":[1],"response":"full"}));
    assert!(full["result"]["mesh"]["vertices"].is_object());
    let inspect = session.execute(json!({"command":"inspect"}));
    assert!(inspect["result"].get("mesh").is_none());
    let inspect = session.execute(json!({"command":"inspect","response":"full"}));
    assert!(inspect["result"]["mesh"]["vertices"].is_object());
    let compact = session.execute(json!({"command":"inspect","response":"compact"}));
    assert!(compact["result"].get("mesh").is_none());
    let saved = session.execute(json!({"command":"save","response":"full"}));
    assert_eq!(saved["result"]["mesh"], session.state()["mesh"]);
    let saved = session.execute(json!({"command":"save"}));
    let path = saved["result"]["artifacts"]["project"]["path"]
        .as_str()
        .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["mesh"], session.state()["mesh"]);
    for requirement in [
        "intent",
        "reference",
        "primary_forms",
        "views",
        "export_validation",
    ] {
        session.execute(json!({"command":"workflow_waive","requirement":requirement,"reason":"Transport regression fixture; visual acceptance is outside this test."}));
    }
    let exported = session.execute(json!({"command":"export","format":"fbx"}));
    assert!(exported["result"].get("fbx_base64").is_none());
    let path = exported["result"]["artifacts"]["fbx"]["path"]
        .as_str()
        .unwrap();
    assert!(
        std::fs::read(path)
            .unwrap()
            .starts_with(b"Kaydara FBX Binary")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn model_workspace_reopens_mesh_and_its_recordings_without_external_session_files() {
    let root = std::env::temp_dir().join(format!("mesh-workspace-test-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let response = session
        .execute(json!({"command":"select","vertices":[1],"note":"Workspace persistence test"}));
    assert_eq!(response["ok"], true);
    let id = response["trace"]["session"].as_str().unwrap().to_string();
    session.execute(json!({"command":"move","value":[0,0.25,0]}));
    let mesh = session.state()["mesh"].clone();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("project.json")).unwrap()).unwrap();
    assert_eq!(saved["mesh"], mesh);
    let detail = session.read(&format!("/sessions/{id}/events/1")).unwrap();
    drop(session);

    let mut reopened = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(reopened.state()["mesh"], mesh);
    assert_eq!(reopened.state()["history"]["undo"], 0);
    let listing = reopened.read("/sessions").unwrap();
    assert_eq!(listing["preferred"], id);
    assert_eq!(
        reopened.read(&format!("/sessions/{id}/events/1")).unwrap(),
        detail
    );
    assert_eq!(detail["input"]["note"], "Workspace persistence test");
    let capture = reopened.execute(json!({"command":"export"}));
    let timeline = reopened.read("/sessions/workspace").unwrap();
    let events = timeline["events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(timeline["initial_session"], id);
    assert_eq!(events[0]["command"], "select");
    assert_eq!(events[1]["command"], "move");
    assert_eq!(events[2]["command"], "export");
    assert_eq!(events[2]["source_event"], 1);
    assert_eq!(events[2]["session"], capture["trace"]["session"]);
    assert_eq!(events[2]["id"], 3);
    assert!(
        root.join("sessions")
            .join(&id)
            .join("states/2.json")
            .is_file()
    );
    reopened.execute(json!({"command":"select","vertices":[]}));
    assert_eq!(
        reopened.read("/sessions").unwrap()["preferred"],
        listing["current"]
    );
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_workspace_project_is_not_overwritten() {
    let root = std::env::temp_dir().join(format!(
        "mesh-invalid-workspace-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = "{\"version\":999,\"mesh\":{}}";
    std::fs::write(root.join("project.json"), source).unwrap();
    assert!(Session::open_workspace(root.clone()).is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("project.json")).unwrap(),
        source
    );
    std::fs::remove_dir_all(root).unwrap();
}
