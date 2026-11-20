use gpt_mesh::Document;
use serde_json::json;
use std::path::Path;

pub fn write_cube_project(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    let project = Document::default().execute(json!({"command":"save"})).unwrap();
    std::fs::write(root.join("project.json"), project.to_string()).unwrap();
}
