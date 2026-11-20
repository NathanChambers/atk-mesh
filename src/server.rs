use gpt_mesh::session::Session;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use workspace_host::WorkspaceHost;

mod workspace_host;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        println!(
            "mesh-server [--project ROOT | --workspace PATH] [--port PORT] [--headless]\n\nRun a read-only project viewer or an API-only editor for one mesh workspace.\n\n  --project ROOT    Serve saved meshes in the project (default current directory)\n  --workspace PATH  Edit one mesh workspace through the JSON API\n  --port PORT       Listen port (overrides MESH_PORT; default 7878)\n  --headless        Accepted for compatibility; open the viewer URL manually\n  -h, --help        Show this help\n  -V, --version     Show version\n\nUse a distinct port for the project viewer and each editor. Stop with Ctrl+C."
        );
        println!(
            "\nList meshes: mesh-cli --port 7878 --list-workspaces\nStart an editor: mesh-server --workspace PATH --port 7879\nCommand discovery: echo '{{\"command\":\"help\"}}' | mesh-cli --port 7879\nWorkflow guidance: send {{\"command\":\"help\",\"topic\":\"workflow\"}}."
        );
        return Ok(());
    }

    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--version" | "-V") {
        println!("mesh-server {}", env!("CARGO_PKG_VERSION"));
        println!("Features: batch, empty-workspaces, masters-only-uv, preview-reuse, camera-only-reviews");
        return Ok(());
    }

    let mut workspace = None;
    let mut project = None;
    let mut port = None;
    let mut headless = false;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let target = match argument.as_str() {
            "--workspace" => &mut workspace,
            "--project" => &mut project,
            "--port" => &mut port,
            "--headless" => {
                if headless {
                    return Err("--headless specified more than once".into());
                }
                headless = true;
                continue;
            }
            _ => return Err(format!("unknown argument: {argument}; use --help").into()),
        };
        let value = arguments
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        let previous = target.replace(value.clone());
        if previous.is_some() {
            return Err(format!("{argument} specified more than once").into());
        }
    }
    if workspace.is_some() && project.is_some() {
        return Err("--project and --workspace are mutually exclusive".into());
    }
    let port = port.unwrap_or_else(|| std::env::var("MESH_PORT").unwrap_or_else(|_| "7878".into()));
    let port: u16 = port
        .parse()
        .map_err(|_| "port must be an integer from 1 to 65535")?;
    if port == 0 {
        return Err("port must be an integer from 1 to 65535".into());
    }

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let mut host = if let Some(workspace) = workspace {
        WorkspaceHost::open_editor(PathBuf::from(workspace))?
    } else {
        let root = project.as_deref().unwrap_or(".");
        WorkspaceHost::open_viewer(Path::new(root))?
    };
    let url = format!("http://127.0.0.1:{port}");
    if matches!(&host, WorkspaceHost::Viewer { .. }) {
        println!("Project viewer: {url}");
        println!("List meshes: mesh-cli --port {port} --list-workspaces");
    } else {
        println!("Mesh editor API: {url}");
    }
    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                if let Err(error) = handle(&mut stream, &mut host) {
                    eprintln!("Request failed: {error}");
                }
            }
            Err(error) => eprintln!("Connection failed: {error}"),
        }
    }
    Ok(())
}

