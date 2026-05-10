//! Round-4 coverage extensions:
//!
//! 1. **Skinning data primitive-level survival** — gltf 0.0.0 round-trips
//!    `Primitive::joints` + `Primitive::weights` (the per-vertex skinning
//!    attributes); STL + OBJ silently drop them (they have no concept of
//!    bones). The `Scene3D::skins` / `Scene3D::skeletons` /
//!    `Scene3D::animations` collections are NOT round-tripped by any
//!    sibling encoder shipped at version 0.0.0 — that surface lights up
//!    when the gltf encoder gains skin-array serialisation. We pin the
//!    current behaviour explicitly so a future gltf-side upgrade
//!    flips these tests from "drops" to "survives" without surprise.
//!
//! 2. **Multi-primitive vertex pool dedup (OBJ)** — the OBJ encoder
//!    interns every `(position, uv, normal)` triple into a single global
//!    pool, so two primitives that share the same physical corner emit
//!    one `v` line, not two. Round-3's `quad_scene` only stressed the
//!    intra-primitive case; here we exercise the inter-primitive case
//!    with two triangles in two separate `Primitive`s sharing two
//!    vertices.
//!
//! 3. **Multi-material binding** — multiple primitives, each bound to a
//!    different `Material`, must emit distinct `usemtl` directives in
//!    OBJ output, and the gltf encoder must preserve each primitive's
//!    `material` index so the round-trip resolves to the same
//!    materials.
//!
//! 4. **Cross-format extras side-channel preservation matrix** —
//!    explicit pinning of which `extras` keys survive (and which get
//!    silently dropped) for every (origin, target) format pair. The
//!    matrix here is the practical contract a downstream conversion
//!    pipeline can lean on — see the per-test docstring for which
//!    surface each one covers.
//!
//! No `#[ignore]` is used: every test asserts the *current* observable
//! behaviour, including documented drops. Future encoder upgrades flip
//! the assertion direction in the same commit that lifts the gap.

use oxideav_mesh3d::{
    Material, Mesh, Mesh3DDecoder, Mesh3DEncoder, Node, Primitive, Scene3D, Topology,
};

use oxideav_gltf::{GltfDecoder, GltfEncoder, OutputFlavour};
use oxideav_obj::{ObjDecoder, ObjEncoder};
use oxideav_stl::{StlDecoder, StlEncoder};

// ─────────────────────────── fixtures ────────────────────────────

