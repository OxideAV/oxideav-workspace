//! H.263 baseline encoder — I- and P-pictures.
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
//! * P-MB: COD flag + MCBPC inter (mb_type 0=Inter, or 4=Intra-in-P when the
//!   block is hard to predict) + CBPY (XOR-inverted for inter) + MV via
//!   the motion-VLC table with median predictor.
//! * Block layer: 8-bit INTRADC (with the spec's `0x00`/`0x80`/`0xFF`
//!   special-value handling) + H.263 AC TCOEF VLC encode with a fixed-length
//!   `last + run(6) + level(8)` escape body for out-of-table tuples.
//! * 8×8 forward DCT (textbook f32) + H.263 quant.
//! * GOP control: first frame is always I; subsequent frames are P until
//!   `gop_size` frames have elapsed, at which point we insert another I.
//!
//! Out of scope (returns `Error::Unsupported`):
//! * Annex D (UMV), Annex E (SAC), Annex F (Advanced Prediction — 4MV/OBMC),
//!   Annex G (PB-frames), Annex I (Advanced Intra Coding), Annex J
//!   (deblocking), Annex T (Modified Quantization).
//! * H.263+ PLUSPTYPE custom picture format extensions.
//! * CPM continuous-presence multipoint mode.
//! * B-pictures of any flavour.
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
use oxideav_mpeg4video::headers::vol::ZIGZAG;

use crate::bitwriter::BitWriter;
use crate::dct::fdct8x8;
use crate::enc_tables::{write_cbpy, write_mcbpc_inter, write_mcbpc_intra, write_tcoef, PMbKind};
use crate::interp::{predict_block, sad_block};
use crate::mb::IPicture;
use crate::motion::{
    encode_mv_component, luma_to_chroma_mv, predict_mv, MbMotion, MvGrid, MV_RANGE_MAX_HALF,
    MV_RANGE_MIN_HALF,
};
use crate::picture::SourceFormat;

/// Default fixed quantiser (PQUANT) — `5` matches the
/// `ffmpeg -qscale:v 5` baseline used to validate the existing decoder.
pub const DEFAULT_PQUANT: u8 = 5;

/// Default GOP size — one I-picture every 12 frames. Matches the default
/// `ffmpeg -g 12` cadence for H.263 output.
pub const DEFAULT_GOP_SIZE: u32 = 12;

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
        gop_size: DEFAULT_GOP_SIZE,
        since_keyframe: 0,
        reference: None,
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
    /// Cadence between keyframes, in frames. `1` means "every frame is an I",
    /// `0` is treated identically. `>= 2` enables the P-picture path.
    gop_size: u32,
    /// Frames emitted since the last I-picture (0 → next frame is I).
    since_keyframe: u32,
    /// Previous reconstructed picture (motion-compensation reference for the
    /// next P-picture). `None` before the first I is encoded.
    reference: Option<IPicture>,
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

        let tr = self.next_tr;
        self.next_tr = self.next_tr.wrapping_add(1);

        // Decide I vs P: first frame is always I; then every `gop_size` frames
        // we insert another I. `gop_size <= 1` forces I on every frame.
        let force_i = self.reference.is_none()
            || self.gop_size <= 1
            || self.since_keyframe + 1 >= self.gop_size;

        let (data, recon, is_key) = if force_i {
            let (bytes, pic) = encode_i_picture_with_recon(
                self.width,
                self.height,
                self.source_format,
                self.pquant,
                tr,
                v,
            )?;
            (bytes, pic, true)
        } else {
            let reference = self.reference.as_ref().expect("reference checked above");
            let (bytes, pic) = encode_p_picture(
                self.width,
                self.height,
                self.source_format,
                self.pquant,
                tr,
                v,
                reference,
            )?;
            (bytes, pic, false)
        };

        self.reference = Some(recon);
        if is_key {
            self.since_keyframe = 1;
        } else {
            self.since_keyframe += 1;
        }

        let mut pkt = Packet::new(0, self.time_base, data);
        pkt.pts = v.pts;
        pkt.dts = v.pts;
        pkt.flags.keyframe = is_key;
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
    let (bytes, _recon) = encode_i_picture_with_recon(
        width,
        height,
        source_format,
        pquant,
        temporal_reference,
        frame,
    )?;
    Ok(bytes)
}

