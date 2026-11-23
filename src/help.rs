use serde_json::{Value, json};
use std::collections::BTreeMap;

fn topics() -> BTreeMap<String, (&'static str, &'static str)> {
    let mut topics = BTreeMap::from([
        (
            "workflow".into(),
            ("Modeling workflow", include_str!("../docs/workflow.md")),
        ),
        (
            "project-integration".into(),
            (
                "Project integration",
                include_str!("../docs/project-integration.md"),
            ),
        ),
    ]);
    let reference = include_str!("../docs/command-reference.md");
    let mut headings = Vec::new();
    let mut offset = 0;
    for line in reference.split_inclusive('\n') {
        let heading = line
            .strip_prefix("## ")
            .or_else(|| line.strip_prefix("### "));
        if let Some(title) = heading {
            headings.push((offset, offset + line.len(), title.trim()));
        }
        offset += line.len();
    }
    for (index, (_, start, title)) in headings.iter().enumerate() {
        let end = headings
            .get(index + 1)
            .map_or(reference.len(), |heading| heading.0);
        let mut words = Vec::new();
        for word in title.split_whitespace() {
            let word = word.trim_matches(|character: char| !character.is_alphanumeric());
            words.push(word.to_ascii_lowercase());
        }
        let key = words.join("-");
        topics.insert(key, (*title, reference[*start..end].trim()));
    }
    topics
}

pub(super) fn execute(request: &Value) -> Result<Value, String> {
    let Some(topic) = request.get("topic") else {
        let all = request
            .get("all")
            .map(|value| value.as_bool().ok_or("all must be a boolean"))
            .transpose()?;
        if all == Some(true) {
            let titles: BTreeMap<_, _> = topics()
                .into_iter()
                .map(|(key, (title, _))| (key, title))
                .collect();
            return Ok(json!({"topics":titles}));
        }

        return Ok(json!({
            "tasks":{
                "get_started":["minimal-example","project-integration"],
                "model_geometry":["workflow","command-api","batch-commands"],
                "sculpt_geometry":["local-sculpting","bounded-surface-cleanup"],
                "inspect_and_review":["object-detail-stages","capture-cameras","validation-render-modes","saved-reference-reviews"],
                "plan_repeated_parts":["linked-mesh-instances"],
                "repair_local_geometry":["boundary-preserving-patch-edits","repair-transactions"],
                "check_moving_parts":["junction-review","independent-defect-diagnostics"],
                "edit_materials":["uv-editing-and-vertex-painting","layered-textures-and-material-previews"],
                "rig_and_animate":["rigging-and-animation"],
                "export_assets":["export"],
                "report_cli_feedback":["cli-feedback"]
            },
            "read_topic":{"command":"help","topic":"minimal-example"},
            "all_topics":{"command":"help","all":true},
            "pagination":"Continue with next_offset until null."
        }));
    };
    let topic = topic
        .as_str()
        .ok_or("topic must be a string")?;
    let topic = if topic == "sculpt" { "local-sculpting" } else { topic };
    let topics = topics();
    let (title, text) = topics
        .get(topic)
        .ok_or("unknown help topic; use help with all:true to list topics")?;
    let (offset, limit) = crate::responses::page(request)?;
    let lines: Vec<_> = text.lines().collect();
    let page: Vec<_> = lines.iter().skip(offset).take(limit).copied().collect();
    let next = offset.saturating_add(limit);
    Ok(
        json!({"topic":topic,"title":title,"text":page.join("\n"),"total":lines.len(),"offset":offset,"next_offset":if next < lines.len() {Some(next)} else {None}}),
    )
}
