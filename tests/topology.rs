use atk_mesh::Document;
use serde_json::{Value, json};

fn inspect(document: &mut Document) -> Value {
    document.execute(json!({"command":"inspect"})).unwrap()
}

fn assert_closed(state: &Value) {
    assert_eq!(state["validation"]["boundary_edges"], 0);
    assert_eq!(state["validation"]["non_manifold_edges"], 0);
    let vertices = state["mesh"]["vertices"].as_object().unwrap().len();
    let edges = state["edges"].as_array().unwrap().len();
    let faces = state["mesh"]["faces"].as_object().unwrap().len();
    assert_eq!(vertices + faces, edges + 2);
}

fn assert_rejected(document: &mut Document, request: Value) {
    let before = inspect(document);
    let project = document.execute(json!({"command":"save"})).unwrap();
    assert!(document.execute(request).is_err());
    assert_eq!(inspect(document), before);
    assert_eq!(
        document.execute(json!({"command":"save"})).unwrap(),
        project
    );
}

#[test]
fn weld_creates_an_exact_apex_and_removes_the_collapsed_cap() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"select", "vertices":[3,4,7,8]}))
        .unwrap();
    let before = inspect(&mut document);
    let view = document.execute(json!({"command":"capture"})).unwrap();
    let welded = document.execute(json!({"command":"weld"})).unwrap();
    assert_eq!(welded["mesh"]["vertices"].as_object().unwrap().len(), 5);
    assert_eq!(welded["mesh"]["faces"].as_object().unwrap().len(), 5);
    assert_eq!(welded["mesh"]["vertices"]["3"], json!([0.0, 1.0, 0.0]));
    assert_eq!(welded["mesh"]["selection"], json!([3]));
    assert_eq!(welded["operation"]["removed_faces"], json!([4]));
    assert_eq!(welded["operation"]["vertex_remap"]["8"], 3);
    assert_closed(&welded);
    assert_rejected(
        &mut document,
        json!({"command":"select_image", "view_id":view["view_id"], "rect":[0,0,800,600]}),
    );
    let undone = document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(undone["mesh"], before["mesh"]);
    let redone = document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(redone["mesh"], welded["mesh"]);
}

#[test]
fn weld_target_and_position_are_explicit_and_bad_welds_are_atomic() {
    let mut document = Document::default();
    assert_rejected(&mut document, json!({"command":"weld", "vertices":[1,7]}));
    assert_rejected(&mut document, json!({"command":"weld", "vertices":[1,3]}));
    assert_rejected(&mut document, json!({"command":"weld", "vertices":[1,1]}));
    assert_rejected(&mut document, json!({"command":"weld", "vertices":[1,999]}));
    assert_rejected(
        &mut document,
        json!({"command":"weld", "vertices":[1,2], "target":3}),
    );
    let welded = document
        .execute(json!({"command":"weld", "vertices":[1,2], "target":2}))
        .unwrap();
    assert_eq!(welded["mesh"]["vertices"]["2"], json!([1.0, -1.0, -1.0]));
    assert!(welded["mesh"]["vertices"].get("1").is_none());
    assert_closed(&welded);
}

#[test]
fn splitting_a_shared_edge_updates_both_face_loops_and_can_be_welded_back() {
    let mut document = Document::default();
    let original = inspect(&mut document);
    let split = document
        .execute(json!({"command":"split_edge", "edge":[1,2], "fraction":0.25}))
        .unwrap();
    assert_eq!(split["mesh"]["vertices"]["9"], json!([-0.5, -1.0, -1.0]));
    assert_eq!(split["mesh"]["faces"]["1"], json!([1, 4, 3, 2, 9]));
    assert_eq!(split["mesh"]["faces"]["3"], json!([1, 9, 2, 6, 5]));
    assert_eq!(split["mesh"]["selection"], json!([9]));
    assert_closed(&split);
    let welded = document
        .execute(json!({"command":"weld", "vertices":[1,9], "target":1}))
        .unwrap();
    assert_eq!(welded["mesh"]["vertices"], original["mesh"]["vertices"]);
    assert_eq!(welded["mesh"]["faces"], original["mesh"]["faces"]);
    let split_again = document
        .execute(json!({"command":"split_edge", "edge":[2,1]}))
        .unwrap();
    assert_eq!(split_again["operation"]["created_vertices"], json!([10]));
    assert_eq!(
        split_again["mesh"]["vertices"]["10"],
        json!([0.0, -1.0, -1.0])
    );
}

