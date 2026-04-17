//! Pure-Rust pixel-format conversions for the oxideav framework.
//!
//! This crate extends [`oxideav_core::PixelFormat`] with the converters
//! that the rest of the codec/container ecosystem depends on — RGB/BGR
//! swizzles, YUV↔RGB (BT.601 and BT.709, limited and full range), chroma
//! subsampling changes (4:2:0 ↔ 4:2:2 ↔ 4:4:4), NV12/NV21 ↔ Yuv420P,
//! grayscale expansion, 16-bit/8-bit bit-depth changes, and palette
//! generation + Pal8 encode/decode with optional dithering.
//!
//! # Entry points
//!
//! - [`convert`] — the single conversion function; dispatches on
//!   `(src.format, dst_format)` and returns a freshly allocated
//!   [`VideoFrame`].
//! - [`generate_palette`] — build a [`Palette`] from one or more source
//!   frames, honouring the selected [`PaletteStrategy`].
//! - [`convert_in_place_if_same`] — trivial passthrough helper for the
//!   "no conversion needed" case so callers don't duplicate the check.
//!
//! # Colour science
//!
//! The YUV↔RGB paths are written as scalar integer pipelines against
//! BT.601 and BT.709 weights. The "limited" variants use the studio
//! range (Y in 16..=235, chroma in 16..=240); "full" variants use the
//! full 0..=255 range, matching JPEG / "J" YUV. Every converter clamps
//! to `[0, 255]` after reconstruction. See [`yuv`] for the exact matrix
//! coefficients.
//!
//! # Feature coverage
//!
//! Not every pair in the Cartesian product of [`PixelFormat`] variants
//! is supported; the first-tier matrix is:
//!
//! - RGB family (Rgb24/Bgr24/Rgba/Bgra/Argb/Abgr) all-to-all.
//! - Yuv420P/422P/444P ↔ Rgb24 / Rgba under BT.601 and BT.709, limited
//!   and full range.
//! - YuvJ420P/422P/444P ↔ Yuv* equivalents — plane copy with range
//!   rescale.
//! - Nv12/Nv21 ↔ Yuv420P.
//! - Gray8 ↔ Rgb24/Rgba broadcast.
//! - Rgb48Le ↔ Rgb24, Rgba64Le ↔ Rgba (bit-shift).
//! - Gray16Le ↔ Gray8.
//! - MonoBlack/MonoWhite ↔ Gray8.
//! - Pal8 → Rgb24/Rgba requires `opts.palette`.
//! - Rgb24/Rgba → Pal8 requires `opts.palette`; dithering per `opts.dither`.
//!
//! Anything else returns `Error::Unsupported` — callers handle it or
//! stage through a supported intermediate (most paths go via Rgba).

pub mod convert;
pub mod dither;
pub mod format_info;
pub mod gray;
pub mod pal8;
pub mod palette;
pub mod rgb;
pub mod yuv;

pub use convert::{convert, convert_in_place_if_same, ColorSpace, ConvertOptions, Dither};
pub use format_info::FormatInfo;
pub use palette::{generate_palette, Palette, PaletteGenOptions, PaletteStrategy};
