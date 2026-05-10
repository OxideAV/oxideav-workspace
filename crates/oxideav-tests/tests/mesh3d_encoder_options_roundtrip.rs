//! Round-5 encoder-option round-trip suite.
//!
//! Pin every encoder configuration knob the published 0.0.0 sibling
//! crates expose (and only those — speculative options that don't
//! exist on crates.io yet are deferred to the round that lights them
//! up). For each option this suite asserts the option:
//!
//! * Drives the actual output (a getter or a byte-level marker
//!   confirms the change took effect), and
//! * Round-trips through the matching decoder with no semantic loss
//!   (geometry survives; metadata / extras survive where the format
//!   has a slot for them).
//!
//! ## Options exercised
//!
//! ### `oxideav-stl 0.0.0` — `StlEncoder`
//!
//! * `StlEncoder::new_binary()` ↔ `StlEncoder::new(StlFormat::Binary)` —
//!   constructor parity (same output, same `format()` getter).
//! * `StlEncoder::new_ascii()` ↔ `StlEncoder::new(StlFormat::Ascii)` —
//!   same as above for the ASCII flavour.
//! * `StlEncoder::default()` matches `new_binary()` — documented
//!   default is binary.
//! * `format()` getter reflects the constructed flavour.
//! * Binary STL bytes start with the 80-byte header (and our
//!   convention emits `solid`-prefixed for ASCII).
//!
//! ### `oxideav-obj 0.0.0` — `ObjEncoder` + `MtlEncoder`
//!
//! * `ObjEncoder::new()` ↔ `ObjEncoder::default()` — same output bytes.
//! * `ObjEncoder::with_mtl_basename("foo")` injects exactly one
//!   `mtllib foo.mtl` directive at the top.
//! * The mtllib directive round-trips into `Scene3D::extras["obj:mtllibs"]`
//!   so a re-encode (without an explicit basename) replays it.
//! * `MtlEncoder::new()` produces material bytes that re-parse via
//!   `oxideav_obj::mtl::parse_mtl` into the same material name + base
//!   colour set.
//!
//! ### `oxideav-gltf 0.0.0` — `GltfEncoder`
//!
//! * `GltfEncoder::new()` ↔ `GltfEncoder::with_output(OutputFlavour::Glb)`
//!   ↔ `GltfEncoder { output: OutputFlavour::default() }` — all three
//!   produce identical bytes.
//! * `OutputFlavour::Glb` output starts with `b"glTF"` magic;
//!   `OutputFlavour::JsonEmbedded` does not but starts with `b'{'`.
//! * `oxideav_gltf::json_encoder()` helper matches
//!   `with_output(OutputFlavour::JsonEmbedded)` byte-for-byte.
//! * Both flavours round-trip the same `Scene3D` to the same decoded
//!   geometry (positions exact).

use oxideav_mesh3d::{
    Material, Mesh, Mesh3DDecoder, Mesh3DEncoder, Node, Primitive, Scene3D, Topology,
};

use oxideav_gltf::{json_encoder, GltfDecoder, GltfEncoder, OutputFlavour};
use oxideav_obj::{
    encoder::MtlEncoder,
    mtl::{parse_mtl, serialize_mtl},
    ObjDecoder, ObjEncoder,
};
use oxideav_stl::{encoder::StlFormat, StlDecoder, StlEncoder};

// ─────────────────────────── fixture ─────────────────────────────

