use serde_json::{Value, json};
use std::io::{IsTerminal, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpt_mesh::feedback;
mod recording;

fn main() {
    let mut replied = false;
    if let Err(error) = run(&mut replied) {
        if !replied {
            println!("{}", serde_json::json!({"ok": false, "error": error.to_string()}));
        }
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(replied: &mut bool) -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|argument| argument == "--replay") {
        return match arguments.as_slice() {
            [_, path] => recording::replay(Path::new(path), None),
            [_, path, option, value] if option == "--frame" => {
                let frame = value.parse().map_err(|_| "--frame requires a nonnegative number")?;
                recording::replay(Path::new(path), Some(frame))
            }
            _ => Err("Use --replay FILE [--frame NUMBER]".into()),
        };
    }
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        println!(
            "atk-mesh-cli [--port PORT] [--batch] [--input FILE | JSON | -] [--output FILE]\natk-mesh-cli [--port PORT] --list-workspaces\n\nSend JSON commands to a running mesh editor. The editor's port selects the mesh.\nWith no input, read stdin. The legacy form atk-mesh-cli JSON OUTPUT is supported.\n\n      --list-workspaces  List meshes discovered by the project viewer\n  -i, --input FILE       Read command JSON from a UTF-8 file\n  -o, --output FILE      Save an artifact or result JSON\n      --batch            Send a JSON array in one request\n      --port PORT        Server port (overrides MESH_PORT; default 7878)\n      --timeout SECONDS  Optional socket read timeout for commands\n  -h, --help             Show this help\n  -V, --version          Show version\n\nExit status: 0 for success, 1 for command, input, connection or output errors.\nServer replies are JSON on stdout; diagnostics go to stderr."
        );
        println!(
            "\nDiscover commands: echo '{{\"command\":\"help\"}}' | atk-mesh-cli --port 7879\nRead workflow: echo '{{\"command\":\"help\",\"topic\":\"workflow\"}}' | atk-mesh-cli --port 7879"
        );
        println!(
            "\nSend a JSON array to run commands in order. Use --batch to require an array. Stops at the first error; earlier commands remain applied. Each reply is one JSON line. --output is unavailable for arrays."
        );
        println!("Use --file FILE as an alias for --input FILE, or --command JSON for inline input.");
        println!("Use --debug to record requests, replies, and session state. Use --replay FILE [--frame NUMBER] to read a recording.");
        println!("\nLocal reconstruction commands: reference_guides (JSON/SVG) and visual_hull (outer volume estimate as project JSON/OBJ). Both take --input and --output, with no editor required.");
        println!("\nMesh sculpting: send JSON help with topic \"local-sculpting\" to an editor for move_vertices, sculpt_stroke, and local connectivity checks.");
        println!("\nLocal feedback commands: feedback_submit (kind, summary, goal, observed) and feedback_list (optional kind, limit, before). Run as single JSON requests without an editor. MESH_FEEDBACK_DIR overrides the per-user inbox.");
        return Ok(());
    }

    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--version" | "-V") {
        println!("atk-mesh-cli {}", env!("CARGO_PKG_VERSION"));
        println!("Features: batch, request-correlation, configurable-read-timeout");
        return Ok(());
    }

    let mut source = None;
    let mut command = None;
    let mut output = None;
    let mut port = None;
    let mut timeout = None;
    let mut list_workspaces = false;
    let mut positional = Vec::new();
    let mut batch = false;
    let mut debug = false;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--debug" if !debug => debug = true,
            "--batch" => {
                if batch {
                    return Err("--batch specified more than once".into());
                }
                batch = true;
            }
            "--list-workspaces" => {
                if list_workspaces {
                    return Err("--list-workspaces specified more than once".into());
                }
                list_workspaces = true;
            }
            "-i" | "--input" | "--file" | "--command" | "-o" | "--output" | "--port" | "--timeout" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| format!("{argument} requires a value"))?;
                let target = match argument.as_str() {
                    "-i" | "--input" | "--file" => &mut source,
                    "--command" => &mut command,
                    "-o" | "--output" => &mut output,
                    "--timeout" => &mut timeout,
                    _ => &mut port,
                };
                let previous = target.replace(value.clone());
                if previous.is_some() {
                    return Err(format!("{argument} specified more than once").into());
                }
            }
            "--" => {
                positional.extend(arguments.cloned());
                break;
            }
            "-" => positional.push(argument.clone()),
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}; use --help").into());
            }
            _ => positional.push(argument.clone()),
        }
    }
    let explicit_sources = usize::from(source.is_some()) + usize::from(command.is_some());
    if positional.len() > 2 || explicit_sources > 1 || (explicit_sources > 0 && !positional.is_empty()) {
        return Err(
            "use one JSON argument or --input FILE, and at most one output path; see --help".into(),
        );
    }

    if positional.len() == 2 {
        let previous = output.replace(positional[1].clone());
        if previous.is_some() {
            return Err("output specified more than once".into());
        }
    }

    let read_port = || -> Result<u16, &'static str> {
        let value = port.clone().unwrap_or_else(|| std::env::var("MESH_PORT").unwrap_or_else(|_| "7878".into()));
        value.parse().ok().filter(|port| *port != 0).ok_or("port must be an integer from 1 to 65535")
    };
    let timeout = timeout
        .map(|value| {
            value.parse::<u64>().ok().filter(|seconds| *seconds > 0)
                .map(Duration::from_secs).ok_or("--timeout must be a positive number of seconds")
        })
        .transpose()?;

    if list_workspaces {
        if batch || debug || explicit_sources > 0 || output.is_some() || !positional.is_empty() {
            return Err("--list-workspaces does not take command input or output".into());
        }
        let response = request_json("GET", "/workspaces", None, read_port()?, timeout.or(Some(Duration::from_secs(120))))?;
        println!("{}", serde_json::to_string(&response)?);
        *replied = true;
        if response["ok"] != true {
            return Err("could not list mesh workspaces; see JSON response on stdout".into());
        }
        return Ok(());
    }

    let mut input = String::new();
    if let Some(json) = command {
        input = json;
    } else if let Some(path) = source {
        if path == "-" {
            std::io::stdin().read_to_string(&mut input)?;
        } else {
            input = std::fs::read_to_string(path)?;
        }
    } else if let Some(value) = positional.first().filter(|value| value.as_str() != "-") {
        input = value.clone();
    } else {
        if std::io::stdin().is_terminal() {
            return Err("provide a JSON command, --input FILE, or piped stdin; see --help".into());
        }
        std::io::stdin().read_to_string(&mut input)?;
    }
    let mut request: Value = serde_json::from_str(input.trim_start_matches('\u{feff}'))?;
    if !batch && !request.is_array() {
        let local = feedback::is_command(&request)
            || matches!(request["command"].as_str(), Some("reference_guides" | "visual_hull"));
        if local {
            let mut recording = if debug {
                Some(recording::Recording::create(Path::new(".mesh/recordings"), "", Value::Null)?)
            } else {
                None
            };
            if let Some(recording) = &mut recording {
                recording.begin(&request)?;
            }
            let result = execute_local(&request, output.as_deref());
            if let Some(recording) = &mut recording {
                let reply = match &result {
                    Ok(reply) => reply.clone(),
                    Err(error) => json!({"ok": false, "error": error.to_string()}),
                };
                recording.finish(&reply, Value::Null)?;
            }
            println!("{}", result?);
            return Ok(());
        }
        let port = read_port()?;
        let recording = open_recording(debug, port, timeout)?;
        return send(request, port, output.as_deref(), timeout, replied, recording);
    }

    if output.is_some() {
        return Err("--batch cannot be combined with --output".into());
    }

    let commands = request
        .as_array()
        .filter(|commands| !commands.is_empty() && commands.len() <= 1024)
        .ok_or("batch input must be a JSON array of 1..1024 commands")?;
    for (index, command) in commands.iter().enumerate() {
        if feedback::is_command(command) {
            return Err(format!("command {}: feedback commands must run without --batch", index + 1).into());
        }
        if !command["command"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
        {
            return Err(format!(
                "command {}: expected a JSON object with a nonempty command string",
                index + 1
            )
            .into());
        }
    }

    let port = read_port()?;
    let mut recording = open_recording(debug, port, timeout)?;
    assign_request_id(&mut request)?;
    if let Some(recording) = &mut recording {
        recording.submit_batch(&request)?;
    }
    let route = if debug { "/command/debug" } else { "/command" };
    let response = request_json("POST", route, Some(&request), port, timeout);
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            if let Some(recording) = &mut recording {
                recording.batch_failure(&error.to_string())?;
            }
            return Err(error);
        }
    };
    if let Some(replies) = response["replies"].as_array_mut() {
        if let Some(recording) = &mut recording {
            let commands = request.as_array().expect("validated batch");
            for (command, reply) in commands.iter().zip(replies.iter_mut()) {
                recording.begin(command)?;
                let state = remove_debug_state(reply)?;
                recording.finish(reply, state)?;
            }
        }
        for reply in replies {
            println!("{}", serde_json::to_string(reply)?);
            *replied = true;
        }
    }
    if let Some(replies) = response["replies"].as_array() {
        if let Some(index) = replies.iter().position(|reply| reply["recording_error"].is_string()) {
            return Err(format!("command {}: workspace persistence failed; see recording_error on stdout", index + 1).into());
        }
    }
    if response["ok"] != true {
        if let Some(error) = response["error"].as_str() {
            return Err(error.to_string().into());
        }
        let index = response["failed_command"]
            .as_u64()
            .ok_or("invalid batch response")?;
        return Err(format!("command {index}: command failed; see JSON response on stdout").into());
    }
    Ok(())
}

