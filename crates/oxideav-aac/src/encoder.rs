//! AAC-LC CBR encoder — ISO/IEC 14496-3 §4.5.2.1.
//!
//! This is a baseline long-only encoder. It mirrors the inverse structure
//! already present in `imdct.rs` / `synth.rs`:
//!
//! 1. Buffer input PCM, keeping a 1024-sample overlap window from the
//!    previous frame.
//! 2. Apply the same sine window used on decode (`window::sine_long`).
//! 3. Run forward MDCT (`mdct::mdct_long`) to produce 1024 spectral
//!    coefficients.
//! 4. For CPE (stereo) channels, consider M/S stereo per scalefactor band
//!    — choose whichever of (L,R) vs (M,S) costs fewer bits.
//! 5. Flat-quantise per scalefactor band: pick one global scalefactor per
//!    band so the largest quantised magnitude stays in the usable range
//!    for codebook 11 (the escape book, LAV=16 plus escape).
//! 6. For each band, pick the cheapest Huffman codebook among 0 (all-zero)
//!    and 1-11 whose LAV is compatible. Merge runs of the same codebook
//!    across bands into a single section.
//! 7. Encode scalefactor deltas via the scalefactor Huffman codebook.
//! 8. Write the SCE or CPE element, then `ID_END`, and pad to byte
//!    boundary.
//! 9. Wrap in an ADTS header (single raw_data_block).
//!
//! The round-trip acceptance bar is ffmpeg's AAC decoder + our own decoder
//! reporting a Goertzel ratio >= 50× at the source tone frequency on a
//! 1-second synthesised sine.
//!
//! Not implemented (deferred): TNS synthesis, PNS, intensity stereo,
//! pulse data, short-block/transient detection, gain control.

use std::collections::VecDeque;

use oxideav_codec::Encoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, MediaType, Packet, Result, SampleFormat,
    TimeBase,
};

use crate::bitwriter::BitWriter;
use crate::huffman_tables::{
    BOOK10_BITS, BOOK10_CODES, BOOK11_BITS, BOOK11_CODES, BOOK1_BITS, BOOK1_CODES, BOOK2_BITS,
    BOOK2_CODES, BOOK3_BITS, BOOK3_CODES, BOOK4_BITS, BOOK4_CODES, BOOK5_BITS, BOOK5_CODES,
    BOOK6_BITS, BOOK6_CODES, BOOK7_BITS, BOOK7_CODES, BOOK8_BITS, BOOK8_CODES, BOOK9_BITS,
    BOOK9_CODES, SCALEFACTOR_BITS, SCALEFACTOR_CODES,
};
use crate::mdct::mdct_long;
use crate::sfband::SWB_LONG;
use crate::syntax::{ElementType, AOT_AAC_LC, SAMPLE_RATES};
use crate::window::sine_long;

/// MDCT length (long block).
const FRAME_LEN: usize = 1024;
/// Full windowed block length (= 2*FRAME_LEN).
const BLOCK_LEN: usize = 2 * FRAME_LEN;

/// Magic number in the AAC quantizer rounding rule (§4.6.6):
/// `ix = floor(|x_scaled|^(3/4) + MAGIC_NUMBER)`.
const QUANT_MAGIC: f32 = 0.4054;

/// Forward-MDCT scale matching ffmpeg's AAC encoder convention
/// (`aacenc.c::dsp_init`: `float scale = 32768.0f`). Without this scale
/// the spectrum values come out ~5 orders of magnitude below what AAC
/// inverse quantisation expects, and reference decoders fall back to
/// near-silent output.
const MDCT_FORWARD_SCALE: f32 = 32768.0;

/// Largest un-escaped amplitude supported by book 11.
const ESC_LAV: i32 = 16;

pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    let channels = params
        .channels
        .ok_or_else(|| Error::invalid("AAC encoder: channels required"))?;
    if !(1..=2).contains(&channels) {
        return Err(Error::unsupported(format!(
            "AAC encoder: {channels}-channel encode not supported (mono/stereo only)"
        )));
    }
    let sample_rate = params
        .sample_rate
        .ok_or_else(|| Error::invalid("AAC encoder: sample_rate required"))?;
    let sf_index = SAMPLE_RATES
        .iter()
        .position(|&r| r == sample_rate)
        .ok_or_else(|| {
            Error::unsupported(format!(
                "AAC encoder: sample rate {sample_rate} not supported"
            ))
        })? as u8;

    let bitrate = params.bit_rate.unwrap_or(128_000).max(16_000);

    let mut out_params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
    out_params.media_type = MediaType::Audio;
    out_params.channels = Some(channels);
    out_params.sample_rate = Some(sample_rate);
    out_params.sample_format = Some(SampleFormat::S16);
    out_params.bit_rate = Some(bitrate);

    Ok(Box::new(AacEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        time_base: TimeBase::new(1, sample_rate as i64),
        channels,
        sample_rate,
        sf_index,
        bitrate,
        input_buf: vec![Vec::with_capacity(BLOCK_LEN * 2); channels as usize],
        overlap: vec![vec![0.0f32; FRAME_LEN]; channels as usize],
        output_queue: VecDeque::new(),
        pts: 0,
        flushed: false,
    }))
}

struct AacEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    time_base: TimeBase,
    channels: u16,
    sample_rate: u32,
    sf_index: u8,
    bitrate: u64,
    /// Per-channel running PCM buffer (float in [-1, 1]).
    input_buf: Vec<Vec<f32>>,
    /// Per-channel overlap from the previous block's right half.
    overlap: Vec<Vec<f32>>,
    output_queue: VecDeque<Packet>,
    pts: i64,
    flushed: bool,
}

