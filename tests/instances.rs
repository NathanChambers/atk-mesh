mod common;

use gpt_mesh::{Document, session::Session};
use serde_json::{Value, json};

fn instance(document: &mut Document) -> Value {
    document
        .execute(json!({"command":"object_instance","object":1,"name":"Linked","value":[4,0,0]}))
        .unwrap()
}

fn state(document: &mut Document) -> Value {
    document.execute(json!({"command":"save"})).unwrap()
}

fn point(mesh: &Value, id: u64) -> [f64; 3] {
    serde_json::from_value(mesh["vertices"][id.to_string()].clone()).unwrap()
}

fn assert_close(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() < 1e-8,
            "{actual:?} != {expected:?}"
        );
    }
}

fn assert_link(mesh: &Value) {
    let link = &mesh["objects"]["2"]["instance"];
    let master = link["master"].as_u64().unwrap().to_string();
    let origin: [f64; 3] =
        serde_json::from_value(mesh["objects"][master]["origin"].clone()).unwrap();
    let anchor: [f64; 3] = serde_json::from_value(link["anchor"].clone()).unwrap();
    let rotation: [[f64; 3]; 3] = serde_json::from_value(link["rotation"].clone()).unwrap();
    for (source, target) in link["vertices"].as_object().unwrap() {
        let source = point(mesh, source.parse().unwrap());
        let expected = std::array::from_fn(|row| {
            anchor[row]
                + (0..3)
                    .map(|axis| rotation[row][axis] * (source[axis] - origin[axis]))
                    .sum::<f64>()
        });
        assert_close(point(mesh, target.as_u64().unwrap()), expected);
    }
    for (source, target) in link["faces"].as_object().unwrap() {
        let expected: Vec<_> = mesh["faces"][source]
            .as_array()
            .unwrap()
            .iter()
            .map(|vertex| link["vertices"][vertex.to_string()].clone())
            .collect();
        assert_eq!(mesh["faces"][target.to_string()], json!(expected));
    }
}

