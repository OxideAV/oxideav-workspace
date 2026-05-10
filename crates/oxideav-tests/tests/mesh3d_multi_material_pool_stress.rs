//! Round-5 multi-mesh / multi-material / vertex-pool dedup stress
//! coverage. Layered on top of round-4's two-primitive vertex-pool
//! tests in `extras_and_skinning_coverage.rs`.
//!
//! Round 4 stayed inside a single `Mesh` with at most two primitives
//! and at most two materials. Real authoring tools regularly produce
//! scenes with five-plus materials per scene, dozens of primitives per
//! mesh, and several meshes attached to a hierarchy of nodes — the
//! OBJ encoder's vertex pool is supposed to dedup *across* meshes too
//! (see the global `positions: Vec<[f32; 3]>` accumulator in the
//! published `oxideav-obj 0.0.0` `serialize_obj`), and the glTF
//! encoder is supposed to keep every primitive's material binding
//! correct even when the binding indices alias each other across
//! distant primitives.
//!
//! The scenes here exercise:
//!
//! 1. **Cross-mesh vertex pool dedup (OBJ)** — two meshes sharing the
//!    same physical corner positions must collapse to one global `v`
//!    entry per unique position, not per (mesh, vertex) pair.
//! 2. **Five-material binding survival** — a single mesh with five
//!    primitives, each bound to a distinct material; both OBJ
//!    (`usemtl` directives) and glTF (per-primitive `material` index)
//!    must round-trip every binding.
//! 3. **Multi-mesh hierarchy** — a parent node with two child nodes,
//!    each child carrying its own mesh; geometry survives across all
//!    three encoders, and the glTF encoder preserves the per-mesh
//!    partition.
//! 4. **Material aliasing** — multiple primitives bound to the *same*
//!    material id (not distinct mats) emit one `usemtl` per primitive
//!    on the OBJ side and identical `material` indices on the glTF
//!    side.

use oxideav_mesh3d::{
    Material, Mesh, Mesh3DDecoder, Mesh3DEncoder, Node, Primitive, Scene3D, Topology,
};

use oxideav_gltf::{GltfDecoder, GltfEncoder, OutputFlavour};
use oxideav_obj::{ObjDecoder, ObjEncoder};
use oxideav_stl::{StlDecoder, StlEncoder};

// ─────────────────────────── fixtures ────────────────────────────

/// Two-mesh scene where both meshes are positioned at the origin and
/// share the *same three corner positions* `(0,0,0) (1,0,0) (0,1,0)`.
/// The OBJ encoder's global vertex pool is supposed to collapse the
/// six raw positions (3 per mesh × 2 meshes) into three `v` lines.
fn two_meshes_sharing_one_triangle_corners() -> Scene3D {
    let mut scene = Scene3D::new();
    let make_tri = |name: &str| {
        let mut prim = Primitive::new(Topology::Triangles);
        prim.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        Mesh::new(Some(name.to_string())).with_primitive(prim)
    };
    let mid_a = scene.add_mesh(make_tri("a"));
    let mid_b = scene.add_mesh(make_tri("b"));
    let na = scene.add_node(Node::new().with_mesh(mid_a));
    let nb = scene.add_node(Node::new().with_mesh(mid_b));
    scene.add_root(na);
    scene.add_root(nb);
    scene
}

/// One mesh, five primitives, five materials. Each primitive is its
/// own triangle at a different y-offset so the geometry doesn't
/// collide; each carries a distinct material id pointing at a
/// distinct named material.
fn five_materials_one_mesh() -> Scene3D {
    let mut scene = Scene3D::new();
    let mat_ids: Vec<_> = ["red", "green", "blue", "yellow", "magenta"]
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let rgba = match i {
                0 => [1.0, 0.0, 0.0, 1.0],
                1 => [0.0, 1.0, 0.0, 1.0],
                2 => [0.0, 0.0, 1.0, 1.0],
                3 => [1.0, 1.0, 0.0, 1.0],
                _ => [1.0, 0.0, 1.0, 1.0],
            };
            scene.add_material(Material::new().with_name(*name).with_base_color(rgba))
        })
        .collect();
    let mut mesh = Mesh::new(Some("painted".to_string()));
    for (i, mat) in mat_ids.iter().enumerate() {
        let y = i as f32; // disjoint vertical bands so positions differ
        let mut prim = Primitive::new(Topology::Triangles);
        prim.positions = vec![[0.0, y, 0.0], [1.0, y, 0.0], [0.0, y + 1.0, 0.0]];
        prim.material = Some(*mat);
        mesh = mesh.with_primitive(prim);
    }
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