impl AacEncoder {
    fn push_audio_frame(&mut self, frame: &AudioFrame) -> Result<()> {
        if frame.channels != self.channels {
            return Err(Error::invalid(format!(
                "AAC encoder: expected {} channels, got {}",
                self.channels, frame.channels
            )));
        }
        let n = frame.samples as usize;
        if n == 0 {
            return Ok(());
        }
        let plane = frame
            .data
            .first()
            .ok_or_else(|| Error::invalid("AAC encoder: frame missing data plane"))?;
        match frame.format {
            SampleFormat::S16 => {
                let stride = self.channels as usize * 2;
                if plane.len() < n * stride {
                    return Err(Error::invalid("AAC encoder: S16 frame too short"));
                }
                for i in 0..n {
                    for ch in 0..self.channels as usize {
                        let off = i * stride + ch * 2;
                        let s = i16::from_le_bytes([plane[off], plane[off + 1]]);
                        self.input_buf[ch].push(s as f32 / 32768.0);
                    }
                }
            }
            SampleFormat::F32 => {
                let stride = self.channels as usize * 4;
                if plane.len() < n * stride {
                    return Err(Error::invalid("AAC encoder: F32 frame too short"));
                }
                for i in 0..n {
                    for ch in 0..self.channels as usize {
                        let off = i * stride + ch * 4;
                        let v = f32::from_le_bytes([
                            plane[off],
                            plane[off + 1],
                            plane[off + 2],
                            plane[off + 3],
                        ]);
                        self.input_buf[ch].push(v);
                    }
                }
            }
            other => {
                return Err(Error::unsupported(format!(
                    "AAC encoder: input sample format {other:?} not supported"
                )));
            }
        }
        Ok(())
    }

    /// Emit one or more AAC frames while we have a full FRAME_LEN of new
    /// samples buffered.
    fn drain_blocks(&mut self) -> Result<()> {
        while self.input_buf[0].len() >= FRAME_LEN {
            self.emit_block(false)?;
        }
        Ok(())
    }

    /// Emit a final (possibly silence-padded) block when flushing.
    fn flush_final(&mut self) -> Result<()> {
        // Drain whatever full blocks remain.
        self.drain_blocks()?;
        // If there are any leftover samples, pad to FRAME_LEN with zeros
        // and emit one more block so the decoder's overlap-add produces
        // the last samples.
        if self.input_buf[0].is_empty() {
            // Even with no pending samples, emit a silence-block tail so
            // the decoder's first-frame latency is flushed out.
            for ch in 0..self.channels as usize {
                self.input_buf[ch].resize(FRAME_LEN, 0.0);
            }
            self.emit_block(true)?;
            return Ok(());
        }
        for ch in 0..self.channels as usize {
            if self.input_buf[ch].len() < FRAME_LEN {
                self.input_buf[ch].resize(FRAME_LEN, 0.0);
            }
        }
        self.emit_block(true)?;
        Ok(())
    }

    fn emit_block(&mut self, _is_last: bool) -> Result<()> {
        let n_ch = self.channels as usize;
        // Build the 2N windowed block per channel:
        //   first_half = overlap[ch]      (was saved after last block)
        //   second_half = next FRAME_LEN samples from input_buf (window'd)
        let mut blocks: Vec<Vec<f32>> = vec![vec![0.0; BLOCK_LEN]; n_ch];
        let win = sine_long();
        for ch in 0..n_ch {
            for i in 0..FRAME_LEN {
                blocks[ch][i] = self.overlap[ch][i] * win[i];
            }
            // Pull next FRAME_LEN samples.
            for i in 0..FRAME_LEN {
                let sample = self.input_buf[ch][i];
                blocks[ch][FRAME_LEN + i] = sample * win[FRAME_LEN - 1 - i];
            }
        }
        // Update overlap to the *unwindowed* upcoming-new samples so the
        // next block's first_half * win(rising) matches the just-emitted
        // second_half * win(falling) and OLA reconstructs the input.
        for ch in 0..n_ch {
            let new_overlap: Vec<f32> = self.input_buf[ch][..FRAME_LEN].to_vec();
            self.overlap[ch] = new_overlap;
            self.input_buf[ch].drain(..FRAME_LEN);
        }

        // Forward MDCT per channel. ffmpeg's AAC encoder applies a 32768
        // scale on the forward MDCT (to match the int16 input range —
        // see `aacenc.c::dsp_init`). We do the same so the spectrum
        // values land in the range the standard inverse-quantisation
        // expects.
        let mut specs: Vec<Vec<f32>> = vec![vec![0.0; FRAME_LEN]; n_ch];
        for ch in 0..n_ch {
            mdct_long(&blocks[ch], &mut specs[ch]);
            for v in specs[ch].iter_mut() {
                *v *= MDCT_FORWARD_SCALE;
            }
        }

        // Frame header + raw_data_block.
        let payload = self.encode_raw_data_block(&specs)?;

        // Wrap in ADTS.
        let samples_per_frame = FRAME_LEN as u32;
        let mut adts_frame = build_adts_frame(self.sf_index, self.channels as u8, payload.len());
        adts_frame.extend_from_slice(&payload);

        let pkt = Packet::new(0, self.time_base, adts_frame).with_pts(self.pts);
        self.pts += samples_per_frame as i64;
        self.output_queue.push_back(pkt);
        Ok(())
    }

