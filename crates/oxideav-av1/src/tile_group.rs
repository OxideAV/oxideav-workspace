//! AV1 tile group OBU — §5.11.
//!
//! This module is intentionally tiny. Tile *decode* (CDF, transforms,
//! intra / inter prediction, deblock, CDEF, loop restoration) is the bulk of
//! a full AV1 decoder and is explicitly out of scope for the parse-only
//! crate. We expose just enough surface here to detect the OBU type and
//! return a precise `Unsupported` error pointing at the exact spec sections
//! that remain.

use oxideav_core::Error;

/// Build the standard "tile decode unsupported" error with a precise
/// reference list. Used by both the decoder and external callers that wish
/// to report the same boundary.
pub fn tile_decode_unsupported() -> Error {
    Error::unsupported(
        "AV1 tile decode pending: §5.11.X (CDF + transforms + intra/inter prediction \
         + loop restoration). Parse-only build — frame header is recovered, \
         pixel reconstruction is not implemented.",
    )
}
