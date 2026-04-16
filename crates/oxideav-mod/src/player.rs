//! ProTracker playback engine.
//!
//! Drives a `PlayerState` forward one tick at a time, rendering mixed
//! stereo PCM as it goes. The mixing core keeps per-channel state in
//! `Channel` so that a future multichannel-output mode can tap the
//! same per-channel buffers without a refactor (see MEMORY.md →
//! "MOD multichannel").
//!
//! Terminology:
//! - **Row**: a line in a pattern. A pattern has 64 rows.
//! - **Tick**: one row is `speed` ticks long (default 6).
//! - **BPM**: governs wall-clock tick duration. Samples-per-tick =
//!   `sample_rate * 2.5 / BPM` — 882 at 44.1 kHz / 125 BPM.
//! - **Period**: the Amiga Paula divider. Output frequency =
//!   PAULA_CLOCK / period.

use crate::header::{ModHeader, PATTERN_ROWS};
use crate::samples::SampleBody;

/// Paula clock (PAL) — classic MOD period→frequency constant. Divide by
/// the period to get the Amiga's output sample rate for that channel.
pub const PAULA_CLOCK: f32 = 7_093_789.2 / 2.0;

pub const DEFAULT_SPEED: u8 = 6;
pub const DEFAULT_BPM: u8 = 125;
pub const CHANNELS_PER_MOD: usize = 4;

/// A single decoded pattern row entry for one channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Note {
    /// Period value (0 means "no new note").
    pub period: u16,
    /// Sample index 1..=31 (0 means "no sample change").
    pub sample: u8,
    /// Effect command nibble (0..=0xF).
    pub effect: u8,
    /// Effect parameter byte.
    pub effect_param: u8,
}

impl Note {
    fn decode(raw: [u8; 4]) -> Self {
        // Byte 0: ssss pppp  (high nibble of sample, high nibble of period)
        // Byte 1: pppp pppp  (low 8 bits of period)
        // Byte 2: ssss eeee  (low nibble of sample, effect nibble)
        // Byte 3: xxxx xxxx  (effect parameter)
        let period = (((raw[0] & 0x0F) as u16) << 8) | raw[1] as u16;
        let sample = (raw[0] & 0xF0) | (raw[2] >> 4);
        let effect = raw[2] & 0x0F;
        let effect_param = raw[3];
        Note {
            period,
            sample,
            effect,
            effect_param,
        }
    }
}

/// A decoded pattern: 64 rows × N channels.
#[derive(Clone, Debug)]
pub struct Pattern {
    pub rows: Vec<Vec<Note>>, // rows[row][channel]
}

/// Parse all patterns from a MOD bytestream.
pub fn parse_patterns(header: &ModHeader, bytes: &[u8]) -> Vec<Pattern> {
    let channels = header.channels as usize;
    let mut patterns = Vec::with_capacity(header.n_patterns as usize);
    let base = header.pattern_data_offset();

    for p in 0..header.n_patterns as usize {
        let mut rows = Vec::with_capacity(PATTERN_ROWS);
        for r in 0..PATTERN_ROWS {
            let mut row = Vec::with_capacity(channels);
            for c in 0..channels {
                let off = base + (p * PATTERN_ROWS + r) * channels * 4 + c * 4;
                let raw = if off + 4 <= bytes.len() {
                    [bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]
                } else {
                    [0; 4]
                };
                row.push(Note::decode(raw));
            }
            rows.push(row);
        }
        patterns.push(Pattern { rows });
    }
    patterns
}

/// Per-channel playback state.
#[derive(Clone, Debug, Default)]
pub struct Channel {
    /// 1-based sample index (0 = no sample ever triggered).
    pub sample_index: u8,
    /// Fractional read position into the sample's pcm buffer.
    pub sample_pos: f32,
    /// Current period (0 = silent / not playing).
    pub period: u16,
    /// Current volume 0..=64.
    pub volume: u8,
    /// Whether this channel is currently sounding.
    pub active: bool,
    /// Effect memory: arpeggio base period for this row.
    pub arp_base_period: u16,
    /// Current effect command (0..=0xF).
    pub effect: u8,
    pub effect_param: u8,
}