    fn encode_raw_data_block(&self, specs: &[Vec<f32>]) -> Result<Vec<u8>> {
        let mut bw = BitWriter::with_capacity(1024);
        if self.channels == 1 {
            bw.write_u32(ElementType::Sce as u32, 3);
            bw.write_u32(0, 4); // element_instance_tag
            write_single_ics(&mut bw, &specs[0], self.sf_index, false)?;
        } else {
            // Channel Pair Element.
            bw.write_u32(ElementType::Cpe as u32, 3);
            bw.write_u32(0, 4); // element_instance_tag
            write_cpe(&mut bw, &specs[0], &specs[1], self.sf_index)?;
        }
        // ID_END
        bw.write_u32(ElementType::End as u32, 3);
        bw.align_to_byte();
        Ok(bw.finish())
    }
}

impl Encoder for AacEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if self.flushed {
            return Err(Error::other(
                "AAC encoder: flushed, cannot accept more frames",
            ));
        }
        match frame {
            Frame::Audio(af) => {
                self.push_audio_frame(af)?;
                self.drain_blocks()
            }
            Frame::Video(_) => Err(Error::invalid("AAC encoder: got video frame")),
        }
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        match self.output_queue.pop_front() {
            Some(p) => Ok(p),
            None => {
                if self.flushed {
                    Err(Error::Eof)
                } else {
                    Err(Error::NeedMore)
                }
            }
        }
    }

    fn flush(&mut self) -> Result<()> {
        if self.flushed {
            return Ok(());
        }
        self.flush_final()?;
        self.flushed = true;
        Ok(())
    }
}

// ==================== ADTS framing ====================

/// Build the 7-byte ADTS header for the given payload length.
fn build_adts_frame(sf_index: u8, channel_configuration: u8, payload_len: usize) -> Vec<u8> {
    let frame_length = payload_len + 7; // no CRC
    assert!(frame_length < (1 << 13));
    let mut hdr = [0u8; 7];
    // syncword 0xFFF
    hdr[0] = 0xFF;
    hdr[1] = 0xF0;
    // ID = 0 (MPEG-4), layer = 00, protection_absent = 1.
    hdr[1] |= 0b0001;
    // profile (AAC-LC) = 1 (0-based, so stored as 2-1=1)
    // sampling_frequency_index (4 bits), private_bit=0, channel_configuration (3 bits)
    let profile = AOT_AAC_LC - 1; // = 1
    hdr[2] = (profile << 6) | ((sf_index & 0x0F) << 2) | ((channel_configuration >> 2) & 0x01);
    hdr[3] = ((channel_configuration & 0x03) << 6) | ((frame_length >> 11) as u8 & 0x03);
    hdr[4] = ((frame_length >> 3) & 0xFF) as u8;
    // buffer_fullness = 0x7FF (variable)
    hdr[5] = (((frame_length & 0x07) << 5) as u8) | 0b11111;
    hdr[6] = 0b11111100; // remaining 6 fullness bits + number_of_raw_blocks=0
    hdr.to_vec()
}

// ==================== SCE / CPE writers ====================

fn write_single_ics(bw: &mut BitWriter, spec: &[f32], sf_index: u8, _in_cpe: bool) -> Result<()> {
    // global_gain (8 bits) — set later. For now write a placeholder.
    // Design: we need to pick scalefactors first so we know the gain, then
    // write the full ICS in one pass. We encode everything into a temp
    // structure and emit at the end.
    let ics = analyse_and_quantise(spec, sf_index)?;
    write_ics(bw, &ics, false)?;
    Ok(())
}

fn write_cpe(bw: &mut BitWriter, spec_l: &[f32], spec_r: &[f32], sf_index: u8) -> Result<()> {
    // Decide M/S stereo per band. We build M/S spectra, then try both
    // representations and pick whichever needs fewer bits overall.
    let (ms_used, ics_l, ics_r) = analyse_cpe(spec_l, spec_r, sf_index)?;

    bw.write_bit(true); // common_window — share ics_info between the channels
                        // The shared ics_info uses ch0's max_sfb (which equals ch1's after the
                        // pad-to-max-sfb done in analyse_cpe).
    write_ics_info(bw, &ics_l.info);
    let any_ms = ms_used.iter().any(|&b| b);
    if any_ms {
        bw.write_u32(1, 2); // ms_mask_present = 1 (explicit per-band mask)
                            // For long blocks num_window_groups = 1, so the mask is written
                            // as max_sfb consecutive bits.
        for sfb in 0..ics_l.info.max_sfb as usize {
            bw.write_bit(ms_used.get(sfb).copied().unwrap_or(false));
        }
    } else {
        bw.write_u32(0, 2); // ms_mask_present = 0
    }
    // Per-channel individual_channel_stream (no ics_info because common):
    //   global_gain (8) | section_data | scalefactor_data |
    //   pulse_data_present (1) | tns_data_present (1) |
    //   gain_control_data_present (1) | spectral_data
    write_ics_body(bw, &ics_l)?;
    write_ics_body(bw, &ics_r)?;
    Ok(())
}

// ==================== ICS analysis & writing ====================

#[derive(Clone, Debug)]
struct Ics {
    info: IcsInfoEnc,
    /// Per-band global scalefactor (8-bit int — first band is absolute,
    /// subsequent bands are deltas on encode).
    sfs: Vec<i32>,
    /// Per-band chosen codebook (0..=11).
    cbs: Vec<u8>,
    /// Per-band quantised coefficients laid out in band order.
    q_bands: Vec<Vec<i32>>,
    /// Global gain value (first non-ZERO band's scalefactor, 8-bit).
    global_gain: u8,
}

#[derive(Clone, Debug)]
struct IcsInfoEnc {
    max_sfb: u8,
    sf_index: u8,
}

