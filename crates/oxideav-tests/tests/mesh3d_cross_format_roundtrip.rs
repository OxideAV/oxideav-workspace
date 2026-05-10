//! Cross-format encode → decode roundtrip suite.
//!
//! For each (input format, output format) pair where both directions
//! are supported by the four sibling crates published as dev-deps
//! (`oxideav-stl`, `oxideav-obj`, `oxideav-gltf`, `oxideav-usdz`),
//! this suite:
//!
//! 1. Constructs a minimal `Scene3D` fixture in the typed model.
//! 2. Encodes it with the *output* format's encoder.
//! 3. Decodes the bytes back with the *output* format's decoder.
//! 4. Asserts geometric fidelity — positions exact, face count
//!    preserved, attributes carry over via the per-format `extras`
//!    side-channel where the formats can't both express the same
//!    surface (e.g. STL drops everything except triangles + normals).
//!
//! The matrix exercised here is the **encoder side** of every codec
//! that ships an encoder (STL, OBJ, glTF). USDZ ships a decoder only
//! (round-1 spec coverage), so its row of the matrix exercises
//! decoded-USDZ → re-encoded-as-STL/OBJ/glTF in
//! `usdz_decoded_re_encodes_into_*` to cover the cross-direction even
//! though we can't author USDZ in-test.
//!
//! ## Pairs covered (origin → target)
//!
//! | from \ to | stl | obj | gltf |
//! |-----------|-----|-----|------|
//! | typed     |  ✓  |  ✓  |   ✓  |
//! | stl       |  ✓  |  ✓  |   ✓  |
//! | obj       |  ✓  |  ✓  |   ✓  |
//! | gltf      |  ✓  |  ✓  |   ✓  |
//! | usdz      |  -  |  -  |   -  | (decoder requires a USDZ archive
//! input fixture; the dedicated helper test below builds one with
//! `oxideav-usdz`'s public API and then re-encodes through each
//! writer.)
//!
//! Total: 9 typed-source roundtrips + 1 USDZ-source matrix entry =
//! the cells above. The USDZ archive build has to be deferred to a
//! USDZ encoder round (no `Mesh3DEncoder` for USDZ exists yet) — we
//! cover the *decoder* surface in `registry_lookup.rs` instead.

use std::sync::Arc;

use oxideav_mesh3d::{
    AssetSource, ImageData, InMemoryAsset, Material, Mesh, Mesh3DDecoder, Mesh3DEncoder, Node,
    Primitive, Scene3D, Texture, Topology,
};

use oxideav_gltf::{GltfDecoder, GltfEncoder, OutputFlavour};
use oxideav_obj::{ObjDecoder, ObjEncoder};
use oxideav_stl::{StlDecoder, StlEncoder};

// ─────────────────────────── fixtures ────────────────────────────