#[test]
fn master_geometry_topology_and_shading_propagate_with_stable_ids() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"collection_create","name":"Kit","objects":[1]}))
        .unwrap();
    let created = instance(&mut document);
    assert_eq!(
        created["operation"],
        json!({"object":2,"master_object":1,"linked":true})
    );
    assert_eq!(
        created["mesh"]["collections"]["1"]["objects"],
        json!([1, 2])
    );
    let remap = created["mesh"]["objects"]["2"]["instance"]["vertices"].clone();
    document
        .execute(json!({"command":"select","object":1,"vertices":[1]}))
        .unwrap();
    let edited = document
        .execute(json!({"command":"move","object":1,"value":[0,0.25,0]}))
        .unwrap();
    assert_eq!(
        edited["mesh"]["objects"]["2"]["instance"]["vertices"],
        remap
    );
    assert_link(&edited["mesh"]);
    let extruded = document
        .execute(json!({"command":"extrude","object":1,"face":4,"value":[0,1,0]}))
        .unwrap();
    assert_eq!(
        extruded["mesh"]["objects"]["2"]["vertices"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    for (source, target) in remap.as_object().unwrap() {
        assert_eq!(
            extruded["mesh"]["objects"]["2"]["instance"]["vertices"][source],
            *target
        );
    }
    assert_link(&extruded["mesh"]);
    document
        .execute(json!({"command":"shade_smooth","object":1,"angle":60}))
        .unwrap();
    let flat = document
        .execute(json!({"command":"shade_region","object":1,"faces":[1],"mode":"flat"}))
        .unwrap();
    assert_eq!(flat["mesh"]["objects"]["2"]["smooth_angle"], 60.0);
    let face = flat["mesh"]["objects"]["2"]["instance"]["faces"]["1"].clone();
    assert!(
        flat["mesh"]["flat_faces"]
            .as_array()
            .unwrap()
            .contains(&face)
    );
    let before = state(&mut document);
    let added = document
        .execute(json!({"command":"object_add_box","mode":"append","object":1,"min":[2,2,2],"max":[3,3,3]}))
        .unwrap();
    let seed = added["operation"]["vertices"][0].clone();
    let deleted = document.execute(json!({"command":"object_delete_components","object":1,"vertices":[seed],"expected_vertices":8})).unwrap();
    assert_eq!(
        deleted["mesh"]["objects"]["2"]["vertices"],
        before["mesh"]["objects"]["2"]["vertices"]
    );
    assert_link(&deleted["mesh"]);
}

#[test]
fn independent_placement_and_pivots_survive_master_transforms() {
    let mut document = Document::default();
    instance(&mut document);
    document
        .execute(json!({"command":"object_set_origin","object":2,"origin":[3,0,0]}))
        .unwrap();
    document
        .execute(json!({"command":"object_rotate","object":2,"value":[20,65,-15]}))
        .unwrap();
    let moved_instance = document
        .execute(json!({"command":"object_move","object":2,"value":[0,2,3]}))
        .unwrap();
    assert!(moved_instance["operation"].get("shared_shape_edit").is_none());
    let before = state(&mut document);
    let ids = before["mesh"]["objects"]["2"]["vertices"]
        .as_array()
        .unwrap();
    for request in [
        json!({"command":"object_move","object":1,"value":[3,4,5]}),
        json!({"command":"object_rotate","object":1,"value":[12,40,-20]}),
        json!({"command":"object_set_origin","object":1,"origin":[2,1,3]}),
    ] {
        let moved_master = request["command"] == "object_move";
        let result = document.execute(request).unwrap();
        let operation = &result["operation"];
        if moved_master {
            assert_eq!(operation["object"], 1);
            assert_eq!(operation["linked_instances_unchanged"], 1);
            assert_eq!(operation["shared_shape_edit"]["select"], json!({"command":"select_object","object":1}));
            assert_eq!(operation["shared_shape_edit"]["move"], json!({"command":"move","object":1,"value":[3,4,5]}));
        } else {
            assert!(operation.get("shared_shape_edit").is_none());
        }
        for id in ids {
            let id = id.as_u64().unwrap();
            assert_close(point(&result["mesh"], id), point(&before["mesh"], id));
        }
        assert_eq!(
            result["mesh"]["objects"]["2"]["origin"],
            before["mesh"]["objects"]["2"]["origin"]
        );
        assert_link(&result["mesh"]);
    }
    document
        .execute(json!({"command":"select_object","object":1}))
        .unwrap();
    let changed = document
        .execute(json!({"command":"scale","object":1,"value":[1,2,1]}))
        .unwrap();
    assert_link(&changed["mesh"]);

    let placement = document
        .execute(json!({"command":"object_move","object":1,"value":[0,0,-0.1]}))
        .unwrap();
    let recipe = &placement["operation"]["shared_shape_edit"];
    document.execute(json!({"command":"undo"})).unwrap();
    document.execute(recipe["select"].clone()).unwrap();
    let shared_edit = document.execute(recipe["move"].clone()).unwrap();
    assert_link(&shared_edit["mesh"]);
    assert_eq!(shared_edit["mesh"]["objects"]["1"]["origin"], changed["mesh"]["objects"]["1"]["origin"]);
    assert_eq!(shared_edit["mesh"]["objects"]["2"]["origin"], changed["mesh"]["objects"]["2"]["origin"]);
    for id in ids {
        let id = id.as_u64().unwrap();
        assert_ne!(point(&shared_edit["mesh"], id), point(&changed["mesh"], id));
    }
}

#[test]
fn collection_transforms_apply_once_to_master_and_instance() {
    let mut document = Document::default();
    instance(&mut document);
    document
        .execute(json!({"command":"collection_create","name":"Both","objects":[1,2]}))
        .unwrap();
    let before = state(&mut document);
    let moved = document
        .execute(json!({"command":"object_move","collection":1,"value":[1,2,3]}))
        .unwrap();
    for (id, position) in before["mesh"]["vertices"].as_object().unwrap() {
        let previous: [f64; 3] = serde_json::from_value(position.clone()).unwrap();
        assert_close(
            point(&moved["mesh"], id.parse().unwrap()),
            [previous[0] + 1.0, previous[1] + 2.0, previous[2] + 3.0],
        );
    }
    let rotated = document
        .execute(json!({"command":"object_rotate","collection":1,"value":[0,90,0]}))
        .unwrap();
    for id in 9..=16 {
        let previous = point(&moved["mesh"], id);
        let origin = [5.0, 2.0, 3.0];
        assert_close(
            point(&rotated["mesh"], id),
            [
                origin[0] + previous[2] - origin[2],
                previous[1],
                origin[2] - previous[0] + origin[0],
            ],
        );
    }
    assert_link(&rotated["mesh"]);
}

#[test]
fn unique_copy_can_become_a_new_master_and_duplicate_stays_independent() {
    let mut document = Document::default();
    instance(&mut document);
    let before = state(&mut document);
    let unique = document
        .execute(json!({"command":"object_make_unique","object":2}))
        .unwrap();
    assert!(unique["mesh"]["objects"]["2"].get("instance").is_none());
    assert_eq!(unique["mesh"]["vertices"], before["mesh"]["vertices"]);
    assert_eq!(unique["mesh"]["faces"], before["mesh"]["faces"]);
    document
        .execute(
            json!({"command":"object_instance","object":2,"name":"New family","value":[0,0,4]}),
        )
        .unwrap();
    document
        .execute(json!({"command":"select","object":2,"vertices":[9]}))
        .unwrap();
    let edited = document
        .execute(json!({"command":"move","object":2,"value":[0,0.2,0]}))
        .unwrap();
    assert_eq!(
        edited["mesh"]["vertices"]["1"],
        before["mesh"]["vertices"]["1"]
    );
    assert_eq!(
        edited["mesh"]["vertices"]["17"][1],
        edited["mesh"]["vertices"]["9"][1]
    );
    let copied = document
        .execute(json!({"command":"object_duplicate","object":3,"name":"Independent"}))
        .unwrap();
    assert!(copied["mesh"]["objects"]["4"].get("instance").is_none());
}

#[test]
fn instance_guards_and_invalid_loads_are_atomic() {
    let mut document = Document::default();
    instance(&mut document);
    for request in [
        json!({"command":"object_add_box","mode":"append","object":2,"min":[4,2,0],"max":[5,3,1]}),
        json!({"command":"extrude","object":2,"face":7,"value":[0,0,-1]}),
        json!({"command":"shade_smooth","object":2,"angle":50}),
        json!({"command":"object_delete","object":1}),
        json!({"command":"object_instance","object":2,"name":"Chain"}),
    ] {
        let before = document.execute(json!({"command":"inspect"})).unwrap();
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    document
        .execute(json!({"command":"select","vertices":[9]}))
        .unwrap();
    let before = state(&mut document);
    assert!(
        document
            .execute(json!({"command":"move","value":[0,1,0]}))
            .unwrap_err()
            .contains("object_make_unique")
    );
    assert_eq!(state(&mut document), before);
    for (pointer, value) in [
        ("/mesh/objects/2/instance/master", json!(2)),
        ("/mesh/objects/2/instance/master", json!(99)),
        ("/mesh/objects/2/instance/vertices/1", json!(1)),
        ("/mesh/objects/2/instance/faces/1", json!(1)),
        ("/mesh/objects/2/instance/rotation/0/0", json!(2)),
        ("/mesh/vertices/9/0", json!(42)),
    ] {
        let mut invalid = before.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            document
                .execute(json!({"command":"load","project":invalid}))
                .is_err()
        );
        assert_eq!(state(&mut document), before);
    }
}

#[test]
fn history_persistence_queries_capture_and_exports_resolve_instances() {
    let mut document = Document::default();
    instance(&mut document);
    let saved = state(&mut document);
    document
        .execute(json!({"command":"object_make_unique","object":2}))
        .unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(state(&mut document), saved);
    document.execute(json!({"command":"redo"})).unwrap();
    assert!(
        state(&mut document)["mesh"]["objects"]["2"]
            .get("instance")
            .is_none()
    );
    let mut restored = Document::default();
    restored
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    let objects = restored
        .execute(json!({"command":"query_mesh","kind":"objects"}))
        .unwrap();
    assert_eq!(objects["items"][0]["instance_count"], 1);
    assert_eq!(objects["items"][0]["instances"], json!([{"id":2,"name":"Linked"}]));
    assert_eq!(objects["items"][1]["master_object"], 1);
    let mapping = restored
        .execute(json!({"command":"query_instance","object":2,"limit":2}))
        .unwrap();
    assert_eq!(mapping["total"], 8);
    assert_eq!(mapping["items"]["1"], 9);
    assert_eq!(mapping["next_offset"], 2);
    let targeted = restored
        .execute(json!({"command":"query_instance","object":2,"ids":[12]}))
        .unwrap();
    assert_eq!(targeted["items"], json!({"4":12}));
    let capture = restored.execute(json!({"command":"capture","mode":"object_ids","width":64,"height":64,"solo_objects":[2]})).unwrap();
    assert_eq!(capture["visibility"]["visible_object_count"], 1);
    let obj = restored
        .execute(json!({"command":"export","format":"obj"}))
        .unwrap();
    assert_eq!(
        obj["obj"]
            .as_str()
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("v "))
            .count(),
        16
    );
    let fbx = restored
        .execute(json!({"command":"export","format":"fbx"}))
        .unwrap();
    let bytes = gpt_mesh::base64::decode(fbx["fbx_base64"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"Kaydara FBX Binary"));
    restored
        .execute(json!({"command":"object_delete","object":2}))
        .unwrap();
    restored
        .execute(
            json!({"command":"object_create_box","name":"Remaining","min":[8,0,0],"max":[9,1,1]}),
        )
        .unwrap();
    restored
        .execute(json!({"command":"object_delete","object":1}))
        .unwrap();
}

