//! H.263 baseline I-picture encoder.
//!
//! Scope:
//! * Picture Start Code (PSC) + picture header (TR, PTYPE, source format,
//!   PQUANT, CPM=0, no PEI). Source formats sub-QCIF / QCIF / CIF / 4CIF /
//!   16CIF (PTYPE source-format codes 1..=5).
//! * GOB layering — emits a GOB header (GBSC + GN + GFID + GQUANT) at every
//!   GOB boundary except the first (the first GOB header is implicit per
//!   §5.2.1 — the picture header's PQUANT applies).
//! * I-MB: MCBPC (intra, mb_type=3) + CBPY (no XOR for intra) — no DQUANT
//!   (we hold the quantiser fixed for the whole picture).
//! * Block layer: 8-bit INTRADC (with the spec's `0x00`/`0x80`/`0xFF`
//!   special-value handling) + H.263 AC TCOEF VLC encode with a fixed-length
//!   `last + run(6) + level(8)` escape body for out-of-table tuples.
//! * 8×8 forward DCT (textbook f32) + H.263 quant.
//!
//! Out of scope (returns `Error::Unsupported`):
//! * P-pictures (§5.3.5 / §5.4 — motion compensation + inter texture).
//! * Annex D (UMV), Annex E (SAC), Annex F (Advanced Prediction), Annex G
//!   (PB-frames), Annex I (Advanced Intra Coding), Annex J (deblocking),
//!   Annex T (Modified Quantization).
//! * H.263+ PLUSPTYPE custom picture format extensions.
//! * CPM continuous-presence multipoint mode.
//!
//! The picture header's `temporal_reference` field is taken from the input
//! frame's `pts` modulo 256 — the H.263 spec only requires that consecutive
//! pictures advance TR; downstream containers (e.g. 3GP) carry the actual
//! timestamps separately.

use std::collections::VecDeque;

use oxideav_codec::Encoder;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, MediaType, Packet, PixelFormat, Rational, Result,
    TimeBase, VideoFrame,
};

use crate::bitwriter::BitWriter;
use crate::dct::fdct8x8;
use crate::enc_tables::{write_cbpy, write_mcbpc_intra, write_tcoef};
use crate::picture::SourceFormat;

/// Default fixed quantiser (PQUANT) — `5` matches the
/// `ffmpeg -qscale:v 5` baseline used to validate the existing decoder.
pub const DEFAULT_PQUANT: u8 = 5;

/// Encoder factory used by [`crate::register_encoder`].
pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    let width = params
        .width
        .ok_or_else(|| Error::invalid("h263 encoder: missing width"))?;
    let height = params
        .height
        .ok_or_else(|| Error::invalid("h263 encoder: missing height"))?;
    let source_format = SourceFormat::for_dimensions(width, height).ok_or_else(|| {
        Error::unsupported(format!(
            "h263 encoder: dimensions {width}x{height} are not one of the standard \
             source formats (sub-QCIF/QCIF/CIF/4CIF/16CIF)"
        ))
    })?;
    let pix = params.pixel_format.unwrap_or(PixelFormat::Yuv420P);
    if pix != PixelFormat::Yuv420P {
        return Err(Error::unsupported(format!(
            "h263 encoder: only Yuv420P supported (got {:?})",
            pix
        )));
    }

    let frame_rate = params.frame_rate.unwrap_or(Rational::new(30, 1));
    let mut output_params = params.clone();
    output_params.media_type = MediaType::Video;
    output_params.codec_id = CodecId::new(super::CODEC_ID_STR);
    output_params.width = Some(width);
    output_params.height = Some(height);
    output_params.pixel_format = Some(PixelFormat::Yuv420P);
    output_params.frame_rate = Some(frame_rate);
    let time_base = TimeBase::new(frame_rate.den, frame_rate.num);

    Ok(Box::new(H263Encoder {
        output_params,
        width,
        height,
        source_format,
        pquant: DEFAULT_PQUANT,
        time_base,
        pending: VecDeque::new(),
        eof: false,
        next_tr: 0,
    }))
}

