use atk_mesh::Document;
use serde_json::{Value, json};

fn legacy() -> Document {
    let mut doc = Document::default();
    let mut project = doc.execute(json!({"command":"save"})).unwrap();
    project["version"] = json!(1);
    doc.execute(json!({"command":"load","project":project}))
        .unwrap();
    doc
}

#[test]
fn tapered_concave_sweep_keeps_the_tip_walls_separate() {
    let sweep = json!({"command":"sweep","name":"Chevron tip",
        "path":[[4.,0.,0.],[4.2,0.,0.]],"up":[0,0,-1],
        "profile":[[-1,-0.38],[0,0],[1,-0.38],[0,-0.14]],"scales":[[0.0835,0.0751],[0.015,0.05]]});
    for (mut doc, expected) in [
        (legacy(), "detected"),
        (Document::default(), "not_detected"),
    ] {
        let result = doc.execute(sweep.clone()).unwrap();
        let object = &result["operation"]["object"];
        let report = doc
            .execute(json!({"command":"inspect_defects","objects":[object]}))
            .unwrap();
        assert_eq!(
            report["geometry"]["self_intersections"], expected,
            "{report}"
        );
    }
}

#[test]
fn generated_uvs_cover_sides_caps_poles_and_periodic_seams() {
    for mut request in [
        json!({"command":"sweep","profile":[[-0.1,-0.1],[0.1,-0.1],[0.1,0.1],[-0.1,0.1]],"path":[[0,0,0],[0,1,0],[0.5,3,0]]}),
        json!({"command":"extrude_polygon","points":[[0,0],[2,0],[2,1],[0,1]],"depth":2,"bevel":0.1}),
        json!({"command":"extrude_polygon","points":[[0,0],[6,0],[6,4],[0,4]],
            "holes":[[[1,1],[2,1],[2,3],[1,3]],[[4,1],[4,2],[5,2],[5,1]]],"depth":0.5}),
        json!({"command":"revolve","segments":8,"sweep":270,"profile":[[0,0],[1,0],[1,2],[0,2]]}),
        json!({"command":"revolve","segments":8,"closed_profile":true,"profile":[[1,0],[2,0],[2,2],[1,2]]}),
        json!({"command":"loft","closed":false,"contours":[[[0,0,0],[1,0,0],[2,0,0]],[[0,1,0],[1,1,0],[2,1,0]]]}),
    ] {
        let mut doc = Document::default();
        request["name"] = json!("Mapped");
        request["uv"] = json!({"side_region":[0,0,0.6,1],"cap_region":[0.6,0,1,1],"padding":0.01});
        let result = doc.execute(request.clone()).unwrap();
        let object = &result["operation"]["object"];
        let report = doc
            .execute(json!({"command":"inspect_uv","object":object}))
            .unwrap();
        for metric in [
            "missing_uv_faces",
            "degenerate_uv_faces",
            "outside_unit_square_faces",
        ] {
            assert_eq!(report[metric], 0, "{} {report}", request["command"]);
        }
        let project = doc.execute(json!({"command":"save"})).unwrap();
        let component = result["operation"]["component_id"].to_string();
        let caps = &project["mesh"]["components"][component]["uv_cap_faces"];
        for face in result["operation"]["created_face_ids"].as_array().unwrap() {
            let cap = caps.as_array().is_some_and(|ids| ids.contains(face));
            let corners = project["mesh"]["uvs"][face.to_string()]
                .as_object()
                .unwrap();
            for uv in corners.values() {
                let u = uv[0].as_f64().unwrap();
                let v = uv[1].as_f64().unwrap();
                assert!((0.009999..=0.990001).contains(&v));
                assert!(if cap {
                    (0.609999..=0.990001).contains(&u)
                } else {
                    (0.009999..=0.590001).contains(&u)
                });
            }
        }
        let checker=doc.execute(json!({"command":"capture","mode":"uv_checker","clean":true,"width":128,"height":128})).unwrap();
        assert_eq!(checker["mode"], "uv_checker");
    }
}

#[test]
fn polygon_holes_create_closed_passages_and_reject_touching_loops() {
    let mut document = Document::default();
    let outer = [[0, 0], [6, 0], [6, 4], [0, 4]];
    let result = document
        .execute(json!({
            "command":"extrude_polygon","name":"Perforated wall","axis":"Z","center":[0,0,3],
            "depth":0.5,"points":outer,
            "holes":[[[1,1],[2,1],[2,3],[1,3]],[[4,1],[4,2],[5,2],[5,1]]]
        }))
        .unwrap();
    assert_eq!(result["operation"]["hole_count"], 2);
    assert_eq!(
        result["operation"]["created_face_ids"]
            .as_array()
            .unwrap()
            .len(),
        40
    );
    assert_eq!(result["validation"]["boundary_edges"], 0);
    assert_eq!(result["validation"]["non_manifold_edges"], 0);
    let saved = document.execute(json!({"command":"save"})).unwrap();
    for holes in [
        json!([[[0, 1], [2, 1], [2, 3], [0, 3]]]),
        json!([
            [[1, 1], [3, 1], [3, 3], [1, 3]],
            [[2, 2], [4, 2], [4, 3], [2, 3]]
        ]),
        json!([
            [[1, 1], [4, 1], [4, 3], [1, 3]],
            [[2, 2], [3, 2], [3, 2.5], [2, 2.5]]
        ]),
    ] {
        assert!(document.execute(json!({
            "command":"extrude_polygon","name":"Invalid wall","axis":"Z","points":outer,"holes":holes
        })).is_err());
        assert_eq!(saved, document.execute(json!({"command":"save"})).unwrap());
    }
}

#[test]
fn shape_mask_defaults_are_versioned_and_coverage_tracks_edits() {
    for (mut doc, expected) in [(legacy(), 1.0), (Document::default(), 0.25)] {
        doc.execute(json!({"command":"texture_create","texture":"paint","width":8,"height":8,"color":[0,0,0,1]})).unwrap();
        let added=doc.execute(json!({"command":"texture_layer_add","texture":"paint","layer":{"name":"red","color":[1,0,0,1],"mask":{"polygons":[[[0,0],[0.5,0],[0.5,0.5],[0,0.5]]]}}})).unwrap();
        assert_eq!(added["operation"]["coverage"], expected);
        let before = doc.execute(json!({"command":"save"})).unwrap();
        let restored = doc
            .execute(json!({"command":"load","project":before}))
            .unwrap();
        assert_ne!(restored, Value::Null);
        doc.execute(json!({"command":"texture_bake","texture":"paint"}))
            .unwrap();
        let after = doc.execute(json!({"command":"save"})).unwrap();
        assert_eq!(before["mesh"]["textures"], after["mesh"]["textures"]);
        let set=doc.execute(json!({"command":"texture_layer_set","texture":"paint","layer":"red","changes":{"mask":{"fill":1}}})).unwrap();
        assert_eq!(set["operation"]["coverage"], 1.0);
    }
}
