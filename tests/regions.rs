use gpt_mesh::Document;
use serde_json::{Value, json};

fn emblem(document: &mut Document) -> u64 {
    let result = document.execute(json!({"command":"extrude_polygon","name":"Emblem","axis":"Z","points":[[0,0],[3,0],[3,3],[2,3],[2,1],[1,1],[1,3],[0,3]],"depth":0.3,"bevel":0.04,"bevel_segments":4})).unwrap();
    let vertices = result["operation"]["front_vertices"].as_array().unwrap();
    let front = result["mesh"]["faces"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, face)| {
            face.as_array()
                .unwrap()
                .iter()
                .all(|v| vertices.contains(v))
        })
        .unwrap()
        .0
        .parse()
        .unwrap();
    document
        .execute(json!({"command":"shade_smooth","object":2,"angle":60}))
        .unwrap();
    front
}

fn saved(document: &mut Document) -> Value {
    document.execute(json!({"command":"save"})).unwrap()
}

fn reject(document: &mut Document, request: Value) {
    let before = saved(document);
    assert!(document.execute(request).is_err());
    assert_eq!(saved(document), before);
}

#[test]
fn planar_cap_diagnosis_and_local_normal_repair_preserve_geometry_and_history() {
    let mut document = Document::default();
    let face = emblem(&mut document);
    let before = saved(&mut document);
    let query =
        json!({"command":"inspect_shading","face":face,"coplanar":true,"object":2,"limit":2});
    let report = document.execute(query.clone()).unwrap();
    assert_eq!(report["face_count"], 6);
    assert!(report["max_normal_deviation_degrees"].as_f64().unwrap() > 1.0);
    assert_eq!(report["issues"].as_array().unwrap().len(), 2);
    assert_eq!(saved(&mut document), before);
    document
        .execute(
            json!({"command":"shade_region","face":face,"coplanar":true,"object":2,"mode":"flat"}),
        )
        .unwrap();
    let repaired = saved(&mut document);
    for field in ["vertices", "faces", "objects", "selection"] {
        assert_eq!(repaired["mesh"][field], before["mesh"][field]);
    }
    assert_eq!(repaired["mesh"]["flat_faces"].as_array().unwrap().len(), 6);
    let report = document.execute(query.clone()).unwrap();
    assert!(report["max_normal_deviation_degrees"].as_f64().unwrap() < 1e-5);
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(saved(&mut document), before);
    document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(saved(&mut document), repaired);
    let mut reopened = Document::default();
    reopened
        .execute(json!({"command":"load","project":repaired}))
        .unwrap();
    assert!(
        reopened.execute(query).unwrap()["max_normal_deviation_degrees"]
            .as_f64()
            .unwrap()
            < 1e-5
    );
    reject(
        &mut reopened,
        json!({"command":"shade_region","face":face,"object":1,"mode":"flat"}),
    );
    reject(
        &mut reopened,
        json!({"command":"inspect_shading","face":face,"limit":999}),
    );
}

#[test]
fn region_retriangulation_and_support_inset_preserve_existing_vertices_and_other_objects() {
    let mut document = Document::default();
    let face = emblem(&mut document);
    let before = saved(&mut document);
    document
        .execute(json!({"command":"retriangulate_region","face":face,"coplanar":true,"object":2}))
        .unwrap();
    let after = saved(&mut document);
    assert_eq!(before["mesh"]["vertices"], after["mesh"]["vertices"]);
    let query = json!({"command":"inspect_shading","face":face,"coplanar":true,"object":2});
    let old_quality = document.execute(query.clone()).unwrap()["min_triangle_quality"]
        .as_f64()
        .unwrap();
    document
        .execute(json!({"command":"retriangulate_region","face":face,"coplanar":true,"object":2}))
        .unwrap();
    assert!(
        document.execute(query).unwrap()["min_triangle_quality"]
            .as_f64()
            .unwrap()
            >= old_quality - 1e-10
    );
    let result = document.execute(json!({"command":"inset_region","face":face,"coplanar":true,"object":2,"distance":0.02})).unwrap();
    assert_eq!(result["validation"]["boundary_edges"], 0);
    assert_eq!(result["validation"]["non_manifold_edges"], 0);
    assert_eq!(
        result["operation"]["created_vertices"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        result["mesh"]["objects"]["1"],
        before["mesh"]["objects"]["1"]
    );
    for (id, point) in before["mesh"]["vertices"].as_object().unwrap() {
        assert_eq!(result["mesh"]["vertices"][id], *point);
    }
    for id in result["operation"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .take(6)
    {
        let report = document
            .execute(json!({"command":"inspect_shading","face":id}))
            .unwrap();
        assert!(
            report["max_normal_deviation_degrees"].as_f64().unwrap() < 1e-5,
            "inner cap must be flat without a normal override"
        );
    }
    reject(
        &mut document,
        json!({"command":"inset_region","face":face,"coplanar":true,"object":2,"distance":1}),
    );
    reject(
        &mut document,
        json!({"command":"retriangulate_region","object":2}),
    );
}

#[test]
fn invalid_region_edits_and_invalid_saved_normal_references_are_atomic() {
    let mut document = Document::default();
    for request in [
        json!({"command":"shade_region","face":1,"mode":"unknown"}),
        json!({"command":"shade_region","face":999,"mode":"flat"}),
        json!({"command":"shade_region","face":1,"mode":"flat","revision":99}),
        json!({"command":"inset_region","face":1,"distance":10}),
        json!({"command":"retriangulate_region","faces":[1,2]}),
        json!({"command":"inspect_shading","face":1,"coplanar":"true"}),
    ] {
        reject(&mut document, request);
    }
    let mut project = saved(&mut document);
    project["mesh"]["flat_faces"] = json!([999]);
    reject(&mut document, json!({"command":"load","project":project}));
}

#[test]
fn edge_flips_improve_a_bad_diagonal_without_moving_or_removing_vertices() {
    let mut document = Document::default();
    document.execute(json!({"command":"load","project":{"version":1,"mesh":{"vertices":{"1":[0,0,0],"2":[3,0,0],"3":[3,1,0],"4":[0,2,0]},"faces":{"1":[1,2,4],"2":[2,3,4]},"selection":[]}}})).unwrap();
    let before = saved(&mut document);
    let query = json!({"command":"inspect_shading","face":1,"coplanar":true});
    let quality = document.execute(query.clone()).unwrap()["min_triangle_quality"]
        .as_f64()
        .unwrap();
    let repaired = document
        .execute(json!({"command":"retriangulate_region","face":1,"coplanar":true}))
        .unwrap();
    assert_eq!(repaired["operation"]["edge_flips"], 1);
    assert_eq!(repaired["mesh"]["vertices"], before["mesh"]["vertices"]);
    assert!(
        document.execute(query).unwrap()["min_triangle_quality"]
            .as_f64()
            .unwrap()
            > quality
    );
}
