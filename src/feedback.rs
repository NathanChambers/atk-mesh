use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const KINDS: [&str; 4] = [
    "missing_capability",
    "command_issue",
    "documentation",
    "workflow",
];

pub fn is_command(request: &Value) -> bool {
    matches!(
        request["command"].as_str(),
        Some("feedback_submit" | "feedback_list")
    )
}

pub fn run(request: &Value) -> Result<Value, String> {
    let directory = feedback_directory()?;
    if request["command"] == "feedback_submit" {
        submit(&directory, request)
    } else {
        list(&directory, request)
    }
}

fn feedback_directory() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("MESH_FEEDBACK_DIR") {
        let directory = PathBuf::from(value);
        if !directory.is_absolute() {
            return Err("MESH_FEEDBACK_DIR must be an absolute directory path".into());
        }
        return Ok(directory);
    }

    #[cfg(target_os = "windows")]
    let directory = environment_directory("LOCALAPPDATA").map(|path| path.join("MeshWorkbench"));
    #[cfg(target_os = "macos")]
    let directory = environment_directory("HOME")
        .map(|path| path.join("Library/Application Support/MeshWorkbench"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let directory = environment_directory("XDG_DATA_HOME")
        .or_else(|| environment_directory("HOME").map(|path| path.join(".local/share")))
        .map(|path| path.join("mesh-workbench"));

    directory
        .map(|path| path.join("feedback"))
        .ok_or_else(|| "could not resolve the feedback directory; set MESH_FEEDBACK_DIR to an absolute directory path".into())
}

fn environment_directory(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os(name)?);
    path.is_absolute().then_some(path)
}

fn submit(directory: &Path, request: &Value) -> Result<Value, String> {
    let kind = read_kind(request)?.ok_or("kind is required")?;
    let mut report =
        json!({"schema_version":1,"kind":kind,"cli_version":env!("CARGO_PKG_VERSION")});
    if let Some(value) = request.get("workspace") {
        let path = fs::canonicalize(value.as_str().ok_or("workspace must be a directory path")?).map_err(|error|error.to_string())?;
        if !path.is_dir() { return Err("workspace must be a directory".into()); }
        report["workspace"] = json!(path);
    }
    for (field, maximum, required) in [
        ("summary", 240, true),
        ("goal", 2_000, true),
        ("observed", 8_000, true),
        ("suggestion", 4_000, false),
        ("related", 1_000, false),
    ] {
        let Some(value) = request.get(field) else {
            if required {
                return Err(format!("{field} is required"));
            }
            continue;
        };
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty() && text.chars().count() <= maximum)
            .ok_or_else(|| format!("{field} must be a string of 1..{maximum} characters"))?;
        report[field] = json!(text);
    }

    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    let id = format!("{:029}-{:010}", created.as_nanos(), std::process::id());
    report["id"] = json!(id);
    report["created_at_unix_ms"] = json!(created.as_millis() as u64);
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;

    write_report(directory, &id, &bytes).map_err(|error| {
        format!(
            "could not save feedback in {}: {error}",
            directory.display()
        )
    })?;
    Ok(json!({"id":id,"directory":directory.display().to_string()}))
}

fn write_report(directory: &Path, id: &str, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let pending = directory.join(format!("{id}.tmp"));
    let destination = directory.join(format!("{id}.json"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);

    let published = written.and_then(|()| {
        if destination.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "feedback ID already exists",
            ));
        }
        fs::rename(&pending, &destination)
    });
    if published.is_err() {
        let _ = fs::remove_file(&pending);
    }
    published
}

fn list(directory: &Path, request: &Value) -> Result<Value, String> {
    let kind = read_kind(request)?;
    let limit = request
        .get("limit")
        .map(|value| {
            value
                .as_u64()
                .filter(|limit| (1..=256).contains(limit))
                .ok_or("limit must be an integer from 1 to 256")
        })
        .transpose()?
        .unwrap_or(32) as usize;
    let before = request
        .get("before")
        .map(|value| {
            value
                .as_str()
                .filter(|id| valid_id(id))
                .ok_or("before must be a feedback ID")
        })
        .transpose()?;

    let (reports, next_before) = read_reports(directory, kind, limit, before).map_err(|error| {
        format!(
            "could not list feedback in {}: {error}",
            directory.display()
        )
    })?;
    Ok(
        json!({"directory":directory.display().to_string(),"reports":reports,"next_before":next_before}),
    )
}

pub(crate) fn for_workspace(workspace: &Path) -> Result<Vec<Value>,String> {
    let directory = feedback_directory()?;
    let (reports,_) = read_reports(&directory,None,usize::MAX,None).map_err(|e|e.to_string())?;
    let workspace=json!(workspace);
    Ok(reports.into_iter().filter(|report|report["workspace"]==workspace).map(|report|json!({"id":report["id"],"summary":report["summary"]})).collect())
}

fn read_reports(
    directory: &Path,
    kind: Option<&str>,
    limit: usize,
    before: Option<&str>,
) -> io::Result<(Vec<Value>, Option<String>)> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), None)),
        Err(error) => return Err(error),
    };

    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
            continue;
        };
        if valid_id(id) && before.is_none_or(|before| id < before) && entry.file_type()?.is_file() {
            ids.push(id.to_owned());
        }
    }
    ids.sort_unstable_by(|left, right| right.cmp(left));

    let mut reports: Vec<Value> = Vec::new();
    for id in ids {
        let path = directory.join(format!("{id}.json"));
        let bytes = fs::read(&path)?;
        let report: Value = serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {error}", path.display()),
            )
        })?;
        if report["id"] != id || read_kind(&report).ok().flatten().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} has an invalid report ID or kind", path.display()),
            ));
        }
        if kind.is_some_and(|kind| report["kind"] != kind) {
            continue;
        }
        if reports.len() == limit {
            let next_before = reports
                .last()
                .and_then(|report| report["id"].as_str())
                .map(str::to_owned);
            return Ok((reports, next_before));
        }
        reports.push(report);
    }
    Ok((reports, None))
}

fn valid_id(id: &str) -> bool {
    let Some((timestamp, process)) = id.split_once('-') else {
        return false;
    };
    timestamp.len() == 29
        && process.len() == 10
        && timestamp
            .bytes()
            .chain(process.bytes())
            .all(|byte| byte.is_ascii_digit())
}

fn read_kind(request: &Value) -> Result<Option<&str>, String> {
    request
        .get("kind")
        .map(|value| {
            value
                .as_str()
                .filter(|kind| KINDS.contains(kind))
                .ok_or_else(|| format!("kind must be one of: {}", KINDS.join(", ")))
        })
        .transpose()
}