/// A parent node with two child nodes, each child carrying its own
/// mesh. Used to confirm that multi-mesh / hierarchical scenes survive
/// the encoder pipeline. The two child meshes have disjoint positions
/// so the encoder doesn't collapse them via dedup.
fn two_child_meshes_under_parent() -> Scene3D {
    let mut scene = Scene3D::new();
    let mut prim_a = Primitive::new(Topology::Triangles);
    prim_a.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let mesh_a = Mesh::new(Some("child_a".to_string())).with_primitive(prim_a);
    let mid_a = scene.add_mesh(mesh_a);
    let nid_a = scene.add_node(Node::new().with_mesh(mid_a).with_name("a"));

    let mut prim_b = Primitive::new(Topology::Triangles);
    prim_b.positions = vec![[10.0, 0.0, 0.0], [11.0, 0.0, 0.0], [10.0, 1.0, 0.0]];
    let mesh_b = Mesh::new(Some("child_b".to_string())).with_primitive(prim_b);
    let mid_b = scene.add_mesh(mesh_b);
    let nid_b = scene.add_node(Node::new().with_mesh(mid_b).with_name("b"));

    let mut parent = Node::new().with_name("parent");
    parent.children = vec![nid_a, nid_b];
    let pid = scene.add_node(parent);
    scene.add_root(pid);
    scene
}

/// Three primitives, all bound to the *same* material id. Different
/// from `five_materials_one_mesh` (which uses one binding per
/// primitive across distinct mats); this one alias-shares a single
/// material across multiple primitives — the OBJ encoder still has to
/// emit `usemtl` once per primitive, and the glTF encoder still has
/// to set the same `material` index on each.
fn three_primitives_one_material() -> Scene3D {
    let mut scene = Scene3D::new();
    let mat = scene.add_material(
        Material::new()
            .with_name("shared")
            .with_base_color([0.5, 0.5, 0.5, 1.0]),
    );
    let mut mesh = Mesh::new(Some("aliased".to_string()));
    for i in 0..3 {
        let y = i as f32;
        let mut prim = Primitive::new(Topology::Triangles);
        prim.positions = vec![[0.0, y, 0.0], [1.0, y, 0.0], [0.0, y + 1.0, 0.0]];
        prim.material = Some(mat);
        mesh = mesh.with_primitive(prim);
    }
    let mid = scene.add_mesh(mesh);
    let nid = scene.add_node(Node::new().with_mesh(mid));
    scene.add_root(nid);
    scene
}

// ─────────── §1. cross-mesh vertex pool dedup (OBJ) ────────────────

#[test]
fn obj_encoder_dedupes_shared_vertices_across_separate_meshes() {
    // Two meshes, three vertices each, all six positions lying on the
    // same three corners. The OBJ encoder's global pool MUST collapse
    // to three `v` lines (not six). This is the cross-*mesh* dedup
    // case, distinct from round-4's cross-*primitive*-within-one-mesh
    // case.
    let scene = two_meshes_sharing_one_triangle_corners();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode two meshes shared verts");
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
        3,
        "two meshes × 3 verts each = 6 raw, dedup should pool to 3 across meshes (got {} lines: {:?})",
        v_lines.len(),
        v_lines,
    );
}

