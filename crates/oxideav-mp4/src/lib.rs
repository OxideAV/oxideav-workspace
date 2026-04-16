//! Pure-Rust MP4 / ISO Base Media File Format container.
//!
//! Scope: demuxer for probe + remux of audio and video tracks, plus a
//! moov-at-end muxer with optional faststart (moov-at-front) rewrite.
//! Three brand presets are registered: `mp4`, `mov`, and `ismv` — all
//! share one implementation and only differ in their `ftyp` preset.

pub mod boxes;
pub mod codec_id;
pub mod demux;
pub mod muxer;
pub mod options;
mod sample_entries;

pub use options::{BrandPreset, Mp4MuxerOptions};

use oxideav_container::ContainerRegistry;

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("mp4", demux::open);
    reg.register_muxer("mp4", muxer::open);
    reg.register_muxer("mov", muxer::open_mov);
    reg.register_muxer("ismv", muxer::open_ismv);
    reg.register_extension("mp4", "mp4");
    reg.register_extension("m4a", "mp4");
    reg.register_extension("m4v", "mp4");
    reg.register_extension("mov", "mov");
    reg.register_extension("3gp", "mp4");
    reg.register_extension("ismv", "ismv");
    reg.register_probe("mp4", probe);
}

/// `....ftyp` at offset 0 — ISO base media file format. Some files lead
/// with a `wide` or `free` box before `ftyp`, so accept that with a
/// slightly lower confidence.
fn probe(p: &oxideav_container::ProbeData) -> u8 {
    if p.buf.len() < 8 {
        return 0;
    }
    if &p.buf[4..8] == b"ftyp" {
        return 100;
    }
    if p.buf.len() >= 16
        && matches!(&p.buf[4..8], b"wide" | b"free" | b"skip")
        && &p.buf[12..16] == b"ftyp"
    {
        return 90;
    }
    // QuickTime sometimes writes `moov` first, no `ftyp`.
    if &p.buf[4..8] == b"moov" {
        return 50;
    }
    0
}