fn analyse_and_quantise(spec: &[f32], sf_index: u8) -> Result<Ics> {
    let swb = SWB_LONG[sf_index as usize];
    let total_sfb = swb.len() - 1;

    // Compute the highest band carrying significant energy. This sets
    // `max_sfb`; we can stop quantising past it. We use a relative
    // threshold (1/8000 of peak) so trivial leakage from the MDCT
    // doesn't pull `max_sfb` out to the Nyquist boundary on every frame.
    let global_peak = spec.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let threshold = (global_peak * 1e-4).max(1e-3);
    let mut max_band_active = 0usize;
    for sfb in 0..total_sfb {
        let start = swb[sfb] as usize;
        let end = swb[sfb + 1] as usize;
        let mx = spec[start..end].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        if mx > threshold {
            max_band_active = sfb + 1;
        }
    }
    let max_sfb = max_band_active.max(1).min(total_sfb);

    // Pick per-band scalefactor so the largest quantised magnitude lands
    // in a useful range. We aim for `target_max ≈ 7` so smaller bands
    // can use the cheaper books 7-8 (LAV 7) instead of always falling
    // back to book 11. Loud bands will still drift higher and end up on
    // book 9/10/11 — that's fine.
    let target_max = 7i32;
    let mut sfs = vec![0i32; max_sfb];
    let mut q_bands: Vec<Vec<i32>> = Vec::with_capacity(max_sfb);
    for sfb in 0..max_sfb {
        let start = swb[sfb] as usize;
        let end = swb[sfb + 1] as usize;
        let band = &spec[start..end];
        let max_abs = band.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        if max_abs <= threshold {
            // Zero band.
            sfs[sfb] = 0; // treated as absent — cb=0
            q_bands.push(vec![0i32; end - start]);
            continue;
        }
        // Find the smallest scalefactor that makes ceil((|max|/2^((sf-100)/4))^(3/4))
        // <= target_max. Solve: 2^((sf-100)/4) >= (max_abs / target_max^(4/3))
        // => sf >= 100 + 4 * log2(max_abs / target_max^(4/3))
        let tgt_inv = (target_max as f32).powf(4.0 / 3.0);
        let ratio = max_abs / tgt_inv;
        let sf_f = 100.0 + 4.0 * ratio.log2();
        let mut sf = sf_f.ceil() as i32;
        sf = sf.clamp(0, 255);
        // Quantise with this sf; if any coefficient lands above ESC_LAV,
        // bump sf and retry (rare path).
        let (q, ok) = quantise_band(band, sf);
        if ok {
            sfs[sfb] = sf;
            q_bands.push(q);
        } else {
            let mut sf2 = sf + 1;
            let final_q;
            loop {
                let (q2, ok2) = quantise_band(band, sf2);
                if ok2 || sf2 >= 255 {
                    final_q = q2;
                    break;
                }
                sf2 += 1;
            }
            sfs[sfb] = sf2;
            q_bands.push(final_q);
        }
    }

    // Pick codebook per band.
    let mut cbs = vec![0u8; max_sfb];
    for sfb in 0..max_sfb {
        let q = &q_bands[sfb];
        cbs[sfb] = if q.iter().all(|&x| x == 0) {
            0
        } else {
            best_codebook_for_band(q)
        };
    }

    // Global gain: first non-zero band's scalefactor. If everything is
    // zero, use 100.
    let mut gg: i32 = 100;
    for sfb in 0..max_sfb {
        if cbs[sfb] != 0 {
            gg = sfs[sfb];
            break;
        }
    }
    let gg_clamped = gg.clamp(0, 255) as u8;

    // Re-anchor sfs so that the *first non-zero band's* scalefactor = gg
    // and subsequent non-zero bands carry deltas on top of each previous
    // non-zero band. The decoder uses `g_gain = global_gain`, then for
    // every band with cb != ZERO it adds a delta. Zero bands don't read
    // a delta. So we just need the non-zero-band scalefactors. Zero-band
    // scalefactors are never written.

    Ok(Ics {
        info: IcsInfoEnc {
            max_sfb: max_sfb as u8,
            sf_index,
        },
        sfs,
        cbs,
        q_bands,
        global_gain: gg_clamped,
    })
}

fn quantise_band(band: &[f32], sf: i32) -> (Vec<i32>, bool) {
    let inv_gain = 2.0f32.powf(-(sf as f32 - 100.0) / 4.0);
    let mut out = Vec::with_capacity(band.len());
    let mut ok = true;
    for &x in band {
        if x == 0.0 {
            out.push(0);
            continue;
        }
        let scaled = x * inv_gain;
        let q_abs = scaled.abs().powf(3.0 / 4.0) + QUANT_MAGIC;
        let q = q_abs.floor() as i32;
        let signed = if scaled < 0.0 { -q } else { q };
        if signed.abs() > 8191 {
            ok = false; // beyond the 13-bit amplitude escape range
        }
        out.push(signed);
    }
    // Also mark failure if max unsigned abs > ESC_LAV and escape is
    // impossible (it's always possible via book 11, but the amplitude
    // field tops out at 13 bits — handled above).
    (out, ok)
}

/// For a given vector of quantised coefficients (length = band size),
/// return the codebook index (1..=11) that minimises total Huffman bits.
fn best_codebook_for_band(q: &[i32]) -> u8 {
    let mut best_cb = 11u8;
    let mut best_bits = u64::MAX;
    for cb in 1u8..=11 {
        if let Some(bits) = try_encode_bits(q, cb) {
            if bits < best_bits {
                best_bits = bits;
                best_cb = cb;
            }
        }
    }
    best_cb
}

