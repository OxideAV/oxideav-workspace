//! Standalone-subtitle codecs + containers for oxideav.
//!
//! Three formats are handled:
//!
//! | Format  | Codec id  | Container name | Extensions |
//! |---------|-----------|----------------|------------|
//! | SubRip  | `subrip`  | `srt`          | `.srt`     |
//! | WebVTT  | `webvtt`  | `webvtt`       | `.vtt`     |
//! | ASS/SSA | `ass`     | `ass`          | `.ass`, `.ssa` |
//!
//! Each format registers a demuxer, a muxer, and a content-based probe.
//! Each codec handles both decode (Packet → `Frame::Subtitle`) and encode
//! (Frame → Packet). The packet payload is the cue's textual form in
//! that format; file-level headers (WebVTT prelude, ASS script info +
//! `[Events]` lead-in) live in the codec parameters' `extradata`.
//!
//! Direct file-level converters live in [`transform`]:
//!
//! * [`transform::srt_to_webvtt`]
//! * [`transform::srt_to_ass`]
//! * [`transform::webvtt_to_srt`]
//! * [`transform::webvtt_to_ass`]
//! * [`transform::ass_to_srt`]
//! * [`transform::ass_to_webvtt`]
//!
//! In-container subtitle tracks (MKV / MP4 sub streams) are out of scope —
//! this crate deals with standalone files only.

pub mod ass;
pub mod codec;
pub mod container;
pub mod ir;
pub mod srt;
pub mod transform;
pub mod webvtt;

use oxideav_codec::CodecRegistry;
use oxideav_container::ContainerRegistry;
use oxideav_core::{CodecCapabilities, CodecId, MediaType};

pub use ir::{SourceFormat, SubtitleTrack};
pub use transform::{
    ass_to_srt, ass_to_webvtt, srt_to_ass, srt_to_webvtt, webvtt_to_ass, webvtt_to_srt,
};

/// Register all three subtitle codecs (decoders + encoders).
pub fn register_codecs(reg: &mut CodecRegistry) {
    for (id, impl_name) in [
        (codec::SRT_CODEC_ID, "subrip_sw"),
        (codec::WEBVTT_CODEC_ID, "webvtt_sw"),
        (codec::ASS_CODEC_ID, "ass_sw"),
    ] {
        let caps = CodecCapabilities {
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
        };
        reg.register_both(
            CodecId::new(id),
            caps,
            codec::make_decoder,
            codec::make_encoder,
        );
    }
}

/// Register all three subtitle containers (demuxers + muxers + probes).
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Convenience combined registration, mirroring `oxideav_webp::register`
/// shape.
pub fn register(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}
