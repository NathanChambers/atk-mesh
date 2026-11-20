use gpt_mesh::{responses, session::Session};
use serde_json::{Value, json};

#[test]
fn large_selections_are_bounded_and_exact_mesh_and_record_pages_are_read_only() {
    let root =
        std::env::temp_dir().join(format!("mesh-bounded-response-test-{}", std::process::id()));
    let mut session = Session::new(root.clone()).unwrap();
    let reply = session.execute(json!({"command":"revolve","name":"Dense solid","profile":[[0,0],[1,0],[1,1],[0,1]],"segments":128}));
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"]["selection"]["count"], 258);
    assert_eq!(
        reply["result"]["selection"]["sample"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(reply["result"]["selection"]["path"], "/mesh/selection");
    assert!(serde_json::to_vec(&reply).unwrap().len() < 2000);
    let before = session.state();
    let expected = before["mesh"]["selection"].as_array().unwrap();
    let mut actual = vec![];
    let mut offset = 0;
    loop {
        let page = session.execute(json!({"command":"query_mesh","kind":"selection","object":2,"limit":17,"offset":offset}));
        assert_eq!(page["ok"], true);
        actual.extend(page["result"]["items"].as_array().unwrap().iter().cloned());
        let Some(next) = page["result"]["next_offset"].as_u64() else {
            break;
        };
        offset = next;
    }
    assert_eq!(actual, *expected);
    assert_eq!(session.state(), before);
    let trace = &reply["trace"];
    let page = session.execute(json!({"command":"query_record","session":trace["session"],"event":trace["event"],"path":reply["result"]["selection"]["path"],"offset":10,"limit":17}));
    assert_eq!(page["result"]["items"], json!(&expected[10..27]));
    assert_eq!(page["result"]["source_revision"], before["revision"]);
    let record = session.read(trace["record"].as_str().unwrap()).unwrap();
    let persisted_before: Value =
        serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
    assert_eq!(
        record["full_output"]["result"]["mesh"],
        persisted_before["mesh"]
    );
    let ids: Vec<_> = expected[..3].to_vec();
    let page =
        session.execute(json!({"command":"query_mesh","kind":"vertices","object":2,"ids":ids}));
    for record in page["result"]["items"].as_array().unwrap() {
        let id = record["id"].as_u64().unwrap().to_string();
        assert_eq!(record["position"], before["mesh"]["vertices"][&id]);
    }
    assert_eq!(session.state(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn artifacts_are_saved_and_recorded_without_payloads_in_default_replies() {
    let root = std::env::temp_dir().join(format!(
        "mesh-artifact-response-test-{}",
        std::process::id()
    ));
    let mut session = Session::new(root.clone()).unwrap();
    let before = session.state();
    let capture = session.execute(json!({"command":"capture","width":128,"height":128}));
    assert_eq!(capture["ok"], true);
    assert!(capture["result"].get("png_base64").is_none());
    let png = &capture["result"]["artifacts"]["png"];
    let bytes = std::fs::read(png["path"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(bytes.len() as u64, png["bytes"].as_u64().unwrap());
    let record = session
        .read(capture["trace"]["record"].as_str().unwrap())
        .unwrap();
    let full_png = record["full_output"]["result"]["png_base64"]
        .as_str()
        .unwrap();
    assert_eq!(bytes, gpt_mesh::base64::decode(full_png).unwrap());
    let svg = &capture["result"]["artifacts"]["svg"]["path"];
    assert_eq!(
        std::fs::read_to_string(svg.as_str().unwrap()).unwrap(),
        record["full_output"]["result"]["svg"]
    );
    assert_eq!(session.state(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_options_do_not_mutate_and_deep_mappings_are_bounded_with_retrieval_paths() {
    let root =
        std::env::temp_dir().join(format!("mesh-invalid-response-test-{}", std::process::id()));
    let mut session = Session::new(root.clone()).unwrap();
    let before = session.state();
    for request in [
        json!({"command":"select","vertices":[1],"response":"typo"}),
        json!({"command":"query_mesh","kind":"vertices","limit":0}),
        json!({"command":"query_mesh","kind":"vertices","ids":[999]}),
        json!({"command":"query_mesh","kind":"objects","object":1}),
        json!({"command":"query_record","session":"../..","event":1,"path":""}),
    ] {
        assert_eq!(session.execute(request)["ok"], false);
        assert_eq!(session.state(), before);
    }
    let mapping: serde_json::Map<String, Value> = (0..10000)
        .map(|id| (id.to_string(), json!([id, id + 1, id + 2])))
        .collect();
    let result = responses::compact(
        json!({"operation":{"mapping":mapping},"revision":1,"validation":{"boundary_edges":0}}),
    );
    assert_eq!(result["operation"]["mapping"]["count"], 10000);
    assert_eq!(result["operation"]["mapping"]["path"], "/operation/mapping");
    assert_eq!(result["revision"], 1);
    assert!(serde_json::to_vec(&result).unwrap().len() < 2000);
    std::fs::remove_dir_all(root).unwrap();
}