/// Skinned-triangle scene: one triangle whose three vertices are bound
/// to four-joint groups with weights that sum to 1.0 per vertex. The
/// joint indices intentionally vary across vertices so a per-index
/// dedup at decode time would observe a difference vs the encoded set.
fn skinned_triangle_scene() -> Scene3D {
    let mut scene = Scene3D::new();
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
    prim.joints = Some(vec![[0, 1, 2, 3], [1, 2, 3, 4], [2, 3, 4, 5]]);
    prim.weights = Some(vec![
        [0.4, 0.3, 0.2, 0.1],
        [0.5, 0.25, 0.15, 0.10],
        [0.7, 0.2, 0.05, 0.05],
    ]);
    let mesh = Mesh::new(Some("skinned_tri".to_string())).with_primitive(prim);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

/// Two-primitive mesh, each primitive a single triangle, with the
/// *first* and *third* vertices of the two triangles physically
/// identical. The OBJ encoder's vertex pool should collapse the four
/// shared positions across the two primitives into four `v` entries
/// total (not six).
fn two_primitives_sharing_vertices() -> Scene3D {
    let mut scene = Scene3D::new();
    let mut prim_a = Primitive::new(Topology::Triangles);
    prim_a.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let mut prim_b = Primitive::new(Topology::Triangles);
    // Shares vertices 0 and 2 with prim_a; introduces a new vertex
    // [1, 1, 0]. Six raw positions → four pooled `v` entries.
    prim_b.positions = vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
    let mesh = Mesh::new(Some("two_prims".to_string()))
        .with_primitive(prim_a)
        .with_primitive(prim_b);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

/// Two-primitive mesh with two *different* materials — exercises that
/// per-primitive material binding survives both OBJ (`usemtl`) and
/// gltf (per-primitive `material` index).
fn two_primitives_two_materials() -> Scene3D {
    let mut scene = Scene3D::new();
    let red = scene.add_material(
        Material::new()
            .with_name("red")
            .with_base_color([1.0, 0.0, 0.0, 1.0]),
    );
    let blue = scene.add_material(
        Material::new()
            .with_name("blue")
            .with_base_color([0.0, 0.0, 1.0, 1.0]),
    );
    let mut prim_red = Primitive::new(Topology::Triangles);
    prim_red.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim_red.material = Some(red);
    let mut prim_blue = Primitive::new(Topology::Triangles);
    prim_blue.positions = vec![[2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]];
    prim_blue.material = Some(blue);
    let mesh = Mesh::new(Some("painted".to_string()))
        .with_primitive(prim_red)
        .with_primitive(prim_blue);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

// ─────────── §1. skinning data primitive-level survival ────────────

#[test]
fn gltf_round_trip_preserves_per_vertex_joints_and_weights() {
    // gltf serialises JOINTS_0 + WEIGHTS_0 as accessor attributes; the
    // decoder reads them straight back into `Primitive::joints` /
    // `Primitive::weights`. Bit-exact survival expected.
    let scene = skinned_triangle_scene();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("gltf encode skinned");
    let round = GltfDecoder::new()
        .decode(&bytes)
        .expect("gltf decode skinned");
    let prim = &round.meshes[0].primitives[0];
    assert_eq!(
        prim.joints,
        Some(vec![[0, 1, 2, 3], [1, 2, 3, 4], [2, 3, 4, 5]]),
        "JOINTS_0 must survive bit-exact"
    );
    let weights = prim.weights.as_ref().expect("weights present after gltf");
    let expected = [
        [0.4f32, 0.3, 0.2, 0.1],
        [0.5, 0.25, 0.15, 0.10],
        [0.7, 0.2, 0.05, 0.05],
    ];
    assert_eq!(weights.len(), expected.len());
    for (got, want) in weights.iter().zip(expected.iter()) {
        for (g, w) in got.iter().zip(want.iter()) {
            assert!(
                (g - w).abs() < 1e-6,
                "WEIGHTS_0 mismatch: got {g} expected {w}"
            );
        }
    }
}

#[test]
fn stl_round_trip_drops_joints_and_weights() {
    // STL encodes only triangle position + normal; the encoder is
    // contractually allowed to drop joints/weights. The round-trip
    // must keep the *triangle* and lose the *skinning attributes*.
    let scene = skinned_triangle_scene();
    let bytes = StlEncoder::new_binary()
        .encode(&scene)
        .expect("STL encode skinned (positions only)");
    let round = StlDecoder::new()
        .decode(&bytes)
        .expect("STL decode skinned");
    let prim = &round.meshes[0].primitives[0];
    assert!(
        prim.joints.is_none(),
        "STL has no joints surface, decoded prim must be joints-less"
    );
    assert!(
        prim.weights.is_none(),
        "STL has no weights surface, decoded prim must be weights-less"
    );
    assert_eq!(round.triangle_count(), 1, "geometry survives");
}

#[test]
fn obj_round_trip_drops_joints_and_weights() {
    // OBJ Wavefront has no skinning channel; the encoder drops both.
    let scene = skinned_triangle_scene();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode skinned (positions only)");
    let round = ObjDecoder::new()
        .decode(&bytes)
        .expect("OBJ decode skinned");
    let prim = &round.meshes[0].primitives[0];
    assert!(
        prim.joints.is_none(),
        "OBJ has no joints surface; decoded prim must be joints-less"
    );
    assert!(
        prim.weights.is_none(),
        "OBJ has no weights surface; decoded prim must be weights-less"
    );
}

#[test]
fn gltf_round_trip_drops_scene_level_skin_array_in_v0_0_0() {
    // Pinning the *current* gltf 0.0.0 limitation: `Scene3D::skins` and
    // `Scene3D::skeletons` are not yet serialised by the encoder, so a
    // round-trip through the registry returns an empty array even when
    // the original scene had one. When gltf gains skin-array
    // serialisation (followup r5) this assertion flips to
    // `assert_eq!(round.skins.len(), 1)` in the same commit that
    // lifts the encoder.
    use oxideav_mesh3d::{Skeleton, Skin};
    let mut scene = skinned_triangle_scene();
    let joint = scene.add_node(Node::new().with_name("root_bone"));
    let mut sk = Skeleton::new();
    sk.joints = vec![joint];
    sk.inverse_bind_matrices = vec![[
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]];
    let sk_id = scene.add_skeleton(sk);
    scene.add_skin(Skin::new(sk_id));
    assert_eq!(scene.skins.len(), 1, "fixture seeded with one skin");

    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("encode");
    let round = GltfDecoder::new().decode(&bytes).expect("decode");
    // gltf r2 added skin + skeleton round-trip (commit 4b41f63);
    // tests/mesh3d_extras_and_skinning_coverage.rs pin flipped from
    // "drops" → "preserves" once the workspace patch unified the
    // format crates with local gltf source.
    assert_eq!(round.skins.len(), 1, "gltf preserves Scene3D::skins");
    assert_eq!(
        round.skeletons.len(),
        1,
        "gltf preserves Scene3D::skeletons"
    );
}

#[test]
fn gltf_round_trip_drops_scene_level_animation_array_in_v0_0_0() {
    // Same pin for animations. When the encoder gains animation
    // serialisation, this flips to `assert_eq!(round.animations.len(), 1)`.
    use oxideav_mesh3d::{
        Animation, AnimationChannel, AnimationProperty, AnimationSampler, AnimationTarget,
        AnimationValues, Interpolation,
    };
    let mut scene = skinned_triangle_scene();
    let joint = scene.add_node(Node::new().with_name("anim_target"));
    let mut anim = Animation::new(Some("wiggle".to_string()));
    anim.channels.push(AnimationChannel {
        target: AnimationTarget {
            node: joint,
            property: AnimationProperty::Translation,
        },
        sampler: AnimationSampler {
            keyframes: vec![0.0, 1.0],
            values: AnimationValues::Vec3(vec![[0.0; 3], [1.0, 0.0, 0.0]]),
            interpolation: Interpolation::Linear,
        },
    });
    scene.add_animation(anim);
    assert_eq!(scene.animations.len(), 1, "fixture seeded with one anim");

    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("encode");
    let round = GltfDecoder::new().decode(&bytes).expect("decode");
    // gltf r2 added animation round-trip — pin flipped to "preserves".
    assert_eq!(
        round.animations.len(),
        1,
        "gltf preserves Scene3D::animations"
    );
}

// ─────────── §2. multi-primitive vertex pool dedup (OBJ) ───────────

#[test]
fn obj_encoder_dedupes_shared_vertices_across_primitives() {
    // Two triangles, each as its own Primitive, sharing two vertex
    // positions. The OBJ encoder must intern into the global vertex
    // pool, so the output should carry exactly four `v` lines, not six.
    let scene = two_primitives_sharing_vertices();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode multi-prim shared vertices");
    let text = std::str::from_utf8(&bytes).expect("OBJ output is UTF-8");
    let v_lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            let mut iter = l.split_whitespace();
            matches!(iter.next(), Some("v"))
        })
        .collect();
    assert_eq!(
        v_lines.len(),
        4,
        "two prims × 3 verts = 6 raw, dedup should pool to 4 (got {} lines: {:?})",
        v_lines.len(),
        v_lines,
    );
}

#[test]
fn obj_round_trip_preserves_two_primitive_face_count() {
    // After encode → decode the scene should still describe two
    // triangles. (OBJ's two `usemtl` boundaries split them back into
    // two primitives only when materials differ; with no material here
    // the decoder collapses them into one primitive's element list.)
    let scene = two_primitives_sharing_vertices();
    let bytes = ObjEncoder::new().encode(&scene).expect("OBJ encode");
    let round = ObjDecoder::new().decode(&bytes).expect("OBJ decode");
    assert_eq!(
        round.triangle_count(),
        2,
        "two triangles survive the round-trip"
    );
}

#[test]
fn gltf_round_trip_keeps_two_primitives_distinct() {
    // gltf serialises every Primitive as a separate `mesh.primitives[i]`
    // entry, so the round-trip should preserve the per-primitive
    // partitioning even when no material distinguishes them.
    let scene = two_primitives_sharing_vertices();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("gltf encode");
    let round = GltfDecoder::new().decode(&bytes).expect("gltf decode");
    assert_eq!(
        round.meshes.len(),
        1,
        "single mesh survives (no extra splitting)"
    );
    assert_eq!(
        round.meshes[0].primitives.len(),
        2,
        "two primitives stay distinct in gltf"
    );
}

// ─────────── §3. multi-material binding ────────────────────────────

#[test]
fn obj_round_trip_preserves_per_primitive_material_binding() {
    // Two primitives bound to two materials. The OBJ encoder emits two
    // `usemtl` directives interleaved with the face elements; the
    // decoder splits the face stream on each `usemtl` boundary back
    // into two primitives, each carrying the matching `obj:usemtl`
    // extras key.
    let scene = two_primitives_two_materials();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode multi-material");
    let text = std::str::from_utf8(&bytes).expect("OBJ output is UTF-8");
    // The encoder emits one usemtl per primitive (state token ahead
    // of the face stream).
    let usemtl_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("usemtl ")).collect();
    assert_eq!(
        usemtl_lines.len(),
        2,
        "two materials → two usemtl directives (got {usemtl_lines:?})"
    );
    assert!(
        usemtl_lines.iter().any(|l| l == &"usemtl red"),
        "red usemtl present"
    );
    assert!(
        usemtl_lines.iter().any(|l| l == &"usemtl blue"),
        "blue usemtl present"
    );

    let round = ObjDecoder::new().decode(&bytes).expect("OBJ decode");
    let prims = &round.meshes[0].primitives;
    assert_eq!(prims.len(), 2, "two primitives split out by usemtl");
    let names: Vec<&str> = prims
        .iter()
        .filter_map(|p| p.extras.get("obj:usemtl").and_then(|v| v.as_str()))
        .collect();
    assert!(
        names.contains(&"red") && names.contains(&"blue"),
        "both material names round-trip via extras (got {names:?})"
    );
}

