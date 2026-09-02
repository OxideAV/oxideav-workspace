//! AC-3 / E-AC-3 through the `oxideav_meta` aggregator (round 455).
//!
//! `oxideav-ac3` is reachable from the umbrella harness only through
//! `oxideav_meta::register_all` (the `ac3` feature of the `audio`
//! bundle). This pins the registry surface a container demuxer relies
//! on: both codec ids carry decoder + encoder factories, and the
//! container tag claims (`WAVEFORMATEX` `0x2000` / `0x00A7`, MP4
//! object types `0xA5` / `0xA6`, Matroska `A_AC3` / `A_EAC3`) resolve
//! to the right id.
//!
//! The round-454 JOC/OAMD object presentation (oxideav-ac3 #15) is a
//! **direct-factory** surface — `decoder::make_eac3_decoder_with_joc`
//! — that the registry does not expose: the registered `eac3` decoder
//! keeps the historical §7.8 compatibility downmix and takes no
//! string-keyed option for it. There is therefore no registry-level
//! JOC pin here; add one when (if) the crate routes the renderer
//! through `CodecParameters::options`.

use oxideav_core::{CodecId, CodecParameters, CodecTag, ProbeContext, RuntimeContext};

fn meta_ctx() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_meta::register_all(&mut ctx);
    ctx
}

#[test]
fn register_all_wires_ac3_and_eac3_factories() {
    let ctx = meta_ctx();
    for id in ["ac3", "eac3"] {
        let cid = CodecId::new(id);
        assert!(ctx.codecs.has_decoder(&cid), "{id}: decoder factory");
        assert!(ctx.codecs.has_encoder(&cid), "{id}: encoder factory");
        // The decoder factory is infallible at construction (the
        // per-packet bsid dispatch decides AC-3 vs Annex E), so a
        // stereo-target request builds without extradata.
        let mut p = CodecParameters::audio(cid);
        p.channels = Some(2);
        assert!(
            ctx.codecs.first_decoder(&p).is_ok(),
            "{id}: stereo-target decoder"
        );
    }
}

#[test]
fn container_tags_resolve_to_the_right_codec_id() {
    let ctx = meta_ctx();
    let cases = [
        (CodecTag::wave_format(0x2000), "ac3"),
        (CodecTag::mp4_object_type(0xA5), "ac3"),
        (CodecTag::matroska("A_AC3"), "ac3"),
        (CodecTag::wave_format(0x00A7), "eac3"),
        (CodecTag::mp4_object_type(0xA6), "eac3"),
        (CodecTag::matroska("A_EAC3"), "eac3"),
    ];
    for (tag, want) in cases {
        let id = ctx
            .codecs
            .resolve_tag_ref(&ProbeContext::new(&tag))
            .unwrap_or_else(|| panic!("{tag:?} must resolve"));
        assert_eq!(id.as_str(), want, "{tag:?}");
    }
}