#[test]
fn bad_edge_splits_leave_history_and_allocators_unchanged() {
    let mut document = Document::default();
    for edge in [json!([1, 7]), json!([1, 1]), json!([1, 999]), json!([1])] {
        assert_rejected(&mut document, json!({"command":"split_edge", "edge":edge}));
    }
    for fraction in [
        json!(0),
        json!(1),
        json!(-0.1),
        json!(1.1),
        json!("half"),
        Value::Null,
    ] {
        assert_rejected(
            &mut document,
            json!({"command":"split_edge", "edge":[1,2], "fraction":fraction}),
        );
    }
    let split = document
        .execute(json!({"command":"split_edge", "edge":[1,2]}))
        .unwrap();
    assert_eq!(split["operation"]["created_vertices"], json!([9]));
}

#[test]
fn weld_can_repair_a_zero_length_edge() {
    let mut document = Document::default();
    let mut project = document.execute(json!({"command":"save"})).unwrap();
    project["mesh"]["vertices"]["9"] = project["mesh"]["vertices"]["1"].clone();
    project["mesh"]["faces"]["1"] = json!([1, 4, 3, 2, 9]);
    project["mesh"]["faces"]["3"] = json!([1, 9, 2, 6, 5]);
    document
        .execute(json!({"command":"load", "project":project}))
        .unwrap();
    let welded = document
        .execute(json!({"command":"weld", "vertices":[1,9], "target":1}))
        .unwrap();
    assert_closed(&welded);
    assert_eq!(welded["mesh"]["vertices"].as_object().unwrap().len(), 8);
    let split = document
        .execute(json!({"command":"split_edge", "edge":[1,2]}))
        .unwrap();
    assert_eq!(split["operation"]["created_vertices"], json!([10]));
}

#[test]
fn split_vertices_detaches_a_face_and_allows_independent_motion() {
    let mut document = Document::default();
    let split = document
        .execute(json!({"command":"split_vertices", "faces":[4]}))
        .unwrap();
    assert_eq!(split["mesh"]["vertices"].as_object().unwrap().len(), 12);
    assert_eq!(split["validation"]["boundary_edges"], 8);
    assert_eq!(split["mesh"]["faces"]["4"], json!([10, 12, 11, 9]));
    let moved = document
        .execute(json!({"command":"move", "value":[0,1,0]}))
        .unwrap();
    assert_eq!(moved["mesh"]["vertices"]["3"][1], 1.0);
    assert_eq!(moved["mesh"]["vertices"]["9"][1], 2.0);
    document.execute(json!({"command":"undo"})).unwrap();
    let undone = document.execute(json!({"command":"undo"})).unwrap();
    assert_closed(&undone);
}

#[test]
fn one_high_valence_vertex_can_be_separated_for_a_chosen_face() {
    let mut document = Document::default();
    let split = document
        .execute(json!({"command":"split_vertices", "vertices":[3], "faces":[4]}))
        .unwrap();
    assert_eq!(split["operation"]["vertex_remap"], json!({"3":9}));
    assert_eq!(split["mesh"]["faces"]["4"], json!([4, 8, 7, 9]));
    assert_eq!(split["mesh"]["faces"]["6"], json!([2, 3, 7, 6]));
    assert_rejected(
        &mut document,
        json!({"command":"split_vertices", "vertices":[1], "faces":[4]}),
    );
    assert_rejected(
        &mut document,
        json!({"command":"split_vertices", "faces":[999]}),
    );
}

#[test]
fn split_face_adds_a_shared_diagonal_with_opposite_winding() {
    let mut document = Document::default();
    assert_rejected(
        &mut document,
        json!({"command":"split_face", "face":4, "vertices":[4,8]}),
    );
    let split = document
        .execute(json!({"command":"split_face", "face":4, "vertices":[4,7]}))
        .unwrap();
    assert_eq!(split["mesh"]["faces"]["4"], json!([4, 8, 7]));
    assert_eq!(split["mesh"]["faces"]["7"], json!([7, 3, 4]));
    assert_closed(&split);
    document
        .execute(json!({"command":"bevel_vertex", "vertex":4}))
        .unwrap();
}

#[test]
fn distance_extrusion_matches_displacement_along_a_sloped_normal() {
    let run = |argument: Value| {
        let mut document = Document::default();
        document.execute(json!({"command":"object_rotate","object":1,"value":[30,0,0]})).unwrap();
        let mut request = json!({"command":"extrude","face":4});
        request.as_object_mut().unwrap().extend(argument.as_object().unwrap().clone());
        document.execute(request).unwrap()["mesh"].clone()
    };
    let angle = 30.0_f64.to_radians();
    let distance = 0.25;
    let a = run(json!({"distance":distance}));
    let b = run(json!({"value":[0,distance * angle.cos(),distance * angle.sin()]}));
    assert_eq!(a["faces"], b["faces"]);
    for (id, point) in a["vertices"].as_object().unwrap() {
        for axis in 0..3 {
            let difference = point[axis].as_f64().unwrap() - b["vertices"][id][axis].as_f64().unwrap();
            assert!(difference.abs() < 1e-12);
        }
    }
}

