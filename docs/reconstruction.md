# Image reconstruction

Run each command with `atk-mesh-cli --input request.json --output FILE`. These commands run locally without an editor. They use the same camera fields as `capture`. Match the camera's width and height to the source image and keep all cameras in the same world coordinate system.

Relative input paths resolve against the process working directory, even when `--input` points to a JSON request in another directory. This includes image and mask paths, depth maps, and the saved project path for `fit_depth`. For stdio MCP, the relevant directory is the one the host uses to start `atk-mesh-cli`. Use absolute input and output paths when the working directory can vary.

## Open a depth reconstruction in the editor

This PowerShell example assumes `depth.png` and `mask.png` are aligned 64 × 64 grayscale PNGs. White mask pixels keep geometry. Adjust the camera and distance range to match how you captured your depth map. Run the commands from your consuming project's root, with the input images stored in a separate directory. Replace the two example paths in the first block.

```powershell
$meshBin = 'C:\work\agentic-toolkit\atk-mesh\target\release'
$referenceDir = 'C:\work\reference-frames'
$output = Join-Path (Get-Location).Path 'reconstruction.json'
$requestFile = Join-Path (Get-Location).Path 'reconstruction-request.json'
$request = @{
    command = 'depth_mesh'
    view = @{
        camera = @{ width = 64; height = 64; yaw = 0; pitch = 0; zoom = 200 }
        mask = Join-Path $referenceDir 'mask.png'
        depth = Join-Path $referenceDir 'depth.png'
        distance_range = @(0, 0.25)
        aspect = 'front_three_quarter'
        source = 'original'
    }
}
[IO.File]::WriteAllText($requestFile, ($request | ConvertTo-Json -Depth 6))
& (Join-Path $meshBin 'atk-mesh-cli.exe') --input $requestFile --output $output
if ($LASTEXITCODE -ne 0) { throw 'Depth reconstruction failed' }

$workspace = Join-Path (Get-Location).Path '.mesh\reconstructed'
New-Item -ItemType Directory -Path $workspace -ErrorAction Stop | Out-Null
Copy-Item -LiteralPath $output -Destination (Join-Path $workspace 'project.json') -ErrorAction Stop
& (Join-Path $meshBin 'atk-mesh.exe') --workspace $workspace --port 0
```

Use a fresh workspace so the copy cannot replace an existing project. The standalone JSON carries the native mesh. The editor opens it as `project.json` and creates any workspace history from that point onward. Use OBJ when you need geometry for interchange.

Keep the editor running. In a second terminal, set `$meshBin` to the same release directory and replace `<PORT>` with the editor port printed at startup. The query should list the reconstructed object.

```powershell
'{"command":"query_mesh","kind":"objects"}' | & (Join-Path $meshBin 'atk-mesh-cli.exe') --port <PORT>
```

See [depth reconstruction](command-reference.md#depth-reconstruction) for the input fields and fitting options.

## Reference guides

`reference_guides` takes image-space traces and writes an SVG overlay or a JSON file containing the camera and a world-space ray for each point. A ray uses `origin + t * direction`. For perspective cameras, `t` is forward camera depth. For orthographic cameras, the ray starts on the camera's center plane.

```json
{
  "command": "reference_guides",
  "camera": {"width": 800, "height": 600, "projection": "perspective", "position": [0, 0, 5], "yaw": 0, "pitch": 0, "fov": 50},
  "guides": [
    {"kind": "silhouette", "points": [[250, 180], [550, 180], [550, 420], [250, 420]]},
    {"kind": "edge", "points": [[300, 200], [300, 400]]},
    {"kind": "landmark", "points": [[400, 300]]}
  ]
}
```

For perspective camera fitting, add an `anchors` array with at least six well-distributed `{"world":[x,y,z],"pixel":[u,v]}` correspondences. Inspect `fit_diagnostics.rms_canvas_pixels` before reusing the fitted camera. Traces and anchors are supplied by the caller, and each ray still needs depth or a matching observation to establish a 3D point.

## Visual hull

`visual_hull` projects a bounded voxel grid into two or more segmented views. The quad mesh is an outer volume estimate for checking proportions before modeling parts. Use `--output hull.obj` for interchange or `--output project.json` in a fresh workspace to edit the mesh with atk-mesh.

The reply's `result.guidance` includes a user-facing summary, the basis for the volume estimate, its limitations, and next steps for part modeling and review. When presenting the hull, explain its role as a proportions reference and describe the intended refinement before showing it to the user.

```json
{
  "command": "visual_hull",
  "bounds": [[-1,-1,-1], [1,1,1]],
  "resolution": 40,
  "views": [
    {"mask": "front.png", "foreground": "black", "camera": {"width": 800, "height": 600, "yaw": 0, "pitch": 0}},
    {"mask": "side.png", "foreground": "black", "camera": {"width": 800, "height": 600, "yaw": 90, "pitch": 0}}
  ]
}
```

Use opaque black-and-white PNG masks. Foreground defaults to white. Use `"foreground":"black"` for atk-mesh silhouette captures. Resolution is the number of cells per axis and accepts 4 through 96. Choose bounds that enclose the object.

By default every view must support a voxel. For approximate views, set a view's `weight` below 1 and lower the top-level `agreement` below 1. For example, two trusted views with weight 1 and one approximate view with weight 0.25 at agreement 0.85 require both trusted views. With only a few views, the blockout can contain extra volume, and recesses that never change the silhouettes remain unresolved.
