//! mesh3d ↔ glTF / USDZ round-trips on the 0.0.5 typed-model surface.
//!
//! `oxideav-mesh3d` 0.0.5 (released 2026-08-15) added node-level
//! morph-weight overrides (`Node::weights` + the §3.7.4 precedence
//! chain), the `Primitive::morphed` / `Mesh::morphed` static folds,
//! the typed `KHR_texture_transform` surface on `TextureRef`, and the
//! `Option`-shaped sampler filters. This suite proves those surfaces
//! survive the two richest codecs — `oxideav-gltf` and
//! `oxideav-usdz` — and their composition.
//!
//! Deliberately 0.0.5-compatible: everything here builds through the
//! released constructors/builders (no `[Unreleased]` master-only API
//! such as in-between shapes or `MorphTarget::with_deltas`), so the
//! suite holds across the release boundary.

use std::sync::Arc;

use oxideav_gltf::{GltfDecoder, GltfEncoder, OutputFlavour};
use oxideav_mesh3d::{
    AssetSource, InMemoryAsset, Material, Mesh, Mesh3DDecoder, Mesh3DEncoder, MorphTarget, Node,
    Primitive, Scene3D, Texture, TextureRef, TextureTransform, Topology, WrapMode,
};
use oxideav_usdz::{UsdzDecoder, UsdzEncoder};

/// Base triangle + one morph target (pure position deltas), one mesh
/// shared by two nodes: node "plain" uses the mesh default weights,
/// node "override" carries a `Node::weights` override.
fn morph_scene() -> Scene3D {
    let mut scene = Scene3D::new();
    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
    let mut target = MorphTarget::default();
    target.position = Some(vec![[0.0, 0.0, 1.0], [0.0, 0.0, 0.5], [0.0, 0.0, 0.25]]);
    prim.targets = vec![target];
    let mut mesh = Mesh::new(Some("morphed-tri".to_string())).with_primitive(prim);
    mesh.weights = vec![0.25];
    let mid = scene.add_mesh(mesh);
    let plain = scene.add_node(Node::new().with_name("plain").with_mesh(mid));
    let overridden = scene.add_node(
        Node::new()
            .with_name("override")
            .with_mesh(mid)
            .with_weights([0.75]),
    );
    scene.add_root(plain);
    scene.add_root(overridden);
    scene
}

/// A textured triangle whose base-color reference carries a
/// `KHR_texture_transform` and whose sampler keeps the glTF
/// "filters undefined" state (0.0.5's `Option`-shaped filters).
fn textured_scene() -> Scene3D {
    let mut scene = Scene3D::new();
    let asset: Arc<dyn AssetSource> = Arc::new(InMemoryAsset::new(
        Some("image/png".into()),
        vec![0x89, 0x50, 0x4e, 0x47],
    ));
    let mut tex = Texture::from_source(Arc::clone(&asset));
    // mag/min stay None (undefined per §3.8.4.1); wraps explicit.
    tex.sampler = tex
        .sampler
        .with_wrap(WrapMode::MirroredRepeat, WrapMode::ClampToEdge);
    let tid = scene.add_texture(tex);

    let transform = TextureTransform::default()
        .with_offset([0.25, 0.5])
        .with_rotation(0.5)
        .with_scale([2.0, 1.0]);
    let mut material = Material::new().with_name("uv-transformed");
    material.base_color_texture = Some(TextureRef::new(tid).with_transform(transform));
    let mat = scene.add_material(material);

    let mut prim = Primitive::new(Topology::Triangles);
    prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    prim.normals = Some(vec![[0.0, 0.0, 1.0]; 3]);
    prim.uvs = vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]];
    prim.material = Some(mat);
    let mesh = Mesh::new(Some("textured-tri".to_string())).with_primitive(prim);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

/// World-space positions of the (single-primitive) mesh instantiated
/// at `node`, blended through the §3.7.4 weight-precedence chain.
fn world_positions(scene: &Scene3D, node: oxideav_mesh3d::NodeId) -> Vec<[f32; 3]> {
    scene
        .world_mesh(node)
        .expect("node instantiates a mesh")
        .primitives[0]
        .positions
        .clone()
}

