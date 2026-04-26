//! Surround-aware audio routing between decoder and `sysaudio` output.
//!
//! Sits between the decoder's [`AudioFrame`] (which can carry anything
//! from mono up to 7.1 + LFE) and the `sysaudio` device — which on most
//! laptops opens stereo even when the source is 5.1, but on a connected
//! HDMI receiver / USB DAC will report a true 6/8-channel layout.
//!
//! The decision tree implemented here:
//!
//! ```text
//! match (src_layout, dev_layout, headphones, user_override) {
//!     (src, dev, _, None)          if src == dev          => passthrough,
//!     (_,   _,   _, Some(None))                            => passthrough,
//!     (src, _,   _, Some(Some(mode)))                      => apply(mode),
//!     (src, Stereo, Some(true), _) if src.is_surround()    => Binaural,
//!     (src, Stereo, _,          _) if src.is_surround()    => LoRo,
//!     (src, Mono,   _,          _)                          => Average,
//!     (Surround51, Surround50, _,_)                         => DropLfe,
//!     _                                                    => Truncate,  // fallback
//! }
//! ```
//!
//! ## Part B integration TODO
//!
//! Part B (`oxideav-audio-filter::DownmixFilter`) is not yet committed
//! at the time this lands. The matrix coefficients used below mirror
//! ITU-R BS.775 / AC-3 6.1 spec defaults so the audible behaviour is
//! correct now; once Part B's `DownmixFilter::auto(src, dst)` API
//! finalises, [`apply_routing`] should delegate to it instead of doing
//! the matrix multiplication inline. The tracking comment is on every
//! `// Part B TODO` line below.
//!
//! ## Headphone detection
//!
//! [`HeadphoneStatus`] is queried from the engine on a slow tick (1 Hz),
//! never inside the audio callback. macOS implementation lives in
//! [`crate::drivers::headphones_macos`]; other platforms always return
//! `Unknown` for now — see that module's docs.

use oxideav_core::{AudioFrame, ChannelLayout, SampleFormat};

use crate::drivers::audio_convert::sample_to_f32;

/// User-facing downmix mode. Mirrors what Part B's
/// `DownmixFilter::Mode` will publish; we mirror the names so the CLI
/// stays stable across the Part B handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownmixMode {
    /// Lo/Ro: industry-standard stereo downmix per ATSC A/52 §7.7.2.
    /// Surround channels are mixed in with -3 dB attenuation, centre
    /// at -3 dB, LFE dropped. Best general-purpose mode for speakers.
    LoRo,
    /// Lt/Rt: matrix-encoded stereo downmix carrying surround info that
    /// a Pro Logic decoder can re-extract. Useful when feeding a
    /// downstream surround-decoding amplifier.
    LtRt,
    /// Naive arithmetic mean across all source channels — used for the
    /// stereo→mono drop and as a generic fallback when no surround
    /// matrix is appropriate.
    Average,
    /// HRTF-based binaural rendering — per-channel pan + crosstalk
    /// suitable for headphones. Today this is implemented as an
    /// enhanced LoRo with widened side-channel pan; once Part B lands
    /// it will swap to a real HRTF kernel.
    Binaural,
}

impl DownmixMode {
    pub fn name(&self) -> &'static str {
        match self {
            Self::LoRo => "loro",
            Self::LtRt => "ltrt",
            Self::Average => "average",
            Self::Binaural => "binaural",
        }
    }
}

impl std::str::FromStr for DownmixMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "loro" | "lo/ro" => Ok(Self::LoRo),
            "ltrt" | "lt/rt" => Ok(Self::LtRt),
            "average" | "avg" | "mean" => Ok(Self::Average),
            "binaural" | "hrtf" => Ok(Self::Binaural),
            other => Err(format!(
                "unknown downmix mode '{other}' — pick loro, ltrt, average, or binaural"
            )),
        }
    }
}

