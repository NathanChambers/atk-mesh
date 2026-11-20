use gpt_mesh::Document;
use serde_json::{Value, json};

fn saved(document: &mut Document) -> Value {
    document.execute(json!({"command":"save"})).unwrap()
}

#[test]
fn projections_fit_scope_filter_faces_and_preserve_geometry() {
    let mut document = Document::default();
    let before = saved(&mut document);
    let result = document.execute(json!({"command":"uv_project","object":1,"mode":"planar","axes":[0,1],"region":[0.5,0,1,0.5],"filter":{"normal":{"direction":[0,0,1],"range":[0.99,1]}}})).unwrap();
    assert_eq!(result["operation"]["face_count"], 1);
    let after = saved(&mut document);
    assert_eq!(after["mesh"]["uvs"]["2"]["5"], json!([0.5, 0.0]));
    assert_eq!(after["mesh"]["uvs"]["2"]["7"], json!([1.0, 0.5]));
    assert_eq!(after["mesh"]["uvs"].as_object().unwrap().len(), 1);
    for field in ["vertices", "faces", "objects", "selection", "flat_faces"] {
        assert_eq!(before["mesh"][field], after["mesh"][field]);
    }
    let query = document.execute(json!({"command":"query_uv","filter":{"normal":{"direction":[0,0,1],"absolute":true,"range":[0.99,1]}},"limit":1})).unwrap();
    assert_eq!(query["total"], 2);
    assert_eq!(query["next_offset"], 1);
    document
        .execute(json!({"command":"uv_project","mode":"box"}))
        .unwrap();
    let report = document.execute(json!({"command":"inspect_uv"})).unwrap();
    assert_eq!(report["missing_uv_faces"], 0);
    assert_eq!(report["degenerate_uv_faces"], 0);
    let before_failure = saved(&mut document);
    let revision = document.revision();
    for request in [
        json!({"command":"uv_project","mode":"planar","axes":[1,1]}),
        json!({"command":"uv_project","mode":"planar","bounds":[[0,0,0],[0,1,1]]}),
        json!({"command":"uv_project","mode":"box","region":[0,0,1e308,1e308]}),
        json!({"command":"uv_project","mode":"box","filter":{"normla":{}}}),
        json!({"command":"uv_project","mode":"box","revision":0}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(saved(&mut document), before_failure);
        assert_eq!(document.revision(), revision);
    }
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(saved(&mut document), after);
}

#[test]
fn cylindrical_mapping_has_local_seams_and_section_fitting() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"uv_project","mode":"box"}))
        .unwrap();
    document.execute(json!({"command":"uv_project","mode":"cylindrical","axis":1,"phase":0.75,"fit_sections":true,"filter":{"normal":{"direction":[0,1,0],"absolute":true,"range":[0,0.1]}}})).unwrap();
    let project = saved(&mut document);
    let vertices = project["mesh"]["faces"].as_object().unwrap();
    let mut seams = std::collections::BTreeMap::<u64, Vec<f64>>::new();
    for (face, corners) in vertices {
        let uv = &project["mesh"]["uvs"][face];
        let mut values = Vec::new();
        for vertex in corners.as_array().unwrap() {
            let id = vertex.as_u64().unwrap();
            let u = uv[id.to_string()][0].as_f64().unwrap();
            if face != "3" && face != "4" {
                values.push(u);
            }
            seams.entry(id).or_default().push(u);
        }
        if !values.is_empty() {
            let min = values.iter().copied().fold(f64::INFINITY, f64::min);
            let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert!(max - min <= 0.5 + 1e-12);
        }
    }
    assert!(seams.values().any(|values| {
        values
            .iter()
            .any(|a| values.iter().any(|b| (a - b).abs() > 0.9))
    }));
}

#[test]
fn paint_gradients_and_area_weighted_normals_support_blending_and_rollback() {
    let mut document = Document::default();
    document.execute(json!({"command":"vertex_color_gradient","object":1,"start":[0,-0.1,0],"end":[0,0.1,0],"from":[1,0,0],"to":[0,0,1]})).unwrap();
    let gradient = saved(&mut document);
    let points = gradient["mesh"]["vertices"].as_object().unwrap();
    for (id, point) in points {
        let y = point[1].as_f64().unwrap();
        let expected = if y < 0.0 {
            json!([1.0, 0.0, 0.0])
        } else {
            json!([0.0, 0.0, 1.0])
        };
        assert_eq!(gradient["mesh"]["vertex_colors"][id], expected);
    }
    document.execute(json!({"command":"vertex_color_shade","vertices":[5],"direction":[0,0,1],"ambient":0.25,"strength":0.5,"color":[0.5,1,1],"blend":"multiply"})).unwrap();
    let shade = saved(&mut document);
    let factor = 0.25 + 0.5 / 3.0_f64.sqrt();
    assert!(
        (shade["mesh"]["vertex_colors"]["5"][0].as_f64().unwrap() - factor * 0.5).abs() < 1e-12
    );
    assert_eq!(
        shade["mesh"]["vertex_colors"]["6"],
        gradient["mesh"]["vertex_colors"]["6"]
    );
    for request in [
        json!({"command":"vertex_color_gradient","start":[0,0,0],"end":[0,0,0],"from":[1,0,0],"to":[0,0,1]}),
        json!({"command":"vertex_color_shade","vertices":[9999],"direction":[0,0,1]}),
        json!({"command":"vertex_color_shade","direction":[0,0,1],"absolute_axes":[3]}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(saved(&mut document), shade);
    }
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(saved(&mut document), gradient);
    document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(saved(&mut document), shade);
}
