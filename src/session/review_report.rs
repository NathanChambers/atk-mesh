use super::*;
use std::fmt::Write as _;

impl Session {
    pub(super) fn workflow_markdown(&self) -> Result<Value, String> {
        let state = self
            .workflow
            .report(&self.state["mesh"], self.workflow_directory());
        let mut markdown = format!(
            "# Mesh workflow report\n\nRevision {}. Workflow stage {}.\n",
            self.document.revision(),
            state["stage"].as_str().unwrap_or("blockout")
        );
        markdown.push_str("\n## Assessments and remaining checks\n\n");
        for record in state["requirements"].as_array().into_iter().flatten() {
            let requirement = record["requirement"].as_str().unwrap_or("");
            let status = record["status"].as_str().unwrap_or("pending");
            let label = if status == "satisfied" {
                "Recorded judgment"
            } else {
                "Remaining check"
            };
            writeln!(
                markdown,
                "- {label} — {requirement} ({status}). {} Evidence {}.",
                record["assessment"].as_str().unwrap_or(""),
                record["evidence"]
            )
            .unwrap();
        }
        markdown.push_str("\n## Registered evidence\n\n");
        if let Some(resources) = state["resources"].as_object() {
            for (id, resource) in resources {
                writeln!(
                    markdown,
                    "- {id} — {}. `{}` ({})",
                    resource["description"].as_str().unwrap_or(""),
                    resource["path"].as_str().unwrap_or(""),
                    resource["content_hash"].as_str().unwrap_or("")
                )
                .unwrap();
            }
        }
        markdown.push_str("\n## Exports, verification and invalidations\n\n");
        let events = self.read("/sessions/workspace")?;
        let mut entries = 0;
        for summary in events["events"].as_array().into_iter().flatten() {
            let command = summary["command"].as_str().unwrap_or("");
            if !matches!(command, "review_run" | "export_verify" | "export")
                && summary["invalidated_review"].is_null()
            {
                continue;
            }
            let session = summary["session"].as_str().ok_or("missing session ID")?;
            let event = summary["source_event"].as_u64().ok_or("missing event ID")?;
            let record = self.read(&format!("/sessions/{session}/events/{event}"))?;
            let result = &record["output"]["result"];
            let receipt = if command == "review_run" {
                &record["full_output"]["result"]["export_verification"]
            } else {
                &record["full_output"]["result"]
            };
            let status = receipt["status"]
                .as_str()
                .unwrap_or(if record["output"]["ok"] == true {
                    "recorded"
                } else {
                    "failed"
                });
            writeln!(
                markdown,
                "- Measured result — {command} ({status}), event `{session}/{event}`, revision {}.",
                summary["revision_after"]
            )
            .unwrap();
            if let Some(hash) = receipt["artifact_hash"].as_str() {
                writeln!(markdown, "  - Artifact hash `{hash}`.").unwrap();
                if let Some(triangles) = receipt["observed"]["triangles"].as_u64() {
                    writeln!(
                        markdown,
                        "  - Exported {triangles} triangles, {} bytes.",
                        receipt["bytes"]
                    )
                    .unwrap();
                }
                for finding in receipt["findings"].as_array().into_iter().flatten() {
                    writeln!(
                        markdown,
                        "  - {} ({}).",
                        finding["check"].as_str().unwrap_or(""),
                        finding["status"].as_str().unwrap_or("")
                    )
                    .unwrap();
                }
            }
            if let Some(artifacts) = result["artifacts"].as_object() {
                for (kind, file) in artifacts {
                    writeln!(
                        markdown,
                        "  - {kind} `{}`.",
                        file["path"].as_str().unwrap_or("")
                    )
                    .unwrap();
                }
            }
            if !summary["invalidated_review"].is_null() {
                writeln!(
                    markdown,
                    "  - Invalidated review {}.",
                    summary["invalidated_review"]
                )
                .unwrap();
            }
            entries += 1;
        }
        if entries == 0 {
            markdown.push_str("No export or verification events are recorded.\n");
        }
        let mut files = vec![];
        for directory in [
            self.workflow_directory().to_path_buf(),
            self.workflow_directory().join("exports"),
        ] {
            if !directory.is_dir() {
                continue;
            }
            for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let path = entry.path();
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_file()
                    && matches!(extension.as_str(), "glb" | "obj" | "fbx" | "stl")
                {
                    files.push((
                        path,
                        entry.metadata().map_err(|error| error.to_string())?.len(),
                    ));
                }
            }
        }
        files.sort();
        if !files.is_empty() {
            markdown.push_str("\n## Export files present in the workspace\n\n");
            for (path, bytes) in files {
                let name = path
                    .strip_prefix(self.workflow_directory())
                    .map_err(|error| error.to_string())?;
                writeln!(
                    markdown,
                    "- Measured file — `{}`, {bytes} bytes.",
                    name.display()
                )
                .unwrap();
            }
        }
        markdown.push_str("\n## Submitted feedback\n\n");
        match crate::feedback::for_workspace(self.workflow_directory()) {
            Ok(feedback) if feedback.is_empty() => {
                markdown.push_str("No workspace-linked submissions.\n")
            }
            Ok(feedback) => {
                for item in feedback {
                    writeln!(
                        markdown,
                        "- {} (`{}`).",
                        item["summary"].as_str().unwrap_or(""),
                        item["id"].as_str().unwrap_or("")
                    )
                    .unwrap();
                }
            }
            Err(_) => markdown.push_str("Feedback lookup unavailable.\n"),
        }
        markdown.push_str("\n## Object stages\n\n");
        for (id, object) in &self.document.mesh.objects {
            writeln!(
                markdown,
                "- {} ({id}) — {}.",
                object.name,
                json!(object.stage).as_str().unwrap_or("blockout")
            )
            .unwrap();
        }
        Ok(json!({"revision":self.document.revision(),"markdown":markdown}))
    }
}
