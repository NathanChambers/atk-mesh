use atk_mesh::Document;
use serde_json::json;

#[test]
fn coincident_weld_repairs_split_seams_without_moving_surfaces() {
    let mut document = Document::default();
    let original = document.execute(json!({"command":"save"})).unwrap();
    document
        .execute(json!({"command":"split_vertices","faces":[4]}))
        .unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let welded = document
        .execute(json!({"command":"weld_coincident","object":1,"expected_removed_vertices":4}))
        .unwrap();
    assert_eq!(welded["operation"]["positions_changed"], 0);
    assert_eq!(welded["mesh"]["vertices"], original["mesh"]["vertices"]);
    assert_eq!(welded["mesh"]["faces"], original["mesh"]["faces"]);
    assert_eq!(welded["validation"]["boundary_edges"], 0);
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
    assert_eq!(
        document.execute(json!({"command":"redo"})).unwrap()["mesh"],
        welded["mesh"]
    );
}

#[test]
fn coincident_weld_count_guard_and_invalid_topology_roll_back() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"split_vertices","faces":[4]}))
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    assert!(
        document
            .execute(json!({"command":"weld_coincident","object":1,"expected_removed_vertices":3}))
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    let mut project = document.execute(json!({"command":"save"})).unwrap();
    project["mesh"]["faces"]["99"] = project["mesh"]["faces"]["1"].clone();
    document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    assert!(
        document
            .execute(json!({"command":"weld_coincident","object":1,"expected_removed_vertices":4}))
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
}

#[test]
fn separate_keeps_face_positions_ids_shading_collections_and_undo() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"shade_smooth","object":1,"angle":35}))
        .unwrap();
    document
        .execute(
            json!({"command":"collection_create","name":"Assembly","objects":[1],"primary":true}),
        )
        .unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let separated = document.execute(json!({"command":"object_separate_faces","object":1,"parts":[{"name":"Top","faces":[4]},{"name":"Front","faces":[1]}]})).unwrap();
    assert_eq!(separated["mesh"]["objects"].as_object().unwrap().len(), 3);
    assert_eq!(separated["mesh"]["objects"]["2"]["smooth_angle"], 35.0);
    assert_eq!(
        separated["mesh"]["collections"]["1"]["objects"],
        json!([1, 2, 3])
    );
    assert_eq!(separated["mesh"]["primary_collections"]["3"], 1);
    for (id, face) in before["mesh"]["faces"].as_object().unwrap() {
        for (index, vertex) in face.as_array().unwrap().iter().enumerate() {
            let original = vertex.as_u64().unwrap().to_string();
            let new = separated["mesh"]["faces"][id][index]
                .as_u64()
                .unwrap()
                .to_string();
            assert_eq!(
                before["mesh"]["vertices"][&original],
                separated["mesh"]["vertices"][&new]
            );
        }
    }
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
    assert_eq!(
        document.execute(json!({"command":"redo"})).unwrap()["mesh"],
        separated["mesh"]
    );
    let project = document.execute(json!({"command":"save"})).unwrap();
    let mut restored = Document::default();
    assert_eq!(
        restored
            .execute(json!({"command":"load","project":project}))
            .unwrap()["mesh"],
        separated["mesh"]
    );
}

#[test]
fn separating_nonadjacent_face_fans_duplicates_boundary_vertices() {
    let mut document = Document::default();
    let project = json!({"version":1,"mesh":{
        "vertices":{"1":[0,1,0],"2":[0,-1,0],"3":[1,0,0],"4":[0,0,1],"5":[-1,0,0],"6":[0,0,-1]},
        "faces":{"1":[1,4,3],"2":[1,5,4],"3":[1,6,5],"4":[1,3,6],"5":[2,3,4],"6":[2,4,5],"7":[2,5,6],"8":[2,6,3]},
        "selection":[]
    }});
    document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    let result = document.execute(json!({"command":"object_separate_faces","object":1,"parts":[{"name":"Opposite Faces","faces":[1,3]}]})).unwrap();
    assert_eq!(
        result["operation"]["duplicated_vertices"]["1"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(result["validation"]["non_manifold_edges"], 0);
    assert_eq!(result["mesh"]["faces"].as_object().unwrap().len(), 8);
}

#[test]
fn separate_rejects_overlap_empty_source_and_duplicate_names_atomically() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for parts in [
        json!([{"name":"Top","faces":[4]},{"name":"Other","faces":[4]}]),
        json!([{"name":"All","faces":[1,2,3,4,5,6]}]),
        json!([{"name":"Mesh","faces":[4]}]),
        json!([{"name":"Missing","faces":[99]}]),
    ] {
        assert!(
            document
                .execute(json!({"command":"object_separate_faces","object":1,"parts":parts}))
                .is_err()
        );
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
}
