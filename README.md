# ATK Mesh

Build, edit, and inspect 3D meshes with an AI agent. Review saved models in a local browser viewer.

**Status:** Experimental and under active development. The command API and workspace format may change.

## What you can do

- Model with geometry edits, solid booleans, connected rim bevels, and creased subdivision.
- Inspect topology, measure geometry, review rendered captures, and preview materials and animation.
- Import FBX or GLB, unwrap and pack UVs, and export OBJ, FBX, GLB, or mesh-only STL.

## Install

Install a stable Rust toolchain. From the atk-mesh source checkout, run:

```sh
cargo install --path . --locked
```

Cargo installs `atk-mesh-cli` for your agent and `atk-mesh` for the browser viewer. Both go in Cargo's bin directory, usually `~/.cargo/bin`.

## Use with an agent

Add the [`atk-mesh` skill](.agents/skills/atk-mesh/SKILL.md) using your agent's skill installation instructions. Open the agent in your project directory and give it a task.

> Create a simple low-poly crate in a fresh workspace, capture a preview, and export it as GLB.

## Open the viewer

Run `atk-mesh` from your project directory and open the viewer URL printed at startup. Once your agent has saved a workspace, choose it with the folder button. From there, orbit the model, switch between shaded and wireframe views, or preview an animation.

## Learn more

- [Command reference](docs/command-reference.md) for modeling, import, export, and debug recordings.
- [Modeling workflow](docs/workflow.md) for inspection, review, and exports.
- [Image-based reconstruction](docs/reconstruction.md) for reference guides and visual hulls.
- [Project integration](docs/project-integration.md) for workspace layout and editor setup.

## Related tools

- [ATK Audio](https://github.com/NathanChambers/atk-audio)
- [ATK UI](https://github.com/NathanChambers/atk-ui)

## License

[MIT](LICENSE)
