//! Blender + Open Asset Import Library (assimp) as black-box oracles.
//!
//! Validates our 3D-format encoders/decoders against two industry
//! converters by feeding the same `Scene3D` through both:
//!
//! * **Path A** — our encoder writes format X, the oracle (Blender or
//!   `assimp` CLI) converts X → Y, our decoder reads Y back.
//! * **Path B** — our encoder writes format Y, our decoder reads Y
//!   back (a clean baseline).
//!
//! The two resulting `Scene3D`s are compared for mesh/primitive count,
//! vertex count, and bounding-box extent. Strict source-isolation:
//! both `blender` and `assimp` are treated as opaque binaries — we
//! never read or vendor their source (per workspace
//! `feedback_no_external_libs`).
//!
//! ## Why both?
//!
//! Blender's importers/exporters are the de-facto reference for FBX
//! (Autodesk's FBX SDK is non-redistributable, so Blender's
//! community-maintained reader/writer is the gold standard). assimp
//! covers a broader matrix of formats and is the canonical "import any
//! 3D file" library, used by virtually every game engine. Comparing
//! against both catches divergences either oracle has on its own.
//!
//! ## Pairwise matrix
//!
//! | from \ to | gltf (Blender) | obj (Blender) | glb (Blender) | gltf (assimp) | obj (assimp) |
//! |-----------|----------------|---------------|---------------|---------------|--------------|
//! | stl       |       ✓        |       —       |       —       |       ✓       |      —       |
//! | obj       |       —        |       —       |       ✓       |       —       |      —       |
//! | gltf      |       —        |       ✓       |       —       |       —       |      ✓       |
//! | fbx       |       ✓        |       —       |       —       |       —       |      ✓       |
//!
//! 8 oracle tests, evenly split (4 Blender, 4 assimp) covering each
//! direction at least once.
//!
//! ## Skip behaviour
//!
//! Probes via `Command::new("blender").arg("--version")` (resp.
//! `assimp version`) at test entry. Missing binary → one-line
//! `eprintln!` notice + early `return`. **No `#[ignore]`** — the
//! workspace memory rules forbid it, and this oracle's CI job hard-
//! fails the verify step if either binary isn't installed.
//!
//! ## Tolerance policy
//!
//! - **Mesh count** — may differ by Blender's "join on import" /
//!   "split by material" heuristics; we assert >= 1 mesh on both
//!   sides and that primitive sums are non-zero.
//! - **Vertex count** — exact equality is fragile across importers
//!   that re-tessellate or merge co-located vertices. We assert that
//!   both counts agree within ±50 % (the typical merge-by-distance
//!   shrinks counts on the cube but not by more than 2×).
//! - **Bounding box** — strict ±1e-2 tolerance. Axis flips, unit
//!   conversions (m vs cm), or bogus mesh substitutions all show up
//!   here as much larger deltas.

use std::path::{Path, PathBuf};
use std::process::Command;

use oxideav_fbx::FbxDecoder;
use oxideav_gltf::{GltfDecoder, GltfEncoder, OutputFlavour};
use oxideav_mesh3d::{
    Indices, Mesh, Mesh3DDecoder, Mesh3DEncoder, Node, Primitive, Scene3D, Topology,
};
use oxideav_obj::{ObjDecoder, ObjEncoder};
use oxideav_stl::{StlDecoder, StlEncoder};

// ───────────────────────── oracle probes ─────────────────────────

