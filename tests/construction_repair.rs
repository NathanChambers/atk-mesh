use atk_mesh::Document;
use serde_json::json;

#[test]
fn triangle_report_ranks_overlapping_regions_and_measures_selected_faces() {
    let mut document = Document::default();
    for (name, faces) in [("panel", vec![1]), ("housing", vec![1, 2, 3])] {
        document.execute(json!({"command":"region_define","object":1,"name":name,"faces":faces})).unwrap();
    }
    let query = json!({"command":"query_components","report":"triangles","limit":1});
    let result = document.execute(query.clone()).unwrap();
    assert_eq!(result["items"][0]["region"], "housing");
    assert_eq!(result["items"][0]["triangles"], 6);
    assert_eq!(result["items"][0]["asset_percent"], 50.0);
    assert_eq!(result["asset_triangles"], 12);
    assert_eq!(result["scope"]["triangles"], 12);
    assert_eq!(result["unassigned"]["triangles"], 6);
    assert_eq!(result["next_offset"], 1);

    let saved = document.execute(json!({"command":"save"})).unwrap();
    document.execute(json!({"command":"select","vertices":saved["mesh"]["faces"]["1"]})).unwrap();
    let mut selected = query;
    selected["selection"] = json!(true);
    let result = document.execute(selected).unwrap();
    assert_eq!(result["scope"]["triangles"], 2);
    assert_eq!(result["scope"]["dimensions"].as_array().unwrap().iter().filter(|v| v.as_f64() == Some(0.0)).count(), 1);
    assert_eq!(result["unassigned"]["dimensions"], serde_json::Value::Null);
}

#[test]
fn explicit_generation_and_named_replacement_are_atomic() {
    let mut document = Document::default();
    let shape = json!({"command":"extrude_polygon","object":1,"mode":"append","region":"cab_upper","points":[[0,0],[2,0],[2,1],[0,1]],"depth":0.3,"center":[0,3,0]});
    let mut missing = shape.clone();
    missing.as_object_mut().unwrap().remove("mode");
    assert!(document.execute(missing).unwrap_err().contains("mode"));
    let created = document.execute(shape.clone()).unwrap();
    let id = created["operation"]["component_id"].clone();
    let original = document.execute(json!({"command":"save"})).unwrap();
    let mut replacement = shape.clone();
    replacement["mode"] = json!("replace_region");
    replacement["points"] = json!([]);
    assert!(document.execute(replacement.clone()).is_err());
    assert_eq!(
        document.execute(json!({"command":"save"})).unwrap(),
        original
    );
    replacement["points"] = shape["points"].clone();
    replacement["depth"] = json!(0.6);
    let replaced = document.execute(replacement).unwrap();
    assert_eq!(replaced["operation"]["component_id"], id);
    assert_eq!(
        replaced["mesh"]["faces"].as_object().unwrap().len(),
        created["mesh"]["faces"].as_object().unwrap().len()
    );
    for vertex in 1..=8 {
        assert_eq!(
            replaced["mesh"]["vertices"][vertex.to_string()],
            original["mesh"]["vertices"][vertex.to_string()]
        );
    }
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(
        document.execute(json!({"command":"save"})).unwrap(),
        original
    );
}

