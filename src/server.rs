mod build_identity;
#[allow(dead_code)]
mod mcp;

use atk_mesh::{command_catalog, session::Session};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use viewer_updates::{ViewerUpdates, WorkspaceUpdates};
use workspace_host::WorkspaceHost;

mod viewer_updates;
mod workspace_host;

const MAX_VIEWER_STREAMS: usize = 16;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        println!(
            "atk-mesh [ROOT | --project ROOT | --workspace PATH] [--port PORT] [--lan] [--headless]\n\nRun a read-only project viewer or an API-only editor for one mesh workspace.\n\n  ROOT              Serve saved meshes under this directory\n  --project ROOT    Serve saved meshes in the project (default current directory)\n  --workspace PATH  Edit one mesh workspace through the JSON API\n  --port PORT       Listen port (overrides MESH_PORT; omitted or 0 picks an available port)\n  --lan             Allow LAN access to the read-only viewer (default localhost, incompatible with --workspace)\n  --headless        Accepted for compatibility\n  -h, --help        Show this help\n  -V, --version     Show version\n\nFor CLI calls, use the port printed on startup. Stop with Ctrl+C."
        );
        println!(
            "\nList meshes: atk-mesh-cli --port 7878 --list-workspaces\nStart an editor: atk-mesh --workspace PATH --port 7879\nCommand discovery: echo '{{\"command\":\"help\"}}' | atk-mesh-cli --port 7879\nWorkflow guidance: send {{\"command\":\"help\",\"topic\":\"workflow\"}}."
        );
        println!("Use atk-mesh-cli --debug to record a command timeline. The editor's --debug flag retains full server traces for query_record and timeout recovery.");
        return Ok(());
    }

    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--version" | "-V") {
        build_identity::print_version()?;
        println!("Features: batch, empty-workspaces, masters-only-uv, preview-reuse, camera-only-reviews");
        return Ok(());
    }

    let mut workspace = None;
    let mut project = None;
    let mut port = None;
    let mut lan = false;
    let mut headless = false;
    let mut debug_session = false;
    let mut compress_recordings = false;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let target = match argument.as_str() {
            "--workspace" => &mut workspace,
            "--project" => &mut project,
            "--port" => &mut port,
            "--lan" => {
                if lan {
                    return Err("--lan specified more than once".into());
                }
                lan = true;
                continue;
            }
            "--debug" => {
                debug_session = true;
                continue;
            }
            "--compress-recordings" => {
                compress_recordings = true;
                continue;
            }
            "--headless" => {
                if headless {
                    return Err("--headless specified more than once".into());
                }
                headless = true;
                continue;
            }
            _ if !argument.starts_with('-') => {
                if project.replace(argument.clone()).is_some() {
                    return Err("Specify the viewer root once, as a path or with --project".into());
                }
                continue;
            }
            _ => return Err(format!("unknown argument: {argument}; use --help").into()),
        };
        if argument == "--project" && target.is_some() {
            return Err("Specify the viewer root once, as a path or with --project".into());
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("{argument} requires a value"))?;
        let previous = target.replace(value.clone());
        if previous.is_some() {
            return Err(format!("{argument} specified more than once").into());
        }
    }
    if workspace.is_some() && project.is_some() {
        return Err("A viewer root and --workspace are mutually exclusive".into());
    }
    if lan && workspace.is_some() {
        return Err("--lan is only available for the read-only project viewer and cannot be combined with --workspace".into());
    }
    if debug_session && workspace.is_none() {
        return Err("--debug requires a workspace editor".into());
    }
    if compress_recordings && workspace.is_none() {
        return Err("--compress-recordings requires a workspace editor".into());
    }
    let port = port.unwrap_or_else(|| std::env::var("MESH_PORT").unwrap_or_else(|_| "0".into()));
    let port: u16 = port
        .parse()
        .map_err(|_| "port must be an integer from 0 to 65535")?;

    let bind_ip = if lan { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST };
    let listener = TcpListener::bind((bind_ip, port))?;
    let mut host = if let Some(workspace) = workspace {
        WorkspaceHost::open_editor(PathBuf::from(workspace), debug_session || compress_recordings, true)?
    } else {
        let root = project.as_deref().unwrap_or(".");
        WorkspaceHost::open_viewer(Path::new(root))?
    };
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}");
    if matches!(&host, WorkspaceHost::Viewer { .. }) {
        println!("Project viewer: {url}");
        if lan {
            println!("LAN viewer: http://<host-LAN-IP>:{port}");
        }
        println!("List meshes: atk-mesh-cli --port {port} --list-workspaces");
    } else {
        println!("Mesh editor API: {url}");
        println!("Send commands with atk-mesh-cli --port {port}");
    }
    let updates = Arc::new(ViewerUpdates::default());
    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                if let Err(error) = handle(&mut stream, &mut host, &updates, lan) {
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
    updates: &Arc<ViewerUpdates>,
    lan: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let lan_client = lan && !stream.peer_addr()?.ip().is_loopback();
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.by_ref().take(8192).read_line(&mut line)?;
    let first_line = line.clone();
    let mut length = 0;
    let mut if_none_match = None;
    let mut has_origin = false;
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
            } else if name.eq_ignore_ascii_case("origin") {
                has_origin = true;
            }
        }
    }
    let mut route: Vec<_> = first_line.split_whitespace().collect();
    let mut query = "";
    if let Some(target) = route.get_mut(1)
        && let Some((path, parameters)) = target.split_once('?')
    {
        *target = path;
        query = parameters;
    }
    if lan_client && matches!(route.as_slice(), [method, path, _] if *method != "GET" || *path == "/mcp") {
        write!(stream, "HTTP/1.0 404 Response\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot found")?;
        stream.shutdown(Shutdown::Write)?;
        return Ok(());
    }

    if length > 64 * 1024 * 1024 {
        return Err("body too large".into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;

    if let ["GET", "/api/events", _] = route.as_slice() {
        match workspace_updates(host, query) {
            Ok(source) if Arc::strong_count(updates) <= MAX_VIEWER_STREAMS => {
                let stream = stream.try_clone()?;
                let updates = Arc::clone(updates);
                std::thread::Builder::new()
                    .name("mesh-viewer-events".into())
                    .spawn(move || {
                        let _ = updates.serve(stream, source);
                    })?;
                return Ok(());
            }
            Ok(_) => {
                write!(stream, "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
                return Ok(());
            }
            Err(error) => {
                write!(stream, "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{error}", error.len())?;
                return Ok(());
            }
        }
    }

    let mut etag_header = String::new();
    let (status, content_type, output) = match route.as_slice() {
        [_, "/mcp", _] if has_origin => (403, "text/plain", "Browser origins are disabled for MCP".into()),
        ["POST", "/mcp", _] => {
            let (status, output) = mcp_route(host, query, &body, updates);
            (status, "application/json", output)
        }
        [_, "/mcp", _] => (405, "text/plain", String::new()),
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
        ["GET", "/project.json", _] => {
            let file = parse_workspace_query(query).and_then(|workspace| host.project_file(workspace.as_deref()));
            match file {
                Ok((mut file, length, tag)) => {
                    if let Some(tag) = tag {
                        etag_header = format!("ETag: {tag}\r\n");
                        if if_none_match.as_ref() == Some(&tag) {
                            write!(stream, "HTTP/1.0 304 Not Modified\r\n{etag_header}Connection: close\r\n\r\n")?;
                            return Ok(());
                        }
                    }
                    write!(stream, "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-cache\r\n{etag_header}Content-Length: {length}\r\nConnection: close\r\n\r\n")?;
                    std::io::copy(&mut file, stream)?;
                    return Ok(());
                }
                Err(error) => (400, "application/json", json!({"ok":false,"error":error}).to_string()),
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
            let response = workspace_route(host, path, query, &body, updates);
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
        "/objects.js" => ("text/javascript", include_str!("../web/objects.js")),
        "/viewer.css" => ("text/css", include_str!("../web/viewer.css")),
        _ => return (404, "text/plain", "Not found".into()),
    };
    (200, content_type, content.into())
}

fn workspace_updates(host: &WorkspaceHost, query: &str) -> Result<WorkspaceUpdates, String> {
    let workspace = parse_workspace_query(query)?;
    match (host, workspace) {
        (WorkspaceHost::Editor { .. }, None) => Ok(WorkspaceUpdates::Editor),
        (WorkspaceHost::Viewer { root }, Some(workspace)) => Ok(WorkspaceUpdates::Saved {
            root: root.clone(),
            workspace,
        }),
        (WorkspaceHost::Editor { .. }, Some(_)) => Err("the editor port already identifies the mesh; omit the workspace query".into()),
        (WorkspaceHost::Viewer { .. }, None) => Err("select a mesh workspace".into()),
    }
}

fn is_workspace_route(method: &str, path: &str) -> bool {
    match method {
        "GET" => path == "/sessions" || path.starts_with("/sessions/"),
        "POST" => matches!(path, "/command" | "/command/debug"),
        _ => false,
    }
}

fn mcp_route(host: &mut WorkspaceHost, query: &str, body: &[u8], updates: &ViewerUpdates) -> (u16, String) {
    let request: Value = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(_) => return (400, json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}).to_string()),
    };
    let tools = command_catalog::editor_tools();
    let response = mcp::dispatch_request(&request, "atk-mesh", tools, true, &mut |name, arguments| {
        if name == "list_workspaces" {
            return Ok(host.list());
        }
        if name == "version" {
            return Ok(json!({"ok":true,"version":env!("CARGO_PKG_VERSION")}));
        }
        let workspace = parse_workspace_query(query)?;
        let session = host.editor(workspace.as_deref())?;
        let previous_revision = session.revision();
        let snapshots = request["params"]["_meta"]["atk-mesh/debug"] == true;
        let reply = if name == "batch" {
            execute_batch(session, mcp::batch_commands(arguments)?.to_vec(), snapshots)
        } else {
            execute_command(session, mcp::native_command(name, arguments)?, snapshots)
        };
        if session.revision() != previous_revision {
            updates.publish();
        }
        Ok(reply)
    });
    match response {
        Some(response) => (200, response.to_string()),
        None => (202, String::new()),
    }
}

fn workspace_route(
    host: &mut WorkspaceHost,
    path: &str,
    query: &str,
    body: &[u8],
    updates: &ViewerUpdates,
) -> Result<(u16, Value), String> {
    let workspace = parse_workspace_query(query)?;
    let workspace = workspace.as_deref();
    if matches!(path, "/command" | "/command/debug") {
        let session = host.editor(workspace)?;
        let snapshots = path == "/command/debug";
        let request = serde_json::from_slice::<Value>(body).map_err(|error| error.to_string())?;
        let previous_revision = session.revision();
        let output = match request {
            Value::Array(commands) => execute_batch(session, commands, snapshots),
            value => execute_command(session, value, snapshots),
        };
        if session.revision() != previous_revision {
            updates.publish();
        }

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

fn execute_command(session: &mut Session, command: Value, snapshot: bool) -> Value {
    let mut reply = session.execute(command);
    if snapshot {
        reply["debug_state"] = session.state();
    }
    reply
}

fn execute_batch(session: &mut Session, commands: Vec<Value>, snapshots: bool) -> Value {
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
        let reply = execute_command(session, command, snapshots);
        let failed = reply["ok"] != true || reply["recording_error"].is_string();
        replies.push(reply);
        if failed {
            return json!({"ok":false,"failed_command":replies.len(),"replies":replies});
        }
    }
    json!({"ok":true,"replies":replies})
}
