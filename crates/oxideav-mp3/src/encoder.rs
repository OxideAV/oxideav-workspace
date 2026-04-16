//! Minimum-viable CBR MPEG-1 Layer III encoder.
//!
//! Scope (deliberately narrow):
//! - MPEG-1 Layer III, 44.1 kHz or 48 kHz, mono or "dual-channel" stereo
//!   (no joint stereo / no MS / no IS).
//! - One CBR bitrate per encoder instance: 128 / 192 / 256 / 320 kbps.
//! - Long blocks only (block_type = 0). No window switching.
//! - No CRC. No psychoacoustic model. No rate-distortion: a simple
//!   global-gain bisection sets the quantisation step so the Huffman bit
//!   count fits in the available main-data budget.
//! - Single big-value Huffman table for the whole spectrum (selected
//!   per granule from a small candidate set). Region splits are
//!   degenerate (region0 spans everything, region1 / region2 empty).
//! - count1 region uses table A.
//! - Bit reservoir on the encode side: any unused bits roll forward via
//!   the next frame's `main_data_begin`.
//!
//! The pipeline is the mirror of the decoder:
//!   PCM → polyphase analysis → forward MDCT → quantise →
//!   Huffman encode → side info + main data → frame emission.

use std::collections::VecDeque;

use oxideav_codec::Encoder;
use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, MediaType, Packet, Result, SampleFormat,
    TimeBase,
};

use crate::analysis::{analyze_granule, AnalysisState};
use crate::bitwriter::BitWriter;
use crate::huffman::{BIG_VALUE_TABLES, COUNT1_A};
use crate::mdct::{mdct_granule, MdctState};
use crate::CODEC_ID_STR;

/// Maximum reservoir lookback per the spec (MPEG-1).
const MAX_LOOKBACK: usize = 511;

/// Candidate big-value tables, in priority order. We try the lowest-cost
/// table first — values bounded by 15 use table 13 (no linbits); larger
/// magnitudes fall through to one of the linbits-equipped variants
/// (table indices 16-23 reuse TABLE_16 with linbits 1..13).
const BIG_VALUE_CANDIDATES: &[u8] = &[1, 5, 7, 13, 16, 17, 18, 19, 20, 21, 22, 23];

/// Build a CBR encoder for the requested parameters.
pub fn make_encoder(params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    let channels = params
        .channels
        .ok_or_else(|| Error::invalid("MP3 encoder: missing channels"))?;
    if !(1..=2).contains(&channels) {
        return Err(Error::invalid("MP3 encoder: channels must be 1 or 2"));
    }
    let sample_rate = params
        .sample_rate
        .ok_or_else(|| Error::invalid("MP3 encoder: missing sample_rate"))?;
    let sr_index = match sample_rate {
        44_100 => 0u8,
        48_000 => 1u8,
        32_000 => 2u8,
        _ => {
            return Err(Error::unsupported(format!(
                "MP3 encoder: unsupported sample rate {sample_rate} (need 32000/44100/48000)"
            )));
        }
    };

    // Bit rate: default 128 kbps; accept other CBRs from a fixed list.
    let bitrate_kbps = params.bit_rate.map(|b| (b / 1000) as u32).unwrap_or(128);
    let br_index = match bitrate_kbps {
        32 => 1u8,
        40 => 2,
        48 => 3,
        56 => 4,
        64 => 5,
        80 => 6,
        96 => 7,
        112 => 8,
        128 => 9,
        160 => 10,
        192 => 11,
        224 => 12,
        256 => 13,
        320 => 14,
        _ => {
            return Err(Error::unsupported(format!(
                "MP3 encoder: unsupported bitrate {bitrate_kbps} kbps"
            )));
        }
    };

    let sample_format = params.sample_format.unwrap_or(SampleFormat::S16);
    if sample_format != SampleFormat::S16 {
        return Err(Error::unsupported(format!(
            "MP3 encoder: input sample format {sample_format:?} not supported (need S16)"
        )));
    }

    let mut output = params.clone();
    output.media_type = MediaType::Audio;
    output.codec_id = CodecId::new(CODEC_ID_STR);
    output.sample_format = Some(sample_format);
    output.channels = Some(channels);
    output.sample_rate = Some(sample_rate);
    output.bit_rate = Some((bitrate_kbps as u64) * 1000);

    Ok(Box::new(Mp3Encoder {
        output_params: output,
        channels,
        sample_rate,
        bitrate_kbps,
        sr_index,
        br_index,
        time_base: TimeBase::new(1, sample_rate as i64),
        analysis_state: [AnalysisState::new(), AnalysisState::new()],
        mdct_state: [MdctState::new(), MdctState::new()],
        pcm_queue: vec![Vec::new(); channels as usize],
        main_data_queue: Vec::new(),
        pending_packets: VecDeque::new(),
        frame_index: 0,
        eof: false,
        cumulative_padded_bits: 0,
    }))
}