fn handle(
    stream: &mut TcpStream,
    host: &mut WorkspaceHost,
) -> Result<(), Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.by_ref().take(8192).read_line(&mut line)?;
    let first_line = line.clone();
    let mut length = 0;
    let mut if_none_match = None;
    let mut header_bytes = line.len();
    loop {
        line.clear();
        reader.by_ref().take(8192).read_line(&mut line)?;
        header_bytes += line.len();
        if header_bytes > 16384 {
            return Err("headers too large".into());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if line.is_empty() {
            return Err("incomplete headers".into());
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>()?;
            } else if name.eq_ignore_ascii_case("if-none-match") {
                if_none_match = Some(value.trim().to_owned());
            }
        }
    }
    if length > 64 * 1024 * 1024 {
        return Err("body too large".into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let mut route: Vec<_> = first_line.split_whitespace().collect();
    let mut query = "";
    if let Some(target) = route.get_mut(1)
        && let Some((path, parameters)) = target.split_once('?')
    {
        *target = path;
        query = parameters;
    }
    let mut etag_header = String::new();
    let (status, content_type, output) = match route.as_slice() {
        ["GET", "/workspaces", _] => (200, "application/json", host.list().to_string()),
        ["GET", "/state/meta", _] => {
            let response = parse_workspace_query(query)
                .and_then(|workspace| host.state_metadata(workspace.as_deref()));
            match response {
                Ok(value) => (200, "application/json", value.to_string()),
                Err(error) => (
                    400,
                    "application/json",
                    json!({"ok":false,"error":error}).to_string(),
                ),
            }
        }
        ["GET", "/state", _] => {
            let response = parse_workspace_query(query)
                .and_then(|workspace| host.state(workspace.as_deref(), if_none_match.as_deref()));
            match response {
                Ok((etag, state)) => {
                    if let Some(tag) = etag {
                        etag_header = format!("ETag: {tag}\r\n");
                    }
                    match state {
                        Some(value) => (200, "application/json", value.to_string()),
                        None => (304, "application/json", String::new()),
                    }
                }
                Err(error) => (
                    400,
                    "application/json",
                    json!({"ok":false,"error":error}).to_string(),
                ),
            }
        }
        [method, path, _] if is_workspace_route(method, path) => {
            let response = workspace_route(host, path, query, &body);
            match response {
                Ok((status, value)) => (status, "application/json", value.to_string()),
                Err(error) => (
                    400,
                    "application/json",
                    json!({"ok":false,"error":error}).to_string(),
                ),
            }
        }
        ["GET", path, _] if matches!(&host, WorkspaceHost::Viewer { .. }) => viewer_asset(path),
        _ => (404, "text/plain", "Not found".into()),
    };
    write!(
        stream,
        "HTTP/1.0 {status} Response\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\n{etag_header}Content-Length: {}\r\nConnection: close\r\n\r\n{output}",
        output.len()
    )?;
    Ok(())
}

fn viewer_asset(path: &str) -> (u16, &'static str, String) {
    let (content_type, content) = match path {
        "/" => ("text/html", include_str!("../web/index.html")),
        "/app.js" => ("text/javascript", include_str!("../web/app.js")),
        "/renderer.js" => ("text/javascript", include_str!("../web/renderer.js")),
        "/geometry.js" => ("text/javascript", include_str!("../web/geometry.js")),
        "/animation.js" => ("text/javascript", include_str!("../web/animation.js")),
        "/replay.js" => ("text/javascript", include_str!("../web/replay.js")),
        "/objects.js" => ("text/javascript", include_str!("../web/objects.js")),
        "/viewer.css" => ("text/css", include_str!("../web/viewer.css")),
        _ => return (404, "text/plain", "Not found".into()),
    };
    (200, content_type, content.into())
}

fn is_workspace_route(method: &str, path: &str) -> bool {
    match method {
        "GET" => path == "/sessions" || path.starts_with("/sessions/"),
        "POST" => path == "/command",
        _ => false,
    }
}

fn workspace_route(
    host: &mut WorkspaceHost,
    path: &str,
    query: &str,
    body: &[u8],
) -> Result<(u16, Value), String> {
    let workspace = parse_workspace_query(query)?;
    let workspace = workspace.as_deref();
    if path == "/command" {
        let session = host.editor(workspace)?;
        let request = serde_json::from_slice::<Value>(body).map_err(|error| error.to_string())?;
        let output = match request {
            Value::Array(commands) => execute_batch(session, commands),
            value => session.execute(value),
        };
        let status = if output["ok"] == true { 200 } else { 400 };
        return Ok((status, output));
    }

    match host.recordings(workspace, path) {
        Ok(value) => Ok((200, value)),
        Err(error) => Ok((404, json!({"error":error}))),
    }
}

fn parse_workspace_query(query: &str) -> Result<Option<String>, String> {
    if query.is_empty() {
        return Ok(None);
    }

    let value = query
        .strip_prefix("workspace=")
        .ok_or("expected a workspace query parameter")?;
    let mut bytes = value.bytes();
    let mut decoded = Vec::with_capacity(value.len());
    while let Some(byte) = bytes.next() {
        let byte = match byte {
            b'%' => {
                let high = bytes.next().and_then(|byte| char::from(byte).to_digit(16));
                let low = bytes.next().and_then(|byte| char::from(byte).to_digit(16));
                let (Some(high), Some(low)) = (high, low) else {
                    return Err("invalid percent escape in workspace path".into());
                };
                (high * 16 + low) as u8
            }
            b'+' => b' ',
            b'&' => return Err("expected one workspace query parameter".into()),
            byte => byte,
        };
        if byte.is_ascii_control() {
            return Err("workspace path must not contain control characters".into());
        }
        decoded.push(byte);
    }
    String::from_utf8(decoded)
        .map(Some)
        .map_err(|_| "workspace path must be UTF-8".into())
}

fn execute_batch(session: &mut Session, commands: Vec<Value>) -> Value {
    if commands.is_empty() || commands.len() > 1024 {
        return json!({"ok":false,"error":"batch input must be a JSON array of 1..1024 commands","replies":[]});
    }

    for (index, command) in commands.iter().enumerate() {
        if !command["command"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
        {
            return json!({
                "ok":false,
                "error":"input must contain JSON objects with nonempty command strings",
                "failed_command":index + 1,
                "replies":[]
            });
        }
    }

    let mut replies = Vec::with_capacity(commands.len());
    for command in commands {
        let reply = session.execute(command);
        let failed = reply["ok"] != true || reply["recording_error"].is_string();
        replies.push(reply);
        if failed {
            return json!({"ok":false,"failed_command":replies.len(),"replies":replies});
        }
    }
    json!({"ok":true,"replies":replies})
}
