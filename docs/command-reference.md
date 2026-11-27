# atk-mesh command reference

A local mesh workspace for iterative AI editing. Rust owns the document; the plain JavaScript browser and JSON CLI use the same command endpoint. No Node build step is needed.

## Install and use in a project

Install both executables from this source directory with a current stable Rust toolchain:

```powershell
cargo install --path . --locked --target-dir target/install
atk-mesh --version
atk-mesh-cli --help
```

Cargo installs them into your Cargo bin directory (usually `~/.cargo/bin`); ensure that directory is on `PATH`. Node and Python are not needed to run the installed tool. The viewer is embedded in the server executable, so the source checkout is not needed at runtime. Rerun the install command after updating the source.

The separate install build directory avoids overwriting binaries running from `target/release`. On Windows, stop installed instances before reinstalling their executables.

Start the read-only viewer from the project root. Use `--project ROOT` to select a different root.

```powershell
atk-mesh --port 7878
```

Open http://127.0.0.1:7878 in your browser. Click the folder icon or press Ctrl+P to search for a mesh workspace. You can also list the discovered directories with `atk-mesh-cli --port 7878 --list-workspaces`.

To edit a mesh, start an API-only editor for its workspace on a different port. The editor creates the workspace if needed and always runs without launching or serving the browser UI.

```powershell
atk-mesh --workspace .mesh/model --port 7879
```

Send commands to that editor from another terminal.

```powershell
'{"command":"help"}' | atk-mesh-cli --port 7879
'{"command":"help","topic":"workflow"}' | atk-mesh-cli --port 7879
'{"command":"workflow_status"}' | atk-mesh-cli --port 7879
'{"command":"query_mesh","kind":"objects"}' | atk-mesh-cli --port 7879
'{"command":"capture","yaw":35,"pitch":25}' | atk-mesh-cli --port 7879 --output preview.png
```

The editor's port selects the mesh for every command or batch. Run each additional editor on its own port to work on multiple meshes in parallel. Each workspace allows one editor at a time.

See [project integration](project-integration.md) for Codex instructions, command files, scripting, backups, and limitations.

## Rigging and animation

A native workspace can store a bone hierarchy, per-vertex skin weights, and named clips. The geometry editor uses the rest mesh. In the viewer header, open **Animation**, then use the search button beside the current clip to choose another clip. Press **Play** or scrub with the timeline slider. **Bones** draws octahedra for authored tails, **Hierarchy** draws parent links, and **Joints** marks the pivots. Toggle mesh visibility to isolate the rig. Switching to **Mesh** in the header shows the rest pose. Returning to **Animation** restores the paused pose.

```json
[
  {"command":"rig_set","rig":{"bones":[{"name":"root","position":[0,0,0]},{"name":"arm","parent":"root","position":[0.2,1.4,0]}]}},
  {"command":"skin_weights_set","weights":{"9":[{"bone":"arm","weight":0.75},{"bone":"root","weight":0.25}]}},
  {"command":"animation_set","name":"Idle","clip":{"duration":2,"looping":true,"tracks":{"root":[{"time":0},{"time":1,"translation":[0,0.01,0],"rotation":[0,3,0]},{"time":2}]}}}
]
```

- `rig_set` replaces the rig, including any existing weights and clips. Supply parents before children. Bone positions are world-space rest pivots with axes aligned to the rest mesh. `rig_clear` removes the rig.
- Supply an optional `tail:[x,y,z]` on a bone to set its display endpoint in world-space rest coordinates. Update an existing rig with `{"command":"rig_display_tails_set","tails":{"arm":[1,1.4,0]}}`. Use null to clear a tail. Tails affect the viewer overlay only.
- Use `{"command":"rig_rotation_limits_set","limits":{"wrist":{"axis":[0,-1,0],"max_swing":55,"twist":[-25,25]}}}` to set local joint limits. The nonzero axis is expressed in the bone's rest axes. Swing limits the tilt away from that axis. Twist is a signed range around it and must include zero. All angles are in degrees. For a hinge, set `max_swing` to zero and use the hinge axis for twist. Use null to clear a limit. Limits apply after clip interpolation in the viewer and pose captures. GLB export bakes constrained motion at 60 Hz, with step tracks kept as step tracks. Authored keys stay editable.
- `skin_weights_set` updates the supplied vertex IDs. Use one to four unique positive influences summing to one, or an empty list to clear that vertex. Unweighted vertices display at rest. Deleted vertices lose their bindings, and new vertices need weights.
- To assign a two-bone blend, use `{"command":"skin_weights_set","object":2,"ramp":{"bones":["upper","lower"],"start":[0,1,0],"end":[0,2,0],"easing":"smoothstep"}}`. Scope it with `object`, a `vertices` list, or the current vertex selection. The command projects rest positions onto the start-to-end axis, clamps both ends to full weight on the corresponding bone, and replaces weights in scope. Choose `linear` or `smoothstep`.
- To bind a rigid part, use `{"command":"skin_weights_set","object":2,"bone":"upper"}`. Every current vertex of the object gets weight 1 on that bone, replacing its previous influences. The command also works on linked instances, with independent weights on each object.
- `animation_set` creates or replaces a named clip. Key times are seconds in strict ascending order. Translation offsets and XYZ rotations (degrees, applied X then Y then Z) are parent-relative. Translation interpolates linearly and rotation follows the shortest quaternion path. Tracks hold their endpoint poses outside the keyed interval. Match the start and end poses for a seamless loop. `animation_remove` deletes a named clip.
- A top-level `shared_curve` in `animation_set` expands scalar `keys:[[seconds,value],...]` into regular bone tracks. For each bone in `channels`, optional `rotation` and `translation` vectors specify the local offsets at value 1. Negative values and overshoot are allowed. Explicit `clip.tracks` can coexist on other bones. The generated tracks are saved as ordinary editable keyframes.

```json
{"command":"animation_set","name":"Close","clip":{"duration":1},"shared_curve":{"keys":[[0,0],[0.7,1.1],[1,1]],"channels":{"left":{"rotation":[0,0,20]},"right":{"rotation":[0,0,-20]},"slider":{"translation":[0.2,0,0]}}}}
```
- `query_rig` lists bones, weighted vertex count and clips. Add `response:"full"` to read all authored data. Changes are undoable and saved with the workspace.
- `query_animation` pages through clip summaries with duration in seconds, looping, and track count. Add `clip:"Idle"` to list tracks, key counts and first/last key times. Add `bone:"Chest"` to page through that track's exact keys. Use `limit` (1–256), `offset`, and the returned `next_offset`. Pass the returned `revision` on successive pages or edits to reject stale reads and writes.
- Use `animation_update` with the clip `name` and `duration` or `looping` to edit metadata. Duration changes preserve key times and must contain all existing keys. Use `animation_track_set` with `name`, `bone`, and `keys` to add or replace a single track. Use `animation_track_remove` with `name` and `bone` to remove a track. Each clip needs at least one track.
- Set track interpolation using `animation_track_set` with `interpolation:"step"`, `"linear"`, or `"cubic"`. Omit `keys` to change only the interpolation. Linear is the default. Cubic applies ease-in and ease-out to each segment, including quaternion rotation, with no authored tangent handles.
- Use `animation_sample` with `clip` and `time` for world-space posed bounds and bone transforms. Pass `bones` to filter by name. Use `kind:"vertices"` and optional `vertices` IDs to read skinned coordinates. Both queries support paging and return a revision. Rotations are XYZW quaternions.
- Use `animation_keys_edit` with a clip `name` and an array of edits. `set` takes `bone` and `key` and replaces a key at the same time. `remove` takes `bone` and `time`. `retime` takes `bones`, inclusive `start` and `end`, and optional `scale` and `shift`. Scale is about `start`, then shift is applied in seconds. Invalid times, duplicate times, and empty tracks reject the entire edit.
- Use `animation_ik_bake` with a clip `name`, a three-bone `chain` in parent order, world-space `target` and `pole` points, and `start`, `end`, and optional `fps`. The pole sets the bend direction. The bake reports unreachable samples and maximum reach error. It replaces keys inside the interval, sampling neighboring poses one frame outside when there is room. Optional `tip_rotation` is a world-space XYZ rotation in degrees. Omit it to preserve the tip's sampled world rotation. The contact is baked into the tracks, so check between-frame drift when precision matters.
- `export` with `format:"glb"` writes static meshes with their pivots, or rigged meshes with skin weights, inverse bind matrices and clips with root motion. Cubic easing is baked at up to 60 samples per second per interval, and step tracks remain step tracks. Looping metadata is stored in clip extras. Faces with complete UVs and an alpha cutoff embed their base-color PNG in a double-sided unlit mask material. Opaque materials with complete UVs embed base color, tangent-space normal and packed metallic/roughness maps. Height and specular are available separately through `texture_export`. OBJ and FBX export rest geometry. Animated GLB export follows the same workflow export gate.
- For still images, pass `clip` and `time` in seconds to `capture`. All camera, visibility and render mode options apply. Looping clips wrap the time and one-shot clips clamp it. The reply reports the requested and sampled times under `animation`. Pose captures render a temporary mesh and return artifacts without creating a modeling selection view.

```powershell
'{"command":"capture","clip":"Squat","time":1.8,"yaw":40,"pitch":5,"center":[0,0.75,0],"zoom":320}' | atk-mesh-cli --port 7879 --output squat.png
'{"command":"animation_sample","clip":"Squat","time":1.8,"bones":["Foot.L","Foot.R"]}' | atk-mesh-cli --port 7879
'{"command":"export","format":"glb"}' | atk-mesh-cli --port 7879 --output character.glb
```

## Project workspaces

The server discovers mesh directories recursively from the project root. A workspace contains a native `project.json` and normally has `workflow.json`, `resources/`, and `sessions/` alongside it. Discovery skips linked directories, `.git`, `.svn`, `node_modules`, `target`, `Library`, and the resources and sessions inside known workspaces.

Each browser tab has its own mesh selection. The viewer polls the selected mesh's latest saved revision. A legacy or standalone `project.json` may have no revision label until an editor opens it. The viewer reads workspace files and never starts an editing session.

Each editor saves changes automatically and owns its own undo history. Stop an editor with Ctrl+C when you finish editing. The project viewer continues to display the latest saved geometry and revision.

Successful mesh edits include a compact top-level `turn_handoff` with the resulting revision and a workflow status request. Use the latest receipt when finishing the user's turn. Summarize what changed, what current evidence verifies, and any remaining uncertainty or next step. Check workflow status when checkpoint or export readiness matters. Selection changes, previews, and read-only commands omit the receipt.

If an edit makes a satisfied modeling assessment stale and its evidence used a saved review with an unchanged preset, the receipt can include one `invalidated_review`. Its `command` reruns that preset at the resulting revision. Use it when the affected assessment matters to the task. Inspect the result before updating the assessment. This also works for edits committed through a transaction.

If a committed edit changes a surface or pivot in an object-to-object clearance check from a saved review preset, the receipt can include one `affected_clearance`. Its `command` measures the pair at the resulting revision. Compare the returned distance with the cue's `minimum` when present. The measurement covers the current pose. Repeat at relevant open poses to check a moving part.

When workspace persistence fails, the CLI prints the command reply and exits with status 1. A batch stops at that reply. `ok` reports whether the command succeeded, while `recording_error` reports the write failure. `persistence.revision` identifies the editor's current revision, and `persistence.revision_changed_in_memory` tells you whether the command advanced it. `persistence.project_saved` is true when that revision reached `project.json`, false when a required project write did not complete, and null when the command required no project write. Check these fields before resending an edit.

## Command discovery

Send `{"command":"help"}` for a short task index or `{"command":"help","all":true}` for the complete topic list. Open one topic at a time with `{"command":"help","topic":"workflow"}`. Use `modeling-requirements` for checkpoint commands and `command-api` for the core command reference.

The server embeds this reference, the modeling workflow, and the project guide. Topic responses contain `text`, `total` line count, `offset`, and `next_offset`. Pages default to 32 lines. Set `limit` to 1–256 and request the next page using the returned `next_offset` until it is null. Help is read-only and appears in the command recording. Invalid topic names or pagination values return a structured error.

For a short inspect/edit/capture/save walkthrough, read `{"command":"help","topic":"minimal-example"}`.

## CLI feedback

Run `feedback_submit` and `feedback_list` as single JSON requests to `atk-mesh-cli`. They work without a running editor and print JSON to stdout.

```powershell
'{"command":"feedback_submit","kind":"workflow","summary":"Document door swing reviews","goal":"Check clearance through the full swing","observed":"The guide covers the closed pose only","suggestion":"Add a worked example with intermediate poses","related":"help:workflow"}' | atk-mesh-cli
'{"command":"feedback_list","limit":32}' | atk-mesh-cli
```

Use `missing_capability`, `command_issue`, `documentation`, or `workflow` as the kind. Include the goal, what you tried, what happened, and any workaround in the report. The required text fields are `summary`, `goal`, and `observed`. The optional `suggestion` and `related` fields accept text, including a command name, help topic, or replay trace reference. Omit credentials and sensitive project data. The CLI assigns an ID, timestamp, and version and returns the ID and inbox directory.

`feedback_list` returns reports newest first. Filter by `kind` if needed. Pass the returned `next_before` as `before` with the same filter to read the next page. A null cursor marks the last page. The default limit is 32 and the maximum is 256.

The inbox is shared across workspaces for the current OS user. It defaults to `%LOCALAPPDATA%\atk-mesh\feedback` on Windows, `~/Library/Application Support/atk-mesh/feedback` on macOS, and `$XDG_DATA_HOME/atk-mesh/feedback` on Linux, with `~/.local/share/atk-mesh/feedback` as the Linux fallback. Existing legacy inboxes are reused when the new inbox does not exist. Set `MESH_FEEDBACK_DIR` to an absolute directory path to override the default. The list response includes the resolved path even when empty.

## Full-volume reconstruction

