//! Bitmap-native subtitle formats for oxideav.
//!
//! These subtitle formats don't carry text — each cue is a picture that
//! overlays the video. The decoders in this crate therefore produce
//! [`oxideav_core::Frame::Video`] values holding an RGBA canvas sized to
//! the subtitle's display context (either the declared video frame size
//! or the bitmap's own size, depending on format), not
//! [`oxideav_core::Frame::Subtitle`].
//!
//! | Format   | Codec id  | Container name | Extensions    |
//! |----------|-----------|----------------|---------------|
//! | PGS      | `pgs`     | `pgs`          | `.sup`        |
//! | DVB sub  | `dvbsub`  | *(none)*       | rides MPEG-TS |
//! | VobSub   | `vobsub`  | `vobsub`       | `.idx`+`.sub` |
//!
//! ## Scope for this first cut
//!
//! * **Decode only.** Encoding palette-indexed bitmap subtitle formats
//!   is follow-up work.
//! * One RGBA [`oxideav_core::VideoFrame`] is emitted per display-set
//!   (cue change) — either the full video-canvas-sized frame (PGS/DVB)
//!   or the bitmap's own rectangle (VobSub).
//! * Output pixel format is always [`oxideav_core::PixelFormat::Rgba`].
//! * `pts` on the emitted frame matches the cue start time (in the
//!   packet's [`oxideav_core::TimeBase`]). Duration is carried on the
//!   [`oxideav_core::Packet`] the container emits.
//!
//! See per-module docs for format-specific limitations.

pub mod dvbsub;
pub mod pgs;
pub mod vobsub;

use oxideav_codec::CodecRegistry;
use oxideav_container::ContainerRegistry;
use oxideav_core::{CodecCapabilities, CodecId, MediaType};

/// Codec id for PGS / HDMV / Blu-ray `.sup` streams.
pub const PGS_CODEC_ID: &str = "pgs";
/// Codec id for DVB subtitle streams (ETSI EN 300 743).
pub const DVBSUB_CODEC_ID: &str = "dvbsub";
/// Codec id for VobSub / DVD SPU streams.
pub const VOBSUB_CODEC_ID: &str = "vobsub";

/// Register decoders for PGS, DVB subtitles, and VobSub.
///
/// Media type is `Subtitle` even though the emitted frames are
/// `Frame::Video(Rgba)`. That matches ffmpeg's convention: the stream's
/// media kind is `Subtitle`, but the codec emits bitmap pictures.
pub fn register_codecs(reg: &mut CodecRegistry) {
    for (id, impl_name) in [
        (PGS_CODEC_ID, "pgs_sw"),
        (DVBSUB_CODEC_ID, "dvbsub_sw"),
        (VOBSUB_CODEC_ID, "vobsub_sw"),
    ] {
        let caps = CodecCapabilities {
            decode: true,
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
        let factory = match id {
            PGS_CODEC_ID => pgs::make_decoder,
            DVBSUB_CODEC_ID => dvbsub::make_decoder,
            VOBSUB_CODEC_ID => vobsub::make_decoder,
            _ => unreachable!(),
        };
        reg.register_decoder_impl(CodecId::new(id), caps, factory);
    }
}

/// Register the PGS (`.sup`) and VobSub (`.idx`+`.sub`) containers.
///
/// DVB subtitles aren't a standalone file container — they ride inside
/// MPEG-TS — so no demuxer is registered for them here. The codec is
/// what the MPEG-TS demuxer would dispatch to.
pub fn register_containers(reg: &mut ContainerRegistry) {
    pgs::register_container(reg);
    vobsub::register_container(reg);
}

/// Convenience combined registration, mirroring `oxideav_webp::register`.
pub fn register(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}
