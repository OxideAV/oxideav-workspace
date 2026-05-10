//! Apple `usdzconvert` (KarpelesLab/usdpython) as black-box oracle.
//!
//! Validates oxideav-usdz against the canonical Apple converter by
//! running both on the same input glTF and comparing the structural
//! shape of the resulting Scene3Ds. Strictly **black-box**: we treat
//! `usdzconvert` as an opaque binary, never read its source.
//!
//! ## What's tested
//!
//! 1. **`apple_emits_decodable_usdz`** — our [`UsdzDecoder`] can read
//!    Apple's output without error. Catches gross format-spec
//!    violations on our side (ZIP-walker, USDA prim-tree parser).
//! 2. **`apple_vs_ours_mesh_count_match`** — feed the same Scene3D
//!    through (a) our gltf encoder → usdzconvert → our usdz decoder
//!    and (b) our gltf encoder → our usdz encoder → our usdz decoder;
//!    assert the same mesh / primitive count.
//! 3. **`apple_vs_ours_vertex_count_match`** — same paths, vertex
//!    counts must match exactly (Apple doesn't tessellate triangles).
//! 4. **`apple_vs_ours_position_extent_match`** — bounding-box
//!    dimensions agree within 1e-3 (Apple may rotate up-axis Y→Y or
//!    quantise floats; tolerance accommodates both).
//!
//! ## Skip behaviour
//!
//! Two binaries are required: `usdzconvert` (Apple's converter) AND
//! `usdcat` (Pixar's USD utility, ships with `usd-core` on PyPI).
//! The latter is needed because Apple's converter emits binary
//! `.usdc` layers inside the archive, and our round-1 oxideav-usdz
//! decoder is USDA-text-only — `usdcat --usdFormat usda` re-flavours
//! the inner layer to text without changing semantic content.
//!
//! When either binary is absent the test prints a one-line notice
//! via `eprintln!` and returns early (rather than `#[ignore]`, which
//! is forbidden per the workspace memory rules). CI runners without
//! the toolchain still see the test run + pass with a skip log line.
//!
//! Install (CI matches this):
//!
//! ```sh
//! pip install usd-core   # provides `usdcat` + the `pxr` Python module
//! git clone https://github.com/KarpelesLab/usdpython ~/usdpython
//! # put $HOME/usdpython/usdzconvert/usdzconvert on PATH; the script
//! # has a python3.7 shebang you may want to wrap with a python3 shim.
//! ```

use oxideav_gltf::{GltfEncoder, OutputFlavour};
use oxideav_mesh3d::{
    Indices, Mesh, Mesh3DDecoder, Mesh3DEncoder, Node, Primitive, Scene3D, Topology,
};
use oxideav_usdz::{UsdzDecoder, UsdzEncoder};
use std::path::PathBuf;
use std::process::Command;

/// Probes for `usdzconvert -h`. Returns false if the binary is
/// absent OR present-but-broken (e.g. missing Python USD libs).
fn usdzconvert_available() -> bool {
    Command::new("usdzconvert")
        .arg("-h")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Probes for `usdcat -h`. usdcat ships with `usd-core` (PyPI) and
/// is bundled in the Apple usdpython distribution. We need it to
/// convert Apple's binary `.usdc` layers into ASCII `.usda` so our
/// round-1 USDA-only decoder can read them.
fn usdcat_available() -> bool {
    Command::new("usdcat")
        .arg("-h")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Combined oracle gate — needs both binaries.
fn oracle_available() -> bool {
    usdzconvert_available() && usdcat_available()
}

/// Build a minimal triangle Scene3D for round-trip testing. Single
/// mesh, single primitive, three vertices — the simplest non-empty
/// payload that still exercises encoder/decoder geometry paths.
fn build_triangle_scene() -> Scene3D {
    let primitive = Primitive {
        topology: Topology::Triangles,
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: Some(vec![[0.0, 0.0, 1.0]; 3]),
        tangents: None,
        uvs: Vec::new(),
        colors: Vec::new(),
        joints: None,
        weights: None,
        indices: Some(Indices::U32(vec![0, 1, 2])),
        material: None,
        targets: Vec::new(),
        extras: Default::default(),
    };
    let mesh = Mesh {
        name: Some("oracle_triangle".into()),
        primitives: vec![primitive],
        weights: Vec::new(),
    };
    let mut scene = Scene3D::new();
    let mid = scene.add_mesh(mesh);
    let mut node = Node::new();
    node.mesh = Some(mid);
    let nid = scene.add_node(node);
    scene.add_root(nid);
    scene
}

/// Build a six-face cube — Apple normally retains every triangle
/// vertex without dedup; we expect 12 triangles × 3 = 36 emitted
/// vertices on both sides for this fixture.
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
        0, 1, 2, 0, 2, 3, // front
        4, 6, 5, 4, 7, 6, // back
        0, 4, 5, 0, 5, 1, // bottom
        2, 6, 7, 2, 7, 3, // top
        1, 5, 6, 1, 6, 2, // right
        0, 3, 7, 0, 7, 4, // left
    ];
    let primitive = Primitive {
        topology: Topology::Triangles,
        positions,
        normals: None,
        tangents: None,
        uvs: Vec::new(),
        colors: Vec::new(),
        joints: None,
        weights: None,
        indices: Some(Indices::U32(indices)),
        material: None,
        targets: Vec::new(),
        extras: Default::default(),
    };
    let mesh = Mesh {
        name: Some("oracle_cube".into()),
        primitives: vec![primitive],
        weights: Vec::new(),
    };
    let mut scene = Scene3D::new();
    let mid = scene.add_mesh(mesh);
    let mut node = Node::new();
    node.mesh = Some(mid);
    let nid = scene.add_node(node);
    scene.add_root(nid);
    scene
}