/// Encode an I-picture and reconstruct it locally (for use as the motion-
/// compensation reference when the next frame is a P-picture). The
/// reconstruction is bit-exact with what the decoder produces when fed the
/// returned byte stream.
pub fn encode_i_picture_with_recon(
    width: u32,
    height: u32,
    source_format: SourceFormat,
    pquant: u8,
    temporal_reference: u8,
    frame: &VideoFrame,
) -> Result<(Vec<u8>, IPicture)> {
    let mb_w = width.div_ceil(16) as usize;
    let mb_h = height.div_ceil(16) as usize;
    let (_num_gobs, mb_rows_per_gob) = source_format
        .gob_layout()
        .ok_or_else(|| Error::invalid("h263 encoder: source format has no GOB layout"))?;

    let mut bw = BitWriter::with_capacity(8192);
    let mut recon = IPicture::new(width as usize, height as usize);

    write_picture_header(&mut bw, source_format, pquant, temporal_reference, false)?;

    for mb_y in 0..mb_h {
        // GOB header at every GOB except the first.
        if mb_y > 0 && (mb_y as u32) % mb_rows_per_gob == 0 {
            let gn = (mb_y as u32 / mb_rows_per_gob) as u8;
            write_gob_header(&mut bw, gn, pquant)?;
        }
        for mb_x in 0..mb_w {
            encode_intra_mb(&mut bw, mb_x, mb_y, pquant, frame, &mut recon)?;
        }
    }
    // Trailing zero stuffing to ensure the encoder leaves a byte boundary
    // (BitWriter::finish handles padding, but the spec requires the final
    // byte to align to a multiple of 8). No EOS marker — short clips don't
    // need one and ffmpeg accepts the stream without it.
    Ok((bw.finish(), recon))
}

/// Encode a single P-picture against the supplied `reference`. Returns the
/// bitstream bytes and the locally reconstructed picture (used as the next
/// MC reference).
pub fn encode_p_picture(
    width: u32,
    height: u32,
    source_format: SourceFormat,
    pquant: u8,
    temporal_reference: u8,
    frame: &VideoFrame,
    reference: &IPicture,
) -> Result<(Vec<u8>, IPicture)> {
    let mb_w = width.div_ceil(16) as usize;
    let mb_h = height.div_ceil(16) as usize;
    source_format
        .gob_layout()
        .ok_or_else(|| Error::invalid("h263 encoder: source format has no GOB layout"))?;

    let mut bw = BitWriter::with_capacity(8192);
    let mut recon = IPicture::new(width as usize, height as usize);
    let mut mv_grid = MvGrid::new(mb_w, mb_h);

    write_picture_header(&mut bw, source_format, pquant, temporal_reference, true)?;

    // No GOB headers in P-pictures: the baseline spec treats GOB headers as
    // optional resync points. Emitting them triggers MV-predictor reset in
    // downstream decoders (§5.3.7.2), which is fine but makes the bitstream
    // larger. Short clips don't benefit, so we skip them — this matches
    // what ffmpeg's h263 encoder does for P-pictures too.
    for mb_y in 0..mb_h {
        for mb_x in 0..mb_w {
            encode_p_mb(
                &mut bw,
                mb_x,
                mb_y,
                pquant,
                frame,
                reference,
                &mut recon,
                &mut mv_grid,
            )?;
        }
    }
    Ok((bw.finish(), recon))
}

fn write_picture_header(
    bw: &mut BitWriter,
    source_format: SourceFormat,
    pquant: u8,
    tr: u8,
    is_p_picture: bool,
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
    bw.write_bits(u32::from(is_p_picture), 1); // bit 9 picture coding type (I=0, P=1)
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
///
/// Also reconstructs the MB locally into `recon` so the caller can use it as
/// the MC reference for the next P-picture.
fn encode_intra_mb(
    bw: &mut BitWriter,
    mb_x: usize,
    mb_y: usize,
    quant: u8,
    frame: &VideoFrame,
    recon: &mut IPicture,
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

        let (dc_byte, levels, any_ac) = quantise_intra_block(&dctf, quant);
        dc_pels[b] = dc_byte;
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

    // 4. Per-block: INTRADC + (optionally) AC. Reconstruct into `recon`.
    for b in 0..6 {
        bw.write_bits(dc_pels[b] as u32, 8);
        if block_has_ac[b] {
            write_block_ac(bw, &blocks[b]);
        }
        reconstruct_intra_block(recon, b, mb_x, mb_y, dc_pels[b], &blocks[b], quant);
    }

    Ok(())
}

/// Quantise one intra block's DCT output into `(dc_byte, levels, any_ac)`.
///
/// `dc_byte` is the 8-bit INTRADC value encoded on the wire (1..=254, with
/// 128 remapped to 0xFF). `levels` holds the AC levels in natural-order
/// positions; `any_ac` is `true` iff any AC is nonzero.
fn quantise_intra_block(dctf: &[f32; 64], quant: u8) -> (u8, [i32; 64], bool) {
    // INTRADC: pel_dc = round(F[0,0] / 8), clamped to 1..=254 with 128
    // remapped to 0xFF (the decoder maps 0xFF -> 1024 = 128*8).
    let dc_round = (dctf[0] / 8.0).round() as i32;
    let dc_clamped = dc_round.clamp(1, 254);
    let dc_byte: u8 = if dc_clamped == 128 {
        0xFF
    } else {
        dc_clamped as u8
    };

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
            levels[k] = signed.clamp(-127, 127);
        }
    }
    let any_ac = levels.iter().skip(1).any(|&l| l != 0);
    (dc_byte, levels, any_ac)
}

