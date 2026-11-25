use gpt_mesh::Document;
use serde_json::json;

#[test]
fn interpolated_loft_matches_explicit_sections() {
    let start = [
        [-1.0, 0.0, -1.0],
        [1.0, 0.0, -1.0],
        [1.0, 0.0, 1.0],
        [-1.0, 0.0, 1.0],
    ];
    let end = start.map(|[x, _, z]| [x * 0.5, 4.0, z * 0.5]);
    for (kind, first, last) in [("linear", 0.25, 0.75), ("smoothstep", 0.15625, 0.84375)] {
        let mut document = Document::default();
        let generated = document.execute(json!({
            "command":"loft","name":"Loft","contours":[start,end],"cap_start":true,"cap_end":true,
            "interpolation":{"type":kind,"segments":4}
        })).unwrap();
        let section =
            |t: f64| start.map(|[x, _, z]| [x * (1.0 - t / 2.0), 4.0 * t, z * (1.0 - t / 2.0)]);
        let mut explicit = Document::default();
        let sampled = explicit
            .execute(json!({
                "command":"loft","name":"Loft","cap_start":true,"cap_end":true,
                "contours":[start,section(first),section(0.5),section(last),end]
            }))
            .unwrap();
        assert_eq!(generated["validation"]["boundary_edges"], 0);
        assert_eq!(generated["mesh"]["vertices"], sampled["mesh"]["vertices"]);
        assert_eq!(generated["mesh"]["faces"], sampled["mesh"]["faces"]);
    }
}

#[test]
fn radial_taper_and_ridge_match_explicit_samples() {
    let create = || {
        let mut document = Document::default();
        document
            .execute(json!({
                "command":"sweep","name":"Trunk","path":[[0,0,0],[0,1,0],[0,2,0],[0,3,0]],
                "profile":[[-1,-1],[1,-1],[1,1],[-1,1]]
            }))
            .unwrap();
        document
            .execute(json!({"command":"select_object","object":2}))
            .unwrap();
        document
    };
    let mut document = create();
    let generated = document.execute(json!({
        "command":"radial_profile","object":2,"axis":"Y",
        "profile":{"taper":{"range":[0,2],"offsets":[0.5,0],"ridge":{"amplitude":0.25,"cycles":1}}}
    })).unwrap();
    let mut explicit = create();
    let sampled = explicit
        .execute(json!({
            "command":"radial_profile","object":2,"axis":"Y","profile":[[0,0.5],[1,0.5],[2,0]]
        }))
        .unwrap();
    assert_eq!(generated["mesh"], sampled["mesh"]);

    let before = document.execute(json!({"command":"save"})).unwrap();
    assert!(
        document
            .execute(json!({
                "command":"radial_profile","object":2,"axis":"Y",
                "profile":{"taper":{"range":[0,2],"offsets":[0,-3]}}
            }))
            .is_err()
    );
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
}
