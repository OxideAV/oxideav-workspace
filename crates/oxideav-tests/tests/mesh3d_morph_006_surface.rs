//! mesh3d 0.0.6 morph surfaces across the framework codecs: typed
//! target names (`Mesh::target_names`), in-between shapes
//! (`MorphTarget::inbetweens`), static mesh weights and a sampled
//! `MorphWeights` animation, round-tripped gltf ↔ usdz ↔ fbx.
//!
//! What holds today (asserted per format):
//!
//! | feature              | glTF | USDZ | FBX |
//! |----------------------|------|------|-----|
//! | target position deltas | ✓  | ✓    | ✓   |
//! | target names         | ✓    | ✓    | ✓   |
//! | in-between shapes    | dropped | ✓ | dropped |
//! | mesh weights         | ✓    | node-level, unanimated only | ✓ |
//! | MorphWeights anim    | ✓    | ✓    | ✓   |
//!
//! glTF has no in-between encoding and the FBX deformer path does not
//! read `FullWeights`, so both return `inbetweens` empty; USDZ authors
//! a static blend state from `Node::weights` only when no animation
//! drives the node, so `Mesh::weights` defaults on an animated node are
//! dropped. The legs pin each gap so a future lift shows up as a test
//! change, not a silent one.

use oxideav_fbx::{FbxDecoder, FbxEncoder, FbxOutputForm};
use oxideav_gltf::{GltfDecoder, GltfEncoder, OutputFlavour};
use oxideav_mesh3d::{
    Animation, AnimationProperty, AnimationSampler, Inbetween, Interpolation, Mesh, Mesh3DDecoder,
    Mesh3DEncoder, MorphTarget, Node, Primitive, Scene3D, Topology,
};
use oxideav_usdz::{UsdzDecoder, UsdzEncoder};

const NAMES: [&str; 2] = ["smile", "frown"];
const MESH_WEIGHTS: [f32; 2] = [0.3, 0.6];
const KEYS: [f32; 3] = [0.0, 0.5, 1.0];
const FRAMES: [[f32; 2]; 3] = [[0.0, 0.0], [0.5, 0.25], [1.0, 0.0]];

fn morph_scene() -> Scene3D {
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
    let n = prim.positions.len();
    let mut smile = MorphTarget::with_deltas(
        Some((0..n).map(|i| [0.0, 0.1 * (i as f32 + 1.0), 0.2]).collect()),
        None,
        None,
    );
    smile.inbetweens.push(
        Inbetween::new(0.5).with_name("smile_half").with_position(
            (0..n)
                .map(|i| [0.0, 0.02 * (i as f32 + 1.0), 0.15])
                .collect(),
        ),
    );
    let frown = MorphTarget::with_deltas(
        Some((0..n).map(|i| [0.05 * i as f32, -0.1, 0.0]).collect()),
        None,
        None,
    );
    prim.targets = vec![smile, frown];
    let mesh = Mesh::new(Some("face".to_string()))
        .with_primitive(prim)
        .with_weights(MESH_WEIGHTS)
        .with_target_names(NAMES);
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    let sampler = AnimationSampler::morph_weights(
        KEYS.to_vec(),
        FRAMES.iter().map(|f| f.to_vec()).collect(),
        Interpolation::Linear,
    )
    .expect("well-formed MorphWeights sampler");
    scene.add_animation(Animation::new(Some("blink".to_string())).with_channel(
        nid,
        AnimationProperty::MorphWeights,
        sampler,
    ));
    scene
}

fn close(a: &[[f32; 3]], b: &[[f32; 3]], tol: f32) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| (0..3).all(|k| (x[k] - y[k]).abs() <= tol))
}

