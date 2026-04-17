//! Direct format-to-format subtitle converters.
//!
//! Each helper parses into the unified IR and re-emits in the target
//! format. Lossy downconversions strip information the target can't
//! represent (positioning, ASS-only styles) but preserve b / i / u /
//! color and line breaks where possible.
//!
//! ASS ↔ SRT / ASS ↔ WebVTT converters live in the sibling `oxideav-ass`
//! crate.

use oxideav_core::Result;

use crate::{srt, webvtt};

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
