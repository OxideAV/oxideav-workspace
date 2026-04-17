//! Unified subtitle cue representation.
//!
//! Produced by subtitle-format decoders (SRT, WebVTT, ASS/SSA) and consumed
//! by the corresponding encoders. Timing is expressed in microseconds from
//! the start of the stream so the IR is format-independent.

/// A single displayable subtitle event.
#[derive(Clone, Debug, Default)]
pub struct SubtitleCue {
    /// Cue start, microseconds from stream start.
    pub start_us: i64,
    /// Cue end, microseconds from stream start.
    pub end_us: i64,
    /// Optional style name this cue inherits from. References an entry in
    /// the track-level style table (ASS `Style:` rows or WebVTT `::cue(.X)` rules).
    pub style_ref: Option<String>,
    /// Optional overriding position for this cue. `None` → use the style default.
    pub positioning: Option<CuePosition>,
    /// Cue body as a sequence of styled segments.
    pub segments: Vec<Segment>,
}

/// Positioning information for a cue.
///
/// Interpretation differs by source format:
/// * WebVTT — `x`/`y` are percentages of the viewport, `align` from cue settings.
/// * ASS `\pos(x, y)` — absolute pixel coordinates in the `PlayResX`×`PlayResY` canvas.
#[derive(Clone, Debug, Default)]
pub struct CuePosition {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub align: TextAlign,
    /// WebVTT `size:N%` cue setting. Irrelevant for ASS.
    pub size: Option<f32>,
}

/// Horizontal alignment for a cue / a style row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Left,
    Right,
}

/// One inline element of a cue body.
#[derive(Clone, Debug)]
pub enum Segment {
    Text(String),
    LineBreak,
    Bold(Vec<Segment>),
    Italic(Vec<Segment>),
    Underline(Vec<Segment>),
    Strike(Vec<Segment>),
    Color {
        rgb: (u8, u8, u8),
        children: Vec<Segment>,
    },
    Font {
        family: Option<String>,
        size: Option<f32>,
        children: Vec<Segment>,
    },
    /// WebVTT `<v Speaker>...</v>`.
    Voice {
        name: String,
        children: Vec<Segment>,
    },
    /// WebVTT `<c.classname>...</c>`.
    Class {
        name: String,
        children: Vec<Segment>,
    },
    /// ASS `{\k<cs>}` — the following text is highlighted for `cs` centiseconds.
    /// The children slice is the text under this karaoke beat (until the next
    /// `\k` override).
    Karaoke {
        cs: u32,
        children: Vec<Segment>,
    },
    /// WebVTT inline timestamp `<00:00:01.500>`.
    Timestamp {
        offset_us: i64,
    },
    /// Fallback for override tags we don't model explicitly. Carries the
    /// textual source verbatim so a re-emit to the same format stays faithful.
    Raw(String),
}

/// A named style definition — reusable across many cues.
#[derive(Clone, Debug, Default)]
pub struct SubtitleStyle {
    pub name: String,
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub primary_color: Option<(u8, u8, u8, u8)>,
    pub outline_color: Option<(u8, u8, u8, u8)>,
    pub back_color: Option<(u8, u8, u8, u8)>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub align: TextAlign,
    pub margin_l: Option<i32>,
    pub margin_r: Option<i32>,
    pub margin_v: Option<i32>,
    pub outline: Option<f32>,
    pub shadow: Option<f32>,
}

impl SubtitleStyle {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
}
