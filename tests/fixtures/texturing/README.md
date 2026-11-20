# Texturing test meshes

Each directory contains a native mesh workspace with authored UVs and blank appearance. Copy the chosen directory to a scratch location before starting an editor. The recipe writes texture resources into the workspace, so use a fresh copy for each run.

| Mesh | What to check |
| --- | --- |
| [Panel](panel/project.json) | One quad with full-square UVs. Check stroke location, hardness, erase behavior and the height-derived normal along the stroke. A mask from face 1 covers the entire atlas. |
| [Atlas cube](atlas-cube/project.json) | Six hard-corner faces with separate, padded UV islands. A mask from face 1 covers only the lower-left island. Assign different colors by face to check island orientation and selection. |
| [Seam cylinder](seam-cylinder/project.json) | A 16-sided wall and two triangulated caps. Faces 1–16 form one side strip, and faces 17–32 and 33–48 form the two cap islands. The wall closes at the shared edge of faces 1 and 16, whose UVs are at opposite ends of the strip. Test seam behavior by placing paired strokes at U = 0.0625 and U = 0.9375 at the same V inside the strip. |

From the repository root, start an editor for a scratch copy in one PowerShell terminal.

```powershell
$fixture = Join-Path $env:TEMP ("mesh-texturing-" + [guid]::NewGuid())
Copy-Item tests/fixtures/texturing/panel $fixture -Recurse
cargo run --bin mesh-server -- --workspace $fixture --port 17891
```

Run these commands in a second terminal from the repository root. Use a different free port if needed.

```powershell
$cli = "./target/debug/mesh-cli.exe"
cargo build --bin mesh-cli
'{"command":"inspect_uv"}' | & $cli --port 17891
& $cli --port 17891 --batch --input tests/fixtures/texturing/smoke-recipe.json
'{"command":"texture_layer_capture","texture":"test-atlas","layer":"face-mask"}' | & $cli --port 17891 --output "$env:TEMP/mesh-face-mask.png"
'{"command":"texture_export","texture":"test-atlas","channel":"normal"}' | & $cli --port 17891 --output "$env:TEMP/mesh-normal.png"
'{"command":"capture","mode":"material","yaw":30,"pitch":20}' | & $cli --port 17891 --output "$env:TEMP/mesh-material.png"
```

All three meshes should report zero missing, degenerate, mirrored and out-of-bounds UV faces. The recipe adds a soft-edged orange stroke with matching height detail and applies a separate roughness mask from face 1. The normal image should be neutral away from the stroke. On the cylinder, the stroke crosses several wall quads and the roughness mask covers one quad. Orbit the cylinder in the viewer to inspect the full stroke.

## Edit a stripe on the cylinder

Start with a fresh copy of `seam-cylinder` and send these commands using `mesh-cli --batch --input FILE`.

```json
[
  {"command":"query_texture_faces","expand":"uv_island","seed_face":1},
  {"command":"texture_create","texture":"test-atlas","width":128,"height":128,"color":[0.12,0.18,0.25,1]},
  {"command":"texture_assign","texture":"test-atlas"},
  {"command":"texture_mask_surface","texture":"test-atlas","name":"stripe","expand":"uv_island","seed_face":1,"shape":{"kind":"band","center":[0,0,0],"axis":[0,1,0],"width":0.25,"feather":0.05}},
  {"command":"texture_layer_add","texture":"test-atlas","layer":{"name":"stripe","color":[1,0.75,0.3,1],"mask":{"source":"stripe"}}}
]
```

The face query should return the 16 wall faces. Capture the stripe with `texture_mask_capture` using `texture:"test-atlas"` and `name:"stripe"`. It should reach both ends of the side strip and leave both caps clear. Send the band command again with `width:0.5` and omit both `expand` and `seed_face` to widen the stripe on the same surface.