fn assert_positions_eq(got: &[[f32; 3]], want: &[[f32; 3]], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: vertex count");
    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        for a in 0..3 {
            assert!(
                (g[a] - w[a]).abs() < 1e-6,
                "{what}: vertex {i} axis {a}: {} vs {}",
                g[a],
                w[a]
            );
        }
    }
}

/// glTF round-trip of the node-weight override: `node.weights` is
/// first-class glTF 2.0, so the override must come back typed and the
/// whole precedence chain must re-resolve identically.
#[test]
fn gltf_roundtrip_carries_node_weight_override() {
    let scene = morph_scene();
    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("glTF encode");
    let round = GltfDecoder::new().decode(&bytes).expect("glTF decode");

    // Both instances survive with their weight state distinguished.
    let overridden: Vec<&Node> = round
        .nodes
        .iter()
        .filter(|n| !n.weights.is_empty())
        .collect();
    assert_eq!(overridden.len(), 1, "exactly one node keeps an override");
    assert_eq!(overridden[0].weights, vec![0.75]);

    // The precedence chain re-resolves: mesh default vs node override.
    let mut resolved: Vec<Vec<f32>> = round
        .roots
        .iter()
        .map(|&n| {
            round
                .effective_morph_weights(n)
                .expect("chain resolves")
                .to_vec()
        })
        .collect();
    resolved.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());
    assert_eq!(resolved, vec![vec![0.25], vec![0.75]]);

    // Instantiated geometry matches the source scene per instance.
    let src = morph_scene();
    for (src_root, round_root) in src.roots.iter().zip(round.roots.iter()) {
        assert_positions_eq(
            &world_positions(&round, *round_root),
            &world_positions(&src, *src_root),
            "glTF world positions",
        );
    }
}

/// The 0.0.5 static fold agrees with the instantiation pipeline: for
/// a node sitting at the origin, `Mesh::morphed(override)` equals
/// `world_mesh` of the overridden instance — before and after the
/// glTF round-trip.
#[test]
fn morphed_static_fold_matches_instantiation() {
    let scene = morph_scene();
    let folded = scene.meshes[0].morphed(&[0.75]);
    assert!(
        folded.primitives[0].targets.is_empty(),
        "the fold consumes the morph roster"
    );
    let overridden = scene
        .roots
        .iter()
        .copied()
        .find(|&n| scene.effective_morph_weights(n) == Some(&[0.75][..]))
        .expect("override node present");
    assert_positions_eq(
        &folded.primitives[0].positions,
        &world_positions(&scene, overridden),
        "static fold vs instantiation",
    );

    // A folded mesh is plain geometry every codec can carry: push it
    // through glTF and confirm the baked positions survive.
    let mut baked = Scene3D::new();
    let mid = baked.add_mesh(folded);
    let nid = baked.add_node(Node::new().with_mesh(mid));
    baked.add_root(nid);
    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&baked)
        .expect("glTF encode");
    let round = GltfDecoder::new().decode(&bytes).expect("glTF decode");
    assert_positions_eq(
        &round.meshes[0].primitives[0].positions,
        &baked.meshes[0].primitives[0].positions,
        "baked positions through glTF",
    );
}

/// glTF round-trip of the typed `KHR_texture_transform` and the
/// `Option`-shaped sampler filters: the declared transform comes back
/// exactly, and "filters undefined" stays undefined instead of
/// snapping to an explicit default.
#[test]
fn gltf_roundtrip_carries_texture_transform_and_sampler_state() {
    let scene = textured_scene();
    let bytes = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("glTF encode");
    let round = GltfDecoder::new().decode(&bytes).expect("glTF decode");

    let material = round
        .materials
        .iter()
        .find(|m| m.name.as_deref() == Some("uv-transformed"))
        .expect("material survives");
    let texref = material
        .base_color_texture
        .expect("base color texture reference survives");
    let transform = texref.transform.expect("KHR_texture_transform survives");
    assert_eq!(transform.offset, [0.25, 0.5]);
    assert_eq!(transform.rotation, 0.5);
    assert_eq!(transform.scale, [2.0, 1.0]);
    assert_eq!(texref.effective_uv_set(), 0);

    let sampler = &round.textures[texref.texture.0 as usize].sampler;
    assert_eq!(
        sampler.mag_filter, None,
        "undefined mag filter must stay undefined"
    );
    assert_eq!(
        sampler.min_filter, None,
        "undefined min filter must stay undefined"
    );
    assert_eq!(sampler.wrap_s, WrapMode::MirroredRepeat);
    assert_eq!(sampler.wrap_t, WrapMode::ClampToEdge);
}