struct Mp3Encoder {
    output_params: CodecParameters,
    channels: u16,
    sample_rate: u32,
    bitrate_kbps: u32,
    sr_index: u8,
    br_index: u8,
    time_base: TimeBase,
    analysis_state: [AnalysisState; 2],
    mdct_state: [MdctState; 2],
    /// Per-channel float queue (samples in -1..=1).
    pcm_queue: Vec<Vec<f32>>,
    /// Pending main-data bytes that have not yet been written to a frame
    /// slot. The next frame's `main_data_begin` is exactly this length
    /// (capped at MAX_LOOKBACK) BEFORE the new main_data gets appended.
    main_data_queue: Vec<u8>,
    pending_packets: VecDeque<Packet>,
    frame_index: u64,
    eof: bool,
    /// Tracks fractional-byte CBR padding so we know when to set the
    /// padding bit. For 44.1 kHz: 144*128_000/44100 = 417.96... so we set
    /// padding ~96/100 frames.
    cumulative_padded_bits: u64,
}

impl Mp3Encoder {
    /// Bytes per CBR frame for given padding.
    fn frame_bytes(&self, padding: bool) -> usize {
        let base = (144 * self.bitrate_kbps * 1000 / self.sample_rate) as usize;
        base + if padding { 1 } else { 0 }
    }

    /// Decide whether this frame should set the padding bit, using the
    /// classic accumulator-style scheme (LAME-equivalent for CBR).
    fn next_padding(&mut self) -> bool {
        // Numerator/denominator of "extra bits per frame" * 8.
        // Frame size in bits = 144_000 * br / sr; integer part is the
        // base; fractional part accumulates and is paid off by inserting
        // a padding byte (8 bits).
        let num = 144_000u64 * self.bitrate_kbps as u64;
        let sr = self.sample_rate as u64;
        // Bits per frame * sr = num. Integer bits = num / sr. Remainder
        // accumulates.
        let _whole = num / sr;
        let rem = num - (num / sr) * sr;
        self.cumulative_padded_bits += rem;
        let pad = self.cumulative_padded_bits >= sr * 8;
        if pad {
            self.cumulative_padded_bits -= sr * 8;
        }
        pad
    }

    fn ingest(&mut self, frame: &AudioFrame) -> Result<()> {
        if frame.channels != self.channels || frame.sample_rate != self.sample_rate {
            return Err(Error::invalid(
                "MP3 encoder: frame channel/sample-rate mismatch",
            ));
        }
        if frame.format != SampleFormat::S16 {
            return Err(Error::invalid(
                "MP3 encoder: input frames must be S16 interleaved",
            ));
        }
        let data = frame
            .data
            .first()
            .ok_or_else(|| Error::invalid("MP3 encoder: empty frame"))?;
        let n_ch = self.channels as usize;
        let n_samples = data.len() / (2 * n_ch);
        for i in 0..n_samples {
            for ch in 0..n_ch {
                let off = (i * n_ch + ch) * 2;
                let s = i16::from_le_bytes([data[off], data[off + 1]]) as f32 / 32768.0;
                self.pcm_queue[ch].push(s);
            }
        }
        self.flush_ready_frames(false)
    }

    fn flush_ready_frames(&mut self, drain: bool) -> Result<()> {
        // MPEG-1: 1152 samples per frame (2 granules of 576).
        let n_ch = self.channels as usize;
        loop {
            let avail = self.pcm_queue[0].len();
            if avail < 1152 {
                if drain && avail > 0 {
                    // Pad with zeros to 1152 to flush the tail.
                    for ch in 0..n_ch {
                        self.pcm_queue[ch].resize(1152, 0.0);
                    }
                } else {
                    return Ok(());
                }
            }
            let pkt = self.encode_one_frame()?;
            self.pending_packets.push_back(pkt);
            if drain && self.pcm_queue[0].iter().all(|&v| v == 0.0) {
                return Ok(());
            }
        }
    }

