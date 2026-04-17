//! Conversion between SRT and WebVTT.
//!
//! ASS conversion tests live in the sibling `oxideav-ass` crate.

use oxideav_subtitle::{ir::plain_text, srt, srt_to_webvtt, webvtt, webvtt_to_srt};

const SRT_SRC: &str = "1
00:00:01,000 --> 00:00:03,000
<b>Hello</b> <i>world</i>
second line

2
00:00:05,000 --> 00:00:07,500
plain text
";

const VTT_SRC: &str = "WEBVTT

00:00:01.000 --> 00:00:03.000
<b>Hello</b> <i>world</i>
second line

00:00:05.000 --> 00:00:07.500
<v Alice>plain text</v>
";

#[test]
fn srt_to_vtt_roundtrip() {
    let out = srt_to_webvtt(SRT_SRC.as_bytes()).unwrap();
    let out_str = String::from_utf8(out).unwrap();
    assert!(out_str.starts_with("WEBVTT"));
    let t = webvtt::parse(out_str.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert_eq!(t.cues[0].end_us, 3_000_000);
    // Bold + italic preserved on roundtrip.
    let body = render_vtt_body(&t.cues[0].segments);
    assert!(body.contains("<b>Hello</b>"));
    assert!(body.contains("<i>world</i>"));
}

#[test]
fn vtt_to_srt_voice_becomes_prefix() {
    let out = webvtt_to_srt(VTT_SRC.as_bytes()).unwrap();
    let out_str = String::from_utf8(out).unwrap();
    let t = srt::parse(out_str.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    let plain = plain_text(&t.cues[1].segments);
    assert!(plain.contains("Alice: plain text"), "got: {plain}");
}

fn render_vtt_body(segments: &[oxideav_core::Segment]) -> String {
    // Cheap renderer to verify tags survive.
    use oxideav_core::Segment;
    let mut out = String::new();
    for s in segments {
        match s {
            Segment::Bold(c) => {
                out.push_str("<b>");
                out.push_str(&render_vtt_body(c));
                out.push_str("</b>");
            }
            Segment::Italic(c) => {
                out.push_str("<i>");
                out.push_str(&render_vtt_body(c));
                out.push_str("</i>");
            }
            Segment::Underline(c) => {
                out.push_str("<u>");
                out.push_str(&render_vtt_body(c));
                out.push_str("</u>");
            }
            Segment::Text(t) => out.push_str(t),
            Segment::LineBreak => out.push('\n'),
            Segment::Color { children, .. }
            | Segment::Font { children, .. }
            | Segment::Class { children, .. }
            | Segment::Voice { children, .. }
            | Segment::Karaoke { children, .. } => {
                out.push_str(&render_vtt_body(children));
            }
            Segment::Timestamp { .. } => {}
            Segment::Raw(s) => out.push_str(s),
            Segment::Strike(c) => out.push_str(&render_vtt_body(c)),
        }
    }
    out
}
