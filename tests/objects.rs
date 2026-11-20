use gpt_mesh::Document;
use serde_json::{Value, json};

fn create_gate(document: &mut Document) -> Value {
    document.execute(json!({"command":"object_create_box","name":"Gate","min":[2,0,-0.1],"max":[3,2,0.1],"origin":[2,0,0]})).unwrap()
}

#[test]
fn stage_labels_round_trip_and_legacy_approval_requires_review() {
    let mut document = Document::default();
    create_gate(&mut document);
    let query = json!({"command":"query_mesh","kind":"objects"});
    let objects = document.execute(query.clone()).unwrap();
    assert_eq!(objects["items"][1]["stage"], "blockout");
    assert!(document.execute(json!({
        "command":"object_set_stage","object":2,"stage":"production_ready","note":"Review"
    })).is_err());
    assert!(document.execute(json!({"command":"object_set_stage","object":2,"stage":"refined"})).is_err());
    document.execute(json!({"command":"object_set_stage","object":2,"stage":"detailed"})).unwrap();
    let mut saved = document.execute(json!({"command":"save"})).unwrap();
    saved["mesh"]["objects"]["1"]["stage"] = json!("production_ready");
    saved["mesh"]["objects"]["1"]["stage_note"] = json!("Old approval");
    document
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    let objects = document.execute(query).unwrap();
    assert_eq!(objects["items"][0]["stage"], "detailed");
    assert!(objects["items"][0]["stage_note"].is_null());
    assert_eq!(objects["items"][1]["stage"], "detailed");
}

#[test]
fn duplicate_preserves_shading_and_pivot_without_sharing_edits() {
    let mut document = Document::default();
    create_gate(&mut document);
    document
        .execute(json!({"command":"shade_smooth","object":2,"angle":45}))
        .unwrap();
    let source = document
        .execute(json!({"command":"shade_region","object":2,"faces":[7],"mode":"flat"}))
        .unwrap();
    let copied = document
        .execute(
            json!({"command":"object_duplicate","object":2,"name":"Gate Copy","value":[0,0,4]}),
        )
        .unwrap();
    assert_eq!(copied["operation"]["object"], 3);
    assert_eq!(
        copied["mesh"]["objects"]["3"]["origin"],
        json!([2.0, 0.0, 4.0])
    );
    assert_eq!(copied["mesh"]["objects"]["3"]["smooth_angle"], 45.0);
    for (original, created) in copied["operation"]["vertex_remap"].as_object().unwrap() {
        let key = created.as_u64().unwrap().to_string();
        let point = &source["mesh"]["vertices"][original];
        assert_eq!(
            copied["mesh"]["vertices"][&key],
            json!([point[0], point[1], point[2].as_f64().unwrap() + 4.0])
        );
    }
    let flat = &copied["operation"]["face_remap"]["7"];
    assert!(
        copied["mesh"]["flat_faces"]
            .as_array()
            .unwrap()
            .contains(flat)
    );
    document
        .execute(json!({"command":"object_move","object":3,"value":[3,0,0]}))
        .unwrap();
    let edited = document.execute(json!({"command":"inspect"})).unwrap();
    for vertex in source["mesh"]["objects"]["2"]["vertices"]
        .as_array()
        .unwrap()
    {
        let key = vertex.as_u64().unwrap().to_string();
        assert_eq!(
            edited["mesh"]["vertices"][&key],
            source["mesh"]["vertices"][&key]
        );
    }
    document.execute(json!({"command":"undo"})).unwrap();
    let restored = document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(restored["mesh"], source["mesh"]);
    let redone = document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(redone["mesh"], copied["mesh"]);
}