    fn encode_one_frame(&mut self) -> Result<Packet> {
        let n_ch = self.channels as usize;

        // Pull 1152 samples per channel into local buffers, drain queue.
        let mut pcm_in: Vec<[f32; 1152]> = vec![[0.0f32; 1152]; n_ch];
        for ch in 0..n_ch {
            for i in 0..1152 {
                pcm_in[ch][i] = self.pcm_queue[ch][i];
            }
            self.pcm_queue[ch].drain(..1152);
        }

        // Analysis + MDCT per granule per channel.
        let mut xr: Vec<Vec<[f32; 576]>> = (0..2)
            .map(|_| (0..n_ch).map(|_| [0.0f32; 576]).collect())
            .collect();
        for gr in 0..2 {
            for ch in 0..n_ch {
                let mut pcm_gr = [0.0f32; 576];
                pcm_gr.copy_from_slice(&pcm_in[ch][gr * 576..gr * 576 + 576]);
                let mut sub = [[0.0f32; 18]; 32];
                analyze_granule(&mut self.analysis_state[ch], &pcm_gr, &mut sub);
                mdct_granule(&sub, &mut xr[gr][ch], &mut self.mdct_state[ch]);
            }
        }

        // Frame layout / size.
        let padding = self.next_padding();
        let frame_bytes = self.frame_bytes(padding);
        let header_bytes = 4usize;
        let si_bytes = if n_ch == 1 { 17 } else { 32 };
        let main_data_slot_bytes = frame_bytes - header_bytes - si_bytes;

        // The encoder maintains a single byte stream of main_data
        // (`self.main_data_queue`). For this frame:
        //   1. main_data_begin = current queue length (capped at MAX_LOOKBACK).
        //   2. Generate this frame's main_data bytes from Huffman.
        //   3. Append to queue.
        //   4. Pop the first `main_data_slot_bytes` from the queue into the
        //      on-wire slot. Pad with zeros if the queue is short.
        //
        // For the budget we target this frame's main_data_bytes length so
        // that the queue stays within MAX_LOOKBACK after popping the slot.
        let pre_queue = self.main_data_queue.len();
        // pre_queue should already satisfy pre_queue <= MAX_LOOKBACK and
        // <= cumulative slots written so far (the budget logic below
        // enforces both invariants).
        debug_assert!(pre_queue <= MAX_LOOKBACK);
        let main_data_begin = pre_queue as u16;

        // Constraints on the size of THIS frame's main_data, in bytes:
        // (A) Decode constraint: this frame's main_data must fit in the
        //     `mdb + slot` window the decoder constructs:
        //       M_N <= pre_queue + main_data_slot_bytes
        // (B) Lookback constraint for FUTURE frames: queue after this
        //     frame (= pre_queue + M_N - main_data_slot_bytes, clamped at
        //     0) must be reachable as a lookback in frame N+1:
        //       queue_after <= MAX_LOOKBACK   (decoder reservoir cap)
        // Combining: M_N <= main_data_slot_bytes + MAX_LOOKBACK - pre_queue
        //                  AND M_N <= pre_queue + main_data_slot_bytes.
        // (A) is the tighter constraint while pre_queue < MAX_LOOKBACK / 2.
        let max_main_bytes_a = pre_queue + main_data_slot_bytes;
        let max_main_bytes_b = main_data_slot_bytes + MAX_LOOKBACK - pre_queue;
        let max_main_bytes = max_main_bytes_a.min(max_main_bytes_b);
        let max_main_bits = max_main_bytes * 8;
        // Per-unit (granule × channel) budget. Allow some slack so the
        // bisection has room to find a good gain.
        let per_unit_budget = max_main_bits / (2 * n_ch);

        // Encode each granule/channel.
        let mut granule_data: Vec<Vec<GranuleEncoded>> =
            (0..2).map(|_| Vec::with_capacity(n_ch)).collect();
        let mut bits_used_total: usize = 0;
        for gr in 0..2 {
            for ch in 0..n_ch {
                let remaining = max_main_bits.saturating_sub(bits_used_total);
                let units_left = (2 - gr) * n_ch - ch;
                let target = remaining / units_left.max(1);
                let target = target.min(per_unit_budget * 2).max(64);
                let g = encode_granule(&xr[gr][ch], target);
                bits_used_total += g.total_bits;
                granule_data[gr].push(g);
            }
        }

        // Compose this frame's main-data bytes.
        let mut main_w = BitWriter::with_capacity(max_main_bits / 8 + 4);
        for gr in 0..2 {
            for ch in 0..n_ch {
                granule_data[gr][ch].emit_main_data(&mut main_w);
            }
        }
        main_w.align_to_byte();
        let main_data_bytes = main_w.into_bytes();

        // Append to queue and pop slot.
        self.main_data_queue.extend_from_slice(&main_data_bytes);
        let slot_take = main_data_slot_bytes.min(self.main_data_queue.len());
        let mut slot_payload: Vec<u8> = self.main_data_queue.drain(..slot_take).collect();
        if slot_payload.len() < main_data_slot_bytes {
            slot_payload.resize(main_data_slot_bytes, 0);
        }
        // Re-cap queue (should already hold).
        if self.main_data_queue.len() > MAX_LOOKBACK {
            let drop = self.main_data_queue.len() - MAX_LOOKBACK;
            self.main_data_queue.drain(..drop);
        }

        // ---- Compose frame bytes ----
        let mut frame_buf: Vec<u8> = Vec::with_capacity(frame_bytes);

        // Header (4 bytes).
        let mut hw = BitWriter::with_capacity(4);
        // Sync 11 bits + version(2) + layer(2) + protection(1)
        hw.write_u32(0x7FF, 11); // sync
        hw.write_u32(0b11, 2); // MPEG-1
        hw.write_u32(0b01, 2); // Layer III
        hw.write_u32(1, 1); // protection bit (1 = no CRC)
        hw.write_u32(self.br_index as u32, 4);
        hw.write_u32(self.sr_index as u32, 2);
        hw.write_u32(if padding { 1 } else { 0 }, 1);
        hw.write_u32(0, 1); // private
                            // Channel mode: mono = 0b11, dual-channel = 0b10.
        let mode_bits = if n_ch == 1 { 0b11u32 } else { 0b10 };
        hw.write_u32(mode_bits, 2);
        hw.write_u32(0, 2); // mode_extension (unused for non-joint stereo)
        hw.write_u32(0, 1); // copyright
        hw.write_u32(0, 1); // original
        hw.write_u32(0, 2); // emphasis
        let header = hw.into_bytes();
        debug_assert_eq!(header.len(), 4);
        frame_buf.extend_from_slice(&header);

        // Side info.
        let mut si_w = BitWriter::with_capacity(si_bytes);
        si_w.write_u32(main_data_begin as u32, 9);
        // private bits: 5 mono / 3 stereo
        si_w.write_u32(0, if n_ch == 1 { 5 } else { 3 });
        // scfsi: ch * 4 bits
        for _ in 0..n_ch {
            si_w.write_u32(0, 4); // never reuse
        }
        for gr in 0..2 {
            for ch in 0..n_ch {
                let g = &granule_data[gr][ch];
                si_w.write_u32(g.part2_3_length as u32, 12);
                si_w.write_u32(g.big_values as u32, 9);
                si_w.write_u32(g.global_gain as u32, 8);
                si_w.write_u32(0, 4); // scalefac_compress = 0 (slen1=0,slen2=0)
                si_w.write_u32(0, 1); // window_switching_flag = 0 (long blocks)
                                      // table_select[0..3] — all the same table so region splits
                                      // don't matter; whichever region a coefficient lands in,
                                      // the decoder reaches for the same Huffman table.
                si_w.write_u32(g.table_select as u32, 5);
                si_w.write_u32(g.table_select as u32, 5);
                si_w.write_u32(g.table_select as u32, 5);
                // region0_count=15, region1_count=7 → r0_end=bounds[16],
                // r1_end=bounds[24]→clamped to bounds[22]=576. Ensures
                // every coefficient in the big-values run gets decoded.
                si_w.write_u32(15, 4); // region0_count
                si_w.write_u32(7, 3); // region1_count
                si_w.write_u32(0, 1); // preflag
                si_w.write_u32(0, 1); // scalefac_scale
                si_w.write_u32(0, 1); // count1table_select = 0 (table A)
            }
        }
        si_w.align_to_byte();
        let side = si_w.into_bytes();
        debug_assert_eq!(side.len(), si_bytes);
        frame_buf.extend_from_slice(&side);

        // Main data slot.
        frame_buf.extend_from_slice(&slot_payload);

        debug_assert_eq!(frame_buf.len(), frame_bytes);

        let pts = (self.frame_index as i64) * 1152;
        let mut pkt = Packet::new(0, self.time_base, frame_buf);
        pkt.pts = Some(pts);
        pkt.dts = Some(pts);
        pkt.duration = Some(1152);
        pkt.flags.keyframe = true;
        self.frame_index += 1;
        Ok(pkt)
    }
}