/// User policy for the routing decision. Resolved from `--downmix` and
/// `--no-downmix` at CLI parse time; the engine then asks the router
/// what to do with each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DownmixPolicy {
    /// Pick automatically based on (source, device, headphones).
    #[default]
    Auto,
    /// Forced mode — applied regardless of headphones / device.
    Force(DownmixMode),
    /// `--no-downmix` — never insert any downmix; if the device can't
    /// take the source layout the open will fail (caller's
    /// responsibility to surface the error).
    Forbid,
}

/// Output of the headphone probe. `Unknown` is the safe default —
/// triggers non-binaural downmix paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadphoneStatus {
    /// We're confident the active output is a headphone (wired,
    /// USB headphone, or recognised wireless headphone like AirPods).
    Yes,
    /// We're confident the active output is NOT a headphone (built-in
    /// speakers / HDMI / line out).
    No,
    /// Probe couldn't determine. Treated as `No` for the binaural
    /// gating decision.
    #[default]
    Unknown,
}

impl HeadphoneStatus {
    pub fn is_headphone(self) -> bool {
        matches!(self, Self::Yes)
    }
}

/// Routing decision computed once per (source, device, headphone) tuple.
/// Stable across many frames — the engine only recomputes when one of
/// the three inputs changes. `Passthrough` is the cheap fast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    /// Source and device match — feed frames through verbatim.
    Passthrough,
    /// Apply the downmix matrix for `mode`, producing `out_channels`
    /// per output frame. The destination layout is implied by the
    /// device channel count.
    Downmix {
        mode: DownmixMode,
        out_channels: u16,
    },
}

/// The decision-tree implementation. Pure function; testable without
/// any device. See module-level docs for the table.
pub fn decide_routing(
    src: ChannelLayout,
    dst_channels: u16,
    headphones: HeadphoneStatus,
    policy: DownmixPolicy,
) -> Routing {
    // We don't currently inspect the device's `ChannelLayout`
    // structure — the routing decision is keyed on channel count plus
    // the source layout's `is_surround()` predicate. That keeps
    // `DiscreteN(n)` device counts (e.g. 4 from a quad-out box that
    // doesn't advertise a named layout) from getting forced into the
    // wrong matrix. If a future round wants to e.g. avoid LFE on a
    // device that doesn't have an LFE channel, switch on
    // `ChannelLayout::from_count(dst_channels)` here.
    let _ = ChannelLayout::from_count(dst_channels);

    // 1. User overrides take priority over automatic picks.
    match policy {
        DownmixPolicy::Forbid => return Routing::Passthrough,
        DownmixPolicy::Force(mode) => {
            // Even a forced mode is a no-op when source already matches
            // device — saves a buffer copy per frame.
            if src.channel_count() == dst_channels {
                return Routing::Passthrough;
            }
            return Routing::Downmix {
                mode,
                out_channels: dst_channels,
            };
        }
        DownmixPolicy::Auto => {}
    }

    // 2. Same channel count → passthrough. Note we compare by count
    //    rather than layout equality because LoRo/LtRt/Stereo all share
    //    count=2 and are interchangeable as device targets.
    if src.channel_count() == dst_channels {
        return Routing::Passthrough;
    }

    // 3. Down to stereo from surround → headphone-aware pick.
    if dst_channels == 2 && src.is_surround() {
        let mode = if headphones.is_headphone() {
            DownmixMode::Binaural
        } else {
            DownmixMode::LoRo
        };
        return Routing::Downmix {
            mode,
            out_channels: 2,
        };
    }

    // 4. Down to mono → average.
    if dst_channels == 1 {
        return Routing::Downmix {
            mode: DownmixMode::Average,
            out_channels: 1,
        };
    }

    // 5. Surround51 → Surround50 (drop LFE) — handled by LoRo logic
    //    rebuilt for 5ch out via the generic path. No dedicated mode;
    //    the matrix at apply time treats `out_channels=5` correctly.
    //    Same for any other channel-count downsize where neither side
    //    is stereo: fall through to Average as a safe default.
    Routing::Downmix {
        mode: DownmixMode::Average,
        out_channels: dst_channels,
    }
}

