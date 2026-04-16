//! Hand-crafted VP8L bitstream tests.
//!
//! Each test builds a minimal VP8L blob bit-by-bit and decodes it through
//! the public [`oxideav_webp::decode_webp`] entry point (wrapped in a
//! synthetic RIFF/WEBP container) so the bit reader, simple-Huffman path,
//! and at least one transform are all exercised end-to-end.
//!
//! The intent is "no fixture file required": the tests are reproducible
//! from source and double as documentation for the bitstream layout.

use oxideav_webp::decode_webp;

/// LSB-first bit writer matching the VP8L bit reader's convention.
struct BitWriter {
    out: Vec<u8>,
    cur: u32,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            cur: 0,
            nbits: 0,
        }
    }

    fn write(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32);
        self.cur |= (value & ((1u64 << n) as u32).wrapping_sub(1)) << self.nbits;
        self.nbits += n;
        while self.nbits >= 8 {
            self.out.push((self.cur & 0xff) as u8);
            self.cur >>= 8;
            self.nbits -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.out.push((self.cur & 0xff) as u8);
        }
        self.out
    }
}

/// Wrap a VP8L payload in a minimal RIFF/WEBP container so we can drive it
/// through the public `decode_webp` entry point.
fn wrap_in_riff(vp8l: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + vp8l.len());
    out.extend_from_slice(b"RIFF");
    let riff_size = (4 + 8 + vp8l.len() + (vp8l.len() & 1)) as u32;
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(b"VP8L");
    out.extend_from_slice(&(vp8l.len() as u32).to_le_bytes());
    out.extend_from_slice(vp8l);
    if vp8l.len() & 1 == 1 {
        out.push(0);
    }
    out
}

/// Emit a "simple" Huffman tree with a single 8-bit symbol. Layout
/// (per VP8L spec §6.2.4):
///   bit 0: simple = 1
///   bit 1: num_symbols-1 = 0  (= 1 symbol)
///   bit 2: is_first_8bits = 1
///   bits 3..10: symbol value
fn write_simple_one_symbol_tree(bw: &mut BitWriter, sym: u32) {
    bw.write(1, 1); // simple
    bw.write(0, 1); // num_symbols - 1 = 0 → one symbol
    bw.write(1, 1); // is_first_8bits = 1 → 8-bit symbol
    bw.write(sym & 0xff, 8);
}

/// Build a minimal 2x2 VP8L bitstream where every pixel literally decodes
/// to the same ARGB constant. No transforms, no color cache, no meta-
/// Huffman — just five single-symbol simple Huffman trees and four pixels.
///
/// `(a, r, g, b)` describes the expected per-channel constant.
fn build_constant_2x2_vp8l(a: u8, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut bw = BitWriter::new();
    // ── Header ─────────────────────────────────────────────────────────
    bw.write(0x2f, 8); // signature
    bw.write(2 - 1, 14); // width-1
    bw.write(2 - 1, 14); // height-1
    bw.write(if a != 0xff { 1 } else { 0 }, 1); // alpha_is_used
    bw.write(0, 3); // version
    bw.write(0, 1); // no transforms

    // ── Main image stream (main_image=true) ───────────────────────────
    bw.write(0, 1); // no color cache
    bw.write(0, 1); // no meta-Huffman image (single group)

    // Five single-symbol trees: green, red, blue, alpha, distance.
    write_simple_one_symbol_tree(&mut bw, g as u32);
    write_simple_one_symbol_tree(&mut bw, r as u32);
    write_simple_one_symbol_tree(&mut bw, b as u32);
    write_simple_one_symbol_tree(&mut bw, a as u32);
    write_simple_one_symbol_tree(&mut bw, 0); // unused distance code

    // No pixel-stream bits needed: every Huffman tree is a single-symbol
    // shortcut, so the decoder consumes zero bits per pixel and walks
    // through 4 literal-green emits to fill the 2x2 image.

    bw.finish()
}