#[test]
fn region_extrusion_preserves_cap_ids_and_walls_only_the_perimeter() {
    let mut document = Document::default();
    let split = document
        .execute(json!({"command":"split_face","face":4,"vertices":[4,7]}))
        .unwrap();
    let other = split["operation"]["created_faces"][0].as_u64().unwrap();
    assert_rejected(
        &mut document,
        json!({"command":"extrude","faces":[4,6],"distance":0.2}),
    );
    let result = document
        .execute(json!({
            "command":"extrude","faces":[4,other],"object":1,"distance":0.25
        }))
        .unwrap();
    assert_closed(&result);
    assert_eq!(result["operation"]["cap_faces"], json!([4, other]));
    assert_eq!(
        result["operation"]["wall_faces"].as_array().unwrap().len(),
        4
    );
    for vertex in result["mesh"]["selection"].as_array().unwrap() {
        assert_eq!(
            result["mesh"]["vertices"][vertex.as_u64().unwrap().to_string()][1],
            1.25
        );
    }
}

#[test]
fn inset_depth_matches_inset_then_normal_extrusion() {
    for depth in [-0.25, 0.25] {
        let mut automatic = Document::default();
        let combined = automatic
            .execute(json!({
                "command":"inset","face":4,"distance":0.2,"depth":depth
            }))
            .unwrap();
        let mut explicit = Document::default();
        explicit
            .execute(json!({"command":"inset","face":4,"distance":0.2}))
            .unwrap();
        let separate = explicit
            .execute(json!({"command":"extrude","face":4,"distance":depth}))
            .unwrap();
        assert_closed(&combined);
        assert_eq!(combined["mesh"], separate["mesh"]);
        assert_eq!(
            combined["operation"]["created_faces"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
    }
}

#[test]
fn thicken_closes_a_bent_open_sheet_and_leaves_other_objects_alone() {
    let mut document = Document::default();
    let sheet = document
        .execute(json!({
            "command":"loft","name":"Roof sheet","closed":false,
            "contours":[[[3,0,0],[4,0.2,0],[5,0,0]],[[3,0,2],[4,0.2,2],[5,0,2]]]
        }))
        .unwrap();
    let object = sheet["operation"]["object"].as_u64().unwrap();
    assert_eq!(sheet["validation"]["boundary_edges"], 6);
    assert_rejected(
        &mut document,
        json!({"command":"thicken","object":object,"thickness":0.3}),
    );
    let result = document
        .execute(json!({
            "command":"thicken","object":object,"thickness":0.3,"direction":[0,-1,0]
        }))
        .unwrap();
    assert_eq!(result["validation"]["boundary_edges"], 0);
    assert_eq!(result["validation"]["non_manifold_edges"], 0);
    assert_eq!(
        result["operation"]["cap_faces"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        result["operation"]["wall_faces"].as_array().unwrap().len(),
        6
    );
    assert_eq!(
        result["mesh"]["vertices"]["1"],
        sheet["mesh"]["vertices"]["1"]
    );
    assert_rejected(
        &mut document,
        json!({"command":"thicken","object":object,"thickness":0.3}),
    );
}

#[test]
fn inset_offsets_edges_by_distance_and_can_be_extruded() {
    let mut document = Document::default();
    let inset = document
        .execute(json!({"command":"inset", "face":4, "distance":0.25}))
        .unwrap();
    assert_eq!(inset["mesh"]["vertices"]["9"], json!([-0.75, 1.0, -0.75]));
    assert_eq!(inset["mesh"]["vertices"]["11"], json!([0.75, 1.0, 0.75]));
    assert_eq!(inset["mesh"]["faces"].as_object().unwrap().len(), 10);
    assert_closed(&inset);
    let extruded = document
        .execute(json!({"command":"extrude", "face":4, "value":[0,1,0]}))
        .unwrap();
    assert_closed(&extruded);
    let exported = document.execute(json!({"command":"export"})).unwrap();
    assert_eq!(
        exported["obj"]
            .as_str()
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("v "))
            .count(),
        16
    );
}

#[test]
fn inset_rejects_inversion_and_works_after_collinear_edge_splits() {
    let mut document = Document::default();
    for distance in [0.0, -1.0, 1.0, 2.0] {
        assert_rejected(
            &mut document,
            json!({"command":"inset", "face":4, "distance":distance}),
        );
    }
    document
        .execute(json!({"command":"split_edge", "edge":[4,8]}))
        .unwrap();
    let inset = document
        .execute(json!({"command":"inset", "face":4, "distance":0.2}))
        .unwrap();
    assert_closed(&inset);
}

#[test]
fn inset_uses_edge_distance_on_a_rectangle_and_rejects_nonplanar_faces() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"select", "vertices":[1,2,3,4,5,6,7,8]}))
        .unwrap();
    document
        .execute(json!({"command":"scale", "value":[2,1,1]}))
        .unwrap();
    let inset = document
        .execute(json!({"command":"inset", "face":4, "distance":0.25}))
        .unwrap();
    assert_eq!(inset["mesh"]["vertices"]["9"], json!([-1.75, 1.0, -0.75]));
    assert_eq!(inset["mesh"]["vertices"]["11"], json!([1.75, 1.0, 0.75]));
    document
        .execute(json!({"command":"select", "vertices":[9]}))
        .unwrap();
    document
        .execute(json!({"command":"move", "value":[0,0.2,0]}))
        .unwrap();
    assert_rejected(
        &mut document,
        json!({"command":"inset", "face":4, "distance":0.1}),
    );
    assert_rejected(
        &mut document,
        json!({"command":"split_face", "face":4, "vertices":[9,11]}),
    );
}

