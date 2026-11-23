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
fn vertex_offsets_are_atomic_scoped_and_undoable() {
    let mut document = grid();
    let before = saved(&mut document);
    let invalid = json!({"command":"move_vertices","object":1,"offsets":[
        {"vertex":13,"delta":[0,0.02,0]},{"vertex":13,"delta":[0,0.03,0]}
    ]});
    assert!(document.execute(invalid).is_err());
    assert_eq!(saved(&mut document), before);

    let revision = document.revision();
    let edit = json!({"command":"move_vertices","object":1,"revision":revision,"offsets":[
        {"vertex":13,"delta":[0,0.02,0]},{"vertex":14,"delta":[0,0.03,0]}
    ]});
    let changed = document.execute(edit.clone()).unwrap();
    assert_eq!(changed["operation"]["moved_vertices"], 2);
    assert_eq!(changed["mesh"]["selection"], before["mesh"]["selection"]);
    assert_eq!(changed["mesh"]["faces"], before["mesh"]["faces"]);
    assert_eq!(
        changed["mesh"]["vertices"]["12"],
        before["mesh"]["vertices"]["12"]
    );
    let y = |mesh: &Value, vertex: &str| mesh["vertices"][vertex][1].as_f64().unwrap();
    assert!((y(&changed["mesh"], "13") - y(&before["mesh"], "13") - 0.02).abs() < 1e-10);
    assert!((y(&changed["mesh"], "14") - y(&before["mesh"], "14") - 0.03).abs() < 1e-10);
    assert!(document.execute(edit).is_err());
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
}

#[test]
fn sculpt_stroke_respects_surface_connectivity_boundary_and_cap() {
    let mut document = grid();
    let mut project = saved(&mut document);
    let original_vertices = project["mesh"]["vertices"].as_object().unwrap().clone();
    let original_faces = project["mesh"]["faces"].as_object().unwrap().clone();
    for (key, mut point) in original_vertices {
        let vertex = key.parse::<u64>().unwrap() + 25;
        point[1] = json!(point[1].as_f64().unwrap() + 0.025);
        project["mesh"]["vertices"][vertex.to_string()] = point;
    }
    for (key, face) in original_faces {
        let id = key.parse::<u64>().unwrap() + 32;
        let vertices: Vec<_> = face
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_u64().unwrap() + 25)
            .collect();
        project["mesh"]["faces"][id.to_string()] = json!(vertices);
    }
    project["mesh"]["objects"]["1"]["vertices"] = json!((1..=50).collect::<Vec<_>>());
    document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    let before = saved(&mut document);
    let region = document
        .execute(json!({"command":"inspect_region","object":1}))
        .unwrap();
    assert_eq!(region["topology"]["disconnected_components"], 2);
    let partial = document
        .execute(json!({"command":"inspect_region","object":1,"faces":[11]}))
        .unwrap();
    assert_eq!(partial["topology"]["disconnected_components"], 1);

    let anchor = json!({"face":11,"barycentric":[0,0,1]});
    let raised = json!({"command":"sculpt_stroke","object":1,"mode":"raise","radius":1.35,
        "strength":0.12,"max_displacement":0.04,"samples":[anchor,anchor]});
    let preview = document
        .execute(json!({"command":"transaction","commit":false,"commands":[raised]}))
        .unwrap();
    assert_eq!(saved(&mut document), before);
    let committed = document
        .execute(json!({"command":"transaction","commit":true,
        "preview_id":preview["preview_id"],"revision":preview["revision"]}))
        .unwrap();
    assert_eq!(committed["committed"], true);
    let after = saved(&mut document);
    let center = |project: &Value| project["mesh"]["vertices"]["13"][1].as_f64().unwrap();
    assert!(center(&after) < center(&before));
    assert!(center(&before) - center(&after) <= 0.04);
    assert_eq!(after["mesh"]["faces"], before["mesh"]["faces"]);
    for vertex in [1_u64, 3, 5, 21, 25].into_iter().chain(26..=50) {
        let key = vertex.to_string();
        assert_eq!(
            after["mesh"]["vertices"][&key],
            before["mesh"]["vertices"][&key]
        );
    }
    let undo = document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(undo["mesh"], before["mesh"]);
    document
        .execute(
            json!({"command":"sculpt_stroke","object":1,"mode":"lower","radius":1.35,
        "strength":0.03,"samples":[anchor]}),
        )
        .unwrap();
    assert!(center(&saved(&mut document)) > center(&before));
    assert!(
        document
            .execute(
                json!({"command":"sculpt_stroke","object":1,"mode":"lower","radius":1,
        "strength":0.05,"samples":[{"face":11,"barycentric":[0,0,0]}]})
            )
            .is_err()
    );
}