impl Encoder for Mp3Encoder {
    fn codec_id(&self) -> &CodecId {
        &self.output_params.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.output_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        match frame {
            Frame::Audio(a) => self.ingest(a),
            _ => Err(Error::invalid("MP3 encoder: audio frames only")),
        }
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        self.pending_packets.pop_front().ok_or(Error::NeedMore)
    }

    fn flush(&mut self) -> Result<()> {
        if !self.eof {
            self.eof = true;
            self.flush_ready_frames(true)?;
        }
        Ok(())
    }
}

// ---------------- Per-granule encoding ----------------

#[derive(Clone)]
struct GranuleEncoded {
    global_gain: u8,
    big_values: u16,
    table_select: u8,
    /// All Huffman-encoded bytes (big_values + count1) staged as a
    /// list of (code, len) writes.
    main_writes: Vec<(u32, u32)>,
    /// Pre-summed bits for the writes.
    total_bits: usize,
    part2_3_length: u16,
}

impl GranuleEncoded {
    fn emit_main_data(&self, w: &mut BitWriter) {
        // We do NOT write any scalefactors because slen1 = slen2 = 0.
        for (code, len) in &self.main_writes {
            w.write_u32(*code, *len);
        }
    }
}

fn encode_granule(xr: &[f32; 576], bit_target: usize) -> GranuleEncoded {
    // Pick global_gain by binary search to fit `bit_target`. Higher
    // global_gain -> smaller is[] values -> fewer bits.
    //
    // We always end up with bits <= bit_target. If we can't fit even at
    // global_gain = 255 (max), we accept the overflow (decoder will read
    // junk but length is correct).
    let mut lo: i32 = 0;
    let mut hi: i32 = 255;
    let mut best: Option<GranuleEncoded> = None;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let g = quantize_and_encode(xr, mid as u8);
        if g.total_bits <= bit_target {
            // This fits — try a smaller gain (more precision).
            best = Some(g);
            hi = mid - 1;
        } else {
            // Doesn't fit — increase gain.
            lo = mid + 1;
        }
    }
    if let Some(g) = best {
        return g;
    }
    // Fallback: return the highest-gain (smallest-bits) result.
    quantize_and_encode(xr, 255)
}

