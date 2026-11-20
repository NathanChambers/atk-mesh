use gpt_mesh::Document;
use serde_json::json;

#[test]
fn deleting_components_preserves_surviving_ids_pivots_membership_and_history() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_add_box","mode":"append","object":1,"min":[3,0,0],"max":[4,1,1]}))
        .unwrap();
    document
        .execute(json!({"command":"collection_create","name":"Assembly","objects":[1]}))
        .unwrap();
    document
        .execute(json!({"command":"shade_region","object":1,"faces":[7],"mode":"flat"}))
        .unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let deleted = document.execute(json!({"command":"object_delete_components","object":1,"vertices":[9,10],"expected_vertices":8})).unwrap();
    assert_eq!(deleted["operation"]["removed_vertices"], 8);
    assert_eq!(deleted["operation"]["removed_faces"], 6);
    assert_eq!(
        deleted["mesh"]["objects"]["1"]["origin"],
        before["mesh"]["objects"]["1"]["origin"]
    );
    assert_eq!(
        deleted["mesh"]["collections"],
        before["mesh"]["collections"]
    );
    assert_eq!(deleted["mesh"]["vertices"].as_object().unwrap().len(), 8);
    assert_eq!(deleted["mesh"]["faces"].as_object().unwrap().len(), 6);
    assert_eq!(deleted["mesh"]["selection"], json!([]));
    assert!(deleted["mesh"].get("flat_faces").is_none());
    for id in 1..=8 {
        assert_eq!(
            deleted["mesh"]["vertices"][id.to_string()],
            before["mesh"]["vertices"][id.to_string()]
        );
    }
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    let restored = document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(restored["mesh"], deleted["mesh"]);
    let added = document
        .execute(json!({"command":"object_add_box","mode":"append","object":1,"min":[3,0,0],"max":[4,1,1]}))
        .unwrap();
    assert_eq!(added["operation"]["vertices"][0], 17);
}

#[test]
fn component_deletion_guards_fail_atomically() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_create_box","name":"Other","min":[3,0,0],"max":[4,1,1]}))
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for request in [
        json!({"command":"object_delete_components","object":1,"vertices":[],"expected_vertices":0}),
        json!({"command":"object_delete_components","object":1,"vertices":[9],"expected_vertices":8}),
        json!({"command":"object_delete_components","object":1,"vertices":[1]}),
        json!({"command":"object_delete_components","object":1,"vertices":[1],"expected_vertices":1}),
        json!({"command":"object_delete_components","object":1,"vertices":[1],"expected_vertices":8}),
        json!({"command":"object_delete_components","object":1,"vertices":null,"expected_vertices":8}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
}
