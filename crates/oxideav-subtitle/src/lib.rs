//! Standalone-subtitle codecs + containers for oxideav.
//!
//! Hosts the lightweight text subtitle formats, their cross-format
//! converters, and a text-to-RGBA rendering stack (bitmap font +
//! compositor + `RenderedSubtitleDecoder` wrapper).
//!
//! ASS/SSA lives in its own sibling crate `oxideav-ass` because advanced
//! rendering (animated tags, sub-pixel positioning, karaoke playback)
//! needs substantial work that shouldn't clutter this hub. Bitmap-native
//! subtitle formats (PGS, DVB, VobSub) live in `oxideav-sub-image`.
//!
//! | Format        | Codec id      | Container name | Extensions       |
//! |---------------|---------------|----------------|------------------|
//! | SubRip        | `subrip`      | `srt`          | `.srt`           |
//! | WebVTT        | `webvtt`      | `webvtt`       | `.vtt`           |
//! | MicroDVD      | `microdvd`    | `microdvd`     | `.sub`, `.txt`   |
//! | MPL2          | `mpl2`        | `mpl2`         | `.mpl`           |
//! | MPsub         | `mpsub`       | `mpsub`        | `.sub`           |
//! | VPlayer       | `vplayer`     | `vplayer`      | `.txt`, `.vpl`   |
//! | PJS           | `pjs`         | `pjs`          | `.pjs`           |
//! | AQTitle       | `aqtitle`     | `aqtitle`      | `.aqt`           |
//! | JACOsub       | `jacosub`     | `jacosub`      | `.jss`, `.js`    |
//! | RealText      | `realtext`    | `realtext`     | `.rt`            |
//! | SubViewer 1   | `subviewer1`  | `subviewer1`   | `.sub`           |
//! | SubViewer 2   | `subviewer2`  | `subviewer2`   | `.sub`           |
//! | TTML          | `ttml`        | `ttml`         | `.ttml`, `.dfxp`, `.xml` |
//! | SAMI          | `sami`        | `sami`         | `.smi`, `.sami`  |
//! | EBU STL       | `ebu_stl`     | `ebu_stl`      | `.stl`           |
//!
//! Shared-extension conflicts (several formats use `.sub` / `.txt`) are
//! resolved content-first: every container ships a probe that scores the
//! first few KB of input and the registry picks the highest-scoring match.
//!
//! ## Cross-format conversion
//!
//! * [`transform::srt_to_webvtt`]
//! * [`transform::webvtt_to_srt`]
//!
//! Converters touching ASS live in the `oxideav-ass` crate.
//!
//! ## Text → RGBA rendering
//!
//! Any subtitle decoder that produces `Frame::Subtitle` can be wrapped in
//! a [`RenderedSubtitleDecoder`] that produces `Frame::Video(Rgba)` at a
//! caller-specified canvas size. Cue dedup means the wrapper emits at most
//! one frame per visible-state change.
//!
//! In-container subtitle tracks (MKV / MP4 sub streams) are out of scope —
//! this crate deals with standalone files only.

pub mod aqtitle;
pub mod codec;
pub mod compositor;
pub mod container;
pub mod ebu_stl;
pub mod font;
pub mod ir;
pub mod jacosub;
pub mod microdvd;
pub mod mpl2;
pub mod mpsub;
pub mod pjs;
pub mod realtext;
pub mod render;
pub mod sami;
pub mod srt;
pub mod subviewer1;
pub mod subviewer2;
pub mod transform;
pub mod ttml;
pub mod vplayer;
pub mod webvtt;

use oxideav_codec::CodecRegistry;
use oxideav_container::ContainerRegistry;
use oxideav_core::{CodecCapabilities, CodecId, MediaType};

pub use compositor::Compositor;
pub use font::BitmapFont;
pub use ir::{SourceFormat, SubtitleTrack};
pub use render::{make_rendered_decoder, RenderedSubtitleDecoder};
pub use transform::{srt_to_webvtt, webvtt_to_srt};

