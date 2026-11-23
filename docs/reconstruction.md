# Image reconstruction

Run each command with `mesh-cli --input request.json --output FILE`. These commands run locally without an editor. They use the same camera fields as `capture`. Match the camera's width and height to the source image and keep all cameras in the same world coordinate system.

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

`visual_hull` projects a bounded voxel grid into two or more segmented views. The quad mesh is an outer volume estimate for checking proportions before modeling parts. Use `--output hull.obj` for interchange or `--output project.json` in a fresh workspace to edit the mesh with Mesh Workbench.

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

Use opaque black-and-white PNG masks. Foreground defaults to white. Use `"foreground":"black"` for Mesh Workbench silhouette captures. Mask paths resolve from the CLI working directory. Resolution is the number of cells per axis and accepts 4 through 96. Choose bounds that enclose the object.

By default every view must support a voxel. For approximate views, set a view's `weight` below 1 and lower the top-level `agreement` below 1. For example, two trusted views with weight 1 and one approximate view with weight 0.25 at agreement 0.85 require both trusted views. With only a few views, the blockout can contain extra volume, and recesses that never change the silhouettes remain unresolved.
