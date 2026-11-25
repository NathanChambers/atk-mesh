---
name: atk-mesh
description: Create, edit, inspect, render, or export 3D meshes in Mesh Workbench. Use for modeling requests in a workbench project, including turning existing artwork into a 3D asset, or when the user names atk-mesh.
---

# atk-mesh

When this skill is invoked, use the Mesh Workbench CLI command workflow for the requested work unless the user explicitly specifies another method. For modeling tasks, author the mesh in the target workspace with CLI modeling commands. Scripts may prepare and submit CLI command batches. If the CLI cannot perform a required operation, explain the limitation and ask before switching to external authoring.

## Select the workspace

- When modeling from an existing design, look for editable source in the consuming project. For ATK Vector, look under `.vector/sessions/` and pass the project and session to `atk-vector-cli`. Use `inspect` for part IDs, `bounds` for proportions, and `contours` for outlines. Map artboard points to the chosen model plane and scale before extrusion. Choose depth and bevel in Mesh Workbench. Open paths and holes need different treatment from solid outer polygons.

- Verify that `atk-mesh-cli` and `atk-mesh` are on PATH. If missing, use `cargo install --path . --locked` from the Mesh Workbench source checkout when installation is in scope.
- Use the workspace and editor port specified by the user or project. If a project viewer is running, list saved workspaces with `atk-mesh-cli --port VIEWER_PORT --list-workspaces`.
- Reuse a running editor for the intended workspace. Verify its workspace with `atk-mesh-cli --port EDITOR_PORT --list-workspaces` before editing if the port assignment is uncertain.
- When an editor is needed, run `atk-mesh --workspace PATH --port FREE_PORT` from the consuming project and keep it available for the task. Use a fresh workspace for a new model. If multiple existing workspaces fit the request, resolve the target before editing.
- Pass the editor port explicitly for command requests. When the user wants a browser view, use the read-only project viewer started with `atk-mesh --project ROOT --port VIEWER_PORT` on a different port.

## Discover and use commands

- For unfamiliar work, send `{"command":"help"}`, then read the relevant topics via `{"command":"help","topic":"workflow"}` or `{"command":"help","topic":"command-api"}`. Follow `next_offset` to read additional pages.
- Check `workflow_status` before modeling changes and use `query_mesh` to find current object IDs and revision. Use the returned IDs and revision guards for edits that depend on the inspected state.
- Pass JSON via stdin or `--input FILE`. For example, PowerShell accepts `'{"command":"workflow_status"}' | atk-mesh-cli --port 7879`.
- Use `--batch --input FILE` for prepared JSON arrays. Split batches where later commands depend on returned IDs, revisions, or review evidence. Earlier edits in a failed batch remain applied.
- Check both the exit status and JSON reply. If a request times out, look up its `cli_request_id` in the workspace session recordings before retrying. If persistence fails, check `recording_error` and `persistence` to see whether the edit changed the model in memory and reached `project.json`.

## Review and deliver

- Use captures and measurements to verify the requested changes. Save a PNG with `atk-mesh-cli --port EDITOR_PORT --input CAPTURE_JSON --output PREVIEW.png` and open it when visual evidence matters.
- Follow the workspace's review requirements when advancing stages or exporting. Use actual capture traces to register evidence and waive only requirements that genuinely do not apply.
- At turn end, use the latest `turn_handoff` receipt to report changes, verified evidence, and remaining uncertainty. If it flags an `invalidated_review` relevant to the requested work, rerun the supplied command and inspect its evidence before reporting.
- Report the workspace and any requested artifact paths. Include a viewer link when a project viewer is running.

## Report CLI friction

- When a missing or broken command, documentation gap, or misleading workflow blocks progress, forces a meaningful workaround, or repeatedly misdirects the work, submit one report per distinct issue.
- Pipe a single `feedback_submit` request into `atk-mesh-cli`. Choose `missing_capability`, `command_issue`, `documentation`, or `workflow`. Provide `summary`, `goal`, and `observed`, including what you tried and any workaround. Add `suggestion` or `related` when useful. Omit credentials and sensitive project data.
- Check that the reply has `ok: true` and returns a report ID. The command works without an editor. If submission fails, mention it in your handoff and continue the task where possible.

```json
{"command":"feedback_submit","kind":"workflow","summary":"Document door swing reviews","goal":"Check clearance through the full swing","observed":"The guide covers the closed pose only","related":"help:workflow"}
```
- Add `--debug` to `atk-mesh-cli` when a command log is requested. Read the printed recording path with `--replay FILE` or use `--replay FILE --frame N` to inspect one snapshot.