`reconstruct_volume` uses a local TripoSR model to generate an editable mesh from an image. Set `ATK_MESH_TRIPOSR` to a checkout of [TripoSR](https://github.com/VAST-AI-Research/TripoSR), and optionally set `ATK_MESH_PYTHON` to the Python executable for that environment. The environment needs PyTorch, Pillow, OmegaConf, einops, trimesh, imageio, rembg, transformers, huggingface-hub and scikit-image. The adapter uses scikit-image for mesh extraction. Model weights are loaded from the Hugging Face cache or downloaded on first use.

```json
{"command":"reconstruct_volume","image":"ref.png","mask":"mask.png","resolution":128,"aspect":"front_three_quarter","source":"original"}
```

Use a PNG with foreground alpha or supply a same-size mask PNG with white foreground. Use `--output` with the CLI, or `output_path` in stdio MCP. Output is editable project JSON or geometry-only OBJ. The model estimates orientation and shape in normalized coordinates. The response reports boundary and nonmanifold edge counts and suggests additional reference views. A closed edge count alone does not establish accurate hidden geometry or absence of self-intersections.

For an offline checkpoint, set `ATK_MESH_TRIPOSR_WEIGHTS` to the directory containing the trusted TripoSR `config.yaml` and `model.ckpt`. Keep TripoSR's DINO encoder configuration in the Hugging Face cache used by Python.

## Depth reconstruction

`depth_mesh` back-projects a depth PNG into an open editable mesh. `fit_depth` takes a saved project and moves explicitly selected visible vertices toward samples from the same kind of depth view. Optional landmark bindings supply a target pixel per selected vertex. Both commands run locally without an editor and write native project JSON or OBJ. For CLI requests, pass the output with `--output`. Stdio MCP takes `output_path`.

Relative paths resolve from the CLI process's working directory. Use absolute paths for images and saved projects when running from another directory. See [the editor handoff walkthrough](reconstruction.md#open-a-depth-reconstruction-in-the-editor) for a complete example.

```json
{"command":"depth_mesh","view":{"camera":{"width":384,"height":384,"yaw":0,"pitch":0,"zoom":125},"mask":"mask.png","depth":"depth.png","distance_range":[0,0.85],"maximum_jump":0.035,"aspect":"front_three_quarter","source":"original"}}
```

Use pixel-aligned depth and mask PNGs with camera dimensions matching the maps. The pixel budget is 262,144. White mask pixels keep geometry and black pixels exclude it, including internal holes. Depth uses 8-bit or 16-bit grayscale and maps black to the near end of `distance_range` and white to the far end in scene units along the camera axis. Orthographic distances are relative to the camera center. Set `maximum_jump` to reject triangles spanning depth discontinuities. It defaults to 5% of the supplied range.

For `fit_depth`, also supply `project` as a saved JSON path and `vertices` as vertex IDs. Optional `landmarks` entries use `{"vertex":12,"pixel":[180,96]}`. `strength` defaults to 0.5 and `maximum_move` defaults to the view's `maximum_jump`. Use a new output path to keep the original project available for comparison. The reported mean distances measure movement toward the sampled targets for that pass. Calibrate the camera and fit range before comparing results.

`depth_mesh`, `fit_depth`, and `visual_hull` include a `reference_coverage` advisory. Label views with `aspect` and `source` when known. The response suggests missing side and rear references. Generated views are recorded as such and can be used as companion references.

## Batch commands

Pass a JSON array of 1–1024 command objects through stdin, `--command JSON`, or a UTF-8 file. The CLI detects arrays automatically. Use `--batch` to require array input. Update both the CLI and server to use single-request batching.

```powershell
'[{"command":"workflow_status"},{"command":"query_mesh","kind":"objects"}]' | atk-mesh-cli --port 7879 --batch
atk-mesh-cli --port 7879 --batch --input recipe.json
```

The CLI sends the whole array in one request. The server checks that every entry is an object with a nonempty `command` string before executing anything. It then runs the commands in order without interleaving requests from other clients. Later commands see earlier changes. Commands that need a returned ID or revision belong in a follow-up request.

The CLI prints one JSON line per attempted command after the batch reply arrives. It stops at the first command failure, includes that reply, and exits with status 1. Earlier edits remain applied and keep their normal undo and recording behavior. Use `transaction` for supported edits that must commit together. `--output` is available for single commands. Batch capture and save replies include their normal artifact paths. Commands wait for the server reply by default. Pass `--timeout SECONDS` to set a socket read timeout. If the connection drops or times out before the reply arrives, use the reported `cli_request_id` to find matching inputs in the workspace session event records. A missing event can mean the command is still running. For batches, check which events were recorded before retrying.

## Reference preparation

Start with the intended use, scale, fixed dimensions, moving parts, and essential visual traits. Separate hard dimensions from approximate image proportions.

Look for references that answer a specific question. Use a concept or photo for the main silhouette, a known dimension or scale proxy for size, and a close-up or second view where construction or depth is unclear. Use an adjacent part or character proxy when fit matters.

Optional brief for the calling agent to fill in and use with its available search or image tools:

> Find or create a modeling reference for [asset] used for [purpose]. Show [missing view or feature] clearly so it resolves [modeling decision]. Preserve [fixed constraints and approved traits]. Use [view direction], a neutral background, and minimal occlusion. Identify inferred details and any conflicts with the approved reference.

Copy selected resources into the workspace. Register them with `workflow_resource` and describe what each view controls. Record the chosen reference IDs and remaining assumptions with `workflow_assess` for `reference`. The CLI supplies predefined guidance locally. The calling agent handles any search or image generation.

## Blockout review

Enter `primary` and block out the main masses and attachment points. Check `workflow_status` when they are recognizable and before adding fittings or dense topology. The response's `next_action` suggests an advisory step based on workspace state.

Use `compare_reference` for the main design view and a distinct capture that resolves depth, width, or fit. Save a review preset when repeating the comparison. Use trusted landmarks for framing and different landmarks or silhouette regions for evaluation. Compare broad shapes first. Record feature ratios or dimensions that matter to the brief and distinguish observed discrepancies from inferred depth.

Open the resulting images. Inspect silhouette, proportions, alignment and clearance as applicable. Revise major mismatches, then repeat the same views. Register fresh captures with `workflow_resource` using their trace session and event. Record the findings with `workflow_assess` for `primary_forms` and `views`. Re-run `workflow_status` after edits to see which assessments need fresh evidence.

Follow rough blockout with primary-form refinement before adding secondary detail. Resolve the interfaces that make the asset read as one designed object in neutral views. Use a connected recess, a shared mating profile, or a deliberate seam according to how the parts fit and move. Review at player height and from an oblique angle to judge whether the transition reads at the intended distance.

New `primary_forms` assessments require `"outcome":"accepted"` or `"outcome":"needs_revision"`. Use `needs_revision` when a primary form still misses the brief. Record the corrections needed in `assessment` and the reviewed views in `evidence`. This blocks detail, completion and export, and keeps the findings and prior evidence IDs visible after edits. Repeat the relevant views and register current evidence before accepting. Existing saved assessments retain their previous status rules.

## Variant review

Use a controlled variant when the right edit is uncertain. Name the question, the region or parts involved, and the measurements or constraints that must hold. Record what would count as acceptable at this phase.

1. Save a baseline project or make a disposable copy. Define fixed views and relevant dimensions, clearances, or protected volumes with `review_define`. Run `review_run` and inspect the saved images and report. Register the useful baseline captures with their trace and include them in the review assessment.
2. Test one change at a time when practical. Use a transaction preview to check edit validity and guards. To render the change, apply it in the disposable copy or use an undoable edit backed by the saved checkpoint.
3. Run the same preset on the variant and compare it with the baseline. A `needs_revision` assessment backed by saved reviews lists them in `workflow_status.result.next_action.baseline_reviews`. Each matching preset includes a `review_run` command pinned to its version. If the preset was updated, collect a new baseline and rerun the variant with that version.
4. Keep the variant that best fits the brief and register its current evidence. Record why you chose it. Use `outcome:"needs_revision"` for remaining blockers or `outcome:"accepted"` with optional `follow_up` text for work that can carry into the next phase. Stage and status replies include those notes.

## Junction review

Decide how the surfaces should meet before adding local detail. For a continuous surface, inspect the silhouette, shading and edge flow in a fixed crop. For a seam, inspect the gap and alignment. For a moving part, check its pivot and clearance in representative poses and record which poses were checked.

Use a review preset for shaded and wireframe views, and add local clearance or protected-volume checks if needed. In a disposable copy, try one expected future edit such as widening an opening or moving a seam. Check whether adjacent surfaces, shading, and clearances still hold. Record any construction issue that would make downstream edits expensive.

When accepting primary forms, provide optional `critical_regions:["spindle seat","cheek bevel"]` on `workflow_assess`. These are planning labels and may identify regions still to build. `workflow_status` exposes the prompts before and during detail, and `workflow_stage` returns them when entering detail.

Plan the connections before building each region. Keep quad strips through curves, bevels, and openings that must support later edits. Transition to coarser topology on flatter areas by redirecting or terminating loops. Flat static caps can use planar n-gons or well-placed triangles. Reuse the construction profile for symmetric parts, or use `select_related` with `kind:"mirrored"` to identify existing counterparts for paired edits. Compare the test edit in shaded, topology, and tessellation views. Record whether it preserved the intended silhouette, shading, symmetry, and clearance in the workflow evidence before accepting delivery.

## Tessellation review

Build deliberate edge loops where surfaces curve, join, or need to support an edit. A flat static panel can use a simple planar n-gon in the editable mesh. Review authored edges with `capture {mode:"topology",clean:true}` and the triangles used by the CPU renderer with `capture {mode:"tessellation"}`. Use the same view for both. A saved review can include the latter with `review_define` or `review_update` and `"tessellation":true`. Register that artifact as evidence with `"capture":"tessellation"`.

The tessellation capture reports the triangle count and a sample of nonplanar face IDs across the included objects, including occluded faces. Its planarity threshold is one millionth of the largest face-vertex distance from the first vertex. Inspect long diagonals near corners and the shading around flagged surfaces. Counts and a clean preview supplement defect checks.

For a flat region with one simple boundary and an undesirable triangle layout, preview `retriangulate_region` in a copy. Compare the render and confirm the boundary stayed fixed. Split or rebuild warped or edit-sensitive areas with intentional edge flow. For delivery, explicitly validate the triangulation in the target application, especially for formats that preserve polygons.

## Boundary preserving patch edits

Use `inspect_patch` with an `object` and a `region`, `face`, or `faces` to get the ordered perimeter IDs and positions. Read the full response when the perimeter is long. A patch must be one connected disk. Its boundary may already be shared with surrounding faces.

Use `replace_patch` with the same selection and exact `boundary` returned by inspection. Pick either of these fills:

- Supply `grid_corners:[id0,id1,id2,id3]` in boundary winding order. The four sides must have equal opposing edge counts. The tool interpolates the interior and rejects nonplanar or nonconvex cells. Split the region into simpler patches when needed.
- Supply `polygons` and optional `interior:[[x,y,z],...]`. Polygon corner indices are zero-based into the ordered boundary followed by the new interior points. Every point must be used, and the polygons must reproduce the same directed perimeter. Use planar convex faces, with triangles available where a patch changes depth.

Both fills preserve boundary positions and IDs, and replace only the selected faces and their interior vertices. Named regions that contain the entire patch are updated. Partial overlaps with other named regions, UV or texture assignments, mixed shading, and edits that would discard or leave gaps in vertex paint are rejected. Use a transaction with `guards.no_new_defects` to check surface intersections before committing.

In a transaction, `guards.protected_boundaries:["panel"]` locks a named perimeter and positions while allowing changes inside it. Use `watch_boundaries:["panel"]` to report changes without blocking. The reply counts added, removed, or rewritten triangles, quads and n-gons. Optional `preview` accepts capture settings and returns fixed-camera before/after PNGs. Preview with `commit:false`, inspect the artifacts, and pass the same revision on commit.

## Minimal example

Use a new, disposable workspace for this example. Start `atk-mesh --workspace .mesh/quickstart --port 17879` in one terminal and leave it running. A fresh workspace starts empty. Use the explicit `cube` command below to create object 1 for the walkthrough. In a second PowerShell terminal:

```powershell
'{"command":"help","topic":"workflow"}' | atk-mesh-cli --port 17879
'{"command":"workflow_status"}' | atk-mesh-cli --port 17879
'{"command":"query_mesh","kind":"objects"}' | atk-mesh-cli --port 17879
'{"command":"workflow_assess","requirement":"intent","assessment":"Practice moving a cube and saving a preview and native geometry.","evidence":[]}' | atk-mesh-cli --port 17879
'{"command":"cube"}' | atk-mesh-cli --port 17879
'{"command":"select_object","object":1}' | atk-mesh-cli --port 17879
'{"command":"move","value":[0,0.25,0]}' | atk-mesh-cli --port 17879
'{"command":"capture","yaw":35,"pitch":25}' | atk-mesh-cli --port 17879 --output quickstart.png
'{"command":"save"}' | atk-mesh-cli --port 17879 --output quickstart.json
```

Check each exit status and JSON response before continuing. Open `quickstart.png` to inspect the result; view the saved mesh in the project viewer with `atk-mesh --project . --port 7878`. The server persists edits in `.mesh/quickstart/project.json`; `quickstart.json` is an additional native model save. Stop and restart the server with the same workspace to resume.

This example stays in blockout and leaves visual requirements pending. Before detailing a real asset, review its references and captures and assess the relevant modeling requirements. When the user is ready to integrate the asset, complete export validation, export the requested format, and verify it with an independent import.

## Run from source

```powershell
cargo run --bin atk-mesh -- --workspace workspaces/example --port 7879
```

This example starts the server in single-workspace mode and creates the directory if needed. Opening a workspace restores its `project.json` and keeps existing trace records under `sessions/`. Document changes automatically update `project.json`. Copy the entire workspace directory to preserve both geometry and replay history.

Start the project viewer in a separate terminal with `cargo run --bin atk-mesh -- --project . --port 7878`. Open http://127.0.0.1:7878 and select the workspace with the folder picker. Drag to orbit, right-drag (or Shift-drag) to pan, and scroll to zoom. Touch supports one-finger orbit and two-finger pan and pinch. Switch between **Mesh** and **Animation** in the header. The render icons show shaded geometry, topology edges, lit vertex paint, or lit textures. Open **Debug** for diagnostic views, the UV checker, and the base-color preview. **Settings** contains the grid, auto rotation, and Objects. Send editing commands to the editor on port 7879.

The **Objects** inspector lists object names, IDs, vertex counts and color swatches matching **Object IDs** mode. Search by name or ID, toggle individual objects, hide or isolate all matches, use **Solo** for one object, and **Frame** to focus it. Visibility is browser-only state: it does not change selections, model data, CLI captures, exports or replay recordings. Hidden objects remain hidden across live edits while their IDs exist; removed IDs are dropped, new objects start visible, and reloading or changing workspaces resets visibility. **Show all** restores the full model.

The viewer polls the shared document every 400 ms while visible, updates changed geometry, and preserves the camera across edits. It retries on disconnect and keeps the last model visible. It does not request SVG captures or mutate mesh selections. WebGL 2 is required; no external libraries, CDN or frontend build step are used. Use simple planar polygons for predictable rendering.

In a separate terminal, send commands to the editor:

```powershell
'{"command":"inspect"}' | cargo run --quiet --bin atk-mesh-cli -- --port 7879
'{"command":"capture","yaw":35,"pitch":25}' | cargo run --quiet --bin atk-mesh-cli -- --port 7879
```

## Recording and replay

Add `--debug` to `atk-mesh-cli` to save a JSONL timeline in the workspace's `recordings/` directory. The CLI prints its path to stderr. Use `atk-mesh-cli --replay FILE` to read the timeline or add `--frame N` to select a snapshot. Frame 0 is the initial state. Replay leaves the workspace untouched. A batch submission without confirmed replies may still have reached the editor. Check the workspace before retrying.

Start a workspace editor with `--debug` in either a debug or release build to record all API/CLI commands, including failures and undo/redo, under `<model-workspace>/sessions/<session-id>/`. The editor writes gzip events and snapshots, starting with the initial document and adding snapshots on revision changes. Without the flag, the editor saves the workspace normally and records successful capture, comparison and review commands with the snapshots needed for workflow evidence. Viewer polling uses `GET /state` and is excluded from recordings. Previous sessions remain readable.

Replies are compact by default, including `inspect`, load, undo and redo: they omit full mesh, adjacency and edge lists. Revision, counts, validation and history remain available. Collections larger than eight entries return `{count, sample, truncated:true, path}` instead of every ID or topology mapping. A shared node budget and nesting limit further bound complex results; large strings return byte counts. `path` is a JSON pointer into the original result and can be passed to `query_record`. Small collections remain unchanged. Add `"response":"full"` to request the original complete result, including inline artifact payloads. This deliberately replaces the earlier default of full `inspect` results and unbounded edit selections.

Prefer targeted reads: `query_mesh` returns exact, paged records for `kind:"vertices"`, `"faces"`, `"selection"` or `"objects"`. Optional `ids:[...]` filters records; `object` scopes vertex/face/selection queries. Pages default to 32 entries, `limit` accepts 1–256, and `offset` plus `next_offset` supports traversal. Mesh records use numeric ID order. Object records contain names, origins, vertex counts and smoothing settings rather than full membership lists. Use `revision` to reject pages from a changed document. Existing region/profile/loop/shading queries remain available; use a small region or `response:"full"` when all of its local context is required.

`query_record` reads immutable recorded data. Start the editor with `--debug` to retrieve pages from ordinary command replies, or request `response:"full"` on the original command. To page a recorded result, provide `session`, `event`, `path`, and optional `offset`/`limit`. Array pages preserve order; object pages use serialized key order. It returns the page, source revision and current revision without re-executing the operation. Nested page values are still bounded and carry deeper retrieval paths; `response:"full"` returns exact page values. This allows detailed topology mappings to remain on the PC until the agent actually needs them. For example:

```json
{"command":"select","vertices":[1,2],"note":"Select the hinge attachment vertices"}
{"command":"query_mesh","kind":"objects"}
{"command":"query_mesh","kind":"vertices","object":2,"ids":[9,10,11]}
{"command":"query_mesh","kind":"selection","limit":32,"offset":0}
{"command":"query_record","session":"<trace.session>","event":1,"path":"/operation/rings","limit":16}
```

Responses include `trace: {session, event, record}` for correlation and direct event retrieval. Optional context such as `note` is retained in the recorded input. Request/response measurements are serialized JSON bytes, not token counts; Codex prompts and actual model usage are not exposed to this server. Execution timing covers command execution, excluding recording, artifact writing and network overhead. Full payloads (including renders and exports) are kept on disk, so sessions can grow substantially; there is no automatic retention limit in this prototype. Read-only commands reuse the session's cached viewer state when the authoritative document revision is unchanged.

Capture, export and save replies return `artifacts` with absolute local file paths and byte sizes, rather than embedded PNG/FBX/SVG/OBJ/project content. These files live under the recording's `artifacts/` directory. The CLI copies the requested artifact to its optional output filename and prints a small receipt; no transport wrapper is needed to suppress encoded data. The CLI and server run on the same PC. Replay continues to use the original full capture stored in the event. Artifact-write failures are reported as `artifact_error`; explicit full replies and original event data remain available.

Read-only recording endpoints are `GET /sessions`, `/sessions/<id>`, `/sessions/<id>/events/<number>`, and `/sessions/<id>/states/<snapshot>`. The session index maps commands to their snapshots. Original command outputs and compact replies are both retained in each event.

The CLI also accepts a JSON argument and an optional output filename. A capture saved with a `.png` extension writes the PNG directly; other capture filenames receive an SVG containing that same PNG. Export to `.fbx` writes native binary FBX; OBJ remains available. Other commands write result JSON. For shells that change argument quoting, use stdin or invoke the executable with a subprocess argument array.

```text
atk-mesh-cli '{"command":"capture","yaw":0,"pitch":0}' front.svg
atk-mesh-cli '{"command":"capture","mode":"face_ids","yaw":35,"pitch":25}' faces.png
atk-mesh-cli '{"command":"export"}' mesh.fbx
atk-mesh-cli '{"command":"export","format":"obj"}' mesh.obj
atk-mesh-cli '{"command":"save"}' mesh.json
```

## Modeling requirements

Every workspace, including existing models, starts with pending `intent`, `reference`, `primary_forms`, `views`, and `export_validation` requirements. Run `workflow_status`. No check is implicitly exempt: use `workflow_waive` with a specific `reason` when inapplicable. Object-scoped exceptions do not replace project defaults.

`workflow_stage` accepts `blockout`, `primary` (intent/reference), and `detail` and `complete` (also primary forms/views). A modeling or texturing session can finish with a reviewed native project and pending export validation. Direct `export` separately requires all five. Reserve export and importer checks for a user-requested export or integration session. Blocking returns `ok:false`, `error:"requirements_missing"`, required resources, reading prompts and command templates. Edits remain available for repairs.

`workflow_status` and the primary/detail stage replies include an advisory `part_review`. Record the plan before building, then add object or collection IDs as those parts exist. Each part requires a unique `name` and `silhouette_views` (an empty list is valid). Relationships require part names `a` and `b`, `intent` and `verification`. Extra notes, including a `shared_profile` or uncertainty, are preserved as authored context.

```json
{"command":"workflow_part_plan","plan":{"parts":[{"name":"Housing","silhouette_views":["side"],"objects":[2]},{"name":"Faceplate","silhouette_views":[],"collections":[1]}],"relationships":[{"a":"Housing","b":"Faceplate","intent":"Seat the faceplate inside the lip","shared_profile":"Front seat","verification":"Check the reveal in three sections"}]}}
{"command":"workflow_part_plan","object":2}
```

Omit `plan` to read. A write replaces the complete workspace plan. Filter reads by `object` or `collection` to retrieve matching parts and their direct relationships. Collection membership comes from the current mesh. Deleted bindings are reported as missing. The part plan records design intent in `workflow.json`; construction geometry and collection membership remain owned by the mesh.

Record silhouette targets under each part's optional `profiles` array. Each entry has a `view` from `silhouette_views`, an `outline`, a `basis` of `observed` or `inferred`, and nonempty `checkpoints`. A `reference` can point to a registered reference ID. Queries flag unrecorded targets in `missing_profile_views`, including on older plans. Describe the direction of taper, section transitions, and which edges should break or roll. Use measurements or landmarks as checkpoints when the reference supports them.

To build from the written plan, save at least one silhouette target and cover every declared silhouette view, then assess the reference checkpoint with `{"command":"workflow_assess","requirement":"reference","basis":"shape_brief","assessment":"Build from the recorded profiles and fit targets.","evidence":[]}`. The primary-form review returns the brief and suggests a shaded capture. Changes to the brief require reassessment. Object and collection binding changes preserve the assessment. Captures and distinct-view checks still apply.

| Command | Input / behavior |
| --- | --- |
| `workflow_status` | Requirements and paged resources: `offset`, `limit` 1..64, default 8. |
| `workflow_resource` | `kind:"reference"` or `"evidence"`, existing workspace-local `path`, `description`, optional `object`. References can save `view` camera/resolution options and `crop:[left,top,right,bottom]`. |
| `workflow_resource` | `kind:"capture"`, trace `session`, `event`, `description`, optional `object`. Derives path/camera from a successful capture/comparison and verifies its source snapshot matches current geometry. |
| `workflow_assess` | `requirement`, `assessment`, required `evidence:[resource_id,...]`, optional `object`. Reference needs reference evidence; primary forms a capture; views two distinct camera records; export validation report/evidence. Intent needs an assessment but no file: explicitly send `evidence:[]`. |
| `workflow_waive` / `workflow_reopen` | `requirement`, `reason`, optional `object`; explicitly exempt or reset a check. |
| `workflow_stage` | `stage`; enforces the corresponding requirements. |
| `compare_reference` | Registered `reference` ID and optional saved-view overrides. Produces PNG: cropped reference left, current render right. Manual camera alignment; no automatic accuracy score. Supports capture camera, mode and resolution. |
| `taper` | Selection, `axis`, `center`, `profile:[[axial_position,first_scale,second_scale],...]`, optional object guard. Positive interpolated scales on perpendicular axes `(axis+1)%3`, `(axis+2)%3`; outside the interval unchanged. Triangulate quads first if deformation would warp them. |

The serialized Session owns workflow state, atomically persisted in `workflow.json`. All commands, assessments and waivers appear in normal Replay input/output. Keep resources and sessions with the workspace. File hashes detect modified/missing evidence; geometry-dependent checks become pending after relevant geometry, origin or shading edits. Selection changes do not invalidate checks. Scoped checks survive unrelated-object edits; hashes survive restarts. Intent/reference checks survive geometry edits. Waivers remain explicit until reopened/reassessed.

Changing intent or reference assessments also invalidates dependent form/view/export checks. Distinct views require different camera directions; changing zoom or adding 360 degrees does not satisfy a second view.

Assessments are agent claims supported by resources, not proof of correct image interpretation. Capture provenance is verified; manually registered reports remain declared evidence. Export validation is a preflight check; independent import verification follows export. Direct UV editing, image texture assignments and vertex painting are supported; full material/shader graphs are not authored by the tool.

An intent assessment without evidence files still includes the required array:

```json
{"command":"workflow_assess","requirement":"intent","assessment":"Create a mesh-only van candidate with separate doors and a rear-clearance study; rigging and engine integration are separate work.","evidence":[]}
```

### Reference alignment

The alignment tools work with any registered PNG reference. Targets are manually identified points in the **original image's pixel coordinates**, including when displaying a crop. They are modeling aids, not automatic image understanding or proof of likeness.

| Command | Behavior |
| --- | --- |
| `reference_landmarks` | `reference`, `landmarks:[{name,pixel:[x,y],vertices:[ids],object?},...]`, optional `contours:[{landmarks:[names],closed:true}]`. A binding with only `object` uses its vertex centroid. A point with neither binding is a reference-only contour point. At most 128 landmarks, 256 explicit vertex IDs each, 32 contours. Replaces the reference's landmark set atomically. |
| `reference_landmarks` (read) | Omit `landmarks` to page saved bindings; `kind:"landmarks"` or `"contours"`, `offset`, `limit` 1..64 (default 8). Full replies or `query_record` retrieve truncated binding arrays. |
| `reference_view` | Save camera/crop/resolution options on `reference`. Uses the capture camera conventions. |
| `fit_reference` | `reference`, optional `names:[calibration_landmarks]`, optional camera overrides. Orthographic: fits uniform scale/framing with at least two separated anchors. Perspective: local least-squares fit of eye position, yaw, pitch and vertical FOV with at least six bound anchors. Optional `fit_parameters` selects from `position_x`, `position_y`, `position_z`, `yaw`, `pitch`, `fov`; `bounds:{parameter:[min,max]}` constrains plausible values. Mesh geometry stays unchanged; roll and lens shift remain fixed. Use trusted anchors and inspect independent regions. |
| `project_reference` | Read saved view: projected points, target-to-mesh deltas, RMS/max errors in original-image pixels. Optional `names`, `offset`, `limit` 1..64 (default 16). Returns mesh `revision` and `alignment_version`; send both on subsequent pages to reject stale data. Group anchors use vertex centroids and are not visibility-tested. |
| `compare_reference` | Add `layout:"overlay"` for the cropped source with cyan mesh silhouette/projected contours, magenta reference contours/markers, and yellow residual lines. Default remains side-by-side. Camera/crop overrides are temporary; measurements for editing should come from `project_reference` using the saved view. |
| `move_in_reference` | `reference`, required `revision` and `alignment_version` from `project_reference`. `landmark:"name"` moves its bound vertices rigidly to the target; optional `delta_pixels` requests a partial/custom move. Alternatively supply `delta_pixels` with current selection or explicit `vertices`. An optional `object` guards ownership. Deltas use source-image pixels. |
| `move_in_view` | Current selection or explicit `vertices`, `delta_pixels:[dx,dy]`, `view_id` or explicit `camera:{yaw,pitch,zoom,center,width,height}`. Deltas use actual capture pixels. Capture after selecting: view IDs become stale after any document edit. |

Both move commands preserve camera-space depth, object origins and connectivity, and use normal document rollback/undo/redo and object guards. A pixel move can still produce poor topology or an implausible hidden shape; validate another view and the affected region. Single-image matching does not determine depth. Moving a point on a sloped planar face can disturb that face's planarity; choose suitable vertex groups and inspect shading.

Saved landmarks, contours, camera/crop and alignment versions live on the existing reference resource in `workflow.json`; geometry stays in `project.json`. Updates invalidate dependent form/view/export assessments. Missing/rebound geometry or modified reference files fail explicitly. `workflow_status` shows counts rather than dumping landmark arrays; original command inputs, outputs and overlay PNGs remain in Replay.

```json
{"command":"reference_landmarks","reference":"resource-1","landmarks":[{"name":"left","pixel":[120,200],"vertices":[11]},{"name":"right","pixel":[420,200],"vertices":[12]},{"name":"fastener","pixel":[270,260],"object":7}]}
{"command":"fit_reference","reference":"resource-1","names":["left","right"],"yaw":0,"pitch":0}
{"command":"project_reference","reference":"resource-1","names":["fastener"]}
{"command":"compare_reference","reference":"resource-1","layout":"overlay"}
```

Use the actual IDs returned by your model, then the `move_template` and current version numbers returned by projection. The example coordinates and IDs above are illustrative.

## Command API

Send JSON to `POST /command` on the editor's loopback port. Responses are `{ "ok": true, "result": ... }` or `{ "ok": false, "error": ... }`. The CLI exits nonzero on errors. Optional `revision` guards any command against concurrent document changes. Each successful mutation, including selection and history, advances the revision. Failed edits are atomic.

On the read-only project viewer's port, `GET /workspaces` lists project-relative paths in `result.workspaces`. Append a URL-encoded `?workspace=.mesh%2Fmodel` to `GET /state/meta`, `GET /state`, or a `GET /sessions` route. The viewer polls `GET /state/meta` for `result.tag`, which is derived from saved file metadata without reading the mesh. The full state reply has `result.mesh` and `result.revision` from the same saved file, with an ETag for that version. Editor routes use the editor's port directly.

To batch commands, send a top-level JSON array to the same endpoint. HTTP 200 returns `{"ok":true,"replies":[...]}` with the normal command replies in order. HTTP 400 for a command failure returns `{"ok":false,"failed_command":2,"replies":[...]}`, where `failed_command` is the one-based position and `replies` ends with the failed reply. An invalid array length or malformed entry returns HTTP 400 with an `error` and an empty `replies` array. A malformed entry also includes its one-based `failed_command` position. See [batch commands](#batch-commands) for execution behavior.

Default replies are compact. Capture/export/save payloads are written to files and returned as `result.artifacts`, keyed by `png`, `svg`, `fbx`, `obj` or `project` as applicable. Each artifact has an absolute local `path` and `bytes` size. A capture reply has this shape (paths and numbers are illustrative):

```json
{"ok":true,"result":{"artifacts":{"png":{"path":"C:/project/.mesh/model/sessions/session-id/artifacts/7.png","bytes":12345},"svg":{"path":"C:/project/.mesh/model/sessions/session-id/artifacts/7.svg","bytes":17000}}},"trace":{"session":"session-id","event":7,"record":"/sessions/session-id/events/7"}}
```

`atk-mesh-cli --output preview.png` copies `result.artifacts.png.path` to the requested local destination and adds a top-level `written:{path,bytes}` receipt. A capture saved to another extension uses SVG; export to `.fbx` selects FBX automatically, while `format:"obj"` selects OBJ. `save` uses the `project` artifact. Create destination directories first. Client output paths resolve from the client's working directory; artifacts remain in the server's session directory. Both programs must run on the same machine. An `artifact_error` means artifact writing failed; an output-copy failure may occur after a successful recorded command, so do not blindly retry an edit.

For inline payloads, send `response:"full"`: captures return `result.png_base64` and `result.svg`; FBX export returns `result.fbx_base64`; OBJ export returns `result.obj`; save returns the native project in `result`. Full replies skip artifact-file projection, and the CLI can also consume their inline payloads with `--output`. Default compact replies avoid transferring these large strings.

Large region/operation arrays and objects can become `{count,sample,truncated:true,path}` summaries. For exact current records, use `query_mesh` with `kind:"vertices"`, `"faces"`, `"objects"` or `"selection"`, optional `object`/`ids`, `offset` and `limit` (default 32, range 1–256). Continue with `next_offset` until null; pass the first page's `revision` on subsequent pages to reject changes during traversal. For an exact local region in one reply, add `response:"full"` to `inspect_region`.

To recover an exact array or object from a previous compact reply, send `query_record` with that reply's `trace.session`, `trace.event`, and the returned `path` verbatim. Its `offset`/`limit` page immutable recorded data without rerunning the command. Add `response:"full"` to preserve nested page values; otherwise those values may themselves be summarized. Paths are relative to the original result, not the response envelope. For example (replace IDs and the pointer with the actual returned values):

```json
{"command":"query_mesh","kind":"vertices","object":1,"offset":0,"limit":32}
{"command":"inspect_region","object":1,"response":"full"}
{"command":"query_record","session":"session-id","event":7,"path":"/operation/rings","offset":0,"limit":32,"response":"full"}
```

| Command | Fields | Result / behavior |
| --- | --- | --- |
| `help` | optional `topic`, `offset`, `limit` | Embedded topic index, workflow guidance and paged documentation |
| `inspect` | optional `response:"full"` | Default: counts, selection summaries, validation and history; full: mesh IDs, positions, faces, adjacency and edges |
| `query_mesh` | `kind`, optional `object`, `ids`, `revision`, `offset`, `limit` | Exact paged current vertices, faces, objects or selection; see pagination above |
| `query_record` | `session`, `event`, `path`, optional `offset`, `limit`, `response:"full"` | Retrieve exact pages from an immutable recorded result; see retrieval above |
| `uv_set`, `uv_clear`, `uv_transform`, `query_uv`, `inspect_uv`, `capture_uv` | Explicit face/vertex corner values and scope | Direct UV editing and validation; read help topic `uv-editing-and-vertex-painting` |
| `texture_import`, `texture_define`, `texture_assign`, `texture_remove`, `texture_export` | Named image resources and face assignments | Direct texture mapping and PNG export; see the same appearance help topic |
| `vertex_color_set`, `vertex_color_clear`, `query_vertex_colors` | Vertex IDs and normalized RGB | Direct vertex painting, exact queries and `capture` with `mode:"vertex_colors"` |
| `uv_project`, `vertex_color_gradient`, `vertex_color_shade` | Explicit scopes, mapping bounds and paint parameters | Optional coordinate/normal operations; read help topic `native-appearance-recipes`; use CLI `--batch` for JSON command recipes |
| `inspect_region` | `faces: [id,...]` and/or `vertices: [id,...]`, optional `object`, `response:"full"` | Read-only local positions, face loops, neighbors, adjacency, bounds, boundary edges and disconnected-component count. Default collections may be summarized; full returns exact region data. Vertex IDs expand to incident faces |
| `object_create_box` | `name`, `min`, `max`, optional `origin` | Add an independent object without replacing existing geometry; returns object ID |
| `object_rename` | `object`, `name` | Set a unique object name |
| `object_set_stage` | `object`, `stage`, optional `note` | Record progress. Production ready requires a completed object review and a note |
| `object_review_plan` | `object`, `checks` | Replace the object’s checklist and reopen its review |
| `object_review_check` | `object`, `check`, `outcome`, `reason`, optional `evidence` | Record a review result against the object’s checklist |
| `query_object_review` | `object` | Read the checklist, recorded outcomes and stage |
| `object_set_origin` | `object`, `origin: [x,y,z]` | Set pivot without moving vertices |
| `object_move` | `object`, `value: [x,y,z]` or `align` | Translate the object's vertices and origin together |
| `object_rotate` | `object`, `value: [x,y,z]`, optional `pivot:[x,y,z]` or `pivot:"bounds_center"` | Rotate all object vertices in incremental degrees, X then Y then Z. Defaults to its origin. A temporary pivot leaves the authored origin in place. |
| `select_object` | `object` | Select all vertices owned by this object |
| `object_instance` | Master `object`, unique `name`, optional translation `value:[x,y,z]` | Create a linked copy; master shape, topology and shading edits propagate |
| `object_make_unique` | `object` | Detach a copy, preserving its placement, pivot, geometry IDs and collections |
| `query_instance` | Instance `object`, `kind:"vertices"` or `"faces"`, optional `ids`, `offset`, `limit` | Paginated master-ID to instance-ID mapping; `ids` filters by instance IDs |
| `object_duplicate` | Source `object`, unique destination `name`, optional translation `value:[x,y,z]`. Independent copy preserving face winding, smoothing and flat-face overrides; translates the source pivot with the geometry. Returns new object and vertex/face remaps. Copies do not share later edits. |
| `capture` | `yaw`, `pitch` in degrees; `zoom` at reference resolution; optional `width`, `height`, `mode`, `clean`, `depth_range`, `response:"full"` | Default: `artifacts.png` and `artifacts.svg`, view ID, revision and render metadata; full: `png_base64` and SVG wrapper. Defaults to 800×600 |
| `select` | `vertices: [1,2]` | Replace vertex selection |
| `select_box` | `min: [x,y,z]`, `max: [x,y,z]`, optional `mode` | Inclusive spatial selection. Use `replace` (default), `add`, `subtract`, or `intersect` to combine the box matches with the current selection. |
| `select_image` | `view_id`, `rect: [left,top,right,bottom]`, optional `through` | Select vertices using capture pixel coordinates; defaults to visible only |
| `move` | `value: [x,y,z]` | Translate selected vertices |
| `move_vertices` | `object`, `offsets:[{vertex,delta:[x,y,z]},...]` | Apply distinct vertex offsets in one edit. See `local-sculpting` |
| `sculpt_stroke` | `object`, `mode`, `radius`, `strength`, `samples:[{face,barycentric},...]`, optional `max_displacement`, `direction:[x,y,z]`, `continuous:true` | Raise or lower a connected neighborhood on a triangulated surface. See `local-sculpting` |
| `scale` | `value: [x,y,z]` | Scale about selection center |
| `rotate` | `value: [x,y,z]` | Rotate about selection center, degrees, X then Y then Z |
| `extrude` | `face` or `faces:[ids]`, plus `value: [x,y,z]` or signed nonzero `distance` | Displace the cap and select its new vertices. Distance follows the unit normal implied by the face winding. A face list extrudes an edge-connected planar patch on one object, adding side walls only on its perimeter. |
| `weld` | optional `vertices`, `target`, `position: [x,y,z]` | Merge vertices, repair face loops and remove collapsed faces; select survivor |
| `split_edge` | `edge: [a,b]`, optional `fraction` (default 0.5) | Insert a vertex into every face sharing the edge; select new vertex |
| `split_vertices` | `faces: [id,...]`, optional `vertices` | Duplicate shared vertices for these faces, separating them from other faces |
| `split_face` | `face`, `vertices: [a,b]` | Split a convex planar polygon with a diagonal between non-adjacent vertices |
| `inset` | `face`, `distance` | Offset a convex planar face's edges inward by a distance, creating a border ring |
| `bevel_vertex` | `vertex`, optional `fraction` (default 0.1) | Truncate a closed convex vertex fan with a plane, replacing the point with a cap |
| `bridge_faces` | `faces: [a,b]`, optional `offset` | Remove two end caps and join their equal-size boundary loops with quads |
| `undo`, `redo` | — | Restore document snapshot |
| `cube` | — | Reset mesh, undoable |
| `save` | optional `response:"full"` | Default: `artifacts.project` path and size; full: versioned native project |
| `load` | `project: {version:6,mesh:...}` | Validate and replace document, undoable; versions 1–5 also load |
| `import_glb` | `path` inside the active model workspace | Append static GLB 2.0 triangle geometry as editable objects, undoable; preserves existing geometry and IDs |
| `import_fbx` | `path` inside the active model workspace | Append static binary or ASCII FBX polygon geometry as editable objects, undoable; preserves existing geometry and IDs |
| `import_obj` | `path` inside the active model workspace | Append Wavefront OBJ polygon geometry as editable objects, undoable; preserves existing geometry and IDs |
| `export` | optional `format: "fbx"`, `"glb"`, `"stl"` or `"obj"`, `response:"full"` | Compact replies provide a file path and size under `artifacts`. Full replies return `fbx_base64`, `glb_base64`, `stl_base64` or `obj`. The API defaults to OBJ. CLI output extensions `.fbx`, `.glb` and `.stl` select their corresponding formats. Workflow checks gate export |

Cube face 4 is the top. IDs remain stable through transforms; extrusion and inset preserve the cap face ID. New topology commands return an `operation` object alongside the normal inspection result, including created IDs, removed IDs or vertex remapping as appropriate. Deleting vertices or faces does not recycle their IDs in subsequent edits. Allocation high-water marks are included in native projects; older version-1 projects without them still load. Undo/redo restores the full mesh state, including selections and ID allocation. Loading or resetting a document replaces its ID namespace. Views expire after 32 captures and become stale after any document mutation. History retains 100 edits. The browser checks for CLI edits every 400 ms.

### Importing GLB geometry

Place the source GLB inside the model workspace and send `{"command":"import_glb","path":"turret-top.glb"}`. Paths resolve relative to the active workspace and must remain inside it. Import appends to the current scene, including an empty workspace. Each mesh node becomes an independent object with its world transform baked into vertices and its transformed node origin as pivot. Mirrored transforms preserve winding. Indexed and non-indexed TRIANGLES with float positions are supported, including interleaved buffers. Source vertices are preserved without welding or simplification.

For OBJ, send `{"command":"import_obj","path":"scan.obj"}`. Coordinates retain their original scale. Import keeps polygon geometry and skips degenerate faces. UVs, materials and source normals are omitted. OBJ input is limited to 512 MiB, two million vertices, two million faces and eight million face corners.

For FBX, send `{"command":"import_fbx","path":"cargo.fbx"}`. The same workspace path and undo rules apply. FBX import keeps polygon boundaries and transforms each mesh node into an independent object with its node origin as pivot. Coordinates convert to right-handed Y-up meters. Import takes base geometry and omits UVs, source normals, colors, materials, textures, rigs and animation. FBX sources are limited to 512 MiB. Polygons with repeated vertex references or degenerate area are skipped and counted in the import result. FBX export remains available through `export` with `format:"fbx"`.

Import is geometry only: UVs, textures, materials, vertex colors and source normals are not retained; the tool recomputes shading. Keep the original GLB alongside the project. Animation, skins, morph targets, required extensions, sparse/normalized geometry accessors, external buffers and non-triangle primitives are rejected. Imports are limited to 64 MiB and one million vertices/faces. Container and transform conventions follow the [Khronos glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html).

Set `$env:MESH_PORT = "7879"` before starting a server and invoking the CLI to work alongside another workspace. Without a configured port, each server gets an available port from the OS. Pass the printed port to the CLI with `--port PORT`.

### Conservative cleanup and part separation

`{"command":"weld_coincident","object":2,"expected_removed_vertices":8546}` merges only vertices at exactly identical positions inside one object, retaining the smallest vertex ID. It never moves a position or removes a face. The required count guards the operation; invalid resulting topology rolls back the entire edit. Existing ownership, revision, undo and instance propagation rules apply. It does not merge nearby points.

`{"command":"object_separate_faces","object":2,"parts":[{"name":"Hatch","faces":[123,124]}]}` transfers named, disjoint face sets to new independent objects. Some faces must remain on the source. Face IDs and surface coordinates remain unchanged. Shared cut vertices are duplicated, including disconnected fans at a boundary; interior vertex IDs survive. Source shading and collection memberships are copied; pivots default to the source origin or each part's optional `origin`. Make linked objects unique before separation. Matching borders remain open, without artificial caps. Both commands use native atomic edits, save/replay and undo/redo.

### Local sculpting

Open this topic with `{"command":"help","topic":"sculpt"}` or `topic:"local-sculpting"`.

Use `inspect_region` with an object, face IDs, or vertex IDs to find the working patch. `topology.disconnected_components` counts groups connected by edges within that scope. Neighbor faces shown for context count only if included in the selected faces. A count greater than one can reveal separate pieces within a single object.

Use `move_vertices` for explicit corrections. Supply 1–16,384 distinct vertices owned by one object. Every delta is in world units. The command applies the offsets in one edit and reports the number of moved vertices and maximum displacement.

```json
{"command":"move_vertices","object":1,"offsets":[{"vertex":5,"delta":[0,0,0.02]},{"vertex":6,"delta":[0,0,0.01]}]}
```

Use `sculpt_stroke` on a triangulated object. Each of the 1–128 samples identifies a face and three barycentric weights in the face's recorded vertex order. To anchor at the first corner use `[1,0,0]`. For the face center use equal weights. Use `mode:"raise"` to move along the local normal or `mode:"lower"` to move against it.

```json
{"command":"sculpt_stroke","object":1,"mode":"raise","radius":0.4,"strength":0.04,"max_displacement":0.025,"samples":[{"face":12,"barycentric":[0.3,0.3,0.4]}]}
```

The radius follows mesh edges from each sampled triangle. Overlapping samples use the strongest influence at each vertex, then the brush blends that influence across neighboring vertices inside its reach to soften steps along the stroke. `max_displacement` caps peak strength before falloff and defaults to `strength`. For relief or repeated passes, supply a fixed world-space `direction`, such as `[0,0,1]` on a +Z-facing relief. The CLI normalizes the vector and reverses it for `lower`. Open boundary vertices stay fixed. If a stroke reaches no movable vertices, use a larger radius or a denser patch. If it would collapse or flip a triangle, reduce strength or the displacement limit.

For a clean relief curve, set `continuous:true` with a fixed `direction` and at least two anchors in stroke order. The brush measures distance to the segments between those anchors in the plane perpendicular to the direction. Keep consecutive anchors within one radius in that plane and on the same surface patch. A separate edge-distance falloff limits the edit to the neighborhood of the anchors. Use barycentric positions to follow the intended curve between mesh vertices. Each anchor can set `pressure` from 0 to 1 (default 1). Pressure blends linearly between anchors. Taper it to 0 at the ends to blend folds into the surrounding surface.

Replace the example IDs with those from your patch. Capture a shaded oblique view before editing, use a small stroke to shape the highlight, and repeat the same view afterward. For smoothing an existing triangle surface see `relax_surface` in `bounded-surface-cleanup`. These edits preserve connectivity. Both support the normal revision guard, undo, and transaction previews.

### Bounded surface cleanup

`{"command":"relax_surface","objects":[2,3],"max_displacement":0.003,"iterations":4,"factor":0.2,"feature_angle":35,"silhouette_views":[{"yaw":35,"pitch":25}]}` applies inverse-edge-length weighted relaxation to triangle objects. Choose `object` or `objects`. Boundary and crease vertices, object bounds extrema, principal-axis silhouette vertices and optional camera silhouettes are fixed. Every vertex stays within the requested displacement from this command's input, across all iterations. Backtracking keeps triangle normals within 15 degrees and areas at least 20 percent of their initial values. The operation retains connectivity, IDs and ownership.

`{"command":"improve_triangulation","object":2,"max_deviation":0.0005,"min_angle_gain":3,"feature_angle":10}` flips diagonals of convex near-planar triangle pairs when their minimum angle improves. It preserves vertices, all patch boundaries and face IDs. Each face participates at most once; the separation at the projected diagonal intersection bounds each patch's surface change. Principal and optional camera silhouette orientations are protected. Flat/smooth face boundaries are preserved. No faces are removed.

These limits are per command, not persistent constraints: successive passes can accumulate changes. Save a baseline and validate the combined deviation. Both operations use normal atomic edits, topology validation, revision guards, undo/redo, instance propagation, workspace saves and replay. Self intersections are not checked. `silhouette_views` accepts up to 16 standard capture camera objects; their viewing directions supplement the three principal axes.

### Topology semantics

`weld` uses the current vertex selection when `vertices` is omitted. It requires at least two distinct existing vertices. By default it preserves the smallest ID and places it at the centroid. Specifying `target` preserves that selected vertex's ID and position; `position` explicitly overrides the resulting position. Consecutive references that collapse together are simplified, and faces collapsing below three vertices are removed. Pinched polygons, duplicate faces, zero-length edges, inconsistent winding, edges shared by more than two faces, and disconnected face fans at a vertex are rejected. Weld can repair pre-existing zero-length edges if its result passes these checks. Self-intersections and all geometric inversions are not detected.

`split_edge` interprets `fraction` from the first endpoint toward the second and updates all incident face loops together, preserving their winding. It subdivides the edge without dividing its faces. `split_face` adds a diagonal; it preserves the original face ID for one half and returns the new face ID for the other.

`split_vertices` separates connectivity without moving geometry. A vertex must be used by both the chosen faces and remaining faces. Omit `vertices` to duplicate all such shared vertices. The chosen faces share the duplicates with each other; other faces retain the originals. This can intentionally create open boundaries. The returned `vertex_remap` maps original IDs to duplicates and the duplicates become selected.

`bevel_vertex` handles a point with three or more incident edges, such as the obelisk apex. It derives a cut-plane normal from the sum of normalized outgoing edge directions. The plane distance is `fraction` of the smallest positive edge projection onto that normal. This keeps the cut inside every incident edge and the cap planar even with unequal edge lengths. The incident faces must be convex and planar, all edges must cross that plane, and the resulting cap must be convex; open fans and unsupported concave/flat corners are rejected. The new cap vertices become selected. This is a single-segment vertex bevel, not a general edge bevel modifier.

`inset` offsets the edges inward with mitered corners on a convex planar face. Excessive distances that collapse or invert the inset are rejected. The inner face keeps the original ID. Add a signed nonzero `depth` to extrude the inset cap in the same edit. Positive depth follows the face normal and negative depth recesses it.

```json
{"command":"inset","face":12,"distance":0.08,"depth":-0.04}
```

To extrude a planar patch, pass its face IDs in `faces`. This preserves the cap face IDs, adds walls along the perimeter, and returns `cap_faces`, `wall_faces`, and `vertex_remap`. The displacement must have a nonzero component along the patch normal.

```json
{"command":"extrude","object":2,"faces":[12,13],"distance":0.12}
```

Use `thicken` to duplicate an open sheet, orient the two layers outward, and close all boundary loops with walls. Supply `object` or `faces:[ids]` and positive `thickness`. A face list must form one edge-connected sheet detached from the unselected faces. Planar sheets default to the face normal. For a bent sheet, supply `direction:[x,y,z]`, a common offset direction pointing through the front of every face. The command normalizes this vector. Thicken returns the new cap and wall IDs and a vertex remap. The layer spacing is measured along that shared direction.

```json
{"command":"thicken","object":2,"thickness":0.06,"direction":[0,1,0]}
```

These operations validate topology and do not check global self-intersection. Region extrusion and thickening copy cap appearance from the source faces. New walls have no automatic UV unwrap.

For bounds-based placement, this command puts object 2's bottom 0.02 units above object 1's top:

```json
{"command":"object_move","object":2,"align":{"target":1,"axis":"Y","from":"min","to":"max","offset":0.02}}
```

`from` and `to` each accept `min`, `center`, or `max` on the requested world axis. The optional signed offset is added to the target coordinate. Only the chosen axis moves. Alignment uses a single source object and a distinct target object.

`bridge_faces` joins two capped ends, either on different components or on one mesh to form a handle/opening. The faces must be convex, planar, vertex-disjoint, have equal vertex counts and face the gap between their centroids. Each cap edge must already have one neighboring face. The second loop is traversed in reverse to preserve winding. By default, the command considers cyclic offsets and chooses the valid correspondence with the shortest total squared connector length. Supply `offset` (an integer from zero to the loop size minus one) to choose which vertex of the second face matches the first vertex of the first face. The command returns that offset, `vertex_pairs`, `removed_faces` and `created_faces`; all end vertices are selected and no vertices are created or moved. Side quads must be planar, convex and nondegenerate. Unequal loops, warped or twisted side quads and invalid topology are rejected atomically. This is a straight bridge; there are no intermediate segments or global self-intersection checks. Subdivide unequal loops first or shape additional sections with extrusion before joining them.

Examples (JSON commands for the CLI or HTTP endpoint):

```json
{"command":"weld","vertices":[25,26,27,28]}
{"command":"bevel_vertex","vertex":25,"fraction":0.15}
{"command":"split_vertices","vertices":[25],"faces":[23,24]}
{"command":"split_edge","edge":[1,2],"fraction":0.5}
{"command":"split_face","face":4,"vertices":[4,7]}
{"command":"inset","face":4,"distance":0.2}
{"command":"bridge_faces","faces":[6,11]}
{"command":"capture","yaw":45,"pitch":15,"zoom":95,"clean":true}
```

These examples illustrate separate operations; IDs must be obtained from the current mesh. Each topology command is atomic, undoable, revision-checked and followed by validation. Use `inspect` and the returned operation mapping to follow changed topology.

## Codex validation loop

1. Start server; inspect the document with the CLI.
2. Capture a PNG and inspect it; compare shaded and silhouette views with the reference.
3. Select a rectangle against its `view_id`; capture again to confirm highlighted vertices.
4. Move the selection; inspect positions and adjacency; capture from another angle.
5. Extrude face 4 upward, inspect boundary/non-manifold counts, undo and redo.
6. Export FBX and save/reload the native project.

Run `cargo test` for engine and CLI integration checks. Run `npm test` for viewer geometry and camera checks.

For localized work, query `{"command":"inspect_region","faces":[40]}` and capture a closeup with `{"command":"capture","center":[0,2.14,0.48],"zoom":650}`. The optional camera `center` defaults to the origin and is included in capture metadata. Image selection uses the same centered projection.

Capture metadata includes `thin_features` for the declared camera and output resolution. Set `thin_feature_pixels` to a positive threshold (default 2). The report counts short authored edges and narrow faces with all vertices in frame, including occluded geometry. Face width is the span perpendicular to the longest projected perimeter edge. Counts and location samples are advisory until a project constraint applies.

Choose capture detail with `width` and `height`, for example `{"command":"capture","width":400,"height":300}` for a quick check or `{"command":"capture","width":1600,"height":1200}` for a detailed view. Each dimension must be an integer from 64 to 4096, with at most 4,194,304 pixels total. Omitted dimensions independently default to 800 and 600. All render modes support the same sizes. Zoom is measured in pixels/unit at the 800×600 reference resolution, then multiplied by `min(width/800, height/600)`, preserving framing at the same aspect ratio. Other aspect ratios expand the field of view along the longer axis. `camera.pixels_per_unit` reports the effective projection scale. `select_image.rect` always uses actual output-image pixel coordinates. Dimensions, command input and render results are retained in the workspace replay log.

## Curved surfaces and detailed props

| Command | Parameters and behavior |
| --- | --- |
| `sweep` | `name` or destination `object`, simple closed `profile:[[u,v],...]`, `path:[[x,y,z],...]`, optional `up` (default X), `scales:[[u,v],...]` per path point. Sweeps capped sections with transported frames and triangulated sides. Alternatively use `path:{"helix":{"radius":[a,b],"height":h,"turns":n,"steps":s,"center":[x,y,z],"start":degrees}}`, with optional `axis` X/Y/Z (default Y); helix sections use radial frames. Profile limit 64 points, path limit 4097 points, vertex budget 150,000; at least 12 steps per helical turn. Repeated/reversing paths, parallel up vectors, invalid scales and degenerate topology reject atomically. Returns ring ID ranges. Tight bends and overlapping sections require visual validation; global self-intersections are not detected. |
| `revolve` | `name` (or destination `object`), `profile`, `segments` (8–512), optional `axis` (X/Y/Z, default Y), `center`, `origin`, `start` (degrees), `sweep` (greater than 0, at most 360), `closed_profile`. Creates shared rings, welded poles and a welded full-sweep seam. Partial sweeps have triangulated end caps. Returns ring ID ranges and face bands. |
| `inspect_loop`, `select_loop` | `edge:[a,b]` traces a closed regular quad edge loop. Inspection is read-only; selection is undoable. Ambiguous junctions, triangles and branching/open paths are rejected. |
| `triangulate` | `object`: triangulate convex planar polygons, preserving each source face ID for its first triangle. |
| `bisect_plane` | `object`, `normal`, `offset` or `offsets:[...]` (up to 32): split convex planar faces at `dot(unit(normal),position)=offset`, retaining both sides. Intersection vertices are shared; the last cut is selected. Returns cut vertices and face mappings. Optional `snap_tolerance` (0–0.001 world units, default zero) projects near-plane vertices and reports their IDs. Exact cuts do not move existing vertices. |
| `radial_offset` | `axis`, optional `center`, `distance`: change selected vertices' distance from an axis, preserving axial positions; crossing the axis is rejected. |
| `radial_profile` | `axis`, optional `center`, `profile:[[axial_position,radial_offset],...]`: linearly interpolated radial displacement on the current selection. Samples must increase in axial position relative to `center`; outside their span geometry is unchanged. Add support loops first for narrow features. |
| `extrude_polygon` | `name` (or destination `object`), `points:[[u,v],...]`, positive `depth`, optional `axis`, `center`, `origin`, `bevel`, `bevel_segments` (1–16, default 3), `holes`. Require `0 <= bevel < depth/2` (strict upper bound; default 0). `center` offsets the outline's starting plane. Extrusion extends from that plane by positive `depth` along `axis`. Caps are triangulated, and concave outlines are supported. Nonempty `holes` require a closed profile with bevel 0. |
| `shade_smooth` | `object`, `angle` (0–180 degrees, default 60): crease-controlled, angle-weighted corner normals in CPU captures, WebGL and native FBX. Zero restores flat normals. Geometry and IDs are unchanged; the setting is saved on the object. |
| `inspect_shading` | `face`, `faces`, or `object`; optional `coplanar:true` grows one seed face across connected, consistently oriented coplanar neighbors. Reports per-face planarity, shading-normal deviation and triangle quality, with offending face/vertex IDs. Read-only; defaults to the worst 16 issues, `limit:0..256`, `normal_threshold` in degrees (default 1), `quality_threshold:0..1` (default 0.1). |
| `shade_region` | Same region selection; `mode:"flat"` overrides selected face corners with geometric normals, `mode:"smooth"` removes that override and resumes object shading. Other faces retain their existing shading. Positions, topology and origins are unchanged. |
| `retriangulate_region` | Same region selection; requires one consistently wound planar patch with one simple boundary. Triangulates polygons and flips internal diagonals when the minimum quality of the two affected triangles improves. Preserves every existing vertex and the boundary. Up to 64 improvement passes; not a globally optimal remesher. |
| `inset_region` | Same region selection plus positive `distance`: adds a coplanar support ring inside a triangulated or concave cap. Preserves all existing positions and the outer boundary. Currently requires one simple boundary and no interior vertices. Collapsing, crossing or invalid offsets reject atomically; reduce the distance for tight corners. |

Use `"profile":{"ellipse":{"radii":[0.13,0.088],"sides":12}}` for a centered sweep cross-section sampled in local U/V coordinates. Radii must be positive and sides must be 3–64.

Sweep paths can also be generated from compact descriptors:

| Descriptor inside `path` | Sampling |
| --- | --- |
| `"line":{"start":[0,0,0],"end":[0,3,0],"segments":24}` | Straight line with evenly spaced rings. |
| `"bezier":{"start":[0,0,0],"controls":[[0,1,0],[0.2,2,0]],"end":[0.6,3,0],"segments":24}` | Uniform parameter steps. Use one control point for a quadratic or two for a cubic. |
| `"spline":{"points":[[0,0,0],[0.3,1,0],[-0.3,2,0],[0,3,0]],"segments_per_span":8}` | Uniform Catmull–Rom spline through every waypoint, with the same sample count per span. |

Generated paths include both endpoints and support up to 4096 segments in total.
Use `scale_keys:[[t,u,v],...]` in place of `scales` with any sweep path. Keys use strictly increasing normalized ring indices from 0 to 1 and positive scale factors. Scales interpolate linearly between keys and hold the nearest key outside that interval. For example, `[[0,1.55,1.55],[0.15,1,1],[1,0.45,0.45]]` creates a flared base and a tapered tip.

`radial_profile` also accepts a function in the `profile` field:

```json
{"command":"radial_profile","axis":"Y","profile":{"taper":{"range":[0,3],"offsets":[0.25,-0.1],"ridge":{"amplitude":0.06,"cycles":8}}}}
```

Offsets are additive in mesh units and interpolate linearly over the axial range relative to `center`. The optional ridge adds a raised cosine that starts and ends at zero in each cycle. Amplitude must be nonnegative and cycles must be an integer from 1 to 4096. The function moves selected vertices within the range. Add enough rings to resolve each ridge before applying it.

For openings, supply each inner contour in `holes`. Each hole must be strictly inside the outer contour. Touching, overlapping, and nested holes are rejected. Either winding is accepted, with up to 32 holes and 1,024 total profile points. The caps leave the openings clear, and each hole receives its own inner walls. An optional generator UV layout gives each contour a separate strip within the side region.

```json
{"command":"extrude_polygon","name":"Window wall","axis":"Z","depth":0.24,"points":[[0,0],[3,0],[3,2.4],[0,2.4]],"holes":[[[0.9,0.8],[2.1,0.8],[2.1,1.75],[0.9,1.75]]]}
```

A small rectangular panel example uses a bevel of 0.02, below half its 0.065 depth (0.0325):

```json
{"command":"extrude_polygon","name":"Practice panel","points":[[-0.5,-0.5],[0.5,-0.5],[0.5,0.5],[-0.5,0.5]],"axis":"Z","depth":0.065,"bevel":0.02,"bevel_segments":3}
```

Region growth uses absolute plane `tolerance` (default `1e-8` world units) and compares all faces against the seed plane, avoiding cumulative drift. Geometry repairs return source/result face IDs, created/removed IDs and edge-flip counts. Reused IDs identify replacement faces, not unchanged diagonals. Uniform flat overrides propagate through these region repairs; mixed overrides must be resolved first. `mesh.flat_faces` stores optional face IDs in native projects and participates in undo/redo, scope checks and validation. Deleted face IDs are removed from this set; unrelated new faces follow object shading by default. Legacy projects without the field retain their appearance.

For an embossed cap, diagnose first, repair triangle quality if needed, preserve its flat shading, then compare shaded and normal captures:

```json
{"command":"inspect_shading","object":6,"face":54060,"coplanar":true,"limit":8}
{"command":"retriangulate_region","object":6,"face":54060,"coplanar":true}
{"command":"shade_region","object":6,"face":54060,"coplanar":true,"mode":"flat"}
```

Obtain seed IDs from the current mesh or a capture. Normal deviation is expected on curved surfaces, so it is a diagnostic rather than a mesh-validity failure. Triangle quality is `4√3 × area / sum(edge_length²)`, from 0 (degenerate) to 1 (equilateral); fan triangles are measured for polygons. Narrow outlines can legitimately retain low-quality triangles. A support ring is an alternative that confines interpolated bevel shading to a narrow perimeter strip without a flat override. These repairs do not perform general remeshing or global self-intersection checks.

Revolve profiles contain `[radius,height]` points and/or sampled arcs, for example:

```json
{"command":"revolve","name":"Sphere","segments":128,"profile":[{"arc":{"center":[0,0],"radius":1,"start":-90,"end":90,"steps":96}}]}
{"command":"shade_smooth","object":2,"angle":55}
```

Adjacent coincident samples merge. Solid profiles start and end on the axis, with no intermediate axis crossings; `closed_profile:true` closes an annular cross-section away from the axis. Order profiles counterclockwise in radius/height coordinates. Limits are 1,024 profile samples and 150,000 generated vertices. The right-handed radius basis starts on the next cyclic axis and rotates toward the following one (Y: Z toward X; Z: X toward Y). `center` places geometry; `origin` independently sets the exported pivot. The example appends to the initial cube; use actual object/vertex IDs returned by the tool.

These operations use document rollback, object scope guards, undo/redo, compact replies and workspace recording. Face degeneracy is tested relative to local scale, allowing valid small triangles in dense cuts while rejecting collapsed or nearly collinear faces. Use `solid_boolean` for closed solids and `subdivision_surface` for editable cages. Their recipes regenerate explicitly. The CLI response timeout is 120 seconds for dense operations.

## Editable solids and subdivision

Build the sources first. Use their returned object IDs on the next command. A derived solid or subdivision surface gets its own object and saved generator recipe. Use `region` to give the result a stable name. In an existing destination, use `object` and an explicit `mode` as with lofts and sweeps.

| Command | Inputs and behavior |
| --- | --- |
| `solid_boolean` | `base`, `tools:[...]` (1–64 distinct object IDs), and `operation` of `union`, `difference`, or `intersection`. Combines closed, outward-wound solids in tool order and reports the volume in cubic mesh units. An empty result rejects the edit. Each triangle inherits its source face material and interpolated complete UVs, including cut walls from the tool. Paint geometry with vertex colors or skin weights after the boolean. |
| `subdivision_surface` | `source`, optional `levels` (1–6, default 1), and optional `creases:[[a,b],...]` using source edge IDs. Evaluates Catmull–Clark quads up to one million output faces. Boundaries and explicit creases follow the crease rule, and three sharp edges pin a corner. Carries complete face UVs across seams with linear interpolation, material assignments, vertex colors and up to four normalized bone influences. |
| `regenerate_component` | `object`, `region`, optional `overrides` containing generator arguments. Re-evaluates the saved recipe from current source geometry. The region name and component ID survive. New topology gets fresh IDs. Same-generator primitive replacements transfer UVs, materials, flat faces, paint and weights when ordered face connectivity matches. The reply reports `finish_transferred` for that case. Generated booleans and subdivision derive their finish from the current sources. |
| `uv_unwrap` | Select an `object`, `faces`, or named `region`. Optionally pass `seams:[[a,b],...]`, square atlas `resolution` (16–4096, default 1024), and `padding` in pixels (default 2). Creates charts and packs them into 0–1, respecting material boundaries. Returns chart count and texels per mesh unit. Geometry and face IDs survive. UV changes affect existing texture placement. |
| `uv_pack` | Same scope, resolution and padding. Repacks existing complete UV islands. If a folded polygon requires a seam across its interior, split the polygon first. |

For example, with housing 1 and a cutter 2, run:

```json
{"command":"solid_boolean","base":1,"tools":[2],"operation":"difference","name":"Vented housing","region":"vented"}
```

Use the returned destination ID on subsequent commands. For example, if the result is object 3, inspect it using `{"command":"capture","solo_objects":[3]}`. Move the cutter or edit the source, then send `{"command":"regenerate_component","region":"vented","object":3}`. For a preview, run that edit in a transaction with `commit:false`. If a downstream generator uses the result, regenerate it next or put both generator steps in a named construction.

To compare subdivision levels, create a named result at level 1, inspect the silhouette, and regenerate with `overrides:{"levels":3}`. Apply manual finishing to the sources when it needs to survive a rebuild. Use the existing workflow checks and `{"command":"export","format":"glb","solo_objects":[3]}` to deliver the result. GLB accepts the same solo/hide object and collection filters as capture.

**Resize a finished assembly.** `stretch_band` takes `axis`, increasing world-coordinate `span:[min,max]`, and signed length `delta`. Vertices within the span stretch linearly about its center. Each side moves rigidly by half the length change. Use `object` for one body or `objects:[...]` for several. With `objects`, optional `rigid_groups:[[id,...],...]` translates each hardware assembly by the displacement at its combined bounding-box center. Group the dial, its pointer and tick marks together. UVs, face IDs, bindings and vertex weights survive. Edits act on rest geometry. The command participates in the existing transaction preview, instance and undo workflows.

```json
{"command":"stretch_band","axis":"X","span":[-0.05,0.05],"delta":0.15,"objects":[1],"rigid_groups":[[2,3],[4,5]]}
```

**Bake surface detail.** Assign an unwrapped target its texture, keep the detailed source at the same world position, then run `{"command":"texture_bake_normals","texture":"Finish","source":4,"target":3,"max_distance":0.02}`. The nearest same-facing source hit in either direction along the target shading normal supplies the tangent-space normal. The result becomes the shared texture stack's base normal channel. Existing layers still apply. Inspect the reported hits and misses. Missed texels get neutral normals. Optional `padding:0..16` dilates successful samples into uncovered texels for filtering.

For a separate PNG, use `bake_surface_map` with the same source, target and distance, `channel:"normal"` or `"displacement"`, and `resolution:1..4096`. Optional `texture` limits the target faces to that assignment. Use CLI `--output detail.png`. Normal PNGs use the native UV convention. Displacement PNGs use linear 16-bit grayscale. Convert a sample code to mesh units with `(code - zero_code) * units_per_code`, using the returned values. This read-only bake leaves the working texture stack and revision intact.

## Export

Run `workflow_status` before exporting. The editor requires all applicable workflow checks, including `export_validation`. See the `modeling-requirements` topic for assessments and the `objects-origins-and-game-assets` topic for pivot and format details.

- Use `format:"obj"` for static geometry and named objects.
- Use `format:"fbx"` for static geometry with object pivots.
- Use `format:"stl"` for mesh-only binary STL. It triangulates the scene's rest geometry in world space and converts from Y-up meters to Z-up millimeters. Import using millimeters because STL has no unit metadata. Each facet has a normal. STL omits object names, pivots, materials, UVs, colors, and rig data. Export preserves the mesh's existing shells and does not repair gaps or fuse overlapping objects.
- Use `format:"glb"` for glTF geometry, with skinning and clips when the workspace has a rig. See `rigging-and-animation` for animation details.

```powershell
'{"command":"export","format":"glb"}' | atk-mesh-cli --port 7879 --output model.glb
'{"command":"export"}' | atk-mesh-cli --port 7879 --output model.stl
```

Use `texture_export` to save texture images for the consuming application. Import the result in the intended tool and verify its geometry, transforms, and animation as applicable.

## Object detail stages

Use a stage to record how far each object has progressed for its intended style and use. A stylized part can be ready with a few clean shapes. Judge it against its own brief, viewing distance, and production requirements.

| Stage | What to look for |
| --- | --- |
| Blockout (`blockout`) | Establish scale, silhouette, major volumes and the placement of parts. Proportions and interfaces are still being worked out. New objects and objects without a saved label start here. |
| Refined (`refined`) | Ready to use as a placeholder for its declared purpose. Verify primary forms, required visible features, assembly and placement from the intended views. |
| Detailed (`detailed`) | Resolve secondary forms and finish to the target style. Check details such as bevels, seams and recesses at the intended viewing distance. Finish topology and any appearance or motion work the part needs. Production review is still pending. |
| Production ready (`production_ready`) | Review the object for its intended use. Verify fit and geometry, plus shading, UVs, materials, motion and budget where applicable. Record the intended use and the checks performed. |

Set a stage by object ID. Revisit the label as the part evolves.

```json
{"command":"object_set_stage","object":12,"stage":"detailed","note":"Checked seams and highlight continuity in the close-up review views."}
```

Use `query_mesh` with `kind:"objects"` to read stages and notes. The viewer shows stages in the Objects panel and includes them in its search. New duplicates and instances begin at Blockout with their own checklist. Workspace workflow and export validation run separately.

Define the checks before detailing so the reviewer knows what to look for. Cover each review area with one or more observable criteria appropriate to the object’s brief, viewing distance and intended use.

| Area | Review question |
| --- | --- |
| `form` | Do the silhouette, proportions and primary forms match the brief in the intended views? |
| `interfaces` | Do supports, attachments, seams, fit and any intended motion read correctly? |
| `finish` | Do the details, shading and applicable materials hold up at the intended distance? |
| `technical_fit` | Does the object meet the relevant topology, budget, rigging or integration requirements? |

```json
{"command":"object_review_plan","object":12,"checks":[{"id":"profile","area":"form","criterion":"The jaw tops align in side view."},{"id":"mount","area":"interfaces","criterion":"The screw visibly seats in its supports."},{"id":"grip","area":"finish","criterion":"The grip grooves shade evenly in close-up."},{"id":"topology","area":"technical_fit","criterion":"The surface defect scan is complete and finds no unexpected defects."}]}
{"command":"object_review_check","object":12,"check":"mount","outcome":"needs_work","reason":"The screw appears unsupported in side view.","evidence":["resource-4"]}
{"command":"query_object_review","object":12}
```

Use `pass`, `needs_work` or `not_applicable` and give a reason. A pass requires registered evidence. Passes for form, interfaces and finish require at least one current capture. Use `workflow_resource` with a capture trace and an optional object ID, or register a technical report as `kind:"evidence"`. For a saved review report use `kind:"evidence",capture:"report"` with the `review_run` trace. Use workspace-scoped evidence for checks that depend on other objects, such as the fit between two parts.

The CLI checks that every result is resolved, at least one passes, and the cited files, geometry and rig are current before accepting `object_set_stage` with `stage:"production_ready"` and a note. The reviewer judges whether the view actually shows the feature and whether it passes. A clean defect scan supports technical fit, while form, interfaces and finish have their own criteria.

For **Refined**, record the intended camera, viewing distance, scale and required interaction in the workspace intent. Record an approved reference or shape brief, accepted primary forms, and two distinct workspace views. Then use `workflow_assess` with each of the following requirements and an `outcome` of `accepted` or `needs_revision`.

| Workspace requirement | Evidence to review |
| --- | --- |
| `placeholder_features` | List the visible features the brief requires and use captures that show each one clearly. |
| `placeholder_assembly` | Show how the primary parts fit. Include representative poses if the brief requires motion. |
| `placeholder_placement` | Show scale, orientation and pivot in the intended camera and placement context, or document them against a measured stand-in. |

Add at least one `required_features` check to each object plan. Form and required features must pass, and interfaces must pass or have an explicit reason they do not apply. Visual passes require current captures. Use `object_set_stage` with `stage:"refined"` to see workspace and object blockers. Relevant object or assembly edits reopen the Refined review. The gate applies to new promotions to Refined. Saved legacy labels can be reset explicitly.

Changing geometry, appearance, pivot, instance source or relevant rig data reopens the affected object’s checks. Assembly changes also reopen checks that used workspace-scoped evidence. Replacing a checklist clears its results. Reopening an approved object returns it to Detailed. Older labels without a completed checklist also appear as Detailed.

## Objects, origins and game assets

`mesh.objects` maps stable IDs to unique names, world-space origins and owned vertex IDs. Vertices remain authoritative world-space coordinates so existing editing, capture and viewer paths share one representation. Every vertex belongs to exactly one object, and a face cannot span objects. Topology edits inherit ownership; welds and bridges between separate objects are rejected. Add `object` to a mutation to guard other objects' geometry, ownership and origins. `inspect_region` also accepts `object`; omit face/vertex IDs to query the whole object. Object ownership and origins participate in revision checks, save/load and undo/redo. Projects without newer metadata retain version 2 for geometry or version 3 for basic appearance; authored component/construction metadata uses version 6. Legacy version-1 projects gain one object at the world origin.

Set the arch origin at its ground anchor and the gate origin on its vertical hinge line. `object_set_origin` preserves geometry exactly. `object_move` moves the origin with the object. `object_rotate` rotates geometry around that origin, independent of vertex selection. Rotation is baked into the current mesh pose; it does not store an animation track or a separate rest pose. Export the closed pose as the runtime starting pose, then rotate the imported gate object in game code.

```json
{"command":"object_set_origin","object":2,"origin":[-1.42,-2.36,0]}
{"command":"object_rotate","object":2,"value":[0,90,0]}
{"command":"inspect_region","object":2}
```

FBX is written directly in Rust as binary FBX 7.4, without Blender or an Autodesk SDK dependency. It contains separate named mesh nodes, polygon topology, flat or crease-controlled smooth corner normals and origin-relative vertex positions with node translations. Coordinates are right-handed and Y-up; one unit is one meter, recorded in FBX unit metadata. OBJ preserves separate object names but cannot carry the origin transforms.

The exporter covers static geometry, pivots, explicit UVs and vertex colors. Texture image bindings remain in the native project; use `texture_export` to save image files and assign them in the consuming application. Material graphs, skinning, animation tracks and a parent hierarchy are not exported. Engine-specific import settings still need testing in the intended engine. The binary layout reference is [Blender's FBX format documentation](https://code.blender.org/2013/08/fbx-binary-file-format-specification/).

Earlier exports omitted scene/header metadata and empty-container terminators and were rejected by that reader despite importing in Blender. Regenerate older FBX files with the updated exporter. Direct Unity Editor import remains to be retested.

## Joining objects and deleting faces

`object_join` transfers source objects into an existing `target`, preserving all vertex/face IDs, world positions, UVs, vertex colors, texture bindings and per-face flat shading. The target keeps its ID, name, origin and primary collection; collection memberships are combined, and source object records/primary assignments are removed. All resulting vertices are selected. Objects must have matching `smooth_angle` (set explicitly with `shade_smooth` if necessary), and all involved instance links must first be detached with `object_make_unique`, including instances of involved masters. `objects` lists sources only; duplicates are deduplicated. Use `target`, not the single-object `object` scope.

```json
{"command":"object_join","target":2,"objects":[3,4]}
{"command":"delete_faces","object":2,"faces":[123,124],"remove_orphan_vertices":true}
```

Joining combines ownership; it does not weld vertices, bridge gaps, union intersecting solids or remove internal faces. Use `delete_faces` to remove explicit faces, then `weld_coincident` with its expected-count guard if matching vertices should be welded. Inspect before and after each step. No global self-intersection check is performed.

`delete_faces` requires one `object`, a nonempty array of `faces` belonging to it, and explicit `remove_orphan_vertices:true` or `false`. Unknown or foreign IDs reject the entire edit. With `true` it removes only vertices made unused by these faces; `false` retains them for reconstruction. Selection, UVs, vertex colors, texture assignments and flat-face flags are reconciled while surviving IDs, object origin and collections are preserved. Texture resources remain available. It may leave open boundaries or disconnected face fans; inspect topology before subsequent operations that require manifold geometry. Deleting every face of an object outside a transaction is rejected: use `object_delete` for a whole object. Instance masters propagate deletion through the existing linked-instance mechanism; edit a master or make an instance unique before deleting its faces.

Both commands use normal atomic document edits, revision guards, undo/redo, workspace persistence and replay. They do not bypass workflow requirements. Discover them in JSON `help` under `geometry_cleanup`.

### Measuring the source mesh

`inspect` and saved reviews include `source_metrics`. Object and material totals count triangles and authored face corners, including corners carrying UV or color values. Texels per meter is the square root of mapped texture area divided by its corresponding surface area. A null density indicates insufficient mapped surface area. Projection and generator regions are checked per UV corner, including corners moved by later UV edits.

Each connected shell reports boundary and non-manifold edge counts. Fold candidates identify face pairs with a normal change greater than 170 degrees. The overlap scan reports its completeness, tested triangle pairs, and detected contacts. The closed-shell integrity gate checks inconsistent winding, inward orientation, and unintended overlap within the same closed shell. Declared contact allowances apply to overlap findings. Back-facing pixels and projected feature widths are measured from each capture's camera.

Supply `constraints` on a project or object `workflow_assess` for `intent`. For example, `{"command":"workflow_assess","requirement":"intent","object":1,"assessment":"Closed housing with a ground pivot","evidence":[],"constraints":{"closed_surfaces":true,"pivot":[0,0,0],"max_triangles":2000}}`. Completion and export evaluate the saved constraints against the current mesh and return the failed metric, target, and measured result. Counts support `max_triangles`, `max_corners`, `max_uv_corners`, and `max_painted_corners`. `max_texture_size` bounds the longest side of any assigned image. For a project-level pivot constraint, every object origin is checked.

`min_wall_thickness` takes a positive distance in meters. Its report describes the sampling method and coverage. `clearance` accepts `minimum` and the surface selectors `a` and `b`. In an object intent, `a` defaults to that object. `protected_volume` accepts the bounds used by `inspect_volume`. `thin_features` accepts a capture `camera`, `threshold_pixels`, and allowed `max_faces` and `max_edges` counts. These depend on the declared camera and resolution.

### Instance scale

`object_instance` accepts an optional `scale` number or three positive factors in master-local axes. Use `{"command":"object_scale","object":2,"value":[2,1,0.5]}` to multiply the placement scale of a linked instance. The instance keeps its master link through save, reload, and later master edits. Captures, viewer geometry, and mesh exports transform its shading normals using the inverse transpose.

## Explicit construction and named components

`sweep`, `loft`, `revolve`, and `extrude_polygon` accept an optional UV layout. Supply normalized side and cap rectangles as `[u_min,v_min,u_max,v_max]` and an optional inset measured in UV units.

```json
{"command":"sweep","name":"Bent tube","profile":[[-0.1,-0.1],[0.1,-0.1],[0.1,0.1],[-0.1,0.1]],"path":[[0,0,0],[0,1,0],[0.4,2,0]],"uv":{"side_region":[0,0,0.75,1],"cap_region":[0.75,0,1,1],"padding":0.01}}
```

U follows the section profile and V follows the sweep or loft path. On a revolution U follows the radius-height profile and V follows the rotation. Closed profiles have distinct UV corners at their seam. Planar end caps share the cap rectangle in two separate islands when both ends are capped. Component recipes retain the target regions for later inspection. During regeneration an explicit UV layout takes precedence over transferred UVs.

For new projects, a polygon or stroke mask supplied while creating a texture layer or defining a named mask starts with zero background coverage. An explicit `fill` overrides the default. Projects loaded from versions 1–8 retain the historical full-background default. Layer add and set replies include `coverage`, the mean resolved mask value over the texture, before layer opacity and visibility.

Generating geometry into an existing `object` now **requires** `mode:"append"` or `mode:"replace_region"`. This applies to `object_add_box`, `object_profile`, `extrude_polygon`, `revolve`, `sweep`, `loft` and `quad_strip`. Update older recipes by adding the intended mode. An object ID communicates ownership only. Creating a new named object can omit the mode.

```json
{"command":"extrude_polygon","object":59,"mode":"append","region":"cab_upper","points":[[0,0],[2,0],[2,1],[0,1]],"depth":3.2}
{"command":"extrude_polygon","object":59,"mode":"replace_region","region":"cab_upper","points":[[0,0],[2.1,0],[2,1],[0,1]],"depth":3.2}
{"command":"query_components","object":59,"offset":0,"limit":16,"response":"full"}
```

Use `query_components` with `report:"triangles"` to rank authored components by their triangulated cost. Each row gives the triangle count, percentage of the whole asset, and axis-aligned XYZ dimensions in model units. Filter with `object` or `objects`, and optionally `region` or explicit `faces`. Add `selection:true` to include only faces whose vertices are all selected. The `scope` total counts each matching face once, even when named regions overlap. The `unassigned` total covers matching faces outside all named components. Empty scopes have zero triangles and null dimensions. Results support `offset`, `limit` and `revision`.

```json
{"command":"query_components","report":"triangles","limit":10}
{"command":"query_components","report":"triangles","selection":true}
```

Each generator returns `operation.component_id`, `region`, `mode`, `bounds`, `geometry_counts`, `created_vertices`, `created_face_ids`, and `removed_faces`, alongside its existing generator-specific receipt. Regions have globally unique names; omit `region` to get `component-N`. Existing names require replacement explicitly. The same component ID survives replacement. Query component records with `query_components`; page with `next_offset`, and use `response:"full"` for exact face/vertex arrays and saved recipes. Operation metadata keys/scalar IDs remain visible in compact replies; large arrays still use count/sample/path metadata and `query_record` retrieval.

Replacing a named, detached component removes its faces and newly unused vertices, constructs the replacement, and validates within one undoable edit. Failure restores the original and allocation state. Regions sharing vertices with surrounding surfaces cannot be replaced this way: use connected surface repair instead. Source mesh IDs within the replaced component are retired; geometry outside it stays unchanged. Bound-overlap warnings identify object candidates only, explicitly report surface intersections as unchecked, and never claim that bounding-box contact proves a defect. Run `inspect_defects` for the precise surface checks below.

Authored component records are named face regions, not a promise of one connected solid. They survive ownership joins and track surviving faces plus locally derived topology where correspondence is known. Bridging between different regions can need an explicit region definition for the new connector. Topological connected components are reported independently by inspection. Component/region/construction metadata and intersection allowances use native **project version 6**; versions 1–5 still load. Projects without these features retain their previous version. Keep the current tool when editing version 6 projects.

## Connected surface construction

| Command | Input and behavior |
| --- | --- |
| `query_boundaries` | `object`, optional pagination. Returns ordered closed boundary loops and branching vertices. These are boundary loops, unlike the regular quad-ring traversal in `inspect_loop`. |
| `bridge_loops` | `object`, `loops:[[vertex IDs],[vertex IDs]]`, optional `offset`. Both closed boundaries must be disjoint and equally sized. Winding is oriented against existing faces; valid planar convex quads are chosen by shortest correspondence unless offset is supplied. `reverse:true` explicitly reverses the second oriented loop; invalid winding then fails validation. |
| `create_face` | `object`, ordered `vertices:[...]`. Fills one existing planar, simple closed boundary, including concave boundaries. Winding follows adjacent faces. No invented UVs or texture binding. |
| `dissolve_region` | `object`, `faces:[...]` or `region`. Combines a connected, consistently wound planar patch with one boundary and no holes into an n-gon, retaining the lowest face ID and boundary vertices. |
| `dissolve_edges` | `object`, `edges:[[a,b],...]`. Dissolves the requested internal two-face edges sequentially. Nonplanar patches, texture/shading differences, UV seams and non-affine UV interpolation reject atomically. Newly unused interior vertices are removed. |
| `extrude_polygon` | Existing fields plus `closed_profile` (default true), `cap_start`/`cap_end` (default true for closed profiles). Disable caps to make an open shell. `closed_profile:false` extrudes a polyline to a quad ribbon, needs at least two points, forbids caps and requires bevel 0. |
| `loft` | `contours:[[[x,y,z],...],...]` or `loops:[[existing vertex IDs],...]`; compatible counts, 2..256 sections, `closed:true` by default. Produces quads between matching indices. Optional `cap_start`/`cap_end` default false. Caps need planar closed contours. |
| `quad_strip` | Same paired inputs as loft, exactly two contours, `closed:false` by default. Useful for an inner wheel-opening contour and its outer arch contour. Existing-ID loops reuse vertices rather than overlapping a new shell. |
| `sweep` | Existing profile/path inputs plus `side_faces:"triangles"` (default) or `"quads"`, and `cap_start`/`cap_end` (default true). Quad output rejects warped or nonconvex sides; choose triangles or additional sections deliberately. |
| `bevel_edges` | `object`, `edges:[[a,b],...]`, positive `width`, `segments:1..32`. Use `corner_mode:"isolated"` for vertex-disjoint convex manifold edges with three incident faces at each endpoint. Use `corner_mode:"rim"` for the complete perimeter of one convex planar cap with perpendicular side edges. The shared corner rings keep the rim watertight. A single segment makes a chamfer. |

With two explicit `contours`, `loft` accepts `"interpolation":{"type":"linear","segments":8}` to generate nine sections including the endpoints. Set `type` to `smoothstep` to concentrate sections near the endpoints. Both modes interpolate corresponding XYZ coordinates and produce the same overall shape with different ring spacing. Segments must be 1–255 and the output is limited to 150,000 vertices.

Loft/strip correspondence is the caller's contour order; it never guesses an offset or reverses individual contours. Quads must be planar, convex and nondegenerate. Existing-loop winding must match adjacent surfaces. Bevel strips and other new faces have no automatic UV unwrap or texture assignment; review affected UVs after topology edits. These commands do not perform solid unions or global intersection repair.

## Repair transactions

`transaction` with `commands:[...]` runs 1..128 native document edits against a temporary document. Use `commit:false` to preview or `commit:true` to validate and commit in one request. Intermediate topology may be open or invalid. The completed mesh must pass topology validation and the requested guards. A commit makes one undo entry. Reset/load, history, workflow, query, export and capture commands are excluded. Ordinary `--batch` remains sequential and non-atomic.

A successful preview returns a `preview_id` and its base `revision`. After reading the evidence, send `{"command":"transaction","commit":true,"preview_id":"ID_FROM_PREVIEW","revision":42}` with the returned revision. This commits the already validated mesh using its original guards and skips rerunning edits, diagnostics, and renders. The editor keeps one candidate in memory. A successful newer preview replaces it, and a revision change, commit, or editor restart requires a fresh preview. Pass commands and guards in the preview request.

```json
{"command":"transaction","commit":false,"commands":[{"command":"object_join","target":2,"objects":[3]},{"command":"delete_faces","object":2,"faces":[123,124],"remove_orphan_vertices":false}],"guards":{"max_displacement":0,"protected_objects":[4],"no_new_components":true,"no_new_defects":true},"camera":{"yaw":30,"pitch":15,"zoom":100,"width":800,"height":600}}
```

The reply reports exact added/removed/moved vertex IDs, added/removed/changed faces, operation receipts and before/after defect reports. With `camera`, it also produces a normal SVG artifact: green new/changed surfaces, red removed/old surfaces, amber vertex displacement. It is a wire overlay without occlusion, not an accuracy assessment. Review it, then send the same transaction with `commit:true` and the current `revision` as a stale-state guard.

Supported guards: `max_displacement` (surviving vertex IDs only, world units), `protected_objects:[IDs]`, `protected_regions:[names]`, `no_new_components:true` (total and within surviving objects), `no_new_defects:true`, and `protected_volumes:[{min:[...],max:[...],objects:[...]}]`. Volume queries can use explicit faces or named regions. Protected object geometry/ownership/origin and protected region vertices/faces must survive unchanged. Use a protected volume for spatial protection against newly created geometry. Unknown guard names and malformed flags fail. The defect guard requires complete, untruncated before/after reports and respects only explicitly recorded pair allowances.

## Independent defect diagnostics

Default `inspect` reports topology and explicitly labels geometry intersections unchecked. `inspect_defects` performs separate triangle-surface checks; optional `object`, `objects`, `faces` or `region` narrows surface checks. Topology summarizes the whole document. Its `geometry` fields independently report `detected`, `not_detected`, or `incomplete` for duplicate surfaces, coplanar overlaps, self intersections and inter-object intersections. `complete:false` means a pair budget stopped the scan; it must never be interpreted as clean.

```json
{"command":"inspect_defects","object":59,"limit":32,"offset":0,"max_pairs":250000,"tolerance":0.00000001}
{"command":"measure_clearance","a":{"region":"cab_upper"},"b":{"region":"cargo_interior"}}
{"command":"inspect_volume","objects":[59],"min":[-1,0,-1],"max":[1,2,1]}
{"command":"intersection_allow","regions":["front_arch_left","cab_upper"],"reason":"Designed attachment overlap at the arch mounting seam"}
```

Findings include object and face IDs, a world-space location, an intentional flag/reason and limitations. Record allowances for exactly two `faces` or two named `regions`, never a global ignore. Region allowances follow stable component IDs across replacement; deleted face/region allowances are retired. Findings remain visible when intentional. Topology separately reports boundary loops, nonmanifold edges and connected components, including loose vertices. Duplicate faces compare positions even when vertex IDs differ. Coplanar overlap uses projected triangle clipping; noncoplanar contact uses segment/triangle intersections with ordinary shared-edge/vertex contacts excluded within an object. AABB overlap is only the broad phase. Adjacent objects touching are still reported so callers can assess intent.

`measure_clearance` computes unsigned minimum triangle-surface distance with face pairs and closest locations; it is not a signed solid-volume separation. Regions must be nonempty and within a 2,000,000 triangle-pair budget. `inspect_volume` clips triangles against an axis-aligned protected box, includes touching, and checks center-point parity containment for complete closed objects. Open/partial regions explicitly lack a solid-inside interpretation. These are floating-point polygon/triangle diagnostics, not exact CSG, material displacement or collision simulation. Near-coplanar flicker is detected only within the supplied coplanarity tolerance. Arbitrary solid containment is not checked by `inspect_defects`.

Surface findings are paged with `next_offset`; `max_pairs` defaults 250,000 and is limited to 2,000,000. Scans also stop at 8,192 distinct findings and report incomplete. `response:"full"` gives exact arrays; default bounded topology metadata can be expanded with a full query. Inspect both completeness and the selected scope before asserting a clean result.

## Regions and reusable construction

`region_define {name,faces:[...],object?}` names an explicit existing face region. `region_remove {name}` removes only the label. `select_region {region,object?}` selects its vertices. Face-oriented editing/query commands such as `delete_faces`, `dissolve_region`, `shade_region`, UV operations and `texture_assign` accept `region` instead of manual face lists. `mesh_region` is an equivalent explicit selector; use it with `uv_project` when its existing numeric `region:[u0,v0,u1,v1]` also specifies an atlas rectangle. Selection transforms use `select_region` first. Region names are global to the native project and resolve to saved face IDs; they do not search by visual proximity.

`select_related {object,kind,...}` supports `component` from seed `vertices`/`faces`, `boundary_loop` with an index from `query_boundaries`, `adjacent_faces` with `faces` and `rings:1..64`, `coplanar_patch` with one seed face and optional tolerance, and `mirrored` with faces/vertices plus `axis:"X"|"Y"|"Z"`, optional plane coordinate and tolerance. Mirrored lookup returns explicit vertex pairs and rejects missing/ambiguous matches. It selects an existing counterpart; it never silently creates or deforms one.

```json
{"command":"construction_define","name":"cab","parameters":{"width":2.0,"rake":0.2},"steps":[{"command":"extrude_polygon","object":59,"mode":"append","region":"cab_upper","points":[[0,0],[2,0],[{"parameter":"rake","scale":-1,"offset":2},1],[0,1]],"depth":{"parameter":"width"}}]}
{"command":"construction_apply","name":"cab","parameters":{"width":2.2,"rake":0.3}}
```

Definitions contain numeric parameters and 1..64 generator steps with explicit object, append mode and distinct region names. Parameter descriptors evaluate `parameter * scale + offset` recursively in command data; omitted scale/offset mean 1/0. `construction_apply` regenerates all dependent steps in recorded order as one atomic edit, initially appending each missing region and subsequently replacing it. Failed parameters/construction restore everything. All participating steps share the same parameter values. Manual details within replaced regions are deliberately replaced; details sharing vertices require surface repair instead. This is an explicit recorded construction recipe, not an inferred constraint solver or unrestricted scripting language. Query parameter plans with paged `query_constructions` and generated face records with `query_components`; use `response:"full"` for exact saved data.

## Export verification and workflow reports

Define the same preset in each workspace to compare variants at identical world-space scale. `standard_views:true` adds front, side, top, three-quarter, and intended-camera material captures. All inherit the fixed center, projection scale and resolution from `view`.

```json
{"command":"review_define","name":"asset","view":{"width":800,"height":600,"center":[0,3,0],"zoom":75,"yaw":35,"pitch":12},"standard_views":true,"export":{"format":"glb"}}
{"command":"review_run","name":"asset"}
```

An optional `export` object passes the format and options to the exporter during the review. Preflight produces the receipt before the export gate. A passed review also returns the model artifact when the standard export gate permits delivery. The report includes the source revision, preset version, options, artifact hash, source measurements, and findings from inspecting the exported bytes. Static GLB geometry is reimported in memory. The receipt identifies unsupported reimport features. OBJ and STL record pivot limitations. Use the report artifact for the full measurements.

To check an existing workspace file, send `{"command":"export_verify","path":"exports/asset.glb"}`. This validates the file and reports its settings. Add `"baseline":"source"` to compare against a fresh export of the current revision, optionally supplying `"export":{"format":"glb","merge":"all"}`. Alternatively, pass a workspace-relative verification receipt or review report path as `baseline`. A receipt with `status:"failed"` identifies the invalid structure or mismatched field. Register a successful trace with `workflow_resource`, `kind:"evidence"`, and `capture:"report"`.

Send `{"command":"workflow_report"}` to generate a Markdown artifact from workflow state and recorded events. Use `--output workflow.md` to copy it. The report lists assessments, remaining checks, evidence, recorded invalidations, export artifacts, verification results, object stages, and workspace-linked feedback. Include a `workspace` path in a local `feedback_submit` request to link the submission. A project intent assessment can set `require_production_ready:true` to require that object stage for workflow completion. The completion reply lists the actual stages.

## Saved reference reviews

`review_define` stores a named preset in `workflow.json` and freezes the resolved camera and resolution. Supply a registered `reference` and optional crop to request a comparison overlay. Omit the reference for native model reviews against a written brief. Optional `visibility` accepts hide/solo object and collection lists. Optional `dimensions`, `clearances` and `protected_volumes` arrays save up to 64 checks each.

```json
{"command":"review_define","name":"cab-side","reference":"resource-1","view":{"yaw":90,"pitch":0,"zoom":100,"width":800,"height":600},"dimensions":[{"name":"upper cab","region":"cab_upper"}],"clearances":[{"name":"cargo gap","a":{"region":"cab_upper"},"b":{"region":"cargo_interior"},"minimum":0.05}]}
{"command":"review_run","name":"cab-side"}
{"command":"review_define","name":"operator-approach","view":{"projection":"perspective","position":[3,1.7,4],"target":[0,1.3,0],"fov":42,"width":1000,"height":700},"wireframe":false,"defects":false}
{"command":"review_run","name":"operator-approach"}
```

One `review_run` produces a neutral PNG and a report with the mesh revision, preset version, fixed camera and requested measurements. Set `wireframe:false` for a shaded-only view or `tessellation:true` to review how a surface triangulates. Pass `defects:false` to skip the surface scan or `defects:{"objects":[2,3]}` to scope it to those objects. Defaults retain the topology view and full-model defect scan. Reference presets also produce a silhouette overlay and track the reference content hash. Use `query_reviews` to list presets and `review_update` to change them. Runs reject inline camera overrides and changed reference contents. Presets survive reopening.

Save different approach and oblique presets once. Run the views relevant to a revision in one CLI batch, for example `[{"command":"review_run","name":"operator-approach"},{"command":"review_run","name":"cab-side"}]`. Put contact checks on the preset that shows the affected interface and use protected volumes for space the new geometry must leave open.

Read the images/reports before assessing accuracy. To register a produced view as workflow evidence, use `workflow_resource` with `kind:"capture"`, returned trace `session`/`event`, and `capture:"neutral"`, `"wireframe"` or `"silhouette_overlay"`. This evidence does not automatically approve the model or waive workflow requirements. Review presets are workspace workflow state; native geometry, regions, parameters and undo remain owned by the document.

## Removing disconnected parts

`object_delete_components` removes complete vertex-connected components inside one object, preserving surviving IDs, origin and collection membership. Supply seed `vertices:[ids]` and required `expected_vertices` for the total expanded deletion; a count mismatch, foreign seed, or attempt to empty the object fails atomically. Multiple seeds in the same component are deduplicated. Selection and flat-face overrides are cleaned up; deletion participates in undo, workspace saves and replay. Use `object_delete` for a whole object.

```json
{"command":"object_delete_components","object":35,"vertices":[6981,7001],"expected_vertices":40}
```

## Collections

Collections are flat, named sets of object IDs with overlapping membership. Workspace saves, undo/redo and replay retain them. They do not introduce parenting, shared transforms or FBX hierarchy; object origins and exports keep their existing meaning.

```json
{"command":"collection_create","name":"ceiling","objects":[67,68,69,70]}
{"command":"collection_update","collection":1,"objects":[67,68,69,70,98]}
{"command":"query_mesh","kind":"collections","limit":16}
{"command":"query_mesh","kind":"objects","collection":1,"limit":16}
{"command":"query_mesh","kind":"vertices","collection":1,"limit":32}
{"command":"inspect_region","collection":1}
{"command":"object_move","collection":1,"value":[0,0.1,0]}
{"command":"shade_smooth","collection":1,"angle":45}
{"command":"select_collection","collection":1}
{"command":"capture","mode":"collection_ids","solo_collections":[1]}
{"command":"compare_reference","reference":"resource-1","hide_collections":[1]}
{"command":"collection_delete","collection":1}
```

`collection_update` can rename via `name` and replaces membership via `objects`. For lean membership changes, use `add_objects:[ids]` and/or `remove_objects:[ids]` instead of sending the whole set; do not combine these with `objects`. Empty collections are allowed. Deleting a collection preserves objects; deleting an object removes memberships. Duplicates inherit memberships and primary collection. Unknown IDs, duplicate/empty names and invalid primary memberships fail atomically. Queries return counts and paginated records. Collection records provide a member-query command rather than dumping membership or geometry.

Direct collection targeting supports `query_mesh` (objects, vertices, faces, selection), `inspect_region`, `select`, `move_in_view`, `object_move`, `object_rotate` and `shade_smooth`. Moves translate each member and its origin once. Rotations use each member's own origin. Each command is one undoable transaction. For other selection-based edits, use `select_collection` followed by the existing command. Unsupported direct targets and empty collection transforms fail clearly. No shared pivot is inferred.

Capture and comparison `solo_collections` unions memberships, then intersects with `solo_objects` if present. `hide_collections` unions with `hide_objects` and wins over solo filters. Empty solo lists render nothing. Filters are temporary and preserve framing. Picking, occlusion and comparison landmarks obey the same visibility. Replay retains the original request; capture visibility metadata reports resolved object IDs and counts.

In **Collection IDs** mode, an object's primary collection determines its colour; unassigned objects are neutral grey `[128,128,128]`. Capture replies include `collection_id_colors` (hex colour to collection ID), retaining entries for hidden collections. Membership updates assign a primary only to objects without one. `primary:true` explicitly assigns this collection to all members; `primary:false` clears its primary assignments. Removing a primary membership leaves that object unassigned until reassigned. Other memberships remain available for targeting.

The viewer Objects panel presents expandable collection branches with member objects underneath, plus Ungrouped for objects without membership. Objects in multiple collections appear in each branch with synchronized visibility; counts and bulk actions deduplicate object IDs. Search reveals matching branches and objects, then restores expansion choices when cleared. Expansion survives live updates within a workspace. Overlapping collections show partial visibility where appropriate. Controls use existing viewer-local object visibility; they do not edit membership, exports or CLI captures. Membership and colours update with live edits.

## Capture object visibility

`capture` and `compare_reference` accept optional `hide_objects:[ids]` and `solo_objects:[ids]`. IDs must exist; invalid IDs/types fail without consuming a view or changing the document. If both are supplied, start with the solo list and then hide the listed objects. `solo_objects:[]` produces an empty render; `hide_objects:[]` keeps everything visible. Duplicate IDs are deduplicated. Filters apply only to that command: no geometry, selection, exports, saved reference view or browser visibility is changed. Camera framing stays fixed for comparisons; adjust camera options explicitly to frame a solo object.

```json
{"command":"capture","mode":"shaded","solo_objects":[141]}
{"command":"capture","hide_objects":[67,68,69,70]}
{"command":"compare_reference","reference":"resource-1","layout":"overlay","solo_objects":[141]}
```

Replies include `visibility` with canonical filter lists and visible/total object counts. Replay retains the full request and result. The full scene's face/object palettes remain unchanged, including entries for hidden geometry, so matching IDs keep the same colors across filtered captures. Hidden geometry is excluded from surfaces, edges, labels, automatic depth ranges and occlusion. `select_image`, including `through:true`, can select only objects included in its source capture; hidden objects do not block selection of geometry behind them. Normal view revision/expiry guards still apply.

Filtered comparisons omit bound landmarks with any hidden vertices and omit entire contours crossing omitted landmarks, avoiding misleading connections. Reference-only guides remain. The returned alignment counts/residuals apply to this retained subset and report `omitted_hidden_landmarks`; `project_reference` still reports all saved bindings. Filtering does not establish accuracy or excuse a whole-model review.

## Capture cameras

Orthographic capture remains the default (`yaw`, `pitch`, `zoom`, `center`). For perspective, set `projection:"perspective"`, `position:[x,y,z]`, and `yaw`/`pitch` or `target:[x,y,z]`. `fov` is the vertical field of view in degrees (default 60, range 5–150); `near` defaults to 0.05 world units. Optional `lens_shift:[x,y]` shifts the image center by fractions of its width/height. Both cameras accept `width`/`height`. Perspective rendering clips triangles and edges at the near plane and interpolates depth/normals correctly; geometry is unchanged.

Saved reference views retain these options. An explicit yaw/pitch override clears a saved target unless a new target is supplied. Behind-camera landmarks are reported as unprojectable. `move_in_view` and `move_in_reference` convert pixel offsets at the selected centroid's depth (optional world-space `anchor` for `move_in_view`), preserving depth and translating the group rigidly. Points at different depths can therefore move different pixel distances.

Concept images and drawings may contain inconsistent perspective. A fitted camera and low residual are diagnostic aids, not accuracy requirements. Constrain plausible camera values, inspect independent regions and other views, and preserve coherent geometry where the source is ambiguous. A single image does not determine hidden depth.

```json
{"command":"capture","projection":"perspective","position":[6,1.8,2],"target":[6,3,-8],"fov":65,"near":0.05,"width":1200,"height":800}
```

## Validation render modes

The browser's mode menu and CLI `capture` provide complementary inspection views:

| Mode | Purpose | Encoding |
| --- | --- | --- |
| `shaded` (default) | Shape, ledges, proportion and reference comparison | Neutral-gray material and fixed white lights; light-gray capture background |
| `topology` | Connectivity and identifying vertices/faces | Gray shading with visible polygon edges; vertex and face IDs by default |
| `tessellation` | Inspect renderer triangulation | Gray shading with visible triangle edges, including internal diagonals; counts and nonplanar face sample |
| `face_ids` | Distinguishing and querying individual faces | Vivid unique flat colors, no lighting; exact `face_id_colors` map in capture metadata |
| `object_ids` | Identifying separate mesh objects | One vivid flat color per object; exact `object_id_colors` hex-color-to-object-ID map in capture metadata |
| `normals` | Surface directions and discontinuities | Interpolated world-space shading normals, RGB = round(255 × (N + 1) / 2), no lighting or gamma |
| `depth` | Relative depth and overlap | Linear camera-space z, far=32 and near=255; background=0 |
| `silhouette` | Outer shape and openings | Black geometry on white background |
| `uv_checker` | UV scale, continuity and distortion | Unlit 8×8 repeating checker; incomplete face mappings are magenta |
| `textured` | Direct image mapping and painted tint | Unlit nearest-sampled repeating texture multiplied by interpolated vertex RGB; unassigned texture is gray and missing UVs are magenta |
| `vertex_colors` | Vertex painting and interpolation | Unlit normalized RGB, interpolated across polygon fan triangles; unpainted vertices are white |
| `orientation` | Face orientation. Capture metadata includes depth-visible back-facing and total surface pixel counts, and their ratio (null for an empty frame). | Front-facing gray, back-facing red; double-sided rasterization |

Use multiple views together. Face and object colors are categorical IDs, not materials. Object IDs use the same palette order in the browser, inspector and CLI; hiding objects does not reassign browser colors. CLI example: `{"command":"capture","mode":"object_ids"}`. Use capture metadata for exact PNG identification; browser edges can be antialiased. Colors may change after objects are added or removed. CLI captures ignore browser visibility; use `hide_objects` or `solo_objects` explicitly to filter them. A color difference can mark a coplanar face boundary without a geometric step. Colors are unique per capture and the returned mapping is authoritative; topology changes can reassign colors. PNG face-ID pixels are unlit and un-antialiased for exact decoding. Browser edge pixels may be blended by GPU antialiasing. Both renderers use the same vivid palette sequence; exact identification should use the capture's mapping rather than visual color names.

Captures default to clean shaded images. `mode: "topology"` defaults to annotations; `clean: true` hides markers and labels while retaining its topology edges. `clean: false` can add edges and labels to shaded captures. Other modes require clean output so their diagnostic colors remain unambiguous. All modes support orthographic and perspective cameras and can be passed to `select_image` via their view ID. They do not mutate geometry or history.

Depth metadata reports `[far_z, near_z]` in world units after camera rotation; larger z is closer to the camera. It auto-ranges to the mesh vertices unless `depth_range: [far_z, near_z]` is specified. Use a fixed range and identical camera when comparing changes, since auto-ranging can conceal a global depth change. Planar meshes with a constant camera-space z render at mid-gray. Normal colors preserve face winding rather than flipping backfaces. Face colors and diagnostic values are encoded directly, without shading/gamma conversion.

The CLI uses a CPU triangle rasterizer with a per-pixel depth buffer. Default replies return PNG and SVG artifact paths; `response:"full"` returns `png_base64` plus the SVG image wrapper inline. SVG output is no longer a set of independently editable vector polygons. The browser uses WebGL with a perspective camera and an optional ground plane; its camera is independent. Captures and browser views share mode semantics and neutral lighting conventions, but are not pixel-identical renderers.

## Prototype limits

Image selections must reference a CLI capture's view ID. The browser uses studio-style directional lighting and an approximate contact shadow. Both renderers use a fan for convex faces and projected ear clipping for concave faces. Use simple planar faces for predictable tessellation. Edits reject invalid references, non-finite coordinates and zero-area faces, but do not detect self-intersections, folded quads, inconsistent winding or all non-manifold topology. Edge incidence diagnostics are reported separately. Extrusion accepts a displacement vector; choose one pointing outward. This prototype stores polygon adjacency rather than a half-edge structure.

Projects contain mesh and selection, not history or cameras. The server is single-document, serialized and local-only, with no authentication or MCP adapter. The CLI is the initial Codex interface.

## UV editing and vertex painting

This toolset exposes explicit values. An agent chooses coordinates, colors, seams and texture assignments, then queries and captures the result. No projection, unwrap, packing or painting algorithm is required. Start with `workflow_status` and inspect the existing IDs before editing. These commands use normal atomic edits, revision guards, undo/redo, save/load and replay.

| Command | Fields and behavior |
| --- | --- |
| `uv_set` | `values:[{vertex,uv:[u,v],face?},...]` (1–4096 entries), optional `object`, `faces`. Without a per-entry `face`, writes that vertex's corners on every incident face in scope; with `face`, writes only that corner. Each face/vertex pair may appear once per command. |
| `uv_clear` | Optional `object`, `faces`, `vertices`; removes mapped values in scope. Without `vertices`, clears all mapped corners in scope. |
| `uv_transform` | Same scope as clear; optional `offset:[u,v]`, `scale:[u,v]`, `center:[u,v]`, `rotation` in degrees. Scales about center, rotates counterclockwise, then offsets existing assignments. Missing values remain unmapped; negative/zero scales are permitted for experiments. |
| `uv_project` | Optional convenience mapping: `mode:"planar"`, `"box"`, or `"cylindrical"`; on a project-wide projection, `masters_only:true` skips direct writes to linked copies so they inherit their masters’ UVs; optional `object`, `faces`, `filter`, `bounds:[[min_x,min_y,min_z],[max_x,max_y,max_z]]`, `region:[u_min,v_min,u_max,v_max]` (default unit square). Fits bounds to the whole object when supplied, otherwise the unfiltered face scope; explicit bounds override fitting. Planar uses `axes:[0,1]`; box chooses XY, ZY or XZ by each face's dominant area normal. Cylindrical uses `axis` (0=X, 1=Y default, 2=Z), `phase` in turns, optional `fit_sections` and nonnegative `section_tolerance` in model units. Writes explicit corners in one undoable edit. |
| `query_uv` | Optional `object`, `faces`, `revision`, `offset`, `limit` (1–256, default 32). Exact face records in ID order, with each corner's vertex and UV (null if unmapped), completeness and texture name. `kind:"textures"` instead pages texture names/dimensions without pixel payloads. |
| `inspect_uv` | Optional `object`, `faces`, `offset`, `limit`. Counts incomplete, degenerate, mirrored and outside-0–1 faces; pages the issues. Mirroring, tiling and overlap can be intentional. Does not automatically classify overlap/distortion or repair anything. |
| `capture_uv` | Optional `object`, `faces`, `texture`, `size` (64–2048, default 512), `bounds:[min_u,min_v,max_u,max_v]` (default `[0,0,1,1]`). PNG/SVG layout with pink mapped edges over the named texture or checker. Missing endpoints leave gaps. Includes revision, inspection summary and a query template for exact corner IDs/values. |
| `texture_import` | `name`, `path` to a PNG inside the model workspace. Normalizes to embedded RGBA pixels; redefining a name updates every face using it. |
| `texture_define` | `name`, `width`, `height`, `rgba_base64`: direct pixel assignment for experiments. RGBA8 bytes, top-to-bottom rows, four bytes per pixel. |
| `texture_settings` | `texture:"name"`, optional `filter:"linear"|"nearest"`, `double_sided:true|false`. Updates the shared material used by every face assigned to that texture. Creation and import commands accept both settings. Query them with `query_uv {kind:"textures"}`. Its sidedness suggestion records open boundary edges on assigned faces. |
| `texture_assign` | `texture:"name"`, optional `object`, `faces`. Assigns an existing image to faces; `texture:null` clears bindings. UVs may be assigned before or after the image. |
| `texture_remove` | `texture:"name"`. Removes an unassigned image; clear face bindings first. |
| `texture_export` | `texture:"name"`. Read-only PNG output using the normal artifact response and CLI `--output image.png`. Preserves image alpha. |
| `vertex_color_set` | `values:[{vertex,color:[r,g,b]},...]` or `vertices:[ids]` plus one `color`; optional `object`. Assigns 1–4096 distinct vertices. RGB values are finite normalized 0–1 numbers, interpreted as display RGB. |
| `vertex_color_clear` | Optional `object`, `vertices`. Removes painted values; omitted vertices means every vertex in scope. |
| `vertex_color_gradient` | `start:[x,y,z]`, `end:[x,y,z]`, `from:[r,g,b]`, `to:[r,g,b]`. Projects vertex positions onto the start/end line and clamps the interpolation to 0–1. Optional `object`, `faces`, `filter`, `vertices`, `blend:"replace"` (default) or `"multiply"`. Without vertices, paints every face corner vertex in scope. |
| `vertex_color_shade` | Same scope/blend as gradient. `direction:[x,y,z]`, optional `color:[r,g,b]` (white), `ambient` (0.5), `strength` (0.5), `absolute_axes:[0,1,2]` (empty by default). Uses normalized, area-weighted vertex normals from all incident faces; optional axes are made absolute before the dot product. Direction is deliberately not normalized, allowing explicit weights. Factor is `clamp(ambient + strength * clamp(dot,0,1),0,1)`; color is multiplied by factor. This writes paint, rather than adding live lighting. |
| `query_vertex_colors` | Optional `object`, `ids`, `revision`, `offset`, `limit` (1–256, default 32). Exact vertex IDs and RGB, or null for unpainted vertices. |

UVs are stored by stable face ID and vertex ID (`mesh.uvs[face][vertex]`). A single geometric vertex can therefore have distinct UVs on adjacent faces, providing an explicit seam without splitting geometry. Coordinates may tile outside 0–1; they must remain finite within ±1,000,000. Editing UVs never changes vertex positions. UV commands with no face/object scope apply across the model, independently of the geometric vertex selection.

Texture names and face bindings live in `mesh.textures` and `mesh.face_textures`. Images are limited to 1024×1024 each and 4,194,304 pixels total per project; PNG input is limited to 8 MiB and animation is rejected. Normalized RGBA data is embedded in native projects and replay snapshots, so a later edit/deletion of the source PNG does not alter the registered texture. Reimport explicitly to update it. The HTTP request limit is 64 MiB. Layer sources and baked maps together have a 32 MiB decoded image budget. Pixel data increases project/replay size and live-state transfer size; use small textures when experimenting.

UVs repeat with the origin at the bottom left. Base-color inspection samples individual texels. Material previews and GLB exports use the texture filter. New version 9 projects use linear filtering with mipmaps. Existing projects retain nearest filtering until explicitly changed. Materials default to double-sided. Textures without an alpha cutoff use their authored RGB in surface previews. Set `alpha_cutoff:0.5` on `texture_import` or `texture_define` to discard texels below the threshold in `textured` and `material` views. The threshold accepts 0 through 1. Texels at or above it use their RGB as opaque color. Use **Base color** in the viewer for unlit sprites, and use the axis gizmo and projection button below the render toolbar for an axis-aligned orthographic view. Painted values live in `mesh.vertex_colors[vertex]`. Unpainted vertices contribute white when interpolating or tinting a texture. The `vertex_colors` and `textured` modes show unlit display RGB. The `shaded` mode retains neutral lighting. Captures and the browser offer `uv_checker`, `textured` and `vertex_colors`. These three modes require clean captures. Textured mode multiplies sampled RGB by interpolated vertex color.

After creating a cube with `{"command":"cube"}`, assign four explicit corners and tint two vertices:

```json
{"command":"uv_set","faces":[2],"values":[{"vertex":5,"uv":[0,0]},{"vertex":6,"uv":[1,0]},{"vertex":7,"uv":[1,1]},{"vertex":8,"uv":[0,1]}]}
{"command":"texture_import","name":"body","path":"resources/body.png"}
{"command":"texture_assign","faces":[2],"texture":"body"}
{"command":"vertex_color_set","vertices":[5,6],"color":[1,0.5,0.25]}
{"command":"query_uv","faces":[2]}
{"command":"query_vertex_colors","ids":[5,6]}
{"command":"capture","mode":"textured","yaw":0,"pitch":0}
{"command":"capture","mode":"vertex_colors","yaw":0,"pitch":0}
{"command":"capture_uv","faces":[2],"texture":"body"}
```

Use actual IDs on an existing mesh. Supply your own `resources/body.png` inside the workspace. Save captures with CLI `--output preview.png`, save the layout with `--output layout.png` or `layout.svg`, and open them before recording assessments. `capture_uv` is a UV-space diagnostic, not a 3D camera capture for the workflow's distinct-view requirement. Read-only queries/captures preserve the model revision.

Moving geometry preserves bound values. Topology edits retain values for surviving face/vertex bindings and remove deleted bindings; newly created corners/vertices remain unmapped/unpainted until explicitly assigned. Incomplete faces show magenta in UV/texture previews. Duplication copies appearance, and linked instances inherit UVs, texture assignments and paint from their master; edit the master or make the instance unique. Linking existing objects requires matching appearance as well as geometry. Appearance edits invalidate affected capture evidence and dependent workflow assessments, including after restart.

Basic UV/texture/vertex-paint data requires at least version 3; layers/maps use version 4, extended shared masks use version 5, and authored components/construction metadata uses version 6. Legacy versions still load, and models without newer data retain their earlier format level. Use the updated tool for version 3 projects. FBX exports UVs and vertex RGB by polygon corner. If any face of an object has UVs, FBX requires that object's entire mapping to be complete rather than inventing coordinates. OBJ emits explicit `vt` indices and supports partially mapped faces; painted `v` records append the common RGB extension, whose reader support varies. GLB embeds supported images. For OBJ and FBX, use `texture_export` to deliver the maps separately. See *Editable solids and subdivision* for automatic UV unwrapping and packing.

### Native appearance recipes

`filter` is available on face-based UV queries/edits, texture assignment, and the new gradient/shade commands. Its fields intersect: `bounds:[[min_x,min_y,min_z],[max_x,max_y,max_z]]` tests the face centroid, `flat:true/false` tests the authored flat-face flag, and `normal:{direction:[x,y,z],range:[min,max],absolute:false}` tests the normalized face normal against a normalized direction. Ranges are inclusive dot products; `absolute:true` includes both sides. Empty matches fail clearly. Query with `query_uv` first to inspect exact IDs, corners, area-weighted `area_normal` and `flat` before editing. Painting affects shared geometric vertices, including their other incident faces.

Cylindrical projection measures `atan2(second_radial_axis,first_radial_axis)` using cyclic radial axes (Y cylinder: Z then X). `phase` offsets this angle in turns. Adjacent seam faces get independent corner values; seam-crossing faces unwrap by subtracting one turn from high U values. This can put coordinates slightly outside the atlas region; inspect the layout when moving a seam through a face. The cylinder fits an ellipse to the scope bounds. With `fit_sections:true`, vertices sorted by axial position are grouped within `section_tolerance` of each group's first vertex, and each group fits its own radial bounds. Default tolerance is zero. Use this for sampled rings, including gently tilted rings with an appropriate tolerance; arbitrary meshes may need different mapping or direct UV assignments. Map caps separately with planar/box projection. These are optional coordinate operations, not an unwrap/packing pipeline; overlap and mirrored faces can be intentional.

Save a JSON array of native commands and run it with `atk-mesh-cli --port 17891 --batch --input recipe.json`. See [batch commands](#batch-commands) for reply and failure behavior. Texture import paths resolve from the server workspace.

For example, on an inspected starter cube:

```json
[
  {"command":"workflow_status"},
  {"command":"uv_project","object":1,"mode":"box","region":[0.012,0.512,0.488,0.988]},
  {"command":"vertex_color_gradient","object":1,"start":[0,-1,0],"end":[0,1,0],"from":[0.3,0.2,0.15],"to":[1,0.9,0.8]},
  {"command":"vertex_color_shade","object":1,"direction":[-0.25,0.45,0.85],"absolute_axes":[2],"ambient":0.66,"strength":0.3,"blend":"multiply"},
  {"command":"inspect_uv"},
  {"command":"capture","mode":"uv_checker","yaw":30,"pitch":15}
]
```


## Layered textures and material previews

Texture resources can retain editable layer stacks and baked base-color, tangent normal, roughness, metallic, specular and height images. Faces continue to bind a texture name; UVs and vertex colors remain directly editable. This is a native CLI painting workflow with optional masks, not a required unwrap or material-generation algorithm.

| Command | Fields and behavior |
| --- | --- |
| `texture_create` | `texture`, `width`, `height` (1–1024), `color:[r,g,b,a]` in 0–1. Creates a solid image; rejects an existing name. |
| `texture_layer_add` | `texture`, `layer:{name,channel?,color?,opacity?,blend?,visible?,mask?}`. Channel defaults to `base_color`; color to white RGBA; opacity to 1; blend to `normal`; visible to true. Optional `source_texture` snapshots another registered image into the layer. Images resample over the whole destination canvas using nearest sampling. The snapshot remains editable with the stack and is independent of later source edits/removal. |
| `texture_layer_set` | `texture`, `layer:"name"`, `changes:{color?,opacity?,blend?,visible?,mask?}`. Mask changes merge named fields into the existing mask, so adjusting noise preserves strokes/polygons. Supported blends are `normal`, `multiply`, `add` and `screen`. |
| `texture_layer_paint` | `texture`, `layer`, `strokes:[{points:[[u,v],...],radius,hardness?,opacity?,value?},...]`. Appends continuous polyline strokes to the layer mask. Radius is 0.000001–2 UV units; hardness/opacity/value are 0–1 (defaults 0/1/1). Value 0 erases; value 1 reveals. A single point makes a circular dab. |
| `texture_layer_mask_faces` | `texture`, `layer`, optional `object`, `faces`, `filter`; `mode:"faces"` (default) or `"boundary"`. Replaces the mask with filled UV polygons or strokes along edges incident to exactly one face in the requested set. Boundary mode requires UV `radius`, optional `hardness` (default 0.5). Requires complete UVs and snapshots their current positions. Internal mesh edges do not become wear automatically. |
| `texture_layer_move` | `texture`, `layer`, zero-based destination `index`. Layers composite from index 0 upward, independently per channel. |
| `texture_layer_remove` | `texture`, `layer`. Removes the layer and recomposites from the stored base. |
| `query_texture_layers` | `texture`, optional `revision`, `offset`, `limit` (1–256, default 32). Exact layer metadata and masks, image dimensions without pixel payloads, channel names and height-to-normal settings. |
| `texture_layer_capture` | `texture`, `layer`. Read-only grayscale mask PNG using the usual artifact reply and CLI `--output mask.png`. |
| `texture_map_set` | `texture`, `channel`, `source_texture`. Snapshots the source image as this channel's base beneath layers. Dimensions must match. Use `texture_import` to register a workspace PNG first; remove an unassigned temporary source afterward if desired. |
| `texture_normal_from_height` | `texture`, `strength` (0–64, or `null` to disable). Derives a tangent-space normal map after compositing height. This overrides the normal-channel layers while enabled. Central differences use neighboring texels with clamped atlas edges; strength controls the slope per texel. |
| `texture_bake` | `texture`, optional `flatten:false`. Recomputes images, preserving the editable stack by default. `flatten:true` discards the stack after baking, with normal undo available. Every layer edit also bakes automatically. |
| `texture_export` | `texture`, optional `channel` (default `base_color`). Saves that channel as a standalone RGBA PNG; use CLI `--output atlas-normal.png`, etc. Missing channels export their documented neutral defaults. |

Channels are `base_color`, `normal`, `roughness`, `metallic`, `specular` and `height`. Base color uses display RGB; numeric maps use linear values. Roughness, metallic and height use the red channel; specular uses RGB reflectance (default 0.04). Default roughness is 0.65, metallic/height zero, normal approximately `[0.5,0.5,1]`. Normal maps use tangent space, positive green along increasing V (+Y convention). Normal vectors are normalized during shading. Normal map green is independent of the PNG's top-to-bottom row order. Scalar/normal alpha is ignored by the renderer.

A layer mask accepts `fill` (default 1), `polygons` (UV point arrays, filled using even/odd crossings), `strokes`, and optional `noise:{scale,seed,amount,threshold}`. Polygons reveal the mask; strokes then blend toward their value in order; deterministic three-octave value noise multiplies the result last. Scale is 1–4096 cells per UV unit, seed is a u32, amount is 0–1. Threshold is 0–1 or null for continuous noise. UV origin is bottom-left. Masks are clipped to the 0–1 canvas and do not wrap. Faces sharing UVs share paint, so allocate separate atlas space where independent detail is needed. Mesh/UV edits do not move previously sampled masks; reissue the mask command to resample geometry.

For example, after inspecting face IDs and assigning UVs:

```json
[
  {"command":"workflow_status"},
  {"command":"texture_create","texture":"head","width":512,"height":512,"color":[0.25,0.18,0.26,1]},
  {"command":"texture_layer_add","texture":"head","layer":{"name":"edge-wear","color":[0.75,0.65,0.72,1],"mask":{"fill":0}}},
  {"command":"texture_layer_mask_faces","texture":"head","layer":"edge-wear","faces":[2],"mode":"boundary","radius":0.015},
  {"command":"texture_layer_set","texture":"head","layer":"edge-wear","changes":{"mask":{"noise":{"scale":80,"seed":7,"amount":1,"threshold":0.45}}}},
  {"command":"texture_layer_add","texture":"head","layer":{"name":"rough-metal","channel":"roughness","color":[0.7,0.7,0.7,1]}},
  {"command":"texture_assign","faces":[2],"texture":"head"},
  {"command":"texture_layer_capture","texture":"head","layer":"edge-wear"},
  {"command":"capture","mode":"material","yaw":20,"pitch":10},
  {"command":"texture_export","texture":"head","channel":"base_color"}
]
```

Supply actual mapped face IDs. Run an array with CLI `--batch --input FILE`. Inspect mask, atlas and mesh captures; command success is not evidence of a reference match. Existing workflow checkpoints still apply.

`capture` mode `material` and the viewer's **Lit material + detail maps** use matching studio light directions, gamma-2.2 base-color conversion, linear data maps, UV-derived tangents with mirrored handedness, roughness-controlled Blinn–Phong highlights, metallic/specular reflectance and tone mapping. This is an approximate material preview, not a full physically based engine renderer. The prior `textured` mode remains unlit for base-color/vertex-paint inspection. Degenerate UV triangles ignore normal perturbation; missing UVs remain magenta. The preview uses a procedural reflection with a brighter overhead region.

Texture definitions own stacks and baked images; both renderers consume the baked images. Editing a shared texture updates every assigned face, including linked instances. Define a separate texture name for independent materials. Ordinary commands participate in revision guards, atomic rollback, undo/redo, save/load, workspace recording and appearance-evidence invalidation. `texture_import`/`texture_define` replace the resource, including its stack. Models containing stacks or additional maps save as version 4; versions 1–3 remain readable. Older executables cannot open version 4.

Limits are 64 layers per texture, 4096 mask shapes and 16384 mask points per layer, 1024×1024 per image, 4,194,304 canvas pixels across textures, and 32 MiB decoded images across baked maps, stored bases and image layer sources. These are prototype limits; embedded maps and recording snapshots can grow workspace size substantially. Layer blends operate in stored normalized channel values, use straight alpha, and quantize to RGBA8 after each layer. Height and roughness details are texture detail and do not change the mesh silhouette.

In Textured and Textured lit previews, textures without an alpha cutoff use their RGB as opaque color. An explicit alpha cutoff still discards masked texels. Textured lit and CLI material captures use the same procedural overhead reflection and fixed highlight rolloff.

### Shared masks and material layers

A material layer writes several channels through one mask. Use `channels:{base_color:[r,g,b,a],roughness:[v,v,v,a],metallic:[v,v,v,a],height:[v,v,v,a],...}` on `texture_layer_add`; supply only the channels to change. Omit legacy `channel`/`color` when providing `channels`. Shared opacity, visibility, blend mode, optional image multiplier and mask apply to all outputs. `texture_layer_set` accepts a nonempty replacement `channels` map. The existing single-channel layer commands remain valid.

Named masks belong to the texture's paint stack. A layer or another named mask can reference one with `mask:{source:"damage"}`. This is a live reference, not a copied raster: edits recompute every dependent layer/channel and the derived normal map in the same undoable command. References stay within the texture. Unknown sources and cycles are rejected atomically. Removing a referenced mask fails until its references are changed or removed, including references from hidden layers.

| Command | Fields and behavior |
| --- | --- |
| `texture_mask_set` | `texture`, `name`, `mask:{...}`. Creates or replaces a named definition. Mask fields include existing fill/polygons/strokes/noise, plus optional `source`, `filters`, `marks` and `transform`. |
| `texture_mask_transform` | `texture`, `name`, optional `offset:[u,v]`, `scale:[u,v]`, `center:[u,v]` (default `[0.5,0.5]`), `rotation` in counterclockwise degrees. Sets an absolute transform; omitted values reset to identity. Scale about center, then rotate, then offset. Negative scale mirrors; zero scale is rejected. Use identity parameters to reset. |
| `texture_mask_paint` | `texture`, `name`, `strokes:[...]`. Appends mask strokes using the existing UV radius/hardness/opacity/value schema. Value 0 erases and value 1 reveals. |
| `texture_mask_stamp` | `texture`, `name`, `source_texture`, `center:[u,v]`, `size:[u,v]`, optional `rotation`, `opacity` (1), `value` (1). Snapshots the registered image's baked base color as a brush alpha. Red multiplied by alpha determines coverage. Size is the full stamp dimensions in UV units; rotated sampling is bilinear. A source PNG can be registered with `texture_import`. Later source edits/removal do not change the stamp. |
| `texture_mask_remove` | `texture`, `name`. Deletes an unreferenced named mask. |
| `query_texture_masks` | `texture`, optional `revision`, `offset`, `limit`. Exact paged definitions in name order, including references, filters and transform. Embedded stamp images expose dimensions, not pixel payloads; the complete project retains the source pixels. |
| `texture_mask_capture` | `texture`, `name`. Read-only grayscale PNG of the resolved definition. Layer-mask captures also resolve shared references and filters. |

`filters` is an ordered list: `{op:"invert"}`, `{op:"levels",low:0,high:1,gamma:1}`, `{op:"blur",radius:0.004}`, `{op:"dilate",radius:0.004}`, `{op:"erode",radius:0.004}`, or `{op:"outline",radius:0.004,side:"inner"|"outer"|"both"}`. Levels remap/clamp low–high, then apply exponent `1/gamma`. Blur is a separable box filter; dilation/erosion use square max/min neighborhoods. Outer outline is expanded mask minus original; inner outline is original minus contracted mask; both is expanded minus contracted. Radius is in units of the shorter canvas dimension, rounded to whole texels, limited to 0–0.0625 (at most 64 texels at 1024px). Outside-canvas samples are zero. Filters clip to 0–1 and do not wrap.

Evaluation order is local legacy polygons/strokes/noise, then ordered `marks`, intersection with `source` when present, transform, then filters. Paint/stamp commands append ordered marks so an erase stroke after a stamp removes its coverage. Serialized marks have `op:"stroke"` plus stroke fields, or `op:"stamp"` plus stamp fields and an embedded `image`. A referenced mask with the default local fill 1 passes the source through unchanged. For a stripe clipped to intact paint, use local stripe polygons with fill 0 and `source:"intact-paint"`.

For chipped paint, keep a complete metal material beneath a paint material. Define damage once; invert it for intact paint and derive an outer outline for a raised rim:

```json
[
  {"command":"texture_mask_set","texture":"panel","name":"damage","mask":{"fill":0,"polygons":[[[0.2,0.4],[0.3,0.42],[0.28,0.5],[0.22,0.48]]]}},
  {"command":"texture_mask_set","texture":"panel","name":"intact-paint","mask":{"source":"damage","filters":[{"op":"invert"}]}},
  {"command":"texture_mask_set","texture":"panel","name":"paint-rim","mask":{"source":"damage","filters":[{"op":"outline","side":"outer","radius":0.006},{"op":"blur","radius":0.002}]}},
  {"command":"texture_layer_add","texture":"panel","layer":{"name":"metal","channels":{"base_color":[0.4,0.43,0.46,1],"roughness":[0.3,0.3,0.3,1],"metallic":[1,1,1,1],"height":[0.5,0.5,0.5,1]}}},
  {"command":"texture_layer_add","texture":"panel","layer":{"name":"paint","channels":{"base_color":[0.08,0.43,0.49,1],"roughness":[0.75,0.75,0.75,1],"metallic":[0,0,0,1],"height":[0.52,0.52,0.52,1]},"mask":{"source":"intact-paint"}}},
  {"command":"texture_layer_add","texture":"panel","layer":{"name":"lifted-rim","channel":"height","color":[0.56,0.56,0.56,1],"mask":{"source":"paint-rim"}}},
  {"command":"texture_normal_from_height","texture":"panel","strength":3},
  {"command":"texture_mask_transform","texture":"panel","name":"damage","offset":[0.1,0]}
]
```

Create the named texture and assign UVs before this example. Moving `damage` moves the exposed metal, coating cutout and raised rim together.

### Selecting and painting the surface

| Command | Fields and behavior |
| --- | --- |
| `query_texture_faces` | Preview the face scope with optional `object`, `faces` or `region`, `filter`, `offset` and `limit`. Add `seed_face` and `expand:"connected"` to follow shared edges, or `expand:"uv_island"` to follow only edges with matching UVs at both corners. Expansion stays within the supplied scope and requires the seed to be in that scope. |
| `texture_layer_mask_faces` | Accepts the same scope and expansion fields. Use `mode:"faces"` to fill the result or `mode:"boundary"` to outline its geometric boundary. |
| `texture_mask_surface` | Set a named mask with `texture`, `name`, a complete `shape`, and the same optional scope and expansion fields. Reissuing the command without a scope keeps the existing position/UV snapshot. Supply a scope to resample after geometry or UV edits. Existing filters, marks, and layer references are retained. |

Surface parameters use mesh coordinates and units. The selected faces require complete, nondegenerate UVs inside the unit square. If selected faces overlap in UV space, the maximum coverage at each texel wins.

- **Band** uses `shape:{kind:"band",center:[x,y,z],axis:[x,y,z],width,feather?}`. Width is the full solid width. Feather extends outward on both sides in mesh units and defaults to zero.
- **Gradient** uses `shape:{kind:"gradient",start:[x,y,z],end:[x,y,z]}`. Coverage ramps linearly from zero to one along the line and clamps beyond the endpoints.
- **Decal** uses `shape:{kind:"decal",center:[x,y,z],right:[x,y,z],up:[x,y,z],size:[width,height],depth}`. The first call also needs `source_texture`, which snapshots the source image's red channel times alpha as coverage. Color comes from the referencing layer. The projector affects faces pointing toward `right × up` within half the depth on either side of the center plane. Use face scope and depth to control which surfaces receive it. Later edits can reuse the snapshotted image.

The surface mask multiplies the local fill, polygon, stroke, and noise result before ordered marks, shared mask references, transforms, and filters. Native projects with surface masks save as version 8.

Stacks using named masks, material-channel bundles, mask filters, transforms or stamps save as project version 5. Older projects still save at their previous format level when these features are absent. There are at most 32 named masks per texture, 16 filters and 256 ordered marks per mask; existing shape/point and image budgets also apply. Masks resolve as floating-point coverage, while baked output channels remain RGBA8. Height and thin rims still need appropriate texel density. Native recordings and workflow appearance checks include the definitions as well as baked results.

## Linked mesh instances

Use `object_instance` for repeated parts that should share future shape edits. `object_duplicate` still creates an independent copy. Any independent object can be a master; no registration step is required.

Read `instancing_review` in `workflow_status` at the start of blockout. Identify repeated shapes within each assembly before building it. For an office, build one workstation master and reuse the typical-floor placement offsets at each storey elevation. Batch direct instances from the master, and use separate masters for geometry variants. Verify placements in fixed review views and preview a representative master edit. Object queries include each master's linked instance IDs and names in `instances`.

```json
{"command":"object_instance","object":1,"name":"Support 02","value":[0,0,-4]}
{"command":"object_rotate","object":2,"value":[0,90,0]}
{"command":"query_mesh","kind":"objects","ids":[1,2]}
{"command":"query_instance","object":2,"kind":"vertices","ids":[9]}
{"command":"object_make_unique","object":2}
```

Use IDs returned by your model. Object queries report `master_object` and `instance_count`; the viewer labels each relationship. `query_instance` maps picked instance vertices/faces back to master IDs without dumping a complete mesh. After detaching, edit the object normally or create a new family of instances from it.

For evenly spaced copies, use `{"command":"object_instance","object":1,"name":"Vent","array":{"count":4,"step":[0.13,0,0]}}`. This creates four new linked instances named `Vent 1` through `Vent 4`, at one through four steps from the master. The step is a nonzero world-space vector. The array accepts 1–256 copies and at most 150,000 new vertices. The operation returns their IDs in placement order and undoes as a single edit.

Master vertex, topology and shading edits propagate in the same undoable transaction. Surviving source IDs retain their instance mappings; new topology receives new IDs, and deleted topology is removed from every linked copy. Each instance has its own name, pivot, collections and placement. Use `object_move`, `object_rotate` and `object_set_origin` for placement; these commands also work on a master without moving its followers. Collection transforms apply once to each member. Selection-based shape edits on instances are rejected with instructions to edit the master or make unique. Instance-specific scaling and nested instance chains are not supported yet. Create additional copies from the reported master.

A direct `object_move` on a linked master reports `operation.linked_instances_unchanged` and a `shared_shape_edit` recipe. If you meant to change the shared shape, restore the master's intended placement first, then use the supplied selection and vertex move to update its copies.

Deleting an instance leaves its master intact. A master with linked followers cannot be deleted until its instances are detached or deleted. Links survive save/load, undo/redo and workspace replay. Existing projects load without conversion. Copies inherit source collection memberships initially, and can then be organized independently.

The master owns editable shape data. World-space instance geometry is currently materialized for stable picking IDs and the existing renderer/export pipeline, so this feature reduces modeling commands, not mesh storage or GPU draw calls. OBJ and FBX export each copy as a separate mesh object with its own pivot and resolved geometry; they do not preserve the internal link as a shared engine mesh resource.

### Export-only consolidation

For GLB, FBX, and OBJ exports use `merge:"all"` for a single mesh or `merge:"collection"` to combine members by primary collection. With no primary collection, the lowest-numbered membership wins. Objects without a collection export separately. Each group uses its lowest-numbered member's pivot. The project retains all source objects, placements, and links. Each merged group records the source names, IDs and world pivots in `sourcePivots` metadata (GLB node extras, FBX model property, or OBJ comment). OBJ vertices are world-space and OBJ readers may discard comments.

Merged FBX writes source corner normals, UVs, and named material slots. On a merged mesh with a UV layer, corners from wholly unmapped source objects receive (0,0). Merged OBJ writes explicit corner normals and `usemtl` boundaries. FBX and OBJ users can bind the separately exported images to those slots.

### Linking existing scene objects

`object_link_instance` takes target `object`, independent `master`, optional Euler `rotation:[x,y,z]` in degrees, and optional `vertex_map` mapping every master vertex ID to a target vertex ID. Translation is inferred from mesh centroids, independently of the object pivots. It preserves object names, IDs, pivots and collection memberships. Vertex/face IDs survive the conversion.

The command requires equivalent positions within 1e-7 world units, identical topology and face winding, and matching smoothing/flat-face overrides. It rejects geometry differences instead of reshaping a target to fit. Coincident vertices require an explicit bijective vertex map; topology is still checked. Existing masters with followers cannot be converted into nested instances. This is useful for migrating repeated independent objects into linked families without deleting and recreating them.