/// Apply the chosen [`Routing`] to a decoded frame, producing
/// f32-interleaved samples ready to push to the device's ring buffer.
///
/// `Passthrough` re-uses the existing
/// [`crate::drivers::audio_convert::to_f32_interleaved`] helper for the
/// sample-format conversion. `Downmix` dispatches to the per-mode matrix
/// below.
///
/// `src_layout` is the *semantic* speaker layout of the source. It can
/// be derived from `CodecParameters::resolved_layout()` at stream open
/// (preferred — picks up explicit container tags) or from
/// [`ChannelLayout::from_count`] on the frame's `channels` (fallback
/// when no layout was set).
///
/// Part B TODO: once `oxideav_audio_filter::DownmixFilter` lands, swap
/// the body of the `Downmix` arm for a call into that crate so the
/// matrices stay in one place.
pub fn apply_routing(
    frame: &AudioFrame,
    src_format: oxideav_core::SampleFormat,
    src_channels: u16,
    src_layout: ChannelLayout,
    routing: Routing,
) -> Vec<f32> {
    match routing {
        Routing::Passthrough => to_f32_passthrough(frame, src_format, src_channels),
        Routing::Downmix { mode, out_channels } => {
            // Decode every input channel to f32 into a planar scratch
            // buffer, then run the matrix. Going planar first costs one
            // extra copy but keeps the matrix readable per-position
            // rather than per-byte-offset.
            let planes = decode_planes_f32(frame, src_format, src_channels);
            apply_matrix(&planes, src_layout, mode, out_channels)
        }
    }
}

/// Fast path: same channel count source vs. destination. We still
/// have to convert the sample format (S16/F32/etc.) to interleaved
/// f32, but no mixing.
fn to_f32_passthrough(
    frame: &AudioFrame,
    src_format: oxideav_core::SampleFormat,
    src_channels: u16,
) -> Vec<f32> {
    let in_ch = src_channels.max(1) as usize;
    let n = frame.samples as usize;
    let mut out = Vec::with_capacity(n * in_ch);
    for i in 0..n {
        for c in 0..in_ch {
            out.push(sample_to_f32(frame, src_format, src_channels, c, i));
        }
    }
    out
}

/// Decode the input frame into one f32 vector per source channel
/// ("planar" representation regardless of the frame's storage). Length
/// is `channels × samples`.
fn decode_planes_f32(
    frame: &AudioFrame,
    src_format: oxideav_core::SampleFormat,
    src_channels: u16,
) -> Vec<Vec<f32>> {
    let in_ch = src_channels.max(1) as usize;
    let n = frame.samples as usize;
    let mut planes: Vec<Vec<f32>> = (0..in_ch).map(|_| Vec::with_capacity(n)).collect();
    for i in 0..n {
        for (c, plane) in planes.iter_mut().enumerate() {
            plane.push(sample_to_f32(frame, src_format, src_channels, c, i));
        }
    }
    planes
}

