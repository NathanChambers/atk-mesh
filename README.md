# Mesh Workbench

Mesh Workbench is a local 3D modeling CLI designed for AI agents. An agent sends JSON commands through `mesh-cli` to build, edit, and inspect meshes, then uses the returned measurements and rendered previews to guide its next edit. You can follow the work and replay commands in the browser viewer.

**Project status:** Experimental and under active development. The command API and workspace format may change.

- **Model and inspect** meshes with geometry editing, topology views, measurements, and reference overlays.
- **Preview materials and animation** with textures, vertex colors, clip playback, and rig overlays.
- **Bring in existing geometry** from FBX or GLB. Save and review your work in native workspaces with command replay, and export OBJ, FBX, GLB, or mesh-only STL.

## Install

With a stable Rust toolchain, run these commands from a source checkout.

```sh
cargo install --path . --locked
mesh-server --version
mesh-cli --help
```

Cargo installs both binaries into its bin directory, usually `~/.cargo/bin`. The browser viewer is bundled with `mesh-server`.

## Quickstart

From your project directory, open a new editor in one terminal. Use a fresh workspace name if `.mesh/quickstart` already exists.

```sh
mesh-server --workspace .mesh/quickstart --port 7879
```

In a second terminal, create a box and save a preview image. This example uses PowerShell.

```powershell
'{"command":"object_create_box","name":"Box","min":[-0.5,0,-0.5],"max":[0.5,1,0.5]}' | mesh-cli --port 7879
'{"command":"capture","yaw":35,"pitch":25}' | mesh-cli --port 7879 --output preview.png
```

Start the project viewer from the same project directory in another terminal.

```sh
mesh-server --port 7878
```

To view a different directory, pass it as a path, for example `mesh-server "D:/Game Assets"`. You can also use `--project ROOT`.

Open [the local viewer](http://127.0.0.1:7878), click the folder icon, and select `.mesh/quickstart`. The editor on port 7879 saves changes to the workspace. The viewer on port 7878 displays the saved model and its replay history.

## Import an existing mesh

To import a static FBX, place it inside the editor's workspace and send `{"command":"import_fbx","path":"cargo.fbx"}` to the editor. Use `import_glb` for static GLB geometry. [Import details](docs/command-reference.md)

## Built-in help

With the editor running, use the CLI to find help by task or open a specific topic. Add `"all":true` to the help request for the complete topic list.

```powershell
'{"command":"help"}' | mesh-cli --port 7879
'{"command":"help","topic":"workflow"}' | mesh-cli --port 7879
'{"command":"help","topic":"command-api"}' | mesh-cli --port 7879
```
