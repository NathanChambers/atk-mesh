use atk_mesh::Document;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn two_cubes() -> Value {
    let mut document = Document::default();
    let mut project = document.execute(json!({"command":"save"})).unwrap();
    let original = project["mesh"].clone();
    for (id, position) in original["vertices"].as_object().unwrap() {
        let id = id.parse::<u64>().unwrap() + 8;
        let mut position = position.clone();
        position[0] = json!(position[0].as_f64().unwrap() + 4.0);
        project["mesh"]["vertices"][id.to_string()] = position;
    }
    for (id, vertices) in original["faces"].as_object().unwrap() {
        let id = id.parse::<u64>().unwrap() + 6;
        let vertices: Vec<_> = vertices
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_u64().unwrap() + 8)
            .collect();
        project["mesh"]["faces"][id.to_string()] = json!(vertices);
    }
    project["mesh"]["objects"]["1"]["vertices"] = json!((1..=16).collect::<Vec<u64>>());
    project
}

fn load(project: Value) -> Document {
    let mut document = Document::default();
    document
        .execute(json!({"command":"load","project":project}))
        .unwrap();
    document
}

fn assert_rejected(document: &mut Document, request: Value) {
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let saved = document.execute(json!({"command":"save"})).unwrap();
    assert!(document.execute(request).is_err());
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), saved);
}

#[test]
fn bridge_removes_caps_preserves_vertices_and_joins_components_with_consistent_winding() {
    let mut document = load(two_cubes());
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let bridged = document
        .execute(json!({"command":"bridge_faces","faces":[6,11]}))
        .unwrap();
    assert_eq!(bridged["mesh"]["vertices"], before["mesh"]["vertices"]);
    assert_eq!(bridged["mesh"]["faces"].as_object().unwrap().len(), 14);
    assert!(bridged["mesh"]["faces"].get("6").is_none());
    assert!(bridged["mesh"]["faces"].get("11").is_none());
    assert_eq!(
        bridged["operation"]["created_faces"],
        json!([13, 14, 15, 16])
    );
    assert_eq!(
        bridged["operation"]["vertex_pairs"],
        json!([[2, 9], [3, 12], [7, 16], [6, 13]])
    );
    assert_eq!(bridged["validation"]["boundary_edges"], 0);
    assert_eq!(bridged["validation"]["non_manifold_edges"], 0);
    assert_eq!(16 + 14, bridged["edges"].as_array().unwrap().len() + 2);
    let saved = document.execute(json!({"command":"save"})).unwrap();
    let mut restored = load(saved);
    let restored = restored.execute(json!({"command":"inspect"})).unwrap();
    assert_eq!(restored["mesh"], bridged["mesh"]);
    let exported = document.execute(json!({"command":"export"})).unwrap();
    assert_eq!(
        exported["obj"]
            .as_str()
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("f "))
            .count(),
        14
    );
    assert_eq!(
        document.execute(json!({"command":"undo"})).unwrap()["mesh"],
        before["mesh"]
    );
    assert_eq!(
        document.execute(json!({"command":"redo"})).unwrap()["mesh"],
        bridged["mesh"]
    );
    // Another topology edit also validates the resulting vertex fans and edge winding.
    document
        .execute(json!({"command":"split_edge","edge":[2,9]}))
        .unwrap();
}

#[test]
fn bridge_finds_correspondence_independent_of_loop_start_and_supports_explicit_offset() {
    let mut project = two_cubes();
    project["mesh"]["faces"]["11"] = json!([13, 16, 12, 9]);
    let mut document = load(project.clone());
    let automatic = document
        .execute(json!({"command":"bridge_faces","faces":[6,11]}))
        .unwrap();
    assert_eq!(automatic["operation"]["offset"], 3);
    let mut document = load(project);
    let explicit = document
        .execute(json!({"command":"bridge_faces","faces":[6,11],"offset":3}))
        .unwrap();
    assert_eq!(automatic["mesh"], explicit["mesh"]);
}

#[test]
fn invalid_bridges_are_atomic_and_preserve_redo() {
    let mut document = load(two_cubes());
    document
        .execute(json!({"command":"select","vertices":[1]}))
        .unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    for faces in [
        json!([6, 6]),
        json!([6, 999]),
        json!([1, 5]),
        json!([5, 11]),
        json!([6]),
        json!(["6", 11]),
    ] {
        assert_rejected(
            &mut document,
            json!({"command":"bridge_faces","faces":faces}),
        );
    }
    for offset in [
        json!(1),
        json!(4),
        json!(-1),
        json!(0.5),
        json!("0"),
        Value::Null,
        json!(u64::MAX),
    ] {
        assert_rejected(
            &mut document,
            json!({"command":"bridge_faces","faces":[6,11],"offset":offset}),
        );
    }
    document.execute(json!({"command":"redo"})).unwrap();
    document
        .execute(json!({"command":"split_edge","edge":[2,3]}))
        .unwrap();
    assert_rejected(
        &mut document,
        json!({"command":"bridge_faces","faces":[6,11]}),
    );
}

