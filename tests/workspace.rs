use gpt_mesh::Document;
use serde_json::json;

#[test]
fn editing_history_and_persistence() {
    let mut document = Document::default();
    let original = document.execute(json!({"command":"inspect"})).unwrap();
    assert_eq!(original["mesh"]["vertices"].as_object().unwrap().len(), 8);
    document
        .execute(json!({"command":"select_box","min":[-2,0,-2],"max":[2,2,2]}))
        .unwrap();
    let moved = document
        .execute(json!({"command":"move","value":[0,0.5,0]}))
        .unwrap();
    assert_eq!(moved["mesh"]["vertices"]["3"][1], 1.5);
    let extruded = document
        .execute(json!({"command":"extrude","face":4,"value":[0,1,0]}))
        .unwrap();
    assert_eq!(extruded["mesh"]["vertices"].as_object().unwrap().len(), 12);
    assert_eq!(extruded["validation"]["boundary_edges"], 0);
    assert_eq!(extruded["validation"]["non_manifold_edges"], 0);
    let project = document.execute(json!({"command":"save"})).unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    let restored = document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(restored["mesh"], extruded["mesh"]);
    document.execute(json!({"command":"cube"})).unwrap();
    let loaded = document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    assert_eq!(loaded["mesh"], extruded["mesh"]);
    let exported = document.execute(json!({"command":"export"})).unwrap();
    assert_eq!(
        exported["obj"]
            .as_str()
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("v "))
            .count(),
        12
    );
}

#[test]
fn image_selection_obeys_occlusion_and_revisions() {
    let mut document = Document::default();
    let view = document
        .execute(json!({"command":"capture","yaw":0,"pitch":0}))
        .unwrap();
    let selected = document
        .execute(json!({"command":"select_image","view_id":view["view_id"],"rect":[0,0,800,600]}))
        .unwrap();
    assert_eq!(selected["mesh"]["selection"], json!([5, 6, 7, 8]));
    assert!(
        document
            .execute(
                json!({"command":"select_image","view_id":view["view_id"],"rect":[0,0,800,600]})
            )
            .is_err()
    );
    assert!(
        document
            .execute(json!({"command":"move","revision":0,"value":[0,1,0]}))
            .is_err()
    );
    let view = document.execute(json!({"command":"capture"})).unwrap();
    let selected = document.execute(json!({"command":"select_image","view_id":view["view_id"],"rect":[0,0,800,600],"through":true})).unwrap();
    assert_eq!(selected["mesh"]["selection"].as_array().unwrap().len(), 8);
}

#[test]
fn failed_edits_are_atomic() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"select","vertices":[1,2,3,4,5,6,7,8]}))
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    assert!(
        document
            .execute(json!({"command":"scale","value":[0,0,0]}))
            .is_err()
    );
    assert_eq!(
        before,
        document.execute(json!({"command":"inspect"})).unwrap()
    );
    assert!(
        document
            .execute(json!({"command":"extrude","face":4,"value":[0,0,0]}))
            .is_err()
    );
    assert_eq!(
        before,
        document.execute(json!({"command":"inspect"})).unwrap()
    );
    assert!(
        document
            .execute(json!({"command":"select","vertices":[999]}))
            .is_err()
    );
}