/// Compute (without writing) the bit cost of encoding `q` under codebook
/// `cb`. Returns `None` if any element exceeds the codebook's LAV with no
/// escape capability.
fn try_encode_bits(q: &[i32], cb: u8) -> Option<u64> {
    let book = encoder_book(cb);
    let dim = book.dim as usize;
    if q.len() % dim != 0 {
        // Bands aren't necessarily multiples of 2/4; this shouldn't
        // happen for AAC-LC (SWB_LONG bands are all multiples of 4),
        // but guard anyway.
        return None;
    }
    let lav = book.lav as i32;
    let mut total_bits = 0u64;
    let mut i = 0;
    while i < q.len() {
        let (idx, extra_bits, ok) = pack_tuple_index(&q[i..i + dim], book, lav);
        if !ok {
            return None;
        }
        total_bits += book.bits[idx] as u64 + extra_bits;
        i += dim;
    }
    Some(total_bits)
}

/// Write the bits for `q` under `cb` to `bw`.
fn write_band_bits(bw: &mut BitWriter, q: &[i32], cb: u8) {
    let book = encoder_book(cb);
    let dim = book.dim as usize;
    let lav = book.lav as i32;
    let mut i = 0;
    while i < q.len() {
        let (idx, _extra_bits, ok) = pack_tuple_index(&q[i..i + dim], book, lav);
        debug_assert!(ok);
        // Huffman codeword.
        bw.write_u32(book.codes[idx] as u32, book.bits[idx] as u32);
        // Unsigned books: append sign bits for non-zero coefficients.
        if !book.signed {
            for &v in &q[i..i + dim] {
                if v != 0 {
                    bw.write_bit(v < 0);
                }
            }
        }
        // Book 11 escape.
        if book.escape {
            for &v in &q[i..i + dim] {
                if v.abs() >= ESC_LAV {
                    write_escape_amp(bw, v.unsigned_abs());
                }
            }
        }
        i += dim;
    }
}

/// Compute the Huffman symbol index for a tuple of `dim` coefficients
/// under `book`. For escape books (11), clamp to ±16 in the index; the
/// caller emits the escape amplitude separately.
///
/// Returns (index, extra-bits-needed-for-escape-amp, ok).
fn pack_tuple_index(tuple: &[i32], book: &EncBook, lav: i32) -> (usize, u64, bool) {
    let dim = book.dim as usize;
    if book.signed {
        // Digits in [-lav, lav], 2*lav+1 possibilities per position.
        let modulo = 2 * lav + 1;
        let mut idx = 0i32;
        for &v in &tuple[..dim] {
            if v < -lav || v > lav {
                return (0, 0, false);
            }
            idx = idx * modulo + (v + lav);
        }
        (idx as usize, 0, true)
    } else {
        // Unsigned: digits in [0, lav]; sign is carried separately.
        let modulo = lav + 1;
        let mut idx = 0i32;
        let mut extra = 0u64;
        for &v in &tuple[..dim] {
            let mut a = v.abs();
            if book.escape {
                if a >= ESC_LAV {
                    extra += escape_amp_bits(a as u32) as u64;
                    a = ESC_LAV;
                }
            } else if a > lav {
                return (0, 0, false);
            }
            idx = idx * modulo + a;
        }
        (idx as usize, extra, true)
    }
}

/// Number of bits used by the escape amplitude code for value `a`. The
/// escape code is a unary-prefix `1..1 0` of length `prefix` ones plus a
/// terminating zero, followed by `prefix + 4` raw bits. For an amplitude
/// `a`, `prefix = floor(log2(a)) - 4`.
fn escape_amp_bits(a: u32) -> u32 {
    // a must be >= 16 (= ESC_LAV).
    let top = 31 - a.leading_zeros(); // floor(log2(a))
    let prefix = top.saturating_sub(4);
    // unary prefix (prefix ones) + terminator zero (1 bit) + prefix+4 raw bits
    prefix + 1 + prefix + 4
}

/// Emit the escape-amplitude code for absolute value `a` (expects a >= 16).
fn write_escape_amp(bw: &mut BitWriter, a: u32) {
    let top = 31 - a.leading_zeros();
    let prefix = top.saturating_sub(4);
    for _ in 0..prefix {
        bw.write_bit(true);
    }
    bw.write_bit(false);
    let raw = a & ((1u32 << (prefix + 4)) - 1);
    bw.write_u32(raw, prefix + 4);
}

// ==================== ICS bitstream writers ====================

fn write_ics_info(bw: &mut BitWriter, info: &IcsInfoEnc) {
    bw.write_bit(false); // ics_reserved_bit
    bw.write_u32(0, 2); // window_sequence = ONLY_LONG_SEQUENCE
    bw.write_u32(0, 1); // window_shape = sine
    bw.write_u32(info.max_sfb as u32, 6);
    bw.write_bit(false); // predictor_data_present
}

/// Write the full SCE individual_channel_stream payload. The SCE caller
/// has already emitted element_instance_tag (4 bits). Layout:
///   global_gain (8) | ics_info | body
fn write_ics(bw: &mut BitWriter, ics: &Ics, _in_cpe: bool) -> Result<()> {
    bw.write_u32(ics.global_gain as u32, 8);
    write_ics_info(bw, &ics.info);
    write_ics_body_no_global_gain(bw, ics)
}

/// Write per-channel CPE body (used inside a CPE with common_window=1).
/// Layout (per spec individual_channel_stream when ics_info is shared):
///   global_gain (8) | section_data | scale_factor_data |
///   pulse_data_present (1) | tns_data_present (1) | gain_control_present (1) |
///   spectral_data.
fn write_ics_body(bw: &mut BitWriter, ics: &Ics) -> Result<()> {
    bw.write_u32(ics.global_gain as u32, 8);
    write_ics_body_no_global_gain(bw, ics)
}