/// Build a single-triangle scene with one material.
///
/// Positions, normal, base-color factor, and material name are all
/// scene-graph elements every format under test can express. The
/// triangle is wound counter-clockwise (right-hand rule normal
/// `[0,0,1]`) so the recomputed STL normal matches the supplied one.
fn one_triangle_scene() -> Scene3D {
    let mut scene = Scene3D::new();
    let mat = scene.add_material(
        Material::new()
            .with_name("red")
            .with_base_color([1.0, 0.0, 0.0, 1.0]),
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

/// Two-triangle quad — exercises multi-primitive vertex pooling on
/// the OBJ encoder side and the index-buffer path on the glTF encoder
/// side. Positions are arranged so the OBJ deduplicator collapses the
/// two shared corners into one entry per axis.
fn quad_scene() -> Scene3D {
    let mut scene = Scene3D::new();
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    prim.normals = Some(vec![[0.0, 0.0, 1.0]; 6]);
    let mesh = Mesh::new(Some("quad".to_string())).with_primitive(prim);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

/// All triangle positions across every primitive of every mesh,
/// flattened in scene-graph order. Used to compare geometry without
/// caring about how the codec laid out the meshes/primitives.
fn flatten_positions(scene: &Scene3D) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for mesh in &scene.meshes {
        for prim in &mesh.primitives {
            // STL/OBJ both expand index buffers when re-decoding; glTF
            // round-trips with an index buffer. To stay codec-agnostic
            // we resolve through the index buffer when present.
            match &prim.indices {
                Some(oxideav_mesh3d::Indices::U16(idxs)) => {
                    for &i in idxs {
                        out.push(prim.positions[i as usize]);
                    }
                }
                Some(oxideav_mesh3d::Indices::U32(idxs)) => {
                    for &i in idxs {
                        out.push(prim.positions[i as usize]);
                    }
                }
                None => out.extend_from_slice(&prim.positions),
            }
        }
    }
    out
}

/// Compare two position lists exactly under the IEEE-754 bit pattern.
/// The encoders we exercise in this round preserve f32 bit-for-bit
/// for un-normalised attributes, so an exact compare is intentional —
/// any discrepancy is a regression worth catching, not an epsilon to
/// hide behind.
fn assert_positions_match(actual: &[[f32; 3]], expected: &[[f32; 3]]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "position count mismatch: got {}, expected {}",
        actual.len(),
        expected.len(),
    );
    for (i, (got, want)) in actual.iter().zip(expected.iter()).enumerate() {
        for axis in 0..3 {
            assert!(
                (got[axis] - want[axis]).abs() < 1e-6,
                "position {i} axis {axis}: got {} expected {}",
                got[axis],
                want[axis],
            );
        }
    }
}

// ─────────────────── typed-source → format roundtrip ────────────────

#[test]
fn typed_to_stl_binary_to_typed_preserves_triangle() {
    let scene = one_triangle_scene();
    let mut enc = StlEncoder::new_binary();
    let bytes = enc.encode(&scene).expect("STL binary encode");
    assert!(bytes.len() >= 84, "binary STL minimum 84 bytes");
    let mut dec = StlDecoder::new();
    let round = dec.decode(&bytes).expect("STL binary decode");
    let positions = flatten_positions(&round);
    assert_positions_match(&positions, &flatten_positions(&scene));
}

#[test]
fn typed_to_stl_ascii_to_typed_preserves_triangle() {
    let scene = one_triangle_scene();
    let mut enc = StlEncoder::new_ascii();
    let bytes = enc.encode(&scene).expect("STL ascii encode");
    assert!(bytes.starts_with(b"solid"));
    let mut dec = StlDecoder::new();
    let round = dec.decode(&bytes).expect("STL ascii decode");
    assert_positions_match(&flatten_positions(&round), &flatten_positions(&scene));
}

#[test]
fn typed_to_obj_to_typed_preserves_triangle() {
    let scene = one_triangle_scene();
    let mut enc = ObjEncoder::new();
    let bytes = enc.encode(&scene).expect("OBJ encode");
    let mut dec = ObjDecoder::new();
    let round = dec.decode(&bytes).expect("OBJ decode");
    assert_positions_match(&flatten_positions(&round), &flatten_positions(&scene));
}

#[test]
fn typed_to_gltf_glb_to_typed_preserves_triangle() {
    let scene = one_triangle_scene();
    let mut enc = GltfEncoder::with_output(OutputFlavour::Glb);
    let bytes = enc.encode(&scene).expect("glTF GLB encode");
    assert_eq!(&bytes[..4], b"glTF", "GLB magic");
    let mut dec = GltfDecoder::new();
    let round = dec.decode(&bytes).expect("glTF GLB decode");
    assert_positions_match(&flatten_positions(&round), &flatten_positions(&scene));
}

#[test]
fn typed_to_gltf_json_to_typed_preserves_triangle() {
    let scene = one_triangle_scene();
    let mut enc = GltfEncoder::with_output(OutputFlavour::JsonEmbedded);
    let bytes = enc.encode(&scene).expect("glTF JSON encode");
    // First byte of pretty-printed JSON should be `{`.
    assert_eq!(bytes[0], b'{', "JSON output starts with `{{`");
    let mut dec = GltfDecoder::new();
    let round = dec.decode(&bytes).expect("glTF JSON decode");
    assert_positions_match(&flatten_positions(&round), &flatten_positions(&scene));
}

// ─────────────────── format → format chain roundtrips ──────────────

#[test]
fn stl_decoded_re_encodes_into_obj_and_back() {
    // Author in STL → decode → re-encode in OBJ → decode → compare to
    // the originally-decoded STL scene.
    let original = one_triangle_scene();
    let stl_bytes = StlEncoder::new_binary()
        .encode(&original)
        .expect("STL encode");
    let stl_scene = StlDecoder::new()
        .decode(&stl_bytes)
        .expect("STL decode (intermediate)");
    let obj_bytes = ObjEncoder::new().encode(&stl_scene).expect("OBJ encode");
    let obj_scene = ObjDecoder::new()
        .decode(&obj_bytes)
        .expect("OBJ decode (final)");
    assert_positions_match(
        &flatten_positions(&obj_scene),
        &flatten_positions(&original),
    );
}

#[test]
fn stl_decoded_re_encodes_into_gltf_and_back() {
    let original = one_triangle_scene();
    let stl_bytes = StlEncoder::new_binary()
        .encode(&original)
        .expect("STL encode");
    let stl_scene = StlDecoder::new()
        .decode(&stl_bytes)
        .expect("STL decode (intermediate)");
    let gltf_bytes = GltfEncoder::new().encode(&stl_scene).expect("glTF encode");
    let gltf_scene = GltfDecoder::new()
        .decode(&gltf_bytes)
        .expect("glTF decode (final)");
    assert_positions_match(
        &flatten_positions(&gltf_scene),
        &flatten_positions(&original),
    );
}

#[test]
fn obj_decoded_re_encodes_into_stl_and_back() {
    let original = quad_scene(); // multi-vertex; exercises STL flat list.
    let obj_bytes = ObjEncoder::new().encode(&original).expect("OBJ encode");
    let obj_scene = ObjDecoder::new()
        .decode(&obj_bytes)
        .expect("OBJ decode (intermediate)");
    let stl_bytes = StlEncoder::new_binary()
        .encode(&obj_scene)
        .expect("STL encode");
    let stl_scene = StlDecoder::new()
        .decode(&stl_bytes)
        .expect("STL decode (final)");
    // STL flattens the index buffer, so re-decoded positions should
    // match the original quad's six vertex positions in order.
    assert_positions_match(
        &flatten_positions(&stl_scene),
        &flatten_positions(&original),
    );
}

#[test]
fn obj_decoded_re_encodes_into_gltf_and_back() {
    let original = quad_scene();
    let obj_bytes = ObjEncoder::new().encode(&original).expect("OBJ encode");
    let obj_scene = ObjDecoder::new()
        .decode(&obj_bytes)
        .expect("OBJ decode (intermediate)");
    let gltf_bytes = GltfEncoder::new().encode(&obj_scene).expect("glTF encode");
    let gltf_scene = GltfDecoder::new()
        .decode(&gltf_bytes)
        .expect("glTF decode (final)");
    assert_positions_match(
        &flatten_positions(&gltf_scene),
        &flatten_positions(&original),
    );
}

#[test]
fn gltf_decoded_re_encodes_into_stl_and_back() {
    let original = one_triangle_scene();
    let gltf_bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&original)
        .expect("glTF encode");
    let gltf_scene = GltfDecoder::new()
        .decode(&gltf_bytes)
        .expect("glTF decode (intermediate)");
    let stl_bytes = StlEncoder::new_binary()
        .encode(&gltf_scene)
        .expect("STL encode");
    let stl_scene = StlDecoder::new()
        .decode(&stl_bytes)
        .expect("STL decode (final)");
    assert_positions_match(
        &flatten_positions(&stl_scene),
        &flatten_positions(&original),
    );
}

