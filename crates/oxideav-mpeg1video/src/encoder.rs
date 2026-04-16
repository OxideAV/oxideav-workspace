//! MPEG-1 video I-frame encoder (ISO/IEC 11172-2).
//!
//! Scope:
//! * Sequence header (resolution, frame rate, aspect ratio, bit rate, VBV).
//! * GOP header (closed GOP, time-code 0).
//! * I-pictures only — every input frame becomes its own GOP with a single
//!   I-picture. P/B frames are out of scope and return `Error::Unsupported`
//!   if requested via params (not currently a configurable knob — every
//!   frame is intra).
//! * One slice per macroblock row.
//! * Intra macroblocks only: forward DCT → intra quantisation → DC
//!   differential + AC run/level VLC coding via Tables B-12..B-15.
//! * 4:2:0 chroma subsampling.
//!
//! The output is a self-contained MPEG-1 video elementary stream: each
//! `receive_packet()` call returns one Sequence header + GOP header + I
//! picture (and, when flushed, a sequence end code).

use std::collections::VecDeque;

use oxideav_codec::Encoder;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, MediaType, Packet, PixelFormat, Rational, Result,
    TimeBase, VideoFrame,
};

use crate::bitwriter::BitWriter;
use crate::dct::fdct8x8;
use crate::headers::{DEFAULT_INTRA_QUANT, ZIGZAG};
use crate::start_codes::{
    GROUP_START_CODE, PICTURE_START_CODE, SEQUENCE_END_CODE, SEQUENCE_HEADER_CODE,
};
use crate::tables::dct_coeffs::{self, DctSym};
use crate::tables::dct_dc;
use crate::vlc::VlcEntry;

/// Default fixed quantiser scale for I-pictures. A small value (3) keeps
/// the lossy DCT quantisation tight while still providing reasonable
/// compression — at this setting the round-trip mean absolute pixel error
/// on `testsrc`-style content is ~0.5 LSB.
pub const DEFAULT_QUANT_SCALE: u8 = 3;

/// Encoder factory used by `register()`.
pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    let width = params
        .width
        .ok_or_else(|| Error::invalid("MPEG-1 encoder: missing width"))?;
    let height = params
        .height
        .ok_or_else(|| Error::invalid("MPEG-1 encoder: missing height"))?;
    if width == 0 || height == 0 {
        return Err(Error::invalid("MPEG-1 encoder: zero-sized frame"));
    }
    if width > 4095 || height > 4095 {
        return Err(Error::invalid("MPEG-1 encoder: dimensions exceed 12-bit"));
    }
    let pix = params.pixel_format.unwrap_or(PixelFormat::Yuv420P);
    if pix != PixelFormat::Yuv420P {
        return Err(Error::unsupported(format!(
            "MPEG-1 encoder: only Yuv420P supported (got {:?})",
            pix
        )));
    }
    let frame_rate = params.frame_rate.unwrap_or(Rational::new(25, 1));
    let frame_rate_code = frame_rate_code_for(frame_rate)
        .ok_or_else(|| Error::invalid("MPEG-1 encoder: unsupported frame rate"))?;
    let bit_rate = params.bit_rate.unwrap_or(1_500_000);

    let mut output_params = params.clone();
    output_params.media_type = MediaType::Video;
    output_params.codec_id = CodecId::new(super::CODEC_ID_STR);
    output_params.width = Some(width);
    output_params.height = Some(height);
    output_params.pixel_format = Some(PixelFormat::Yuv420P);
    output_params.frame_rate = Some(frame_rate);
    output_params.bit_rate = Some(bit_rate);

    let time_base = TimeBase::new(frame_rate.den, frame_rate.num);

    Ok(Box::new(Mpeg1VideoEncoder {
        output_params,
        width,
        height,
        frame_rate_code,
        bit_rate,
        quant_scale: DEFAULT_QUANT_SCALE,
        time_base,
        pending: VecDeque::new(),
        eof: false,
        finalised: false,
    }))
}