fn write_ics_body_no_global_gain(bw: &mut BitWriter, ics: &Ics) -> Result<()> {
    let _ = ics.info.sf_index;
    write_section_data(bw, ics);
    write_scalefactors(bw, ics)?;
    bw.write_bit(false); // pulse_data_present
    bw.write_bit(false); // tns_data_present
    bw.write_bit(false); // gain_control_data_present
    write_spectral_data(bw, ics);
    Ok(())
}

fn write_section_data(bw: &mut BitWriter, ics: &Ics) {
    let max_sfb = ics.info.max_sfb as usize;
    if max_sfb == 0 {
        return;
    }
    // Long-only: sect_bits = 5, sect_esc_val = 31.
    let sect_bits: u32 = 5;
    let sect_esc_val: u32 = (1 << sect_bits) - 1;

    let mut k = 0usize;
    while k < max_sfb {
        let cb = ics.cbs[k];
        let mut run = 1usize;
        while k + run < max_sfb && ics.cbs[k + run] == cb {
            run += 1;
        }
        bw.write_u32(cb as u32, 4);
        // Write the length in `sect_bits` chunks; each chunk == sect_esc_val
        // means "more bits follow", and the terminating chunk is < sect_esc_val.
        let mut remaining = run as u32;
        while remaining >= sect_esc_val {
            bw.write_u32(sect_esc_val, sect_bits);
            remaining -= sect_esc_val;
        }
        bw.write_u32(remaining, sect_bits);
        k += run;
    }
}

fn write_scalefactors(bw: &mut BitWriter, ics: &Ics) -> Result<()> {
    let max_sfb = ics.info.max_sfb as usize;
    // Walk bands in decode order; for each non-ZERO band emit the
    // scalefactor delta via the scalefactor Huffman codebook. The
    // decoder seeds `g_gain = global_gain` and we need the *first*
    // non-zero band to land exactly on `global_gain` so that delta=0.
    let mut cur: i32 = ics.global_gain as i32;
    for sfb in 0..max_sfb {
        let cb = ics.cbs[sfb];
        if cb == 0 {
            continue;
        }
        let target = ics.sfs[sfb];
        let delta = (target - cur).clamp(-60, 60);
        cur += delta;
        // Emit via the SF Huffman table (index = delta + 60).
        let idx = (delta + 60) as usize;
        let code = SCALEFACTOR_CODES[idx] as u32;
        let bits = SCALEFACTOR_BITS[idx] as u32;
        bw.write_u32(code, bits);
    }
    Ok(())
}

fn write_spectral_data(bw: &mut BitWriter, ics: &Ics) {
    let max_sfb = ics.info.max_sfb as usize;
    for sfb in 0..max_sfb {
        let cb = ics.cbs[sfb];
        if cb == 0 {
            continue; // codebook 0 bands emit no coefficients
        }
        write_band_bits(bw, &ics.q_bands[sfb], cb);
    }
}

// ==================== CPE analysis ====================

/// Choose M/S stereo per band and return per-channel ICS.
///
/// For common-window CPE both channels MUST share the same ics_info
/// (window_sequence, max_sfb, etc.). We therefore pad both per-channel
/// ICS structures to a single unified max_sfb after analysis.
fn analyse_cpe(l: &[f32], r: &[f32], sf_index: u8) -> Result<(Vec<bool>, Ics, Ics)> {
    // Quantise L/R and M/S independently; pick the cheaper one per band.
    let ics_l_alone = analyse_and_quantise(l, sf_index)?;
    let ics_r_alone = analyse_and_quantise(r, sf_index)?;
    let mut m = vec![0.0f32; l.len()];
    let mut s = vec![0.0f32; l.len()];
    for i in 0..l.len() {
        m[i] = (l[i] + r[i]) * 0.5;
        s[i] = (l[i] - r[i]) * 0.5;
    }
    let ics_m = analyse_and_quantise(&m, sf_index)?;
    let ics_s = analyse_and_quantise(&s, sf_index)?;

    let max_sfb_lr = ics_l_alone.info.max_sfb.max(ics_r_alone.info.max_sfb);
    let max_sfb_ms = ics_m.info.max_sfb.max(ics_s.info.max_sfb);
    let max_sfb = max_sfb_lr.max(max_sfb_ms) as usize;

    let cost_lr: Vec<u64> = (0..max_sfb)
        .map(|sfb| band_bit_cost(sfb, &ics_l_alone) + band_bit_cost(sfb, &ics_r_alone))
        .collect();
    let cost_ms: Vec<u64> = (0..max_sfb)
        .map(|sfb| band_bit_cost(sfb, &ics_m) + band_bit_cost(sfb, &ics_s))
        .collect();
    let mut ms_used = vec![false; max_sfb];
    for sfb in 0..max_sfb {
        ms_used[sfb] = cost_ms[sfb] < cost_lr[sfb];
    }

    let mut ch0 = empty_ics(max_sfb, sf_index);
    let mut ch1 = empty_ics(max_sfb, sf_index);
    for sfb in 0..max_sfb {
        if ms_used[sfb] {
            copy_band(&mut ch0, sfb, &ics_m, sfb);
            copy_band(&mut ch1, sfb, &ics_s, sfb);
        } else {
            copy_band(&mut ch0, sfb, &ics_l_alone, sfb);
            copy_band(&mut ch1, sfb, &ics_r_alone, sfb);
        }
    }
    // Common-window CPE: both channels MUST share ics_info.max_sfb.
    // Anchor global_gain on each channel without trimming max_sfb.
    finalize_ics_keep_max_sfb(&mut ch0);
    finalize_ics_keep_max_sfb(&mut ch1);

    Ok((ms_used, ch0, ch1))
}