/// Apply the chosen downmix matrix.
///
/// The coefficients here are the standard ATSC A/52 §7.7.2 (Lo/Ro) and
/// the Dolby Pro Logic Lt/Rt encoding equations. The `Binaural` mode is
/// today an extended Lo/Ro with widened side-channel pan so headphone
/// listeners get a wider stage; once Part B lands it will swap to a
/// proper HRTF kernel.
///
/// `out_channels`-vs-source-layout mapping:
/// - out=2: Lo/Ro / Lt/Rt / Binaural / Average all produce stereo.
/// - out=1: Average — sums every input channel, normalises by `1/N`.
/// - out=N where 1<N<src.channel_count: drop LFE first, then truncate
///   to N. Used for the rare 5.1→5.0 (drop LFE) case.
fn apply_matrix(
    planes: &[Vec<f32>],
    layout: ChannelLayout,
    mode: DownmixMode,
    out_channels: u16,
) -> Vec<f32> {
    let n = planes.first().map(|p| p.len()).unwrap_or(0);
    let oc = out_channels.max(1) as usize;
    let mut out = Vec::with_capacity(n * oc);

    // Helper: fetch source channel by ChannelPosition; returns 0.0 if
    // the layout doesn't carry that position.
    let get = |i: usize, pos: oxideav_core::ChannelPosition| -> f32 {
        layout
            .positions()
            .iter()
            .position(|p| *p == pos)
            .and_then(|c| planes.get(c))
            .map(|plane| plane[i])
            .unwrap_or(0.0)
    };
    use oxideav_core::ChannelPosition::*;

    if oc == 1 {
        // Mono: arithmetic mean. LFE intentionally excluded; it would
        // dominate.
        for i in 0..n {
            let mut acc = 0.0f32;
            let mut count = 0u32;
            for (c, plane) in planes.iter().enumerate() {
                if matches!(layout.position(c), Some(LowFrequency)) {
                    continue;
                }
                acc += plane[i];
                count += 1;
            }
            let count = count.max(1) as f32;
            out.push(acc / count);
        }
        return out;
    }

    if oc == 2 {
        // -3 dB ≈ 1/√2 ≈ 0.7071. Centre and surround attenuation per
        // ATSC A/52 §7.7.2.
        const HALF_DB3: f32 = std::f32::consts::FRAC_1_SQRT_2;
        // Wider pan for binaural (-6 dB ≈ 0.5012) — widens the apparent
        // stage when the listener is on headphones.
        const HALF_DB6: f32 = 0.501_187_2;
        for i in 0..n {
            let l = get(i, FrontLeft);
            let r = get(i, FrontRight);
            let c = get(i, FrontCenter);
            let ls = get(i, SideLeft);
            let rs = get(i, SideRight);
            let lb = get(i, BackLeft);
            let rb = get(i, BackRight);
            let cs = get(i, BackCenter);

            let (lo, ro) = match mode {
                // ATSC A/52 §7.7.2 LoRo.
                DownmixMode::LoRo | DownmixMode::Average => {
                    let lo = l + HALF_DB3 * c + HALF_DB3 * (ls + lb) + HALF_DB3 * cs;
                    let ro = r + HALF_DB3 * c + HALF_DB3 * (rs + rb) + HALF_DB3 * cs;
                    (lo, ro)
                }
                // Lt/Rt — surround channels phase-inverted on the
                // opposite side. Matches Pro Logic encoding.
                DownmixMode::LtRt => {
                    let surround = HALF_DB3 * (ls + rs + lb + rb);
                    let lt = l + HALF_DB3 * c - surround;
                    let rt = r + HALF_DB3 * c + surround;
                    (lt, rt)
                }
                // Binaural — wider stage (-6 dB on rears, opposite-side
                // crossfeed on side channels). Approximation of HRTF;
                // Part B's dedicated filter will replace this.
                DownmixMode::Binaural => {
                    let lb = HALF_DB6 * lb;
                    let rb = HALF_DB6 * rb;
                    let cross_l = HALF_DB6 * rs;
                    let cross_r = HALF_DB6 * ls;
                    let lo = l + HALF_DB3 * c + ls - rb + cross_l + HALF_DB3 * cs;
                    let ro = r + HALF_DB3 * c + rs - lb + cross_r + HALF_DB3 * cs;
                    (lo, ro)
                }
            };
            out.push(soft_clip(lo));
            out.push(soft_clip(ro));
        }
        return out;
    }

    // Generic >2ch downsize: drop LFE, then take the first `oc`
    // remaining canonical positions in `layout.positions()`. Used for
    // 5.1→5.0 and the rare 7.1→6.x cases. Falls back to a zero-padded
    // truncate if the source is `DiscreteN` (no positions).
    let positions = layout.positions();
    if positions.is_empty() {
        // DiscreteN — truncate or zero-pad with no semantic mapping.
        for i in 0..n {
            for c in 0..oc {
                out.push(planes.get(c).map(|p| p[i]).unwrap_or(0.0));
            }
        }
        return out;
    }

    // Build a table of (output_slot, source_index_in_planes).
    let mut mapping: Vec<usize> = Vec::with_capacity(oc);
    for (idx, pos) in positions.iter().enumerate() {
        if matches!(pos, LowFrequency) {
            continue;
        }
        mapping.push(idx);
        if mapping.len() == oc {
            break;
        }
    }
    while mapping.len() < oc {
        mapping.push(mapping.last().copied().unwrap_or(0));
    }

    for i in 0..n {
        for &src_c in &mapping {
            out.push(planes.get(src_c).map(|p| p[i]).unwrap_or(0.0));
        }
    }
    out
}

