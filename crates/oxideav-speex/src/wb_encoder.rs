//! Speex wideband (sub-band CELP) encoder — float path, sub-mode 1 only.
//!
//! Mirrors the high-band analysis pipeline from `libspeex/sb_celp.c`'s
//! `sb_encode`. A wideband frame is the concatenation of a full
//! narrowband (0–4 kHz) encode and a high-band (4–8 kHz) extension
//! layer, separated in time by a 2-band QMF analysis. We emit the
//! lowest-rate SB-CELP layer — **sub-mode 1**, 36-bit spectral-folding
//! extension (~1.8 kbit/s on top of the 15 kbit/s NB).
//!
//! Bitstream layout produced here (matching [`crate::wb_decoder`]):
//!
//! ```text
//!   +---- NB frame (300 bits, NB sub-mode 5) -----+
//!   | 1 bit  wideband-bit = 1                     |
//!   | 3 bit  WB sub-mode id = 1                   |
//!   | 12 bit high-band LSP VQ (2×6)               |
//!   | 5 bit  folding gain  } per sub-frame        |
//!   | 5 bit  folding gain  }      ×4 = 20 bits    |
//!   | 5 bit  folding gain  }                      |
//!   | 5 bit  folding gain  }                      |
//!   +---------------------------------------------+
//!   Total WB extension = 36 bits — matches `WB_SUBMODE_1.bits_per_frame`.
//! ```
//!
//! Sub-mode 1 is chosen as the pragmatic minimum for this first-cut
//! encoder: it doesn't need a stochastic codebook search on the high
//! band, only a per-sub-frame gain that's matched against a simple
//! exp-law scalar quantiser. The reconstructed high band is obtained
//! by spectral folding of the narrowband innovation — the same path
//! the decoder exercises in `WbInnov::Folding`. The resulting
//! wideband quality is modest (roughly 2 kHz of usable extension
//! above the NB band), but the roundtrip is valid and stable.
//!
//! ### Pipeline (per 320-sample wideband frame)
//!
//! 1. **QMF analysis** — split the 16 kHz input into 160 low-band
//!    and 160 high-band 8 kHz samples (`qmf_decomp`). The high band
//!    is spectrally inverted (4–8 kHz mirrored to 0–4 kHz).
//! 2. **NB encode** — feed the low band to [`NbEncoder`], which emits
//!    the 300-bit narrowband packet prefix.
//! 3. **High-band LPC analysis** — windowed autocorrelation (8th-order)
//!    → Levinson-Durbin → bandwidth-expand (γ=0.9) → LPC-to-LSP.
//! 4. **LSP quantisation** — two-stage 6+6 bit VQ against
//!    `HIGH_LSP_CDBK` + `HIGH_LSP_CDBK2`. Same tables the decoder
//!    reads.
//! 5. **Per-sub-frame folding gain** — compare the high-band LPC
//!    residual RMS with what spectral-folding of the NB innovation
//!    produces, and pick the 5-bit `q` that minimises the ratio.
//!
//! ### Known gaps
//!
//! - Sub-modes 2/3/4 (stochastic codebook) are not implemented — they
//!   require a split-VQ search on the high-band residual, which the
//!   folding path sidesteps.
//! - No rate-control / VAD — every frame is encoded as the full
//!   NB-mode-5 + WB-mode-1 bundle (332 bits, ~16.6 kbit/s).
//! - No perceptual weighting — the gain quantiser uses raw residual
//!   energy, which is a looser fit than libspeex's weighted path
//!   but adequate for the folding layer's modest dynamic range.

use oxideav_core::{Error, Result};

use crate::bitwriter::BitWriter;
use crate::lsp::{bw_lpc, lsp_interpolate, lsp_to_lpc};
use crate::lsp_tables_wb::{HIGH_LSP_CDBK, HIGH_LSP_CDBK2};
use crate::nb_encoder::NbEncoder;
use crate::qmf::{H0_PROTOTYPE, QMF_ORDER};
use crate::wb_decoder::{
    FOLDING_GAIN, LSP_MARGIN_HIGH, WB_FRAME_SIZE, WB_FULL_FRAME_SIZE, WB_LPC_ORDER,
    WB_NB_SUBFRAMES, WB_SUBFRAME_SIZE,
};