/// Build a 2x2 VP8L bitstream that exercises the *subtract-green*
/// transform. Strategy:
///   * 1 transform: SubtractGreen (no parameters).
///   * Main image residual: every pixel ARGB = (a, r-g mod 256, g, b-g mod 256)
///     using single-symbol simple Huffman trees (so the value is constant
///     for every pixel and no bits are consumed in the pixel loop).
///   * On decode the transform recomputes `r += g; b += g`, restoring
///     `(a, r, g, b)`.
///
/// This proves the transform-parse + reverse-apply pipeline runs end-to-
/// end and that a parameter-less transform composes correctly with the
/// constant-Huffman fast path.
fn build_subtract_green_2x2_vp8l(a: u8, r: u8, g: u8, b: u8) -> Vec<u8> {
    let r_resid = r.wrapping_sub(g);
    let b_resid = b.wrapping_sub(g);
    let mut bw = BitWriter::new();
    // Header.
    bw.write(0x2f, 8);
    bw.write(1, 14); // w-1 = 1
    bw.write(1, 14); // h-1 = 1
    bw.write(if a != 0xff { 1 } else { 0 }, 1); // alpha_is_used
    bw.write(0, 3); // version

    // One transform, type 2 = SubtractGreen.
    bw.write(1, 1); // transform present
    bw.write(2, 2); // type 2
                    // SubtractGreen carries no parameters.
    bw.write(0, 1); // no further transforms

    // ── Main image stream ──────────────────────────────────────────────
    bw.write(0, 1); // no color cache
    bw.write(0, 1); // no meta-Huffman image

    // Five single-symbol trees emitting the residual values.
    write_simple_one_symbol_tree(&mut bw, g as u32);
    write_simple_one_symbol_tree(&mut bw, r_resid as u32);
    write_simple_one_symbol_tree(&mut bw, b_resid as u32);
    write_simple_one_symbol_tree(&mut bw, a as u32);
    write_simple_one_symbol_tree(&mut bw, 0);

    bw.finish()
}

#[test]
fn vp8l_2x2_constant_pixel() {
    // Decode a hand-built 2x2 image where every pixel is ARGB(ff, 80, 40, 20).
    let blob = build_constant_2x2_vp8l(0xff, 0x80, 0x40, 0x20);
    let riff = wrap_in_riff(&blob);
    let img = decode_webp(&riff).expect("decode 2x2 constant VP8L");
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 2);
    assert_eq!(img.frames.len(), 1);
    let f = &img.frames[0];
    assert_eq!(f.rgba.len(), 4 * 4);
    for i in 0..4 {
        let r = f.rgba[i * 4];
        let g = f.rgba[i * 4 + 1];
        let b = f.rgba[i * 4 + 2];
        let a = f.rgba[i * 4 + 3];
        assert_eq!(
            (r, g, b, a),
            (0x80, 0x40, 0x20, 0xff),
            "pixel {i} mismatch: got rgba=({r:#04x}, {g:#04x}, {b:#04x}, {a:#04x})"
        );
    }
}