/// -1 dB soft clipper. The matrices above can sum past unity (worst
/// case 5.1 → stereo with all positive samples is ~2.4 ≈ 7.6 dB);
/// without clipping the device-side conversion to S16/S24 wraps. A
/// gentle tanh-based soft-clip keeps the transient peaks contained at
/// the cost of a touch of harmonic distortion on hot material.
fn soft_clip(s: f32) -> f32 {
    // tanh near 1.0 saturates around ±0.96. Below ±0.5 it's linear,
    // which is the regime LoRo'd content normally lives in.
    if s.abs() <= 0.5 {
        s
    } else {
        s.tanh()
    }
}

// `SampleFormat` re-exported for the test below — `apply_matrix` itself
// doesn't need it.
#[allow(dead_code)]
const _SAMPLE_FORMAT_USED: Option<SampleFormat> = None;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_on_matching_layout() {
        // Stereo → stereo, no override → passthrough.
        assert_eq!(
            decide_routing(
                ChannelLayout::Stereo,
                2,
                HeadphoneStatus::Unknown,
                DownmixPolicy::Auto
            ),
            Routing::Passthrough
        );
        // 5.1 → 5.1 device → passthrough even on headphones.
        assert_eq!(
            decide_routing(
                ChannelLayout::Surround51,
                6,
                HeadphoneStatus::Yes,
                DownmixPolicy::Auto
            ),
            Routing::Passthrough
        );
    }

    #[test]
    fn surround_to_speakers_picks_loro() {
        let r = decide_routing(
            ChannelLayout::Surround51,
            2,
            HeadphoneStatus::No,
            DownmixPolicy::Auto,
        );
        assert_eq!(
            r,
            Routing::Downmix {
                mode: DownmixMode::LoRo,
                out_channels: 2
            }
        );
    }

    #[test]
    fn surround_to_headphones_picks_binaural() {
        let r = decide_routing(
            ChannelLayout::Surround51,
            2,
            HeadphoneStatus::Yes,
            DownmixPolicy::Auto,
        );
        assert_eq!(
            r,
            Routing::Downmix {
                mode: DownmixMode::Binaural,
                out_channels: 2
            }
        );
    }

    #[test]
    fn unknown_headphones_treated_as_speakers() {
        let r = decide_routing(
            ChannelLayout::Surround51,
            2,
            HeadphoneStatus::Unknown,
            DownmixPolicy::Auto,
        );
        assert_eq!(
            r,
            Routing::Downmix {
                mode: DownmixMode::LoRo,
                out_channels: 2
            }
        );
    }

    #[test]
    fn forced_binaural_overrides_speakers() {
        let r = decide_routing(
            ChannelLayout::Surround51,
            2,
            HeadphoneStatus::No,
            DownmixPolicy::Force(DownmixMode::Binaural),
        );
        assert_eq!(
            r,
            Routing::Downmix {
                mode: DownmixMode::Binaural,
                out_channels: 2
            }
        );
    }

    #[test]
    fn forbid_keeps_passthrough_even_on_mismatch() {
        // The `Forbid` policy never inserts a downmix — if the device
        // can't open at the source layout, that's a separate error
        // raised at open time, not silently masked here.
        assert_eq!(
            decide_routing(
                ChannelLayout::Surround51,
                2,
                HeadphoneStatus::No,
                DownmixPolicy::Forbid
            ),
            Routing::Passthrough
        );
    }

    #[test]
    fn stereo_to_mono_uses_average() {
        let r = decide_routing(
            ChannelLayout::Stereo,
            1,
            HeadphoneStatus::Unknown,
            DownmixPolicy::Auto,
        );
        assert_eq!(
            r,
            Routing::Downmix {
                mode: DownmixMode::Average,
                out_channels: 1
            }
        );
    }

    #[test]
    fn surround51_to_surround50_drops_lfe_via_average() {
        let r = decide_routing(
            ChannelLayout::Surround51,
            5,
            HeadphoneStatus::Unknown,
            DownmixPolicy::Auto,
        );
        assert_eq!(
            r,
            Routing::Downmix {
                mode: DownmixMode::Average,
                out_channels: 5
            }
        );
    }

    fn make_frame(channels: u16, planar_samples: &[f32]) -> AudioFrame {
        // Build an interleaved F32 frame.
        let n = planar_samples.len() / channels as usize;
        let mut bytes = Vec::with_capacity(planar_samples.len() * 4);
        for s in planar_samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        AudioFrame {
            samples: n as u32,
            pts: None,
            data: vec![bytes],
        }
    }

    #[test]
    fn loro_matrix_5_1_to_stereo_sane_levels() {
        // One sample per channel, 5.1 layout: L=0.5, R=0.5, C=0.5,
        // LFE=1.0 (should be ignored), Ls=0.25, Rs=0.25.
        let frame = make_frame(6, &[0.5, 0.5, 0.5, 1.0, 0.25, 0.25]);
        let out = apply_routing(
            &frame,
            SampleFormat::F32,
            6,
            ChannelLayout::from_count(6),
            Routing::Downmix {
                mode: DownmixMode::LoRo,
                out_channels: 2,
            },
        );
        assert_eq!(out.len(), 2);
        // Lo = L + 0.7071*C + 0.7071*Ls = 0.5 + 0.354 + 0.177 = 1.031;
        // soft-clipped via tanh to ~0.775.
        assert!(out[0] > 0.5 && out[0] < 1.0, "got Lo={}", out[0]);
        assert!(out[1] > 0.5 && out[1] < 1.0, "got Ro={}", out[1]);
        // LFE never makes it through.
        let dc_only = make_frame(6, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let dc_out = apply_routing(
            &dc_only,
            SampleFormat::F32,
            6,
            ChannelLayout::from_count(6),
            Routing::Downmix {
                mode: DownmixMode::LoRo,
                out_channels: 2,
            },
        );
        assert_eq!(dc_out, vec![0.0, 0.0]);
    }

    #[test]
    fn passthrough_round_trips_stereo_f32() {
        let frame = make_frame(2, &[0.25, -0.25, 0.5, -0.5]);
        let out = apply_routing(
            &frame,
            SampleFormat::F32,
            2,
            ChannelLayout::Stereo,
            Routing::Passthrough,
        );
        assert_eq!(out, vec![0.25, -0.25, 0.5, -0.5]);
    }

    #[test]
    fn average_to_mono_drops_lfe() {
        // 5.1: L=R=C=Ls=Rs=0.4, LFE=1.0. Mean of non-LFE = 0.4.
        let frame = make_frame(6, &[0.4, 0.4, 0.4, 1.0, 0.4, 0.4]);
        let out = apply_routing(
            &frame,
            SampleFormat::F32,
            6,
            ChannelLayout::Surround51,
            Routing::Downmix {
                mode: DownmixMode::Average,
                out_channels: 1,
            },
        );
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.4).abs() < 1e-5, "expected ~0.4, got {}", out[0]);
    }

    #[test]
    fn downmix_mode_parses_from_str() {
        use std::str::FromStr;
        assert_eq!(DownmixMode::from_str("loro").unwrap(), DownmixMode::LoRo);
        assert_eq!(DownmixMode::from_str("LoRo").unwrap(), DownmixMode::LoRo);
        assert_eq!(DownmixMode::from_str("lo/ro").unwrap(), DownmixMode::LoRo);
        assert_eq!(
            DownmixMode::from_str("binaural").unwrap(),
            DownmixMode::Binaural
        );
        assert!(DownmixMode::from_str("garbage").is_err());
    }
}