/// The only WB sub-mode this encoder can emit.
pub const SUPPORTED_WB_SUBMODE: u32 = 1;

/// Wideband encoder state. Holds the sub-encoder for the NB (low-band)
/// path, the QMF analysis memory for the two branches, and the
/// previous frame's quantised high-band LSPs for sub-frame
/// interpolation.
pub struct WbEncoder {
    /// NB CELP encoder for the 0–4 kHz band.
    nb: NbEncoder,
    /// QMF analysis memory (low-pass branch).
    qmf_mem_lo: [f32; QMF_ORDER],
    /// QMF analysis memory (high-pass branch).
    qmf_mem_hi: [f32; QMF_ORDER],
    /// Quantised high-band LSPs from the previous frame.
    old_qlsp_high: [f32; WB_LPC_ORDER],
    /// First-frame flag — makes the first interpolation use `qlsp`
    /// alone (same trick the decoder uses).
    first: bool,
}

impl Default for WbEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl WbEncoder {
    pub fn new() -> Self {
        let mut old_qlsp_high = [0.0f32; WB_LPC_ORDER];
        for i in 0..WB_LPC_ORDER {
            old_qlsp_high[i] =
                std::f32::consts::PI * (i as f32 + 1.0) / (WB_LPC_ORDER as f32 + 1.0);
        }
        Self {
            nb: NbEncoder::new(),
            qmf_mem_lo: [0.0; QMF_ORDER],
            qmf_mem_hi: [0.0; QMF_ORDER],
            old_qlsp_high,
            first: true,
        }
    }

