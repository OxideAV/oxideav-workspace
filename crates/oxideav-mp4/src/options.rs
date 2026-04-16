//! Muxer configuration for the MP4 / ISOBMFF writer.
//!
//! The default [`Mp4MuxerOptions`] matches what `muxer::open` has always done:
//! major brand `mp42`, no faststart, no fragmentation. Three convenience
//! presets are provided via [`BrandPreset`] for the common `mp4`, `mov`, and
//! `ismv` registry entries; a `Custom` variant lets callers supply any major
//! + compatible brand list directly.

/// Brand preset controlling the `ftyp` box written at the start of the file.
///
/// The four-byte codes follow ISO/IEC 14496-12 and the de-facto QuickTime /
/// Smooth Streaming conventions:
///
/// * [`Mp4`](BrandPreset::Mp4): `mp42` / `isom mp42 mp41 iso2`
/// * [`Mov`](BrandPreset::Mov): `qt  ` / `qt  `
/// * [`Ismv`](BrandPreset::Ismv): `iso4` / `iso4 piff iso6 isml`
/// * [`Custom`](BrandPreset::Custom): caller-supplied major + compatible list
#[derive(Clone, Debug)]
pub enum BrandPreset {
    /// Standard MP4 — `major=mp42`, compatible=`isom mp42 mp41 iso2`.
    Mp4,
    /// Apple QuickTime — `major=qt  `, compatible=`qt  `.
    Mov,
    /// Microsoft Smooth Streaming / ISMV — `major=iso4`, compatible=`iso4 piff iso6 isml`.
    ///
    /// NOTE: real ISMV requires fragmentation (moof/mfra); our muxer currently
    /// emits a non-fragmented layout. The fragmentation agent will wire
    /// `frag_keyframe` on for this preset later. For now, the file is
    /// structurally an MP4 with an ISMV ftyp brand.
    Ismv,
    /// Custom brand with an explicit major + compatible list.
    Custom {
        major: [u8; 4],
        compatible: Vec<[u8; 4]>,
    },
}

impl BrandPreset {
    /// Return the major brand for this preset.
    pub fn major_brand(&self) -> [u8; 4] {
        match self {
            BrandPreset::Mp4 => *b"mp42",
            BrandPreset::Mov => *b"qt  ",
            BrandPreset::Ismv => *b"iso4",
            BrandPreset::Custom { major, .. } => *major,
        }
    }

    /// Return the list of compatible brands for this preset.
    pub fn compatible_brands(&self) -> Vec<[u8; 4]> {
        match self {
            BrandPreset::Mp4 => vec![*b"isom", *b"mp42", *b"mp41", *b"iso2"],
            BrandPreset::Mov => vec![*b"qt  "],
            BrandPreset::Ismv => vec![*b"iso4", *b"piff", *b"iso6", *b"isml"],
            BrandPreset::Custom { compatible, .. } => compatible.clone(),
        }
    }
}

/// Runtime options controlling how the MP4 muxer shapes its output.
///
/// Call [`Mp4MuxerOptions::default`] for the historical behavior of the
/// plain `"mp4"` registry entry (major=`mp42`, no faststart, no frag).
#[derive(Clone, Debug)]
pub struct Mp4MuxerOptions {
    /// `ftyp` brand preset written at the beginning of the file.
    pub brand: BrandPreset,
    /// If `true`, rewrite the file at `write_trailer` time so `moov` precedes
    /// `mdat` ("faststart" / "web-optimized" layout). Requires a seekable
    /// output (which `WriteSeek` already provides).
    pub faststart: bool,

    // --- Fragmentation flags (owned by another agent) ----------------------
    // These are kept in the struct so the API is stable; defaults are all
    // `false` / sensible no-ops until the fragmentation state machine is
    // implemented.
    //
    /// TODO(fragmentation-agent): emit a new fragment whenever a keyframe arrives.
    pub frag_keyframe: bool,
    /// TODO(fragmentation-agent): initial `moov` contains no samples (all data
    /// lives in moof fragments).
    pub empty_moov: bool,
    /// TODO(fragmentation-agent): delay writing `moov` until the first
    /// fragment is finalised.
    pub delay_moov: bool,
    /// TODO(fragmentation-agent): target fragment duration in milliseconds.
    pub fragment_duration_ms: u32,
}

impl Default for Mp4MuxerOptions {
    fn default() -> Self {
        Self {
            brand: BrandPreset::Mp4,
            faststart: false,
            frag_keyframe: false,
            empty_moov: false,
            delay_moov: false,
            fragment_duration_ms: 2000,
        }
    }
}