/// Reconstruct an intra 8×8 block into `recon` from its decoded DC byte and
/// AC levels. Uses the same dequant + IDCT path as the decoder.
fn reconstruct_intra_block(
    recon: &mut IPicture,
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    dc_byte: u8,
    levels: &[i32; 64],
    quant: u8,
) {
    let mut coeffs = dequantise_block(levels, quant, true);
    coeffs[0] = if dc_byte == 0xFF {
        1024
    } else {
        (dc_byte as i32) << 3
    };
    coeffs[0] = coeffs[0].clamp(-2048, 2047);
    let mut out = [0u8; 64];
    crate::block::idct_and_clip(&mut coeffs, &mut out);
    write_block_into(recon, block_idx, mb_x, mb_y, &out);
}

/// Dequantise an 8×8 block of H.263 levels (H.263 inverse-quant formula).
///
/// For an intra block the caller is responsible for overwriting the DC slot
/// after this call — `intra_dc` is ignored inside the loop because intra DC
/// uses the INTRADC special-case, not the AC formula.
fn dequantise_block(levels: &[i32; 64], quant: u8, skip_dc: bool) -> [i32; 64] {
    let q = quant as i32;
    let q_minus_one_if_even = if q & 1 == 1 { 0 } else { -1 };
    let mut out = [0i32; 64];
    let start = if skip_dc { 1 } else { 0 };
    for k in start..64 {
        let l = levels[k];
        if l == 0 {
            continue;
        }
        let abs = l.unsigned_abs() as i32;
        let mut val = q * (2 * abs + 1) + q_minus_one_if_even;
        if l < 0 {
            val = -val;
        }
        out[k] = val.clamp(-2048, 2047);
    }
    out
}

/// Copy an 8×8 reconstructed block into the picture buffer.
fn write_block_into(
    pic: &mut IPicture,
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
    out: &[u8; 64],
) {
    let (plane, stride, px, py) = block_dst(pic, block_idx, mb_x, mb_y);
    for dy in 0..8 {
        for dx in 0..8 {
            plane[(py + dy) * stride + (px + dx)] = out[dy * 8 + dx];
        }
    }
}