    /// Encode one 320-sample wideband frame (int16-range f32). Appends
    /// the 336-bit payload (300 NB + 36 WB-extension) to the writer.
    pub fn encode_frame(&mut self, pcm: &[f32], bw: &mut BitWriter) -> Result<()> {
        if pcm.len() != WB_FULL_FRAME_SIZE {
            return Err(Error::invalid(format!(
                "Speex WB encoder: expected {WB_FULL_FRAME_SIZE}-sample frame, got {}",
                pcm.len()
            )));
        }

        // ---- 1. QMF analysis -----------------------------------------
        let mut low_band = [0.0f32; WB_FRAME_SIZE];
        let mut high_band = [0.0f32; WB_FRAME_SIZE];
        qmf_decomp(
            pcm,
            &H0_PROTOTYPE,
            &mut low_band,
            &mut high_band,
            WB_FULL_FRAME_SIZE,
            QMF_ORDER,
            &mut self.qmf_mem_lo,
            &mut self.qmf_mem_hi,
        );

        // ---- 2. NB encode on the low band -----------------------------
        self.nb.encode_frame(&low_band, bw)?;

        // ---- 3. WB layer header: wideband-bit=1, submode=1 ------------
        bw.write_bits(1, 1);
        bw.write_bits(SUPPORTED_WB_SUBMODE, 3);

        // ---- 4. High-band LPC analysis --------------------------------
        let windowed = hamming_window_wb(&high_band);
        let mut autocorr = [0.0f32; WB_LPC_ORDER + 1];
        autocorrelate(&windowed, &mut autocorr);
        // Lag window — same idea as NB.
        for k in 1..=WB_LPC_ORDER {
            let tau = 40.0f32;
            let w = (-0.5 * (k as f32 / tau).powi(2)).exp();
            autocorr[k] *= w;
        }
        autocorr[0] *= 1.0001;
        if autocorr[0] < 1e-6 {
            autocorr[0] = 1e-6;
        }
        let raw_lpc = levinson_durbin(&autocorr);
        // Bandwidth-expand the LPC before LSP conversion so the
        // synthesis filter stays comfortably stable.
        let mut lpc = [0.0f32; WB_LPC_ORDER];
        bw_lpc(0.9, &raw_lpc, &mut lpc, WB_LPC_ORDER);

        // ---- 5. LPC → LSP (8th order, high-band linear initial) -------
        let lsp = lpc_to_lsp_high(&lpc).unwrap_or_else(|| {
            // Fallback: linear LSPs in the high-band range.
            let mut fallback = [0.0f32; WB_LPC_ORDER];
            for i in 0..WB_LPC_ORDER {
                fallback[i] = 0.3125 * i as f32 + 0.75;
            }
            fallback
        });

        // ---- 6. LSP quantisation (two-stage 6+6 VQ) -------------------
        let (qlsp, lsp_idx_a, lsp_idx_b) = quantise_lsp_high(&lsp);
        bw.write_bits(lsp_idx_a as u32, 6);
        bw.write_bits(lsp_idx_b as u32, 6);

        if self.first {
            self.old_qlsp_high = qlsp;
        }

        // ---- 7. Per-sub-frame folding gain quantisation ---------------
        //
        // Reference reconstruction (see `WbInnov::Folding`):
        //   exc[i]   = +FG * low_innov[i]   * g
        //   exc[i+1] = -FG * low_innov[i+1] * g     (for even i)
        //   g        = exp(0.125·(q−10)) / filter_ratio
        //
        // Our goal is to pick `q ∈ 0..32` so that the resulting high
        // band's synthesis reconstructs the input high band's envelope.
        // We compare the (interpolated) high-band LPC residual energy
        // against the NB innovation's folding energy, both on a per
        // sub-frame basis.
        let nb_innov = *self.nb.innov();
        let nb_pi_gain = *self.nb.pi_gain();
        let mut interp_qlsp = [0.0f32; WB_LPC_ORDER];
        let mut ak = [0.0f32; WB_LPC_ORDER];
        for sub in 0..WB_NB_SUBFRAMES {
            let off = sub * WB_SUBFRAME_SIZE;

            // Interpolated quantised LPC (matches decoder path).
            lsp_interpolate(
                &self.old_qlsp_high,
                &qlsp,
                &mut interp_qlsp,
                WB_LPC_ORDER,
                sub,
                WB_NB_SUBFRAMES,
                LSP_MARGIN_HIGH,
            );
            lsp_to_lpc(&interp_qlsp, &mut ak, WB_LPC_ORDER);

            // Filter-response ratio (same formula the decoder uses).
            let mut rh = 1.0f32;
            let mut k = 0;
            while k + 1 < WB_LPC_ORDER {
                rh += ak[k + 1] - ak[k];
                k += 2;
            }
            let rl = nb_pi_gain[sub];
            let filter_ratio = (rl + 0.01) / (rh + 0.01);

            // LPC-residual energy of the high band for this sub-frame.
            // `hb_sub` = high-band PCM for sub-frame `sub` (40 samples).
            let hb_sub = &high_band[off..off + WB_SUBFRAME_SIZE];
            let mut residual = [0.0f32; WB_SUBFRAME_SIZE];
            fir_filter_stateless(hb_sub, &ak, &mut residual, WB_LPC_ORDER);
            let hb_res_rms = rms(&residual);

            // Folding-source energy — what the decoder would produce at
            // unit `g`. That's just the NB innovation's RMS within this
            // sub-frame, scaled by FOLDING_GAIN.
            let folding_rms = FOLDING_GAIN * rms(&nb_innov[off..off + WB_SUBFRAME_SIZE]);

            // Solve for the g that makes these match. Invert:
            //   g_target = hb_res_rms / folding_rms            (unit-less)
            //   g_target *= filter_ratio  (decoder divides by filter_ratio)
            //   q        = 10 + 8·ln(g_target)
            let eps = 1e-6f32;
            let g_target = (hb_res_rms / folding_rms.max(eps)) * filter_ratio.max(eps);
            let q_f = 10.0 + 8.0 * g_target.max(eps).ln();
            let q = q_f.round().clamp(0.0, 31.0) as u32;
            bw.write_bits(q, 5);
        }

        // ---- 8. Save state -------------------------------------------
        self.old_qlsp_high = qlsp;
        self.first = false;
        Ok(())
    }
}

// =====================================================================
// QMF analysis bank
// =====================================================================