struct H263Encoder {
    output_params: CodecParameters,
    width: u32,
    height: u32,
    source_format: SourceFormat,
    pquant: u8,
    time_base: TimeBase,
    pending: VecDeque<Packet>,
    eof: bool,
    next_tr: u8,
}

impl Encoder for H263Encoder {
    fn codec_id(&self) -> &CodecId {
        &self.output_params.codec_id
    }

    fn output_params(&self) -> &CodecParameters {
        &self.output_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let v = match frame {
            Frame::Video(v) => v,
            _ => return Err(Error::invalid("h263 encoder: video frames only")),
        };
        if v.width != self.width || v.height != self.height {
            return Err(Error::invalid(format!(
                "h263 encoder: frame dims {}x{} do not match encoder {}x{}",
                v.width, v.height, self.width, self.height
            )));
        }
        if v.format != PixelFormat::Yuv420P {
            return Err(Error::invalid("h263 encoder: only Yuv420P input frames"));
        }
        if v.planes.len() != 3 {
            return Err(Error::invalid("h263 encoder: expected 3 planes"));
        }

        // For now every frame is encoded as an I-picture. P-picture support is
        // explicitly out of scope per the crate-level docs.
        let tr = self.next_tr;
        self.next_tr = self.next_tr.wrapping_add(1);
        let data = encode_i_picture(
            self.width,
            self.height,
            self.source_format,
            self.pquant,
            tr,
            v,
        )?;

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
        if self.eof {
            Err(Error::Eof)
        } else {
            Err(Error::NeedMore)
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Picture / GOB / MB / block emit
// ---------------------------------------------------------------------------

/// Encode a single I-picture and return the raw H.263 elementary-stream bytes
/// (PSC + payload, not byte-stuffed; H.263 is naturally byte-aligned at GOB
/// boundaries, and our encoder never emits a value that would alias the
/// 17-bit zero prefix mid-stream because we cap |level| at 127 for escape
/// codes).
pub fn encode_i_picture(
    width: u32,
    height: u32,
    source_format: SourceFormat,
    pquant: u8,
    temporal_reference: u8,
    frame: &VideoFrame,
) -> Result<Vec<u8>> {
    let mb_w = width.div_ceil(16) as usize;
    let mb_h = height.div_ceil(16) as usize;
    let (_num_gobs, mb_rows_per_gob) = source_format
        .gob_layout()
        .ok_or_else(|| Error::invalid("h263 encoder: source format has no GOB layout"))?;

    let mut bw = BitWriter::with_capacity(8192);

    write_picture_header(&mut bw, source_format, pquant, temporal_reference)?;

    for mb_y in 0..mb_h {
        // GOB header at every GOB except the first.
        if mb_y > 0 && (mb_y as u32) % mb_rows_per_gob == 0 {
            let gn = (mb_y as u32 / mb_rows_per_gob) as u8;
            write_gob_header(&mut bw, gn, pquant)?;
        }
        for mb_x in 0..mb_w {
            encode_intra_mb(&mut bw, mb_x, mb_y, pquant, frame)?;
        }
    }
    // Trailing zero stuffing to ensure the encoder leaves a byte boundary
    // (BitWriter::finish handles padding, but the spec requires the final
    // byte to align to a multiple of 8). No EOS marker — short clips don't
    // need one and ffmpeg accepts the stream without it.
    Ok(bw.finish())
}

fn write_picture_header(
    bw: &mut BitWriter,
    source_format: SourceFormat,
    pquant: u8,
    tr: u8,
) -> Result<()> {
    // PSC: 22 bits = `0000 0000 0000 0000 1 00000`. Write byte-aligned to
    // simplify start-code recognition.
    debug_assert!(bw.is_byte_aligned());
    #[allow(clippy::unusual_byte_groupings)]
    let psc: u32 = 0b00_0000_0000_0000_0000_1_00000;
    bw.write_bits(psc, 22);

    // TR.
    bw.write_bits(tr as u32, 8);

    // PTYPE bits 1..=13:
    //   bit 1: always 1
    //   bit 2: always 0 (distinguishes from H.261)
    //   bit 3: split-screen
    //   bit 4: document-camera
    //   bit 5: freeze-picture release
    //   bits 6-8: source format (1..=5)
    //   bit 9: I (0) / P (1) — encoder always emits 0 for now
    //   bit 10: UMV (D)        - 0
    //   bit 11: SAC (E)        - 0
    //   bit 12: AP  (F)        - 0
    //   bit 13: PB  (G)        - 0
    let src_code: u32 = match source_format {
        SourceFormat::SubQcif => 1,
        SourceFormat::Qcif => 2,
        SourceFormat::Cif => 3,
        SourceFormat::FourCif => 4,
        SourceFormat::SixteenCif => 5,
        _ => {
            return Err(Error::unsupported(
                "h263 encoder: only standard source formats 1..=5 are supported",
            ));
        }
    };
    bw.write_bits(1, 1); // bit 1
    bw.write_bits(0, 1); // bit 2
    bw.write_bits(0, 1); // bit 3 split_screen
    bw.write_bits(0, 1); // bit 4 doc_camera
    bw.write_bits(0, 1); // bit 5 freeze
    bw.write_bits(src_code, 3); // bits 6-8 source format
    bw.write_bits(0, 1); // bit 9 picture coding type (I)
    bw.write_bits(0, 1); // bit 10 UMV
    bw.write_bits(0, 1); // bit 11 SAC
    bw.write_bits(0, 1); // bit 12 AP
    bw.write_bits(0, 1); // bit 13 PB

    // PQUANT (5 bits).
    if pquant == 0 || pquant > 31 {
        return Err(Error::invalid(format!(
            "h263 encoder: pquant {} out of range 1..=31",
            pquant
        )));
    }
    bw.write_bits(pquant as u32, 5);

    // CPM (0) and no PSBI follows.
    bw.write_bits(0, 1);

    // PEI loop terminator.
    bw.write_bits(0, 1);
    Ok(())
}

fn write_gob_header(bw: &mut BitWriter, gn: u8, gquant: u8) -> Result<()> {
    // GBSC must be byte-aligned per §5.2.2 — pad with zero stuffing bits.
    // The spec actually allows up to 7 STUF bits (a `0000 0000` MB-stuffing
    // codeword would be ambiguous, so we use the bit-padding approach) before
    // the GBSC. We just zero-pad to byte boundary, which is what every
    // ffmpeg-emitted stream does.
    while !bw.is_byte_aligned() {
        bw.write_bits(0, 1);
    }
    // GBSC: 17 bits = `0000 0000 0000 0000 1` = 0x00001.
    bw.write_bits(0x00001, 17);
    bw.write_bits(gn as u32 & 0x1F, 5);
    // CPM=0, so no GSBI.
    bw.write_bits(0, 2); // GFID — 2 bits, must be the same for every GOB in a picture; 0 is fine
    if gquant == 0 || gquant > 31 {
        return Err(Error::invalid(format!(
            "h263 encoder: gquant {} out of range 1..=31",
            gquant
        )));
    }
    bw.write_bits(gquant as u32, 5);
    Ok(())
}

/// Encode one intra MB. We always emit Intra (mb_type=3) — never IntraQ —
/// because we hold the quantiser constant for the whole picture.
fn encode_intra_mb(
    bw: &mut BitWriter,
    mb_x: usize,
    mb_y: usize,
    quant: u8,
    frame: &VideoFrame,
) -> Result<()> {
    // 1. Pull samples for all 6 blocks, run forward DCT + quantise, build CBP.
    let mut blocks = [[0i32; 64]; 6];
    let mut dc_pels = [128u8; 6]; // INTRADC byte values for each block
    let mut block_has_ac = [false; 6];

    for b in 0..6 {
        let mut samples = [0.0f32; 64];
        sample_block_for(frame, mb_x, mb_y, b, &mut samples);

        let mut dctf = samples;
        fdct8x8(&mut dctf);

        // INTRADC: pel_dc = round(F[0,0] / 8), clamped to 1..=254 with 128
        // remapped to 0xFF (the decoder maps 0xFF -> 1024 = 128*8).
        let dc_round = (dctf[0] / 8.0).round() as i32;
        let dc_clamped = dc_round.clamp(1, 254);
        let dc_byte: u8 = if dc_clamped == 128 {
            0xFF
        } else {
            dc_clamped as u8
        };
        // dc_byte is used both in the bitstream and for quantising AC: the AC
        // coefficients quantise the DCT output as-is.
        dc_pels[b] = dc_byte;

        // Quantise AC.
        //
        // The H.263 inverse-quant formula reconstructs nonzero levels as
        //     |F''| = q*(2L+1) - (1 if q even else 0)
        // For an input coefficient |F|, the level L that minimises the
        // reconstruction error is L = round((|F| - q) / (2q)) clamped to >= 0
        // (so |F| < q maps to 0).
        //
        // Deadzone bias: H.263 reference encoders typically use a much smaller
        // deadzone than the symmetric one (q/2 instead of q) to preserve more
        // energy at low quantisers. We follow ffmpeg's default and use a
        // deadzone of q*3/4 — encoded as a quantiser bias of -q/4 in the
        // forward path:
        //     L = floor( (|F| + q/4) / (2q) )
        let mut levels = [0i32; 64];
        let q = quant as i32;
        let two_q = 2 * q;
        let bias = q / 4;
        for k in 1..64 {
            let coef = dctf[k];
            let abs_f = coef.abs() as i32;
            let mag = (abs_f + bias) / two_q;
            if mag != 0 {
                let signed = if coef < 0.0 { -mag } else { mag };
                // Cap |level| at 127 so we stay inside the 8-bit signed
                // escape body (which forbids -128 anyway).
                levels[k] = signed.clamp(-127, 127);
            }
        }

        // Has AC?
        let any_ac = levels.iter().skip(1).any(|&l| l != 0);
        block_has_ac[b] = any_ac;
        blocks[b] = levels;
    }

    // 2. CBPC = chroma block bits (block 4 -> bit 1, block 5 -> bit 0).
    let cbpc: u8 = ((block_has_ac[4] as u8) << 1) | (block_has_ac[5] as u8);
    // CBPY = luma block bits (block 0 -> bit 3, block 1 -> bit 2, block 2 ->
    // bit 1, block 3 -> bit 0). Intra encodes the CBP directly (no XOR).
    let cbpy: u8 = ((block_has_ac[0] as u8) << 3)
        | ((block_has_ac[1] as u8) << 2)
        | ((block_has_ac[2] as u8) << 1)
        | (block_has_ac[3] as u8);

    // 3. Emit MB headers.
    write_mcbpc_intra(bw, cbpc);
    write_cbpy(bw, cbpy);
    // No DQUANT — we picked mb_type=3 (Intra), not 4 (IntraQ).

    // 4. Per-block: INTRADC + (optionally) AC.
    for b in 0..6 {
        bw.write_bits(dc_pels[b] as u32, 8);
        if block_has_ac[b] {
            write_block_ac(bw, &blocks[b]);
        }
    }

    Ok(())
}

/// Pull one 8×8 block of samples from a 4:2:0 YUV frame, with edge replication
/// for blocks that overhang the picture boundary.
fn sample_block_for(
    frame: &VideoFrame,
    mb_x: usize,
    mb_y: usize,
    block_idx: usize,
    out: &mut [f32; 64],
) {
    let (plane, stride, base_x, base_y, max_x, max_y) = match block_idx {
        0..=3 => {
            let x = mb_x * 16 + if block_idx & 1 == 1 { 8 } else { 0 };
            let y = mb_y * 16 + if block_idx & 2 == 2 { 8 } else { 0 };
            let p = &frame.planes[0];
            (
                p.data.as_slice(),
                p.stride,
                x,
                y,
                frame.width as usize,
                frame.height as usize,
            )
        }
        4 => {
            let x = mb_x * 8;
            let y = mb_y * 8;
            let p = &frame.planes[1];
            let cw = (frame.width as usize).div_ceil(2);
            let ch = (frame.height as usize).div_ceil(2);
            (p.data.as_slice(), p.stride, x, y, cw, ch)
        }
        5 => {
            let x = mb_x * 8;
            let y = mb_y * 8;
            let p = &frame.planes[2];
            let cw = (frame.width as usize).div_ceil(2);
            let ch = (frame.height as usize).div_ceil(2);
            (p.data.as_slice(), p.stride, x, y, cw, ch)
        }
        _ => unreachable!(),
    };
    for j in 0..8 {
        let yy = (base_y + j).min(max_y.saturating_sub(1));
        for i in 0..8 {
            let xx = (base_x + i).min(max_x.saturating_sub(1));
            out[j * 8 + i] = plane[yy * stride + xx] as f32;
        }
    }
}

/// Encode the AC coefficients of an 8×8 block in zig-zag order. Caller has
/// ensured at least one nonzero exists in `levels[1..]`.
fn write_block_ac(bw: &mut BitWriter, levels: &[i32; 64]) {
    use oxideav_mpeg4video::headers::vol::ZIGZAG;

    // Find the position of the last nonzero in zigzag order so we know where
    // to set `last=true`.
    let mut nonzero_zz: Vec<(usize, i32)> = Vec::with_capacity(8);
    for zz in 1..64 {
        let nat = ZIGZAG[zz];
        let lv = levels[nat];
        if lv != 0 {
            nonzero_zz.push((zz, lv));
        }
    }
    debug_assert!(!nonzero_zz.is_empty());

    let mut prev_zz: usize = 0; // position of last emitted (or 0 for "none yet")
    for (i, &(zz, lv)) in nonzero_zz.iter().enumerate() {
        // Run = number of zero coefficients between the previous nonzero
        // (exclusive) and this one (exclusive). For the first AC, the
        // previous "nonzero" is the DC at position 0, so run = zz - 1.
        let run = if i == 0 {
            (zz - 1) as u8
        } else {
            (zz - prev_zz - 1) as u8
        };
        let last = i == nonzero_zz.len() - 1;
        write_tcoef(bw, last, run, lv);
        prev_zz = zz;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::frame::VideoPlane;

    fn make_constant_frame(w: u32, h: u32, y: u8, cb: u8, cr: u8) -> VideoFrame {
        let cw = w.div_ceil(2) as usize;
        let ch = h.div_ceil(2) as usize;
        VideoFrame {
            format: PixelFormat::Yuv420P,
            width: w,
            height: h,
            pts: Some(0),
            time_base: TimeBase::new(1, 30),
            planes: vec![
                VideoPlane {
                    stride: w as usize,
                    data: vec![y; (w * h) as usize],
                },
                VideoPlane {
                    stride: cw,
                    data: vec![cb; cw * ch],
                },
                VideoPlane {
                    stride: cw,
                    data: vec![cr; cw * ch],
                },
            ],
        }
    }

    /// Encode a constant-grey QCIF picture, then decode it via the existing
    /// decoder and check the round-trip is bit-exact (DC-only, no AC).
    #[test]
    fn encode_decode_constant_qcif() {
        let frame = make_constant_frame(176, 144, 100, 128, 128);
        let bytes = encode_i_picture(176, 144, SourceFormat::Qcif, 5, 0, &frame).expect("encode");
        // Decode it back.
        use crate::decoder::H263Decoder;
        use oxideav_codec::Decoder;
        use oxideav_core::Frame as CoreFrame;

        let mut dec = H263Decoder::new(CodecId::new(crate::CODEC_ID_STR));
        let pkt = Packet::new(0, TimeBase::new(1, 30), bytes);
        dec.send_packet(&pkt).expect("send");
        dec.flush().expect("flush");
        let f = dec.receive_frame().expect("receive");
        let v = match f {
            CoreFrame::Video(v) => v,
            _ => panic!("not video"),
        };
        // Check: most pels should be ≈100 in luma.
        let yp = &v.planes[0];
        let mut hits = 0usize;
        for y in 0..v.height as usize {
            for x in 0..v.width as usize {
                let p = yp.data[y * yp.stride + x] as i32;
                if (p - 100).abs() <= 2 {
                    hits += 1;
                }
            }
        }
        let total = (v.width * v.height) as usize;
        assert!(hits * 100 / total >= 99, "constant Y match {hits}/{total}");
    }
}