fn quantize_and_encode(xr: &[f32; 576], global_gain: u8) -> GranuleEncoded {
    // Quantisation step: step_factor = 2^((global_gain - 210) / 4).
    // is[i] = nint( (|xr[i]|/step_factor)^(3/4) - 0.0946 )
    //       = nint( |xr[i]|^(3/4) * 2^(-(global_gain-210)*3/16) - 0.0946 )
    //
    // We compute per-sample directly to keep the code obvious. Cap |is|
    // at 8191 (the spec's hard ceiling — table 24 with linbits=13 +
    // value 15 reaches 32 + 8191 = 8223, but practical encoders cap
    // at 8191 to be safe). For our v1 we cap at 8191 and rely on the
    // outer global_gain bisection to keep us within table reach.
    let g = global_gain as i32;
    // Effective scaling exponent after the 3/4 power is (210 - g)*3/16.
    let exp = ((210 - g) as f32) * 3.0 / 16.0;
    let scale = (exp * std::f32::consts::LN_2).exp();
    let mut is_ = [0i32; 576];
    let mut max_abs = 0i32;
    for i in 0..576 {
        let a = xr[i].abs();
        let mag = a.powf(0.75) * scale;
        // Spec's quantizer subtracts 0.0946 then rounds. LAME uses
        // 0.4054 to bias toward over-quant; for simplicity we use 0.4054.
        let v = (mag + 0.4054).floor() as i32;
        let v = v.min(8191);
        let signed = if xr[i] < 0.0 { -v } else { v };
        is_[i] = signed;
        if v > max_abs {
            max_abs = v;
        }
    }

    // Find the trailing-zero region: scan from high index down for the
    // last non-zero coefficient.
    let mut last_nonzero = 0usize;
    for i in (0..576).rev() {
        if is_[i] != 0 {
            last_nonzero = i + 1;
            break;
        }
    }

    // Identify the count1 region. Walk from `last_nonzero` backward in
    // groups of 4, counting how many trailing groups consist solely of
    // 0/+-1 values.
    //
    // big_values_count must be even (it counts pairs * 2 in side info).
    let mut big_end = last_nonzero;
    // Round big_end up to a multiple of 2 since big_values runs in pairs.
    if big_end % 2 != 0 {
        big_end += 1;
        if big_end > 576 {
            big_end = 576;
        }
    }

    // Try sliding count1 region: take groups of 4 at the end where
    // every value is in {-1, 0, 1} for "free".
    let mut count1_start = big_end;
    while count1_start >= 4 {
        let g0 = is_[count1_start - 4];
        let g1 = is_[count1_start - 3];
        let g2 = is_[count1_start - 2];
        let g3 = is_[count1_start - 1];
        if g0.abs() <= 1 && g1.abs() <= 1 && g2.abs() <= 1 && g3.abs() <= 1 {
            count1_start -= 4;
        } else {
            break;
        }
    }
    // count1_start is now where count1 region starts (a multiple of 4
    // boundary at or before big_end). big_values ends at count1_start.
    let big_values_end = count1_start;
    let big_values_count = big_values_end as u16; // pairs * 2 = sample count

    // Pick a Huffman table that can encode all (x, y) pairs.
    let table_idx = choose_big_value_table(&is_, big_values_end);

    // Stage Huffman writes.
    let mut writes: Vec<(u32, u32)> =
        Vec::with_capacity(big_values_end / 2 + (576 - big_values_end) / 4);
    let mut total_bits: usize = 0;

    // Big-values pairs.
    for i in (0..big_values_end).step_by(2) {
        let x = is_[i];
        let y = is_.get(i + 1).copied().unwrap_or(0);
        let bits = emit_big_pair(table_idx, x, y, &mut writes);
        total_bits += bits;
    }

    // count1 region: groups of 4. Use table A.
    let count1_end = (last_nonzero + 3) & !3; // round up to multiple of 4
    let count1_end = count1_end.min(576);
    for i in (big_values_end..count1_end).step_by(4) {
        let v = is_[i];
        let w = is_.get(i + 1).copied().unwrap_or(0);
        let x = is_.get(i + 2).copied().unwrap_or(0);
        let y = is_.get(i + 3).copied().unwrap_or(0);
        let bits = emit_count1_quad(v, w, x, y, &mut writes);
        total_bits += bits;
    }

    // part2_3_length = scalefactor bits (0) + huffman bits (total_bits).
    let part2_3_length = total_bits as u16;

    GranuleEncoded {
        global_gain,
        big_values: (big_values_count / 2),
        table_select: table_idx,
        main_writes: writes,
        total_bits,
        part2_3_length,
    }
}