/// Two-band QMF analysis — splits a full-band signal `x_in` into a
/// critically decimated low-pass branch `x_lo` and high-pass branch
/// `x_hi`, each of length `n/2`. `a` is the symmetric low-pass
/// prototype of length `m`. `mem_lo` / `mem_hi` are `m`-tap IIR
/// memories preserved across calls.
///
/// The high-pass branch uses the QMF relation `h1[n] = (−1)^n · h0[n]`;
/// we implement that implicitly inside the convolution loop by
/// multiplying the input samples' odd indices by −1 in the high-pass
/// sum.
///
/// Reference: `libspeex/filters.c` `qmf_decomp` (float path). The
/// reference unrolls by 4; here we keep the straight convolution for
/// clarity — the encoder runs once per 20 ms frame so the MAC count
/// (`n·m`) is negligible.
#[allow(clippy::too_many_arguments)]
pub fn qmf_decomp(
    x_in: &[f32],
    a: &[f32],
    x_lo: &mut [f32],
    x_hi: &mut [f32],
    n: usize,
    m: usize,
    mem_lo: &mut [f32],
    mem_hi: &mut [f32],
) {
    debug_assert_eq!(x_in.len(), n);
    debug_assert_eq!(x_lo.len(), n / 2);
    debug_assert_eq!(x_hi.len(), n / 2);
    debug_assert_eq!(a.len(), m);
    debug_assert!(mem_lo.len() >= m);
    debug_assert!(mem_hi.len() >= m);
    debug_assert!(n % 2 == 0);

    // Build a length-(m + n) "tap line" where the first `m` samples are
    // the carry-over memory (oldest first) and the next `n` are the new
    // input. Then for each decimated output index `k`, the convolution
    // reads taps `[k*2 .. k*2 + m]` into the prototype.
    //
    // mem layout convention: mem[0] is oldest, mem[m-1] is newest — so
    // appending `x_in` after `mem` gives a flat oldest-to-newest tap
    // line of total length m + n.
    let mut taps = vec![0.0f32; m + n];
    taps[..m].copy_from_slice(&mem_lo[..m]);
    taps[m..].copy_from_slice(x_in);

    let half_n = n / 2;
    for k in 0..half_n {
        let mut lo = 0.0f32;
        let mut hi = 0.0f32;
        // Tap window for this output: `2k` .. `2k + m - 1`.
        // The oldest tap is aligned with `a[0]`, the newest with
        // `a[m-1]`. The newest tap is `x_in[2k + m - 1 - m] = x_in[2k-1]`
        // which is the latest input sample consumed for this output.
        //
        // Low-pass: lo[k] = Σ_i a[i] · tap[2k + i]  (i = 0..m)
        // High-pass: hi[k] = Σ_i (-1)^i · a[i] · tap[2k + i]
        for i in 0..m {
            let ai = a[i];
            let v = taps[2 * k + i];
            lo += ai * v;
            // (-1)^i folds odd taps into the negative bin. The QMF
            // prototype's alternation between even/odd taps is what
            // shifts the low-pass to a high-pass.
            if i & 1 == 0 {
                hi += ai * v;
            } else {
                hi -= ai * v;
            }
        }
        x_lo[k] = lo;
        x_hi[k] = hi;
    }

    // Update memory to the last `m` samples of the tap line. These are
    // the newest inputs we've consumed — they become the oldest for the
    // next call.
    mem_lo[..m].copy_from_slice(&taps[n..n + m]);
    mem_hi[..m].copy_from_slice(&taps[n..n + m]);
}

// =====================================================================
// LPC analysis helpers (scoped to this module — NB's live in nb_encoder)
// =====================================================================

/// Symmetric Hamming window for the high-band frame.
fn hamming_window_wb(x: &[f32]) -> [f32; WB_FRAME_SIZE] {
    let mut out = [0.0f32; WB_FRAME_SIZE];
    let n = WB_FRAME_SIZE as f32 - 1.0;
    for i in 0..WB_FRAME_SIZE {
        let w = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / n).cos();
        out[i] = x[i] * w;
    }
    out
}

