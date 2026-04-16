//! Pure-Rust ITU-T H.263 baseline video decoder.
//!
//! Scope:
//! * H.263 picture header (PSC, TR, PTYPE, source format, PQUANT, CPM, PEI/
//!   PSPARE loop) — Annex C of ITU-T Rec. H.263 (02/98).
//! * GOB header parser (GBSC, GN, GFID, GQUANT) — §5.2.
//! * **I-picture decode** — MB layer (MCBPC for I, CBPY, optional DQUANT),
//!   block layer (8-bit INTRADC + AC TCOEF VLC), H.263 dequantisation, 8×8
//!   IDCT, output 4:2:0 YUV.
//! * Source formats 1..=5: sub-QCIF, QCIF, CIF, 4CIF, 16CIF.
//! * Reuses VLC tables and IDCT/dequantisation from `oxideav-mpeg4video`
//!   (the MPEG-4 Part 2 VLCs are identical to the H.263 baseline ones).
//!
//! Out of scope (returns `Error::Unsupported`):
//! * **P-pictures** — motion compensation + inter texture decode (§5.3.5).
//! * PB-frames mode (§G).
//! * Annex D (Unrestricted MV), Annex E (SAC), Annex F (Advanced Prediction),
//!   Annex G (PB-frames), Annex I (Advanced Intra Coding), Annex J
//!   (Deblocking filter), Annex K (Slice Structured Mode), Annex N (RPS),
//!   Annex P (Reference Picture Resampling), Annex T (Modified Quantization).
//! * H.263+/PLUSPTYPE custom picture format extensions.
//! * Encoder.
//!
//! No runtime dependencies beyond `oxideav-core`, `oxideav-codec`, and
//! `oxideav-mpeg4video` (whose VLC tables we share).

#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]

pub mod block;
pub mod decoder;
pub mod gob;
pub mod mb;
pub mod picture;
pub mod start_code;

use oxideav_codec::CodecRegistry;
use oxideav_core::{CodecCapabilities, CodecId};

/// The canonical oxideav codec id for ITU-T H.263 baseline video.
///
/// MP4 sample entries `s263` and `h263` map to this id; raw `.h263`
/// elementary-stream files probe to it as well.
pub const CODEC_ID_STR: &str = "h263";

/// Register the H.263 decoder with a codec registry.
pub fn register(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("h263_sw")
        .with_lossy(true)
        .with_intra_only(false)
        .with_max_size(1408, 1152);
    reg.register_decoder_impl(CodecId::new(CODEC_ID_STR), caps, decoder::make_decoder);
}