#[test]
fn sculpt_stroke_can_use_a_fixed_world_direction() {
    let mut document = grid();
    let before = saved(&mut document);
    let mut stroke = json!({"command":"sculpt_stroke","object":1,"mode":"lower","radius":1.35,
        "strength":0.12,"direction":[0,0,0],"samples":[{"face":11,"barycentric":[0,0,1]}]});

    assert!(document.execute(stroke.clone()).is_err());
    assert_eq!(saved(&mut document), before);

    stroke["direction"] = json!([3, 4, 0]);
    document.execute(stroke).unwrap();
    let after = saved(&mut document);
    let delta = |axis: usize| {
        after["mesh"]["vertices"]["13"][axis].as_f64().unwrap()
            - before["mesh"]["vertices"]["13"][axis].as_f64().unwrap()
    };
    assert!(delta(0) < 0.0 && delta(1) < 0.0);
    assert!((4.0 * delta(0) - 3.0 * delta(1)).abs() < 1e-12);
    assert_eq!(delta(2), 0.0);
}

#[test]
fn continuous_stroke_blends_between_ordered_anchors() {
    let mut document = grid();
    let before = saved(&mut document);
    let start = json!({"face":11,"barycentric":[1,0,0]});
    let end = json!({"face":11,"barycentric":[0,0,1]});
    let mut stroke = json!({"command":"sculpt_stroke","object":1,"mode":"raise","continuous":true,
        "radius":1.5,"strength":0.04,"samples":[start,end]});
    assert!(document.execute(stroke.clone()).is_err());
    assert_eq!(saved(&mut document), before);

    stroke["direction"] = json!([0, 1, 0]);
    let coarse = document.execute(stroke.clone()).unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    stroke["samples"] = json!([start, {"face":11,"barycentric":[0.5,0,0.5]}, end]);
    let dense = document.execute(stroke).unwrap();
    for vertex in ["7", "8", "12", "13", "17"] {
        let y = |result: &Value| result["mesh"]["vertices"][vertex][1].as_f64().unwrap();
        assert!((y(&coarse) - y(&dense)).abs() < 1e-12);
    }
    assert_eq!(
        dense["mesh"]["vertices"]["1"],
        before["mesh"]["vertices"]["1"]
    );
    assert_eq!(dense["mesh"]["faces"], before["mesh"]["faces"]);

    document.execute(json!({"command":"undo"})).unwrap();
    let mut tapered = json!({"command":"sculpt_stroke","object":1,"mode":"raise","continuous":true,
        "direction":[0,1,0],"radius":1.5,"strength":0.04,
        "samples":[start,{"face":11,"barycentric":[0,0,1],"pressure":1.1}]});
    assert!(document.execute(tapered.clone()).is_err());
    assert_eq!(saved(&mut document)["mesh"], before["mesh"]);

    tapered["samples"][1]["pressure"] = json!(0);
    let result = document.execute(tapered).unwrap();
    assert_eq!(
        result["mesh"]["vertices"]["13"],
        before["mesh"]["vertices"]["13"]
    );
    assert_ne!(
        result["mesh"]["vertices"]["7"],
        before["mesh"]["vertices"]["7"]
    );
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