fn execute_local(request: &Value, output: Option<&str>) -> Result<Value, Box<dyn std::error::Error>> {
    let result = if feedback::is_command(request) {
        if output.is_some() {
            return Err("feedback commands write their replies to stdout; omit --output".into());
        }
        feedback::run(request)?
    } else {
        gpt_mesh::reconstruction::run(request, output)?.ok_or("expected a reconstruction command")?
    };
    Ok(json!({"ok": true, "result": result}))
}

fn open_recording(
    debug: bool,
    port: u16,
    timeout: Option<Duration>,
) -> Result<Option<recording::Recording>, Box<dyn std::error::Error>> {
    if !debug {
        return Ok(None);
    }

    let mut state = request_json("GET", "/state", None, port, timeout)?;
    let workspace = state["workspace"].as_str().ok_or("--debug requires a mesh editor port")?;
    let directory = Path::new(workspace).join("recordings");
    let session = workspace.to_owned();
    let state = state["result"].take();
    Ok(Some(recording::Recording::create(&directory, &session, state)?))
}

fn remove_debug_state(reply: &mut Value) -> Result<Value, Box<dyn std::error::Error>> {
    reply.as_object_mut().and_then(|reply| reply.remove("debug_state")).ok_or_else(|| {
        "Mesh editor did not return a debug snapshot. Check the workspace before retrying.".into()
    })
}