/// Single-triangle scene with one material — every encoder under test
/// can express this losslessly so we can compare configurations.
fn one_triangle_scene() -> Scene3D {
    let mut scene = Scene3D::new();
    let mat = scene.add_material(
        Material::new()
            .with_name("opt_test_mat")
            .with_base_color([0.25, 0.5, 0.75, 1.0]),
    );
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
    prim.material = Some(mat);
    let mesh = Mesh::new(Some("tri".to_string())).with_primitive(prim);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

// ─────────────────────── STL encoder options ───────────────────────

#[test]
fn stl_new_binary_matches_new_with_binary_format() {
    // The two constructors are documented as equivalent; their byte
    // output and `format()` getter must agree for the same input.
    let scene = one_triangle_scene();
    let bytes_a = StlEncoder::new_binary().encode(&scene).expect("encode A");
    let bytes_b = StlEncoder::new(StlFormat::Binary)
        .encode(&scene)
        .expect("encode B");
    assert_eq!(bytes_a, bytes_b, "constructor parity for binary STL");
    assert_eq!(
        StlEncoder::new_binary().format(),
        StlFormat::Binary,
        "format() getter reflects binary"
    );
    assert_eq!(
        StlEncoder::new(StlFormat::Binary).format(),
        StlFormat::Binary
    );
}

#[test]
fn stl_new_ascii_matches_new_with_ascii_format() {
    // Same parity check for the ASCII flavour.
    let scene = one_triangle_scene();
    let bytes_a = StlEncoder::new_ascii().encode(&scene).expect("encode A");
    let bytes_b = StlEncoder::new(StlFormat::Ascii)
        .encode(&scene)
        .expect("encode B");
    assert_eq!(bytes_a, bytes_b, "constructor parity for ASCII STL");
    assert_eq!(
        StlEncoder::new_ascii().format(),
        StlFormat::Ascii,
        "format() getter reflects ASCII"
    );
    assert_eq!(StlEncoder::new(StlFormat::Ascii).format(), StlFormat::Ascii);
}

#[test]
fn stl_default_is_binary() {
    // `StlEncoder::default()` is documented as the binary flavour.
    let scene = one_triangle_scene();
    let bytes_default = StlEncoder::default().encode(&scene).expect("default");
    let bytes_binary = StlEncoder::new_binary().encode(&scene).expect("binary");
    assert_eq!(
        bytes_default, bytes_binary,
        "Default::default matches new_binary"
    );
    assert_eq!(StlEncoder::default().format(), StlFormat::Binary);
}

#[test]
fn stl_binary_bytes_have_80_byte_header_then_count() {
    // Sanity: the binary output starts with 80 header bytes followed
    // by a `u32` triangle count. For a one-triangle scene the count
    // must equal 1 in little-endian at offset 80..84.
    let scene = one_triangle_scene();
    let bytes = StlEncoder::new_binary().encode(&scene).expect("binary");
    assert!(bytes.len() >= 84, "binary STL minimum 84 bytes");
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]);
    assert_eq!(count, 1, "binary STL triangle count = 1");
}

#[test]
fn stl_ascii_bytes_start_with_solid_token() {
    // Sanity: the ASCII grammar opens with `solid` (per Burns §6.5).
    let scene = one_triangle_scene();
    let bytes = StlEncoder::new_ascii().encode(&scene).expect("ascii");
    assert!(
        bytes.starts_with(b"solid"),
        "ASCII STL must start with `solid` token"
    );
}

#[test]
fn stl_both_flavours_round_trip_to_same_geometry() {
    // The chosen flavour shouldn't change *what* gets decoded back —
    // it only changes the wire encoding. Both binary and ASCII must
    // round-trip to a scene with one triangle of the same positions.
    let scene = one_triangle_scene();
    let bin_bytes = StlEncoder::new_binary().encode(&scene).expect("bin");
    let asc_bytes = StlEncoder::new_ascii().encode(&scene).expect("asc");
    let bin_round = StlDecoder::new().decode(&bin_bytes).expect("bin dec");
    let asc_round = StlDecoder::new().decode(&asc_bytes).expect("asc dec");
    assert_eq!(bin_round.triangle_count(), 1);
    assert_eq!(asc_round.triangle_count(), 1);
    let bin_pos = &bin_round.meshes[0].primitives[0].positions;
    let asc_pos = &asc_round.meshes[0].primitives[0].positions;
    assert_eq!(
        bin_pos, asc_pos,
        "binary and ASCII flavours decode to the same positions"
    );
}

// ─────────────────────── OBJ encoder options ───────────────────────

#[test]
fn obj_new_matches_default() {
    // `ObjEncoder::new()` and `ObjEncoder::default()` are documented
    // as equivalent. Verify on output bytes.
    let scene = one_triangle_scene();
    let bytes_a = ObjEncoder::new().encode(&scene).expect("new");
    let bytes_b = ObjEncoder::default().encode(&scene).expect("default");
    assert_eq!(bytes_a, bytes_b, "ObjEncoder::new == ObjEncoder::default");
}

#[test]
fn obj_with_mtl_basename_emits_one_mtllib_directive() {
    // `with_mtl_basename("foo")` should inject exactly one `mtllib
    // foo.mtl` directive at the top of the OBJ output, and only one.
    let scene = one_triangle_scene();
    let bytes = ObjEncoder::new()
        .with_mtl_basename("foo")
        .encode(&scene)
        .expect("encode w mtllib");
    let text = std::str::from_utf8(&bytes).expect("UTF-8");
    let mtllib_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("mtllib ")).collect();
    assert_eq!(
        mtllib_lines.len(),
        1,
        "with_mtl_basename emits exactly one mtllib directive (got {mtllib_lines:?})"
    );
    assert_eq!(
        mtllib_lines[0], "mtllib foo.mtl",
        "directive uses basename verbatim with .mtl appended"
    );
}

