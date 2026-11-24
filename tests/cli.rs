mod common;

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    workspace: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.workspace);
    }
}

fn invoke(port: u16, arguments: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_atk-mesh-cli"))
        .env("MESH_PORT", "invalid")
        .args(["--port", &port.to_string()])
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn reply(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid reply: {error}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn start_server(project_mode: bool) -> (Server, u16) {
    start_server_at(project_mode, None, true)
}

fn start_server_at(project_mode: bool, workspace: Option<PathBuf>, seed_cube: bool) -> (Server, u16) {
    start_server_with_options(project_mode, workspace, seed_cube, &[])
}

fn start_server_with_options(
    project_mode: bool,
    workspace: Option<PathBuf>,
    seed_cube: bool,
    options: &[&str],
) -> (Server, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let workspace = workspace.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("mesh-cli-test-{}-{port}", std::process::id()))
    });
    let mut command = Command::new(env!("CARGO_BIN_EXE_atk-mesh"));
    command
        .args(options)
        .env("MESH_PORT", "invalid")
        .args(["--port", &port.to_string()]);
    if project_mode {
        std::fs::create_dir(&workspace).unwrap();
        command.arg("--headless").arg(&workspace);
    } else {
        if seed_cube {
            common::write_cube_project(&workspace);
        }
        command.arg("--workspace").arg(&workspace);
    }
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut server = Server { child, workspace };
    let started = Instant::now();
    loop {
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "server exited during startup"
        );
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "server startup timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    (server, port)
}

fn get(port: u16, path: &str) -> (u16, String) {
    let (status, _, body) = get_with_headers(port, path, "");
    (status, body)
}

fn get_with_headers(port: u16, path: &str, request_headers: &str) -> (u16, String, String) {
    let mut connection = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(connection, "GET {path} HTTP/1.0\r\n{request_headers}\r\n").unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, headers.to_string(), body.to_string())
}

fn get_json(port: u16, path: &str) -> (u16, Value) {
    let (status, body) = get(port, path);
    (status, serde_json::from_str(&body).unwrap())
}

