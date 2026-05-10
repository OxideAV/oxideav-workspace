//! `Mesh3DRegistry` lookup contract — exercised against the four
//! sibling format crates' `register()` helpers.
//!
//! These tests do NOT decode anything; they only verify that the
//! registry's extension/format-id surface resolves correctly after
//! the standard `oxideav_<fmt>::register(&mut reg)` calls have run.
//! Decode/encode coverage lives in `cross_format_roundtrip.rs`.

use oxideav_mesh3d::{Mesh3DDecoder, Mesh3DEncoder, Mesh3DRegistry, Result, Scene3D};

// ─────────────────────── populated-registry shape ──────────────────

/// Build a fully-populated registry with every sibling format crate
/// wired in.
fn registry_with_all_formats() -> Mesh3DRegistry {
    let mut reg = Mesh3DRegistry::new();
    oxideav_stl::register(&mut reg);
    oxideav_obj::register(&mut reg);
    oxideav_gltf::register(&mut reg);
    oxideav_usdz::register(&mut reg);
    reg
}

#[test]
fn every_sibling_register_lands_decoders() {
    let reg = registry_with_all_formats();
    let mut formats: Vec<&str> = reg.decoder_formats().collect();
    formats.sort();
    // OBJ registers two formats (`obj` + `mtl`); USDZ + STL + glTF
    // register one each.
    assert_eq!(
        formats,
        vec!["gltf", "mtl", "obj", "stl", "usdz"],
        "decoder format ids registered by the four sibling crates"
    );
}

#[test]
fn every_sibling_register_lands_encoders() {
    let reg = registry_with_all_formats();
    let mut formats: Vec<&str> = reg.encoder_formats().collect();
    formats.sort();
    // glTF registers `gltf` and `glb` as separate encoder formats so
    // the `.gltf` extension routes to the JSON-flavour encoder while
    // `.glb` routes to the binary-flavour. STL + OBJ contribute one
    // each. USDZ encoder added in r2 (commit 263ff4b).
    assert_eq!(
        formats,
        vec!["glb", "gltf", "mtl", "obj", "stl", "usdz"],
        "encoder format ids registered by the four sibling crates"
    );
}

// ─────────────── extension-route resolution per format ─────────────

#[test]
fn decoder_for_extension_resolves_each_canonical_extension() {
    let reg = registry_with_all_formats();
    for ext in ["stl", "obj", "mtl", "gltf", "glb", "usdz"] {
        assert!(
            reg.decoder_for_extension(ext).is_some(),
            "decoder_for_extension({ext:?}) should resolve",
        );
    }
}

#[test]
fn encoder_for_extension_resolves_each_canonical_extension() {
    let reg = registry_with_all_formats();
    // USDZ encoder added in r2 (commit 263ff4b) — every format now
    // has both decoder + encoder.
    for ext in ["stl", "obj", "mtl", "gltf", "glb", "usdz"] {
        assert!(
            reg.encoder_for_extension(ext).is_some(),
            "encoder_for_extension({ext:?}) should resolve",
        );
    }
}

#[test]
fn decoder_for_format_id_resolves_each_canonical_id() {
    let reg = registry_with_all_formats();
    for fmt in ["stl", "obj", "mtl", "gltf", "usdz"] {
        assert!(
            reg.decoder_for_format(fmt).is_some(),
            "decoder_for_format({fmt:?}) should resolve",
        );
    }
}

#[test]
fn encoder_for_format_id_resolves_each_canonical_id() {
    let reg = registry_with_all_formats();
    for fmt in ["stl", "obj", "mtl", "gltf", "glb", "usdz"] {
        assert!(
            reg.encoder_for_format(fmt).is_some(),
            "encoder_for_format({fmt:?}) should resolve",
        );
    }
}

// ───────────────── case-insensitivity contract ─────────────────────

