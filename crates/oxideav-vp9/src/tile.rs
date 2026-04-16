//! VP9 tile / partition / block decode — scaffold.
//!
//! Reference: VP9 spec §6.4 (`decode_tiles`), §6.4.1 (`decode_tile`),
//! §6.4.2 (`decode_partition`), §6.4.3 (`decode_block`), §7.4 (block-level
//! semantic process), §8.5 (intra prediction), §8.6 (inter prediction),
//! §8.7 (reconstruction), §8.8 (loop filter).
//!
//! Status: this module exposes a typed entry point that *would* run the
//! tile-decode loop, but immediately returns `Error::Unsupported` with a
//! pointer to the VP9 spec section that needs implementing. Intentional —
//! VP9 is roughly 10 KLOC for full decode and lives outside the scope
//! of the current scaffold landing.

use oxideav_core::{Error, Result};

use crate::compressed_header::CompressedHeader;
use crate::headers::UncompressedHeader;

/// Walk the tile / partition / block tree per §6.4. Currently a stub that
/// returns `Unsupported`.
pub fn decode_tiles(
    _tile_payload: &[u8],
    _hdr: &UncompressedHeader,
    _ch: &CompressedHeader,
) -> Result<()> {
    Err(Error::unsupported(
        "vp9 §6.4 decode_tiles: tile/partition/block decode not implemented \
         (needs §6.4.2 decode_partition, §6.4.3 decode_block, §8.5 intra prediction, \
         §8.6 inter prediction, §8.7 reconstruction, §8.8 loop filter)",
    ))
}

/// Recurse into a single 64×64 superblock per §6.4.2 — stub.
pub fn decode_partition(
    _bd: &mut crate::bool_decoder::BoolDecoder<'_>,
    _row: u32,
    _col: u32,
    _sb_size: u32,
) -> Result<()> {
    Err(Error::unsupported(
        "vp9 §6.4.2 decode_partition: superblock partition tree not implemented",
    ))
}

/// Decode one block per §6.4.3 — stub.
pub fn decode_block(
    _bd: &mut crate::bool_decoder::BoolDecoder<'_>,
    _row: u32,
    _col: u32,
    _bsize: u32,
) -> Result<()> {
    Err(Error::unsupported(
        "vp9 §6.4.3 decode_block: block decode (residual + prediction + reconstruction) not implemented",
    ))
}