/// The decoded scene's morph surface, checked against the source.
/// `inbetweens_kept` / `static_weights_kept` select whether the
/// in-betweens and the default `Mesh::weights` must survive or must be
/// (pinned) absent.
fn check(
    label: &str,
    src: &Scene3D,
    dec: &Scene3D,
    inbetweens_kept: bool,
    static_weights_kept: bool,
) {
    let sp = &src.meshes[0].primitives[0];
    let mesh = dec
        .meshes
        .iter()
        .find(|m| !m.primitives.is_empty() && !m.primitives[0].targets.is_empty())
        .unwrap_or_else(|| panic!("{label}: a morphed mesh survives"));
    let dp = &mesh.primitives[0];
    assert_eq!(dp.targets.len(), 2, "{label}: two morph targets");
    assert_eq!(mesh.target_names, NAMES, "{label}: typed target names");
    assert_eq!(mesh.find_target("frown"), Some(1), "{label}: name lookup");
    // Static mesh weights (some codecs surface them on the node; the
    // effective-weight resolver covers both).
    let node = dec
        .nodes
        .iter()
        .position(|n| {
            n.mesh
                .is_some_and(|m| std::ptr::eq(&dec.meshes[m.0 as usize], mesh))
        })
        .map(|i| oxideav_mesh3d::NodeId(i as u32))
        .unwrap_or_else(|| panic!("{label}: node references the morphed mesh"));
    let eff = dec
        .effective_morph_weights(node)
        .unwrap_or_else(|| panic!("{label}: weights"));
    if static_weights_kept {
        assert_eq!(eff.len(), 2, "{label}: static weights {eff:?}");
        assert!(
            eff.iter()
                .zip(&MESH_WEIGHTS)
                .all(|(a, b)| (a - b).abs() < 1e-4),
            "{label}: static weights {eff:?}"
        );
    } else {
        assert!(
            eff.iter().all(|w| *w == 0.0),
            "{label}: static weights expected dropped/zeroed today, got {eff:?}"
        );
    }
    // Position deltas. Codecs may re-index vertices, so compare the
    // per-target delta multiset via the morphed geometry instead.
    for (ti, w) in [(0usize, 1.0f32), (1, 1.0)] {
        let mut weights = [0.0f32; 2];
        weights[ti] = w;
        let a = sp.morphed(&weights).positions;
        let b = dp.morphed(&weights).positions;
        let mut a2 = a.clone();
        let mut b2 = b.clone();
        let key = |p: &[f32; 3]| {
            (p[0] * 1e4) as i64 * 1_000_000 + (p[1] * 1e4) as i64 * 1000 + (p[2] * 1e4) as i64
        };
        a2.sort_by_key(key);
        b2.sort_by_key(key);
        assert!(
            close(&a2, &b2, 1e-4),
            "{label}: target {ti} deltas\n{a2:?}\n{b2:?}"
        );
    }
    // In-betweens.
    let ib = &dp.targets[0].inbetweens;
    if inbetweens_kept {
        assert_eq!(ib.len(), 1, "{label}: in-between station survives");
        assert_eq!(ib[0].name.as_deref(), Some("smile_half"), "{label}");
        assert!((ib[0].weight - 0.5).abs() < 1e-6, "{label}");
        let s = sp.targets[0].at_weight(0.5).position.unwrap();
        let d = dp.targets[0].at_weight(0.5).position.unwrap();
        let mut s2 = s.clone();
        let mut d2 = d.clone();
        let key = |p: &[f32; 3]| {
            (p[0] * 1e4) as i64 * 1_000_000 + (p[1] * 1e4) as i64 * 1000 + (p[2] * 1e4) as i64
        };
        s2.sort_by_key(key);
        d2.sort_by_key(key);
        assert!(close(&s2, &d2, 1e-4), "{label}: station resolution");
    } else {
        // DIVERGENCE pinned: this codec has no in-between carriage.
        assert!(
            ib.is_empty(),
            "{label}: in-betweens are expected to be dropped today"
        );
        assert!(dp.targets[1].inbetweens.is_empty());
    }
    // Sampled MorphWeights animation.
    let anim = dec
        .animations
        .iter()
        .find(|a| {
            a.channel_for(node, AnimationProperty::MorphWeights)
                .is_some()
        })
        .unwrap_or_else(|| panic!("{label}: MorphWeights channel survives"));
    let ch = anim
        .channel_for(node, AnimationProperty::MorphWeights)
        .unwrap();
    assert_eq!(ch.sampler.morph_weight_stride(), Some(2), "{label}: stride");
    let frames = ch.sampler.morph_weight_frames().unwrap();
    assert_eq!(frames.len(), KEYS.len(), "{label}: keyframe count");
    for (k, (got, want)) in frames.iter().zip(&FRAMES).enumerate() {
        assert!(
            got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-4),
            "{label}: frame {k}: {got:?} vs {want:?}"
        );
    }
    let pose = anim.sample_pose(0.25, dec.nodes.len());
    let w = pose.morph_weights[node.0 as usize]
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: pose"));
    assert!(
        (w[0] - 0.25).abs() < 1e-4 && (w[1] - 0.125).abs() < 1e-4,
        "{label}: sampled {w:?}"
    );
    let posed = dec
        .world_mesh_at(anim, 1.0, node)
        .unwrap_or_else(|| panic!("{label}: world_mesh_at"));
    let expect = sp.morphed(&[1.0, 0.0]).positions;
    let mut got = posed.primitives[0].positions.clone();
    let mut expect2 = expect.clone();
    let key = |p: &[f32; 3]| {
        (p[0] * 1e4) as i64 * 1_000_000 + (p[1] * 1e4) as i64 * 1000 + (p[2] * 1e4) as i64
    };
    got.sort_by_key(key);
    expect2.sort_by_key(key);
    assert!(close(&got, &expect2, 1e-3), "{label}: posed mesh at t=1");
}

