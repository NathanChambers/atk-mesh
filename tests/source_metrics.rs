mod common;

use atk_mesh::{Document, session::Session};
use serde_json::{Value, json};

fn prism(profile: &[[f64; 2]], capped: bool) -> Document {
    let count = profile.len();
    let mut vertices = serde_json::Map::new();
    let mut faces = serde_json::Map::new();
    for (i, &[x, y]) in profile.iter().enumerate() {
        let a = i + 1;
        let b = (i + 1) % count + 1;
        vertices.insert(a.to_string(), json!([x, y, 0]));
        vertices.insert((a + count).to_string(), json!([x, y, 1]));
        faces.insert(a.to_string(), json!([a, b, b + count, a + count]));
    }
    if capped {
        for corner in 2..count {
            let id = count + 1 + 2 * (corner - 2);
            faces.insert(id.to_string(), json!([1, corner + 1, corner]));
            faces.insert(
                (id + 1).to_string(),
                json!([count + 1, count + corner, count + corner + 1]),
            );
        }
    }
    let mut document = Document::empty();
    document
        .execute(json!({"command":"load","project":{"version":1,"mesh":{
        "vertices":vertices,"faces":faces,"selection":[]}}}))
        .unwrap();
    document
}

fn inspect(document: &mut Document) -> Value {
    document.execute(json!({"command":"inspect"})).unwrap()["source_metrics"].clone()
}

#[test]
fn a_closed_sharp_return_and_interior_view_do_not_confirm_a_defect() {
    let mut document = prism(&[[0., 0.], [2., 0.], [0.1, 0.04]], true);
    let metrics = inspect(&mut document);
    assert_eq!(metrics["shells"][0]["closed"], true);
    assert!(!metrics["fold_candidates"].as_array().unwrap().is_empty());
    assert_eq!(metrics["confirmed_defects"], json!([]));
    assert_eq!(metrics["intersection_findings"], json!([]));

    let mut cube = Document::default();
    let interior = cube
        .execute(json!({"command":"capture","mode":"orientation",
        "projection":"perspective","position":[0,0,0.9],"target":[0,0,-1],"near":0.2,
        "width":64,"height":64}))
        .unwrap();
    assert_eq!(interior["orientation"]["back_facing_ratio"], 1.0);
    assert_eq!(inspect(&mut cube)["confirmed_defects"], json!([]));
}

#[test]
fn overlapping_folds_are_classified_in_their_shell_context() {
    let profile = [[-2., 0.], [2., 0.], [-1., 0.04], [1., -0.2]];
    for capped in [false, true] {
        let mut document = prism(&profile, capped);
        let metrics = inspect(&mut document);
        assert_eq!(metrics["shells"][0]["closed"], capped);
        assert_eq!(metrics["overlap_scan_complete"], true);
        assert!(!metrics["fold_candidates"].as_array().unwrap().is_empty());
        let findings = metrics["intersection_findings"].as_array().unwrap();
        let cross = findings
            .iter()
            .find(|issue| issue["faces"] == json!([1, 3]));
        assert!(
            cross.is_some(),
            "missing crossing between disjoint edges: {findings:?}"
        );
        let confirmed = metrics["confirmed_defects"].as_array().unwrap();
        assert_eq!(
            confirmed
                .iter()
                .any(|issue| issue["faces"] == json!([1, 3])),
            capped
        );
    }
}

#[test]
fn workflow_constraints_report_camera_threshold_and_pivot_failures() {
    let path = std::env::temp_dir().join(format!("mesh-source-metrics-{}", std::process::id()));
    common::write_cube_project(&path);
    let mut session = Session::open_workspace(path.clone()).unwrap();
    let sheet = json!({"camera":{"width":100,"height":100,"zoom":120,"yaw":22,"pitch":20},"threshold_pixels":100,"max_faces":0,"max_edges":0});
    let intent = session.execute(json!({"command":"workflow_assess","requirement":"intent",
        "assessment":"Check measured placement and projected detail","evidence":[],
        "constraints":{"pivot":[0,0,1],"thin_features":sheet,"max_corners":24}}));
    assert_eq!(intent["ok"], true, "{intent}");
    let stage = session.execute(json!({"command":"workflow_stage","stage":"complete"}));
    assert_eq!(stage["error"], "requirements_missing", "{stage}");
    let issues = stage["result"]["requirements"].as_array().unwrap();
    let thin = issues
        .iter()
        .find(|issue| issue["requirement"] == "thin_features")
        .unwrap();
    assert_eq!(thin["check"]["measured"]["face_count"], 6);
    assert_eq!(thin["check"]["measured"]["edge_count"], 12);
    assert!(issues.iter().any(|issue| issue["requirement"] == "pivot"));
    assert!(
        !issues
            .iter()
            .any(|issue| issue["requirement"] == "max_corners")
    );
    drop(session);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn sharing_vertices_does_not_allow_a_surface_across_a_polygon_diagonal() {
    let mut document = Document::empty();
    document
        .execute(json!({"command":"load","project":{"version":1,"mesh":{
        "vertices":{"1":[0,0,0],"2":[2,0,0],"3":[2,2,0],"4":[0,2,0],"5":[1,1,1]},
        "faces":{"1":[1,2,3,4],"2":[1,3,5]},"selection":[]}}}))
        .unwrap();
    let metrics = inspect(&mut document);
    let findings = metrics["intersection_findings"].as_array().unwrap();
    let issue = findings
        .iter()
        .find(|issue| issue["faces"] == json!([1, 2]))
        .unwrap();
    assert_eq!(issue["kind"], "surface_intersection");
}
#[test]
fn an_inward_cavity_is_distinct_from_an_inverted_outer_shell() {
    let mut doc = atk_mesh::Document::default();
    let saved = doc.execute(serde_json::json!({"command":"save"})).unwrap();
    let mut project = saved.clone();
    let original_faces = project["mesh"]["faces"].as_object().unwrap().clone();
    let original_vertices = project["mesh"]["vertices"].as_object().unwrap().clone();
    for (id, point) in original_vertices {
        let inner = id.parse::<u64>().unwrap()+8;
        let coords:Vec<_> = point.as_array().unwrap().iter().map(|v|v.as_f64().unwrap()*0.5).collect();
        project["mesh"]["vertices"][inner.to_string()] = serde_json::json!(coords);
        project["mesh"]["objects"]["1"]["vertices"].as_array_mut().unwrap().push(serde_json::json!(inner));
    }
    for (id, face) in original_faces {
        let inner=id.parse::<u64>().unwrap()+6;
        let points:Vec<_>=face.as_array().unwrap().iter().rev().map(|v|v.as_u64().unwrap()+8).collect();
        project["mesh"]["faces"][inner.to_string()]=serde_json::json!(points);
    }
    project["mesh"]["vertex_id_high_watermark"]=serde_json::json!(16);
    project["mesh"]["face_id_high_watermark"]=serde_json::json!(12);
    doc.execute(serde_json::json!({"command":"load","project":project})).unwrap();
    let report=doc.execute(serde_json::json!({"command":"inspect"})).unwrap();
    assert_eq!(report["source_metrics"]["confirmed_defects"],serde_json::json!([]));
    let mut inverted=saved;
    for face in inverted["mesh"]["faces"].as_object_mut().unwrap().values_mut() { face.as_array_mut().unwrap().reverse(); }
    doc.execute(serde_json::json!({"command":"load","project":inverted})).unwrap();
    let report=doc.execute(serde_json::json!({"command":"inspect"})).unwrap();
    assert_eq!(report["source_metrics"]["confirmed_defects"][0]["kind"],"inverted_shell");
}