#[test]
fn open_boundaries_can_be_filled_and_bridged_without_new_vertices() {
    let mut document = Document::default();
    document
        .execute(
            json!({"command":"delete_faces","object":1,"faces":[2],"remove_orphan_vertices":false}),
        )
        .unwrap();
    let boundary = document
        .execute(json!({"command":"query_boundaries","object":1,"response":"full"}))
        .unwrap();
    assert_eq!(boundary["total"], 1);
    let filled = document
        .execute(
            json!({"command":"create_face","object":1,"vertices":boundary["items"][0]["vertices"]}),
        )
        .unwrap();
    assert_eq!(filled["validation"]["boundary_edges"], 0);
    assert_eq!(filled["mesh"]["vertices"].as_object().unwrap().len(), 8);
    document.execute(json!({"command":"object_add_box","object":1,"mode":"append","min":[-1,-1,3],"max":[1,1,5]})).unwrap();
    let project = document.execute(json!({"command":"save"})).unwrap();
    let first = project["mesh"]["faces"]["7"].clone();
    let second = project["mesh"]["faces"]["8"].clone();
    document.execute(json!({"command":"delete_faces","object":1,"faces":[7,8],"remove_orphan_vertices":false})).unwrap();
    let bridged = document
        .execute(json!({"command":"bridge_loops","object":1,"loops":[first,second]}))
        .unwrap();
    assert_eq!(
        bridged["operation"]["created_faces"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(bridged["validation"]["boundary_edges"], 0);
}

#[test]
fn planar_dissolve_preserves_boundary_and_rejects_nonplanar_regions() {
    let mut document = Document::default();
    let original = document.execute(json!({"command":"save"})).unwrap();
    document
        .execute(json!({"command":"split_face","face":2,"vertices":[5,7]}))
        .unwrap();
    let dissolved = document
        .execute(json!({"command":"dissolve_region","object":1,"faces":[2,7]}))
        .unwrap();
    assert_eq!(dissolved["mesh"]["vertices"], original["mesh"]["vertices"]);
    assert_eq!(dissolved["mesh"]["faces"], original["mesh"]["faces"]);
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    assert!(
        document
            .execute(json!({"command":"dissolve_region","object":1,"faces":[1,2]}))
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
}

#[test]
fn prepared_preview_rejects_replacement_and_revision_changes() {
    let mut document = Document::default();
    let preview = json!({
        "command":"transaction","commit":false,
        "commands":[{"command":"object_move","object":1,"value":[1,0,0]}]
    });
    let first = document.execute(preview.clone()).unwrap();
    let second = document.execute(preview).unwrap();
    let commit = |preview: &serde_json::Value, revision| {
        json!({"command":"transaction","commit":true,"preview_id":preview["preview_id"],"revision":revision})
    };
    assert!(document.execute(commit(&first, 0)).is_err());
    assert!(document.execute(json!({
        "command":"transaction","commit":true,"preview_id":second["preview_id"],"revision":0,
        "guards":{"protected_objects":[1]}
    })).is_err());
    document.execute(json!({"command":"select_object","object":1})).unwrap();
    assert!(document.execute(commit(&second, 0)).is_err());
    assert!(document.execute(commit(&second, 1)).is_err());
    assert_eq!(document.revision(), 1);
}

#[test]
fn transactions_preview_guard_rollback_and_commit_as_one_undo() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let transaction = json!({"command":"transaction","commit":false,"camera":{"width":400,"height":300,"zoom":60},"commands":[{"command":"select_object","object":1},{"command":"move","value":[0.1,0,0]}],"guards":{"max_displacement":0.2,"no_new_components":true,"no_new_defects":true}});
    let preview = document.execute(transaction.clone()).unwrap();
    assert_eq!(
        preview["diff"]["moved_vertices"].as_array().unwrap().len(),
        8
    );
    assert!(preview["svg"].as_str().unwrap().contains("Amber"));
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    let mut guarded = transaction.clone();
    guarded["commit"] = json!(true);
    guarded["guards"]["max_displacement"] = json!(0.01);
    assert!(document.execute(guarded.clone()).is_err());
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
    guarded["guards"]["max_displacement"] = json!(0.2);
    document.execute(guarded).unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap()["mesh"],
        before["mesh"]
    );
}

#[test]
fn patch_grid_preserves_a_shared_boundary_and_previews_changes_without_editing() {
    let mut document = Document::default();
    let source = document.execute(json!({"command":"inspect_patch","object":1,"face":2})).unwrap();
    let corners: Vec<u64> = serde_json::from_value(source["boundary"].clone()).unwrap();
    for index in 0..corners.len() {
        let edge = [corners[index], corners[(index + 1) % corners.len()]];
        document.execute(json!({"command":"split_edge","object":1,"edge":edge})).unwrap();
    }
    document.execute(json!({"command":"region_define","name":"panel","object":1,"faces":[2]})).unwrap();
    let source = document.execute(json!({"command":"inspect_patch","object":1,"region":"panel"})).unwrap();
    let original = document.execute(json!({"command":"save"})).unwrap();
    let patch = json!({"command":"replace_patch","object":1,"region":"panel","boundary":source["boundary"],"grid_corners":corners});
    let mut invalid = patch.clone();
    invalid["boundary"].as_array_mut().unwrap().reverse();
    assert!(document.execute(invalid).is_err());
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), original);

    let mut transaction = json!({"command":"transaction","commit":false,"commands":[patch],"guards":{"protected_boundaries":["panel"],"no_new_defects":true},"preview":{"mode":"topology","clean":true,"width":120,"height":100,"yaw":0,"pitch":0}});
    let preview = document.execute(transaction.clone()).unwrap();
    assert_eq!(preview["face_layout"]["added_or_rewritten"]["quads"], 4);
    assert_eq!(preview["face_layout"]["removed_or_rewritten"]["ngons"], 1);
    assert_eq!(preview["boundary_changes"][0]["perimeter_unchanged"], true);
    assert_eq!(preview["boundary_changes"][0]["moved_vertices"], json!([]));
    assert!(preview["preview_views"]["after"]["png_base64"].is_string());
    assert_eq!(preview["preview_views"]["before"]["camera"], preview["preview_views"]["after"]["camera"]);
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), original);

    transaction["commit"] = json!(true);
    document.execute(transaction).unwrap();
    let after = document.execute(json!({"command":"inspect_patch","object":1,"region":"panel"})).unwrap();
    assert_eq!(source["boundary"], after["boundary"]);
    assert_eq!(source["boundary_positions"], after["boundary_positions"]);
    assert_eq!(after["faces"].as_array().unwrap().len(), 4);
    let mut movement = json!({"command":"transaction","commit":false,"commands":[{"command":"select_region","object":1,"region":"panel"},{"command":"move","object":1,"value":[0,0,0.01]}],"watch_boundaries":["panel"]});
    let moved = document.execute(movement.clone()).unwrap();
    assert_eq!(moved["boundary_changes"][0]["moved_vertices"].as_array().unwrap().len(), 8);
    movement["guards"] = json!({"protected_boundaries":["panel"]});
    assert!(document.execute(movement).is_err());
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), original);
}

