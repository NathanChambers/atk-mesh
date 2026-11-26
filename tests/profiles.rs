use atk_mesh::Document;
use serde_json::{Value, json};

fn profile() -> Value {
    json!({"command":"object_profile","source_object":1,"vertices":[5,8,7,6],"name":"Frame","offset":0.1,"width":0.2,"depth":0.1,"z":2.0,"origin":[-1,0,2]})
}

#[test]
fn copies_real_edges_into_a_closed_offset_solid_without_changing_source() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let query = document
        .execute(json!({"command":"inspect_profile","source_object":1,"vertices":[5,8,7,6]}))
        .unwrap();
    assert_eq!(query["length"], 6.0);
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    let result = document.execute(profile()).unwrap();
    assert_eq!(result["operation"]["object"], 2);
    assert_eq!(
        result["operation"]["outer_profile"],
        json!([[-0.9, -1.0], [-0.9, 0.9], [0.9, 0.9], [0.9, -1.0]])
    );
    assert_eq!(result["operation"]["sections"].as_array().unwrap().len(), 4);
    assert_eq!(result["validation"]["boundary_edges"], 0);
    assert_eq!(result["validation"]["non_manifold_edges"], 0);
    for (id, point) in before["mesh"]["vertices"].as_object().unwrap() {
        assert_eq!(&result["mesh"]["vertices"][id], point);
    }
    for (id, face) in before["mesh"]["faces"].as_object().unwrap() {
        assert_eq!(&result["mesh"]["faces"][id], face);
    }
    document.execute(json!({"command":"undo"})).unwrap();
    let restored = document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(restored["mesh"], result["mesh"]);
    let faces = result["operation"]["faces"].as_array().unwrap();
    document
        .execute(json!({"command":"inset","object":2,"face":faces[0],"distance":0.01}))
        .unwrap();
}

#[test]
fn bad_chains_and_collapsing_offsets_are_atomic() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for (field, value) in [
        ("vertices", json!([5, 7])),
        ("vertices", json!([1, 5])),
        ("vertices", json!([5, 8, 5])),
        ("vertices", json!([5, 99])),
        ("offset", json!(1.1)),
        ("width", json!(-1)),
        ("depth", json!(0)),
    ] {
        let mut request = profile();
        request[field] = value;
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
}

#[test]
fn appending_parts_preserves_origin_and_deleting_object_preserves_other_geometry() {
    let mut document = Document::default();
    let created = document.execute(profile()).unwrap();
    let appended = document.execute(json!({"command":"object_add_box","mode":"append","object":2,"min":[-0.9,-1.2,1.95],"max":[0.9,-1.0,2.05]})).unwrap();
    assert_eq!(
        appended["mesh"]["objects"]["2"]["origin"],
        created["mesh"]["objects"]["2"]["origin"]
    );
    assert_eq!(
        appended["mesh"]["objects"]["2"]["vertices"]
            .as_array()
            .unwrap()
            .len(),
        24
    );
    let removed = document
        .execute(json!({"command":"object_delete","object":2}))
        .unwrap();
    assert_eq!(removed["mesh"]["vertices"].as_object().unwrap().len(), 8);
    let fresh = document.execute(profile()).unwrap();
    assert_eq!(fresh["operation"]["object"], 3);
}
