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
    # Support the synthetic ".bbox.txt" output mode used by the
    # canonical-extents test in `mesh3d_blender_assimp_oracle.rs`.
    # Detected as a multi-suffix because Path.rsplit only picks up the
    # final ".txt" — we want the full "bbox.txt" composite.
    if out_path.lower().endswith(".bbox.txt"):
        out_ext = "bbox.txt"
    else:
        out_ext = out_path.rsplit(".", 1)[-1].lower()

    # Pre-enable every format addon we might touch. Ubuntu's
    # `blender-data` package ships the addon source but leaves them
    # disabled — without these calls, the importer ops are missing.
    #
    # `io_scene_obj` is intentionally absent: Blender 4.0+ replaced
    # the Python OBJ addon with a built-in C++ operator (`wm.obj_import`
    # / `wm.obj_export`) that doesn't need an addon registration —
    # trying to enable the old module logs a benign "No module named
    # 'io_scene_obj'" warning.
    for addon in (
        "io_mesh_stl",
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
    elif out_ext == "bbox.txt":
        write_bbox_report(out_path)
    else:
        raise SystemExit(f"blender_convert.py: unsupported output extension {out_ext!r}")


def write_bbox_report(out_path: str) -> None:
    """Emit a 3-line text file with the scene's axis-aligned bbox.

    The lines are:
        min  X Y Z
        max  X Y Z
        dims X Y Z

    Blender's internal coordinate frame is Z-up, and `bpy.ops.import_*`
    permutes Y-up sources into that frame on import. So the dims line
    is already in the canonical Z-up frame — the Rust test parses it
    verbatim into a `CanonicalExtents`.

    Only MESH objects participate. We aggregate over every mesh object
    in the scene (the importer may create more than one — e.g.
    Blender's STL importer splits a multi-solid file).
    """
    import math
    import mathutils  # type: ignore[import-not-found]

    mn = [math.inf, math.inf, math.inf]
    mx = [-math.inf, -math.inf, -math.inf]
    found = False
    for obj in bpy.context.scene.objects:
        if obj.type != "MESH" or obj.data is None:
            continue
        wm = obj.matrix_world
        for v in obj.data.vertices:
            p = wm @ v.co  # mathutils.Vector
            for i in range(3):
                if p[i] < mn[i]:
                    mn[i] = p[i]
                if p[i] > mx[i]:
                    mx[i] = p[i]
            found = True

    if not found:
        # Write an explicit zero-bbox so the caller fails with a
        # readable assertion (rather than "no such file").
        mn = [0.0, 0.0, 0.0]
        mx = [0.0, 0.0, 0.0]

    dims = [mx[i] - mn[i] for i in range(3)]
    with open(out_path, "w", encoding="utf-8") as fh:
        fh.write(f"min  {mn[0]:.9f} {mn[1]:.9f} {mn[2]:.9f}\n")
        fh.write(f"max  {mx[0]:.9f} {mx[1]:.9f} {mx[2]:.9f}\n")
        fh.write(f"dims {dims[0]:.9f} {dims[1]:.9f} {dims[2]:.9f}\n")
    # Touch a no-op reference to keep linters happy if `mathutils`
    # ever isn't strictly required (we use it via `obj.matrix_world @ v.co`).
    _ = mathutils


if __name__ == "__main__":
    sys.exit(main())
