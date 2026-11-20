use gpt_mesh::Document;
use serde_json::{Value, json};

fn patch(height: f64) -> Document {
    let mut document = Document::default();
    let project = json!({"version":1,"mesh":{
        "vertices":{"1":[-1,0,0],"2":[1,0,0],"3":[0,height,0.1],"4":[0,0,-1]},
        "faces":{"1":[1,2,3],"2":[2,1,4]},"selection":[]
    }});
    document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    document
}

fn grid() -> Document {
    let mut vertices = serde_json::Map::new();
    let mut faces = serde_json::Map::new();
    for z in 0..5 {
        for x in 0..5 {
            let bump = if x == 2 && z == 2 { 0.05 } else { 0.0 };
            vertices.insert(
                (z * 5 + x + 1).to_string(),
                json!([x as f64, 0.1 * x as f64 + 0.2 * z as f64 + bump, z as f64]),
            );
        }
    }
    let mut face = 1;
    for z in 0..4 {
        for x in 0..4 {
            let a = z * 5 + x + 1;
            faces.insert(face.to_string(), json!([a, a + 1, a + 6]));
            face += 1;
            faces.insert(face.to_string(), json!([a, a + 6, a + 5]));
            face += 1;
        }
    }
    let mut document = Document::default();
    document.execute(json!({"command":"load","project":{"version":1,"mesh":{"vertices":vertices,"faces":faces,"selection":[]}}})).unwrap();
    document
}

fn saved(document: &mut Document) -> Value {
    document.execute(json!({"command":"save"})).unwrap()
}

#[test]
fn edge_flips_improve_skinny_triangles_without_moving_vertices() {
    let mut document = patch(0.0);
    let before = saved(&mut document);
    let result = document.execute(json!({"command":"improve_triangulation","object":1,"max_deviation":0,"min_angle_gain":3})).unwrap();
    assert_eq!(result["operation"]["flipped_edges"], 1);
    assert_eq!(result["mesh"]["vertices"], before["mesh"]["vertices"]);
    assert_eq!(result["operation"]["max_patch_deviation"], 0.0);
    assert!(
        result["operation"]["changes"][0]["minimum_angle_gain"]
            .as_f64()
            .unwrap()
            > 30.0
    );
    assert_eq!(result["validation"]["boundary_edges"], 4);
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
    assert_eq!(
        document.execute(json!({"command":"redo"})).unwrap()["mesh"],
        result["mesh"]
    );
}

#[test]
fn nonplanar_patch_respects_surface_error_and_other_object_scope() {
    let mut document = patch(0.001);
    let mut project = saved(&mut document);
    for position in project["mesh"]["vertices"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        let height = position[1].as_f64().unwrap()
            + 0.1 * position[0].as_f64().unwrap()
            + 0.2 * position[2].as_f64().unwrap();
        position[1] = json!(height);
    }
    document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    document
        .execute(
            json!({"command":"object_create_box","name":"Untouched","min":[5,0,0],"max":[6,1,1]}),
        )
        .unwrap();
    let before = saved(&mut document);
    let result = document
        .execute(json!({"command":"improve_triangulation","object":1,"max_deviation":0}))
        .unwrap();
    assert_eq!(result["operation"]["flipped_edges"], 0);
    assert_eq!(result["mesh"], before["mesh"]);
    assert!(
        document
            .execute(json!({"command":"improve_triangulation","object":1,"max_deviation":-1}))
            .is_err()
    );
    assert_eq!(saved(&mut document), before);
    let accepted = document
        .execute(json!({"command":"improve_triangulation","object":1,"max_deviation":0.01}))
        .unwrap();
    assert_eq!(accepted["operation"]["flipped_edges"], 1);
    assert!(
        accepted["operation"]["max_patch_deviation"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert_eq!(
        accepted["mesh"]["objects"]["2"],
        before["mesh"]["objects"]["2"]
    );
    assert_eq!(accepted["mesh"]["vertices"], before["mesh"]["vertices"]);
}

#[test]
fn relaxation_keeps_boundaries_and_enforces_cumulative_vertex_limit() {
    let mut document = grid();
    let before = saved(&mut document);
    let result = document.execute(json!({"command":"relax_surface","object":1,"max_displacement":0.01,"iterations":5,"factor":0.4})).unwrap();
    assert!(result["operation"]["moved_vertices"].as_u64().unwrap() > 0);
    assert_eq!(result["mesh"]["faces"], before["mesh"]["faces"]);
    for z in 0..5 {
        for x in 0..5 {
            let id = (z * 5 + x + 1).to_string();
            let a = &before["mesh"]["vertices"][&id];
            let b = &result["mesh"]["vertices"][&id];
            if x == 0 || x == 4 || z == 0 || z == 4 {
                assert_eq!(a, b);
            }
            let distance = (0..3)
                .map(|axis| (a[axis].as_f64().unwrap() - b[axis].as_f64().unwrap()).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(distance <= 0.010000000001);
        }
    }
    assert!(
        result["mesh"]["vertices"]["13"][1].as_f64().unwrap()
            < before["mesh"]["vertices"]["13"][1].as_f64().unwrap()
    );
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
    assert_eq!(
        document.execute(json!({"command":"redo"})).unwrap()["mesh"],
        result["mesh"]
    );
    let project = saved(&mut document);
    let mut restored = Document::default();
    assert_eq!(
        restored
            .execute(json!({"command":"load","project":project}))
            .unwrap()["mesh"],
        result["mesh"]
    );
}

#[test]
fn invalid_cleanup_parameters_are_atomic() {
    let mut document = grid();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for request in [
        json!({"command":"relax_surface","object":1,"max_displacement":0}),
        json!({"command":"relax_surface","object":1,"iterations":100}),
        json!({"command":"relax_surface","objects":[99]}),
        json!({"command":"relax_surface","object":1,"objects":[1]}),
        json!({"command":"improve_triangulation","object":1,"feature_angle":90}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
}
