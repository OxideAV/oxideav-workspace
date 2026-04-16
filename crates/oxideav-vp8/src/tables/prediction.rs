//! Constants used by intra prediction.
//!
//! VP8's keyframe luma intra prediction probability table for the 16×16
//! mode is fixed (see `KF_YMODE_PROBS` in `trees.rs`). For 4×4 sub-blocks
//! the probabilities depend on the modes of the up- and left-neighbours;
//! that table (`kf_bmode_prob`) lives in `trees.rs` too.
//!
//! Nothing prediction-related is needed beyond the trees module; this
//! file is reserved for future use.

#[allow(dead_code)]
pub const _MARKER: () = ();