fn choose_big_value_table(is_: &[i32; 576], big_end: usize) -> u8 {
    // Find the maximum coefficient magnitude in the big-values region.
    let mut max_abs = 0i32;
    for i in 0..big_end {
        let a = is_[i].abs();
        if a > max_abs {
            max_abs = a;
        }
    }
    // Try candidates in priority order; pick the first whose effective
    // range covers max_abs. Range = 15 + (1 << linbits) - 1 when
    // linbits>0, else 15.
    for &t in BIG_VALUE_CANDIDATES {
        let bvt = &BIG_VALUE_TABLES[t as usize];
        if bvt.tab.is_empty() {
            continue;
        }
        // Find max (x,y) value in this table.
        let max_xy = bvt.tab.iter().map(|e| e.2.max(e.3)).max().unwrap_or(0) as i32;
        let reach = if bvt.linbits == 0 {
            max_xy
        } else {
            // Symbol 15 + linbits → up to 15 + (2^linbits - 1).
            max_xy + (1i32 << bvt.linbits) - 1
        };
        if reach >= max_abs {
            return t;
        }
    }
    23 // last-resort: TABLE_16 with linbits=13
}

fn emit_big_pair(table_idx: u8, x: i32, y: i32, writes: &mut Vec<(u32, u32)>) -> usize {
    let bvt = &BIG_VALUE_TABLES[table_idx as usize];
    if bvt.tab.is_empty() {
        // Table 0 — both must be zero.
        return 0;
    }

    let ax = x.unsigned_abs() as i32;
    let ay = y.unsigned_abs() as i32;

    // Determine the symbol to emit. With linbits, values >= 15 get
    // mapped to 15 plus extra bits (the residual).
    let (sym_x, lin_x) = if bvt.linbits > 0 && ax >= 15 {
        (15i32, ax - 15)
    } else {
        (ax, 0)
    };
    let (sym_y, lin_y) = if bvt.linbits > 0 && ay >= 15 {
        (15i32, ay - 15)
    } else {
        (ay, 0)
    };

    // Linear search for the matching (x, y) entry in the table.
    let mut found: Option<(u32, u8)> = None;
    for &(code, len, tx, ty) in bvt.tab {
        if tx as i32 == sym_x && ty as i32 == sym_y {
            found = Some((code, len));
            break;
        }
    }
    let (code, len) = match found {
        Some(v) => v,
        None => {
            // Shouldn't happen given our table choice; fall back to (0,0).
            (bvt.tab[0].0, bvt.tab[0].1)
        }
    };

    let mut bits = len as usize;
    writes.push((code, len as u32));

    if bvt.linbits > 0 && ax >= 15 {
        writes.push((lin_x as u32, bvt.linbits as u32));
        bits += bvt.linbits as usize;
    }
    if x != 0 {
        writes.push((if x < 0 { 1 } else { 0 }, 1));
        bits += 1;
    }
    if bvt.linbits > 0 && ay >= 15 {
        writes.push((lin_y as u32, bvt.linbits as u32));
        bits += bvt.linbits as usize;
    }
    if y != 0 {
        writes.push((if y < 0 { 1 } else { 0 }, 1));
        bits += 1;
    }
    bits
}

