use gpt_mesh::session::Session;
use serde_json::json;

#[test]
fn help_exposes_embedded_guidance_with_exact_pages_and_no_model_changes() {
    let root = std::env::temp_dir().join(format!("mesh-help-test-{}", std::process::id()));
    let mut session = Session::new(root.clone()).unwrap();
    let initial = session.state();
    let index = session.execute(json!({"command":"help"}));
    assert_eq!(index["ok"], true);
    assert!(index["result"].get("topics").is_none());
    let catalog = session.execute(index["result"]["all_topics"].clone());
    assert_eq!(catalog["ok"], true);
    let topics = catalog["result"]["topics"].as_object().unwrap();
    assert_eq!(topics["workflow"], "Modeling workflow");
    assert_eq!(topics["command-api"], "Command API");
    assert_eq!(topics["reference-alignment"], "Reference alignment");
    let tasks = index["result"]["tasks"].as_object().unwrap();
    assert_eq!(tasks["export_assets"], json!(["export"]));
    assert_eq!(tasks["sculpt_geometry"][0], "local-sculpting");
    let sculpt = session.execute(json!({"command":"help","topic":"local-sculpting"}));
    let sculpt_alias = session.execute(json!({"command":"help","topic":"sculpt"}));
    assert_eq!(sculpt_alias["result"], sculpt["result"]);
    let sculpt_text = sculpt["result"]["text"].as_str().unwrap();
    for term in ["move_vertices", "sculpt_stroke", "disconnected_components"] {
        assert!(sculpt_text.contains(term));
    }
    for task in tasks.values() {
        for topic in task.as_array().unwrap() {
            assert!(topics.contains_key(topic.as_str().unwrap()), "{topic}");
        }
    }

    let mut offset = 0;
    let mut pages = Vec::new();
    loop {
        let page =
            session.execute(json!({"command":"help","topic":"workflow","offset":offset,"limit":3}));
        assert_eq!(page["ok"], true);
        assert_eq!(page["result"]["offset"], offset);
        pages.push(page["result"]["text"].as_str().unwrap().to_string());
        let Some(next) = page["result"]["next_offset"].as_u64() else {
            break;
        };
        assert!(next > offset);
        offset = next;
    }
    let guidance = pages.join("\n");
    let expected = include_str!("../docs/workflow.md")
        .trim()
        .lines()
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(guidance, expected);

    for topic in topics.keys() {
        let page = session.execute(json!({"command":"help","topic":topic,"limit":256}));
        assert_eq!(page["ok"], true, "{topic}");
        assert!(page["result"]["text"].is_string(), "{topic}");
        assert!(page["result"]["total"].as_u64().unwrap() > 0, "{topic}");
    }

    let commands = session.execute(json!({"command":"help","topic":"command-api","limit":256}));
    assert!(
        commands["result"]["text"]
            .as_str()
            .unwrap()
            .contains("`capture`")
    );
    let empty = session.execute(json!({"command":"help","topic":"workflow","offset":10000}));
    assert_eq!(empty["result"]["text"], "");
    assert!(empty["result"]["next_offset"].is_null());

    for request in [
        json!({"command":"help","all":"true"}),
        json!({"command":"help","topic":"missing"}),
        json!({"command":"help","topic":42}),
        json!({"command":"help","topic":"workflow","limit":0}),
        json!({"command":"help","topic":"workflow","limit":257}),
        json!({"command":"help","topic":"workflow","offset":-1}),
    ] {
        let failed = session.execute(request);
        assert_eq!(failed["ok"], false);
        assert!(failed["error"].is_string());
    }
    assert_eq!(session.state(), initial);
    let trace = &index["trace"];
    let record = session.read(trace["record"].as_str().unwrap()).unwrap();
    assert_eq!(record["summary"]["changed"], false);
    assert_eq!(record["input"]["command"], "help");
    std::fs::remove_dir_all(root).unwrap();
}