impl Channel {
    /// Mix one sample from this channel into (left, right) accumulators.
    /// `pan_right` is 0.0 for hard-left, 1.0 for hard-right.
    fn mix_one(&mut self, samples: &[SampleBody], out_rate: f32) -> f32 {
        if !self.active || self.period == 0 {
            return 0.0;
        }
        let idx = self.sample_index as usize;
        if idx == 0 || idx > samples.len() {
            return 0.0;
        }
        let body = &samples[idx - 1];
        if body.pcm.is_empty() {
            return 0.0;
        }

        let pos = self.sample_pos;
        let len = body.pcm.len() as f32;
        if pos >= len {
            // Either loop or stop.
            if body.is_looped() {
                let loop_end = (body.loop_start + body.loop_length) as f32;
                let loop_start = body.loop_start as f32;
                let span = loop_end - loop_start;
                if span > 0.0 {
                    let over = pos - loop_start;
                    self.sample_pos = loop_start + over.rem_euclid(span);
                } else {
                    self.active = false;
                    return 0.0;
                }
            } else {
                self.active = false;
                return 0.0;
            }
        }

        // Linear interpolation between two nearest samples.
        let i = self.sample_pos as usize;
        let frac = self.sample_pos - i as f32;
        let s0 = body.pcm[i.min(body.pcm.len() - 1)] as f32 / 128.0;
        let s1_idx = if i + 1 < body.pcm.len() {
            i + 1
        } else if body.is_looped() {
            body.loop_start as usize
        } else {
            i
        };
        let s1 = body.pcm[s1_idx.min(body.pcm.len() - 1)] as f32 / 128.0;
        let interp = s0 + (s1 - s0) * frac;

        let out = interp * (self.volume as f32 / 64.0);

        // Advance sample_pos by output-rate-scaled increment.
        let chan_rate = PAULA_CLOCK / self.period as f32;
        let step = chan_rate / out_rate;
        self.sample_pos += step;

        out
    }
}

/// Top-level player state. Owns samples, patterns, order, and the
/// per-channel mixer. Feeds `render(dst)` to fill an interleaved stereo
/// S16 buffer.
pub struct PlayerState {
    pub samples: Vec<SampleBody>,
    pub patterns: Vec<Pattern>,
    pub order: Vec<u8>,
    pub song_length: u8,

    pub channels: Vec<Channel>,
    pub speed: u8,
    pub bpm: u8,

    /// Current position in the order table (0..song_length).
    pub order_index: u8,
    /// Current row inside the current pattern (0..64).
    pub row: u8,
    /// Current tick inside the current row (0..speed).
    pub tick: u8,
    /// Samples emitted so far within the current tick.
    pub tick_sample_cursor: u32,

    pub sample_rate: u32,
    /// Flag set when the song has wrapped past its last order.
    pub ended: bool,

    /// Pending pattern break / position jump (consumed on tick advance).
    pending_jump: Option<Jump>,
}

#[derive(Clone, Copy, Debug)]
struct Jump {
    /// Next order index (None = next order + 1).
    order: Option<u8>,
    /// Row to start at in the new pattern (default 0).
    row: u8,
}

impl PlayerState {
    pub fn new(
        header: &ModHeader,
        samples: Vec<SampleBody>,
        patterns: Vec<Pattern>,
        sample_rate: u32,
    ) -> Self {
        let channels = (0..header.channels)
            .map(|_| Channel::default())
            .collect::<Vec<_>>();
        PlayerState {
            samples,
            patterns,
            order: header.order.clone(),
            song_length: header.song_length,
            channels,
            speed: DEFAULT_SPEED,
            bpm: DEFAULT_BPM,
            order_index: 0,
            row: 0,
            tick: 0,
            tick_sample_cursor: 0,
            sample_rate,
            ended: false,
            pending_jump: None,
        }
    }