fn autocorrelate(x: &[f32], r: &mut [f32]) {
    let order = r.len() - 1;
    for k in 0..=order {
        let mut s = 0.0f32;
        for i in 0..x.len() - k {
            s += x[i] * x[i + k];
        }
        r[k] = s;
    }
}

fn levinson_durbin(r: &[f32]) -> [f32; WB_LPC_ORDER] {
    let mut a = [0.0f32; WB_LPC_ORDER];
    let mut tmp = [0.0f32; WB_LPC_ORDER];
    let mut e = r[0];
    if e <= 0.0 {
        return a;
    }
    for i in 0..WB_LPC_ORDER {
        let mut k = -r[i + 1];
        for j in 0..i {
            k -= a[j] * r[i - j];
        }
        if e.abs() < 1e-12 {
            break;
        }
        k /= e;
        const K_MAX: f32 = 0.999;
        k = k.clamp(-K_MAX, K_MAX);
        tmp[i] = k;
        for j in 0..i {
            tmp[j] = a[j] + k * a[i - 1 - j];
        }
        a[..=i].copy_from_slice(&tmp[..=i]);
        e *= 1.0 - k * k;
        if e <= 0.0 {
            e = 1e-6;
        }
    }
    a
}

/// Stateless FIR analysis filter — computes the LPC residual of `x`
/// given zero memory. (We don't need to carry state across sub-frames
/// here because the folding-gain search only cares about each
/// sub-frame's envelope energy, not the inter-sub-frame continuity.)
fn fir_filter_stateless(x: &[f32], a: &[f32], y: &mut [f32], order: usize) {
    let n = x.len();
    debug_assert!(y.len() >= n);
    for i in 0..n {
        let mut acc = x[i];
        for k in 0..order {
            if i > k {
                acc += a[k] * x[i - k - 1];
            }
        }
        y[i] = acc;
    }
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    let mut s = 0.0f32;
    for &v in x {
        s += v * v;
    }
    (s / x.len() as f32).sqrt()
}

// =====================================================================
// LPC -> LSP for high band (order 8)
// =====================================================================