/// Build a 2x2 VP8L bitstream that exercises the *predictor* transform
/// with mode 0 ("opaque black"). Residual is constant `(0, R, G, B)` so
/// the top-left and bottom-right pixels end up at `(ff, R, G, B)` (their
/// pred is ff_00_00_00 — top-left by spec, bottom-right by mode 0). The
/// two edge pixels get pred = the *previously decoded* pixel and so end
/// up at `(ff, 2R, 2G, 2B)`.
fn build_predictor_2x2_vp8l(r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut bw = BitWriter::new();
    // Header.
    bw.write(0x2f, 8);
    bw.write(1, 14); // w-1 = 1
    bw.write(1, 14); // h-1 = 1
    bw.write(0, 1); // alpha_is_used
    bw.write(0, 3); // version

    // One transform: type 0 = Predictor, tile_bits = 0+2 = 2.
    bw.write(1, 1); // transform present
    bw.write(0, 2); // type 0 = Predictor
    bw.write(0, 3); // tile_bits raw = 0 → tile_bits = 2 → 1×1 sub-image

    // Predictor sub-image: 1×1, single pixel whose green = mode 0.
    bw.write(0, 1); // no color cache (sub-image)
    write_simple_one_symbol_tree(&mut bw, 0); // green = mode 0
    write_simple_one_symbol_tree(&mut bw, 0);
    write_simple_one_symbol_tree(&mut bw, 0);
    write_simple_one_symbol_tree(&mut bw, 0);
    write_simple_one_symbol_tree(&mut bw, 0);
    // 1×1 sub-image → 1 pixel decoded with single-symbol trees → no extra
    // bits.

    bw.write(0, 1); // no further transforms

    // ── Main image stream ──────────────────────────────────────────────
    bw.write(0, 1); // no color cache
    bw.write(0, 1); // no meta-Huffman image

    // Residuals: (alpha=0, red=R, green=G, blue=B), single-symbol.
    write_simple_one_symbol_tree(&mut bw, g as u32);
    write_simple_one_symbol_tree(&mut bw, r as u32);
    write_simple_one_symbol_tree(&mut bw, b as u32);
    write_simple_one_symbol_tree(&mut bw, 0); // alpha residual
    write_simple_one_symbol_tree(&mut bw, 0); // unused distance

    bw.finish()
}

#[test]
fn vp8l_2x2_predictor_transform() {
    // Decode a 2x2 image whose predictor pipeline yields a known
    // diagonal pattern.
    let blob = build_predictor_2x2_vp8l(0x10, 0x20, 0x30);
    let riff = wrap_in_riff(&blob);
    let img = decode_webp(&riff).expect("decode 2x2 predictor VP8L");
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 2);
    let f = &img.frames[0];
    // Per the analysis in `build_predictor_2x2_vp8l`:
    //   pixel 0 (top-left)     → (ff, R, G, B)        = (ff, 10, 20, 30)
    //   pixel 1 (y=0, x=1)     → (ff, 2R, 2G, 2B)     = (ff, 20, 40, 60)
    //   pixel 2 (y=1, x=0)     → (ff, 2R, 2G, 2B)     = (ff, 20, 40, 60)
    //   pixel 3 (y=1, x=1) m=0 → (ff, R, G, B)        = (ff, 10, 20, 30)
    let want: [(u8, u8, u8, u8); 4] = [
        (0x10, 0x20, 0x30, 0xff),
        (0x20, 0x40, 0x60, 0xff),
        (0x20, 0x40, 0x60, 0xff),
        (0x10, 0x20, 0x30, 0xff),
    ];
    for (i, exp) in want.iter().enumerate() {
        let r = f.rgba[i * 4];
        let g = f.rgba[i * 4 + 1];
        let b = f.rgba[i * 4 + 2];
        let a = f.rgba[i * 4 + 3];
        assert_eq!(
            &(r, g, b, a),
            exp,
            "pixel {i} (predictor): got rgba=({r:#04x}, {g:#04x}, {b:#04x}, {a:#04x})"
        );
    }
}

#[test]
fn vp8l_2x2_subtract_green_transform() {
    // Decode a 2x2 image whose subtract-green pipeline yields
    // ARGB(ff, 0x90, 0x40, 0x60). The encoded residual is
    // (ff, 0x50, 0x40, 0x20); the transform restores the original.
    let blob = build_subtract_green_2x2_vp8l(0xff, 0x90, 0x40, 0x60);
    let riff = wrap_in_riff(&blob);
    let img = decode_webp(&riff).expect("decode 2x2 subtract-green VP8L");
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 2);
    let f = &img.frames[0];
    for i in 0..4 {
        let r = f.rgba[i * 4];
        let g = f.rgba[i * 4 + 1];
        let b = f.rgba[i * 4 + 2];
        let a = f.rgba[i * 4 + 3];
        assert_eq!(
            (r, g, b, a),
            (0x90, 0x40, 0x60, 0xff),
            "pixel {i} (subtract-green): got rgba=({r:#04x}, {g:#04x}, {b:#04x}, {a:#04x})"
        );
    }
}