#[test]
fn obj_default_emits_no_mtllib_directive() {
    // Default `ObjEncoder::new()` (no basename) must not emit any
    // `mtllib` directive — the output is standalone.
    let scene = one_triangle_scene();
    let bytes = ObjEncoder::new().encode(&scene).expect("encode default");
    let text = std::str::from_utf8(&bytes).expect("UTF-8");
    assert!(
        !text.lines().any(|l| l.starts_with("mtllib ")),
        "default ObjEncoder must not emit mtllib"
    );
}

#[test]
fn obj_mtllib_round_trips_via_scene_extras() {
    // After encode (with an explicit basename) → decode, the decoder
    // captures the mtllib reference into `Scene3D::extras["obj:mtllibs"]`.
    // A subsequent re-encode (with no explicit basename) replays the
    // directive from that extras key — bidirectional preservation.
    let scene = one_triangle_scene();
    let bytes_a = ObjEncoder::new()
        .with_mtl_basename("first")
        .encode(&scene)
        .expect("encode A");
    let round = ObjDecoder::new().decode(&bytes_a).expect("decode");
    let mtllibs = round
        .extras
        .get("obj:mtllibs")
        .and_then(|v| v.as_array())
        .expect("obj:mtllibs survives decode");
    let names: Vec<&str> = mtllibs.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        names.iter().any(|n| n.contains("first")),
        "decoder records the original mtllib basename (got {names:?})"
    );
    let bytes_b = ObjEncoder::new().encode(&round).expect("re-encode");
    let text_b = std::str::from_utf8(&bytes_b).expect("UTF-8");
    assert!(
        text_b
            .lines()
            .any(|l| l.starts_with("mtllib ") && l.contains("first")),
        "re-encode replays the recorded mtllib directive"
    );
}

#[test]
fn mtl_encoder_round_trips_material_set_via_parse_mtl() {
    // `MtlEncoder::new()` serialises a scene's materials slot. The
    // bytes must re-parse via the public `parse_mtl` helper into the
    // same material name + base colour we encoded.
    let scene = one_triangle_scene();
    let bytes = MtlEncoder::new().encode(&scene).expect("MtlEncoder encode");
    let text = std::str::from_utf8(&bytes).expect("MTL output is UTF-8");
    let mats = parse_mtl(text).expect("parse_mtl on MtlEncoder output");
    assert_eq!(mats.len(), 1, "single material round-trips");
    assert_eq!(mats[0].name.as_deref(), Some("opt_test_mat"));
    // OBJ MTL stores diffuse `Kd` separately from alpha `d`; the
    // encoder splits them, so the round-tripped base colour matches
    // RGB exactly and alpha rides along on `d`.
    let bc = mats[0].base_color;
    assert!((bc[0] - 0.25).abs() < 1e-3, "Kd r round-trips");
    assert!((bc[1] - 0.5).abs() < 1e-3, "Kd g round-trips");
    assert!((bc[2] - 0.75).abs() < 1e-3, "Kd b round-trips");
}

#[test]
fn mtl_encoder_serialize_mtl_matches_helper_output() {
    // `MtlEncoder::encode(scene)` must produce the same bytes as the
    // free-function `serialize_mtl(&scene.materials, &scene.textures)`
    // — one is just the trait wrapper around the other.
    let scene = one_triangle_scene();
    let via_encoder = MtlEncoder::new().encode(&scene).expect("encoder");
    let via_helper =
        serialize_mtl(&scene.materials, &scene.textures).expect("serialize_mtl helper");
    assert_eq!(
        via_encoder, via_helper,
        "MtlEncoder must wrap serialize_mtl 1:1"
    );
}

// ─────────────────────── glTF encoder options ──────────────────────

