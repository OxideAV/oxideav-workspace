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

import os
import sys
import traceback

import bpy  # type: ignore[import-not-found]


def ensure_addon(name: str) -> None:
    """Enable a Blender addon, swallowing the 'already enabled' no-op.

    On stock Ubuntu blender, several format addons (`io_scene_gltf2`,
    `io_scene_fbx`, `io_scene_obj`, `io_mesh_stl`) ship installed but
    NOT enabled by default. Calling `bpy.ops.wm.addon_enable` brings
    them up before we invoke their operators.
    """
    try:
        bpy.ops.preferences.addon_enable(module=name)
    except Exception as e:  # noqa: BLE001 — Blender raises bare exceptions
        # Already-enabled and "not found" both surface here; we log and
        # continue. The subsequent operator call will fail loudly if
        # the addon really is unavailable.
        print(f"[blender_convert] addon_enable({name!r}) note: {e}", file=sys.stderr)


def main() -> int:
    if "--" not in sys.argv:
        print(
            "blender_convert.py: missing '--' separator before script args",
            file=sys.stderr,
        )
        return 2
    args = sys.argv[sys.argv.index("--") + 1 :]
    if len(args) != 2:
        print(
            f"blender_convert.py: expected exactly 2 args (in, out), got {len(args)}: {args}",
            file=sys.stderr,
        )
        return 2
    in_path, out_path = args
    in_ext = in_path.rsplit(".", 1)[-1].lower()
    out_ext = out_path.rsplit(".", 1)[-1].lower()

    # Pre-enable every format addon we might touch. Ubuntu's
    # `blender-data` package ships the addon source but leaves them
    # disabled — without these calls, the importer ops are missing.
    for addon in (
        "io_mesh_stl",
        "io_scene_obj",
        "io_scene_gltf2",
        "io_scene_fbx",
    ):
        ensure_addon(addon)

    # Empty Blender startup file so the default cube doesn't pollute the
    # output. We must do this BEFORE any importer call.
    bpy.ops.wm.read_factory_settings(use_empty=True)

    try:
        do_convert(in_path, in_ext, out_path, out_ext)
    except Exception:  # noqa: BLE001 — we just want stderr trace + nonzero exit
        traceback.print_exc()
        return 1

    if not os.path.exists(out_path):
        # Blender's exporters return without raising even when they
        # silently skip writing (e.g. addon disabled, no compatible
        # objects). Treat a missing output file as a hard failure so
        # the Rust caller doesn't have to second-guess.
        print(
            f"blender_convert.py: exporter ran without raising but "
            f"{out_path!r} does not exist",
            file=sys.stderr,
        )
        return 1
    return 0


def do_convert(in_path: str, in_ext: str, out_path: str, out_ext: str) -> None:
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
    sys.exit(main())
