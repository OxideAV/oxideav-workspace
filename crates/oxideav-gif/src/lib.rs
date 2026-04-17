//! Pure-Rust GIF codec + container.
//!
//! Handles both GIF87a and GIF89a. Decoding supports:
//!
//! * Logical Screen Descriptor with optional Global Color Table.
//! * Per-frame Image Descriptor with optional Local Color Table and the
//!   classic 4-pass interlace.
//! * Graphic Control Extension — delay time, disposal method, transparent
//!   colour index.
//! * Application Extension — NETSCAPE2.0 loop count is surfaced in
//!   container metadata; other application extensions are skipped.
//! * Comment + Plain Text extensions are silently skipped.
//! * LZW decode covering the whole 2..=12 bit code-width ladder, clear
//!   codes, and EOI.
//!
//! Encoding produces GIF89a output:
//!
//! * A Global Color Table sourced from the first frame's palette.
//! * A Graphic Control Extension per frame (delay + disposal).
//! * A NETSCAPE2.0 application extension when writing more than one
//!   frame (loop count = 0 = infinite).
//! * LZW-compressed image data with clear-on-full semantics (clear code
//!   emitted when the dictionary fills at 4096 entries).
//!
//! The encoder requires `Pal8` input. The DAG pipeline resolver will
//! auto-insert a pixfmt conversion when the upstream frame is RGBA.

#![allow(clippy::needless_range_loop)]

pub mod container;
pub mod decoder;
pub mod encoder;
pub mod lzw;

use oxideav_codec::CodecRegistry;
use oxideav_container::ContainerRegistry;
use oxideav_core::{CodecCapabilities, CodecId, PixelFormat};

/// Codec id for GIF image frames.
pub const GIF_CODEC_ID: &str = "gif";

/// Register both the encoder and the decoder under codec id `"gif"`.
pub fn register_codecs(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("gif_sw")
        .with_lossless(true)
        .with_intra_only(true)
        .with_max_size(65535, 65535)
        .with_pixel_format(PixelFormat::Pal8);
    reg.register_both(
        CodecId::new(GIF_CODEC_ID),
        caps,
        decoder::make_decoder,
        encoder::make_encoder,
    );
}

/// Register the GIF container's demuxer + muxer + probe + extension.
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Combined registration — matches the shape of `oxideav_webp::register` etc.
pub fn register(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}

pub use encoder::DEFAULT_DELAY_CS;
pub use lzw::{Lzw, LzwDecoder, LzwEncoder};
