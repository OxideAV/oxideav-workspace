//! Pure-Rust MP4 / ISO Base Media File Format container.
//!
//! Scope: demuxer for probe + remux of audio and video tracks, plus a basic
//! moov-at-end muxer. Fast-start (moov-at-front) remains future work.

pub mod boxes;
pub mod codec_id;
pub mod demux;
pub mod muxer;
mod sample_entries;

use oxideav_container::ContainerRegistry;

pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer("mp4", demux::open);
    reg.register_muxer("mp4", muxer::open);
    reg.register_extension("mp4", "mp4");
    reg.register_extension("m4a", "mp4");
    reg.register_extension("m4v", "mp4");
    reg.register_extension("mov", "mp4");
    reg.register_extension("3gp", "mp4");
}