#[test]
fn cli_imports_binary_fbx_and_persists_editable_geometry() {
    let mut source = gpt_mesh::Document::default();
    source.execute(serde_json::json!({"command":"object_create_box","name":"Cargo","min":[2,0,0],"max":[3,1,1],"origin":[2,0,0]})).unwrap();
    let exported = source.execute(serde_json::json!({"command":"export","format":"fbx"})).unwrap();
    let bytes = gpt_mesh::base64::decode(exported["fbx_base64"].as_str().unwrap()).unwrap();

    let (server, port) = start_server_at(false, None, false);
    std::fs::write(server.workspace.join("cargo.fbx"), bytes).unwrap();
    let output = invoke(port, &[], r#"{"command":"import_fbx","path":"cargo.fbx","response":"full"}"#);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    let response = reply(&output);
    assert_eq!(response["result"]["operation"]["vertices"], 16);
    assert_eq!(response["result"]["operation"]["faces"], 12);
    let mesh = &response["result"]["mesh"];
    assert_eq!(mesh["objects"]["2"]["name"], "Cargo");
    assert_eq!(mesh["objects"]["2"]["origin"], serde_json::json!([2.0,0.0,0.0]));
    assert_eq!(mesh["vertices"]["9"], serde_json::json!([2.0,0.0,0.0]));
    assert_eq!(mesh["faces"]["7"].as_array().unwrap().len(), 4);
    let saved: Value = serde_json::from_slice(&std::fs::read(server.workspace.join("project.json")).unwrap()).unwrap();
    assert_eq!(&saved["mesh"], mesh);

    std::fs::write(server.workspace.join("broken.fbx"), b"not an FBX file").unwrap();
    let rejected = invoke(port, &[], r#"{"command":"import_fbx","path":"broken.fbx"}"#);
    assert_eq!(rejected.status.code(), Some(1));
    let (_, state) = get_json(port, "/state");
    assert_eq!(&state["result"]["mesh"], mesh);
    let undone = invoke(port, &[], r#"{"command":"undo","response":"full"}"#);
    assert_eq!(reply(&undone)["result"]["mesh"]["objects"], serde_json::json!({}));
}

#[test]
fn cli_imports_ascii_fbx_with_units_and_mirrored_winding() {
    let (server, port) = start_server_at(false, None, false);
    let ascii = r#"; FBX 7.4.0 project file
FBXHeaderExtension: {
    FBXHeaderVersion: 1003
    FBXVersion: 7400
}
GlobalSettings: {
    Properties70: {
        P: "UpAxis", "int", "Integer", "", 1
        P: "UpAxisSign", "int", "Integer", "", 1
        P: "FrontAxis", "int", "Integer", "", 2
        P: "FrontAxisSign", "int", "Integer", "", 1
        P: "CoordAxis", "int", "Integer", "", 0
        P: "CoordAxisSign", "int", "Integer", "", 1
        P: "UnitScaleFactor", "double", "Number", "", 1
    }
}
Objects: {
    Geometry: 100, "Geometry::Panel", "Mesh" {
        Vertices: *9 { a: 0,0,0,100,0,0,0,100,0 }
        PolygonVertexIndex: *6 { a: 0,1,-3,0,1,-1 }
    }
    Model: 200, "Model::Panel", "Mesh" {
        Properties70: {
            P: "Lcl Translation", "Lcl Translation", "", "A", 200,0,0
            P: "Lcl Scaling", "Lcl Scaling", "", "A", -1,1,1
        }
    }
}
Connections: {
    C: "OO",100,200
    C: "OO",200,0
}
"#;
    std::fs::write(server.workspace.join("panel.fbx"), ascii).unwrap();
    let output = invoke(port, &[], r#"{"command":"import_fbx","path":"panel.fbx","response":"full"}"#);
    let response = reply(&output);
    assert!(output.status.success(), "{response}");
    let mesh = &response["result"]["mesh"];
    assert_eq!(mesh["objects"]["1"]["name"], "Panel");
    assert_eq!(mesh["objects"]["1"]["origin"], serde_json::json!([2.0, 0.0, 0.0]));
    assert_eq!(mesh["vertices"]["2"], serde_json::json!([1.0, 0.0, 0.0]));
    assert_eq!(mesh["faces"]["1"], serde_json::json!([1, 3, 2]));
    assert_eq!(mesh["faces"].as_object().unwrap().len(), 1);
}

#[test]
fn binary_stl_export_works_with_compact_and_full_replies() {
    let (server, port) = start_server_at(false, None, false);
    let create = invoke(port, &[], r#"{"command":"object_create_box","name":"Part","min":[2,0,-0.25],"max":[3,0.5,0.75],"origin":[2,0,0]}"#);
    assert_eq!(reply(&create)["ok"], true);
    for requirement in ["intent", "reference", "primary_forms", "views", "export_validation"] {
        let request = serde_json::json!({
            "command":"workflow_waive",
            "requirement":requirement,
            "reason":"Procedural box fixture for testing the STL file layout."
        });
        let waived = reply(&invoke(port, &[], &request.to_string()));
        assert_eq!(waived["ok"], true, "{waived}");
    }

    let path = server.workspace.join("part.stl");
    let output_path = path.to_str().unwrap();
    let compact = reply(&invoke(port, &["--output", output_path], r#"{"command":"export"}"#));
    assert_eq!(compact["ok"], true, "{compact}");
    assert_eq!(compact["result"]["artifacts"]["stl"]["bytes"], 684);
    assert!(compact["result"]["stl_base64"].is_null());

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), 84 + 12 * 50);
    assert_eq!(u32::from_le_bytes(bytes[80..84].try_into().unwrap()), 12);
    let mut bounds = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for facet in bytes[84..].chunks_exact(50) {
        let values: Vec<_> = facet[..48]
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect();
        let normal_length_squared: f32 = values[..3].iter().map(|value| value * value).sum();
        assert_eq!(normal_length_squared, 1.0);
        assert_eq!(facet[48..], [0, 0]);
        for vertex in values[3..].chunks_exact(3) {
            for axis in 0..3 {
                bounds.0[axis] = bounds.0[axis].min(vertex[axis]);
                bounds.1[axis] = bounds.1[axis].max(vertex[axis]);
            }
        }
    }
    assert_eq!(bounds, ([2000.0, -750.0, 0.0], [3000.0, 250.0, 500.0]));

    let full = reply(&invoke(port, &["--output", output_path], r#"{"command":"export","response":"full"}"#));
    assert_eq!(full["ok"], true, "{full}");
    assert_eq!(full["result"]["unit"], "millimeter");
    assert_eq!(full["result"]["up_axis"], "Z");
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn camera_only_review_and_preview_commit_work_through_cli() {
    let (_server, port) = start_server(false);
    let send = |input: Value| reply(&invoke(port, &[], &input.to_string()));
    let preview = send(serde_json::json!({
        "command":"transaction", "commit":false,
        "commands":[{"command":"object_move","object":1,"value":[2,0,0]}],
        "preview":{"width":64,"height":64}
    }));
    assert_eq!(preview["ok"], true);
    assert!(preview.get("turn_handoff").is_none());
    let request = serde_json::json!({
        "command":"transaction", "commit":true,
        "preview_id":preview["result"]["preview_id"], "revision":preview["result"]["revision"]
    });
    let defined = send(serde_json::json!({
        "command":"review_define","name":"approach",
        "view":{"width":64,"height":64,"yaw":24},
        "dimensions":[{"object":1}],"wireframe":false,"defects":false
    }));
    assert_eq!(defined["ok"], true);
    let reviewed = send(serde_json::json!({"command":"review_run","name":"approach","preset_version":1}));
    assert_eq!(reviewed["ok"], true, "{reviewed}");
    let result = &reviewed["result"];
    assert_eq!(result["artifacts"].as_object().unwrap().len(), 2);
    assert!(result["artifacts"]["neutral"]["path"].is_string());
    assert_eq!(result["dimensions"][0]["bounds"]["min"], serde_json::json!([-1.0,-1.0,-1.0]));

    let committed = send(request.clone());
    assert_eq!(committed["ok"], true, "{committed}");
    let base = preview["result"]["revision"].as_u64().unwrap();
    assert_eq!(committed["result"]["revision"], base + 1);
    assert_eq!(committed["turn_handoff"]["revision"], base + 1);
    assert_eq!(committed["turn_handoff"]["workflow_status_command"]["command"], "workflow_status");
    assert!(committed["result"].get("defects_after").is_none());
    assert_eq!(send(request)["ok"], false);
    let state = send(serde_json::json!({"command":"query_mesh","kind":"objects"}));
    assert_eq!(state["result"]["items"][0]["origin"], serde_json::json!([2.0,0.0,0.0]));
    assert!(state.get("turn_handoff").is_none());
    assert_eq!(send(serde_json::json!({"command":"undo"}))["ok"], true);
    let state = send(serde_json::json!({"command":"query_mesh","kind":"objects"}));
    assert_eq!(state["result"]["items"][0]["origin"], serde_json::json!([0.0,0.0,0.0]));
    let selected = send(serde_json::json!({"command":"select","vertices":[1]}));
    assert_eq!(selected["ok"], true);
    assert!(selected.get("turn_handoff").is_none());
}

#[test]
fn empty_editor_accepts_first_object_and_persists_last_object_deletion() {
    let (mut server, port) = start_server_at(false, None, false);
    let send = |input| reply(&invoke(port, &[], input));
    let initial = send(r#"{"command":"inspect"}"#);
    assert_eq!(initial["result"]["counts"]["objects"], 0);
    assert!(initial.get("trace").is_none());
    let capture = send(r#"{"command":"capture","width":64,"height":64}"#);
    assert_eq!(capture["ok"], true);
    let record = capture["trace"]["record"].as_str().unwrap();
    let (status, event) = get_json(port, record);
    assert_eq!(status, 200);
    assert_eq!(event["input"]["command"], "capture");
    let session = capture["trace"]["session"].as_str().unwrap();
    let snapshot = event["summary"]["snapshot"].as_u64().unwrap();
    assert_eq!(get_json(port, &format!("/sessions/{session}/states/{snapshot}")).0, 200);
    let created = send(r#"{"command":"object_create_box","name":"Box","min":[0,0,0],"max":[1,1,1]}"#);
    assert_eq!(created["ok"], true);
    assert_eq!(send(r#"{"command":"object_instance","object":1,"name":"Copy"}"#)["ok"], true);
    assert_eq!(send(r#"{"command":"uv_project","mode":"box","masters_only":true}"#)["ok"], true);
    assert_eq!(send(r#"{"command":"inspect_uv"}"#)["result"]["missing_uv_faces"], 0);
    assert_eq!(send(r#"{"command":"object_delete","object":2}"#)["ok"], true);
    assert_eq!(send(r#"{"command":"object_delete","object":1}"#)["ok"], true);
    assert_eq!(send(r#"{"command":"undo"}"#)["ok"], true);
    assert_eq!(send(r#"{"command":"inspect"}"#)["result"]["counts"]["objects"], 1);
    assert_eq!(send(r#"{"command":"redo"}"#)["ok"], true);

    server.child.kill().unwrap();
    server.child.wait().unwrap();
    let reopened = gpt_mesh::session::Session::open_workspace(server.workspace.clone()).unwrap();
    assert_eq!(reopened.state()["mesh"]["objects"], serde_json::json!({}));
    assert_eq!(reopened.state()["mesh"]["object_id_high_watermark"], 2);
}

#[test]
fn persistence_failure_reports_applied_revision_and_stops_batch() {
    let (server, port) = start_server_at(false, None, false);
    let project = server.workspace.join("project.json");
    let original = std::fs::read(&project).unwrap();
    std::fs::create_dir(server.workspace.join("project.tmp")).unwrap();

    let output = invoke(port, &[], r#"{"command":"object_create_box","name":"First","min":[0,0,0],"max":[1,1,1]}"#);
    assert_eq!(output.status.code(), Some(1));
    let response = reply(&output);
    assert_eq!(response["ok"], true);
    assert!(response["recording_error"].is_string());
    assert_eq!(response["persistence"]["revision_changed_in_memory"], true);
    assert_eq!(response["persistence"]["project_saved"], false);
    assert_eq!(std::fs::read(&project).unwrap(), original);

    let output = invoke(port, &["--batch"], r#"[
        {"command":"object_create_box","name":"Second","min":[0,0,0],"max":[1,1,1]},
        {"command":"object_create_box","name":"Skipped","min":[0,0,0],"max":[1,1,1]}
    ]"#);
    assert_eq!(output.status.code(), Some(1));
    let response = reply(&output);
    assert_eq!(response["ok"], true);
    assert_eq!(response["persistence"]["project_saved"], false);
    let (_, state) = get_json(port, "/state");
    assert_eq!(state["result"]["revision"], response["persistence"]["revision"]);
    assert_eq!(state["result"]["mesh"]["objects"].as_object().unwrap().len(), 2);
    assert_eq!(std::fs::read(&project).unwrap(), original);
}

#[test]
fn timed_out_write_reports_the_id_sent_to_the_server() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let client = std::thread::spawn(move || invoke(port, &["--timeout", "1"], r#"{"command":"cube"}"#));
    let (stream, _) = listener.accept().unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reader = BufReader::new(stream);
    let mut length = 0;
    loop {
        let mut header = String::new();
        assert_ne!(reader.read_line(&mut header).unwrap(), 0);
        if header == "\r\n" {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length: ") {
            length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let request: Value = serde_json::from_slice(&body).unwrap();
    let id = request["cli_request_id"].as_str().unwrap();

    let result = client.join().unwrap();
    assert!(!result.status.success());
    let failure = reply(&result);
    assert_eq!(failure["ok"], false);
    assert!(failure["error"].as_str().unwrap().contains(id));
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(error.contains("could not confirm the command outcome"));
    assert!(error.contains("Check the current workspace before retrying"));
    assert!(error.contains("If the editor was started with --debug"));
    assert!(error.contains(id));
}

#[test]
fn debug_replay_preserves_request_ids_and_requires_saved_snapshots() {
    let (server, port) = start_server_with_options(false, None, false, &["--debug"]);
    let output = invoke(port, &["--debug"], r#"{"command":"inspect"}"#);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let response = reply(&output);
    let (_, event) = get_json(port, response["trace"]["record"].as_str().unwrap());
    let id = event["input"]["cli_request_id"].as_str().unwrap();

    let recordings = server.workspace.join("recordings");
    let recording = std::fs::read_dir(recordings).unwrap().next().unwrap().unwrap().path();
    let replay = |frame: &str| {
        Command::new(env!("CARGO_BIN_EXE_atk-mesh-cli"))
            .arg("--replay")
            .arg(&recording)
            .args(["--frame", frame])
            .output()
            .unwrap()
    };
    let timeline = std::fs::read_to_string(&recording).unwrap();
    let request: Value = serde_json::from_str(timeline.lines().nth(1).unwrap()).unwrap();
    assert_eq!(request["request"]["cli_request_id"], id);
    let saved = replay("1");
    assert!(saved.status.success());
    assert_eq!(reply(&saved)["revision"], 0);

    let mut file = std::fs::OpenOptions::new().append(true).open(&recording).unwrap();
    writeln!(file, r#"{{"type":"reply","sequence":2,"reply":{{"ok":false}},"state":null}}"#).unwrap();
    drop(file);
    let missing = replay("2");
    assert_eq!(missing.status.code(), Some(1));
    let failure = reply(&missing);
    assert_eq!(failure["ok"], false);
    assert_eq!(failure["error"], "Recording has no saved snapshot for frame 2");
}

#[test]
fn installed_binaries_support_project_workspaces_and_structured_failures() {
    let (server, port) = start_server(false);
    assert_eq!(get(port, "/").0, 404);
    assert_eq!(get(port, "/app.js").0, 404);

    let discovered = invoke(port, &[], r#"{"command":"help"}"#);
    assert!(discovered.status.success());
    assert_eq!(
        reply(&discovered)["result"]["tasks"]["model_geometry"][0],
        "workflow"
    );
    let catalog = invoke(port, &[], r#"{"command":"help","all":true}"#);
    assert!(catalog.status.success());
    assert_eq!(
        reply(&catalog)["result"]["topics"]["batch-commands"],
        "Batch commands"
    );
    let guidance = invoke(port, &[], r#"{"command":"help","topic":"workflow"}"#);
    assert!(guidance.status.success());
    assert!(
        reply(&guidance)["result"]["text"]
            .as_str()
            .unwrap()
            .contains("workflow_status")
    );

    let status = invoke(port, &[], r#"{"command":"workflow_status"}"#);
    assert!(status.status.success());
    assert_eq!(reply(&status)["ok"], true);

    let batched = invoke(
        port,
        &["--batch"],
        r#"[{"command":"uv_project","mode":"box"},{"command":"vertex_color_gradient","start":[0,-1,0],"end":[0,1,0],"from":[1,0,0],"to":[0,0,1]},{"command":"inspect_uv"}]"#,
    );
    assert!(
        batched.status.success(),
        "{}",
        String::from_utf8_lossy(&batched.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(batched.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[2]["result"]["missing_uv_faces"], 0);
    let failed_batch = invoke(
        port,
        &["--batch"],
        r#"[{"command":"uv_clear"},{"command":"uv_project","mode":"invalid"},{"command":"uv_project","mode":"box"}]"#,
    );
    assert!(!failed_batch.status.success());
    let replies: Vec<Value> = String::from_utf8(failed_batch.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["ok"], true);
    assert_eq!(replies[1]["ok"], false);
    let queried = invoke(port, &[], r#"{"command":"inspect_uv"}"#);
    assert_eq!(reply(&queried)["result"]["missing_uv_faces"], 6);
    let invalid_batch = invoke(
        port,
        &["--batch"],
        r#"[{"command":"uv_project","mode":"box"},{}]"#,
    );
    assert!(!invalid_batch.status.success());
    assert_eq!(reply(&invalid_batch)["ok"], false);

    let input = r#"[{"command":"uv_project","mode":"box"},{}]"#;
    let mut connection = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        connection,
        "POST /command HTTP/1.0\r\nContent-Length: {}\r\n\r\n{input}",
        input.len()
    )
    .unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.0 400 "));
    let rejected: Value = serde_json::from_str(body).unwrap();
    assert_eq!(rejected["ok"], false);
    assert_eq!(rejected["failed_command"], 2);
    assert_eq!(rejected["replies"], serde_json::json!([]));

    let queried = invoke(port, &[], r#"{"command":"inspect_uv"}"#);
    assert_eq!(reply(&queried)["result"]["missing_uv_faces"], 6);

    let input = server.workspace.join("inspect.json");
    std::fs::write(
        &input,
        "\u{feff}{\"command\":\"query_mesh\",\"kind\":\"objects\"}",
    )
    .unwrap();
    let inspected = invoke(port, &["--input", input.to_str().unwrap()], "");
    assert!(inspected.status.success());
    assert_eq!(reply(&inspected)["ok"], true);

    let destination = server.workspace.join("saved.json");
    let saved = invoke(
        port,
        &[r#"{"command":"save"}"#, destination.to_str().unwrap()],
        "",
    );
    assert!(saved.status.success());
    assert!(reply(&saved)["written"]["bytes"].as_u64().unwrap() > 0);
    let project: Value = serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
    assert!(project["mesh"].is_object());

    let capture = server.workspace.join("capture.png");
    let captured = invoke(
        port,
        &["-", "--output", capture.to_str().unwrap()],
        r#"{"command":"capture","width":160,"height":120}"#,
    );
    assert!(
        captured.status.success(),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    assert!(
        std::fs::read(capture)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    let layout = server.workspace.join("uv.svg");
    let captured = invoke(
        port,
        &["--output", layout.to_str().unwrap()],
        r#"{"command":"capture_uv","size":64}"#,
    );
    assert!(captured.status.success());
    assert!(std::fs::read_to_string(layout).unwrap().starts_with("<svg"));
    let texture = invoke(
        port,
        &[],
        r#"{"command":"texture_define","name":"red","width":1,"height":1,"rgba_base64":"/wAA/w=="}"#,
    );
    assert!(texture.status.success());
    let output = server.workspace.join("texture.png");
    let exported = invoke(
        port,
        &["--output", output.to_str().unwrap()],
        r#"{"command":"texture_export","texture":"red"}"#,
    );
    assert!(exported.status.success());
    assert!(
        std::fs::read(output)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );

    let failed = invoke(
        port,
        &["--output", destination.to_str().unwrap()],
        r#"{"command":"export"}"#,
    );
    assert!(!failed.status.success());
    assert_eq!(reply(&failed)["ok"], false);
    assert_eq!(reply(&failed)["error"], "requirements_missing");
    let unchanged: Value = serde_json::from_slice(&std::fs::read(destination).unwrap()).unwrap();
    assert_eq!(unchanged, project);

    for input in ["", "[]", "null", r#"{"command":2}"#] {
        let invalid = invoke(port, &[], input);
        assert!(!invalid.status.success());
        assert_eq!(reply(&invalid)["ok"], false);
    }

    let edited = invoke(port, &[], r#"{"command":"select","vertices":[1]}"#);
    assert!(edited.status.success());
    let moved = invoke(port, &[], r#"{"command":"move","value":[0,0.1,0]}"#);
    assert!(moved.status.success());
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(server.workspace.join("project.json")).unwrap())
            .unwrap();
    assert_ne!(persisted["mesh"], project["mesh"]);
}

#[test]
fn project_viewer_reads_current_revisions_from_independent_editors() {
    let (viewer, viewer_port) = start_server(true);
    let (status, page) = get(viewer_port, "/");
    assert_eq!(status, 200);
    assert!(page.contains("workspace-picker"));
    assert_eq!(get(viewer_port, "/app.js").0, 200);
    let first = ".mesh/Left & Café";
    let second = "models/right";
    let first_query = "?workspace=.mesh%2FLeft+%26+Caf%C3%A9";
    let second_query = "?workspace=models%2Fright";

    let source = gpt_mesh::Document::default()
        .execute(serde_json::json!({"command":"save"}))
        .unwrap();
    let first_path = viewer.workspace.join(first);
    std::fs::create_dir_all(&first_path).unwrap();
    let source = serde_json::to_vec(&source).unwrap();
    std::fs::write(first_path.join("project.json"), &source).unwrap();
    std::fs::create_dir(viewer.workspace.join("other")).unwrap();
    std::fs::write(viewer.workspace.join("other/project.json"), "{}").unwrap();

    let list = invoke(viewer_port, &["--list-workspaces"], "");
    assert!(list.status.success());
    assert_eq!(
        reply(&list)["result"]["workspaces"],
        serde_json::json!([{"path":first}])
    );
    let first_state_path = format!("/state{first_query}");
    let (status, headers, saved) = get_with_headers(viewer_port, &first_state_path, "");
    assert_eq!(status, 200);
    let etag = headers
        .lines()
        .find_map(|line| line.strip_prefix("ETag: "))
        .unwrap();
    let (status, metadata) = get_json(viewer_port, &format!("/state/meta{first_query}"));
    assert_eq!(status, 200);
    assert_eq!(metadata, serde_json::json!({"ok":true,"result":{"tag":etag}}));
    let revalidate = format!("If-None-Match: {etag}\r\n");
    let (status, _, unchanged) = get_with_headers(viewer_port, &first_state_path, &revalidate);
    assert_eq!(status, 304);
    assert!(unchanged.is_empty());
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert!(saved["result"]["revision"].is_null());
    let (_, listing) = get_json(viewer_port, &format!("/sessions{first_query}"));
    assert_eq!(listing["sessions"], serde_json::json!([]));
    assert!(!first_path.join("sessions").exists());
    assert_eq!(
        std::fs::read(first_path.join("project.json")).unwrap(),
        source
    );

    let rejected = invoke(viewer_port, &[], r#"{"command":"move","value":[0,1,0]}"#);
    assert!(!rejected.status.success());
    assert!(
        reply(&rejected)["error"]
            .as_str()
            .unwrap()
            .contains("read-only")
    );
    assert_eq!(
        get_json(viewer_port, "/state?workspace=..%2Foutside").0,
        400
    );

    let (mut first_editor, first_port) =
        start_server_with_options(false, Some(first_path.clone()), false, &["--debug"]);
    let second_path = viewer.workspace.join(second);
    let (_second_editor, second_port) = start_server_at(false, Some(second_path.clone()), true);
    let lock = std::fs::File::options()
        .read(true)
        .write(true)
        .open(first_path.join(".mesh-editor.lock"))
        .unwrap();
    assert!(lock.try_lock().is_err());

    let (edited, other) = std::thread::scope(|thread| {
        let other = thread.spawn(|| {
            invoke(
                second_port,
                &["--batch"],
                r#"[{"command":"select","vertices":[1]},{"command":"move","value":[0,-1,0]}]"#,
            )
        });
        let edited = invoke(
            first_port,
            &["--batch"],
            r#"[{"command":"select","vertices":[1]},{"command":"move","value":[0,1,0]}]"#,
        );
        (edited, other.join().unwrap())
    });
    assert!(
        edited.status.success(),
        "{}",
        String::from_utf8_lossy(&edited.stderr)
    );
    assert!(other.status.success());
    assert!(!second_path.join("sessions").exists());
    let lines = String::from_utf8(edited.stdout).unwrap();
    let last: Value = serde_json::from_str(lines.lines().last().unwrap()).unwrap();
    let record = last["trace"]["record"].as_str().unwrap();
    let (status, event) = get_json(viewer_port, &format!("{record}{first_query}"));
    assert_eq!(status, 200);
    assert_eq!(event["input"]["command"], "move");
    assert_eq!(
        get_json(viewer_port, &format!("{record}{second_query}")).0,
        404
    );

    let (status, headers, first_state) =
        get_with_headers(viewer_port, &first_state_path, &revalidate);
    assert_eq!(status, 200);
    assert!(
        headers
            .lines()
            .all(|line| line.strip_prefix("ETag: ") != Some(etag))
    );
    let first_state: Value = serde_json::from_str(&first_state).unwrap();
    let (_, second_state) = get_json(viewer_port, &format!("/state{second_query}"));
    assert_eq!(
        first_state["result"]["revision"],
        last["result"]["revision"]
    );
    let original_y = saved["result"]["mesh"]["vertices"]["1"][1]
        .as_f64()
        .unwrap();
    assert_eq!(
        first_state["result"]["mesh"]["vertices"]["1"][1],
        original_y + 1.0
    );
    assert_eq!(
        second_state["result"]["mesh"]["vertices"]["1"][1],
        original_y - 1.0
    );

    first_editor.child.kill().unwrap();
    first_editor.child.wait().unwrap();
    assert_eq!(
        get_json(viewer_port, &format!("/state{first_query}")).1,
        first_state
    );
    assert_eq!(
        get_json(viewer_port, &format!("{record}{first_query}")).1,
        event
    );
}

#[test]
fn help_version_and_invalid_options_do_not_need_a_server() {
    for binary in [
        env!("CARGO_BIN_EXE_atk-mesh-cli"),
        env!("CARGO_BIN_EXE_atk-mesh"),
    ] {
        for flag in ["--help", "--version"] {
            let output = Command::new(binary)
                .env("MESH_PORT", "invalid")
                .arg(flag)
                .output()
                .unwrap();
            assert!(output.status.success());
            assert!(!output.stdout.is_empty());
            if flag == "--help" {
                let help = String::from_utf8(output.stdout).unwrap();
                assert!(help.contains("command\":\"help"));
                assert!(help.contains("workflow"));
                if binary == env!("CARGO_BIN_EXE_atk-mesh") {
                    assert!(help.contains("--headless"));
                }
            }
        }

        for arguments in [
            vec!["--unknown"],
            vec!["--port"],
            vec!["--port", "0"],
            vec!["--port", "70000"],
            vec!["--port", "7878", "--port", "7879"],
        ] {
            let output = Command::new(binary)
                .args(arguments)
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(!output.stderr.is_empty());
        }
    }

    let extra = invoke(1, &["{}", "one", "two"], "");
    assert!(!extra.status.success());
    let duplicate = invoke(1, &["{}", "one", "--output", "two"], "");
    assert!(!duplicate.status.success());
}