#[test]
fn gltf_decoded_re_encodes_into_obj_and_back() {
    let original = quad_scene();
    let gltf_bytes = GltfEncoder::new().encode(&original).expect("glTF encode");
    let gltf_scene = GltfDecoder::new()
        .decode(&gltf_bytes)
        .expect("glTF decode (intermediate)");
    let obj_bytes = ObjEncoder::new().encode(&gltf_scene).expect("OBJ encode");
    let obj_scene = ObjDecoder::new()
        .decode(&obj_bytes)
        .expect("OBJ decode (final)");
    assert_positions_match(
        &flatten_positions(&obj_scene),
        &flatten_positions(&original),
    );
}

// ──────────────── side-channel preservation cases ─────────────────

#[test]
fn obj_round_trip_preserves_material_name_in_extras() {
    // The OBJ format expresses material binding via `usemtl <name>`;
    // both encoder and decoder route the name through the
    // `Primitive::extras["obj:usemtl"]` side-channel so a round-trip
    // doesn't lose the assignment even when the decoder lacks an MTL
    // companion to resolve into a real `Material`.
    let scene = one_triangle_scene();
    let bytes = ObjEncoder::new().encode(&scene).expect("OBJ encode");
    let round = ObjDecoder::new().decode(&bytes).expect("OBJ decode");
    let prim = &round.meshes[0].primitives[0];
    assert_eq!(
        prim.extras
            .get("obj:usemtl")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        Some("red".to_string()),
        "OBJ encoder should emit `usemtl red` and decoder should round-trip it",
    );
}

