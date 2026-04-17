//! Direct format-to-format subtitle converters.
//!
//! Each helper parses into the unified IR and re-emits in the target
//! format. Lossy downconversions strip information the target can't
//! represent (positioning, karaoke timing, ASS-only styles) but preserve
//! b / i / u / color and line breaks where possible.

use oxideav_core::Result;

use crate::{ass, srt, webvtt};

/// SRT → WebVTT.
///
/// * Preserved: timing, b/i/u, color via `<font color>` (converted to
///   class-less markup because WebVTT inline color is class-based).
/// * Lost: nothing beyond what SRT already has.
pub fn srt_to_webvtt(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut t = srt::parse(bytes)?;
    t.extradata.clear();
    Ok(webvtt::write(&t))
}

/// SRT → ASS. Adds a `Default` style. Color spans become `\c` overrides.
pub fn srt_to_ass(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut t = srt::parse(bytes)?;
    t.extradata.clear();
    Ok(ass::write(&t))
}

/// WebVTT → SRT. Drops STYLE/REGION blocks, positioning, and class tags.
/// Voice tags `<v Name>` survive by prefixing the line with `Name: `.
pub fn webvtt_to_srt(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut t = webvtt::parse(bytes)?;
    // Inline voice name into text prefixes so SRT keeps it.
    for cue in &mut t.cues {
        cue.segments = flatten_voice(std::mem::take(&mut cue.segments), None);
        cue.positioning = None;
    }
    t.extradata.clear();
    Ok(srt::write(&t))
}

/// WebVTT → ASS. Preserves styles (converted to `[V4+ Styles]`),
/// positioning (→ `\pos`), bold/italic/underline, and voice names
/// (collapsed into the dialogue text).
pub fn webvtt_to_ass(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut t = webvtt::parse(bytes)?;
    for cue in &mut t.cues {
        cue.segments = flatten_voice(std::mem::take(&mut cue.segments), None);
    }
    t.extradata.clear();
    Ok(ass::write(&t))
}

/// ASS → SRT. Drops styles, positioning, and karaoke timing; keeps
/// `b`/`i`/`u`/`s`/color spans.
pub fn ass_to_srt(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut t = ass::parse(bytes)?;
    for cue in &mut t.cues {
        cue.style_ref = None;
        cue.positioning = None;
        cue.segments = drop_karaoke(std::mem::take(&mut cue.segments));
    }
    t.extradata.clear();
    Ok(srt::write(&t))
}

/// ASS → WebVTT. Styles map to `STYLE ::cue()` blocks; positioning maps
/// loosely to `line:`/`position:`; karaoke is dropped.
pub fn ass_to_webvtt(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut t = ass::parse(bytes)?;
    for cue in &mut t.cues {
        cue.segments = drop_karaoke(std::mem::take(&mut cue.segments));
    }
    t.extradata.clear();
    Ok(webvtt::write(&t))
}

use oxideav_core::Segment;

fn flatten_voice(segments: Vec<Segment>, voice: Option<&str>) -> Vec<Segment> {
    let mut out = Vec::with_capacity(segments.len());
    for seg in segments {
        match seg {
            Segment::Voice { name, children } => {
                let effective = if voice.is_some() {
                    voice.map(|s| s.to_string())
                } else if name.is_empty() {
                    None
                } else {
                    Some(name.clone())
                };
                if let Some(n) = &effective {
                    out.push(Segment::Text(format!("{}: ", n)));
                }
                out.extend(flatten_voice(children, effective.as_deref()));
            }
            Segment::Bold(c) => out.push(Segment::Bold(flatten_voice(c, voice))),
            Segment::Italic(c) => out.push(Segment::Italic(flatten_voice(c, voice))),
            Segment::Underline(c) => out.push(Segment::Underline(flatten_voice(c, voice))),
            Segment::Strike(c) => out.push(Segment::Strike(flatten_voice(c, voice))),
            Segment::Color { rgb, children } => out.push(Segment::Color {
                rgb,
                children: flatten_voice(children, voice),
            }),
            Segment::Font {
                family,
                size,
                children,
            } => out.push(Segment::Font {
                family,
                size,
                children: flatten_voice(children, voice),
            }),
            Segment::Class { children, .. } => out.extend(flatten_voice(children, voice)),
            Segment::Karaoke { children, .. } => out.extend(flatten_voice(children, voice)),
            other => out.push(other),
        }
    }
    out
}

fn drop_karaoke(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out = Vec::with_capacity(segments.len());
    for seg in segments {
        match seg {
            Segment::Karaoke { children, .. } => {
                out.extend(drop_karaoke(children));
            }
            Segment::Bold(c) => out.push(Segment::Bold(drop_karaoke(c))),
            Segment::Italic(c) => out.push(Segment::Italic(drop_karaoke(c))),
            Segment::Underline(c) => out.push(Segment::Underline(drop_karaoke(c))),
            Segment::Strike(c) => out.push(Segment::Strike(drop_karaoke(c))),
            Segment::Color { rgb, children } => out.push(Segment::Color {
                rgb,
                children: drop_karaoke(children),
            }),
            Segment::Font {
                family,
                size,
                children,
            } => out.push(Segment::Font {
                family,
                size,
                children: drop_karaoke(children),
            }),
            Segment::Voice { name, children } => out.push(Segment::Voice {
                name,
                children: drop_karaoke(children),
            }),
            Segment::Class { name, children } => out.push(Segment::Class {
                name,
                children: drop_karaoke(children),
            }),
            other => out.push(other),
        }
    }
    out
}