#[test]
fn bridge_rejects_warped_sides_and_rolls_back_allocation_failures() {
    let mut project = two_cubes();
    for id in 9..=16 {
        let point = &mut project["mesh"]["vertices"][id.to_string()];
        let y = point[1].as_f64().unwrap();
        let z = point[2].as_f64().unwrap();
        point[1] = json!(y * 0.3_f64.cos() - z * 0.3_f64.sin());
        point[2] = json!(y * 0.3_f64.sin() + z * 0.3_f64.cos());
    }
    let mut document = load(project);
    assert_rejected(
        &mut document,
        json!({"command":"bridge_faces","faces":[6,11]}),
    );
    let mut project = two_cubes();
    project["mesh"]["face_id_high_watermark"] = json!(u64::MAX - 2);
    let mut document = load(project);
    assert_rejected(
        &mut document,
        json!({"command":"bridge_faces","faces":[6,11]}),
    );
}

#[test]
fn bridge_can_close_a_handle_on_one_connected_mesh() {
    let cells = BTreeSet::from([(0, 0), (1, 0), (2, 0), (0, 1), (2, 1), (0, 2), (2, 2)]);
    let mut vertex_ids = BTreeMap::new();
    let mut vertices = BTreeMap::new();
    let mut faces = BTreeMap::new();
    let mut ends = vec![];
    let x_positions = [-3.0, -1.0, 1.0, 3.0];
    let y_positions = [-1.0, 1.0, 3.0, 4.0];
    for &(x, y) in &cells {
        let front = [(x, y, 0), (x, y + 1, 0), (x + 1, y + 1, 0), (x + 1, y, 0)];
        let back = [(x, y, 1), (x + 1, y, 1), (x + 1, y + 1, 1), (x, y + 1, 1)];
        let mut surfaces = vec![front, back];
        if !cells.contains(&(x - 1, y)) {
            surfaces.push([(x, y, 0), (x, y, 1), (x, y + 1, 1), (x, y + 1, 0)]);
        }
        if !cells.contains(&(x + 1, y)) {
            surfaces.push([
                (x + 1, y, 0),
                (x + 1, y + 1, 0),
                (x + 1, y + 1, 1),
                (x + 1, y, 1),
            ]);
        }
        if !cells.contains(&(x, y - 1)) {
            surfaces.push([(x, y, 0), (x + 1, y, 0), (x + 1, y, 1), (x, y, 1)]);
        }
        if !cells.contains(&(x, y + 1)) {
            surfaces.push([
                (x, y + 1, 0),
                (x, y + 1, 1),
                (x + 1, y + 1, 1),
                (x + 1, y + 1, 0),
            ]);
        }
        for surface in surfaces {
            let mut face = vec![];
            for key in surface {
                let next = vertex_ids.len() as u64 + 1;
                let id = *vertex_ids.entry(key).or_insert(next);
                vertices.insert(
                    id,
                    [
                        x_positions[key.0 as usize],
                        y_positions[key.1 as usize],
                        key.2 as f64 - 0.5,
                    ],
                );
                face.push(id);
            }
            let id = faces.len() as u64 + 1;
            let left_end = surface.iter().all(|&(x, y, _)| x == 1 && y >= 2);
            let right_end = surface.iter().all(|&(x, y, _)| x == 2 && y >= 2);
            if left_end || right_end {
                ends.push(id);
            }
            faces.insert(id, face);
        }
    }
    assert_eq!(ends.len(), 2);
    let project = json!({"version":1,"mesh":{"vertices":vertices,"faces":faces,"selection":[]}});
    let mut document = load(project);
    let bridged = document
        .execute(json!({"command":"bridge_faces","faces":ends}))
        .unwrap();
    assert_eq!(bridged["validation"]["boundary_edges"], 0);
    assert_eq!(bridged["validation"]["non_manifold_edges"], 0);
    let vertex_count = bridged["mesh"]["vertices"].as_object().unwrap().len();
    let face_count = bridged["mesh"]["faces"].as_object().unwrap().len();
    assert_eq!(
        vertex_count + face_count,
        bridged["edges"].as_array().unwrap().len()
    );
    let pair = &bridged["operation"]["vertex_pairs"][0];
    document
        .execute(json!({"command":"split_edge","edge":pair}))
        .unwrap();
}

#[test]
fn clean_capture_hides_annotations_without_changing_view_selection_or_document() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let annotated = document
        .execute(json!({"command":"capture","yaw":0,"pitch":0,"mode":"topology"}))
        .unwrap();
    assert_eq!(annotated["clean"], false);
    let clean = document
        .execute(json!({"command":"capture","yaw":0,"pitch":0,"clean":true}))
        .unwrap();
    let svg = clean["svg"].as_str().unwrap();
    assert!(!svg.contains("<circle"));
    assert!(!svg.contains("<text"));
    assert!(!svg.contains("stroke=\"#"));
    assert!(svg.contains("data:image/png;base64,"));
    assert_ne!(annotated["png_base64"], clean["png_base64"]);
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    let selected = document
        .execute(json!({"command":"select_image","view_id":clean["view_id"],"rect":[0,0,800,600]}))
        .unwrap();
    assert_eq!(selected["mesh"]["selection"], json!([5, 6, 7, 8]));
    assert_rejected(&mut document, json!({"command":"capture","clean":"true"}));
}
