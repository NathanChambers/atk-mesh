mod common;

use gpt_mesh::{Document, base64, session::Session};
use serde_json::{Value, json};

fn scene() -> Document {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_create_box","name":"Door","min":[3,0,0],"max":[4,2,1]}))
        .unwrap();
    document.execute(json!({"command":"collection_create","name":"Architecture","objects":[1,2],"primary":true})).unwrap();
    document
        .execute(json!({"command":"collection_create","name":"Moving","objects":[2]}))
        .unwrap();
    document
}

#[test]
fn collections_scope_queries_edits_and_history() {
    let mut document = scene();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let list = document
        .execute(json!({"command":"query_mesh","kind":"collections","limit":1}))
        .unwrap();
    assert_eq!(list["total"], 2);
    assert_eq!(list["next_offset"], 1);
    assert!(list["items"][0].get("objects").is_none());
    let members = document
        .execute(json!({"command":"query_mesh","kind":"objects","collection":2}))
        .unwrap();
    assert_eq!(members["total"], 1);
    assert_eq!(members["items"][0]["id"], 2);
    let vertices = document
        .execute(json!({"command":"query_mesh","kind":"vertices","collection":2}))
        .unwrap();
    assert_eq!(vertices["total"], 8);
    let moved = document
        .execute(json!({"command":"object_move","collection":2,"value":[0,0,2]}))
        .unwrap();
    for id in 1..=8 {
        let key = id.to_string();
        assert_eq!(
            moved["mesh"]["vertices"][&key],
            before["mesh"]["vertices"][&key]
        );
    }
    assert_eq!(
        moved["mesh"]["objects"]["1"],
        before["mesh"]["objects"]["1"]
    );
    assert_eq!(moved["mesh"]["objects"]["2"]["origin"][2], 2.5);
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    document.execute(json!({"command":"redo"})).unwrap();
    document
        .execute(json!({"command":"select_collection","collection":2}))
        .unwrap();
    let selected = document
        .execute(json!({"command":"query_mesh","kind":"selection"}))
        .unwrap();
    assert_eq!(selected["total"], 8);
    let saved = document.execute(json!({"command":"save"})).unwrap();
    let mut restored = Document::default();
    restored
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    assert_eq!(restored.execute(json!({"command":"save"})).unwrap(), saved);
}

#[test]
fn invalid_collections_and_unsupported_targets_are_atomic() {
    let mut document = scene();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for request in [
        json!({"command":"collection_create","name":"Architecture"}),
        json!({"command":"collection_update","collection":2,"objects":[999]}),
        json!({"command":"collection_update","collection":2,"primary":"yes"}),
        json!({"command":"collection_delete","collection":99}),
        json!({"command":"collection_update","collection":2,"remove_objects":[999]}),
        json!({"command":"collection_update","collection":2,"objects":[1],"add_objects":[2]}),
        json!({"command":"object_move","collection":2,"object":1,"value":[1,0,0]}),
        json!({"command":"move","collection":2,"value":[1,0,0]}),
        json!({"command":"object_move","collection":1,"value":[1,0]}),
        json!({"command":"capture","solo_collections":[99]}),
        json!({"command":"capture","hide_collections":null}),
    ] {
        assert!(document.execute(request.clone()).is_err(), "{request}");
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    let mut invalid = document.execute(json!({"command":"save"})).unwrap();
    invalid["mesh"]["primary_collections"]["1"] = json!(2);
    assert!(
        document
            .execute(json!({"command":"load","project":invalid}))
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
}

#[test]
fn duplication_deletion_and_membership_updates_keep_primary_valid() {
    let mut document = scene();
    let removed = document.execute(json!({"command":"collection_update","collection":2,"add_objects":[1],"remove_objects":[2]})).unwrap();
    assert_eq!(removed["mesh"]["collections"]["2"]["objects"], json!([1]));
    document.execute(json!({"command":"undo"})).unwrap();
    let copied = document
        .execute(json!({"command":"object_duplicate","object":2,"name":"Copy"}))
        .unwrap();
    assert_eq!(copied["mesh"]["collections"]["2"]["objects"], json!([2, 3]));
    assert_eq!(copied["mesh"]["primary_collections"]["3"], 1);
    document
        .execute(json!({"command":"object_delete","object":3}))
        .unwrap();
    document
        .execute(json!({"command":"collection_update","collection":1,"objects":[1]}))
        .unwrap();
    let saved = document.execute(json!({"command":"save"})).unwrap();
    assert!(saved["mesh"]["primary_collections"].get("2").is_none());
    document
        .execute(json!({"command":"collection_delete","collection":2}))
        .unwrap();
    let created = document
        .execute(json!({"command":"collection_create","name":"New"}))
        .unwrap();
    assert_eq!(created["operation"]["collection"], 3);
}

fn pixels(result: &Value) -> Vec<u8> {
    let bytes = base64::decode(result["png_base64"].as_str().unwrap()).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut pixels).unwrap();
    pixels
}

#[test]
fn collection_filters_match_object_filters_and_keep_colors_and_camera() {
    let mut document = scene();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let capture = |document: &mut Document, filter: Value| {
        let mut request = json!({"command":"capture","mode":"collection_ids","width":128,"height":128,"yaw":0,"pitch":0});
        for (key, value) in filter.as_object().unwrap() {
            request[key] = value.clone();
        }
        document.execute(request).unwrap()
    };
    let all = capture(&mut document, json!({}));
    let solo = capture(&mut document, json!({"solo_collections":[2]}));
    let objects = capture(&mut document, json!({"solo_objects":[2]}));
    assert_eq!(pixels(&solo), pixels(&objects));
    assert_eq!(solo["camera"], all["camera"]);
    assert_eq!(solo["collection_id_colors"], all["collection_id_colors"]);
    assert_eq!(solo["visibility"]["visible_object_count"], 1);
    let empty = capture(
        &mut document,
        json!({"solo_collections":[1],"hide_collections":[1]}),
    );
    assert!(pixels(&empty).iter().all(|value| *value == 0));
    let intersection = capture(
        &mut document,
        json!({"solo_collections":[2],"solo_objects":[1]}),
    );
    assert_eq!(intersection["visibility"]["visible_object_count"], 0);
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    let selected = document.execute(json!({"command":"select_image","view_id":solo["view_id"],"rect":[0,0,128,128],"through":true})).unwrap();
    assert!(
        selected["mesh"]["selection"]
            .as_array()
            .unwrap()
            .iter()
            .all(|id| id.as_u64().unwrap() > 8)
    );
}

#[test]
fn workspace_replay_and_restart_preserve_membership() {
    let root = std::env::temp_dir().join(format!("mesh-collections-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let request = json!({"command":"collection_create","name":"Shell","objects":[1]});
    let result = session.execute(request.clone());
    assert_eq!(result["ok"], true);
    let trace = &result["trace"];
    let recorded = session
        .read(&format!(
            "/sessions/{}/events/{}",
            trace["session"].as_str().unwrap(),
            trace["event"]
        ))
        .unwrap();
    assert_eq!(recorded["input"], request);
    let state = session.state();
    drop(session);
    let restored = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(restored.state()["mesh"], state["mesh"]);
    drop(restored);
    std::fs::remove_dir_all(root).unwrap();
}