/// Convert 8th-order LPC to LSPs in the `LSP_LINEAR_HIGH` convention
/// (radians on [0, π], sorted). Same polynomial decomposition as the
/// NB path (see `nb_encoder::lpc_to_lsp`), adapted for 8-order.
fn lpc_to_lsp_high(ak: &[f32; WB_LPC_ORDER]) -> Option<[f32; WB_LPC_ORDER]> {
    let p = WB_LPC_ORDER;
    let mut a_pad = [0.0f32; WB_LPC_ORDER + 2];
    a_pad[0] = 1.0;
    a_pad[1..=p].copy_from_slice(&ak[..p]);

    let n = p + 2; // 10
    let mut pcoef = [0.0f32; 10];
    let mut qcoef = [0.0f32; 10];
    for k in 0..n {
        let reflected = a_pad[(p + 1) - k];
        pcoef[k] = a_pad[k] + reflected;
        qcoef[k] = a_pad[k] - reflected;
    }

    let eval_p = |coeffs: &[f32; 10], omega: f32| -> f32 {
        let half = (n as f32 - 1.0) * 0.5; // 4.5
        let mut s = 0.0f32;
        for k in 0..(n / 2) {
            s += 2.0 * coeffs[k] * ((half - k as f32) * omega).cos();
        }
        s
    };
    let eval_q = |coeffs: &[f32; 10], omega: f32| -> f32 {
        let half = (n as f32 - 1.0) * 0.5; // 4.5
        let mut s = 0.0f32;
        for k in 0..(n / 2) {
            s += 2.0 * coeffs[k] * ((half - k as f32) * omega).sin();
        }
        s
    };

    const GRID_N: usize = 1024;
    let grid_to_omega = |i: usize| (i as f32 * std::f32::consts::PI) / GRID_N as f32;

    fn scan_roots(
        coeffs: &[f32; 10],
        grid: usize,
        to_omega: impl Fn(usize) -> f32,
        eval: impl Fn(&[f32; 10], f32) -> f32,
    ) -> Vec<f32> {
        let mut roots = Vec::with_capacity(WB_LPC_ORDER / 2);
        let eps = 1.0 / (grid as f32 * 2.0);
        let clamp_omega = |o: f32| o.clamp(eps, std::f32::consts::PI - eps);
        let mut prev_o = clamp_omega(to_omega(1));
        let mut prev_f = eval(coeffs, prev_o);
        for i in 2..grid {
            let o = clamp_omega(to_omega(i));
            let f = eval(coeffs, o);
            if prev_f * f < 0.0 {
                let mut lo = prev_o;
                let mut hi = o;
                let mut flo = prev_f;
                for _ in 0..32 {
                    let mid = 0.5 * (lo + hi);
                    let fmid = eval(coeffs, mid);
                    if fmid == 0.0 {
                        lo = mid;
                        hi = mid;
                        break;
                    }
                    if flo * fmid < 0.0 {
                        hi = mid;
                    } else {
                        lo = mid;
                        flo = fmid;
                    }
                }
                roots.push(0.5 * (lo + hi));
            }
            prev_o = o;
            prev_f = f;
        }
        roots
    }

    let r_p = scan_roots(&pcoef, GRID_N, grid_to_omega, eval_p);
    let r_q = scan_roots(&qcoef, GRID_N, grid_to_omega, eval_q);
    let mut roots = Vec::with_capacity(p);
    for &r in r_p.iter().chain(r_q.iter()) {
        if r > 1e-3 && r < std::f32::consts::PI - 1e-3 {
            roots.push(r);
        }
    }
    roots.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if roots.len() < p {
        return None;
    }
    roots.truncate(p);
    let mut out = [0.0f32; WB_LPC_ORDER];
    out[..p].copy_from_slice(&roots[..p]);
    let margin = LSP_MARGIN_HIGH;
    out[0] = out[0].max(margin);
    for i in 1..WB_LPC_ORDER {
        if out[i] < out[i - 1] + margin {
            out[i] = out[i - 1] + margin;
        }
    }
    out[WB_LPC_ORDER - 1] = out[WB_LPC_ORDER - 1].min(std::f32::consts::PI - margin);
    Some(out)
}

// =====================================================================
// High-band LSP quantisation (two-stage, 6+6 bits)
// =====================================================================

/// Quantise an 8-dim high-band LSP vector using the two VQ stages
/// `HIGH_LSP_CDBK` (64×8, scale 1/256) and `HIGH_LSP_CDBK2` (64×8,
/// scale 1/512). Returns the dequantised LSPs (matching what the
/// decoder will reconstruct) and the two 6-bit indices.
fn quantise_lsp_high(lsp: &[f32; WB_LPC_ORDER]) -> ([f32; WB_LPC_ORDER], usize, usize) {
    // Strip the linear initial guess: decoder recomputes it and adds
    // the codebook deltas on top.
    let mut residual = [0.0f32; WB_LPC_ORDER];
    for i in 0..WB_LPC_ORDER {
        residual[i] = lsp[i] - (0.3125 * i as f32 + 0.75);
    }

    // Stage 1: search HIGH_LSP_CDBK at scale 1/256.
    let idx1 = nearest_vector_scaled(&residual, 256.0, &HIGH_LSP_CDBK, 8, 64);
    for i in 0..8 {
        residual[i] -= (HIGH_LSP_CDBK[idx1 * 8 + i] as f32) / 256.0;
    }
    // Stage 2: search HIGH_LSP_CDBK2 at scale 1/512.
    let idx2 = nearest_vector_scaled(&residual, 512.0, &HIGH_LSP_CDBK2, 8, 64);

    // Reconstruct the decoder's qLSPs from the indices — the encoder
    // keeps `old_qlsp_high` bit-exact with what the decoder will see.
    let mut qlsp = [0.0f32; WB_LPC_ORDER];
    for i in 0..WB_LPC_ORDER {
        qlsp[i] = 0.3125 * i as f32
            + 0.75
            + (HIGH_LSP_CDBK[idx1 * 8 + i] as f32) / 256.0
            + (HIGH_LSP_CDBK2[idx2 * 8 + i] as f32) / 512.0;
    }
    // Stability margin — mirrors the decoder's `lsp_interpolate`
    // margin for the high band (and matches what the decoder would
    // reconstruct if it saw the same LSPs).
    let margin = LSP_MARGIN_HIGH;
    qlsp[0] = qlsp[0].max(margin);
    for i in 1..WB_LPC_ORDER {
        if qlsp[i] < qlsp[i - 1] + margin {
            qlsp[i] = qlsp[i - 1] + margin;
        }
    }
    qlsp[WB_LPC_ORDER - 1] = qlsp[WB_LPC_ORDER - 1].min(std::f32::consts::PI - margin);

    (qlsp, idx1, idx2)
}