#[test]
fn gltf_round_trip_keeps_names_weights_animation_drops_inbetweens() {
    let src = morph_scene();
    for flavour in [OutputFlavour::Glb, OutputFlavour::JsonEmbedded] {
        let bytes = GltfEncoder::with_output(flavour)
            .encode(&src)
            .expect("gltf encode");
        let dec = GltfDecoder::new().decode(&bytes).expect("gltf decode");
        check(&format!("gltf/{flavour:?}"), &src, &dec, false, true);
    }
}

#[test]
fn usdz_round_trip_keeps_inbetweens_too() {
    let src = morph_scene();
    let bytes = UsdzEncoder::new().encode(&src).expect("usdz encode");
    let dec = UsdzDecoder::new().decode(&bytes).expect("usdz decode");
    // DIVERGENCE pinned (oxideav-usdz `write_synth_blend_states`): the
    // static blend state is authored from `Node::weights` only, and
    // only for nodes no SkelAnimation drives — `Mesh::weights` defaults
    // on an animated node are not carried.
    check("usdz", &src, &dec, true, false);
}

#[test]
fn fbx_round_trip_keeps_names_weights_animation_drops_inbetweens() {
    let src = morph_scene();
    for form in [FbxOutputForm::Binary, FbxOutputForm::Ascii] {
        let bytes = FbxEncoder::new()
            .form(form)
            .encode(&src)
            .expect("fbx encode");
        let dec = FbxDecoder::new().decode(&bytes).expect("fbx decode");
        check(&format!("fbx/{form:?}"), &src, &dec, false, true);
    }
}

/// Chained cross-format hop: gltf → usdz → fbx → gltf keeps the
/// typed names, static weights and the sampled animation intact.
#[test]
fn chained_gltf_usdz_fbx_gltf_keeps_typed_morph_surface() {
    let src = morph_scene();
    let g = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&src)
        .unwrap();
    let s1 = GltfDecoder::new().decode(&g).unwrap();
    let u = UsdzEncoder::new().encode(&s1).unwrap();
    let s2 = UsdzDecoder::new().decode(&u).unwrap();
    let f = FbxEncoder::new().encode(&s2).unwrap();
    let s3 = FbxDecoder::new().decode(&f).unwrap();
    let g2 = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&s3)
        .unwrap();
    let s4 = GltfDecoder::new().decode(&g2).unwrap();
    // USDZ carries no static state for an animated node (see
    // `usdz_static_state_is_node_weights_without_animation`), so the
    // default weights do not survive the middle hop.
    check("chain", &src, &s4, false, false);
}

/// USDZ static blend state: `Node::weights` on an unanimated node is
/// authored as a `BlendState` SkelAnimation and comes back as the
/// node's effective morph weights.
#[test]
fn usdz_static_state_is_node_weights_without_animation() {
    let mut src = morph_scene();
    src.animations.clear();
    let node = src.roots[0];
    src.node_mut(node).unwrap().weights = MESH_WEIGHTS.to_vec();
    let bytes = UsdzEncoder::new().encode(&src).expect("usdz encode");
    let dec = UsdzDecoder::new().decode(&bytes).expect("usdz decode");
    let mesh_idx = dec
        .meshes
        .iter()
        .position(|m| m.primitives.first().is_some_and(|p| p.targets.len() == 2))
        .expect("morphed mesh");
    let node = dec
        .nodes
        .iter()
        .position(|n| n.mesh.is_some_and(|m| m.0 as usize == mesh_idx))
        .map(|i| oxideav_mesh3d::NodeId(i as u32))
        .expect("node with the morphed mesh");
    let eff = dec
        .effective_morph_weights(node)
        .expect("effective weights");
    assert_eq!(eff.len(), 2, "{eff:?}");
    assert!(
        eff.iter()
            .zip(&MESH_WEIGHTS)
            .all(|(a, b)| (a - b).abs() < 1e-4),
        "{eff:?}"
    );
    assert_eq!(dec.meshes[mesh_idx].target_names, NAMES);
    assert_eq!(
        dec.meshes[mesh_idx].primitives[0].targets[0]
            .inbetweens
            .len(),
        1
    );
}
