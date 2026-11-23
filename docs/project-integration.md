# Using Mesh Workbench in a project

Install the tools using the [README](../README.md#install). Keep the tool source checkout separate from your consuming project. The consuming project needs the installed binaries and a model workspace directory.

## Project structure and responsibilities

| Component | Location | Responsibility |
| --- | --- | --- |
| Rust model library | [`src/lib.rs`](../src/lib.rs) and feature modules under `src/` | Owns mesh geometry, UVs, textures, rigs, clips, and model history. Validates edits and computes queries, sampled poses, captures, and exports. |
| Editor session | [`src/session.rs`](../src/session.rs) and [`src/workflow.rs`](../src/workflow.rs) | Runs commands against one model, owns workflow checks, saves native projects, and records command replies, snapshots, and artifacts. |
| Server | [`src/server.rs`](../src/server.rs) and [`src/workspace_host.rs`](../src/workspace_host.rs) | Runs in editor mode for one locked workspace or in project viewer mode for a project root. Editor mode accepts JSON commands. Project viewer mode serves the web assets, saved model state, and replay history. |
| CLI | [`src/cli.rs`](../src/cli.rs) | Reads JSON from arguments, files, or stdin, sends it to the selected server, and returns structured replies and exit status. Copies requested artifacts to local output paths. |
| Browser viewer | [`web/`](../web/) | Reads saved state and replay records. Owns the viewport camera, projection, display modes, visibility filters, and playback controls. Stores view preferences in the browser and renders the model with WebGL. |
| Tests | [`tests/`](../tests/) | Rust tests cover commands, persistence, captures, and exports. `viewer.mjs` covers browser rendering data, interaction, and replay. |

An agent sends a JSON command through `mesh-cli` to an editor port. The editor validates the command, updates its model, and saves the resulting revision. The project viewer reads that saved revision on its own port. During replay it displays recorded snapshots, then returns to the latest saved state when the user exits replay.

### Where new features belong

- Implement anything that changes the saved model or affects exports and scripted results in the Rust model library. Expose it through the editor's JSON command API so agents and scripts can use the same operation.
- Keep CLI-specific work in the client. Flags, stdin, exit status, and local output paths belong here.
- Keep inspection and navigation in the viewer. The axis gizmo and projection toggle control the browser camera. CLI captures take explicit camera parameters, so the same request can be reproduced independently of the browser.
- Keep preview computation in the browser when it supports interactive playback or rendering. Rust and JavaScript each sample animation and render geometry for their respective outputs. When a behavior is shared, such as clip interpolation, UV orientation, or alpha cutout, define its semantics in the model or command contract and test both paths on representative inputs.
- If a future browser control edits a mesh, have it call the editor command API. The editor remains responsible for validation, undo, persistence, and recording.

## Start the project viewer and mesh editors

From your project root:

```powershell
mesh-server --port 7878
```

The viewer uses its working directory as the project root. Pass `--project ROOT` to choose another root. Open http://127.0.0.1:7878 in your browser. Click the folder button or press Ctrl+P to search for a workspace. Use the arrow keys and Enter to select a result. Each tab shows the latest saved revision of its selected mesh and can replay its recorded commands.

List discovered meshes with `mesh-cli --port 7878 --list-workspaces`. To edit a mesh, start a separate editor in another terminal. A new workspace starts empty with pending modeling requirements.

```powershell
mesh-server --workspace .mesh/model --port 7879
```

Send commands to port 7879 for this mesh. Give each additional editor its own workspace and port so the processes can edit in parallel. Each editor locks its workspace for the duration of the process. Use Ctrl+C to stop an editor. The project viewer can still show its saved mesh and replay.

The project viewer prints its browser URL. Each editor prints the address of its JSON API. The former `--headless` flag is accepted for compatibility.

The server binds only to localhost. The CLI connects to localhost and copies artifacts using local file paths; run both on the same machine. It is a local development tool with no authentication. Use one command at a time when a sequence depends on returned IDs or revisions.

## Give Codex project instructions

Add this section to the consuming project's `AGENTS.md`, adjusting the directory and port for that project:

```markdown
## Mesh editing

- Use the installed `mesh-server` and `mesh-cli` tools for modeling.
- Use the read-only project viewer on port 7878. Discover meshes with `mesh-cli --port 7878 --list-workspaces`.
- Reuse an editor for the intended mesh if one is running. Otherwise, start
  `mesh-server --workspace PATH --port PORT` with a free port.
- Send commands to that editor via `mesh-cli --port PORT`. Use `mesh-cli --port PORT --batch --input FILE` for a prepared JSON array. The batch uses one CLI invocation and one HTTP request. Split the sequence where a later command needs a returned ID, revision, or review result.
- Discover the tool with `help`, then read `help` with `topic:"workflow"`.
  Read `topic:"command-api"` for commands; continue topic pages using `next_offset`.
- Begin with `workflow_status`, then `query_mesh` to identify current objects and IDs.
- Preserve existing geometry and IDs; use native commands for localized edits.
- Read reference images and captures before recording workflow assessments.
- Register references with `workflow_resource`; use `reference_landmarks`,
  `fit_reference` and `project_reference` when matching image landmarks.
- Declare `workflow_stage` transitions through primary, detail and complete.
  Follow `requirements_missing` instructions. Waive only genuinely inapplicable
  requirements with a specific reason; never waive solely to unblock an export.
- Capture and inspect distinct views to verify forms, thickness and clearances.
- Use trace session/event IDs when registering capture evidence. Refresh checks
  invalidated by edits. Keep resources and sessions with the model workspace.
- Keep each session focused on geometry, texturing, or integration. Save the native
  project and review views for modeling and texturing. Export and verify the
  import in an integration session when the user requests it.
- Check the CLI exit status and JSON `ok` value; inspect structured failure details.
```

The command reference is in the tool's [README](../README.md#modeling-requirements). Codex can inspect objects and their vertices:

```powershell
'{"command":"query_mesh","kind":"objects"}' | mesh-cli --port 7879
'{"command":"query_mesh","kind":"vertices","object":1}' | mesh-cli --port 7879
```

Object `1` is illustrative; use the actual returned ID. Add `revision` to editing requests when you need to reject edits against a changed document.

## Input, output and errors

`mesh-cli` accepts a single JSON argument, piped stdin, or a command file:

```powershell
mesh-cli --port 7879 --input commands/capture.json --output preview.png
'{"command":"save"}' | mesh-cli --port 7879 --output geometry.json
'{"command":"export"}' | mesh-cli --port 7879 --output model.fbx
```

The capture file contains one JSON object such as `{"command":"capture","yaw":35,"pitch":25}`. UTF-8 files with or without a BOM are supported. `--input -` or a positional `-` explicitly reads stdin. The legacy `mesh-cli JSON OUTPUT` form remains supported. Relative command and output paths resolve from the client's working directory. Create destination directories before saving.

Use `mesh-cli --port 7879 --batch --input commands/recipe.json` for a JSON array of 1–1024 commands. Piped stdin works with `--batch` too. The editor runs the array in order in one request and stops at the first failed command. Earlier edits remain applied. Read stdout as one JSON reply per attempted command, including a failed reply when present. Use the artifact paths in those replies for batch captures and saves. Use `--output` with a single command. Direct HTTP clients can post the same array to the editor's `/command` endpoint and read the ordered `replies` array. See the [command API](../README.md#command-api) for the HTTP response format.

Port selection is `--port`, then `MESH_PORT`, then 7878. Ports must be 1–65535. `mesh-server --help` and `mesh-cli --help` describe their options; `--version` reports the installed version.

Successful commands exit with status 0 and print a JSON response on stdout. Failed server commands exit with status 1 and still print their JSON response, including workflow requirements and trace information, even when an output file was requested. Input, connection and local output failures exit with status 1 and write diagnostics to stderr; these failures may have no JSON response. A failed server command does not write the requested output file. An output copy failure can occur after the server has successfully executed and recorded a command, so inspect the result before retrying an edit.

Artifact extensions select the output: `.png` saves capture pixels; other capture paths save SVG; `.fbx` requests FBX export; `export` with `format:"obj"` saves OBJ. `save` writes the native mesh and appearance data. Other commands write result JSON. Existing output files are overwritten on success. Responses are compact by default; use paged `query_mesh` / `query_record` reads or `response:"full"` for exact detailed results.

For scripting, use stdin and check the exit code before consuming a successful result:

```python
import json
import subprocess

process = subprocess.run(
    ["mesh-cli", "--port", "7879"],
    input=json.dumps({"command": "workflow_status"}),
    text=True,
    capture_output=True,
)
if process.returncode:
    raise RuntimeError(process.stdout or process.stderr)
response = json.loads(process.stdout)
```

## Keep and back up your work

Keep `.mesh/model/project.json`, `workflow.json`, reference/evidence files and `sessions/` together. Geometry and workflow decisions persist across server restarts; in-memory undo/redo does not. Copy the whole model workspace to preserve replay and evidence. A standalone `save` output contains native mesh and appearance data, without workflow decisions or replay.

Choose whether to version the workspace in your project or back it up separately. Recordings and captures can grow substantially; there is no automatic retention limit. Keep exported FBX/OBJ files in your normal asset pipeline. Import accepts static GLB and FBX geometry from files inside the workspace. FBX import converts units to meters. For supported features and limits, see the [command reference](command-reference.md).

## Current limits

If the shell runner cannot launch a new process but an existing editor and Node session remain available, send the same JSON to the editor's HTTP endpoint:

```javascript
const response = await fetch("http://127.0.0.1:7879/command", {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({command: "workflow_status"}),
});
const reply = await response.json();
if (!response.ok || reply.ok !== true) {
    throw new Error(JSON.stringify(reply));
}
console.log(reply);
```

This is an alternative transport to the same server; workflow gates, recordings and artifact paths still apply. Direct HTTP callers receive artifact paths and must copy files themselves when needed. A cold-start run on one Windows host observed new shell/child-process launches failing with Access denied / EPERM after server launch; that observation does not establish a mesh-server defect or its cause. No fix or general diagnosis is claimed here.

Each editor handles requests sequentially for its own mesh. The project viewer reads saved revisions independently. Workflow checkpoints require visual review. See the README for topology, rendering, import and export limitations.

For tool development, run `cargo test --locked` and `npm test` in the tool checkout. The Rust CLI integration tests start an isolated server and temporary workspace. Legacy Python smoke scripts can replace the active model, so run them only against a disposable workspace on a separate port.

For large assets, start the editor with `--compress-recordings` to store new replay states and events as gzip JSON. The project and session index stay directly readable. The current CLI and project viewer resolve either encoding on the usual replay routes. Use an updated viewer to read compressed sessions.

Run `cargo run --release --locked --example bench_dense -- NEW_DIRECTORY 100000` to time workspace open, a sculpt stroke, capture, undo, save and reopen on a triangular grid. Use `1000000` for the larger fixture and append `--compress-recordings` for a storage comparison. Each run creates a fresh directory and prints elapsed milliseconds and cumulative on-disk bytes.