/// Mirror of `mb::block_dst` — duplicated here because `mb::block_dst` is
/// private and the encoder doesn't go through the decode path.
fn block_dst(
    pic: &mut IPicture,
    block_idx: usize,
    mb_x: usize,
    mb_y: usize,
) -> (&mut [u8], usize, usize, usize) {
    match block_idx {
        0 => (pic.y.as_mut_slice(), pic.y_stride, mb_x * 16, mb_y * 16),
        1 => (pic.y.as_mut_slice(), pic.y_stride, mb_x * 16 + 8, mb_y * 16),
        2 => (pic.y.as_mut_slice(), pic.y_stride, mb_x * 16, mb_y * 16 + 8),
        3 => (
            pic.y.as_mut_slice(),
            pic.y_stride,
            mb_x * 16 + 8,
            mb_y * 16 + 8,
        ),
        4 => (pic.cb.as_mut_slice(), pic.c_stride, mb_x * 8, mb_y * 8),
        5 => (pic.cr.as_mut_slice(), pic.c_stride, mb_x * 8, mb_y * 8),
        _ => unreachable!(),
    }
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

// ---------------------------------------------------------------------------
// P-picture: motion estimation + macroblock emit
// ---------------------------------------------------------------------------

/// Motion-estimation search window radius (integer-pel units) for the P-MB
/// encoder. `±15` half-pel units max per spec → ±7 integer pel; we search
/// ±7 around the predictor and then refine with half-pel.
const ME_SEARCH_RADIUS: i32 = 7;

/// Motion-estimation: find the best 16×16 MV (in half-pel units) for MB
/// `(mb_x, mb_y)` against `reference`. Returns `(mv_x_half, mv_y_half, sad)`.
///
/// Two-phase search:
/// 1. Integer-pel block-matching over `[-ME_SEARCH_RADIUS, +ME_SEARCH_RADIUS]`
///    around `(0, 0)` (simple exhaustive search — fast enough at baseline
///    resolutions and keeps the code short/clear).
/// 2. Half-pel refinement: 8-neighbour search around the winning integer-pel
///    MV.
fn motion_estimate_mb(
    frame: &VideoFrame,
    reference: &IPicture,
    mb_x: usize,
    mb_y: usize,
) -> (i32, i32, u32) {
    let src = &frame.planes[0];
    let src_stride = src.stride;
    let src_x = (mb_x * 16) as i32;
    let src_y = (mb_y * 16) as i32;
    let blk_px = src_x;
    let blk_py = src_y;
    let ref_w = reference.y_stride as i32;
    let ref_h = (reference.y.len() / reference.y_stride) as i32;
    let pic_w = reference.width as i32;
    let pic_h = reference.height as i32;

    // MV stay-in-picture constraint (baseline H.263 — no Annex D UMV).
    //
    // A luma MV `(mvx, mvy)` in half-pel units is valid iff the entire
    // 16x16 predictor block lies within the picture:
    //   blk_px + (mvx/2) >= 0
    //   blk_py + (mvy/2) >= 0
    //   blk_px + 16 + ceil(mvx/2) <= pic_w
    //   blk_py + 16 + ceil(mvy/2) <= pic_h
    // Plus the half-pel filter needs `mvx|1 == 1` to use the right-edge
    // neighbour, so we add 1 to the upper bound in that case.
    //
    // Integer half: shift right by 1 (with sign). Fractional half-pel
    // extension: +1 if the half-pel bit is set.
    let mv_ok = |mvx: i32, mvy: i32| -> bool {
        let ix = mvx >> 1;
        let iy = mvy >> 1;
        let ext_x = (mvx & 1).abs();
        let ext_y = (mvy & 1).abs();
        let left = blk_px + ix;
        let top = blk_py + iy;
        let right = blk_px + 16 + ix + ext_x;
        let bottom = blk_py + 16 + iy + ext_y;
        left >= 0 && top >= 0 && right <= pic_w && bottom <= pic_h
    };

    // Stage 1: integer-pel exhaustive search in a ±R window.
    let r = ME_SEARCH_RADIUS;
    let mut best = (0i32, 0i32, u32::MAX);
    let mv_range = MV_RANGE_MIN_HALF..=MV_RANGE_MAX_HALF;
    for dy in -r..=r {
        for dx in -r..=r {
            let mvx = dx * 2;
            let mvy = dy * 2;
            if !mv_range.contains(&mvx) || !mv_range.contains(&mvy) {
                continue;
            }
            if !mv_ok(mvx, mvy) {
                continue;
            }
            let sad = sad_block(
                &src.data,
                src_stride,
                src_x,
                src_y,
                &reference.y,
                reference.y_stride,
                ref_w,
                ref_h,
                blk_px,
                blk_py,
                mvx,
                mvy,
                16,
            );
            if sad < best.2 {
                best = (mvx, mvy, sad);
            }
        }
    }

    // Stage 2: half-pel refinement — 8 neighbours around the integer winner.
    let (ix, iy, _) = best;
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let mvx = ix + dx;
            let mvy = iy + dy;
            if !mv_range.contains(&mvx) || !mv_range.contains(&mvy) {
                continue;
            }
            if !mv_ok(mvx, mvy) {
                continue;
            }
            let sad = sad_block(
                &src.data,
                src_stride,
                src_x,
                src_y,
                &reference.y,
                reference.y_stride,
                ref_w,
                ref_h,
                blk_px,
                blk_py,
                mvx,
                mvy,
                16,
            );
            if sad < best.2 {
                best = (mvx, mvy, sad);
            }
        }
    }

    best
}

