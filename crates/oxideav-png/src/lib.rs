//! Pure-Rust PNG + APNG codec and container.
//!
//! Supports (decode):
//! * colour type 0 (grayscale) — 8-bit, 16-bit
//! * colour type 2 (RGB) — 8-bit, 16-bit
//! * colour type 3 (palette) — 8-bit
//! * colour type 4 (grayscale + alpha) — 8-bit, 16-bit
//! * colour type 6 (RGBA) — 8-bit, 16-bit
//! * all five PNG row filters (None / Sub / Up / Average / Paeth)
//! * multiple IDAT chunks
//! * PLTE + tRNS palettes
//! * APNG animation: `acTL`, `fcTL`, `fdAT` with `None`/`Background`/`Previous`
//!   disposal and `Source`/`Over` blending.
//!
//! Supports (encode):
//! * `Rgba` / `Rgb24` / `Gray8` / `Pal8` at 8-bit
//! * `Rgb48Le` / `Rgba64Le` / `Gray16Le` at 16-bit
//! * `Ya8` grayscale + alpha
//! * Single IDAT, DEFLATE via `miniz_oxide`, per-row heuristic filter
//!   selection (PNG §12.8 min-sum-abs-delta).
//! * APNG: `acTL` + per-frame `fcTL`/`fdAT` when `frame_rate` is set or
//!   more than one frame is submitted.
//!
//! Not implemented:
//! * Adam7 interlacing
//! * colour type 3 with sub-byte bit depths (1/2/4-bit palette)
//! * colour type 0 with 1/2/4-bit grayscale
//! * cICP / sRGB / gAMA / cHRM colour management (chunks are ignored but
//!   CRC'd, so they round-trip through the container transparently).

pub mod apng;
pub mod chunk;
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod filter;

pub use decoder::CODEC_ID_STR;

/// Register the PNG codec (both decoder and encoder).
pub fn register_codecs(reg: &mut oxideav_codec::CodecRegistry) {
    container::register_codecs(reg);
}

/// Register the PNG / APNG container (demuxer + muxer + extensions + probe).
pub fn register_containers(reg: &mut oxideav_container::ContainerRegistry) {
    container::register_containers(reg);
}

/// Combined registration: codecs + containers.
pub fn register(
    codecs: &mut oxideav_codec::CodecRegistry,
    containers: &mut oxideav_container::ContainerRegistry,
) {
    register_codecs(codecs);
    register_containers(containers);
}
