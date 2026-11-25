mod common;

use gpt_mesh::session::Session;
use serde_json::{Value, json};

fn root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("mesh-workflow-{name}-{}", std::process::id()))
}

#[test]
fn measured_intent_survives_reopen_and_blocks_on_the_named_metric() {
    let root = root("measured-intent");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let accepted=session.execute(json!({"command":"workflow_assess","requirement":"intent","object":1,
        "assessment":"Closed cube fixture","evidence":[],"constraints":{"max_triangles":10,"closed_surfaces":true,"pivot":[0,0,0]}}));
    assert_eq!(accepted["ok"],true,"{accepted}");
    drop(session);
    let mut session=Session::open_workspace(root.clone()).unwrap();
    let blocked=session.execute(json!({"command":"workflow_stage","stage":"complete"}));
    assert_eq!(blocked["error"],"requirements_missing","{blocked}");
    let issues=blocked["result"]["requirements"].as_array().unwrap();
    assert!(issues.iter().any(|entry|entry["requirement"]=="max_triangles" && entry["check"]["measured"]==12 && entry["check"]["object"]==1));
    assert!(!issues.iter().any(|entry|entry["requirement"]=="closed_shell_integrity"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn projected_face_regions_survive_reload_and_track_later_uv_edits() {
    let mut doc=gpt_mesh::Document::default();
    doc.execute(json!({"command":"uv_project","object":1,"mode":"box","region":[0.1,0.1,0.4,0.4]})).unwrap();
    doc.execute(json!({"command":"uv_transform","faces":[1],"offset":[0.45,0]})).unwrap();
    let saved=doc.execute(json!({"command":"save"})).unwrap();
    let mut loaded=gpt_mesh::Document::default();
    loaded.execute(json!({"command":"load","project":saved})).unwrap();
    let report=loaded.execute(json!({"command":"inspect"})).unwrap();
    let outside=report["source_metrics"]["region_violations"].as_array().unwrap();
    assert_eq!(outside.len(),4);
    assert!(outside.iter().all(|issue|issue["face"]==1 && issue["region"]==json!([0.1,0.1,0.4,0.4])));
}

fn status(session: &mut Session, requirement: &str, object: Option<u64>) -> Value {
    let report = session.execute(json!({"command":"workflow_status"}));
    report["result"]["requirements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["requirement"] == requirement && record["object"].as_u64() == object)
        .unwrap()
        .clone()
}

#[test]
fn refined_requires_assembly_evidence_and_visible_feature_checks() {
    let root = root("refined-review");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let run = |session: &mut Session, request: Value| {
        let reply = session.execute(request);
        assert_eq!(reply["ok"], true, "{reply}");
        reply["result"].clone()
    };
    let mut evidence = Vec::new();
    for yaw in [0, 35] {
        let capture = session.execute(json!({"command":"capture","yaw":yaw,"width":120,"height":100}));
        evidence.push(run(&mut session, json!({
            "command":"workflow_resource","kind":"capture","description":"Fixture context",
            "session":capture["trace"]["session"],"event":capture["trace"]["event"]
        }))["id"].clone());
    }
    run(&mut session, json!({"command":"workflow_assess","requirement":"intent","assessment":"Static game prop viewed from above.","evidence":[]}));
    run(&mut session, json!({"command":"workflow_part_plan","plan":{"parts":[{"name":"Block","objects":[1],
        "silhouette_views":["side"],"profiles":[{"view":"side","basis":"inferred","outline":"Square","checkpoints":["Equal sides"]}]
    }],"relationships":[]}}));
    run(&mut session, json!({"command":"workflow_assess","requirement":"reference","basis":"shape_brief","assessment":"Use the square brief.","evidence":[]}));
    for requirement in ["primary_forms","views","placeholder_features","placeholder_assembly","placeholder_placement"] {
        run(&mut session, json!({"command":"workflow_assess","requirement":requirement,"outcome":"accepted","assessment":"Checked the square in context.","evidence":evidence}));
    }
    run(&mut session, json!({"command":"object_review_plan","object":1,"checks":[
        {"id":"form","area":"form","criterion":"Check proportions from both angles."},
        {"id":"interfaces","area":"interfaces","criterion":"Check for applicable mating parts."},
        {"id":"features","area":"required_features","criterion":"Confirm the top step is present."},
        {"id":"finish","area":"finish","criterion":"Check final shading."},
        {"id":"technical","area":"technical_fit","criterion":"Check final topology."}
    ]}));
    let check = |id, outcome, evidence: Value| json!({
        "command":"object_review_check","object":1,"check":id,"outcome":outcome,
        "evidence":evidence,"reason":"Checked the static fixture against the brief."
    });
    run(&mut session, check("form","pass",json!(evidence)));
    run(&mut session, check("interfaces","not_applicable",json!([])));
    run(&mut session, check("features","needs_work",json!(evidence)));
    let promote = json!({"command":"object_set_stage","object":1,"stage":"refined"});
    let blocked = session.execute(promote.clone());
    assert_eq!(blocked["error"], "requirements_missing");
    assert_eq!(blocked["result"]["object_checks"][0]["check"], "features");
    assert!(blocked["result"]["requirements"].as_array().unwrap().is_empty());
    assert_eq!(session.execute(check("features","pass",json!(evidence)))["ok"], true);
    assert_eq!(session.execute(promote)["ok"], true);
    assert_eq!(session.execute(json!({"command":"object_move","object":1,"value":[0.1,0,0]}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"query_object_review","object":1}))["result"]["stage"], "blockout");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn production_gate_requires_current_visual_checks_and_reopens_them_after_edits() {
    let root = root("production-review");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let approve = json!({"command":"object_set_stage","object":1,"stage":"production_ready","note":"Static display"});
    let query = json!({"command":"query_object_review","object":1});
    assert_eq!(session.execute(approve.clone())["ok"], false);
    let plan = session.execute(json!({"command":"object_review_plan","object":1,"checks":[
        {"id":"outline","area":"form","criterion":"Check the silhouette from the intended view."},
        {"id":"mount","area":"interfaces","criterion":"Check whether the part has a mounting interface."},
        {"id":"shading","area":"finish","criterion":"Check the visible highlights."},
        {"id":"budget","area":"technical_fit","criterion":"Review the topology report for the static prop."}
    ]}));
    assert_eq!(plan["ok"], true, "{plan}");
    let capture = session.execute(json!({"command":"capture","width":200,"height":150}));
    let image = session.execute(json!({
        "command":"workflow_resource","kind":"capture","description":"Static shape and finish",
        "session":capture["trace"]["session"],"event":capture["trace"]["event"]
    }));
    assert_eq!(image["ok"], true, "{image}");
    let preset = session.execute(json!({"command":"review_define","name":"static-check",
        "view":{"width":200,"height":150},"wireframe":false
    }));
    assert_eq!(preset["ok"], true, "{preset}");
    let review = session.execute(json!({"command":"review_run","name":"static-check"}));
    let report = session.execute(json!({
        "command":"workflow_resource","kind":"evidence","capture":"report","description":"Topology report",
        "session":review["trace"]["session"],"event":review["trace"]["event"]
    }));
    assert_eq!(report["ok"], true, "{report}");
    let check = |id: &str, outcome: &str, evidence: Value| json!({
        "command":"object_review_check","object":1,"check":id,"outcome":outcome,
        "reason":"Checked against the static display brief.","evidence":evidence
    });
    let evidence = json!([image["result"]["id"]]);
    let report_evidence = json!([report["result"]["id"]]);
    assert_eq!(session.execute(check("outline", "pass", json!([])))["ok"], false);
    assert_eq!(session.execute(check("outline", "pass", report_evidence.clone()))["ok"], false);
    assert_eq!(session.execute(check("mount", "not_applicable", json!([])))["ok"], true);
    assert_eq!(session.execute(check("outline", "pass", evidence.clone()))["ok"], true);
    assert_eq!(session.execute(check("shading", "needs_work", evidence.clone()))["ok"], true);
    assert_eq!(session.execute(check("budget", "pass", report_evidence))["ok"], true);
    assert_eq!(session.execute(approve.clone())["ok"], false);
    assert_eq!(session.execute(check("shading", "pass", evidence.clone()))["ok"], true);
    assert_eq!(session.execute(approve.clone())["ok"], true);
    assert_eq!(session.execute(query.clone())["result"]["stage"], "production_ready");
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(session.execute(query.clone())["result"]["stage"], "production_ready");
    let neighbor = session.execute(json!({
        "command":"object_create_box","name":"Neighbor","min":[2,0,0],"max":[3,1,1]
    }));
    assert_eq!(neighbor["ok"], true, "{neighbor}");
    let assembly_changed = session.execute(query.clone());
    assert_eq!(assembly_changed["result"]["stage"], "detailed");
    assert!(assembly_changed["result"]["checks"][0]["result"].is_null());
    assert_eq!(assembly_changed["result"]["checks"][1]["result"]["outcome"], "not_applicable");
    assert_eq!(session.execute(json!({"command":"undo"}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"shade_smooth","object":1,"angle":40}))["ok"], true);
    let reopened = session.execute(query.clone());
    assert_eq!(reopened["result"]["stage"], "detailed");
    assert!(reopened["result"]["checks"].as_array().unwrap().iter().all(|check| check["result"].is_null()));
    assert_eq!(session.execute(check("shading", "pass", evidence))["ok"], false);
    assert_eq!(session.execute(approve)["ok"], false);
    assert_eq!(session.execute(json!({"command":"undo"}))["ok"], true);
    assert_eq!(session.execute(query)["result"]["stage"], "production_ready");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn written_shape_brief_gates_primary_and_survives_binding_changes() {
    let root = root("written-brief");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let assessment = json!({"command":"workflow_assess","requirement":"reference","basis":"shape_brief","assessment":"Build from the written roof target.","evidence":[]});
    assert_eq!(session.execute(assessment.clone())["ok"], false);
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"intent","assessment":"Build a housing.","evidence":[]}))["ok"], true);
    let mut part = json!({"name":"Housing","silhouette_views":["side"],"profiles":[
        {"view":"side","outline":"Straight roof with a rising rear","basis":"inferred","checkpoints":["Rear is 10 mm taller"]}
    ]});
    let save = |part: &Value| json!({"command":"workflow_part_plan","plan":{"parts":[part],"relationships":[]}});
    assert_eq!(session.execute(save(&part))["ok"], true);
    assert_eq!(session.execute(assessment.clone())["ok"], true);
    assert_eq!(session.execute(json!({"command":"workflow_stage","stage":"primary"}))["ok"], true);
    part["objects"] = json!([1]);
    assert_eq!(session.execute(save(&part))["ok"], true);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(status(&mut session, "reference", None)["status"], "satisfied");
    let action = session.execute(json!({"command":"workflow_status"}))["result"]["next_action"].clone();
    assert_eq!(action["decision"], "review_primary_forms");
    assert_eq!(action["command"]["command"], "capture");
    assert_eq!(action["shape_brief"]["parts"][0]["profiles"], part["profiles"]);
    part["profiles"][0]["outline"] = json!("Flat roof");
    assert_eq!(session.execute(save(&part))["ok"], true);
    assert_eq!(status(&mut session, "reference", None)["status"], "pending");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn part_plan_resolves_current_collection_members_and_flags_deleted_bindings() {
    let root = root("part-plan");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let saved = session.execute(json!({"command":"workflow_part_plan","plan":{
        "parts":[{"name":"Housing","silhouette_views":["side","top"],"profiles":[
            {"view":"side","outline":"Roof rises toward the back","basis":"observed","checkpoints":["Rear roof is taller"]}
        ]},{"name":"Faceplate","silhouette_views":[]}],
        "relationships":[{"a":"Housing","b":"Faceplate","intent":"Fit the recessed seat","verification":"Check the seam in section"}]
    }}));
    assert_eq!(saved["ok"], true);
    assert_eq!(saved["result"]["parts"][0]["missing_profile_views"], json!(["top"]));
    let mut invalid = saved["result"].clone();
    invalid["parts"][0]["profiles"][0]["basis"] = json!("unknown");
    assert_eq!(session.execute(json!({"command":"workflow_part_plan","plan":invalid}))["ok"], false);
    assert_eq!(session.execute(json!({"command":"object_create_box","name":"Faceplate","min":[2,0,0],"max":[3,1,1]}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"collection_create","name":"Front","objects":[2]}))["ok"], true);
    let mut plan = saved["result"].clone();
    plan["parts"][0]["objects"] = json!([1]);
    plan["parts"][1]["collections"] = json!([1]);
    assert_eq!(session.execute(json!({"command":"workflow_part_plan","plan":plan}))["ok"], true);

    let mut session = Session::open_workspace(root.clone()).unwrap();
    let filtered = session.execute(json!({"command":"workflow_part_plan","object":2}));
    assert_eq!(filtered["result"], plan);
    assert_eq!(session.execute(json!({"command":"object_delete","object":1}))["ok"], true);
    let filtered = session.execute(json!({"command":"workflow_part_plan","collection":1}));
    assert_eq!(filtered["result"]["parts"][0]["missing_objects"], json!([1]));
    assert_eq!(session.execute(json!({"command":"workflow_part_plan","plan":plan}))["ok"], false);
    assert_eq!(session.execute(json!({"command":"workflow_part_plan"}))["result"], filtered["result"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn defaults_gate_export_and_detail_without_mutation_and_waivers_survive_restart() {
    let root = root("gates");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let initial = session.state();
    for request in [
        json!({"command":"export"}),
        json!({"command":"workflow_stage","stage":"detail"}),
    ] {
        let blocked = session.execute(request);
        assert_eq!(blocked["ok"], false);
        assert_eq!(blocked["result"]["status"], "requirements_missing");
        assert!(blocked["result"]["requirements"][0]["waive_command"].is_object());
        assert_eq!(session.state(), initial);
    }
    assert_eq!(
        session.execute(json!({"command":"workflow_waive","requirement":"reference","reason":" "}))
            ["ok"],
        false
    );
    assert_eq!(session.execute(json!({"command":"workflow_waive","requirement":"reference","reason":"REPLACE with why"}))["ok"], false);
    for requirement in [
        "intent",
        "reference",
        "primary_forms",
        "views",
    ] {
        assert_eq!(session.execute(json!({"command":"workflow_waive","requirement":requirement,"reason":"Procedural test fixture; deliberately exempt from visual modeling acceptance."}))["ok"], true);
    }
    let completed = session.execute(json!({"command":"workflow_stage","stage":"complete"}));
    assert_eq!(completed["result"]["stage"], "complete");
    assert_eq!(status(&mut session, "export_validation", None)["status"], "pending");
    let blocked = session.execute(json!({"command":"export"}));
    assert_eq!(blocked["ok"], false);
    assert_eq!(blocked["result"]["requirements"][0]["requirement"], "export_validation");
    assert_eq!(session.execute(json!({"command":"workflow_waive","requirement":"export_validation","reason":"Exercise the export gate with a fixture reviewed in the importer tests."}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"export"}))["ok"], true);
    let reopened = session.execute(json!({"command":"workflow_reopen","requirement":"views","reason":"Need a new thickness review."}));
    assert_eq!(reopened["ok"], true);
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(status(&mut session, "reference", None)["status"], "waived");
    assert_eq!(status(&mut session, "views", None)["status"], "pending");
    assert_eq!(session.execute(json!({"command":"export"}))["ok"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn resources_require_real_files_and_captures_invalidate_only_with_relevant_geometry() {
    let root = root("resources");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    session.execute(json!({"command":"select_object","object":1}));
    session.execute(
        json!({"command":"scale","value":[0.012000000000000006,0.016434393427494268,0.00016]}),
    );
    assert_eq!(session.execute(json!({"command":"workflow_resource","kind":"reference","path":"missing.png","description":"Missing reference"}))["ok"], false);
    assert_eq!(session.execute(json!({"command":"workflow_resource","kind":"capture","path":"project.json","description":"Not a capture"}))["ok"], false);
    let capture = session.execute(json!({"command":"capture","width":64,"height":64}));
    let source = capture["trace"].clone();
    let resource = session.execute(json!({"command":"workflow_resource","kind":"capture","session":source["session"],"event":source["event"],"object":1,"description":"Cube primary forms"}));
    assert_eq!(resource["ok"], true);
    let evidence = json!([resource["result"]["id"]]);
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"primary_forms","object":1,"outcome":"accepted","assessment":"Cube shape is appropriate for the test.","evidence":evidence}))["ok"], true);
    assert_eq!(
        status(&mut session, "primary_forms", None)["status"],
        "pending"
    );
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    assert_eq!(
        status(&mut session, "primary_forms", Some(1))["status"],
        "satisfied"
    );
    session.execute(json!({"command":"select","vertices":[1]}));
    assert_eq!(
        status(&mut session, "primary_forms", Some(1))["status"],
        "satisfied"
    );
    session
        .execute(json!({"command":"object_create_box","name":"Other","min":[3,3,3],"max":[4,4,4]}));
    assert_eq!(
        status(&mut session, "primary_forms", Some(1))["status"],
        "satisfied"
    );
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"views","assessment":"Duplicate camera is insufficient.","evidence":evidence}))["ok"], false);
    session.execute(json!({"command":"select_object","object":1}));
    session.execute(json!({"command":"move","value":[0,0.1,0],"object":1}));
    assert_eq!(
        status(&mut session, "primary_forms", Some(1))["status"],
        "pending"
    );
    assert_eq!(session.execute(json!({"command":"workflow_resource","kind":"capture","session":source["session"],"event":source["event"],"object":1,"description":"Old capture"}))["ok"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reference_comparison_is_persisted_and_recorded_as_read_only_evidence() {
    let root = root("comparison");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let capture = session.execute(json!({"command":"capture","width":64,"height":64}));
    let path = capture["result"]["artifacts"]["png"]["path"]
        .as_str()
        .unwrap();
    std::fs::copy(path, root.join("reference.png")).unwrap();
    let resource = session.execute(json!({"command":"workflow_resource","kind":"reference","path":"reference.png","description":"Synthetic reference fixture","view":{"width":64,"height":64,"yaw":0,"pitch":0,"crop":[0,0,64,64]}}));
    let id = resource["result"]["id"].clone();
    let state = session.state();
    let comparison = session.execute(json!({"command":"compare_reference","reference":id}));
    assert_eq!(comparison["ok"], true);
    assert_eq!(comparison["result"]["width"], 128);
    assert_eq!(comparison["result"]["height"], 64);
    assert!(
        std::path::Path::new(
            comparison["result"]["artifacts"]["png"]["path"]
                .as_str()
                .unwrap()
        )
        .is_file()
    );
    assert_eq!(session.state(), state);
    let invalid =
        session.execute(json!({"command":"compare_reference","reference":id,"crop":[0,0,999,64]}));
    assert_eq!(invalid["ok"], false);
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"reference","assessment":"Approved synthetic fixture.","evidence":[id]}))["ok"], true);
    let mut views = vec![];
    for (yaw, zoom) in [(0, 100), (360, 120), (45, 100)] {
        let capture = session.execute(
            json!({"command":"capture","width":64,"height":64,"yaw":yaw,"pitch":0,"zoom":zoom}),
        );
        let resource = session.execute(json!({"command":"workflow_resource","kind":"capture","session":capture["trace"]["session"],"event":capture["trace"]["event"],"description":"Distinct view test"}));
        assert_eq!(resource["ok"], true);
        views.push(resource["result"]["id"].clone());
    }
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"views","assessment":"Same direction with different zoom is insufficient.","evidence":views[..2]}))["ok"], false);
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"views","assessment":"Distinct front and oblique views reviewed.","evidence":[views[0],views[2]]}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"workflow_assess","requirement":"intent","assessment":"Changed target now requires different dimensions.","evidence":[]}))["ok"], true);
    assert_eq!(status(&mut session, "views", None)["status"], "pending");
    let primary = session.execute(json!({"command":"workflow_stage","stage":"primary"}));
    assert_eq!(primary["ok"], true);
    let report = session.execute(json!({"command":"workflow_status"}));
    assert_eq!(report["result"]["next_action"]["command"], json!({"command":"compare_reference","reference":id}));
    let blocked = session.execute(json!({"command":"workflow_stage","stage":"detail"}));
    assert_eq!(blocked["result"]["next_action"], report["result"]["next_action"]);
    std::fs::write(root.join("reference.png"), b"changed").unwrap();
    assert_eq!(status(&mut session, "reference", None)["status"], "pending");
    let report = session.execute(json!({"command":"workflow_status"}));
    assert_eq!(report["result"]["next_action"]["decision"], "prepare_references");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn primary_review_needs_explicit_acceptance_and_keeps_revision_findings_after_edits() {
    let root = root("primary-outcome");
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    let blockout_status = session.execute(json!({"command":"workflow_status"}));
    let blockout_stage = session.execute(json!({"command":"workflow_stage","stage":"blockout"}));
    let blockout_review = &blockout_status["result"]["instancing_review"];
    assert_eq!(blockout_review["instance_command"]["command"], "object_instance");
    assert_eq!(blockout_review, &blockout_stage["result"]["instancing_review"]);
    for requirement in ["intent", "reference", "views", "export_validation"] {
        let waiver = session.execute(json!({"command":"workflow_waive","requirement":requirement,"reason":"Isolate primary review for this test."}));
        assert_eq!(waiver["ok"], true);
    }
    assert_eq!(session.execute(json!({"command":"workflow_stage","stage":"primary"}))["ok"], true);
    let capture = |session: &mut Session| {
        let view = session.execute(json!({"command":"capture","width":64,"height":64}));
        let resource = session.execute(json!({"command":"workflow_resource","kind":"capture","session":view["trace"]["session"],"event":view["trace"]["event"],"description":"Primary form view"}));
        assert_eq!(resource["ok"], true);
        resource["result"]["id"].clone()
    };
    let original = capture(&mut session);
    let mut review = json!({"command":"workflow_assess","requirement":"primary_forms","assessment":"The cube is too tall. Recheck its front silhouette.","evidence":[original]});
    assert_eq!(session.execute(review.clone())["ok"], false);
    review["outcome"] = json!("needs_revision");
    assert_eq!(session.execute(review.clone())["ok"], true);
    assert_eq!(session.execute(json!({"command":"select_object","object":1}))["ok"], true);
    assert_eq!(session.execute(json!({"command":"scale","value":[1,0.75,1]}))["ok"], true);
    drop(session);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    for request in [
        json!({"command":"workflow_stage","stage":"detail"}),
        json!({"command":"workflow_stage","stage":"complete"}),
        json!({"command":"export"}),
    ] {
        let reply = session.execute(request);
        assert_eq!(reply["ok"], false);
        let action = &reply["result"]["next_action"];
        assert_eq!(action["decision"], "revise_primary_forms");
        assert_eq!(action["findings"], review["assessment"]);
        assert_eq!(action["previous_evidence"], review["evidence"]);
    }
    review["outcome"] = json!("accepted");
    review["assessment"] = json!("The front silhouette now has the intended height.");
    assert_eq!(session.execute(review.clone())["ok"], false);
    review["evidence"] = json!([capture(&mut session)]);
    assert_eq!(session.execute(review)["ok"], true);
    let status = session.execute(json!({"command":"workflow_status"}));
    let detail = session.execute(json!({"command":"workflow_stage","stage":"detail"}));
    assert_eq!(detail["ok"], true);
    let review = &detail["result"]["instancing_review"];
    assert_eq!(review, &status["result"]["instancing_review"]);
    assert_eq!(review["advisory"], true);
    assert_eq!(review["instance_command"]["command"], "object_instance");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_workflow_is_not_silently_reset() {
    let root = root("corrupt");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("workflow.json"), "{invalid").unwrap();
    assert!(Session::open_workspace(root.clone()).is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("workflow.json")).unwrap(),
        "{invalid"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn taper_preserves_ids_and_rejects_invalid_profiles_atomically() {
    let mut document = gpt_mesh::Document::default();
    document
        .execute(json!({"command":"select_object","object":1}))
        .unwrap();
    let before = document.execute(json!({"command":"inspect"})).unwrap();
    let tapered = document.execute(json!({"command":"taper","axis":"X","center":[0,0,0],"profile":[[-1,1,1],[1,1,0.5]],"object":1})).unwrap();
    assert_eq!(before["mesh"]["faces"], tapered["mesh"]["faces"]);
    for (id, point) in before["mesh"]["vertices"].as_object().unwrap() {
        let x = point[0].as_f64().unwrap();
        let z = point[2].as_f64().unwrap();
        let expected = z * (1.0 - (x + 1.0) * 0.25);
        assert!((tapered["mesh"]["vertices"][id][2].as_f64().unwrap() - expected).abs() < 1e-12);
    }
    assert!(
        document
            .execute(json!({"command":"taper","profile":[[0,1,1],[0,1,-1]]}))
            .is_err()
    );
    assert_eq!(
        document.execute(json!({"command":"inspect"})).unwrap()["mesh"],
        tapered["mesh"]
    );
    let undo = document.execute(json!({"command":"undo"})).unwrap();
    assert_eq!(undo["mesh"], before["mesh"]);
}