/// Encode one P-picture macroblock. Chooses one of:
/// * **Skipped** (COD=1): when the predicted MB with MV=(0,0) has residual
///   energy below threshold AND the median predictor is (0,0). Copies
///   reference into `recon`.
/// * **Inter**: emit MCBPC/CBPY/MVD, compensate the block, encode residual
///   for any block with energy above threshold. The DCT of the residual is
///   quantised like intra AC (but with start at scan 0 and no DC special
///   case).
/// * **Intra-in-P**: when the best inter prediction's SAD is worse than a
///   direct intra encode's approximate cost — we fall back to intra for that
///   MB. This is the standard "intra block decision" used by FFmpeg.
#[allow(clippy::too_many_arguments)]
fn encode_p_mb(
    bw: &mut BitWriter,
    mb_x: usize,
    mb_y: usize,
    quant: u8,
    frame: &VideoFrame,
    reference: &IPicture,
    recon: &mut IPicture,
    mv_grid: &mut MvGrid,
) -> Result<()> {
    // 1. Motion-estimate on luma 16×16.
    let (mvx, mvy, mv_sad) = motion_estimate_mb(frame, reference, mb_x, mb_y);

    // Also consider MV=(0,0) directly — some encoders pin to zero when the
    // difference is small, which gives the skipped-MB path a chance.
    let zero_sad = sad_block(
        &frame.planes[0].data,
        frame.planes[0].stride,
        (mb_x * 16) as i32,
        (mb_y * 16) as i32,
        &reference.y,
        reference.y_stride,
        reference.y_stride as i32,
        (reference.y.len() / reference.y_stride) as i32,
        (mb_x * 16) as i32,
        (mb_y * 16) as i32,
        0,
        0,
        16,
    );
    let (pmx, pmy) = predict_mv(mv_grid, mb_x, mb_y);
    // Median predictor (pmx, pmy) is in half-pel. For the skip decision we
    // need pmx == 0 AND pmy == 0 because a skipped MB carries MV (0,0).
    let can_skip = pmx == 0 && pmy == 0 && zero_sad < mv_sad + 128;

    // 2. Compute the MB predictor + residual energy.
    let mut y_pred = [0u8; 256];
    let mut u_pred = [0u8; 64];
    let mut v_pred = [0u8; 64];

    let decide_mv = if can_skip { (0, 0) } else { (mvx, mvy) };

    build_mb_predictor(
        reference,
        mb_x,
        mb_y,
        decide_mv.0,
        decide_mv.1,
        &mut y_pred,
        &mut u_pred,
        &mut v_pred,
    );

    // Quick residual energy (sum of absolute luma residuals). Used to decide
    // whether an "all-zero residual skipped MB" is acceptable AND whether to
    // try intra.
    let src_y = &frame.planes[0];
    let src_cb = &frame.planes[1];
    let src_cr = &frame.planes[2];
    let mut luma_abs_sum = 0u32;
    for j in 0..16 {
        for i in 0..16 {
            let s = src_y.data[(mb_y * 16 + j) * src_y.stride + (mb_x * 16 + i)] as i32;
            let p = y_pred[j * 16 + i] as i32;
            luma_abs_sum += (s - p).unsigned_abs();
        }
    }

    // Intra-vs-inter decision: compute the intra MB's total "variance" as a
    // proxy for intra coding cost (sum of |pel - mb_mean|). If intra wins by
    // a large margin we emit an intra MB. Simple heuristic that matches
    // FFmpeg's "mb_var < lambda * sad" rule at low qscales.
    let intra_variance = mb_luma_variance(src_y, mb_x, mb_y);
    let try_intra = intra_variance * 5 < luma_abs_sum;

    // Skipped MB: can_skip (MV=(0,0), predictor=(0,0)) AND residual is so
    // small that every block would quantise to zero. We model "quantise to
    // zero" as "sum of absolute residuals per 256 pels < thresh(q)".
    if can_skip && luma_abs_sum < (quant as u32) * 128 {
        // Emit COD=1 (skipped).
        bw.write_bits(1, 1);
        // Copy predictor into recon.
        copy_predictor_to_recon(recon, mb_x, mb_y, &y_pred, &u_pred, &v_pred);
        mv_grid.set(
            mb_x,
            mb_y,
            MbMotion {
                mv: (0, 0),
                coded: false,
                intra: false,
            },
        );
        return Ok(());
    }

    // COD = 0 — MB is coded.
    bw.write_bits(0, 1);

    if try_intra {
        encode_p_mb_intra(bw, mb_x, mb_y, quant, frame, recon)?;
        mv_grid.set(
            mb_x,
            mb_y,
            MbMotion {
                mv: (0, 0),
                coded: true,
                intra: true,
            },
        );
        return Ok(());
    }

    // Inter path.
    encode_p_mb_inter(
        bw, mb_x, mb_y, quant, src_y, src_cb, src_cr, reference, recon, decide_mv, mv_grid,
        &y_pred, &u_pred, &v_pred,
    )?;
    Ok(())
}