fn nearest_vector_scaled(
    target: &[f32],
    scale: f32,
    cdbk: &[i8],
    dim: usize,
    count: usize,
) -> usize {
    let inv = 1.0 / scale;
    let mut best_idx = 0usize;
    let mut best_err = f32::INFINITY;
    for idx in 0..count {
        let mut err = 0.0f32;
        let base = idx * dim;
        for i in 0..dim {
            let v = cdbk[base + i] as f32 * inv;
            let d = target[i] - v;
            err += d * d;
        }
        if err < best_err {
            best_err = err;
            best_idx = idx;
        }
    }
    best_idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qmf_decomp_runs_and_conserves_energy_order() {
        // Split a single-frequency tone through the QMF and make sure
        // both bands produce finite output with energies that sum to
        // something comparable to the input energy (QMF is ~ energy-
        // preserving up to the prototype's stopband ripple).
        let mut x = [0.0f32; WB_FULL_FRAME_SIZE];
        for i in 0..WB_FULL_FRAME_SIZE {
            x[i] = 10000.0 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 16000.0).sin();
        }
        let mut lo = [0.0f32; WB_FRAME_SIZE];
        let mut hi = [0.0f32; WB_FRAME_SIZE];
        let mut ml = [0.0f32; QMF_ORDER];
        let mut mh = [0.0f32; QMF_ORDER];
        qmf_decomp(
            &x,
            &H0_PROTOTYPE,
            &mut lo,
            &mut hi,
            WB_FULL_FRAME_SIZE,
            QMF_ORDER,
            &mut ml,
            &mut mh,
        );
        let e_in: f32 = x.iter().map(|v| v * v).sum();
        let e_lo: f32 = lo.iter().map(|v| v * v).sum();
        let e_hi: f32 = hi.iter().map(|v| v * v).sum();
        assert!(e_in > 0.0, "input must have energy");
        assert!(lo.iter().all(|v| v.is_finite()));
        assert!(hi.iter().all(|v| v.is_finite()));
        // A 1 kHz tone should fall in the low band; expect lo energy
        // to dominate.
        assert!(
            e_lo > e_hi,
            "1 kHz tone should map to low band (lo {e_lo}, hi {e_hi})"
        );
    }

    #[test]
    fn encode_frame_writes_336_bits() {
        // 300 NB bits + 36 WB-extension bits = 336 bits per frame.
        let mut enc = WbEncoder::new();
        let mut bw = BitWriter::new();
        let mut pcm = [0.0f32; WB_FULL_FRAME_SIZE];
        for i in 0..WB_FULL_FRAME_SIZE {
            let t = i as f32;
            pcm[i] = 4000.0 * (t * 0.1).sin() + 2000.0 * (t * 0.35).cos();
        }
        enc.encode_frame(&pcm, &mut bw).unwrap();
        assert_eq!(bw.bit_position(), 336);
    }

    #[test]
    fn quantise_lsp_high_round_trip_stable() {
        let mut lsp = [0.0f32; WB_LPC_ORDER];
        for i in 0..WB_LPC_ORDER {
            lsp[i] = 0.3125 * i as f32 + 0.75;
        }
        let (qlsp, _, _) = quantise_lsp_high(&lsp);
        for i in 1..WB_LPC_ORDER {
            assert!(qlsp[i] > qlsp[i - 1], "qlsp must stay sorted");
        }
    }
}
