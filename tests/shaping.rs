use gpt_mesh::Document;
use serde_json::{Value, json};

fn cylinder(document: &mut Document, sweep: f64) -> Value {
    document.execute(json!({"command":"revolve","name":"Turned solid","profile":[[0,0],[1,0],[1,2],[0,2]],"segments":32,"sweep":sweep})).unwrap()
}

fn assert_closed(state: &Value) {
    assert_eq!(state["validation"]["boundary_edges"], 0);
    assert_eq!(state["validation"]["non_manifold_edges"], 0);
}

fn reject(document: &mut Document, request: Value) {
    let before = document.execute(json!({"command":"save"})).unwrap();
    assert!(document.execute(request).is_err());
    assert_eq!(before, document.execute(json!({"command":"save"})).unwrap());
}

#[test]
fn revolve_welds_poles_and_seam_and_partial_sweeps_have_consistent_closed_caps() {
    for sweep in [360.0, 270.0] {
        let mut document = Document::default();
        let result = cylinder(&mut document, sweep);
        assert_closed(&result);
        assert_eq!(result["operation"]["rings"][0]["count"], 1);
        assert_eq!(
            result["operation"]["rings"][1]["count"],
            if sweep == 360.0 { 32 } else { 33 }
        );
        let mesh = &result["mesh"];
        let vertices = mesh["vertices"].as_object().unwrap().len();
        let faces = mesh["faces"].as_object().unwrap().len();
        let edges = result["edges"].as_array().unwrap().len();
        assert_eq!(vertices + faces, edges + 4); // Two separate closed components, including the initial cube.
    }
}

