//! MPEG-1 Audio Layer I (MP1) decoder.
//!
//! Packet-in / `AudioFrame`-out decoder covering:
//!
//! - MPEG-1 sample rates: 32 000, 44 100, and 48 000 Hz.
//! - All Layer I channel modes: single-channel, stereo, dual-channel,
//!   joint-stereo (bound-based sample sharing; Layer I has no intensity
//!   stereo scaling — §2.4.2.3).
//! - Bit-allocation codes 0..14 (15 is forbidden and rejected).
//! - 6-bit scalefactor indices into the `SCALE[64]` table
//!   (ISO/IEC 11172-3 Table 3-B.1).
//! - 32-band polyphase synthesis filter per Annex B / Annex D.
//!
//! Not in scope: CRC verification, free-format frames (bitrate index 0).
//!
//! See [`decoder::make_decoder`] for the entry point and
//! [`crate::bitalloc`] for the requantization math.

#![allow(
    clippy::needless_range_loop,
    clippy::excessive_precision,
    clippy::unreadable_literal,
    clippy::too_many_arguments,
    clippy::doc_overindented_list_items
)]

pub mod bitalloc;
pub mod bitreader;
pub mod decoder;
pub mod header;
pub mod synthesis;
pub mod window;

use oxideav_codec::CodecRegistry;
use oxideav_core::{CodecCapabilities, CodecId};

pub const CODEC_ID_STR: &str = "mp1";

pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::audio("mp1_sw")
        .with_lossy(true)
        .with_intra_only(true)
        .with_max_channels(2)
        .with_max_sample_rate(48_000);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps, decoder::make_decoder);
}
