mod common;

use gpt_mesh::{Document, session::Session};
use serde_json::json;

#[test]
fn joining_preserves_surfaces_appearance_collections_and_history() {
    let mut document = Document::default();
    for request in [
        json!({"command":"object_create_box","name":"Other","min":[2,0,0],"max":[3,1,1]}),
        json!({"command":"object_create_box","name":"Untouched","min":[5,0,0],"max":[6,1,1]}),
        json!({"command":"vertex_color_set","object":2,"vertices":[9,10,11,12,13,14,15,16],"color":[0.2,0.4,0.6]}),
        json!({"command":"uv_set","values":[{"face":7,"vertex":9,"uv":[0.2,0.3]}]}),
        json!({"command":"texture_create","texture":"paint","width":2,"height":2,"color":[1,0,0,1]}),
        json!({"command":"texture_assign","object":2,"texture":"paint"}),
        json!({"command":"shade_region","object":2,"faces":[7],"mode":"flat"}),
        json!({"command":"collection_create","name":"Target collection","objects":[1],"primary":true}),
        json!({"command":"collection_create","name":"Source collection","objects":[2],"primary":true}),
    ] {
        document.execute(request).unwrap();
    }
    let before = document.execute(json!({"command":"save"})).unwrap();
    let joined = document
        .execute(json!({"command":"object_join","target":1,"objects":[2,2]}))
        .unwrap();
    for field in [
        "vertices",
        "faces",
        "uvs",
        "vertex_colors",
        "textures",
        "face_textures",
        "flat_faces",
    ] {
        assert_eq!(joined["mesh"][field], before["mesh"][field]);
    }
    assert_eq!(
        joined["mesh"]["objects"]["3"],
        before["mesh"]["objects"]["3"]
    );
    assert!(joined["mesh"]["objects"].get("2").is_none());
    assert_eq!(
        joined["mesh"]["objects"]["1"]["origin"],
        before["mesh"]["objects"]["1"]["origin"]
    );
    assert_eq!(
        joined["mesh"]["objects"]["1"]["vertices"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
    assert_eq!(joined["mesh"]["collections"]["2"]["objects"], json!([1]));
    assert_eq!(joined["mesh"]["primary_collections"], json!({"1":1}));
    let saved = document.execute(json!({"command":"save"})).unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    document.execute(json!({"command":"redo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), saved);
    let mut restored = Document::default();
    restored
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    assert_eq!(restored.execute(json!({"command":"save"})).unwrap(), saved);
}

#[test]
fn deleting_faces_prunes_only_new_orphans_and_cleans_appearance() {
    let mut document = Document::default();
    for request in [
        json!({"command":"split_vertices","faces":[2]}),
        json!({"command":"vertex_color_set","object":1,"vertices":[1,2,3,4,5,6,7,8,9,10,11,12],"color":[1,0,0]}),
        json!({"command":"uv_set","values":[{"vertex":9,"uv":[0.2,0.3]}]}),
        json!({"command":"texture_create","texture":"paint","width":2,"height":2,"color":[1,0,0,1]}),
        json!({"command":"texture_assign","faces":[2],"texture":"paint"}),
        json!({"command":"shade_region","object":1,"faces":[2],"mode":"flat"}),
    ] {
        document.execute(request).unwrap();
    }
    let before = document.execute(json!({"command":"save"})).unwrap();
    let deleted = document
        .execute(json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[2,2]}))
        .unwrap();
    assert_eq!(deleted["operation"]["removed_faces"], json!([2]));
    assert_eq!(
        deleted["operation"]["removed_vertices"],
        json!([9, 10, 11, 12])
    );
    assert_eq!(deleted["mesh"]["vertices"].as_object().unwrap().len(), 8);
    for field in ["uvs", "face_textures", "flat_faces"] {
        assert!(deleted["mesh"].get(field).is_none());
    }
    assert_eq!(
        deleted["mesh"]["vertex_colors"].as_object().unwrap().len(),
        8
    );
    assert_eq!(deleted["mesh"]["textures"], before["mesh"]["textures"]);
    assert_eq!(deleted["validation"]["boundary_edges"], 4);
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    assert_eq!(
        document.execute(json!({"command":"redo"})).unwrap()["mesh"],
        deleted["mesh"]
    );
}

#[test]
fn cleanup_errors_are_atomic_and_preserve_redo() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_create_box","name":"Other","min":[2,0,0],"max":[3,1,1]}))
        .unwrap();
    document
        .execute(
            json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[1]}),
        )
        .unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for request in [
        json!({"command":"object_join","target":1,"objects":[]}),
        json!({"command":"object_join","target":1,"objects":[1,2]}),
        json!({"command":"object_join","target":1,"objects":[2,999]}),
        json!({"command":"object_join","object":1,"target":1,"objects":[2]}),
        json!({"command":"object_join","target":999,"objects":[2]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[1,7]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[1,999]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[1,2,3,4,5,6]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"faces":[1]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[1],"revision":999}),
    ] {
        assert!(document.execute(request.clone()).is_err(), "{request}");
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    document.execute(json!({"command":"redo"})).unwrap();
}

#[test]
fn instance_guards_and_master_face_deletion_follow_existing_links() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_instance","object":1,"name":"Linked","value":[3,0,0]}))
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    for request in [
        json!({"command":"object_join","target":1,"objects":[2]}),
        json!({"command":"object_join","target":2,"objects":[1]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"object":2,"faces":[7]}),
    ] {
        assert!(document.execute(request).is_err());
        assert_eq!(
            document.execute(json!({"command":"inspect"})).unwrap(),
            before
        );
    }
    let deleted = document
        .execute(
            json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[2]}),
        )
        .unwrap();
    assert_eq!(deleted["mesh"]["faces"].as_object().unwrap().len(), 10);
    assert!(deleted["mesh"]["faces"].get("8").is_none());
    document
        .execute(json!({"command":"object_make_unique","object":2}))
        .unwrap();
    document
        .execute(json!({"command":"shade_smooth","object":2,"angle":35}))
        .unwrap();
    assert!(
        document
            .execute(json!({"command":"object_join","target":1,"objects":[2]}))
            .unwrap_err()
            .contains("smooth_angle")
    );
}

#[test]
fn session_reopens_cleanup_and_deleted_highest_ids_are_not_reused() {
    let root = std::env::temp_dir().join(format!("mesh-cleanup-commands-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    for request in [
        json!({"command":"workflow_status"}),
        json!({"command":"object_create_box","name":"Other","min":[2,0,0],"max":[3,1,1]}),
        json!({"command":"object_join","target":1,"objects":[2]}),
        json!({"command":"delete_faces","remove_orphan_vertices":true,"object":1,"faces":[12]}),
    ] {
        assert_eq!(session.execute(request)["ok"], true);
    }
    let before = std::fs::read(root.join("project.json")).unwrap();
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(std::fs::read(root.join("project.json")).unwrap(), before);
    let added = session
        .execute(json!({"command":"object_create_box","name":"New","min":[5,0,0],"max":[6,1,1]}));
    assert_eq!(added["ok"], true);
    assert_eq!(added["result"]["operation"]["object"], 3);
    assert_eq!(added["result"]["operation"]["faces"][0], 13);
    std::fs::remove_dir_all(root).unwrap();
}