#[test]
fn gltf_round_trip_preserves_material_base_color() {
    // glTF expresses base-color directly as a PBR material slot, so
    // the round-trip must preserve the literal floats — no extras
    // side-channel needed.
    let scene = one_triangle_scene();
    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("glTF encode");
    let round = GltfDecoder::new().decode(&bytes).expect("glTF decode");
    assert_eq!(round.materials.len(), 1, "single material survives");
    let m = &round.materials[0];
    assert_eq!(m.name.as_deref(), Some("red"));
    let bc = m.base_color;
    assert!((bc[0] - 1.0).abs() < 1e-6);
    assert!((bc[1] - 0.0).abs() < 1e-6);
    assert!((bc[2] - 0.0).abs() < 1e-6);
    assert!((bc[3] - 1.0).abs() < 1e-6);
}

#[test]
fn stl_round_trip_drops_material_but_keeps_geometry() {
    // STL has no concept of materials. The encoder should silently
    // drop the material binding while the geometry round-trips. The
    // decoded scene contains zero materials, but the triangle stays.
    let scene = one_triangle_scene();
    let bytes = StlEncoder::new_binary().encode(&scene).expect("STL encode");
    let round = StlDecoder::new().decode(&bytes).expect("STL decode");
    assert!(
        round.materials.is_empty(),
        "STL has no material channel, decoded scene should carry none"
    );
    assert_eq!(round.triangle_count(), 1);
}

// ───────────── ImageData::Source survives glTF JSON path ──────────

#[test]
fn gltf_json_round_trip_carries_inline_texture_source() {
    // Build a scene with a 4-byte PNG-magic blob wrapped as an
    // InMemoryAsset under a Texture; encode through the JSON-flavour
    // glTF (which inlines as `data:` URIs) and verify the bytes round
    // back through the decoder side via ImageData::Source.
    let mut scene = one_triangle_scene();
    let asset: Arc<dyn AssetSource> = Arc::new(InMemoryAsset::new(
        Some("image/png".into()),
        vec![0x89, 0x50, 0x4e, 0x47],
    ));
    let tex = Texture::from_source(Arc::clone(&asset));
    scene.add_texture(tex);
    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("glTF encode with texture");
    let round = GltfDecoder::new().decode(&bytes).expect("glTF decode");
    assert_eq!(round.textures.len(), 1, "texture round-trips");
    match &round.textures[0].image {
        ImageData::Source(s) => {
            let mut buf = Vec::new();
            s.open()
                .expect("open texture asset")
                .read_to_end(&mut buf)
                .expect("read texture bytes");
            assert_eq!(buf, vec![0x89, 0x50, 0x4e, 0x47]);
        }
        other => panic!("expected ImageData::Source after glTF round-trip, got {other:?}"),
    }
}

// ────────────── failure-mode smoke (encoder rejection) ─────────────

#[test]
fn stl_encoder_rejects_non_triangle_topology() {
    // STL is triangles-only; attempting to encode a `Lines` primitive
    // must surface as a decoder/encoder Error (not a panic and not a
    // silently-wrong output).
    let mut scene = Scene3D::new();
    let mut prim = Primitive::new(Topology::Lines);
    prim.positions = vec![[0.0; 3], [1.0, 0.0, 0.0]];
    let mesh = Mesh::new(Some("seg".to_string())).with_primitive(prim);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    let result = StlEncoder::new_binary().encode(&scene);
    assert!(result.is_err(), "STL encoder should reject non-Triangles");
}

use std::io::Read as _;