fn emit_count1_quad(v: i32, w: i32, x: i32, y: i32, writes: &mut Vec<(u32, u32)>) -> usize {
    // count1 table A: each value is 0 or +-1.
    let av = v.unsigned_abs().min(1) as u8;
    let aw = w.unsigned_abs().min(1) as u8;
    let ax = x.unsigned_abs().min(1) as u8;
    let ay = y.unsigned_abs().min(1) as u8;
    let mut bits = 0usize;
    for &(code, len, tv, tw, tx, ty) in COUNT1_A {
        if tv == av && tw == aw && tx == ax && ty == ay {
            writes.push((code, len as u32));
            bits += len as usize;
            break;
        }
    }
    if v != 0 {
        writes.push((if v < 0 { 1 } else { 0 }, 1));
        bits += 1;
    }
    if w != 0 {
        writes.push((if w < 0 { 1 } else { 0 }, 1));
        bits += 1;
    }
    if x != 0 {
        writes.push((if x < 0 { 1 } else { 0 }, 1));
        bits += 1;
    }
    if y != 0 {
        writes.push((if y < 0 { 1 } else { 0 }, 1));
        bits += 1;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_four_bytes() {
        let p = CodecParameters {
            codec_id: CodecId::new("mp3"),
            channels: Some(1),
            sample_rate: Some(44_100),
            sample_format: Some(SampleFormat::S16),
            bit_rate: Some(128_000),
            ..CodecParameters::audio(CodecId::new("mp3"))
        };
        let enc = make_encoder(&p).unwrap();
        assert_eq!(enc.codec_id().as_str(), "mp3");
    }

    #[test]
    fn quantize_silence_gives_zero_bits() {
        let xr = [0.0f32; 576];
        let g = quantize_and_encode(&xr, 100);
        assert_eq!(g.total_bits, 0);
        assert_eq!(g.big_values, 0);
    }
}
