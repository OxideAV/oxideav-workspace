//! Pipeline composition.
//!
//! The simplest pipeline is a **remux** — copy packets from a demuxer straight
//! into a muxer. The next step up routes packets through a decoder, optional
//! filter chain, then an encoder before the muxer. Both live here.

use oxideav_codec::CodecRegistry;
use oxideav_container::{Demuxer, Muxer};
use oxideav_core::{Error, Result};

/// Copy all packets from `demuxer` into `muxer` without re-encoding.
///
/// The muxer is expected to already be configured for the demuxer's streams.
pub fn remux(demuxer: &mut dyn Demuxer, muxer: &mut dyn Muxer) -> Result<u64> {
    muxer.write_header()?;
    let mut packets = 0u64;
    loop {
        match demuxer.next_packet() {
            Ok(pkt) => {
                muxer.write_packet(&pkt)?;
                packets += 1;
            }
            Err(Error::Eof) => break,
            Err(e) => return Err(e),
        }
    }
    muxer.write_trailer()?;
    Ok(packets)
}

/// Transcode: demux → decode → encode → mux, stream-by-stream.
///
/// The caller provides a `StreamPlan` for each input stream describing how it
/// should be processed. Keeping this simple for the initial design — a fuller
/// graph-based pipeline will come as format coverage grows.
pub fn transcode(
    _demuxer: &mut dyn Demuxer,
    _muxer: &mut dyn Muxer,
    _codecs: &CodecRegistry,
    _plans: &[StreamPlan],
) -> Result<()> {
    Err(Error::unsupported(
        "transcode pipeline not yet implemented",
    ))
}

/// Per-stream instruction for `transcode`.
#[derive(Clone, Debug)]
pub enum StreamPlan {
    /// Pass the stream through unchanged (copy codec).
    Copy { input_index: u32 },
    /// Decode + re-encode to a different codec.
    Reencode {
        input_index: u32,
        output_codec: String,
    },
    /// Drop the stream entirely.
    Drop { input_index: u32 },
}