#[test]
fn sampled_arcs_form_a_round_sphere_and_loop_queries_are_read_only() {
    let mut document = Document::default();
    let sphere=document.execute(json!({"command":"revolve","name":"Sphere","profile":[{"arc":{"center":[0,0],"radius":1,"start":-90,"end":90,"steps":32}}],"segments":64})).unwrap();
    assert_closed(&sphere);
    let rings = sphere["operation"]["rings"].as_array().unwrap();
    let first = rings[16]["first_vertex"].as_u64().unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let loop_data = document
        .execute(json!({"command":"inspect_loop","edge":[first,first+1]}))
        .unwrap();
    assert_eq!(loop_data["count"], 64);
    assert_eq!(before, document.execute(json!({"command":"save"})).unwrap());
    document
        .execute(json!({"command":"select_loop","edge":[first,first+1],"object":2}))
        .unwrap();
    let changed = document
        .execute(json!({"command":"radial_offset","axis":"Y","distance":-0.02,"object":2}))
        .unwrap();
    assert_closed(&changed);
    assert_eq!(
        changed["operation"]["changed_vertices"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    let p = &changed["mesh"]["vertices"][first.to_string()];
    let radius = p[0].as_f64().unwrap().hypot(p[2].as_f64().unwrap());
    assert!((radius - 0.98).abs() < 1e-10);
}

#[test]
fn plane_cuts_share_intersections_preserve_other_objects_and_allow_local_grooves() {
    let mut document = Document::default();
    cylinder(&mut document, 360.0);
    let original = document.execute(json!({"command":"save"})).unwrap();
    document
        .execute(json!({"command":"triangulate","object":2}))
        .unwrap();
    let cut = document
        .execute(json!({"command":"bisect_plane","object":2,"normal":[0,1,0],"offset":0.9}))
        .unwrap();
    assert_closed(&cut);
    for i in 1..=8 {
        assert_eq!(
            cut["mesh"]["vertices"][i.to_string()],
            original["mesh"]["vertices"][i.to_string()]
        );
    }
    let selected = cut["mesh"]["selection"].as_array().unwrap();
    assert!(selected.len() >= 32);
    for id in selected {
        assert!((cut["mesh"]["vertices"][id.to_string()][1].as_f64().unwrap() - 0.9).abs() < 1e-10);
    }
    let changed = document
        .execute(json!({"command":"radial_offset","axis":"Y","distance":-0.03,"object":2}))
        .unwrap();
    assert_closed(&changed);
    let undo = document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(undo["mesh"], cut["mesh"]);
    let redo = document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(redo["mesh"], changed["mesh"]);
}

#[test]
fn bad_shaping_requests_are_atomic_and_smoothing_survives_save_load() {
    let mut document = Document::default();
    for request in [
        json!({"command":"revolve","name":"Bad","segments":2,"profile":[[0,0],[1,0],[0,1]]}),
        json!({"command":"revolve","name":"Bad","segments":16,"profile":[[1,0],[1,1]]}),
        json!({"command":"revolve","name":"Bad","segments":16,"profile":[[0,0],[-1,1],[0,2]]}),
        json!({"command":"bisect_plane","object":1,"normal":[0,0,0]}),
        json!({"command":"shade_smooth","object":1,"angle":181}),
    ] {
        reject(&mut document, request);
    }
    let result = document
        .execute(json!({"command":"shade_smooth","object":1,"angle":45}))
        .unwrap();
    assert_eq!(result["mesh"]["objects"]["1"]["smooth_angle"], 45.0);
    let project = document.execute(json!({"command":"save"})).unwrap();
    let mut loaded = Document::default();
    let state = loaded
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    assert_eq!(state["mesh"], result["mesh"]);
}

#[test]
fn polygon_extrusion_caps_concave_outlines_and_rejects_crossing_contours() {
    let mut document = Document::default();
    let result=document.execute(json!({"command":"extrude_polygon","axis":"Z","name":"Emblem","depth":0.02,"points":[[-1,-1],[1,-1],[1,1],[0.4,1],[0.4,-0.4],[-0.4,-0.4],[-0.4,1],[-1,1]]})).unwrap();
    assert_closed(&result);
    reject(
        &mut document,
        json!({"command":"extrude_polygon","name":"Crossed","points":[[0,0],[1,1],[0,1],[1,0]]}),
    );
}

#[test]
fn dense_curved_surfaces_accept_precise_parallel_cuts_and_profiled_channels() {
    let mut document = Document::default();
    document.execute(json!({"command":"revolve","name":"Dense sphere","axis":"Z","segments":128,"profile":[{"arc":{"center":[0,0],"radius":1,"start":-90,"end":90,"steps":96}}]})).unwrap();
    document
        .execute(json!({"command":"triangulate","object":2}))
        .unwrap();
    let cut=document.execute(json!({"command":"bisect_plane","object":2,"normal":[1,0,0],"offsets":[0.738,0.744,0.75,0.758,0.766,0.774,0.782,0.79,0.798,0.804,0.81]})).unwrap();
    assert_closed(&cut);
    document
        .execute(json!({"command":"select_object","object":2}))
        .unwrap();
    let changed=document.execute(json!({"command":"radial_profile","object":2,"axis":"X","profile":[[0.738,0],[0.75,-0.01],[0.774,-0.028],[0.798,-0.01],[0.81,0]]})).unwrap();
    assert_closed(&changed);
    for (id, point) in cut["mesh"]["vertices"].as_object().unwrap() {
        let x = point[0].as_f64().unwrap();
        if !(0.738..=0.81).contains(&x) {
            assert_eq!(&changed["mesh"]["vertices"][id], point);
        }
    }
    reject(
        &mut document,
        json!({"command":"radial_profile","object":2,"profile":[[1,0],[0,-1]]}),
    );
}

#[test]
fn face_degeneracy_is_relative_to_local_scale() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"select_object","object":1}))
        .unwrap();
    let small = document
        .execute(json!({"command":"scale","value":[0.00001,0.00001,0.00001]}))
        .unwrap();
    assert_closed(&small);
    reject(&mut document, json!({"command":"scale","value":[1,0,1]}));
}

#[test]
fn hollow_partial_revolves_and_beveled_concave_outlines_are_closed() {
    let mut document = Document::default();
    let rim=document.execute(json!({"command":"revolve","name":"Rim","segments":32,"closed_profile":true,"sweep":270,"profile":[[0.8,0],[1,0],[1,0.2],[0.8,0.2]]})).unwrap();
    assert_closed(&rim);
    let relief=document.execute(json!({"command":"extrude_polygon","name":"Relief","axis":"Z","depth":0.2,"bevel":0.05,"bevel_segments":4,"points":[[-1,-1],[1,-1],[1,1],[0.4,1],[0.4,-0.4],[-0.4,-0.4],[-0.4,1],[-1,1]]})).unwrap();
    assert_closed(&relief);
    assert_eq!(relief["operation"]["bevel_segments"], 4);
    reject(
        &mut document,
        json!({"command":"extrude_polygon","name":"Bad bevel","depth":10,"bevel":2,"points":[[-1,-1],[1,-1],[1,1],[-1,1]]}),
    );
}
