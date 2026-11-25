use gpt_mesh::Document;
use serde_json::{Value, json};

#[test]
fn ellipse_profile_matches_explicit_vertices() {
    let run = |profile: Value| {
        let mut document = Document::default();
        let result = document
            .execute(json!({
                "command":"sweep","name":"Elliptical","profile":profile,
                "path":[[0,0,0],[0,2,0]]
            }))
            .unwrap();
        (
            result["mesh"]["vertices"].clone(),
            result["mesh"]["faces"].clone(),
        )
    };
    let profile: Vec<_> = (0..12)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 12.0;
            [0.13 * angle.cos(), 0.105 * angle.sin()]
        })
        .collect();
    assert_eq!(
        run(json!({"ellipse":{"radii":[0.13,0.105],"sides":12}})),
        run(json!(profile))
    );
}

#[test]
fn bezier_and_scale_keys_match_explicit_samples() {
    let run = |path: Value, scales: Value| {
        let mut document = Document::default();
        let mut request = json!({
            "command":"sweep","name":"Curve","path":path,
            "profile":[[-0.1,-0.1],[0.1,-0.1],[0.1,0.1],[-0.1,0.1]]
        });
        request
            .as_object_mut()
            .unwrap()
            .extend(scales.as_object().unwrap().clone());
        let result = document.execute(request).unwrap();
        assert_eq!(result["validation"]["boundary_edges"], 0);
        assert_eq!(result["validation"]["non_manifold_edges"], 0);
        (
            result["mesh"]["vertices"].clone(),
            result["mesh"]["faces"].clone(),
        )
    };
    assert_eq!(
        run(
            json!({"bezier":{"start":[0,0,0],"controls":[[0,2,0]],"end":[1,4,0],"segments":2}}),
            json!({"scale_keys":[[0,2,1],[1,1,0.5]]})
        ),
        run(
            json!([[0, 0, 0], [0.25, 2, 0], [1, 4, 0]]),
            json!({"scales":[[2,1],[1.5,0.75],[1,0.5]]})
        )
    );
}

#[test]
fn swept_sections_and_elliptic_helices_are_closed_and_undoable() {
    for path in [
        json!([[0, 0, 0], [0, 1, 0], [0.2, 2, 0]]),
        json!({"helix":{"radius":[1,0.8],"height":2,"turns":3,"steps":144}}),
        json!({"line":{"start":[0,0,0],"end":[0,2,0],"segments":8}}),
        json!({"spline":{"points":[[0,0,0],[0.2,1,0],[-0.2,2,0],[0,3,0]],"segments_per_span":8}}),
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
        json!({"line":{"start":[0,0,0],"end":[0,2,0],"segments":4097}}),
        json!({"spline":{"points":[[0,0,0],[0,1,0],[0,2,0]],"segments_per_span":2049}}),
        json!({"spline":{"points":[[0,0,0],[0,0,0],[0,2,0]],"segments_per_span":3}}),
    ] {
        let before: Value = document.execute(json!({"command":"save"})).unwrap();
        assert!(document.execute(json!({"command":"sweep","name":"Invalid","profile":[[0,0],[1,0],[0,1]],"path":path})).is_err());
        assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    }
}

#[test]
fn invalid_scale_keys_leave_the_mesh_unchanged() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"save"})).unwrap();
    for keys in [
        json!([]),
        json!([[0.5, 1, 1], [0.5, 2, 2]]),
        json!([[0, 1, 0]]),
        json!([[2, 1, 1]]),
    ] {
        let request = json!({
            "command":"sweep","name":"Invalid","profile":[[0,0],[1,0],[0,1]],
            "path":{"line":{"start":[0,0,0],"end":[0,2,0],"segments":3}},
            "scale_keys":keys
        });
        assert!(document.execute(request).is_err());
        assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    }
}