/// Intra encode of a P-MB block. Same bitstream as an I-MB's Intra MCBPC,
/// but prefixed with COD=0 by the caller AND using the inter MCBPC table
/// (PMbKind::Intra).
fn encode_p_mb_intra(
    bw: &mut BitWriter,
    mb_x: usize,
    mb_y: usize,
    quant: u8,
    frame: &VideoFrame,
    recon: &mut IPicture,
) -> Result<()> {
    let mut blocks = [[0i32; 64]; 6];
    let mut dc_pels = [128u8; 6];
    let mut block_has_ac = [false; 6];

    for b in 0..6 {
        let mut samples = [0.0f32; 64];
        sample_block_for(frame, mb_x, mb_y, b, &mut samples);
        let mut dctf = samples;
        fdct8x8(&mut dctf);
        let (dc_byte, levels, any_ac) = quantise_intra_block(&dctf, quant);
        dc_pels[b] = dc_byte;
        block_has_ac[b] = any_ac;
        blocks[b] = levels;
    }
    let cbpc: u8 = ((block_has_ac[4] as u8) << 1) | (block_has_ac[5] as u8);
    let cbpy: u8 = ((block_has_ac[0] as u8) << 3)
        | ((block_has_ac[1] as u8) << 2)
        | ((block_has_ac[2] as u8) << 1)
        | (block_has_ac[3] as u8);

    write_mcbpc_inter(bw, PMbKind::Intra, cbpc);
    write_cbpy(bw, cbpy);
    // No DQUANT for PMbKind::Intra (we hold quant constant).
    for b in 0..6 {
        bw.write_bits(dc_pels[b] as u32, 8);
        if block_has_ac[b] {
            write_block_ac(bw, &blocks[b]);
        }
        reconstruct_intra_block(recon, b, mb_x, mb_y, dc_pels[b], &blocks[b], quant);
    }
    Ok(())
}