#[test]
fn extension_lookup_is_case_insensitive() {
    let reg = registry_with_all_formats();
    // Path::extension() returns the file's last suffix verbatim, so
    // upper-case extensions ("Foo.STL", "scene.GLTF") show up in the
    // wild. Registry must fold both sides for a hit.
    assert!(reg.decoder_for_extension("STL").is_some(), "STL");
    assert!(reg.decoder_for_extension("Stl").is_some(), "Stl");
    assert!(reg.decoder_for_extension("GLTF").is_some(), "GLTF");
    assert!(reg.decoder_for_extension("Glb").is_some(), "Glb");
    assert!(reg.decoder_for_extension("USDZ").is_some(), "USDZ");
    assert!(reg.encoder_for_extension("OBJ").is_some(), "OBJ encoder");
}

#[test]
fn format_id_lookup_is_case_insensitive() {
    let reg = registry_with_all_formats();
    assert!(reg.decoder_for_format("STL").is_some());
    assert!(reg.decoder_for_format("GlTf").is_some());
    assert!(reg.encoder_for_format("OBJ").is_some());
}

// ───────────────────── overwrite semantics ─────────────────────────

/// Stand-in decoder that returns a scene whose `extras` carries a
/// distinguishing tag — lets us tell which factory the registry
/// dispatched without poking at internal state.
struct TaggedDecoder {
    tag: &'static str,
}
impl std::fmt::Debug for TaggedDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaggedDecoder")
            .field("tag", &self.tag)
            .finish()
    }
}
impl Mesh3DDecoder for TaggedDecoder {
    fn decode(&mut self, _: &[u8]) -> Result<Scene3D> {
        let mut s = Scene3D::new();
        s.extras.insert(
            "tag".to_string(),
            serde_json::Value::String(self.tag.into()),
        );
        Ok(s)
    }
}

struct TaggedEncoder {
    tag: &'static str,
}
impl std::fmt::Debug for TaggedEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaggedEncoder")
            .field("tag", &self.tag)
            .finish()
    }
}
impl Mesh3DEncoder for TaggedEncoder {
    fn encode(&mut self, _: &Scene3D) -> Result<Vec<u8>> {
        Ok(self.tag.as_bytes().to_vec())
    }
}

#[test]
fn register_decoder_overwrites_same_format_id() {
    let mut reg = Mesh3DRegistry::new();
    reg.register_decoder(
        "stl",
        &["stl"],
        Box::new(|| Box::new(TaggedDecoder { tag: "first" })),
    );
    reg.register_decoder(
        "stl",
        &["stl"],
        Box::new(|| Box::new(TaggedDecoder { tag: "second" })),
    );
    let s = reg
        .decoder_for_format("stl")
        .expect("stl resolves")
        .decode(b"")
        .expect("decode");
    assert_eq!(
        s.extras.get("tag").and_then(|v| v.as_str()),
        Some("second"),
        "register_decoder twice with same format id should overwrite"
    );
}

#[test]
fn register_encoder_overwrites_same_format_id() {
    let mut reg = Mesh3DRegistry::new();
    reg.register_encoder(
        "stl",
        &["stl"],
        Box::new(|| Box::new(TaggedEncoder { tag: "v1" })),
    );
    reg.register_encoder(
        "stl",
        &["stl"],
        Box::new(|| Box::new(TaggedEncoder { tag: "v2" })),
    );
    let bytes = reg
        .encoder_for_format("stl")
        .expect("stl resolves")
        .encode(&Scene3D::new())
        .expect("encode");
    assert_eq!(bytes, b"v2", "last writer wins");
}

