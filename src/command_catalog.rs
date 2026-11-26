use serde_json::{Value, json};
use std::sync::OnceLock;

pub fn cli_tools() -> &'static [Value] {
    static TOOLS: OnceLock<Vec<Value>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        let mut tools = legacy_tools();
        tools.extend(crate::feedback::tools());
        tools.push(crate::help::tool());
        tools.sort_unstable_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        tools
    })
}

pub fn editor_tools() -> &'static [Value] {
    static TOOLS: OnceLock<Vec<Value>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        cli_tools().iter().filter(|tool| {
            let name = tool["name"].as_str().expect("mesh tool name");
            !crate::feedback::is_name(name) && !matches!(name, "reference_guides" | "visual_hull" | "replay")
        }).cloned().collect()
    })
}

fn legacy_tools() -> Vec<Value> {
    let mut catalog: Value = serde_json::from_str(include_str!("mcp_tools.json")).expect("valid mesh command catalog");
    catalog["result"]["tools"].as_array_mut().map(std::mem::take).expect("mesh command list")
}

pub(crate) fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({"name":name,"description":description,"inputSchema":schema})
}