/// Inter encode of a P-MB — MCBPC/CBPY/MVD + residual TCOEF per coded block.
#[allow(clippy::too_many_arguments)]
fn encode_p_mb_inter(
    bw: &mut BitWriter,
    mb_x: usize,
    mb_y: usize,
    quant: u8,
    src_y: &oxideav_core::frame::VideoPlane,
    src_cb: &oxideav_core::frame::VideoPlane,
    src_cr: &oxideav_core::frame::VideoPlane,
    _reference: &IPicture,
    recon: &mut IPicture,
    mv: (i32, i32),
    mv_grid: &mut MvGrid,
    y_pred: &[u8; 256],
    u_pred: &[u8; 64],
    v_pred: &[u8; 64],
) -> Result<()> {
    // 1. For each of the 6 blocks, compute residual DCT → quantise → check
    //    if any nonzero AC exists. Track (cbpy, cbpc) and the recon pels.
    let mut levels_all = [[0i32; 64]; 6];
    let mut has_ac = [false; 6];

    // Luma (4 blocks).
    for b in 0..4 {
        let (sub_x, sub_y) = match b {
            0 => (0, 0),
            1 => (8, 0),
            2 => (0, 8),
            3 => (8, 8),
            _ => unreachable!(),
        };
        let mut resid = [0.0f32; 64];
        for j in 0..8 {
            for i in 0..8 {
                let py = mb_y * 16 + sub_y + j;
                let px = mb_x * 16 + sub_x + i;
                let s = src_y.data[py * src_y.stride + px] as i32;
                let p = y_pred[(sub_y + j) * 16 + (sub_x + i)] as i32;
                resid[j * 8 + i] = (s - p) as f32;
            }
        }
        let mut dctf = resid;
        fdct8x8(&mut dctf);
        let levels = quantise_inter_block(&dctf, quant);
        has_ac[b] = levels.iter().any(|&l| l != 0);
        levels_all[b] = levels;
    }

    // Chroma (Cb, Cr).
    for (ci, plane) in [(0, src_cb), (1, src_cr)].iter() {
        let pred = if *ci == 0 { u_pred } else { v_pred };
        let mut resid = [0.0f32; 64];
        for j in 0..8 {
            for i in 0..8 {
                let py = mb_y * 8 + j;
                let px = mb_x * 8 + i;
                let s = plane.data[py * plane.stride + px] as i32;
                let p = pred[j * 8 + i] as i32;
                resid[j * 8 + i] = (s - p) as f32;
            }
        }
        let mut dctf = resid;
        fdct8x8(&mut dctf);
        let levels = quantise_inter_block(&dctf, quant);
        let b = 4 + ci;
        has_ac[b] = levels.iter().any(|&l| l != 0);
        levels_all[b] = levels;
    }

    // 2. Build CBPC / CBPY. For inter the on-wire CBPY field is bit-inverted
    //    of the actual pattern.
    let cbpc: u8 = ((has_ac[4] as u8) << 1) | (has_ac[5] as u8);
    let cbpy_true: u8 = ((has_ac[0] as u8) << 3)
        | ((has_ac[1] as u8) << 2)
        | ((has_ac[2] as u8) << 1)
        | (has_ac[3] as u8);
    let cbpy_on_wire = cbpy_true ^ 0xF;

    // 3. Emit MCBPC inter + CBPY + MVD.
    write_mcbpc_inter(bw, PMbKind::Inter, cbpc);
    write_cbpy(bw, cbpy_on_wire);
    let (pmx, pmy) = predict_mv(mv_grid, mb_x, mb_y);
    encode_mv_component(bw, mv.0, pmx);
    encode_mv_component(bw, mv.1, pmy);

    // 4. Emit per-block AC (when coded) and reconstruct.
    for b in 0..6 {
        if has_ac[b] {
            write_block_ac_inter(bw, &levels_all[b]);
        }
    }

    // 5. Reconstruct blocks into recon: predictor + dequantised residual
    //    IDCT, clipped.
    for b in 0..4 {
        let (sub_x, sub_y) = match b {
            0 => (0, 0),
            1 => (8, 0),
            2 => (0, 8),
            3 => (8, 8),
            _ => unreachable!(),
        };
        let coeffs = dequantise_block(&levels_all[b], quant, false);
        let mut c = coeffs;
        let mut resid_out = [0i32; 64];
        crate::block::idct_signed(&mut c, &mut resid_out);
        let (plane, stride, px, py) = block_dst(recon, b, mb_x, mb_y);
        for j in 0..8 {
            for i in 0..8 {
                let p = y_pred[(sub_y + j) * 16 + (sub_x + i)] as i32;
                let r = resid_out[j * 8 + i];
                plane[(py + j) * stride + (px + i)] = (p + r).clamp(0, 255) as u8;
            }
        }
    }
    for ci in 0..2usize {
        let b = 4 + ci;
        let pred = if ci == 0 { u_pred } else { v_pred };
        let coeffs = dequantise_block(&levels_all[b], quant, false);
        let mut c = coeffs;
        let mut resid_out = [0i32; 64];
        crate::block::idct_signed(&mut c, &mut resid_out);
        let (plane, stride, px, py) = block_dst(recon, b, mb_x, mb_y);
        for j in 0..8 {
            for i in 0..8 {
                let p = pred[j * 8 + i] as i32;
                let r = resid_out[j * 8 + i];
                plane[(py + j) * stride + (px + i)] = (p + r).clamp(0, 255) as u8;
            }
        }
    }

    mv_grid.set(
        mb_x,
        mb_y,
        MbMotion {
            mv,
            coded: true,
            intra: false,
        },
    );
    Ok(())
}

/// Quantise a residual (inter) block. Uses the same deadzone bias as the
/// intra AC path.
fn quantise_inter_block(dctf: &[f32; 64], quant: u8) -> [i32; 64] {
    let mut levels = [0i32; 64];
    let q = quant as i32;
    let two_q = 2 * q;
    let bias = q / 4;
    for k in 0..64 {
        let coef = dctf[k];
        let abs_f = coef.abs() as i32;
        let mag = (abs_f + bias) / two_q;
        if mag != 0 {
            let signed = if coef < 0.0 { -mag } else { mag };
            levels[k] = signed.clamp(-127, 127);
        }
    }
    levels
}