fn send(
    mut request: Value,
    port: u16,
    output: Option<&str>,
    timeout: Option<Duration>,
    replied: &mut bool,
    mut recording: Option<recording::Recording>,
) -> Result<(), Box<dyn std::error::Error>> {
    let valid_command = request["command"]
        .as_str()
        .is_some_and(|command| !command.is_empty());
    if !request.is_object() || !valid_command {
        return Err("input must be a JSON object with a nonempty command string".into());
    }

    let binary_output = output.as_ref().and_then(|path| {
        let extension = std::path::Path::new(path).extension()?;
        ["fbx", "glb", "stl"].into_iter().find(|format| extension.eq_ignore_ascii_case(format))
    });
    if let Some(format) = binary_output {
        if request["command"] != "export" {
            return Err(format!("{} output requires an export command", format.to_uppercase()).into());
        }
        request["format"] = Value::String(format.into());
    }
    let route = if recording.is_some() { "/command/debug" } else { "/command" };
    assign_request_id(&mut request)?;
    if let Some(recording) = &mut recording {
        recording.begin(&request)?;
    }
    let result = request_json("POST", route, Some(&request), port, timeout);
    let mut response = match result {
        Ok(response) => response,
        Err(error) => {
            if let Some(recording) = &mut recording {
                recording.finish(&json!({"ok": false, "error": error.to_string()}), Value::Null)?;
            }
            return Err(error);
        }
    };
    let state = if recording.is_some() { remove_debug_state(&mut response)? } else { Value::Null };
    let result = write_requested_output(&mut response, &request, output, binary_output);
    if let Some(recording) = &mut recording {
        let reply = match &result {
            Ok(()) => response.clone(),
            Err(error) => json!({"ok": false, "error": error.to_string(), "server_reply": response}),
        };
        recording.finish(&reply, state)?;
    }
    result?;
    if response["recording_error"].is_string() {
        println!("{}", serde_json::to_string(&response)?);
        *replied = true;
        return Err("workspace persistence failed; see recording_error on stdout".into());
    }
    if response["ok"] != true {
        println!("{}", serde_json::to_string(&response)?);
        *replied = true;
        return Err("command failed; see JSON response on stdout".into());
    }

    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

fn write_requested_output(
    response: &mut Value,
    request: &Value,
    output: Option<&str>,
    binary_output: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    if response["ok"] != true || response["recording_error"].is_string() {
        return Ok(());
    }
    if let Some(path) = output {
        if let Some(error) = response["artifact_error"].as_str() {
            return Err(format!("artifact could not be saved: {error}").into());
        }
        let result = &response["result"];
        let png_output = std::path::Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
        let extension = if let Some(format) = binary_output {
            format
        } else if png_output {
            "png"
        } else if request["command"] == "workflow_report" {
            "markdown"
        } else if request["command"] == "export_verify" {
            "report"
        } else if request["command"] == "save" {
            "project"
        } else if request["command"] == "capture" || request["command"] == "capture_uv" {
            "svg"
        } else {
            "obj"
        };
        if let Some(source) = result["artifacts"][extension]["path"].as_str() {
            let source_path = std::path::Path::new(source);
            let destination = std::path::Path::new(path);
            let same_file = std::fs::canonicalize(destination).is_ok_and(|p| p == source_path);
            if !same_file {
                std::fs::copy(source_path, destination)?;
            }
        } else if let Some(format) = binary_output {
            let encoded = result[format!("{format}_base64")].as_str().ok_or_else(|| format!("missing {} data", format.to_uppercase()))?;
            let bytes = gpt_mesh::base64::decode(encoded)?;
            std::fs::write(path, bytes)?;
        } else if png_output {
            let encoded = result["png_base64"]
                .as_str()
                .ok_or("PNG output requires a capture command")?;
            let bytes = gpt_mesh::base64::decode(encoded)?;
            std::fs::write(path, bytes)?;
        } else {
            let output = result["svg"]
                .as_str()
                .or(result["markdown"].as_str())
                .or(result["obj"].as_str())
                .map(str::to_string)
                .unwrap_or(serde_json::to_string_pretty(result)?);
            std::fs::write(path, output)?;
        }
        response["written"] = serde_json::json!({"path":std::fs::canonicalize(path)?,"bytes":std::fs::metadata(path)?.len()});
    }
    Ok(())
}

fn assign_request_id(request: &mut Value) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let id = Value::String(format!("{timestamp}-{}", std::process::id()));
    if let Some(commands) = request.as_array_mut() {
        for command in commands {
            command["cli_request_id"] = id.clone();
        }
    } else {
        request["cli_request_id"] = id;
    }
    Ok(())
}

