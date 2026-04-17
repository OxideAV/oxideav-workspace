//! End-to-end CABAC I-slice decode test.
//!
//! Hand-crafts the smallest legal CABAC IDR picture — a single 16×16
//! macroblock encoded as `I_16x16` DC with `cbp_luma=0`, `cbp_chroma=0`,
//! `mb_qp_delta=0`, `intra_chroma_pred_mode=DC` and a `coded_block_flag=0`
//! luma DC block — and verifies it decodes to an all-grey (Y=128, Cb/Cr=128)
//! YUV420P frame.
//!
//! The test is self-contained: it builds SPS, PPS and slice NAL units from
//! bit strings, emits the CABAC payload via a spec-faithful miniature
//! encoder (`CabacEncoder`), wraps everything in Annex B start codes and
//! feeds the bytes to `H264Decoder`.

use oxideav_codec::Decoder;
use oxideav_core::{CodecId, Frame, Packet, TimeBase};
use oxideav_h264::decoder::H264Decoder;

// ---------------------------------------------------------------------------
// BitWriter — MSB-first.
// ---------------------------------------------------------------------------

struct BitWriter {
    bits: Vec<u8>,
}

impl BitWriter {
    fn new() -> Self {
        Self { bits: Vec::new() }
    }
    fn write_bit(&mut self, b: u8) {
        self.bits.push(b & 1);
    }
    fn write_bits(&mut self, v: u64, n: u32) {
        for i in (0..n).rev() {
            self.bits.push(((v >> i) & 1) as u8);
        }
    }
    fn write_u(&mut self, v: u64, n: u32) {
        self.write_bits(v, n);
    }
    fn write_ue(&mut self, v: u32) {
        // Exp-Golomb unsigned: write `leading_zeros` zeros, a 1, then
        // `leading_zeros` bits equal to `v + 1 - (1 << leading_zeros)`.
        let x = (v + 1) as u64;
        let bits = 64 - x.leading_zeros();
        let zeros = bits - 1;
        for _ in 0..zeros {
            self.write_bit(0);
        }
        self.write_bits(x, bits);
    }
    fn write_se(&mut self, v: i32) {
        let mapped = if v <= 0 {
            (-v) as u32 * 2
        } else {
            v as u32 * 2 - 1
        };
        self.write_ue(mapped);
    }
    fn align_to_byte_with_stop_bit(&mut self) {
        // rbsp_trailing_bits: a 1 then zero pad.
        self.write_bit(1);
        while self.bits.len() % 8 != 0 {
            self.write_bit(0);
        }
    }
    fn into_bytes(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bits.len() / 8);
        for chunk in self.bits.chunks(8) {
            let mut b = 0u8;
            for (i, &bit) in chunk.iter().enumerate() {
                b |= bit << (7 - i);
            }
            out.push(b);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// CabacEncoder — mirrors H.264 §9.3.4 (encoding process).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
struct CabacCtx {
    p_state_idx: u8,
    val_mps: u8,
}

impl CabacCtx {
    fn init(m: i32, n: i32, qpy: i32) -> Self {
        let qpy = qpy.clamp(0, 51);
        let pre = (((m * qpy) >> 4) + n).clamp(1, 126);
        if pre <= 63 {
            Self {
                p_state_idx: (63 - pre) as u8,
                val_mps: 0,
            }
        } else {
            Self {
                p_state_idx: (pre - 64) as u8,
                val_mps: 1,
            }
        }
    }
}

#[rustfmt::skip]
const RANGE_TAB_LPS: [[u16; 4]; 64] = [
    [128, 176, 208, 240], [128, 167, 197, 227], [128, 158, 187, 216], [123, 150, 178, 205],
    [116, 142, 169, 195], [111, 135, 160, 185], [105, 128, 152, 175], [100, 123, 144, 166],
    [ 95, 116, 137, 158], [ 90, 110, 130, 150], [ 85, 104, 123, 142], [ 81,  99, 117, 135],
    [ 77,  94, 111, 128], [ 73,  89, 105, 122], [ 69,  85, 100, 116], [ 66,  80,  95, 110],
    [ 62,  76,  90, 104], [ 59,  72,  86,  99], [ 56,  69,  81,  94], [ 53,  65,  77,  89],
    [ 51,  62,  73,  85], [ 48,  59,  69,  80], [ 46,  56,  66,  76], [ 43,  53,  63,  72],
    [ 41,  50,  59,  69], [ 39,  48,  56,  65], [ 37,  45,  54,  62], [ 35,  43,  51,  59],
    [ 33,  41,  48,  56], [ 32,  39,  46,  53], [ 30,  37,  43,  50], [ 29,  35,  41,  48],
    [ 27,  33,  39,  45], [ 26,  31,  37,  43], [ 24,  30,  35,  41], [ 23,  28,  33,  39],
    [ 22,  27,  32,  37], [ 21,  26,  30,  35], [ 20,  24,  29,  33], [ 19,  23,  27,  31],
    [ 18,  22,  26,  30], [ 17,  21,  25,  28], [ 16,  20,  23,  27], [ 15,  19,  22,  25],
    [ 14,  18,  21,  24], [ 14,  17,  20,  23], [ 13,  16,  19,  22], [ 12,  15,  18,  21],
    [ 12,  14,  17,  20], [ 11,  14,  16,  19], [ 11,  13,  15,  18], [ 10,  12,  15,  17],
    [ 10,  12,  14,  16], [  9,  11,  13,  15], [  9,  11,  12,  14], [  8,  10,  12,  14],
    [  8,   9,  11,  13], [  7,   9,  11,  12], [  7,   9,  10,  12], [  7,   8,  10,  11],
    [  6,   8,   9,  11], [  6,   7,   9,  10], [  6,   7,   8,   9], [  2,   2,   2,   2],
];

#[rustfmt::skip]
const TRANS_IDX_LPS: [u8; 64] = [
     0,  0,  1,  2,  2,  4,  4,  5,  6,  7,  8,  9,  9, 11, 11, 12,
    13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21, 21, 22, 22, 23, 24,
    24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33,
    33, 33, 34, 34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];

#[rustfmt::skip]
const TRANS_IDX_MPS: [u8; 64] = [
     1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15, 16,
    17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32,
    33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];

struct CabacEncoder {
    low: u32,
    range: u32,
    outstanding: u32,
    first: bool,
    out_bits: Vec<u8>,
}

impl CabacEncoder {
    fn new() -> Self {
        Self {
            low: 0,
            range: 0x01FE,
            outstanding: 0,
            first: true,
            out_bits: Vec::new(),
        }
    }

    fn put_bit(&mut self, bit: u8) {
        if self.first {
            self.first = false;
        } else {
            self.out_bits.push(bit);
        }
        for _ in 0..self.outstanding {
            self.out_bits.push(1 - bit);
        }
        self.outstanding = 0;
    }

    fn renorm(&mut self) {
        while self.range < 0x0100 {
            if self.low < 0x0100 {
                self.put_bit(0);
            } else if self.low >= 0x0200 {
                self.low -= 0x0200;
                self.put_bit(1);
            } else {
                self.low -= 0x0100;
                self.outstanding += 1;
            }
            self.low <<= 1;
            self.range <<= 1;
        }
    }

    fn encode_bin(&mut self, ctx: &mut CabacCtx, bin: u8) {
        let rlps_idx = ((self.range >> 6) & 3) as usize;
        let p = ctx.p_state_idx as usize;
        let rlps = RANGE_TAB_LPS[p][rlps_idx] as u32;
        self.range -= rlps;
        if bin != ctx.val_mps {
            self.low += self.range;
            self.range = rlps;
            if ctx.p_state_idx == 0 {
                ctx.val_mps = 1 - ctx.val_mps;
            }
            ctx.p_state_idx = TRANS_IDX_LPS[p];
        } else {
            ctx.p_state_idx = TRANS_IDX_MPS[p];
        }
        self.renorm();
    }

    /// §9.3.4.5 — encode_terminate. Pass `bin = 1` for end-of-slice. The
    /// caller is expected to invoke [`finish_flush`](Self::finish_flush)
    /// immediately after to emit §9.3.4.6's trailing bits.
    fn encode_terminate(&mut self, bin: u8) {
        self.range -= 2;
        if bin == 1 {
            self.low += self.range;
            self.range = 2;
            // No RenormE here — see §9.3.4.5. Flush drives it.
        } else {
            self.renorm();
        }
    }

    /// §9.3.4.6 — encoding flush at end-of-slice.
    fn finish_flush(&mut self) {
        self.range = 2;
        self.renorm();
        let hi = ((self.low >> 9) & 1) as u8;
        self.put_bit(hi);
        // Spec: WriteBits( ((codILow >> 7) & 3) | 1, 2 ). The "| 1" sets
        // the lowest of those two bits to 1, doubling as the rbsp stop bit.
        let tail = (((self.low >> 7) & 3) | 1) as u8;
        self.out_bits.push((tail >> 1) & 1);
        self.out_bits.push(tail & 1);
    }

    fn finish_bytes_aligned(mut self) -> Vec<u8> {
        // `finish_flush` already injected the rbsp stop bit (the `| 1` trick
        // in §9.3.4.6 turns the low bit of WriteBits into a 1). Here we just
        // need to pad to byte alignment.
        while self.out_bits.len() % 8 != 0 {
            self.out_bits.push(0);
        }
        // Engine reads 9 bits at init + up to 8 renorm bits per decoded bin.
        // Pad generously so the decoder never runs off the end while still
        // producing deterministic behaviour (all-zero tail bits translate to
        // "MPS path, no renorm drama" for any unrelated context state).
        for _ in 0..64 {
            self.out_bits.push(0);
        }
        let mut out = Vec::with_capacity(self.out_bits.len() / 8);
        for chunk in self.out_bits.chunks(8) {
            let mut b = 0u8;
            for (i, &bit) in chunk.iter().enumerate() {
                b |= bit << (7 - i);
            }
            out.push(b);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// SPS / PPS / slice construction.
// ---------------------------------------------------------------------------

fn build_sps_rbsp() -> Vec<u8> {
    // Main profile (77) supports CABAC; Baseline does not.
    let profile_idc: u8 = 77;
    let constraint_flags: u8 = 0;
    let level_idc: u8 = 30; // Level 3.0 — plenty for a 16×16 picture.

    let mut bw = BitWriter::new();
    bw.write_ue(0); // seq_parameter_set_id
                    // Main profile (77) is NOT one of the "high profile" families listed in
                    // sps.rs — the parser won't read chroma_format_idc / bit_depth / etc.
    bw.write_ue(0); // log2_max_frame_num_minus4
    bw.write_ue(0); // pic_order_cnt_type
    bw.write_ue(0); // log2_max_pic_order_cnt_lsb_minus4
    bw.write_ue(1); // max_num_ref_frames
    bw.write_bit(0); // gaps_in_frame_num_value_allowed_flag
    bw.write_ue(0); // pic_width_in_mbs_minus1 = 0 → 16 px
    bw.write_ue(0); // pic_height_in_map_units_minus1 = 0 → 16 px
    bw.write_bit(1); // frame_mbs_only_flag
    bw.write_bit(0); // direct_8x8_inference_flag
    bw.write_bit(0); // frame_cropping_flag
    bw.write_bit(0); // vui_parameters_present_flag
    bw.align_to_byte_with_stop_bit();
    let body = bw.into_bytes();

    let mut out = Vec::with_capacity(3 + body.len());
    out.push(profile_idc);
    out.push(constraint_flags);
    out.push(level_idc);
    out.extend_from_slice(&body);
    out
}

fn build_pps_rbsp() -> Vec<u8> {
    let mut bw = BitWriter::new();
    bw.write_ue(0); // pic_parameter_set_id
    bw.write_ue(0); // seq_parameter_set_id
    bw.write_bit(1); // entropy_coding_mode_flag = 1 → CABAC
    bw.write_bit(0); // bottom_field_pic_order_in_frame_present_flag
    bw.write_ue(0); // num_slice_groups_minus1
    bw.write_ue(0); // num_ref_idx_l0_default_active_minus1
    bw.write_ue(0); // num_ref_idx_l1_default_active_minus1
    bw.write_bit(0); // weighted_pred_flag
    bw.write_u(0, 2); // weighted_bipred_idc
    bw.write_se(0); // pic_init_qp_minus26 → QP = 26
    bw.write_se(0); // pic_init_qs_minus26
    bw.write_se(0); // chroma_qp_index_offset
    bw.write_bit(1); // deblocking_filter_control_present_flag
    bw.write_bit(0); // constrained_intra_pred_flag
    bw.write_bit(0); // redundant_pic_cnt_present_flag
    bw.align_to_byte_with_stop_bit();
    bw.into_bytes()
}

fn build_idr_slice_rbsp() -> Vec<u8> {
    // ----- Slice header -----
    let mut bw = BitWriter::new();
    bw.write_ue(0); // first_mb_in_slice
    bw.write_ue(7); // slice_type = 7 → I (and signals "only I in this frame")
    bw.write_ue(0); // pic_parameter_set_id
                    // frame_num_bits = log2_max_frame_num_minus4 + 4 = 4
    bw.write_u(0, 4); // frame_num
                      // field_pic_flag skipped (frame_mbs_only = 1)
    bw.write_ue(0); // idr_pic_id
                    // pic_order_cnt_type == 0 → read pic_order_cnt_lsb, 4 bits
    bw.write_u(0, 4); // pic_order_cnt_lsb
                      // No delta_pic_order_cnt_bottom (bottom_field_pic_order = 0)
                      // No ref_pic_list_modification (I slice)
                      // No pred weight table, no dec ref pic marking for non-ref? Actually
                      // nal_ref_idc != 0 for IDR, so we DO emit dec_ref_pic_marking.
                      // For IDR: no_output_of_prior_pics_flag + long_term_reference_flag.
    bw.write_bit(0); // no_output_of_prior_pics_flag
    bw.write_bit(0); // long_term_reference_flag
                     // cabac_init_idc — SKIPPED for I slices (per slice.rs parser).
    bw.write_se(0); // slice_qp_delta
                    // slice_alpha_c0_offset_div2 / slice_beta_offset_div2 — only if
                    // deblocking_filter_control_present_flag && disable_deblocking_filter_idc != 1
    bw.write_ue(1); // disable_deblocking_filter_idc = 1 → skip alpha/beta offsets
                    // (no alpha/beta offsets since idc == 1)

    // ----- slice_data(): cabac_alignment_one_bit + macroblock_layer -----
    // Pad bit-stream up to byte boundary with 1s (cabac_alignment_one_bit).
    // Spec: before the first CABAC engine init, bits up to the next byte
    // boundary are cabac_alignment_one_bit (each = 1). Our CabacDecoder::new
    // aligns UP to the next byte boundary but does not validate the pad bits
    // — so any padding would work; we follow the spec.
    while bw.bits.len() % 8 != 0 {
        bw.write_bit(1);
    }

    let header_bytes = bw.into_bytes();

    // ----- CABAC macroblock encode -----
    // Slice QPY = pic_init_qp_minus26 + 26 + slice_qp_delta = 26.
    let slice_qpy = 26;

    // Build only the handful of contexts we actually touch. The values are
    // identical to `cabac::tables::init_slice_contexts(0, true, 26)` for
    // those ctxIdx entries we cite below.
    // I-slice mb_type live at ctxIdxOffset 3..=10. For our fixture we need:
    //  - mb_type bin 0   ctx_idx = 3 (ctx_idx_inc=0, no neighbours)
    //  - mb_type bin 2/3/5/6 ctx_idx = 6, 7, 9, 9  (offsets 3 + 3/4/6/6)
    //  mb_type encoding for I_16x16 DC cbp_luma=0 cbp_chroma=0:
    //    b0 = 1 (not I_NxN)
    //    b1 = 0 (terminate bin — not I_PCM)
    //    b2 = 0 (cbp_luma)
    //    b3 = 0 (cbp_chroma bin 0 — 0 short-circuits)
    //    b5 = 0 (intra_pred bin 0)
    //    b6 = 0 (intra_pred bin 1)
    //  → mb_type = 1 + (0) + 4*(0) + 12*(0) = 1
    //  (Per binarize::decode_mb_type_i + decode_i_slice_mb_type Table:
    //   mb_type=1 → I_16x16 pred_mode=DC, cbp_luma=0, cbp_chroma=0.)
    //
    // mb_qp_delta bin 0 ctx_idx = 60 (ctx_idx_inc=0, prev_delta=0) — emit 0.
    // intra_chroma_pred_mode bin 0 ctx_idx = 64 (ctx_idx_inc=0) — emit 0.
    // Luma16x16 DC block coded_block_flag: ctxIdxOffset 85 + cat*4 + inc
    //   ctxBlockCat Luma16x16Dc = 0, inc = 0 → ctxIdx = 85.
    //   We emit 0 → block is skipped, no further residual bins.
    // end_of_slice_flag via encode_terminate(1).

    let ctx = |idx: usize| -> CabacCtx {
        // Pull (m,n) from the inline-literal rows listed below. These match
        // cabac::tables::INIT_MN_DATA's column 0 (I/SI slice) at that row.
        let mn = match idx {
            3 => (20, -15),
            6 => (-28, 127),
            7 => (-23, 104),
            9 => (-1, 54),
            60 => (0, 41),
            61 => (0, 63),
            62 => (0, 63),
            63 => (0, 63),
            64 => (13, 41),
            65 => (3, 62),
            66 => (0, 58),
            67 => (0, 63),
            85 => (-8, 71),
            _ => panic!("unexpected ctxIdx {idx}"),
        };
        CabacCtx::init(mn.0, mn.1, slice_qpy)
    };

    // Build the single "slice ctx table" the same way init_slice_contexts
    // does for the entries we actually touch. IMPORTANT: contexts must be
    // shared between bins that reuse the same ctxIdx (e.g. mb_type b5 and b6
    // both use the saturated ctx index 9), otherwise encoder-decoder state
    // drift produces garbage bitstreams.
    let mut slice_ctx: std::collections::HashMap<usize, CabacCtx> =
        std::collections::HashMap::new();
    let get_ctx = |idx: usize,
                   slice_ctx: &mut std::collections::HashMap<usize, CabacCtx>|
     -> CabacCtx { *slice_ctx.entry(idx).or_insert_with(|| ctx(idx)) };

    let mut enc = CabacEncoder::new();

    // Tiny wrapper so we can transparently fetch-update-write back without
    // the borrow checker complaining.
    macro_rules! enc_bin {
        ($idx:expr, $bin:expr) => {{
            let mut c = get_ctx($idx, &mut slice_ctx);
            enc.encode_bin(&mut c, $bin);
            slice_ctx.insert($idx, c);
        }};
    }

    // mb_type bins for mb_type = 1 (I_16x16 DC, cbp_luma=0, cbp_chroma=0).
    enc_bin!(3, 1); // bin 0 — not I_NxN
    enc.encode_terminate(0); // bin 1 — not I_PCM (terminate, no ctx)
    enc_bin!(6, 0); // cbp_luma
    enc_bin!(7, 0); // cbp_chroma bin 0 (0 → short-circuit)
    enc_bin!(9, 0); // intra_pred bin 0
    enc_bin!(9, 0); // intra_pred bin 1 — REUSES ctxs[9]

    // intra_chroma_pred_mode = 0 → single bin 0 at ctx 64.
    enc_bin!(64, 0);

    // mb_qp_delta = 0 → single bin 0 at ctx 60.
    enc_bin!(60, 0);

    // Luma DC residual block — coded_block_flag = 0, no further residual.
    enc_bin!(85, 0);

    // end_of_slice_flag = 1 (terminate) + encoding flush (§9.3.4.6).
    enc.encode_terminate(1);
    enc.finish_flush();

    let cabac_bytes = enc.finish_bytes_aligned();

    let mut out = header_bytes;
    out.extend_from_slice(&cabac_bytes);
    out
}

// ---------------------------------------------------------------------------
// NALU / Annex B packaging.
// ---------------------------------------------------------------------------

fn wrap_nalu(nal_unit_type: u8, nal_ref_idc: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + rbsp.len() + 4);
    let header = ((nal_ref_idc & 0x03) << 5) | (nal_unit_type & 0x1F);
    out.push(header);
    // Insert emulation-prevention bytes on 00 00 00..=03 sequences per §7.4.1.1.
    let mut zero_run = 0u8;
    for &b in rbsp {
        if zero_run >= 2 && b <= 0x03 {
            out.push(0x03);
            zero_run = 0;
        }
        out.push(b);
        if b == 0 {
            zero_run += 1;
        } else {
            zero_run = 0;
        }
    }
    out
}

fn build_annex_b_packet() -> Vec<u8> {
    let sps_rbsp = build_sps_rbsp();
    let pps_rbsp = build_pps_rbsp();
    let idr_rbsp = build_idr_slice_rbsp();

    let sps = wrap_nalu(7, 3, &sps_rbsp);
    let pps = wrap_nalu(8, 3, &pps_rbsp);
    let idr = wrap_nalu(5, 3, &idr_rbsp);

    let mut pkt = Vec::new();
    pkt.extend_from_slice(&[0, 0, 0, 1]);
    pkt.extend_from_slice(&sps);
    pkt.extend_from_slice(&[0, 0, 0, 1]);
    pkt.extend_from_slice(&pps);
    pkt.extend_from_slice(&[0, 0, 0, 1]);
    pkt.extend_from_slice(&idr);
    pkt
}

// ---------------------------------------------------------------------------
// Actual test.
// ---------------------------------------------------------------------------

#[test]
fn cabac_encoder_roundtrip_terminate() {
    // Sanity-check the in-test CabacEncoder / decoder_terminate pairing.
    // Encode three regular bins then an end-of-slice terminate; decode with
    // the real CabacDecoder and verify the termination fires at the right
    // spot.
    use oxideav_h264::cabac::context::CabacContext;
    use oxideav_h264::cabac::engine::CabacDecoder;

    let mut enc = CabacEncoder::new();
    let mut ctx = CabacCtx::init(20, -15, 26);
    enc.encode_bin(&mut ctx, 1);
    enc.encode_bin(&mut ctx, 0);
    enc.encode_bin(&mut ctx, 1);
    enc.encode_terminate(1);
    enc.finish_flush();
    let bytes = enc.finish_bytes_aligned();

    let mut dec = CabacDecoder::new(&bytes, 0).expect("decoder init");
    let mut dctx = CabacContext {
        p_state_idx: ctx_init(20, -15, 26).0,
        val_mps: ctx_init(20, -15, 26).1,
    };
    let b0 = dec.decode_bin(&mut dctx).unwrap();
    let b1 = dec.decode_bin(&mut dctx).unwrap();
    let b2 = dec.decode_bin(&mut dctx).unwrap();
    assert_eq!((b0, b1, b2), (1, 0, 1), "regular bin roundtrip");
    let end = dec.decode_terminate().unwrap();
    assert_eq!(end, 1, "decode_terminate must report end-of-slice");
}

fn ctx_init(m: i32, n: i32, qpy: i32) -> (u8, u8) {
    let c = CabacCtx::init(m, n, qpy);
    (c.p_state_idx, c.val_mps)
}

#[test]
fn cabac_iframe_single_mb_all_grey() {
    let pkt_data = build_annex_b_packet();

    let mut dec = H264Decoder::new(CodecId::new("h264"));
    let pkt = Packet::new(0, TimeBase::new(1, 90_000), pkt_data)
        .with_pts(0)
        .with_keyframe(true);
    dec.send_packet(&pkt).expect("send_packet");
    let frame = match dec.receive_frame().expect("receive_frame") {
        Frame::Video(f) => f,
        other => panic!(
            "expected video frame, got {:?}",
            std::mem::discriminant(&other)
        ),
    };

    assert_eq!(frame.width, 16);
    assert_eq!(frame.height, 16);
    assert_eq!(frame.planes.len(), 3);

    // Luma plane: 16×16, all samples should equal 128 (DC prediction for
    // no-neighbour MB + zero residual).
    for (i, &v) in frame.planes[0].data.iter().enumerate() {
        assert_eq!(v, 128, "Y[{i}] expected 128, got {v}");
    }
    // Chroma planes: 8×8 each, DC prediction with no neighbours = 128.
    for (i, &v) in frame.planes[1].data.iter().enumerate() {
        assert_eq!(v, 128, "Cb[{i}] expected 128, got {v}");
    }
    for (i, &v) in frame.planes[2].data.iter().enumerate() {
        assert_eq!(v, 128, "Cr[{i}] expected 128, got {v}");
    }
}