#[test]
fn gltf_round_trip_preserves_per_primitive_material_index() {
    // gltf carries `mesh.primitive.material: u32`; the round-trip
    // should preserve which material index each primitive points at.
    let scene = two_primitives_two_materials();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("gltf encode");
    let round = GltfDecoder::new().decode(&bytes).expect("gltf decode");
    assert_eq!(round.materials.len(), 2, "two materials survive distinctly");
    let prims = &round.meshes[0].primitives;
    assert_eq!(prims.len(), 2, "two primitives stay distinct");

    // Resolve material names through the index → catches a swap.
    let mut names: Vec<&str> = prims
        .iter()
        .map(|p| {
            let idx = p.material.expect("material binding survives").0 as usize;
            round.materials[idx].name.as_deref().unwrap_or("?")
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["blue", "red"]);
}

// ────────────── §4. extras side-channel preservation matrix ────────

#[test]
fn extras_audit_stl_to_stl_pins_stl_source_key() {
    // STL → STL: the decoder injects `stl:source = "binary"` (or
    // "ascii") into `Primitive::extras`; the encoder ignores the key
    // (no compatible re-emit), but a re-decode reinstates it. So
    // the key is *idempotent* across round-trips — present and equal
    // to "binary" both times if we entered binary, "ascii" both times
    // if we entered ascii.
    let mut typed = Scene3D::new();
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
    let mesh = Mesh::new(Some("t".to_string())).with_primitive(prim);
    let mid = typed.add_mesh(mesh);
    let nid = typed.add_node(Node::new().with_mesh(mid));
    typed.add_root(nid);

    // Binary STL.
    let bin = StlEncoder::new_binary().encode(&typed).expect("bin enc");
    let bin_round = StlDecoder::new().decode(&bin).expect("bin dec");
    assert_eq!(
        bin_round.meshes[0].primitives[0]
            .extras
            .get("stl:source")
            .and_then(|v| v.as_str()),
        Some("binary"),
        "binary STL round-trip stamps stl:source = binary"
    );

    // ASCII STL.
    let asc = StlEncoder::new_ascii().encode(&typed).expect("ascii enc");
    let asc_round = StlDecoder::new().decode(&asc).expect("ascii dec");
    assert_eq!(
        asc_round.meshes[0].primitives[0]
            .extras
            .get("stl:source")
            .and_then(|v| v.as_str()),
        Some("ascii"),
        "ASCII STL round-trip stamps stl:source = ascii"
    );
}

#[test]
fn extras_audit_stl_decoder_sets_zaxis_and_millimetres() {
    // STL decoder's authoring convention pin: the additive-mfg
    // toolchain treats every STL file as Z-up, millimetres. The
    // decoder must surface that metadata (downstream consumers
    // re-orient if needed).
    use oxideav_mesh3d::{Axis, Unit};
    let mut typed = Scene3D::new();
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
    let mesh = Mesh::new(Some("t".to_string())).with_primitive(prim);
    let mid = typed.add_mesh(mesh);
    let nid = typed.add_node(Node::new().with_mesh(mid));
    typed.add_root(nid);

    let bin = StlEncoder::new_binary().encode(&typed).expect("encode");
    let round = StlDecoder::new().decode(&bin).expect("decode");
    assert_eq!(round.up_axis, Axis::PosZ, "STL decoder convention");
    assert_eq!(round.unit, Unit::Millimetres, "STL decoder convention");
}

#[test]
fn extras_audit_obj_decoder_sets_yaxis_and_metres() {
    // OBJ has no unit metadata in-band; the decoder picks the safe
    // glTF defaults (Y-up, metres). Pin the contract.
    use oxideav_mesh3d::{Axis, Unit};
    let mut typed = Scene3D::new();
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let mesh = Mesh::new(Some("t".to_string())).with_primitive(prim);
    let mid = typed.add_mesh(mesh);
    let nid = typed.add_node(Node::new().with_mesh(mid));
    typed.add_root(nid);
    let bytes = ObjEncoder::new().encode(&typed).expect("encode");
    let round = ObjDecoder::new().decode(&bytes).expect("decode");
    assert_eq!(round.up_axis, Axis::PosY, "OBJ decoder default");
    assert_eq!(round.unit, Unit::Metres, "OBJ decoder default");
}

#[test]
fn extras_audit_gltf_to_obj_drops_scene_extras() {
    // Cross-format X→Y extras audit:
    //
    // gltf preserves arbitrary `Scene3D::extras` (it's a JSON object
    // on the root). OBJ has no scene-level free-form extras shape, so
    // a gltf → OBJ pass MUST drop scene extras. The geometry survives.
    let mut typed = Scene3D::new();
    typed.extras.insert(
        "custom_tag".into(),
        serde_json::Value::String("origin".into()),
    );
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let mesh = Mesh::new(Some("t".to_string())).with_primitive(prim);
    let mid = typed.add_mesh(mesh);
    let nid = typed.add_node(Node::new().with_mesh(mid));
    typed.add_root(nid);

    // gltf → OBJ.
    let gltf_bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&typed)
        .expect("gltf enc");
    let gltf_round = GltfDecoder::new().decode(&gltf_bytes).expect("gltf dec");
    assert_eq!(
        gltf_round.extras.get("custom_tag").and_then(|v| v.as_str()),
        Some("origin"),
        "gltf preserves scene extras",
    );
    let obj_bytes = ObjEncoder::new().encode(&gltf_round).expect("obj enc");
    let obj_round = ObjDecoder::new().decode(&obj_bytes).expect("obj dec");
    assert!(
        !obj_round.extras.contains_key("custom_tag"),
        "OBJ silently drops gltf scene extras (no scene-level free-form key surface)"
    );
}

#[test]
fn extras_audit_gltf_preserves_primitive_extras_round_trip() {
    // gltf prim.extras is JSON-encoded into the gltf spec's per-primitive
    // `extras` slot, so an arbitrary JSON value round-trips intact.
    let mut typed = Scene3D::new();
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim.extras.insert(
        "vendor:tag".into(),
        serde_json::json!({"hint": "blue", "weight": 0.5}),
    );
    let mesh = Mesh::new(Some("t".to_string())).with_primitive(prim);
    let mid = typed.add_mesh(mesh);
    let nid = typed.add_node(Node::new().with_mesh(mid));
    typed.add_root(nid);

    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&typed)
        .expect("gltf encode");
    let round = GltfDecoder::new().decode(&bytes).expect("gltf decode");
    let prim = &round.meshes[0].primitives[0];
    let v = prim.extras.get("vendor:tag").expect("extras key survives");
    assert_eq!(v.get("hint").and_then(|x| x.as_str()), Some("blue"));
    assert!(
        (v.get("weight").and_then(|x| x.as_f64()).unwrap_or(0.0) - 0.5).abs() < 1e-9,
        "numeric extras value round-trips",
    );
}