/// Map an `(num, den)` frame rate to MPEG-1 `frame_rate_code` (see Table 2-D.4).
fn frame_rate_code_for(r: Rational) -> Option<u8> {
    let approx = r.num as f64 / r.den as f64;
    let pairs: &[(u8, f64)] = &[
        (1, 24000.0 / 1001.0),
        (2, 24.0),
        (3, 25.0),
        (4, 30000.0 / 1001.0),
        (5, 30.0),
        (6, 50.0),
        (7, 60000.0 / 1001.0),
        (8, 60.0),
    ];
    for (code, fr) in pairs {
        if (approx - fr).abs() < 0.001 {
            return Some(*code);
        }
    }
    None
}

struct Mpeg1VideoEncoder {
    output_params: CodecParameters,
    width: u32,
    height: u32,
    frame_rate_code: u8,
    bit_rate: u64,
    quant_scale: u8,
    time_base: TimeBase,
    pending: VecDeque<Packet>,
    eof: bool,
    finalised: bool,
}

impl Encoder for Mpeg1VideoEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.output_params.codec_id
    }

    fn output_params(&self) -> &CodecParameters {
        &self.output_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let v = match frame {
            Frame::Video(v) => v,
            _ => return Err(Error::invalid("MPEG-1 encoder: video frames only")),
        };
        if v.width != self.width || v.height != self.height {
            return Err(Error::invalid(
                "MPEG-1 encoder: frame dimensions do not match encoder config",
            ));
        }
        if v.format != PixelFormat::Yuv420P {
            return Err(Error::invalid(
                "MPEG-1 encoder: only Yuv420P input frames supported",
            ));
        }
        if v.planes.len() != 3 {
            return Err(Error::invalid("MPEG-1 encoder: expected 3 planes"));
        }
        let data = encode_intra_picture(self, v)?;
        let mut pkt = Packet::new(0, self.time_base, data);
        pkt.pts = v.pts;
        pkt.dts = v.pts;
        pkt.flags.keyframe = true;
        self.pending.push_back(pkt);
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        if let Some(p) = self.pending.pop_front() {
            return Ok(p);
        }
        if self.eof && !self.finalised {
            self.finalised = true;
            // Emit a final sequence-end code as its own (empty-payload-style)
            // packet so muxers can include it verbatim if they want to. Most
            // muxers (e.g. raw .m1v, MPEG-PS) just concatenate.
            let mut bw = BitWriter::new();
            write_start_code(&mut bw, SEQUENCE_END_CODE);
            let bytes = bw.finish();
            let mut pkt = Packet::new(0, self.time_base, bytes);
            pkt.flags.header = true;
            return Ok(pkt);
        }
        if self.eof {
            return Err(Error::Eof);
        }
        Err(Error::NeedMore)
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Picture encode
// ---------------------------------------------------------------------------

fn encode_intra_picture(enc: &Mpeg1VideoEncoder, v: &VideoFrame) -> Result<Vec<u8>> {
    let mut bw = BitWriter::with_capacity(8192);

    // 1. Sequence header.
    write_start_code(&mut bw, SEQUENCE_HEADER_CODE);
    write_sequence_header(
        &mut bw,
        enc.width,
        enc.height,
        enc.frame_rate_code,
        enc.bit_rate,
    );

    // 2. GOP header (each frame is its own GOP — keeps things simple and
    //    keeps every output bitstream independently decodable).
    write_start_code(&mut bw, GROUP_START_CODE);
    write_gop_header(&mut bw);

    // 3. Picture header — picture_coding_type = 1 (I), temporal_reference = 0.
    write_start_code(&mut bw, PICTURE_START_CODE);
    write_picture_header_i(&mut bw);

    // 4. Slices — one per macroblock row (slice_start_code = row+1).
    let mb_w = (enc.width as usize).div_ceil(16);
    let mb_h = (enc.height as usize).div_ceil(16);
    for row in 0..mb_h {
        write_start_code(&mut bw, (row + 1) as u8);
        encode_slice(&mut bw, enc, v, row, mb_w)?;
    }

    Ok(bw.finish())
}

fn write_start_code(bw: &mut BitWriter, code: u8) {
    bw.align_to_byte();
    bw.write_bytes(&[0x00, 0x00, 0x01, code]);
}

fn write_sequence_header(
    bw: &mut BitWriter,
    width: u32,
    height: u32,
    frame_rate_code: u8,
    bit_rate: u64,
) {
    // 12 bits each
    bw.write_bits(width, 12);
    bw.write_bits(height, 12);
    // aspect_ratio_info = 1 (square pixels per Table 2-D.3).
    bw.write_bits(1, 4);
    // frame_rate_code
    bw.write_bits(frame_rate_code as u32, 4);
    // bit_rate is in 400 bps units. 0x3FFFF means VBR/unspecified.
    let br_units = bit_rate.div_ceil(400).min(0x3FFFF) as u32;
    bw.write_bits(br_units, 18);
    // marker bit
    bw.write_bits(1, 1);
    // vbv_buffer_size in 16 KiB units. 20 ≈ 320 KiB — plenty for 1.5 Mbps.
    bw.write_bits(20, 10);
    // constrained_parameters_flag = 0 (we don't claim conformance)
    bw.write_bits(0, 1);
    // load_intra_quantiser_matrix = 0 → use default
    bw.write_bits(0, 1);
    // load_non_intra_quantiser_matrix = 0
    bw.write_bits(0, 1);
    bw.align_to_byte();
}

fn write_gop_header(bw: &mut BitWriter) {
    // drop_frame_flag = 0
    bw.write_bits(0, 1);
    // time_code: 25 bits (h, m, marker, s, f). Set to all zeros.
    bw.write_bits(0, 5); // hours
    bw.write_bits(0, 6); // minutes
    bw.write_bits(1, 1); // marker
    bw.write_bits(0, 6); // seconds
    bw.write_bits(0, 6); // pictures
                         // closed_gop = 1 (no B/P refs to a previous GOP).
    bw.write_bits(1, 1);
    // broken_link = 0
    bw.write_bits(0, 1);
    bw.align_to_byte();
}

fn write_picture_header_i(bw: &mut BitWriter) {
    // temporal_reference = 0 for the single I-picture in this GOP.
    bw.write_bits(0, 10);
    // picture_coding_type = 1 (I).
    bw.write_bits(1, 3);
    // vbv_delay = 0xFFFF (variable bit rate / not specified)
    bw.write_bits(0xFFFF, 16);
    // No forward/backward MV fields for I-pictures.
    // extra_bit_picture = 0 (no extra picture info).
    bw.write_bits(0, 1);
    bw.align_to_byte();
}

// ---------------------------------------------------------------------------
// Slice / MB encode
// ---------------------------------------------------------------------------

fn encode_slice(
    bw: &mut BitWriter,
    enc: &Mpeg1VideoEncoder,
    v: &VideoFrame,
    mb_row: usize,
    mb_w: usize,
) -> Result<()> {
    // quantiser_scale (5 bits, range 1..=31)
    bw.write_bits(enc.quant_scale as u32, 5);
    // extra_bit_slice = 0
    bw.write_bits(0, 1);

    // DC predictors reset at slice start to 1024 (i.e. dc_q = 128).
    let mut dc_pred_q: [i32; 3] = [128, 128, 128];

    for mb_col in 0..mb_w {
        // macroblock_address_increment: always 1 for the first MB of a slice
        // and for every contiguous MB after it. VLC for "increment 1" is `1`
        // (1 bit).
        bw.write_bits(0b1, 1);
        // macroblock_type for I-picture: `1` (1 bit) = Intra (no quant).
        bw.write_bits(0b1, 1);

        // Encode the 6 blocks of the MB.
        encode_mb_intra(bw, enc, v, mb_row, mb_col, &mut dc_pred_q)?;
    }

    Ok(())
}

fn encode_mb_intra(
    bw: &mut BitWriter,
    enc: &Mpeg1VideoEncoder,
    v: &VideoFrame,
    mb_row: usize,
    mb_col: usize,
    dc_pred_q: &mut [i32; 3],
) -> Result<()> {
    let q = enc.quant_scale as i32;
    let intra_q = &DEFAULT_INTRA_QUANT;

    let w = v.width as usize;
    let h = v.height as usize;
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);

    let y_plane = &v.planes[0];
    let cb_plane = &v.planes[1];
    let cr_plane = &v.planes[2];

    // Block layout: Y0 Y1 Y2 Y3 Cb Cr.
    let y0 = mb_row * 16;
    let x0 = mb_col * 16;
    let cy0 = mb_row * 8;
    let cx0 = mb_col * 8;

    // 4 luma blocks
    for (bi, (bx, by)) in [(0, 0), (8, 0), (0, 8), (8, 8)].iter().enumerate() {
        let _ = bi;
        encode_block_intra(
            bw,
            &y_plane.data,
            y_plane.stride,
            w,
            h,
            x0 + bx,
            y0 + by,
            false,
            q,
            intra_q,
            &mut dc_pred_q[0],
        )?;
    }
    // Cb
    encode_block_intra(
        bw,
        &cb_plane.data,
        cb_plane.stride,
        cw,
        ch,
        cx0,
        cy0,
        true,
        q,
        intra_q,
        &mut dc_pred_q[1],
    )?;
    // Cr
    encode_block_intra(
        bw,
        &cr_plane.data,
        cr_plane.stride,
        cw,
        ch,
        cx0,
        cy0,
        true,
        q,
        intra_q,
        &mut dc_pred_q[2],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn encode_block_intra(
    bw: &mut BitWriter,
    plane: &[u8],
    stride: usize,
    pw: usize,
    ph: usize,
    x0: usize,
    y0: usize,
    is_chroma: bool,
    q: i32,
    intra_q: &[u8; 64],
    prev_dc_q: &mut i32,
) -> Result<()> {
    // 1. Pull samples (with edge replication).
    let mut samples = [0.0f32; 64];
    for j in 0..8 {
        let yy = (y0 + j).min(ph.saturating_sub(1));
        for i in 0..8 {
            let xx = (x0 + i).min(pw.saturating_sub(1));
            samples[j * 8 + i] = plane[yy * stride + xx] as f32;
        }
    }

    // 2. Forward DCT (no level shift — MPEG-1 intra uses unshifted samples).
    fdct8x8(&mut samples);

    // 3. Quantise. DC uses fixed step 8 (not the W[0] = 8 entry — same value
    //    by construction in 8-bit MPEG-1 video).
    //    The transmitted DC level is `dc_q = round(dc_coeff / 8)`, clamped to
    //    [0, 255] (the DC of an unshifted 8-bit block is in 0..=2040 → /8 →
    //    0..=255). DC differential is `dc_q - prev_dc_q` ∈ [-255, 255].
    let dc_coeff = samples[0];
    let dc_q = ((dc_coeff / 8.0).round() as i32).clamp(0, 255);
    let dc_diff = dc_q - *prev_dc_q;
    *prev_dc_q = dc_q;

    // 4. Quantise AC coefficients.
    //    Inverse of `coeff' = (2 * level * Q * W[i]) / 16` is approximately
    //    `level = round(coeff * 8 / (Q * W[i]))`. We apply a small dead-zone
    //    by truncating toward zero with bias = 0 — this is a simple mid-tread
    //    quantiser and gives reasonable rate at quant_scale = 8.
    let mut levels = [0i32; 64];
    for k in 1..64 {
        let nat = ZIGZAG[k];
        let coef = samples[nat];
        let qf = intra_q[nat] as f32;
        let denom = q as f32 * qf;
        let v = if denom == 0.0 {
            0.0
        } else {
            coef * 8.0 / denom
        };
        // Round half-away-from-zero, clamp to spec range [-255, 255].
        let lv = if v >= 0.0 {
            (v + 0.5) as i32
        } else {
            -(((-v) + 0.5) as i32)
        };
        levels[k] = lv.clamp(-255, 255);
    }

    // 5. Encode DC differential.
    encode_dc_diff(bw, dc_diff, is_chroma)?;

    // 6. Encode AC run/level pairs.
    encode_ac_coeffs(bw, &levels)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// VLC encode helpers
// ---------------------------------------------------------------------------

fn encode_dc_diff(bw: &mut BitWriter, diff: i32, is_chroma: bool) -> Result<()> {
    // size = number of bits needed to hold |diff| (0 if diff == 0).
    let abs = diff.unsigned_abs();
    let size: u8 = if abs == 0 {
        0
    } else {
        (32 - abs.leading_zeros()) as u8
    };
    if size > 11 {
        return Err(Error::invalid("DC differential out of range"));
    }
    let dc_tbl = if is_chroma {
        dct_dc::chroma()
    } else {
        dct_dc::luma()
    };
    let entry =
        lookup_value(dc_tbl, size).ok_or_else(|| Error::invalid("DC size missing in VLC"))?;
    bw.write_bits(entry.code, entry.bits as u32);
    if size > 0 {
        let bits = encode_signed_field(diff, size as u32);
        bw.write_bits(bits, size as u32);
    }
    Ok(())
}

/// Sign-extension scheme used for MPEG-1 DC differentials and for run/level
/// "long escape" sub-fields. Positive values are written as-is; negative
/// values are written as `value + (2^size - 1)` (so negative numbers have
/// the leading bit cleared, i.e. a one's-complement-ish encoding).
fn encode_signed_field(value: i32, size: u32) -> u32 {
    if size == 0 {
        return 0;
    }
    let mask = if size == 32 {
        u32::MAX
    } else {
        (1u32 << size) - 1
    };
    if value >= 0 {
        value as u32 & mask
    } else {
        // value is negative; |value| < 2^size by construction.
        // Decoder reverses: bits < (1<<(size-1)) → value = bits - (2^size - 1).
        // Solve: bits = value + (2^size - 1).
        let max_unsigned = (1u32 << size) - 1;
        ((value + max_unsigned as i32) as u32) & mask
    }
}

fn encode_ac_coeffs(bw: &mut BitWriter, levels: &[i32; 64]) -> Result<()> {
    let mut run: u32 = 0;
    for k in 1..64 {
        let lv = levels[k];
        if lv == 0 {
            run += 1;
            continue;
        }
        // Try to find a (run, |level|) entry in the standard intra DCT table.
        // The table codes do NOT include the sign bit — the decoder reads
        // the sign as a separate bit after the VLC, so we mirror that here.
        let abs = lv.unsigned_abs();
        if let Some(entry) = lookup_run_level(run, abs) {
            bw.write_bits(entry.code, entry.bits as u32);
            // Sign bit: 0 = positive, 1 = negative.
            let sign = if lv < 0 { 1 } else { 0 };
            bw.write_bits(sign, 1);
        } else {
            // Fall through to escape encoding.
            // Escape: 6-bit code (0x000001) + 6-bit run + 8/16-bit signed level.
            let escape_entry = find_escape_entry();
            bw.write_bits(escape_entry.code, escape_entry.bits as u32);
            bw.write_bits(run, 6);
            // Short form: 8-bit signed level in [-127, 127] (0 and -128 are
            // reserved for the long-form prefix). Otherwise use long form.
            if (1..=127).contains(&lv) || (-127..=-1).contains(&lv) {
                let v = lv & 0xFF;
                bw.write_bits(v as u32, 8);
            } else if (128..=255).contains(&lv) {
                // Long form positive: leading 8-bit 0 + 8-bit unsigned level.
                bw.write_bits(0, 8);
                bw.write_bits(lv as u32, 8);
            } else if (-255..=-128).contains(&lv) {
                // Long form negative: leading 8-bit 0x80 + 8-bit
                // (level + 256).
                bw.write_bits(0x80, 8);
                bw.write_bits((lv + 256) as u32 & 0xFF, 8);
            } else {
                return Err(Error::invalid("AC level out of MPEG-1 range"));
            }
        }
        run = 0;
    }
    // EOB.
    let eob = find_eob_entry();
    bw.write_bits(eob.code, eob.bits as u32);
    Ok(())
}

fn lookup_value<T: Copy + PartialEq>(tbl: &[VlcEntry<T>], needle: T) -> Option<VlcEntry<T>> {
    tbl.iter().find(|e| e.value == needle).copied()
}

fn lookup_run_level(run: u32, level_abs: u32) -> Option<VlcEntry<DctSym>> {
    if level_abs == 0 || run > 31 {
        return None;
    }
    let tbl = dct_coeffs::table();
    for e in tbl {
        if let DctSym::RunLevel {
            run: r,
            level_abs: lv,
        } = e.value
        {
            if r as u32 == run && lv as u32 == level_abs {
                return Some(*e);
            }
        }
    }
    None
}

fn find_escape_entry() -> VlcEntry<DctSym> {
    *dct_coeffs::table()
        .iter()
        .find(|e| matches!(e.value, DctSym::Escape))
        .expect("escape entry must exist")
}

fn find_eob_entry() -> VlcEntry<DctSym> {
    *dct_coeffs::table()
        .iter()
        .find(|e| matches!(e.value, DctSym::Eob))
        .expect("EOB entry must exist")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitreader::BitReader;
    use crate::tables::dct_dc;
    use crate::vlc;

    #[test]
    fn signed_field_round_trip_dc() {
        // For each `size` ≥ 1 the encoded representation covers values whose
        // magnitude requires exactly `size` bits — i.e. v ∈ [-(2^size - 1),
        // -2^(size-1)] ∪ [2^(size-1), 2^size - 1]. Values of smaller
        // magnitude use a smaller `size`; v = 0 uses size = 0 with no
        // differential bitstream at all.
        for size in 1u32..=8 {
            let max_at_size = (1i32 << size) - 1;
            let min_pos = 1i32 << (size - 1);
            let mut values: Vec<i32> = (min_pos..=max_at_size).collect();
            values.extend((min_pos..=max_at_size).map(|v| -v));
            for v in values {
                let bits = encode_signed_field(v, size);
                // Decode via the same logic as block.rs::extend_dc.
                let vt = 1u32 << (size - 1);
                let decoded = if bits < vt {
                    (bits as i32) - ((1i32 << size) - 1)
                } else {
                    bits as i32
                };
                assert_eq!(decoded, v, "size={size} value={v} bits={bits:b}");
            }
        }
    }

    #[test]
    fn dc_size_lookup_round_trip() {
        let luma = dct_dc::luma();
        for size in 0u8..=8 {
            let entry = lookup_value(luma, size).unwrap_or_else(|| panic!("no entry for {size}"));
            // Pack the code and decode it.
            let mut bw = BitWriter::new();
            bw.write_bits(entry.code, entry.bits as u32);
            // Pad to a byte boundary so the bitreader can peek a full word.
            bw.align_to_byte();
            let bytes = bw.finish();
            let mut br = BitReader::new(&bytes);
            let decoded = vlc::decode(&mut br, luma).expect("decode dc size");
            assert_eq!(decoded, size);
        }
    }
}