#[test]
fn obj_round_trip_preserves_two_triangles_across_meshes() {
    // Even with cross-mesh dedup, the face count survives: two `f`
    // lines emitted, two triangles after round-trip.
    let scene = two_meshes_sharing_one_triangle_corners();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode two meshes");
    let round = ObjDecoder::new()
        .decode(&bytes)
        .expect("OBJ decode two meshes");
    assert_eq!(
        round.triangle_count(),
        2,
        "two triangles survive across the two-mesh round-trip"
    );
}

#[test]
fn gltf_round_trip_keeps_two_meshes_distinct() {
    // glTF preserves the per-mesh partition; both meshes survive as
    // their own `meshes[i]` entry on the JSON side.
    let scene = two_meshes_sharing_one_triangle_corners();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("gltf encode");
    let round = GltfDecoder::new().decode(&bytes).expect("gltf decode");
    assert_eq!(
        round.meshes.len(),
        2,
        "two meshes stay distinct in glTF round-trip (got {})",
        round.meshes.len()
    );
}

#[test]
fn stl_round_trip_flattens_two_meshes_into_one_stream() {
    // STL has no concept of meshes — the encoder emits one big
    // triangle list. Six positions in, two triangles out (not three
    // because dedup doesn't apply: STL stores triangles flat).
    let scene = two_meshes_sharing_one_triangle_corners();
    let bytes = StlEncoder::new_binary()
        .encode(&scene)
        .expect("STL encode multi-mesh");
    let round = StlDecoder::new()
        .decode(&bytes)
        .expect("STL decode multi-mesh");
    assert_eq!(
        round.triangle_count(),
        2,
        "STL flattens but keeps the two triangles"
    );
}

// ─────────── §2. five-material binding survival ────────────────────

#[test]
fn obj_round_trip_emits_five_distinct_usemtl_directives() {
    // Five primitives, five materials → five `usemtl` directives in
    // the OBJ output, one per primitive.
    let scene = five_materials_one_mesh();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode five mats");
    let text = std::str::from_utf8(&bytes).expect("OBJ output is UTF-8");
    let usemtl_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("usemtl ")).collect();
    assert_eq!(
        usemtl_lines.len(),
        5,
        "five primitives × distinct materials → five usemtl directives (got {usemtl_lines:?})"
    );
    for name in ["red", "green", "blue", "yellow", "magenta"] {
        let directive = format!("usemtl {name}");
        assert!(
            usemtl_lines.iter().any(|l| l == &directive),
            "usemtl directive for `{name}` missing from {usemtl_lines:?}"
        );
    }
}

#[test]
fn obj_round_trip_preserves_all_five_material_names_via_extras() {
    // After encode → decode the five primitives split back out (each
    // usemtl boundary opens a fresh primitive) and each carries its
    // matching `obj:usemtl` extras key.
    let scene = five_materials_one_mesh();
    let bytes = ObjEncoder::new().encode(&scene).expect("OBJ encode");
    let round = ObjDecoder::new().decode(&bytes).expect("OBJ decode");
    let prims = &round.meshes[0].primitives;
    assert_eq!(prims.len(), 5, "five primitives split out by usemtl");
    let mut names: Vec<&str> = prims
        .iter()
        .filter_map(|p| p.extras.get("obj:usemtl").and_then(|v| v.as_str()))
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["blue", "green", "magenta", "red", "yellow"],
        "all five material names round-trip via extras"
    );
}