#[test]
fn workspace_records_instance_commands_and_restores_links() {
    let root = std::env::temp_dir().join(format!("mesh-instances-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(
        session.execute(json!({"command":"workflow_status"}))["ok"],
        true
    );
    let input = json!({"command":"object_instance","object":1,"name":"Linked","value":[4,0,0]});
    let result = session.execute(input.clone());
    assert_eq!(result["ok"], true, "{result}");
    let trace = &result["trace"];
    let event = session
        .read(&format!(
            "/sessions/{}/events/{}",
            trace["session"].as_str().unwrap(),
            trace["event"]
        ))
        .unwrap();
    assert_eq!(event["input"], input);
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let result = session.execute(json!({"command":"query_instance","object":2}));
    assert_eq!(result["ok"], true);
    assert_eq!(result["result"]["master_object"], 1);
    drop(session);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_objects_link_without_replacing_ids_pivots_or_memberships() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_duplicate","object":1,"name":"Existing","value":[4,0,0]}))
        .unwrap();
    document
        .execute(json!({"command":"object_set_origin","object":2,"origin":[8,2,1]}))
        .unwrap();
    document
        .execute(json!({"command":"object_rotate","object":2,"value":[0,90,0]}))
        .unwrap();
    document
        .execute(json!({"command":"collection_create","name":"Target only","objects":[2]}))
        .unwrap();
    let before = state(&mut document);
    let linked = document
        .execute(
            json!({"command":"object_link_instance","object":2,"master":1,"rotation":[0,90,0]}),
        )
        .unwrap();
    assert_eq!(linked["mesh"]["faces"], before["mesh"]["faces"]);
    assert_eq!(linked["mesh"]["collections"], before["mesh"]["collections"]);
    assert_eq!(
        linked["mesh"]["objects"]["2"]["origin"],
        before["mesh"]["objects"]["2"]["origin"]
    );
    assert_eq!(
        linked["mesh"]["objects"]["2"]["vertices"],
        before["mesh"]["objects"]["2"]["vertices"]
    );
    for id in 1..=16 {
        assert_close(point(&linked["mesh"], id), point(&before["mesh"], id));
    }
    assert_link(&linked["mesh"]);
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(state(&mut document), before);
    document.execute(json!({"command":"redo"})).unwrap();
    document
        .execute(json!({"command":"select","object":1,"vertices":[1]}))
        .unwrap();
    let edited = document
        .execute(json!({"command":"move","object":1,"value":[0,0.25,0]}))
        .unwrap();
    assert_link(&edited["mesh"]);
}

#[test]
fn linking_rejects_shape_topology_shading_and_mapping_mismatches_atomically() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_duplicate","object":1,"name":"Existing","value":[4,0,0]}))
        .unwrap();
    for request in [
        json!({"command":"object_link_instance","object":2,"master":2}),
        json!({"command":"object_link_instance","object":2,"master":999}),
        json!({"command":"object_link_instance","object":2,"master":1,"vertex_map":{"1":9}}),
        json!({"command":"object_link_instance","object":2,"master":1,"rotation":[0,45,0]}),
    ] {
        let before = state(&mut document);
        assert!(document.execute(request).is_err());
        assert_eq!(state(&mut document), before);
    }
    for change in [
        json!({"command":"split_face","face":7,"vertices":[9,11]}),
        json!({"command":"shade_smooth","object":2,"angle":60}),
        json!({"command":"shade_region","object":2,"faces":[7],"mode":"flat"}),
    ] {
        document.execute(change).unwrap();
        let before = state(&mut document);
        assert!(
            document
                .execute(json!({"command":"object_link_instance","object":2,"master":1}))
                .is_err()
        );
        assert_eq!(state(&mut document), before);
        document.execute(json!({"command":"undo"})).unwrap();
    }
}

#[test]
fn coincident_vertices_require_an_explicit_topology_checked_mapping() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_add_box","mode":"append","object":1,"min":[1,-1,-1],"max":[3,1,1]}))
        .unwrap();
    let copied = document
        .execute(json!({"command":"object_duplicate","object":1,"name":"Existing","value":[0,0,4]}))
        .unwrap();
    let before = state(&mut document);
    assert!(
        document
            .execute(json!({"command":"object_link_instance","object":2,"master":1}))
            .unwrap_err()
            .contains("ambiguous")
    );
    assert_eq!(state(&mut document), before);
    let mapping = copied["operation"]["vertex_remap"].clone();
    let linked = document
        .execute(
            json!({"command":"object_link_instance","object":2,"master":1,"vertex_map":mapping}),
        )
        .unwrap();
    assert_link(&linked["mesh"]);
    assert_eq!(linked["mesh"]["faces"], before["mesh"]["faces"]);
}
