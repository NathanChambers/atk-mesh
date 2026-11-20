use gpt_mesh::Document;
use serde_json::{Value, json};

#[test]
fn swept_sections_and_elliptic_helices_are_closed_and_undoable() {
    for path in [
        json!([[0, 0, 0], [0, 1, 0], [0.2, 2, 0]]),
        json!({"helix":{"radius":[1,0.8],"height":2,"turns":3,"steps":144}}),
    ] {
        let mut document = Document::default();
        let before = document.execute(json!({"command":"save"})).unwrap();
        let result = document.execute(json!({"command":"sweep","name":"Sweep","profile":[[-0.02,-0.02],[0.02,-0.02],[0.02,0.02],[-0.02,0.02]],"path":path})).unwrap();
        assert_eq!(result["validation"]["boundary_edges"], 0);
        assert_eq!(result["validation"]["non_manifold_edges"], 0);
        document.execute(json!({"command":"undo"})).unwrap();
        assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
        document.execute(json!({"command":"redo"})).unwrap();
        assert_eq!(
            document.execute(json!({"command":"save"})).unwrap()["mesh"],
            result["mesh"]
        );
    }
}

#[test]
fn bad_sweeps_are_atomic() {
    let mut document = Document::default();
    for path in [
        json!([[0, 0, 0], [0, 0, 0]]),
        json!([[0, 0, 0], [1, 0, 0]]),
        json!({"helix":{"radius":[0,1],"height":2,"turns":2,"steps":48}}),
    ] {
        let before: Value = document.execute(json!({"command":"save"})).unwrap();
        assert!(document.execute(json!({"command":"sweep","name":"Invalid","profile":[[0,0],[1,0],[0,1]],"path":path})).is_err());
        assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    }
}
