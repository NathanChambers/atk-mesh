mod common;

use gpt_mesh::session::Session;
use serde_json::{Value, json};
use std::path::Path;

fn send(session: &mut Session, request: Value) -> Value {
    let reply = session.execute(request);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn allow_fixture_export(session: &mut Session) {
    for requirement in [
        "intent",
        "reference",
        "primary_forms",
        "views",
        "export_validation",
    ] {
        send(
            session,
            json!({"command":"workflow_waive","requirement":requirement,"reason":"Isolate exporter behavior in this fixture."}),
        );
    }
}

fn modify_glb(path: &Path, destination: &Path, edit: impl FnOnce(&mut Value)) {
    let bytes = std::fs::read(path).unwrap();
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let mut document: Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    edit(&mut document);
    let mut encoded = serde_json::to_vec(&document).unwrap();
    encoded.resize(encoded.len().next_multiple_of(4), b' ');
    let mut output = bytes[..12].to_vec();
    output.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    output.extend_from_slice(b"JSON");
    output.extend_from_slice(&encoded);
    output.extend_from_slice(&bytes[20 + length..]);
    let total = output.len() as u32;
    output[8..12].copy_from_slice(&total.to_le_bytes());
    std::fs::write(destination, output).unwrap();
}

#[test]
fn each_exporter_verifies_its_output_and_glb_respects_merge_and_visibility() {
    let root = std::env::temp_dir().join(format!("mesh-verify-formats-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_workspace(root.clone()).unwrap();
    allow_fixture_export(&mut session);
    let box_result = send(
        &mut session,
        json!({"command":"object_create_box","name":"Upper","min":[0,3,0],"max":[1,4,1]}),
    );
    let second = box_result["result"]["operation"]["object"]
        .as_u64()
        .unwrap_or(2);
    for (format, merge, filtered) in [
        ("glb", true, true),
        ("glb", true, false),
        ("fbx", true, false),
        ("obj", true, false),
        ("stl", false, false),
    ] {
        let mut options = json!({"format":format});
        if filtered {
            options["solo_objects"] = json!([second]);
        }
        if merge {
            options["merge"] = json!("all");
        }
        let name = format!("{format}-{filtered}");
        send(
            &mut session,
            json!({"command":"review_define","name":name,"view":{"width":64,"height":64},"wireframe":false,"defects":false,"export":options}),
        );
        let review = send(&mut session, json!({"command":"review_run","name":name}));
        let path = &review["result"]["artifacts"][format]["path"];
        let compared = send(
            &mut session,
            json!({"command":"export_verify","path":path,"export":options,"baseline":"source","response":"full"}),
        );
        let result = &compared["result"];
        assert_eq!(result["status"], "passed", "{format} {result}");
        assert_eq!(
            result["source"]["triangles"],
            if filtered { 12 } else { 24 }
        );
        assert_eq!(
            result["observed"]["triangles"],
            result["source"]["triangles"]
        );
        assert_eq!(result["source_revision"], review["result"]["revision"]);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn review_verifies_bytes_and_reuses_receipts_across_sessions() {
    let root = std::env::temp_dir().join(format!("mesh-verify-receipt-{}", std::process::id()));
    common::write_cube_project(&root);
    let mut session = Session::open_editor(root.clone(), false, false).unwrap();
    send(
        &mut session,
        json!({"command":"uv_project","mode":"box","object":1}),
    );
    send(
        &mut session,
        json!({"command":"texture_define","name":"swatch","width":1,"height":1,"rgba_base64":"/////w=="}),
    );
    send(
        &mut session,
        json!({"command":"texture_assign","texture":"swatch"}),
    );
    send(
        &mut session,
        json!({"command":"review_define","name":"shared","view":{"width":64,"height":64,"center":[0,0,0],"zoom":30},"standard_views":true,"export":{"format":"glb"},"defects":false}),
    );
    let review = send(
        &mut session,
        json!({"command":"review_run","name":"shared"}),
    );
    let result = &review["result"];
    let report_data =
        std::fs::read(result["artifacts"]["report"]["path"].as_str().unwrap()).unwrap();
    let report: Value = serde_json::from_slice(&report_data).unwrap();
    let receipt = &report["export_verification"];
    assert_eq!(receipt["status"], "passed", "{receipt}");
    assert_eq!(receipt["observed"]["triangles"], 12);
    assert_eq!(receipt["observed"]["geometry_reimport"]["status"], "passed");
    for name in ["front", "side", "top", "three_quarter", "intended"] {
        assert!(Path::new(result["artifacts"][name]["path"].as_str().unwrap()).is_file());
    }
    assert!(result["artifacts"]["glb"].is_null());
    allow_fixture_export(&mut session);
    let exported = send(&mut session, json!({"command":"export","format":"glb"}));
    let export = Path::new(
        exported["result"]["artifacts"]["glb"]["path"]
            .as_str()
            .unwrap(),
    );
    modify_glb(export, &root.join("changed.glb"), |document| {
        document["samplers"][0]["magFilter"] =
            json!(if document["samplers"][0]["magFilter"] == 9728 {
                9729
            } else {
                9728
            });
        document["meshes"][0]["primitives"][0]["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("TEXCOORD_0");
    });
    let verified = send(
        &mut session,
        json!({"command":"export_verify","path":"changed.glb"}),
    );
    assert_eq!(verified["result"]["status"], "passed");
    assert!(verified["result"]["source_revision"].is_null());
    let compared = send(
        &mut session,
        json!({"command":"export_verify","path":"changed.glb","baseline":result["artifacts"]["report"]["path"]}),
    );
    assert_eq!(compared["result"]["status"], "failed");
    for field in ["attributes", "samplers"] {
        assert!(
            compared["result"]["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding["check"] == field && finding["status"] == "failed")
        );
    }
    let registration = send(
        &mut session,
        json!({"command":"workflow_resource","kind":"evidence","capture":"report","description":"Verified test export","session":review["trace"]["session"],"event":review["trace"]["event"]}),
    );
    drop(session);
    let mut reopened = Session::open_editor(root.clone(), false, false).unwrap();
    let workflow = send(&mut reopened, json!({"command":"workflow_report"}));
    let report = std::fs::read_to_string(
        workflow["result"]["artifacts"]["markdown"]["path"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(report.contains(registration["result"]["id"].as_str().unwrap()));
    assert!(report.contains(receipt["artifact_hash"].as_str().unwrap()));
    assert!(report.contains("export_verify (failed)"));
    std::fs::write(
        root.join("sample.obj"),
        "o test\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )
    .unwrap();
    let obj = send(
        &mut reopened,
        json!({"command":"export_verify","path":"sample.obj","response":"full"}),
    );
    assert_eq!(
        obj["result"]["observed"]["pivots"]["status"], "format_limitation",
        "{obj}"
    );
    send(
        &mut reopened,
        json!({"command":"workflow_assess","requirement":"intent","assessment":"Final asset must be production-ready","evidence":[],"require_production_ready":true}),
    );
    for requirement in ["primary_forms", "reference", "views", "export_validation"] {
        send(
            &mut reopened,
            json!({"command":"workflow_waive","requirement":requirement,"reason":"Isolate the production stage check in this fixture."}),
        );
    }
    let completion = reopened.execute(json!({"command":"workflow_stage","stage":"complete"}));
    assert_eq!(
        completion["result"]["object_stages"][0]["stage"],
        "blockout"
    );
    assert_eq!(completion["result"]["requirement"], "production_ready");
    std::fs::remove_dir_all(root).unwrap();
}