#[test]
fn gltf_round_trip_preserves_all_five_material_indices_distinctly() {
    // glTF preserves both the material set (five `materials[i]`) and
    // each primitive's `material` index. We resolve every primitive's
    // bound material name through the index table and confirm the
    // five distinct names round-trip.
    let scene = five_materials_one_mesh();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("gltf encode five mats");
    let round = GltfDecoder::new().decode(&bytes).expect("gltf decode");
    assert_eq!(
        round.materials.len(),
        5,
        "five materials survive distinctly"
    );
    let prims = &round.meshes[0].primitives;
    assert_eq!(prims.len(), 5, "five primitives stay distinct");
    let mut names: Vec<&str> = prims
        .iter()
        .map(|p| {
            let idx = p.material.expect("material binding survives").0 as usize;
            round.materials[idx].name.as_deref().unwrap_or("?")
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["blue", "green", "magenta", "red", "yellow"]);
}

// ─────────── §3. multi-mesh hierarchy ──────────────────────────────

#[test]
fn obj_round_trip_preserves_geometry_under_parent_node() {
    // A parent node with two child meshes — geometry survives even
    // though OBJ has no node hierarchy.
    let scene = two_child_meshes_under_parent();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode hierarchy");
    let round = ObjDecoder::new().decode(&bytes).expect("OBJ decode");
    assert_eq!(
        round.triangle_count(),
        2,
        "both child triangles survive the OBJ flatten"
    );
}

#[test]
fn gltf_round_trip_preserves_two_child_meshes_distinctly() {
    // glTF carries the node graph, so the two child meshes survive
    // as two `meshes[i]` entries even after a round-trip. We don't
    // rely on the parent/child structure surviving (that's a separate
    // test; see r4 hierarchy coverage), only on the per-mesh
    // partition staying intact.
    let scene = two_child_meshes_under_parent();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("gltf encode hierarchy");
    let round = GltfDecoder::new().decode(&bytes).expect("gltf decode");
    assert_eq!(
        round.meshes.len(),
        2,
        "two child meshes survive as distinct gltf meshes"
    );
    let total_tris: usize = round
        .meshes
        .iter()
        .flat_map(|m| m.primitives.iter())
        .map(|p| p.triangle_count())
        .sum();
    assert_eq!(total_tris, 2, "both child triangles survive");
}

#[test]
fn stl_round_trip_flattens_hierarchy_into_two_triangles() {
    // STL drops the hierarchy and emits one big triangle list;
    // both triangles survive in flat order.
    let scene = two_child_meshes_under_parent();
    let bytes = StlEncoder::new_binary()
        .encode(&scene)
        .expect("STL encode hierarchy");
    let round = StlDecoder::new().decode(&bytes).expect("STL decode");
    assert_eq!(
        round.triangle_count(),
        2,
        "two triangles survive STL flatten"
    );
}

// ─────────── §4. material aliasing ─────────────────────────────────

#[test]
fn obj_round_trip_emits_three_usemtl_directives_for_aliased_material() {
    // Three primitives bound to the same material id. The OBJ encoder
    // currently emits one `usemtl` per primitive (state-token before
    // the face stream) — even though they all reference the same
    // material name.
    let scene = three_primitives_one_material();
    let bytes = ObjEncoder::new()
        .encode(&scene)
        .expect("OBJ encode aliased mat");
    let text = std::str::from_utf8(&bytes).expect("OBJ output is UTF-8");
    let usemtl_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("usemtl ")).collect();
    assert_eq!(
        usemtl_lines.len(),
        3,
        "three primitives → three usemtl directives even when aliased to the same mat (got {usemtl_lines:?})"
    );
    assert!(
        usemtl_lines.iter().all(|l| l == &"usemtl shared"),
        "every directive names the shared material"
    );
}

#[test]
fn gltf_round_trip_keeps_aliased_material_index_collapsed_to_one() {
    // glTF only needs one `materials[]` entry — the three primitives
    // each carry the same `material` index pointing at it.
    let scene = three_primitives_one_material();
    let bytes = GltfEncoder::with_output(OutputFlavour::Glb)
        .encode(&scene)
        .expect("gltf encode aliased mat");
    let round = GltfDecoder::new().decode(&bytes).expect("gltf decode");
    assert_eq!(
        round.materials.len(),
        1,
        "aliased material collapses to one materials[] entry"
    );
    let prims = &round.meshes[0].primitives;
    assert_eq!(prims.len(), 3, "three primitives stay distinct");
    let idxs: Vec<u32> = prims
        .iter()
        .map(|p| p.material.expect("material binding survives").0)
        .collect();
    assert_eq!(
        idxs,
        vec![0, 0, 0],
        "all three primitives reference materials[0] after aliasing collapse"
    );
}