#[test]
fn explicit_patch_can_change_depth_without_moving_neighbor_faces() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let patch = document.execute(json!({"command":"inspect_patch","object":1,"face":2})).unwrap();
    let vertices: Vec<[f64; 3]> = serde_json::from_value(patch["boundary_positions"].clone()).unwrap();
    let mut center: [f64; 3] = std::array::from_fn(|axis| vertices.iter().map(|p| p[axis]).sum::<f64>() / 4.0);
    center[2] += 0.2;
    let mut request = json!({"command":"replace_patch","object":1,"face":2,"boundary":patch["boundary"],"interior":[center],"polygons":[[0,1,4],[1,2,4],[2,3,4],[3,0,4]]});
    request["polygons"][0] = json!([1,0,4]);
    assert!(document.execute(request.clone()).is_err());
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    request["polygons"][0] = json!([0,1,4]);
    let result = document.execute(request).unwrap();
    assert_eq!(result["operation"]["boundary_vertices_moved"], 0);
    let after = document.execute(json!({"command":"save"})).unwrap();
    for (id, face) in before["mesh"]["faces"].as_object().unwrap() {
        if id != "2" {
            assert_eq!(&after["mesh"]["faces"][id], face);
        }
    }
}

#[test]
fn defects_are_precise_and_do_not_confuse_bounds_with_intersections() {
    let mut document = Document::default();
    let clean = document
        .execute(json!({"command":"inspect_defects","response":"full"}))
        .unwrap();
    assert_eq!(clean["total"], 0);
    document
        .execute(
            json!({"command":"object_create_box","name":"Overlap","min":[0,0,0],"max":[2,2,2]}),
        )
        .unwrap();
    let crossed = document
        .execute(json!({"command":"inspect_defects","response":"full","limit":256}))
        .unwrap();
    assert_eq!(
        crossed["geometry"]["inter_object_intersections"],
        "detected"
    );
    assert_eq!(crossed["topology"]["disconnected_components"], 2);
    let clearance = document
        .execute(json!({"command":"measure_clearance","a":{"object":1},"b":{"object":2}}))
        .unwrap();
    assert_eq!(clearance["distance"], 0.0);
    let volume=document.execute(json!({"command":"inspect_volume","object":1,"min":[0.9,-0.1,-0.1],"max":[1.1,0.1,0.1]})).unwrap();
    assert_eq!(volume["detected"], true);
    let partial = document
        .execute(json!({"command":"inspect_defects","max_pairs":1}))
        .unwrap();
    assert_eq!(partial["complete"], false);
}

