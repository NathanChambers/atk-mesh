use gpt_mesh::session::Session;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub enum WorkspaceHost {
    Editor { session: Session, _lock: File },
    Viewer { root: PathBuf },
}

impl WorkspaceHost {
    pub fn open_editor(path: PathBuf, debug_session: bool, compress_recordings: bool) -> Result<Self, Box<dyn std::error::Error>> {
        fs::create_dir_all(&path)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join(".mesh-editor.lock"))?;
        lock.try_lock()
            .map_err(|error| format!("cannot lock mesh workspace {}: {error}", path.display()))?;
        let session = Session::open_editor(path, debug_session, compress_recordings)?;
        Ok(Self::Editor {
            session,
            _lock: lock,
        })
    }

    pub fn open_viewer(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let root = fs::canonicalize(path)?;
        if !root.is_dir() {
            return Err("project root must be a directory".into());
        }
        Ok(Self::Viewer { root })
    }

    pub fn list(&self) -> Value {
        match self {
            Self::Editor { session, .. } => {
                json!({"ok":true,"result":{"mode":"editor","workspaces":[{"path":session.workspace()}]}})
            }
            Self::Viewer { root } => {
                let workspaces = discover(root);
                json!({"ok":true,"result":{"mode":"viewer","root":root,"workspaces":workspaces}})
            }
        }
    }

    pub fn state(
        &self,
        workspace: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<(Option<String>, Option<Value>), String> {
        match self {
            Self::Editor { session, .. } => {
                require_editor_target(workspace)?;
                let state =
                    json!({"ok":true,"result":session.state(),"workspace":session.workspace()});
                Ok((None, Some(state)))
            }
            Self::Viewer { root } => {
                let workspace = resolve(root, workspace)?;
                let mut file = File::open(workspace.join("project.json"))
                    .map_err(|error| format!("cannot read saved mesh: {error}"))?;
                let etag = file
                    .metadata()
                    .ok()
                    .and_then(|metadata| saved_state_tag(&workspace, &metadata));
                if let (Some(candidate), Some(tag)) = (if_none_match, etag.as_deref())
                    && candidate == tag
                {
                    return Ok((etag, None));
                }

                let mut source = String::new();
                file.read_to_string(&mut source)
                    .map_err(|error| format!("cannot read saved mesh: {error}"))?;
                let mut project: Value =
                    serde_json::from_str(source.trim_start_matches('\u{feff}'))
                        .map_err(|error| format!("cannot read saved mesh: {error}"))?;
                let mut mesh = project["mesh"].take();
                if !mesh["vertices"].is_object() || !mesh["faces"].is_object() {
                    return Err("project.json does not contain a native mesh".into());
                }
                Session::normalize_saved_stages(&mut mesh);
                let state = json!({"ok":true,"result":{"mesh":mesh,"revision":project["revision"]},"workspace":workspace});
                Ok((etag, Some(state)))
            }
        }
    }

    pub fn state_metadata(&self, workspace: Option<&str>) -> Result<Value, String> {
        let Self::Viewer { root } = self else {
            return Err("saved state metadata is available on the project viewer port".into());
        };
        let tag = saved_workspace_tag(root, workspace)?;
        Ok(json!({"ok":true,"result":{"tag":tag}}))
    }

    pub fn recordings(&self, workspace: Option<&str>, route: &str) -> Result<Value, String> {
        match self {
            Self::Editor { session, .. } => {
                require_editor_target(workspace)?;
                session.read(route)
            }
            Self::Viewer { root } => {
                let path = resolve(root, workspace)?;
                Session::read_saved_recordings(&path, route)
            }
        }
    }

    pub fn editor(&mut self, workspace: Option<&str>) -> Result<&mut Session, String> {
        match self {
            Self::Editor { session, .. } => {
                require_editor_target(workspace)?;
                Ok(session)
            }
            Self::Viewer { .. } => {
                Err("the project viewer is read-only; use a mesh editor's port".into())
            }
        }
    }
}

pub fn saved_workspace_tag(root: &Path, workspace: Option<&str>) -> Result<Option<String>, String> {
    let workspace = resolve(root, workspace)?;
    let metadata = fs::symlink_metadata(workspace.join("project.json"))
        .map_err(|error| format!("cannot read saved mesh metadata: {error}"))?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err("cannot read saved mesh metadata".into());
    }

    Ok(saved_state_tag(&workspace, &metadata))
}

fn saved_state_tag(workspace: &Path, metadata: &fs::Metadata) -> Option<String> {
    let modified = metadata.modified().ok()?;
    let mut hash = DefaultHasher::new();
    (workspace, metadata.len(), modified).hash(&mut hash);
    Some(format!("W/\"{:016x}\"", hash.finish()))
}

fn require_editor_target(workspace: Option<&str>) -> Result<(), String> {
    if workspace.is_some() {
        return Err("the editor port already identifies the mesh; omit the workspace query".into());
    }
    Ok(())
}

fn discover(root: &Path) -> Vec<Value> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(directory) = pending.pop() {
        let is_workspace = is_mesh_workspace(&directory);
        if is_workspace
            && let Ok(relative) = directory.strip_prefix(root)
            && let Some(relative) = relative.to_str()
        {
            let path = if relative.is_empty() { "." } else { relative };
            paths.push(path.replace(std::path::MAIN_SEPARATOR, "/"));
        }

        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | ".svn" | "node_modules" | "target" | "Library"
            ) {
                continue;
            }
            if is_workspace && matches!(name.as_ref(), "resources" | "sessions") {
                continue;
            }
            let path = entry.path();
            if fs::symlink_metadata(&path).is_ok_and(|metadata| is_real_directory(&metadata)) {
                pending.push(path);
            }
        }
    }
    paths.sort();
    paths.into_iter().map(|path| json!({"path":path})).collect()
}

fn resolve(root: &Path, workspace: Option<&str>) -> Result<PathBuf, String> {
    let workspace = workspace
        .filter(|path| !path.is_empty())
        .ok_or("select a mesh workspace")?;
    let mut path = root.to_path_buf();
    for component in Path::new(workspace).components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => path.push(name),
            _ => return Err("workspace path must be relative to the project root".into()),
        }
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| is_real_directory(&metadata)) {
            return Err(format!("cannot access mesh workspace {workspace}"));
        }
    }

    let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if !path.starts_with(root) || !has_project_file(&path) {
        return Err(format!("cannot access mesh workspace {workspace}"));
    }
    Ok(path)
}

fn has_project_file(path: &Path) -> bool {
    fs::symlink_metadata(path.join("project.json"))
        .is_ok_and(|metadata| metadata.is_file() && !is_link(&metadata))
}

fn is_mesh_workspace(path: &Path) -> bool {
    if !has_project_file(path) {
        return false;
    }
    if path.join("workflow.json").is_file() {
        return true;
    }

    #[derive(Deserialize)]
    struct MeshProject {
        #[serde(rename = "version")]
        _version: u64,
        #[serde(rename = "mesh")]
        _mesh: MeshFields,
    }

    #[derive(Deserialize)]
    struct MeshFields {
        #[serde(rename = "vertices")]
        _vertices: IgnoredAny,
        #[serde(rename = "faces")]
        _faces: IgnoredAny,
    }

    let Ok(source) = fs::read_to_string(path.join("project.json")) else {
        return false;
    };
    serde_json::from_str::<MeshProject>(source.trim_start_matches('\u{feff}')).is_ok()
}

fn is_real_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !is_link(metadata)
}

#[cfg(windows)]
fn is_link(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