/// Probe `blender --version`. Returns false if the binary is absent or
/// returns a non-zero exit code (e.g. crashed at startup). We ONLY use
/// the exit status — we do not parse stdout, so the script stays
/// black-box: we don't depend on Blender's version string format.
fn blender_available() -> bool {
    Command::new("blender")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Probe `assimp version` (note: subcommand `version`, not `--version`,
/// per upstream's CLI). Returns false on missing binary or non-zero
/// exit.
fn assimp_available() -> bool {
    Command::new("assimp")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ───────────────────────── tempdir helper ────────────────────────

/// Create a fresh per-test tempdir under `std::env::temp_dir()`.
///
/// The directory is removed on entry (so prior runs don't poison) and
/// returned so the caller keeps it alive for the duration of the test
/// (artefacts can be inspected by hand if a test fails locally).
fn fresh_tempdir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "oxideav-mesh3d-oracle-{}-{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

// ─────────────────── Blender / assimp drivers ────────────────────

/// Path to the Blender Python conversion script committed alongside
/// this test file. CARGO_MANIFEST_DIR points at `crates/oxideav-tests`;
/// the script lives at `tests/blender_convert.py`.
fn blender_convert_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("blender_convert.py")
}

/// Spawn `blender --background --python <script> -- <in> <out>` and
/// panic on non-zero exit, dumping stdout/stderr for diagnosis.
fn blender_convert(input: &Path, output: &Path) {
    let script = blender_convert_script();
    let result = Command::new("blender")
        .arg("--background")
        .arg("--python")
        .arg(&script)
        .arg("--")
        .arg(input)
        .arg(output)
        .output()
        .expect("spawn blender");
    if !result.status.success() {
        panic!(
            "blender conversion exit {:?}\nin: {}\nout: {}\nstdout:\n{}\nstderr:\n{}",
            result.status,
            input.display(),
            output.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
        );
    }
    assert!(
        output.exists(),
        "blender ran but did not produce {}",
        output.display()
    );
}

/// Spawn `assimp export <in> <out>` (assimp infers I/O format from
/// the file extensions). Panics on non-zero exit.
fn assimp_convert(input: &Path, output: &Path) {
    let result = Command::new("assimp")
        .arg("export")
        .arg(input)
        .arg(output)
        .output()
        .expect("spawn assimp");
    if !result.status.success() {
        panic!(
            "assimp conversion exit {:?}\nin: {}\nout: {}\nstdout:\n{}\nstderr:\n{}",
            result.status,
            input.display(),
            output.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
        );
    }
    assert!(
        output.exists(),
        "assimp ran but did not produce {}",
        output.display()
    );
}

// ─────────────────────── fixtures (Scene3D) ──────────────────────

/// Six-face cube — 8 unique positions × 36 emitted vertices (12
/// triangles × 3 corners). Same shape as the USDZ oracle's cube
/// fixture, kept intentionally so failures in one oracle vs the other
/// are easy to triangulate against the existing baselines.
fn build_cube_scene() -> Scene3D {
    let positions = vec![
        [0.0_f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    let indices: Vec<u32> = vec![
        0, 1, 2, 0, 2, 3, // front  (z=0)
        4, 6, 5, 4, 7, 6, // back   (z=1)
        0, 4, 5, 0, 5, 1, // bottom (y=0)
        2, 6, 7, 2, 7, 3, // top    (y=1)
        1, 5, 6, 1, 6, 2, // right  (x=1)
        0, 3, 7, 0, 7, 4, // left   (x=0)
    ];
    let mut primitive = Primitive::new(Topology::Triangles);
    primitive.positions = positions;
    primitive.indices = Some(Indices::U32(indices));
    let mesh = Mesh::new(Some("oracle_cube".to_string())).with_primitive(primitive);
    let mut scene = Scene3D::new();
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

// ──────────────────────── shape comparators ──────────────────────

/// Sum of `positions.len()` across every primitive of every mesh.
/// Mirrors the helper in `mesh3d_usdz_apple_oracle.rs` so both oracle
/// suites speak the same vocabulary.
fn total_vertex_count(scene: &Scene3D) -> usize {
    scene
        .meshes
        .iter()
        .flat_map(|m| m.primitives.iter())
        .map(|p| p.positions.len())
        .sum()
}

/// Total primitive count across the scene.
fn total_primitive_count(scene: &Scene3D) -> usize {
    scene.meshes.iter().map(|m| m.primitives.len()).sum()
}

/// Bounding box (min, max) over every position in every primitive of
/// every mesh.
fn position_extent(scene: &Scene3D) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for mesh in &scene.meshes {
        for prim in &mesh.primitives {
            for [x, y, z] in &prim.positions {
                for (i, v) in [*x, *y, *z].iter().enumerate() {
                    if *v < min[i] {
                        min[i] = *v;
                    }
                    if *v > max[i] {
                        max[i] = *v;
                    }
                }
            }
        }
    }
    (min, max)
}

/// Asserts that two scenes agree on *shape* within the loose-but-
/// meaningful tolerances documented in the module-level doc comment:
///
/// * Both must have at least one mesh and one primitive.
/// * Vertex counts must be within ±50 % of each other (importers
///   often merge or split vertices).
/// * Bounding boxes must agree per-axis within `extent_tol` (default
///   1e-2 — loose enough for unit-scale rounding, tight enough to
///   catch axis flips).
fn assert_scenes_shape_agree(a: &Scene3D, b: &Scene3D, label: &str, extent_tol: f32) {
    assert!(
        !a.meshes.is_empty(),
        "{label}: oracle scene has zero meshes"
    );
    assert!(
        !b.meshes.is_empty(),
        "{label}: baseline scene has zero meshes"
    );
    let a_prims = total_primitive_count(a);
    let b_prims = total_primitive_count(b);
    assert!(
        a_prims > 0 && b_prims > 0,
        "{label}: primitive count zero on one side (oracle={a_prims}, baseline={b_prims})"
    );

    let av = total_vertex_count(a);
    let bv = total_vertex_count(b);
    assert!(av > 0 && bv > 0, "{label}: zero vertices");
    let ratio = av.max(bv) as f32 / av.min(bv).max(1) as f32;
    assert!(
        ratio <= 1.5,
        "{label}: vertex counts disagree too much: oracle={av} baseline={bv} (ratio={ratio:.2}, max=1.5)"
    );

    let (a_min, a_max) = position_extent(a);
    let (b_min, b_max) = position_extent(b);
    for i in 0..3 {
        let dmin = (a_min[i] - b_min[i]).abs();
        let dmax = (a_max[i] - b_max[i]).abs();
        assert!(
            dmin < extent_tol,
            "{label}: bbox min[{i}] disagrees: oracle={} baseline={} (Δ={dmin}, tol={extent_tol})",
            a_min[i],
            b_min[i],
        );
        assert!(
            dmax < extent_tol,
            "{label}: bbox max[{i}] disagrees: oracle={} baseline={} (Δ={dmax}, tol={extent_tol})",
            a_max[i],
            b_max[i],
        );
    }
}

// ───────────────────── encoder/decoder helpers ───────────────────

fn encode_stl(scene: &Scene3D, path: &Path) {
    let bytes = StlEncoder::new_binary()
        .encode(scene)
        .expect("our STL encode");
    std::fs::write(path, bytes).expect("write STL");
}

fn encode_obj(scene: &Scene3D, path: &Path) {
    let bytes = ObjEncoder::new().encode(scene).expect("our OBJ encode");
    std::fs::write(path, bytes).expect("write OBJ");
}

fn encode_gltf_separate(scene: &Scene3D, path: &Path) {
    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(scene)
        .expect("our gltf JSON-embedded encode");
    std::fs::write(path, bytes).expect("write gltf");
}

fn encode_glb(scene: &Scene3D, path: &Path) {
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(scene)
        .expect("our GLB encode");
    std::fs::write(path, bytes).expect("write glb");
}

fn decode_stl(path: &Path) -> Scene3D {
    let bytes = std::fs::read(path).expect("read STL");
    StlDecoder::new().decode(&bytes).expect("our STL decode")
}

fn decode_obj(path: &Path) -> Scene3D {
    let bytes = std::fs::read(path).expect("read OBJ");
    ObjDecoder::new().decode(&bytes).expect("our OBJ decode")
}

fn decode_gltf(path: &Path) -> Scene3D {
    let bytes = std::fs::read(path).expect("read gltf/glb");
    GltfDecoder::new()
        .decode(&bytes)
        .expect("our gltf/glb decode")
}

fn decode_fbx(path: &Path) -> Scene3D {
    let bytes = std::fs::read(path).expect("read fbx");
    FbxDecoder::new().decode(&bytes).expect("our fbx decode")
}

/// Baseline path: encode `scene` as `out_ext` directly with our
/// encoder, decode back with our decoder. Returned scene is the
/// "ground truth" the oracle is compared against.
fn ours_roundtrip(scene: &Scene3D, dir: &Path, out_ext: &str) -> Scene3D {
    let path = dir.join(format!("ours.{out_ext}"));
    match out_ext {
        "stl" => {
            encode_stl(scene, &path);
            decode_stl(&path)
        }
        "obj" => {
            encode_obj(scene, &path);
            decode_obj(&path)
        }
        "gltf" => {
            encode_gltf_separate(scene, &path);
            decode_gltf(&path)
        }
        "glb" => {
            encode_glb(scene, &path);
            decode_gltf(&path)
        }
        other => panic!("ours_roundtrip: no encoder for output extension {other:?}"),
    }
}

// ──────────────────────── Blender oracles ────────────────────────

/// stl → gltf via Blender; assert mesh/vertex/extent shape agrees
/// with our gltf self-roundtrip.
#[test]
fn blender_oracle_stl_to_gltf() {
    if !blender_available() {
        eprintln!(
            "[oracle skip] blender not in PATH — install Blender 3.x+ \
             (apt: blender) to enable this test"
        );
        return;
    }
    let dir = fresh_tempdir("blender_stl_to_gltf");
    let scene = build_cube_scene();

    let stl_in = dir.join("cube.stl");
    encode_stl(&scene, &stl_in);
    let gltf_out = dir.join("from_blender.gltf");
    blender_convert(&stl_in, &gltf_out);

    let oracle_scene = decode_gltf(&gltf_out);
    let baseline = ours_roundtrip(&scene, &dir, "gltf");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "blender stl→gltf", 1e-2);
}

/// obj → glb via Blender; assert shape agrees.
#[test]
fn blender_oracle_obj_to_glb() {
    if !blender_available() {
        eprintln!("[oracle skip] blender not in PATH");
        return;
    }
    let dir = fresh_tempdir("blender_obj_to_glb");
    let scene = build_cube_scene();

    let obj_in = dir.join("cube.obj");
    encode_obj(&scene, &obj_in);
    let glb_out = dir.join("from_blender.glb");
    blender_convert(&obj_in, &glb_out);

    let oracle_scene = decode_gltf(&glb_out);
    let baseline = ours_roundtrip(&scene, &dir, "glb");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "blender obj→glb", 1e-2);
}

/// gltf → obj via Blender; assert shape agrees with our obj baseline.
#[test]
fn blender_oracle_gltf_to_obj() {
    if !blender_available() {
        eprintln!("[oracle skip] blender not in PATH");
        return;
    }
    let dir = fresh_tempdir("blender_gltf_to_obj");
    let scene = build_cube_scene();

    let gltf_in = dir.join("cube.gltf");
    encode_gltf_separate(&scene, &gltf_in);
    let obj_out = dir.join("from_blender.obj");
    blender_convert(&gltf_in, &obj_out);

    let oracle_scene = decode_obj(&obj_out);
    let baseline = ours_roundtrip(&scene, &dir, "obj");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "blender gltf→obj", 1e-2);
}

/// fbx → gltf via Blender; the FBX itself is produced by Blender (we
/// have no FBX encoder), so the oracle path is:
/// gltf (ours) → fbx (Blender) → gltf (Blender) → decode with ours.
/// Compare against our gltf self-roundtrip baseline.
#[test]
fn blender_oracle_fbx_to_gltf() {
    if !blender_available() {
        eprintln!("[oracle skip] blender not in PATH");
        return;
    }
    let dir = fresh_tempdir("blender_fbx_to_gltf");
    let scene = build_cube_scene();

    // Stage 1: ours → gltf, then have Blender export it as FBX. This
    // is the "Blender writes FBX" leg (we can't author FBX directly).
    let gltf_seed = dir.join("seed.gltf");
    encode_gltf_separate(&scene, &gltf_seed);
    let fbx_path = dir.join("from_blender.fbx");
    blender_convert(&gltf_seed, &fbx_path);

    // Stage 2: now FBX-to-gltf via Blender — exercises Blender's FBX
    // importer + gltf exporter together, decoded by our gltf reader.
    let gltf_out = dir.join("from_blender.gltf");
    blender_convert(&fbx_path, &gltf_out);
    let oracle_scene = decode_gltf(&gltf_out);

    // ALSO sanity-check we can decode the Blender-emitted FBX directly
    // via our oxideav-fbx round-1 decoder. This is the bit the task
    // brief specifically calls out as the FBX coverage path.
    let fbx_scene = decode_fbx(&fbx_path);
    assert!(
        !fbx_scene.meshes.is_empty(),
        "our FBX decoder returned an empty scene from Blender's .fbx"
    );

    let baseline = ours_roundtrip(&scene, &dir, "gltf");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "blender fbx→gltf", 1e-2);
}