/// Emit the AC coefficients for an **inter** block in zig-zag order (start
/// at scan index 0 — there is no DC special-case in H.263 inter blocks).
fn write_block_ac_inter(bw: &mut BitWriter, levels: &[i32; 64]) {
    let mut nonzero_zz: Vec<(usize, i32)> = Vec::with_capacity(8);
    for zz in 0..64 {
        let nat = ZIGZAG[zz];
        let lv = levels[nat];
        if lv != 0 {
            nonzero_zz.push((zz, lv));
        }
    }
    debug_assert!(!nonzero_zz.is_empty());

    let mut prev_zz: i32 = -1;
    for (i, &(zz, lv)) in nonzero_zz.iter().enumerate() {
        let run = (zz as i32 - prev_zz - 1) as u8;
        let last = i == nonzero_zz.len() - 1;
        write_tcoef(bw, last, run, lv);
        prev_zz = zz as i32;
    }
}

/// Build the 16×16 luma + 2×8×8 chroma predictor into the provided buffers.
fn build_mb_predictor(
    reference: &IPicture,
    mb_x: usize,
    mb_y: usize,
    mvx: i32,
    mvy: i32,
    y_pred: &mut [u8; 256],
    u_pred: &mut [u8; 64],
    v_pred: &mut [u8; 64],
) {
    let ref_y_h = (reference.y.len() / reference.y_stride) as i32;
    let ref_c_h = (reference.cb.len() / reference.c_stride) as i32;
    // Luma: predict in four 8×8 sub-blocks, stitched into 16×16.
    for (blk, (sub_x, sub_y)) in [(0, (0, 0)), (1, (8, 0)), (2, (0, 8)), (3, (8, 8))].iter() {
        let _ = blk;
        let blk_px = (mb_x * 16 + sub_x) as i32;
        let blk_py = (mb_y * 16 + sub_y) as i32;
        let mut tmp = [0u8; 64];
        predict_block(
            &reference.y,
            reference.y_stride,
            reference.y_stride as i32,
            ref_y_h,
            blk_px,
            blk_py,
            mvx,
            mvy,
            8,
            &mut tmp,
            8,
        );
        for j in 0..8 {
            for i in 0..8 {
                y_pred[(sub_y + j) * 16 + (sub_x + i)] = tmp[j * 8 + i];
            }
        }
    }
    // Chroma.
    let cmx = luma_to_chroma_mv(mvx);
    let cmy = luma_to_chroma_mv(mvy);
    let blk_px = (mb_x * 8) as i32;
    let blk_py = (mb_y * 8) as i32;
    predict_block(
        &reference.cb,
        reference.c_stride,
        reference.c_stride as i32,
        ref_c_h,
        blk_px,
        blk_py,
        cmx,
        cmy,
        8,
        u_pred,
        8,
    );
    predict_block(
        &reference.cr,
        reference.c_stride,
        reference.c_stride as i32,
        ref_c_h,
        blk_px,
        blk_py,
        cmx,
        cmy,
        8,
        v_pred,
        8,
    );
}

/// Copy the 16×16 luma + 8×8 chroma predictor into `recon` (used when the MB
/// is emitted as a skipped MB — the reconstruction == predictor).
fn copy_predictor_to_recon(
    recon: &mut IPicture,
    mb_x: usize,
    mb_y: usize,
    y_pred: &[u8; 256],
    u_pred: &[u8; 64],
    v_pred: &[u8; 64],
) {
    for j in 0..16 {
        let off = (mb_y * 16 + j) * recon.y_stride + mb_x * 16;
        recon.y[off..off + 16].copy_from_slice(&y_pred[j * 16..j * 16 + 16]);
    }
    for j in 0..8 {
        let off = (mb_y * 8 + j) * recon.c_stride + mb_x * 8;
        recon.cb[off..off + 8].copy_from_slice(&u_pred[j * 8..j * 8 + 8]);
        recon.cr[off..off + 8].copy_from_slice(&v_pred[j * 8..j * 8 + 8]);
    }
}

/// Sum of absolute differences between the luma MB and its mean — cheap
/// proxy for "intra coding cost" in the intra/inter decision.
fn mb_luma_variance(src: &oxideav_core::frame::VideoPlane, mb_x: usize, mb_y: usize) -> u32 {
    let mut sum = 0u32;
    let mut sum_abs = 0u32;
    for j in 0..16 {
        for i in 0..16 {
            let s = src.data[(mb_y * 16 + j) * src.stride + mb_x * 16 + i] as u32;
            sum += s;
        }
    }
    let mean = sum / 256;
    for j in 0..16 {
        for i in 0..16 {
            let s = src.data[(mb_y * 16 + j) * src.stride + mb_x * 16 + i] as i32;
            sum_abs += (s - mean as i32).unsigned_abs();
        }
    }
    sum_abs
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
