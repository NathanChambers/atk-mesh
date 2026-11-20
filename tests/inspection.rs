use gpt_mesh::Document;
use serde_json::json;

#[test]
fn face_region_contains_exact_boundary_and_neighbor_context_without_mutating_state() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let region = document
        .execute(json!({"command":"inspect_region","faces":[4]}))
        .unwrap();
    assert_eq!(region["faces"], json!({"4":[4,8,7,3]}));
    assert_eq!(region["vertices"].as_object().unwrap().len(), 4);
    assert_eq!(region["neighbor_faces"].as_object().unwrap().len(), 4);
    assert_eq!(region["context_vertices"].as_object().unwrap().len(), 8);
    assert_eq!(region["bounds"]["min"], json!([-1.0, 1.0, -1.0]));
    assert_eq!(region["bounds"]["max"], json!([1.0, 1.0, 1.0]));
    assert_eq!(region["boundary_edges"].as_array().unwrap().len(), 4);
    for edge in region["edge_incidence"].as_array().unwrap() {
        assert_eq!(edge["faces"].as_array().unwrap().len(), 2);
    }
    assert_eq!(
        before,
        document.execute(json!({"command":"inspect"})).unwrap()
    );
}

#[test]
fn vertex_region_expands_to_incident_faces_and_rejects_unknown_ids() {
    let mut document = Document::default();
    let region = document
        .execute(json!({"command":"inspect_region","vertices":[1]}))
        .unwrap();
    assert_eq!(region["faces"].as_object().unwrap().len(), 3);
    assert_eq!(region["vertices"].as_object().unwrap().len(), 7);
    for request in [
        json!({"faces":[999]}),
        json!({"vertices":[999]}),
        json!({"faces":[]}),
        json!({"vertices":"1"}),
    ] {
        let mut request = request;
        request["command"] = json!("inspect_region");
        assert!(document.execute(request).is_err());
    }
}

#[test]
fn centered_closeup_keeps_selection_in_the_captured_coordinate_system() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"select","vertices":[1,2,3,4,5,6,7,8]}))
        .unwrap();
    document
        .execute(json!({"command":"move","value":[10,20,30]}))
        .unwrap();
    let view = document
        .execute(json!({"command":"capture","yaw":0,"pitch":0,"center":[10,20,30],"zoom":100}))
        .unwrap();
    assert_eq!(view["camera"]["center"], json!([10.0, 20.0, 30.0]));
    let selection = document
        .execute(
            json!({"command":"select_image","view_id":view["view_id"],"rect":[295,195,505,205]}),
        )
        .unwrap();
    assert_eq!(selection["mesh"]["selection"], json!([7, 8]));
    assert!(
        document
            .execute(json!({"command":"capture","center":[0,0]}))
            .is_err()
    );
}