fn band_bit_cost(sfb: usize, ics: &Ics) -> u64 {
    if sfb >= ics.info.max_sfb as usize {
        return 0;
    }
    let cb = ics.cbs[sfb];
    if cb == 0 {
        return 0;
    }
    try_encode_bits(&ics.q_bands[sfb], cb).unwrap_or(u64::MAX / 4)
}

fn empty_ics(max_sfb: usize, sf_index: u8) -> Ics {
    let swb = SWB_LONG[sf_index as usize];
    let mut q_bands = Vec::with_capacity(max_sfb);
    for sfb in 0..max_sfb {
        let len = (swb[sfb + 1] - swb[sfb]) as usize;
        q_bands.push(vec![0i32; len]);
    }
    Ics {
        info: IcsInfoEnc {
            max_sfb: max_sfb as u8,
            sf_index,
        },
        sfs: vec![0; max_sfb],
        cbs: vec![0; max_sfb],
        q_bands,
        global_gain: 100,
    }
}

fn copy_band(dst: &mut Ics, dst_sfb: usize, src: &Ics, src_sfb: usize) {
    if src_sfb >= src.info.max_sfb as usize {
        // Zero band — leave dst defaults.
        return;
    }
    dst.sfs[dst_sfb] = src.sfs[src_sfb];
    dst.cbs[dst_sfb] = src.cbs[src_sfb];
    dst.q_bands[dst_sfb] = src.q_bands[src_sfb].clone();
}

fn finalize_ics(ics: &mut Ics) {
    // Trim trailing zero bands from max_sfb.
    let mut max_sfb = ics.info.max_sfb as usize;
    while max_sfb > 0 && ics.cbs[max_sfb - 1] == 0 {
        max_sfb -= 1;
    }
    ics.info.max_sfb = max_sfb as u8;
    ics.sfs.truncate(max_sfb);
    ics.cbs.truncate(max_sfb);
    ics.q_bands.truncate(max_sfb);
    finalize_ics_keep_max_sfb(ics);
}

/// Pick global_gain (= first non-zero band's scalefactor) without trimming
/// `max_sfb` — used for CPE common-window where both channels must share
/// the same band count.
fn finalize_ics_keep_max_sfb(ics: &mut Ics) {
    let mut gg = 100i32;
    for sfb in 0..ics.info.max_sfb as usize {
        if ics.cbs[sfb] != 0 {
            gg = ics.sfs[sfb];
            break;
        }
    }
    ics.global_gain = gg.clamp(0, 255) as u8;
}

// ==================== Encoder-side Huffman helpers ====================

struct EncBook {
    dim: u8,
    lav: u8,
    signed: bool,
    escape: bool,
    codes: &'static [u16],
    bits: &'static [u8],
}

static BOOK1_E: EncBook = EncBook {
    dim: 4,
    lav: 1,
    signed: true,
    escape: false,
    codes: BOOK1_CODES,
    bits: BOOK1_BITS,
};
static BOOK2_E: EncBook = EncBook {
    dim: 4,
    lav: 1,
    signed: true,
    escape: false,
    codes: BOOK2_CODES,
    bits: BOOK2_BITS,
};
static BOOK3_E: EncBook = EncBook {
    dim: 4,
    lav: 2,
    signed: false,
    escape: false,
    codes: BOOK3_CODES,
    bits: BOOK3_BITS,
};
static BOOK4_E: EncBook = EncBook {
    dim: 4,
    lav: 2,
    signed: false,
    escape: false,
    codes: BOOK4_CODES,
    bits: BOOK4_BITS,
};
static BOOK5_E: EncBook = EncBook {
    dim: 2,
    lav: 4,
    signed: true,
    escape: false,
    codes: BOOK5_CODES,
    bits: BOOK5_BITS,
};
static BOOK6_E: EncBook = EncBook {
    dim: 2,
    lav: 4,
    signed: true,
    escape: false,
    codes: BOOK6_CODES,
    bits: BOOK6_BITS,
};
static BOOK7_E: EncBook = EncBook {
    dim: 2,
    lav: 7,
    signed: false,
    escape: false,
    codes: BOOK7_CODES,
    bits: BOOK7_BITS,
};
static BOOK8_E: EncBook = EncBook {
    dim: 2,
    lav: 7,
    signed: false,
    escape: false,
    codes: BOOK8_CODES,
    bits: BOOK8_BITS,
};
static BOOK9_E: EncBook = EncBook {
    dim: 2,
    lav: 12,
    signed: false,
    escape: false,
    codes: BOOK9_CODES,
    bits: BOOK9_BITS,
};
static BOOK10_E: EncBook = EncBook {
    dim: 2,
    lav: 12,
    signed: false,
    escape: false,
    codes: BOOK10_CODES,
    bits: BOOK10_BITS,
};
static BOOK11_E: EncBook = EncBook {
    dim: 2,
    lav: 16,
    signed: false,
    escape: true,
    codes: BOOK11_CODES,
    bits: BOOK11_BITS,
};

