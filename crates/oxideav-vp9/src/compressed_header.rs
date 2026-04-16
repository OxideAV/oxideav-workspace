//! VP9 compressed header (§6.3).
//!
//! The compressed header lives between the uncompressed header and the
//! first tile partition. Its length is `header_size` (read at the very end
//! of the uncompressed header). It is decoded with the boolean engine
//! (§9.2). The decoded values steer downstream tile decode.
//!
//! Subsections:
//! * §6.3.1 read_tx_mode         — 2..3 bits, picks one of {ONLY_4x4,
//!   ALLOW_8x8, ALLOW_16x16, ALLOW_32x32, TX_MODE_SELECT}.
//! * §6.3.2 read_tx_mode_probs   — only when tx_mode == TX_MODE_SELECT.
//! * §6.3.3 read_coef_probs      — fully parsed by spec but voluminous;
//!   this scaffold *skips* (returns Unsupported when invoked) since we
//!   don't yet decode any coefficients.
//! * §6.3.4 read_skip_prob       — 3 probabilities.
//! * §6.3.5 read_inter_mode_probs (P frames only).
//! * §6.3.6 read_interp_filter_probs.
//! * §6.3.7 read_is_inter_probs.
//! * §6.3.8 frame_reference_mode (§6.3.8/9 — single/compound mode probs).
//! * §6.3.10 read_y_mode_probs.
//! * §6.3.11 read_partition_probs.
//! * §6.3.12 mv_probs.
//!
//! Status: this module parses tx_mode and reference_mode, then bails with
//! `Error::Unsupported` because the coefficient/probability surface is
//! large and the rest of the decoder is not implemented yet. The struct
//! is shaped so that the rest can be filled in incrementally.

use oxideav_core::{Error, Result};

use crate::bool_decoder::BoolDecoder;
use crate::headers::{FrameType, UncompressedHeader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxMode {
    Only4x4 = 0,
    Allow8x8 = 1,
    Allow16x16 = 2,
    Allow32x32 = 3,
    Select = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceMode {
    SingleReference = 0,
    CompoundReference = 1,
    ReferenceModeSelect = 2,
}

#[derive(Clone, Debug, Default)]
pub struct CompressedHeader {
    pub tx_mode: Option<TxMode>,
    pub reference_mode: Option<ReferenceMode>,
}

/// Parse the §6.3 compressed header up to the point at which we can no
/// longer make forward progress without the full VP9 probability /
/// inter-mode infrastructure. Returns whatever was decoded plus an
/// `Unsupported` flag for callers that need to know.
pub fn parse_compressed_header(
    payload: &[u8],
    hdr: &UncompressedHeader,
) -> Result<CompressedHeader> {
    if payload.is_empty() {
        return Err(Error::invalid("vp9 §6.3: compressed header missing"));
    }
    let mut bd = BoolDecoder::new(payload)?;
    let mut out = CompressedHeader::default();
    if !hdr.quantization.lossless {
        out.tx_mode = Some(read_tx_mode(&mut bd)?);
    } else {
        out.tx_mode = Some(TxMode::Only4x4);
    }
    // For inter frames we'd read frame_reference_mode here. For key/intra
    // frames it's implicitly SINGLE_REFERENCE.
    if hdr.frame_type == FrameType::Key || hdr.intra_only {
        out.reference_mode = Some(ReferenceMode::SingleReference);
    } else {
        out.reference_mode = Some(read_reference_mode(&mut bd)?);
    }
    // The remaining sub-procedures (coef_probs, skip_prob, mv_probs) are
    // not parsed yet — they do not influence the public CodecParameters
    // surface. Tile decode (which DOES need them) returns Unsupported in
    // `tile.rs`.
    Ok(out)
}

fn read_tx_mode(bd: &mut BoolDecoder<'_>) -> Result<TxMode> {
    // §6.3.1 read_tx_mode.
    let tx_mode = bd.read_literal(2)?;
    let tx_mode = if tx_mode == 3 {
        let extra = bd.read_literal(1)?;
        3 + extra
    } else {
        tx_mode
    };
    Ok(match tx_mode {
        0 => TxMode::Only4x4,
        1 => TxMode::Allow8x8,
        2 => TxMode::Allow16x16,
        3 => TxMode::Allow32x32,
        _ => TxMode::Select,
    })
}

fn read_reference_mode(bd: &mut BoolDecoder<'_>) -> Result<ReferenceMode> {
    // §6.3.8 read_frame_reference_mode.
    let comp_mode = bd.read_literal(1)?;
    let mode = if comp_mode == 0 {
        ReferenceMode::SingleReference
    } else {
        let select = bd.read_literal(1)?;
        if select == 1 {
            ReferenceMode::ReferenceModeSelect
        } else {
            ReferenceMode::CompoundReference
        }
    };
    Ok(mode)
}