// ───────────────────────── assimp oracles ────────────────────────

/// stl → gltf via assimp; assert shape agrees.
#[test]
fn assimp_oracle_stl_to_gltf() {
    if !assimp_available() {
        eprintln!(
            "[oracle skip] assimp not in PATH — install (apt: \
             assimp-utils) to enable this test"
        );
        return;
    }
    let dir = fresh_tempdir("assimp_stl_to_gltf");
    let scene = build_cube_scene();

    let stl_in = dir.join("cube.stl");
    encode_stl(&scene, &stl_in);
    // assimp picks the writer by extension; gltf2 = JSON gltf.
    let gltf_out = dir.join("from_assimp.gltf");
    assimp_convert(&stl_in, &gltf_out);

    let oracle_scene = decode_gltf(&gltf_out);
    let baseline = ours_roundtrip(&scene, &dir, "gltf");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "assimp stl→gltf", 1e-2);
}

/// obj → gltf via assimp; assert shape agrees.
#[test]
fn assimp_oracle_obj_to_gltf() {
    if !assimp_available() {
        eprintln!("[oracle skip] assimp not in PATH");
        return;
    }
    let dir = fresh_tempdir("assimp_obj_to_gltf");
    let scene = build_cube_scene();

    let obj_in = dir.join("cube.obj");
    encode_obj(&scene, &obj_in);
    let gltf_out = dir.join("from_assimp.gltf");
    assimp_convert(&obj_in, &gltf_out);

    let oracle_scene = decode_gltf(&gltf_out);
    let baseline = ours_roundtrip(&scene, &dir, "gltf");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "assimp obj→gltf", 1e-2);
}