#[test]
fn register_decoder_with_new_extensions_remaps_extension_route() {
    // Re-registering a format with a different extension list moves
    // the extension → format id mapping for the new list. Old-only
    // extensions stay pointed at the (now-overwritten) format entry,
    // which still resolves because the format-id key didn't change.
    let mut reg = Mesh3DRegistry::new();
    reg.register_decoder(
        "stl",
        &["stl", "stla"],
        Box::new(|| Box::new(TaggedDecoder { tag: "a" })),
    );
    reg.register_decoder(
        "stl",
        &["stl"], // dropped `.stla`
        Box::new(|| Box::new(TaggedDecoder { tag: "b" })),
    );
    // `.stl` now resolves to the new factory…
    let s_stl = reg
        .decoder_for_extension("stl")
        .expect("stl resolves")
        .decode(b"")
        .unwrap();
    assert_eq!(s_stl.extras.get("tag").and_then(|v| v.as_str()), Some("b"));
    // …and `.stla` (which was removed from the new ext list) still
    // resolves to the same format id but now reaches the new
    // factory, since `decoder_by_ext` only writes on register and the
    // shared format key was overwritten.
    let s_stla = reg
        .decoder_for_extension("stla")
        .expect("stla still mapped")
        .decode(b"")
        .unwrap();
    assert_eq!(s_stla.extras.get("tag").and_then(|v| v.as_str()), Some("b"));
}

// ───────────────────── unknown-key behaviour ───────────────────────

#[test]
fn unknown_extension_returns_none() {
    let reg = registry_with_all_formats();
    assert!(reg.decoder_for_extension("fbx").is_none());
    assert!(reg.encoder_for_extension("fbx").is_none());
    assert!(reg.decoder_for_extension("").is_none());
}

#[test]
fn unknown_format_id_returns_none() {
    let reg = registry_with_all_formats();
    assert!(reg.decoder_for_format("fbx").is_none());
    assert!(reg.encoder_for_format("fbx").is_none());
}

// ──────────── extensions-reverse-lookup / introspection ────────────

#[test]
fn decoder_extensions_reverse_lookup() {
    let reg = registry_with_all_formats();
    // glTF decoder is registered under the `gltf` format id with both
    // extensions; `glb` resolves through the same decoder route.
    let exts = reg
        .decoder_extensions("gltf")
        .expect("gltf decoder format known");
    let mut sorted: Vec<&str> = exts.iter().map(String::as_str).collect();
    sorted.sort();
    assert_eq!(sorted, vec!["glb", "gltf"]);
    // STL only knows `.stl`.
    assert_eq!(
        reg.decoder_extensions("stl")
            .expect("stl known")
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["stl"]
    );
}

#[test]
fn encoder_extensions_reverse_lookup() {
    let reg = registry_with_all_formats();
    // glTF JSON-flavour encoder is registered under format id `gltf`
    // with extension `gltf`; the binary-flavour encoder is a separate
    // format id `glb` with extension `glb`.
    assert_eq!(
        reg.encoder_extensions("gltf")
            .expect("gltf encoder known")
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["gltf"]
    );
    assert_eq!(
        reg.encoder_extensions("glb")
            .expect("glb encoder known")
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["glb"]
    );
    assert!(
        reg.encoder_extensions("usdz").is_some(),
        "USDZ encoder added in r2; reverse lookup should resolve"
    );
}

#[test]
fn debug_impl_lists_registered_format_ids() {
    let reg = registry_with_all_formats();
    let dbg = format!("{reg:?}");
    // Debug surface enumerates both decoder + encoder format keysets;
    // we just check our representative ids show up — exact ordering
    // is a HashMap iteration artefact.
    assert!(dbg.contains("stl"));
    assert!(dbg.contains("gltf"));
    assert!(dbg.contains("usdz"));
    assert!(dbg.contains("obj"));
}

// ─────────────────── fresh-registry empty contract ─────────────────

#[test]
fn empty_registry_resolves_nothing() {
    let reg = Mesh3DRegistry::new();
    assert_eq!(reg.decoder_formats().count(), 0);
    assert_eq!(reg.encoder_formats().count(), 0);
    assert!(reg.decoder_for_extension("stl").is_none());
    assert!(reg.encoder_for_format("gltf").is_none());
}

#[test]
fn default_registry_matches_new() {
    // `Default::default()` is the documented alias for `new()`.
    let r1 = Mesh3DRegistry::default();
    let r2 = Mesh3DRegistry::new();
    assert_eq!(r1.decoder_formats().count(), r2.decoder_formats().count());
    assert_eq!(r1.encoder_formats().count(), r2.encoder_formats().count());
}