fn encoder_book(cb: u8) -> &'static EncBook {
    match cb {
        1 => &BOOK1_E,
        2 => &BOOK2_E,
        3 => &BOOK3_E,
        4 => &BOOK4_E,
        5 => &BOOK5_E,
        6 => &BOOK6_E,
        7 => &BOOK7_E,
        8 => &BOOK8_E,
        9 => &BOOK9_E,
        10 => &BOOK10_E,
        11 => &BOOK11_E,
        _ => panic!("encoder_book: invalid codebook {cb}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_adts_header_fields() {
        let frame = build_adts_frame(4, 1, 100); // 44.1 kHz, mono, 100-byte payload
        assert_eq!(frame.len(), 7);
        assert_eq!(frame[0], 0xFF);
        assert!((frame[1] & 0xF0) == 0xF0);
        // frame_length = 107
        let flen = (((frame[3] & 0x3) as usize) << 11)
            | ((frame[4] as usize) << 3)
            | ((frame[5] >> 5) as usize);
        assert_eq!(flen, 107);
    }

    #[test]
    fn escape_amp_bits_exact() {
        // a = 16 → prefix = 0, bits = 0 + 1 + 0 + 4 = 5
        assert_eq!(escape_amp_bits(16), 5);
        // a = 32 → top = 5, prefix = 1, bits = 1 + 1 + 1 + 4 = 7
        assert_eq!(escape_amp_bits(32), 7);
        // a = 8191 → top = 12, prefix = 8, bits = 8 + 1 + 8 + 4 = 21
        assert_eq!(escape_amp_bits(8191), 21);
    }

    #[test]
    fn scalefactor_zero_delta_is_one_bit() {
        // delta=0 is SCALEFACTOR_CODES[60]=0x00 with 1 bit.
        assert_eq!(SCALEFACTOR_CODES[60], 0);
        assert_eq!(SCALEFACTOR_BITS[60], 1);
    }

    #[test]
    fn sf_huffman_roundtrip() {
        use crate::bitreader::BitReader;
        use crate::huffman::decode_scalefactor_delta;
        // Write a series of deltas via the encoder's SF writer logic and
        // verify the decoder reads them back unchanged.
        let deltas: Vec<i32> = (-30..=30).step_by(3).collect();
        let mut bw = BitWriter::new();
        for &d in &deltas {
            let idx = (d + 60) as usize;
            bw.write_u32(SCALEFACTOR_CODES[idx] as u32, SCALEFACTOR_BITS[idx] as u32);
        }
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        for &expect in &deltas {
            let got = decode_scalefactor_delta(&mut br).unwrap();
            assert_eq!(got, expect, "SF roundtrip mismatch");
        }
    }

    #[test]
    fn spectral_book_roundtrip_book8() {
        use crate::bitreader::BitReader;
        use crate::huffman::{decode_spectral, BOOK8};
        // Encode a few unsigned (lav 7) pairs and verify decode.
        let pairs = [(3i32, -5i32), (7, 0), (0, 0), (-2, -7), (1, 1)];
        let mut bw = BitWriter::new();
        for &(a, b) in &pairs {
            let q = [a, b];
            write_band_bits(&mut bw, &q, 8);
        }
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        for &(want_a, want_b) in &pairs {
            let v = decode_spectral(&mut br, &BOOK8).unwrap();
            assert_eq!(v[0] as i32, want_a, "book8 A mismatch");
            assert_eq!(v[1] as i32, want_b, "book8 B mismatch");
        }
    }

    #[test]
    fn book7_index_layout() {
        use crate::bitreader::BitReader;
        use crate::huffman::{decode_spectral, BOOK7};
        // Book 7 (dim=2, lav=7, unsigned, no escape): index = i*8 + j.
        // Try (1, 0) by setting q = [1, 0].
        let mut bw = BitWriter::new();
        write_band_bits(&mut bw, &[1, 0], 7);
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        let v = decode_spectral(&mut br, &BOOK7).unwrap();
        assert_eq!(v[0] as i32, 1);
        assert_eq!(v[1] as i32, 0);
    }

    #[test]
    fn spectral_book_roundtrip_book11_with_escape() {
        use crate::bitreader::BitReader;
        use crate::huffman::{decode_spectral, BOOK11};
        let pairs = [(3i32, -5i32), (16, 0), (-32, 12), (100, -200), (1, 1)];
        let mut bw = BitWriter::new();
        for &(a, b) in &pairs {
            let q = [a, b];
            write_band_bits(&mut bw, &q, 11);
        }
        let bytes = bw.finish();
        let mut br = BitReader::new(&bytes);
        for &(want_a, want_b) in &pairs {
            let v = decode_spectral(&mut br, &BOOK11).unwrap();
            assert_eq!(v[0] as i32, want_a, "book11 A mismatch");
            assert_eq!(v[1] as i32, want_b, "book11 B mismatch");
        }
    }

    #[test]
    fn encoder_smoke_mono() {
        let mut params = CodecParameters::audio(CodecId::new(crate::CODEC_ID_STR));
        params.sample_rate = Some(44_100);
        params.channels = Some(1);
        let mut enc = make_encoder(&params).expect("make encoder");
        // Feed 2048 samples of a 440 Hz sine.
        let mut pcm = Vec::with_capacity(2048 * 2);
        for i in 0..2048 {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44_100.0).sin();
            let s = (v * 0.5 * 32767.0) as i16;
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        let frame = Frame::Audio(AudioFrame {
            format: SampleFormat::S16,
            channels: 1,
            sample_rate: 44_100,
            samples: 2048,
            pts: None,
            time_base: TimeBase::new(1, 44_100),
            data: vec![pcm],
        });
        enc.send_frame(&frame).unwrap();
        let pkt1 = enc.receive_packet().unwrap();
        assert!(pkt1.data.len() >= 7);
        let pkt2 = enc.receive_packet().unwrap();
        assert!(pkt2.data.len() >= 7);
        // Both should be ADTS-framed.
        assert_eq!(pkt1.data[0], 0xFF);
        assert_eq!(pkt2.data[0], 0xFF);
    }
}