#[test]
fn open_extrusions_quad_strips_lofts_and_bevels_build_connected_surfaces() {
    let mut document = Document::default();
    let ribbon=document.execute(json!({"command":"extrude_polygon","name":"Ribbon","closed_profile":false,"points":[[0,0],[1,0],[1,1]],"depth":0.2})).unwrap();
    assert_eq!(ribbon["operation"]["geometry_counts"]["faces"], 2);
    let strip=document.execute(json!({"command":"quad_strip","name":"Arch strip","contours":[[[3,0,0],[4,1,0],[5,0,0]],[[3,0.2,0],[4,1.2,0],[5,0.2,0]]]})).unwrap();
    assert_eq!(strip["operation"]["geometry_counts"]["faces"], 2);
    let loft=document.execute(json!({"command":"loft","name":"Loft","contours":[[[7,0,0],[8,0,0],[8,1,0],[7,1,0]],[[7,0,1],[8,0,1],[8,1,1],[7,1,1]]],"cap_start":true,"cap_end":true})).unwrap();
    assert_eq!(loft["operation"]["geometry_counts"]["faces"], 8);
    let beveled=document.execute(json!({"command":"bevel_edges","object":1,"edges":[[1,2]],"width":0.1,"segments":3,"corner_mode":"isolated"})).unwrap();
    assert_eq!(
        beveled["operation"]["created_faces"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(beveled["validation"]["non_manifold_edges"], 0);
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    assert!(document.execute(json!({"command":"bevel_edges","object":1,"edges":[[3,4],[4,8]],"width":0.1,"corner_mode":"isolated"})).is_err());
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap(),
        before
    );
}

#[test]
fn named_relationships_and_shared_parameters_reduce_id_bookkeeping() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"region_define","name":"front","object":1,"faces":[2]}))
        .unwrap();
    let mirrored=document.execute(json!({"command":"select_related","object":1,"region":"front","kind":"mirrored","axis":"Z"})).unwrap();
    assert_eq!(mirrored["operation"]["faces"], json!([1]));
    let adjacent = document
        .execute(json!({"command":"select_related","object":1,"faces":[2],"kind":"adjacent_faces"}))
        .unwrap();
    assert_eq!(adjacent["operation"]["faces"].as_array().unwrap().len(), 5);
    document.execute(json!({"command":"construction_define","name":"cab","parameters":{"width":2.0},"steps":[{"command":"extrude_polygon","object":1,"mode":"append","region":"cab_upper","points":[[0,0],[1,0],[1,1],[0,1]],"center":[0,4,0],"depth":{"parameter":"width","scale":0.5}}]})).unwrap();
    document
        .execute(json!({"command":"construction_apply","name":"cab"}))
        .unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let updated = document
        .execute(json!({"command":"construction_apply","name":"cab","parameters":{"width":3.0}}))
        .unwrap();
    assert_eq!(
        updated["mesh"]["faces"].as_object().unwrap().len(),
        before["mesh"]["faces"].as_object().unwrap().len()
    );
    assert_eq!(
        updated["operation"]["operations"][0]["mode"],
        "replace_region"
    );
    let selected = document
        .execute(json!({"command":"select_region","region":"cab_upper","object":1}))
        .unwrap();
    assert_eq!(
        selected["operation"]["vertices"].as_array().unwrap().len(),
        8
    );
    let stable = document.execute(json!({"command":"save"})).unwrap();
    assert!(
        document
            .execute(
                json!({"command":"construction_apply","name":"cab","parameters":{"width":-1.0}})
            )
            .is_err()
    );
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), stable);
}