#[test]
fn duplicate_rejects_invalid_destinations_atomically() {
    let mut document = Document::default();
    create_gate(&mut document);
    let saved = document.execute(json!({"command":"save"})).unwrap();
    for request in [
        json!({"command":"object_duplicate","object":2,"name":"Gate"}),
        json!({"command":"object_duplicate","object":99,"name":"Unknown"}),
        json!({"command":"object_duplicate","object":2,"name":"Copy","value":[0,0]}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(document.execute(json!({"command":"save"})).unwrap(), saved);
    }
}

#[test]
fn origin_changes_preserve_geometry_and_gate_rotation_preserves_frame() {
    let mut document = Document::default();
    let frame = document.execute(json!({"command":"inspect"})).unwrap();
    let created = create_gate(&mut document);
    assert_eq!(created["operation"]["object"], 2);
    let shifted = document
        .execute(json!({"command":"object_set_origin","object":2,"origin":[2,1,0]}))
        .unwrap();
    assert_eq!(created["mesh"]["vertices"], shifted["mesh"]["vertices"]);
    let open = document
        .execute(json!({"command":"object_rotate","object":2,"value":[0,90,0]}))
        .unwrap();
    for (id, point) in frame["mesh"]["vertices"].as_object().unwrap() {
        assert_eq!(&open["mesh"]["vertices"][id], point);
    }
    assert_eq!(
        open["mesh"]["objects"]["2"]["origin"],
        json!([2.0, 1.0, 0.0])
    );
    let point = &open["mesh"]["vertices"]["10"];
    assert!((point[0].as_f64().unwrap() - 1.9).abs() < 1e-12);
    assert!((point[2].as_f64().unwrap() + 1.0).abs() < 1e-12);
    let closed = document
        .execute(json!({"command":"object_rotate","object":2,"value":[0,-90,0]}))
        .unwrap();
    for (id, point) in created["mesh"]["vertices"].as_object().unwrap() {
        for axis in 0..3 {
            assert!(
                (closed["mesh"]["vertices"][id][axis].as_f64().unwrap()
                    - point[axis].as_f64().unwrap())
                .abs()
                    < 1e-12
            );
        }
    }
    let saved = document.execute(json!({"command":"save"})).unwrap();
    document.execute(json!({"command":"cube"})).unwrap();
    document
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    let restored = document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(restored["mesh"], closed["mesh"]);
}

#[test]
fn scoped_edits_and_cross_object_topology_are_atomic() {
    let mut document = Document::default();
    create_gate(&mut document);
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for request in [
        json!({"command":"move","object":1,"value":[1,0,0]}),
        json!({"command":"bridge_faces","faces":[6,11]}),
        json!({"command":"object_rename","object":2,"name":"Mesh"}),
        json!({"command":"select","object":2,"vertices":[1]}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    let region = document
        .execute(json!({"command":"inspect_region","object":2}))
        .unwrap();
    assert_eq!(region["vertices"].as_object().unwrap().len(), 8);
    assert_eq!(region["object"]["origin"], json!([2.0, 0.0, 0.0]));
    assert!(
        document
            .execute(json!({"command":"inspect_region","object":2,"faces":[1]}))
            .is_err()
    );
    let inset = document
        .execute(json!({"command":"inset","object":2,"face":8,"distance":0.03}))
        .unwrap();
    assert_eq!(
        inset["mesh"]["objects"]["2"]["vertices"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    let detached = document
        .execute(json!({"command":"split_vertices","object":2,"faces":[8]}))
        .unwrap();
    assert_eq!(
        detached["mesh"]["objects"]["2"]["vertices"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
}

#[test]
fn legacy_projects_gain_an_object_and_invalid_ownership_is_rejected() {
    let mut document = Document::default();
    let mut legacy = document.execute(json!({"command":"save"})).unwrap();
    legacy["version"] = json!(1);
    legacy["mesh"].as_object_mut().unwrap().remove("objects");
    let loaded = document
        .execute(json!({"command":"load","project":legacy}))
        .unwrap();
    assert_eq!(
        loaded["mesh"]["objects"]["1"]["vertices"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    create_gate(&mut document);
    let saved = document.execute(json!({"command":"save"})).unwrap();
    let mut invalid = saved.clone();
    invalid["mesh"]["objects"]["2"]["vertices"] = json!([1, 9, 10, 11, 12, 13, 14, 15, 16]);
    assert!(
        document
            .execute(json!({"command":"load","project":invalid}))
            .is_err()
    );
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), saved);
    let export = document
        .execute(json!({"command":"export","format":"fbx"}))
        .unwrap();
    let bytes = gpt_mesh::base64::decode(export["fbx_base64"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"Kaydara FBX Binary  \0\x1a\0"));
    assert_eq!(export["version"], 7400);
}