#[test]
fn weld_rejects_duplicate_faces_from_collapsing_a_tetrahedron_edge() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"load", "project":{"version":1,"mesh":{
            "vertices":{"1":[0,0,0],"2":[1,0,0],"3":[0,1,0],"4":[0,0,1]},
            "faces":{"1":[1,3,2],"2":[1,2,4],"3":[2,3,4],"4":[3,1,4]},
            "selection":[]
        }}}))
        .unwrap();
    let before = inspect(&mut document);
    let error = document
        .execute(json!({"command":"weld", "vertices":[1,2]}))
        .unwrap_err();
    assert!(error.contains("duplicate faces"));
    assert_eq!(inspect(&mut document), before);
}

#[test]
fn vertex_bevel_opens_an_apex_into_a_four_sided_cap_and_weld_closes_it() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"weld", "vertices":[3,4,7,8]}))
        .unwrap();
    let bevel = document
        .execute(json!({"command":"bevel_vertex", "vertex":3, "fraction":0.25}))
        .unwrap();
    assert_eq!(bevel["mesh"]["vertices"].as_object().unwrap().len(), 8);
    assert_eq!(bevel["operation"]["created_faces"], json!([7]));
    assert_eq!(bevel["mesh"]["faces"]["7"].as_array().unwrap().len(), 4);
    for id in bevel["mesh"]["selection"].as_array().unwrap() {
        let id = id.as_u64().unwrap().to_string();
        assert_eq!(bevel["mesh"]["vertices"][id][1], 0.5);
    }
    assert_closed(&bevel);
    let welded = document
        .execute(json!({"command":"weld", "position":[0,1,0]}))
        .unwrap();
    assert_eq!(welded["mesh"]["vertices"].as_object().unwrap().len(), 5);
    assert_closed(&welded);
    let saved = document.execute(json!({"command":"save"})).unwrap();
    let mut loaded = Document::default();
    loaded
        .execute(json!({"command":"load", "project":saved}))
        .unwrap();
    let bevel_again = loaded
        .execute(json!({"command":"bevel_vertex", "vertex":9}))
        .unwrap();
    assert_eq!(
        bevel_again["operation"]["created_vertices"],
        json!([13, 15, 16, 14])
    );
    assert_eq!(bevel_again["operation"]["created_faces"], json!([8]));
}

#[test]
fn boundary_bevel_and_exhausted_allocations_are_atomic() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"split_vertices", "faces":[4]}))
        .unwrap();
    assert_rejected(&mut document, json!({"command":"bevel_vertex", "vertex":3}));
    let mut project = document.execute(json!({"command":"save"})).unwrap();
    project["mesh"]["vertex_id_high_watermark"] = json!(u64::MAX);
    document
        .execute(json!({"command":"load", "project":project}))
        .unwrap();
    assert_rejected(&mut document, json!({"command":"split_edge", "edge":[1,2]}));
    assert_rejected(
        &mut document,
        json!({"command":"inset", "face":1, "distance":0.2}),
    );
    assert_rejected(
        &mut document,
        json!({"command":"extrude", "face":1, "value":[0,0,-1]}),
    );
}