/// Shape a shared capability set for a subtitle codec. Every text
/// subtitle registered here is `decode=true, encode=true, intra_only=true,
/// lossless=true, media_type=Subtitle`.
fn subtitle_caps(impl_name: &str) -> CodecCapabilities {
    CodecCapabilities {
        decode: false,
        encode: false,
        media_type: MediaType::Subtitle,
        intra_only: true,
        lossy: false,
        lossless: true,
        hardware_accelerated: false,
        implementation: impl_name.into(),
        max_width: None,
        max_height: None,
        max_bitrate: None,
        max_sample_rate: None,
        max_channels: None,
        priority: 100,
        accepted_pixel_formats: Vec::new(),
    }
}

/// Register all text subtitle codecs (decoders + encoders). Each
/// format's `make_decoder` / `make_encoder` lives in its own module and
/// is registered independently — no dispatch branch to maintain here.
pub fn register_codecs(reg: &mut CodecRegistry) {
    // SRT + WebVTT share codec.rs's dispatcher (legacy).
    reg.register_both(
        CodecId::new(codec::SRT_CODEC_ID),
        subtitle_caps("subrip_sw"),
        codec::make_decoder,
        codec::make_encoder,
    );
    reg.register_both(
        CodecId::new(codec::WEBVTT_CODEC_ID),
        subtitle_caps("webvtt_sw"),
        codec::make_decoder,
        codec::make_encoder,
    );

    // Per-format factories for the rest.
    reg.register_both(
        CodecId::new(microdvd::CODEC_ID),
        subtitle_caps("microdvd_sw"),
        microdvd::make_decoder,
        microdvd::make_encoder,
    );
    reg.register_both(
        CodecId::new(mpl2::CODEC_ID),
        subtitle_caps("mpl2_sw"),
        mpl2::make_decoder,
        mpl2::make_encoder,
    );
    reg.register_both(
        CodecId::new(mpsub::CODEC_ID),
        subtitle_caps("mpsub_sw"),
        mpsub::make_decoder,
        mpsub::make_encoder,
    );
    reg.register_both(
        CodecId::new(vplayer::CODEC_ID),
        subtitle_caps("vplayer_sw"),
        vplayer::make_decoder,
        vplayer::make_encoder,
    );
    reg.register_both(
        CodecId::new(pjs::CODEC_ID),
        subtitle_caps("pjs_sw"),
        pjs::make_decoder,
        pjs::make_encoder,
    );
    reg.register_both(
        CodecId::new(aqtitle::CODEC_ID),
        subtitle_caps("aqtitle_sw"),
        aqtitle::make_decoder,
        aqtitle::make_encoder,
    );
    reg.register_both(
        CodecId::new(jacosub::CODEC_ID),
        subtitle_caps("jacosub_sw"),
        jacosub::make_decoder,
        jacosub::make_encoder,
    );
    reg.register_both(
        CodecId::new(realtext::CODEC_ID),
        subtitle_caps("realtext_sw"),
        realtext::make_decoder,
        realtext::make_encoder,
    );
    reg.register_both(
        CodecId::new(subviewer1::CODEC_ID),
        subtitle_caps("subviewer1_sw"),
        subviewer1::make_decoder,
        subviewer1::make_encoder,
    );
    reg.register_both(
        CodecId::new(subviewer2::CODEC_ID),
        subtitle_caps("subviewer2_sw"),
        subviewer2::make_decoder,
        subviewer2::make_encoder,
    );
    reg.register_both(
        CodecId::new(ttml::CODEC_ID),
        subtitle_caps("ttml_sw"),
        ttml::make_decoder,
        ttml::make_encoder,
    );
    reg.register_both(
        CodecId::new(sami::CODEC_ID),
        subtitle_caps("sami_sw"),
        sami::make_decoder,
        sami::make_encoder,
    );
    reg.register_both(
        CodecId::new(ebu_stl::CODEC_ID),
        subtitle_caps("ebu_stl_sw"),
        ebu_stl::make_decoder,
        ebu_stl::make_encoder,
    );
}

/// Register the text subtitle containers (demuxers + muxers + probes).
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Convenience combined registration.
pub fn register(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}
