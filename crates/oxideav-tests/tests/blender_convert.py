"""Black-box conversion driver for the Blender oracle (oxideav-tests).

Invocation contract (matches `blender_convert()` in
`mesh3d_blender_assimp_oracle.rs`):

    blender --background --python tests/blender_convert.py -- IN OUT

Supported extensions are inferred from the IN/OUT path suffixes:
    stl, obj, gltf, glb, fbx

This script is a thin shim over Blender's public `bpy.ops.*` operators.
We do NOT read Blender's source — it's invoked here as an opaque
binary, with input/output exclusively through file paths the caller
controls. The script never imports anything outside `bpy` + stdlib.
"""

import sys

import bpy  # type: ignore[import-not-found]


def main() -> None:
    if "--" not in sys.argv:
        raise SystemExit("blender_convert.py: missing '--' separator before script args")
    args = sys.argv[sys.argv.index("--") + 1 :]
    if len(args) != 2:
        raise SystemExit(
            f"blender_convert.py: expected exactly 2 args (in, out), got {len(args)}: {args}"
        )
    in_path, out_path = args
    in_ext = in_path.rsplit(".", 1)[-1].lower()
    out_ext = out_path.rsplit(".", 1)[-1].lower()

    # Empty Blender startup file so the default cube doesn't pollute the
    # output. We must do this BEFORE any importer call.
    bpy.ops.wm.read_factory_settings(use_empty=True)

    # ── import ──────────────────────────────────────────────────────
    if in_ext == "stl":
        # Blender 4.x ships a fast C++ STL importer at `wm.stl_import`;
        # the legacy Python addon at `import_mesh.stl` is also kept.
        # Prefer the new operator and fall back to the addon name.
        if hasattr(bpy.ops.wm, "stl_import"):
            bpy.ops.wm.stl_import(filepath=in_path)
        else:
            bpy.ops.import_mesh.stl(filepath=in_path)
    elif in_ext == "obj":
        # Blender 3.2+ replaced the slow Python OBJ importer with a
        # native one at `wm.obj_import`. The legacy operator was at
        # `import_scene.obj` — keep it as a fallback for 3.0/3.1.
        if hasattr(bpy.ops.wm, "obj_import"):
            bpy.ops.wm.obj_import(filepath=in_path)
        else:
            bpy.ops.import_scene.obj(filepath=in_path)
    elif in_ext in ("gltf", "glb"):
        bpy.ops.import_scene.gltf(filepath=in_path)
    elif in_ext == "fbx":
        bpy.ops.import_scene.fbx(filepath=in_path)
    else:
        raise SystemExit(f"blender_convert.py: unsupported input extension {in_ext!r}")

    # ── export ──────────────────────────────────────────────────────
    if out_ext == "stl":
        if hasattr(bpy.ops.wm, "stl_export"):
            bpy.ops.wm.stl_export(filepath=out_path)
        else:
            bpy.ops.export_mesh.stl(filepath=out_path)
    elif out_ext == "obj":
        if hasattr(bpy.ops.wm, "obj_export"):
            bpy.ops.wm.obj_export(filepath=out_path)
        else:
            bpy.ops.export_scene.obj(filepath=out_path)
    elif out_ext == "gltf":
        bpy.ops.export_scene.gltf(filepath=out_path, export_format="GLTF_SEPARATE")
    elif out_ext == "glb":
        bpy.ops.export_scene.gltf(filepath=out_path, export_format="GLB")
    elif out_ext == "fbx":
        bpy.ops.export_scene.fbx(filepath=out_path)
    else:
        raise SystemExit(f"blender_convert.py: unsupported output extension {out_ext!r}")


if __name__ == "__main__":
    main()