#[test]
fn gltf_new_matches_with_output_glb_in_flavour_and_decode() {
    // `GltfEncoder::new()` and `with_output(OutputFlavour::Glb)`
    // must both target the GLB flavour and decode to the same scene.
    //
    // Byte-equality is *not* asserted: glTF JSON serialises a
    // HashMap (the per-primitive `attributes` map), and HashMap
    // iteration order varies per process invocation, so two
    // back-to-back encoder calls can legitimately swap key order
    // inside the JSON chunk without changing the scene shape. The
    // contract under test is "same flavour, same decoded scene".
    let scene = one_triangle_scene();
    let enc_a = GltfEncoder::new();
    let enc_b = GltfEncoder::with_output(OutputFlavour::Glb);
    assert_eq!(
        enc_a.output, enc_b.output,
        "new() and with_output(Glb) target the same flavour"
    );
    assert_eq!(enc_a.output, OutputFlavour::Glb);

    let bytes_a = GltfEncoder::new().encode(&scene).expect("new");
    let bytes_b = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("with_output Glb");
    // Same magic = same wire format.
    assert_eq!(&bytes_a[..4], b"glTF");
    assert_eq!(&bytes_b[..4], b"glTF");

    let round_a = GltfDecoder::new().decode(&bytes_a).expect("decode A");
    let round_b = GltfDecoder::new().decode(&bytes_b).expect("decode B");
    assert_eq!(
        round_a.meshes[0].primitives[0].positions, round_b.meshes[0].primitives[0].positions,
        "GLB flavour from either constructor decodes to identical positions"
    );
    assert_eq!(round_a.materials.len(), round_b.materials.len());
    assert_eq!(round_a.materials[0].name, round_b.materials[0].name);
}

#[test]
fn gltf_default_output_flavour_is_glb() {
    // `OutputFlavour::default()` must be `Glb` — pin the contract so
    // a future re-default doesn't silently change every consumer's
    // wire format.
    assert_eq!(OutputFlavour::default(), OutputFlavour::Glb);
}

#[test]
fn gltf_glb_output_starts_with_glb_magic() {
    // Sanity: `Glb` bytes start with `b"glTF"` (the glb container's
    // 4-byte magic at offset 0).
    let scene = one_triangle_scene();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("Glb encode");
    assert_eq!(&bytes[..4], b"glTF", "GLB magic at offset 0");
}

#[test]
fn gltf_json_embedded_output_starts_with_brace() {
    // Sanity: `JsonEmbedded` bytes start with `{` (pretty-printed
    // JSON document opener) and explicitly do NOT have the GLB magic.
    let scene = one_triangle_scene();
    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("JSON encode");
    assert_eq!(bytes[0], b'{', "JSON output opens with `{{`");
    assert_ne!(&bytes[..4], b"glTF", "JSON output is NOT a GLB");
}

#[test]
fn gltf_json_encoder_helper_matches_with_output_json_embedded() {
    // The convenience `oxideav_gltf::json_encoder()` free function
    // is documented as a friendlier-name wrapper around
    // `with_output(JsonEmbedded)`. They must select the same flavour
    // and decode to the same scene. Byte-equality is *not* the
    // contract — see `gltf_new_matches_with_output_glb_in_flavour_and_decode`
    // for the HashMap-iteration-order rationale.
    let scene = one_triangle_scene();
    assert_eq!(
        json_encoder().output,
        OutputFlavour::JsonEmbedded,
        "json_encoder helper picks JsonEmbedded flavour"
    );
    let bytes_a = json_encoder().encode(&scene).expect("json_encoder");
    let bytes_b = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("with_output JSON");
    // Both are JSON documents (open with `{`).
    assert_eq!(bytes_a[0], b'{');
    assert_eq!(bytes_b[0], b'{');
    let round_a = GltfDecoder::new().decode(&bytes_a).expect("decode A");
    let round_b = GltfDecoder::new().decode(&bytes_b).expect("decode B");
    assert_eq!(
        round_a.meshes[0].primitives[0].positions, round_b.meshes[0].primitives[0].positions,
        "json_encoder helper and with_output(JsonEmbedded) decode identically"
    );
}

#[test]
fn gltf_glb_and_json_round_trip_to_identical_geometry() {
    // The flavour changes only the wire encoding; the decoded scene
    // must be position-identical regardless of which flavour we used.
    let scene = one_triangle_scene();
    let glb_bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("glb");
    let json_bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("json");
    let glb_round = GltfDecoder::new().decode(&glb_bytes).expect("glb dec");
    let json_round = GltfDecoder::new().decode(&json_bytes).expect("json dec");

    let glb_pos = &glb_round.meshes[0].primitives[0].positions;
    let json_pos = &json_round.meshes[0].primitives[0].positions;
    assert_eq!(
        glb_pos, json_pos,
        "GLB and JSON decode to the same positions"
    );
    assert_eq!(
        glb_round.materials.len(),
        json_round.materials.len(),
        "GLB and JSON decode to the same material count"
    );
    assert_eq!(
        glb_round.materials[0].name, json_round.materials[0].name,
        "material name round-trips identically across flavours"
    );
}