    /// Samples-per-tick rounded down. Classic formula is
    /// `sample_rate * 2.5 / BPM`.
    pub fn samples_per_tick(&self) -> u32 {
        ((self.sample_rate as f32) * 2.5 / self.bpm as f32) as u32
    }

    /// Process the row at the current position (called at tick 0).
    fn enter_row(&mut self) {
        let pattern_idx = self
            .order
            .get(self.order_index as usize)
            .copied()
            .unwrap_or(0) as usize;
        if pattern_idx >= self.patterns.len() {
            self.ended = true;
            return;
        }
        let row_notes: Vec<Note> = self.patterns[pattern_idx].rows[self.row as usize].clone();
        for (ch_idx, note) in row_notes.iter().enumerate() {
            if ch_idx >= self.channels.len() {
                break;
            }
            let ch = &mut self.channels[ch_idx];
            ch.effect = note.effect;
            ch.effect_param = note.effect_param;

            // Sample change (loads volume etc. even without a note).
            if note.sample != 0 {
                ch.sample_index = note.sample;
                if let Some(body) = self.samples.get(note.sample as usize - 1) {
                    ch.volume = body.volume;
                }
            }
            // Note trigger.
            if note.period != 0 {
                ch.period = note.period;
                ch.sample_pos = 0.0;
                ch.active = true;
                ch.arp_base_period = note.period;
            } else {
                ch.arp_base_period = ch.period;
            }

            // Tick-0 effects.
            apply_tick0_effect(ch, note.effect, note.effect_param, &mut self.pending_jump);
        }

        // Speed / BPM change Fxx applies immediately on tick 0 — already
        // handled in apply_tick0_effect via pending_speed? No — handled
        // directly here for simplicity.
        for ch in &self.channels {
            if ch.effect == 0xF {
                let p = ch.effect_param;
                if p == 0 {
                    // "Fxx 00" — halts; ignore for now.
                } else if p < 0x20 {
                    self.speed = p;
                } else {
                    self.bpm = p;
                }
            }
        }
    }

    /// Advance one tick (called at the start of every tick).
    fn advance_tick(&mut self) {
        if self.tick == 0 {
            self.enter_row();
        } else {
            // Tick-N effects.
            for ch in &mut self.channels {
                apply_tickn_effect(ch, self.tick);
            }
        }
    }

    /// Move to next row (or jump).
    fn next_row(&mut self) {
        if let Some(jump) = self.pending_jump.take() {
            if let Some(order) = jump.order {
                self.order_index = order;
            } else {
                self.order_index = self.order_index.saturating_add(1);
            }
            self.row = jump.row;
        } else {
            self.row += 1;
            if self.row as usize >= PATTERN_ROWS {
                self.row = 0;
                self.order_index = self.order_index.saturating_add(1);
            }
        }
        if self.order_index >= self.song_length {
            self.ended = true;
        }
    }

    /// Render one stereo S16 interleaved sample pair by mixing all channels.
    /// Channels 0 and 3 pan hard-left, 1 and 2 hard-right (Amiga convention).
    fn render_one(&mut self, out: &mut [i16]) {
        let out_rate = self.sample_rate as f32;
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        let n_ch = self.channels.len();
        for (i, ch) in self.channels.iter_mut().enumerate() {
            let s = ch.mix_one(&self.samples, out_rate);
            // Amiga pan: channels 0 & 3 left, 1 & 2 right. For >4 channels
            // alternate L/R.
            let left = matches!(i % 4, 0 | 3);
            if left {
                l += s;
            } else {
                r += s;
            }
        }
        // Scale: divide by expected max contributions so 4-channel full
        // output stays in -1..1 range.
        let norm = (n_ch as f32 / 2.0).max(1.0);
        l /= norm;
        r /= norm;
        // Clip softly.
        let l = l.clamp(-1.0, 1.0);
        let r = r.clamp(-1.0, 1.0);
        out[0] = (l * 32767.0) as i16;
        out[1] = (r * 32767.0) as i16;
    }