fn request_json(
    method: &str,
    path: &str,
    request: Option<&Value>,
    port: u16,
    timeout: Option<Duration>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let request_id = request.and_then(|request| {
        let command = request.as_array().and_then(|commands| commands.first()).unwrap_or(request);
        command["cli_request_id"].as_str()
    });
    let input = request
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?
        .unwrap_or_default();
    let mut connection = TcpStream::connect(("127.0.0.1", port)).map_err(|error| format!("cannot connect to atk-mesh on 127.0.0.1:{port}: {error}; check the viewer or editor port"))?;
    connection.set_read_timeout(timeout)?;
    connection.set_write_timeout(Some(Duration::from_secs(120)))?;
    let response_error = |error: &dyn std::fmt::Display| match &request_id {
        Some(id) => format!("could not confirm the command outcome on port {port}: {error}. The server may still apply the request. Check the current workspace before retrying. If the editor was started with --debug, look for cli_request_id {id} in its session recordings"),
        None => format!("could not read the response on port {port}: {error}"),
    };
    write!(
        connection,
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        input.len(),
        input
    ).map_err(|error| response_error(&error))?;
    let mut response = String::new();
    connection.read_to_string(&mut response).map_err(|error| response_error(&error))?;
    let (_, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| response_error(&"invalid HTTP response"))?;
    serde_json::from_str(body).map_err(|error| response_error(&error).into())
}