/// USDZ round-trip of the same scene: geometry preserved, and the
/// texture transform *baked* — the staged USD material schema has no
/// UV-transform prim, so the encoder's documented lossy-flattening
/// step appends a pre-transformed UV channel, retargets the
/// reference, and consumes the typed transform. The sampled
/// coordinates (the ground truth) must equal
/// `TextureTransform::apply` over the source channel.
#[test]
fn usdz_roundtrip_bakes_texture_transform_into_uvs() {
    let scene = textured_scene();
    let bytes = UsdzEncoder::new().encode(&scene).expect("USDZ encode");
    let round = UsdzDecoder::new().decode(&bytes).expect("USDZ decode");

    assert_positions_eq(
        &round.meshes[0].primitives[0].positions,
        &scene.meshes[0].primitives[0].positions,
        "USDZ positions",
    );

    let material = round
        .materials
        .iter()
        .find(|m| m.base_color_texture.is_some())
        .expect("textured material survives");
    let texref = material.base_color_texture.expect("checked above");
    assert!(
        texref.transform.is_none(),
        "the transform is consumed by the bake, not re-surfaced"
    );

    // The reference now samples the baked channel; its coordinates
    // equal the typed transform applied to the source channel.
    let src = textured_scene();
    let src_transform = src.materials[0]
        .base_color_texture
        .unwrap()
        .transform
        .unwrap();
    let expected: Vec<[f32; 2]> = src_transform.apply_channel(&src.meshes[0].primitives[0].uvs[0]);
    let prim = &round.meshes[0].primitives[0];
    let baked = &prim.uvs[texref.effective_uv_set() as usize];
    assert_eq!(baked.len(), expected.len());
    for (i, (got, want)) in baked.iter().zip(expected.iter()).enumerate() {
        for a in 0..2 {
            assert!(
                (got[a] - want[a]).abs() < 1e-5,
                "baked UV {i}.{a}: {} vs {}",
                got[a],
                want[a]
            );
        }
    }
}

/// Cross-codec chain: glTF → USDZ → glTF. Geometry and the material
/// binding survive two format conversions on the 0.0.5 surface.
#[test]
fn gltf_usdz_gltf_chain_preserves_geometry() {
    let scene = textured_scene();
    let gltf1 = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&scene)
        .expect("glTF encode");
    let via_gltf = GltfDecoder::new().decode(&gltf1).expect("glTF decode");
    let usdz = UsdzEncoder::new().encode(&via_gltf).expect("USDZ encode");
    let via_usdz = UsdzDecoder::new().decode(&usdz).expect("USDZ decode");
    let gltf2 = GltfEncoder::with_output(OutputFlavour::JsonEmbedded)
        .encode(&via_usdz)
        .expect("glTF re-encode");
    let last = GltfDecoder::new().decode(&gltf2).expect("glTF re-decode");

    assert_positions_eq(
        &last.meshes[0].primitives[0].positions,
        &scene.meshes[0].primitives[0].positions,
        "chained positions",
    );
    // The USDZ hop bakes the texture transform into an appended UV
    // channel (source channel + baked channel = 2).
    assert_eq!(
        last.meshes[0].primitives[0].uvs.len(),
        2,
        "UV channels after the USDZ bake"
    );
    assert!(
        last.materials
            .iter()
            .any(|m| m.base_color_texture.is_some()),
        "texture binding survives the chain"
    );
}