    /// Render `n_frames` stereo samples into `dst` (interleaved S16,
    /// length = n_frames * 2). Returns samples actually rendered (may be
    /// less than requested if song ends).
    pub fn render(&mut self, dst: &mut [i16]) -> usize {
        assert!(dst.len() % 2 == 0);
        let mut produced = 0usize;
        let total_frames = dst.len() / 2;

        while produced < total_frames {
            if self.ended {
                break;
            }
            if self.tick_sample_cursor == 0 {
                self.advance_tick();
            }
            let spt = self.samples_per_tick().max(1);
            let remaining_in_tick = spt.saturating_sub(self.tick_sample_cursor);
            let want = (total_frames - produced).min(remaining_in_tick as usize);

            for _ in 0..want {
                let off = produced * 2;
                self.render_one(&mut dst[off..off + 2]);
                produced += 1;
            }

            self.tick_sample_cursor += want as u32;
            if self.tick_sample_cursor >= spt {
                self.tick_sample_cursor = 0;
                self.tick += 1;
                if self.tick >= self.speed {
                    self.tick = 0;
                    self.next_row();
                }
            }
        }
        produced
    }
}

/// Apply tick-0 (row-start) effects. Effects that operate at every tick
/// are applied in `apply_tickn_effect`.
fn apply_tick0_effect(ch: &mut Channel, effect: u8, param: u8, pending_jump: &mut Option<Jump>) {
    let x = param >> 4;
    let y = param & 0x0F;
    match effect {
        0xC => {
            // Cxx: set volume.
            ch.volume = param.min(64);
        }
        0xB => {
            // Bxx: position jump.
            *pending_jump = Some(Jump {
                order: Some(param),
                row: 0,
            });
        }
        0xD => {
            // Dxy: pattern break — next row = x*10 + y in next pattern.
            let row = (x * 10 + y).min(63);
            *pending_jump = Some(Jump { order: None, row });
        }
        _ => {
            // Other effects are per-tick or ignored.
        }
    }
}

