import os
import sys
from pathlib import Path
from types import ModuleType

import torch
from PIL import Image
from skimage.measure import marching_cubes


def extract_isosurface(volume, level):
    vertices, faces, _, _ = marching_cubes(
        volume.float().cpu().numpy(), level, allow_degenerate=False
    )
    # TripoSR expects torchmcubes' reversed axis order.
    return torch.from_numpy(vertices[:, ::-1].copy()), torch.from_numpy(faces.copy())


def prepare_image(path, mask_path):
    with Image.open(path) as source:
        image = source.convert("RGBA")
    if mask_path:
        with Image.open(mask_path) as source:
            mask = source.convert("L")
        if image.size != mask.size:
            raise ValueError("mask dimensions must match the image")
        image.putalpha(mask)

    bounds = image.getchannel("A").getbbox()
    if bounds is None:
        raise ValueError("image has no foreground")
    image = image.crop(bounds)
    side = int(max(image.size) / 0.85 + 1)
    canvas = Image.new("RGB", (side, side), (128, 128, 128))
    offset = ((side - image.width) // 2, (side - image.height) // 2)
    canvas.paste(image, offset, image.getchannel("A"))
    return canvas


def main():
    checkout, image_path, output, resolution, *masks = sys.argv[1:]
    sys.path.insert(0, str(Path(checkout).resolve()))
    cubes = ModuleType("torchmcubes")
    cubes.marching_cubes = extract_isosurface
    sys.modules[cubes.__name__] = cubes
    from tsr.system import TSR

    image = prepare_image(image_path, masks[0] if masks else None)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    torch.set_num_threads(min(8, torch.get_num_threads()))
    weights = os.environ.get("ATK_MESH_TRIPOSR_WEIGHTS", "stabilityai/TripoSR")
    model = TSR.from_pretrained(weights, config_name="config.yaml", weight_name="model.ckpt")
    model = model.eval().to(device)
    model.renderer.set_chunk_size(8192)
    with torch.inference_mode():
        scene = model([image], device=device)
        mesh, = model.extract_mesh(scene, False, resolution=int(resolution))

    # Convert Z-up with +X toward the input camera to Y-up with +Z toward it.
    mesh.vertices = mesh.vertices[:, [1, 2, 0]]
    mesh.fix_normals(multibody=True)
    mesh.export(output, include_normals=False)


if __name__ == "__main__":
    main()