#[test]
fn duplicate_and_region_allowances_are_explicit_and_containment_is_checked() {
    let mut document = Document::default();
    document
        .execute(json!({"command":"object_duplicate","object":1,"name":"Duplicate"}))
        .unwrap();
    let report = document
        .execute(json!({"command":"inspect_defects","response":"full","limit":256}))
        .unwrap();
    assert_eq!(report["geometry"]["duplicate_surfaces"], "detected");
    document
        .execute(
            json!({"command":"region_define","name":"original","object":1,"faces":[1,2,3,4,5,6]}),
        )
        .unwrap();
    document
        .execute(
            json!({"command":"region_define","name":"copy","object":2,"faces":[7,8,9,10,11,12]}),
        )
        .unwrap();
    document.execute(json!({"command":"intersection_allow","regions":["original","copy"],"reason":"Deliberate duplicated surface fixture"})).unwrap();
    let allowed = document
        .execute(json!({"command":"inspect_defects","response":"full","limit":256}))
        .unwrap();
    assert!(
        allowed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["intentional"] == true)
    );
    let saved = document.execute(json!({"command":"save"})).unwrap();
    let mut reopened = Document::default();
    reopened
        .execute(json!({"command":"load","project":saved}))
        .unwrap();
    assert_eq!(
        reopened
            .execute(json!({"command":"inspect_defects","response":"full","limit":256}))
            .unwrap()["items"],
        allowed["items"]
    );
    let contained=document.execute(json!({"command":"inspect_volume","object":1,"min":[-0.1,-0.1,-0.1],"max":[0.1,0.1,0.1]})).unwrap();
    assert_eq!(contained["detected"], true);
    assert_eq!(contained["findings"], json!([]));
    assert_eq!(contained["containment"][0]["object"], 1);
}

#[test]
fn transaction_reconstructs_temporarily_empty_object_and_guards_protected_geometry() {
    let mut document = Document::default();
    let before = document.execute(json!({"command":"save"})).unwrap();
    let transaction = json!({"command":"transaction","commit":true,"commands":[{"command":"delete_faces","object":1,"faces":[1,2,3,4,5,6],"remove_orphan_vertices":true},{"command":"object_add_box","object":1,"mode":"append","min":[-1,-1,-1],"max":[1,1,1]}]});
    document.execute(transaction.clone()).unwrap();
    document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    for guards in [
        json!({"protected_objects":[1]}),
        json!({"protected_volumes":[{"object":1,"min":[-0.1,-0.1,-0.1],"max":[0.1,0.1,0.1]}]}),
        json!({"max_displacment":0.0}),
        json!({"no_new_components":"true"}),
    ] {
        let mut guarded = transaction.clone();
        guarded["guards"] = guards;
        assert!(document.execute(guarded).is_err());
        assert_eq!(document.execute(json!({"command":"save"})).unwrap(), before);
    }
    let preserved = document
        .execute(
            json!({"command":"delete_faces","object":1,"faces":[2],"remove_orphan_vertices":false}),
        )
        .unwrap();
    assert_eq!(preserved["mesh"]["vertices"], before["mesh"]["vertices"]);
    assert!(
        document
            .execute(json!({"command":"delete_faces","object":1,"faces":[1]}))
            .unwrap_err()
            .contains("remove_orphan_vertices")
    );
}