/// gltf → obj via assimp; assert shape agrees.
#[test]
fn assimp_oracle_gltf_to_obj() {
    if !assimp_available() {
        eprintln!("[oracle skip] assimp not in PATH");
        return;
    }
    let dir = fresh_tempdir("assimp_gltf_to_obj");
    let scene = build_cube_scene();

    let gltf_in = dir.join("cube.gltf");
    encode_gltf_separate(&scene, &gltf_in);
    let obj_out = dir.join("from_assimp.obj");
    assimp_convert(&gltf_in, &obj_out);

    let oracle_scene = decode_obj(&obj_out);
    let baseline = ours_roundtrip(&scene, &dir, "obj");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "assimp gltf→obj", 1e-2);
}

/// fbx → obj via assimp. Like the Blender FBX test, the FBX itself is
/// produced by Blender (we have no FBX encoder), so this exercises
/// assimp's FBX *importer* against Blender's FBX *writer* on the
/// reading side, with the resulting OBJ checked by our decoder.
///
/// This test depends on BOTH oracle binaries being present (Blender to
/// produce the .fbx fixture, assimp to import it).
#[test]
fn assimp_oracle_fbx_to_obj() {
    if !blender_available() {
        eprintln!("[oracle skip] blender not in PATH (needed to author .fbx for assimp)");
        return;
    }
    if !assimp_available() {
        eprintln!("[oracle skip] assimp not in PATH");
        return;
    }
    let dir = fresh_tempdir("assimp_fbx_to_obj");
    let scene = build_cube_scene();

    let gltf_seed = dir.join("seed.gltf");
    encode_gltf_separate(&scene, &gltf_seed);
    let fbx_path = dir.join("from_blender.fbx");
    blender_convert(&gltf_seed, &fbx_path);

    let obj_out = dir.join("from_assimp.obj");
    assimp_convert(&fbx_path, &obj_out);
    let oracle_scene = decode_obj(&obj_out);

    let baseline = ours_roundtrip(&scene, &dir, "obj");
    assert_scenes_shape_agree(&oracle_scene, &baseline, "assimp fbx→obj", 1e-2);
}