/// Apply per-tick (tick > 0) effects.
fn apply_tickn_effect(ch: &mut Channel, tick: u8) {
    let effect = ch.effect;
    let param = ch.effect_param;
    let x = param >> 4;
    let y = param & 0x0F;
    match effect {
        // 0xy: arpeggio. On tick 0 it's a no-op (handled there).
        // On subsequent ticks, cycle through base / +x / +y semitones.
        0x0 if param != 0 => {
            let semis = match tick % 3 {
                0 => 0,
                1 => x as i32,
                2 => y as i32,
                _ => 0,
            };
            if semis == 0 {
                ch.period = ch.arp_base_period;
            } else {
                // period / 2^(semis/12)
                let factor = 2.0f32.powf(semis as f32 / 12.0);
                let p = (ch.arp_base_period as f32 / factor) as u16;
                ch.period = p.max(1);
            }
        }
        0x1 => {
            // 1xx: portamento up.
            ch.period = ch.period.saturating_sub(param as u16).max(113);
        }
        0x2 => {
            // 2xx: portamento down.
            ch.period = (ch.period + param as u16).min(856);
        }
        0xA => {
            // Axy: volume slide. +x or -y per tick (after tick 0).
            if x != 0 {
                ch.volume = (ch.volume as u16 + x as u16).min(64) as u8;
            } else if y != 0 {
                ch.volume = ch.volume.saturating_sub(y);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::header::parse_header;
    use crate::samples::extract_samples;

    /// Build a tiny synthetic 4-channel M.K. MOD with one square-wave
    /// sample and one pattern that triggers notes on channel 0 across
    /// the first 4 rows.
    pub fn synth_square_mod() -> Vec<u8> {
        let mut out = vec![0u8; crate::header::HEADER_FIXED_SIZE];
        out[0..4].copy_from_slice(b"test");
        // Sample 1: 32 samples, length-in-words = 16.
        out[20 + 22..20 + 24].copy_from_slice(&16u16.to_be_bytes());
        // Finetune 0, volume 64.
        out[20 + 24] = 0;
        out[20 + 25] = 64;
        // Loop points: start 0, length 16 words (= 32 samples) — loops full.
        out[20 + 26..20 + 28].copy_from_slice(&0u16.to_be_bytes());
        out[20 + 28..20 + 30].copy_from_slice(&16u16.to_be_bytes());
        // Song length 1, order[0] = 0.
        out[950] = 1;
        out[951] = 0x7F;
        out[952] = 0;
        // Signature.
        out[1080..1084].copy_from_slice(b"M.K.");
        // Pattern 0: 64 rows × 4 channels × 4 bytes = 1024 bytes.
        let mut pat = vec![0u8; 64 * 4 * 4];
        // Rows 0,16,32,48 — trigger sample 1 on channel 0 with
        // descending periods (higher pitch first). Pick periods C-2, D-2,
        // E-2, F-2 — classic PT values: 428, 381, 339, 320.
        let rows_and_periods = [(0, 428u16), (16, 381), (32, 339), (48, 320)];
        for &(row, period) in &rows_and_periods {
            let off = row * 4 * 4;
            // Sample high nibble (sample = 1, high = 0, low = 1).
            // Byte 0 = (sample_hi << 4) | period_hi.
            let p_hi = ((period >> 8) & 0x0F) as u8;
            let p_lo = (period & 0xFF) as u8;
            let sample_hi = 0u8; // high nibble of sample index 1
            let sample_lo = 1u8;
            pat[off] = (sample_hi << 4) | p_hi;
            pat[off + 1] = p_lo;
            pat[off + 2] = sample_lo << 4; // effect 0
            pat[off + 3] = 0; // param
        }
        out.extend(pat);
        // Sample body: 32-sample square wave (16 hi, 16 lo).
        for i in 0..32 {
            let v: i8 = if i < 16 { 100 } else { -100 };
            out.push(v as u8);
        }
        out
    }

    #[test]
    fn decodes_patterns() {
        let bytes = synth_square_mod();
        let header = parse_header(&bytes).unwrap();
        let patterns = parse_patterns(&header, &bytes);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].rows.len(), 64);
        assert_eq!(patterns[0].rows[0].len(), 4);
        let n = patterns[0].rows[0][0];
        assert_eq!(n.period, 428);
        assert_eq!(n.sample, 1);
    }

    #[test]
    fn player_renders_nonzero_audio() {
        let bytes = synth_square_mod();
        let header = parse_header(&bytes).unwrap();
        let samples = extract_samples(&header, &bytes);
        let patterns = parse_patterns(&header, &bytes);
        let mut player = PlayerState::new(&header, samples, patterns, 44_100);

        // Render ~0.1 s (4410 frames × 2 channels = 8820 samples).
        let mut buf = vec![0i16; 4410 * 2];
        let produced = player.render(&mut buf);
        assert_eq!(produced, 4410);

        // Must have at least some non-zero samples.
        let nonzero = buf.iter().filter(|&&x| x != 0).count();
        assert!(
            nonzero > 100,
            "expected non-silent PCM, got {nonzero} non-zero samples"
        );
    }

    #[test]
    fn samples_per_tick_default() {
        let bytes = synth_square_mod();
        let header = parse_header(&bytes).unwrap();
        let samples = extract_samples(&header, &bytes);
        let patterns = parse_patterns(&header, &bytes);
        let player = PlayerState::new(&header, samples, patterns, 44_100);
        assert_eq!(player.samples_per_tick(), 882);
    }
}