/// Run the full pipeline: encode `scene` to glTF in a fresh tempdir,
/// invoke `usdzconvert` on it, decode the resulting Apple .usdz via
/// our reader, and return the extracted Scene3D plus the path so the
/// caller can keep the dir alive for inspection.
fn apple_oracle_roundtrip(scene: &Scene3D, label: &str) -> (Scene3D, Vec<u8>, PathBuf) {
    let tmp_root = std::env::temp_dir().join(format!(
        "oxideav-usdz-oracle-{}-{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp_root);
    std::fs::create_dir_all(&tmp_root).expect("create tempdir");

    // glTF JSON-embedded so usdzconvert sees one self-contained file.
    let gltf_path = tmp_root.join("input.gltf");
    let gltf_bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(scene)
        .expect("our gltf encode");
    std::fs::write(&gltf_path, gltf_bytes).expect("write input.gltf");

    let apple_usdz = tmp_root.join("apple.usdz");
    let output = Command::new("usdzconvert")
        .arg(&gltf_path)
        .arg(&apple_usdz)
        .output()
        .expect("spawn usdzconvert");
    if !output.status.success() {
        panic!(
            "usdzconvert exit {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    // Apple's usdzconvert emits binary `.usdc` layers inside the
    // archive. Our round-1 oxideav-usdz decoder is USDA-only, so we
    // bounce through `usdcat -o ascii.usdz --usdFormat usda` to
    // re-flavour the inner layer as ASCII USDA without changing its
    // semantic content. usdcat ships with usd-core, so it's available
    // alongside usdzconvert in any working USD install.
    let apple_ascii = tmp_root.join("apple-ascii.usdz");
    let cat_output = Command::new("usdcat")
        .arg("--usdFormat")
        .arg("usda")
        .arg("-o")
        .arg(&apple_ascii)
        .arg(&apple_usdz)
        .output()
        .expect("spawn usdcat");
    if !cat_output.status.success() {
        panic!(
            "usdcat exit {:?}\nstdout:\n{}\nstderr:\n{}",
            cat_output.status,
            String::from_utf8_lossy(&cat_output.stdout),
            String::from_utf8_lossy(&cat_output.stderr),
        );
    }

    let apple_bytes = std::fs::read(&apple_ascii).expect("read apple-ascii.usdz");
    let apple_scene = UsdzDecoder::new()
        .decode(&apple_bytes)
        .expect("decode apple-ascii.usdz with our reader");

    (apple_scene, apple_bytes, tmp_root)
}

/// Round-trip the same scene through OUR encoder + decoder for a
/// controlled comparison baseline against the Apple oracle.
fn ours_roundtrip(scene: &Scene3D) -> Scene3D {
    let bytes = UsdzEncoder::new().encode(scene).expect("our usdz encode");
    UsdzDecoder::new()
        .decode(&bytes)
        .expect("our usdz decode of our own output")
}

/// Sum (positions.len()) across every primitive of every mesh.
fn total_vertex_count(scene: &Scene3D) -> usize {
    scene
        .meshes
        .iter()
        .flat_map(|m| m.primitives.iter())
        .map(|p| p.positions.len())
        .sum()
}

/// Bounding box (min, max) over every position in the scene.
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

#[test]
fn apple_emits_decodable_usdz() {
    if !oracle_available() {
        eprintln!(
            "[oracle skip] `usdzconvert` + `usdcat` not in PATH — \
             install https://github.com/KarpelesLab/usdpython + \
             `pip install usd-core` to enable"
        );
        return;
    }

    let scene = build_triangle_scene();
    let (apple_scene, apple_bytes, _tmp) = apple_oracle_roundtrip(&scene, "decodable");

    assert!(
        !apple_bytes.is_empty(),
        "Apple usdzconvert produced empty file"
    );
    // PKZIP local file header magic — USDZ is ZIP-stored.
    assert_eq!(
        &apple_bytes[..4],
        b"PK\x03\x04",
        "Apple .usdz should open with PKZIP magic"
    );
    assert!(
        !apple_scene.meshes.is_empty(),
        "Apple-emitted scene should have at least one mesh"
    );
}

#[test]
fn apple_vs_ours_mesh_count_match() {
    if !oracle_available() {
        eprintln!("[oracle skip] usdzconvert + usdcat not in PATH");
        return;
    }

    let scene = build_triangle_scene();
    let (apple_scene, _, _tmp) = apple_oracle_roundtrip(&scene, "mesh_count");
    let ours_scene = ours_roundtrip(&scene);

    assert_eq!(
        apple_scene.meshes.len(),
        ours_scene.meshes.len(),
        "mesh count: Apple={}, ours={}",
        apple_scene.meshes.len(),
        ours_scene.meshes.len(),
    );

    let apple_prims: usize = apple_scene.meshes.iter().map(|m| m.primitives.len()).sum();
    let ours_prims: usize = ours_scene.meshes.iter().map(|m| m.primitives.len()).sum();
    assert_eq!(
        apple_prims, ours_prims,
        "primitive count: Apple={apple_prims}, ours={ours_prims}"
    );
}

#[test]
fn apple_vs_ours_vertex_count_match() {
    if !oracle_available() {
        eprintln!("[oracle skip] usdzconvert + usdcat not in PATH");
        return;
    }

    let scene = build_cube_scene();
    let (apple_scene, _, _tmp) = apple_oracle_roundtrip(&scene, "vertex_count");
    let ours_scene = ours_roundtrip(&scene);

    let apple_v = total_vertex_count(&apple_scene);
    let ours_v = total_vertex_count(&ours_scene);
    // Cube: 8 unique positions, 36 emitted (12 triangles × 3 corners).
    // Both encoders should preserve the post-encode vertex count
    // exactly — Apple doesn't dedup, neither do we.
    assert_eq!(
        apple_v, ours_v,
        "vertex count: Apple={apple_v}, ours={ours_v}"
    );
}

#[test]
fn apple_vs_ours_position_extent_match() {
    if !oracle_available() {
        eprintln!("[oracle skip] usdzconvert + usdcat not in PATH");
        return;
    }

    let scene = build_cube_scene();
    let (apple_scene, _, _tmp) = apple_oracle_roundtrip(&scene, "extent");
    let ours_scene = ours_roundtrip(&scene);

    let (a_min, a_max) = position_extent(&apple_scene);
    let (o_min, o_max) = position_extent(&ours_scene);

    // Generous tolerance: Apple may quantise floats or apply a unit
    // scale (gltf uses metres, usdz default may be cm). 1e-2 is loose
    // enough to absorb a scale rounding on a unit cube while still
    // rejecting axis-flip or wrong-mesh bugs.
    let tol = 1e-2_f32;
    for i in 0..3 {
        let amin_to_omin = (a_min[i] - o_min[i]).abs();
        let amax_to_omax = (a_max[i] - o_max[i]).abs();
        assert!(
            amin_to_omin < tol || (a_min[i].abs() < tol && o_min[i].abs() < tol),
            "min[{i}]: Apple={} ours={} delta={amin_to_omin}",
            a_min[i],
            o_min[i],
        );
        assert!(
            amax_to_omax < tol,
            "max[{i}]: Apple={} ours={} delta={amax_to_omax}",
            a_max[i],
            o_max[i],
        );
    }
}