#[test]
fn loft_reuses_existing_loops_and_quad_sweep_and_bevel_are_clean() {
    let mut document = Document::default();
    document.execute(json!({"command":"delete_faces","object":1,"faces":[1,2],"remove_orphan_vertices":false})).unwrap();
    let before = document.execute(json!({"command":"save"})).unwrap();
    // Rebuilding the existing side shell would duplicate surfaces; remove them first inside a transaction.
    document.execute(json!({"command":"transaction","commit":true,"commands":[{"command":"delete_faces","object":1,"faces":[3,4,5,6],"remove_orphan_vertices":false},{"command":"loft","object":1,"mode":"append","loops":[[1,2,3,4],[5,6,7,8]],"cap_start":true,"cap_end":true}]})).unwrap();
    assert_eq!(
        document.execute(json!({"command":"save"})).unwrap()["mesh"]["vertices"],
        before["mesh"]["vertices"]
    );
    let mut cube = Document::default();
    let beveled=cube.execute(json!({"command":"bevel_edges","object":1,"edges":[[1,2]],"width":0.2,"segments":5,"corner_mode":"isolated"})).unwrap();
    assert_eq!(beveled["validation"]["boundary_edges"], 0);
    let report = cube
        .execute(json!({"command":"inspect_defects","response":"full"}))
        .unwrap();
    assert_eq!(report["total"], 0, "{report}");
    let swept=cube.execute(json!({"command":"sweep","name":"Quad sweep","profile":[[-0.1,-0.1],[0.1,-0.1],[0.1,0.1],[-0.1,0.1]],"path":[[3,0,0],[3,1,0],[3,2,0]],"up":[1,0,0],"side_faces":"quads","cap_start":false,"cap_end":false})).unwrap();
    assert_eq!(swept["operation"]["created_faces"], 8);
}

#[test]
fn concave_boundary_fill_does_not_render_across_its_notch() {
    let mut document = Document::default();
    let created=document.execute(json!({"command":"extrude_polygon","name":"U panel","axis":"Z","points":[[0,0],[3,0],[3,3],[2,3],[2,1],[1,1],[1,3],[0,3]],"depth":0.2})).unwrap();
    let front = created["operation"]["front_vertices"].as_array().unwrap();
    let cap: Vec<_> = created["mesh"]["faces"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, face)| face.as_array().unwrap().iter().all(|v| front.contains(v)))
        .map(|(id, _)| id.parse::<u64>().unwrap())
        .collect();
    document
        .execute(
            json!({"command":"delete_faces","object":2,"faces":cap,"remove_orphan_vertices":false}),
        )
        .unwrap();
    document
        .execute(json!({"command":"create_face","object":2,"vertices":front}))
        .unwrap();
    let capture=document.execute(json!({"command":"capture","mode":"silhouette","solo_objects":[2],"width":400,"height":400,"yaw":0,"pitch":0,"zoom":80,"center":[1.5,1.5,0.1]})).unwrap();
    let encoded = atk_mesh::base64::decode(capture["png_base64"].as_str().unwrap()).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(encoded))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let channels = info.color_type.samples();
    let pixels_per_unit = capture["camera"]["pixels_per_unit"].as_f64().unwrap();
    let y = (200.0 - 0.5 * pixels_per_unit) as usize;
    let side_x = (200.0 - pixels_per_unit) as usize;
    assert!(
        pixels[(y * 400 + 200) * channels] > 240,
        "notch must remain background"
    );
    assert!(
        pixels[(y * 400 + side_x) * channels] < 20,
        "panel side must remain filled"
    );
}
