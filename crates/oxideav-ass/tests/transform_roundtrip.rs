//! Conversion between SRT, WebVTT, and ASS.

use oxideav_ass::{self as ass, ass_to_srt, ass_to_webvtt, srt_to_ass, webvtt_to_ass};
use oxideav_subtitle::{ir::plain_text, srt, webvtt};

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

const ASS_SRC: &str = r"[Script Info]
Title: x
ScriptType: v4.00+

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, Alignment, MarginL, MarginR, MarginV, Outline, Shadow
Style: Default,Arial,20,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,2,10,10,10,1,0

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\b1}Hello{\b0} world
Dialogue: 0,0:00:05.00,0:00:07.50,Default,,0,0,0,,plain text
";

#[test]
fn srt_to_ass_adds_default_style() {
    let out = srt_to_ass(SRT_SRC.as_bytes()).unwrap();
    let out_str = String::from_utf8(out).unwrap();
    assert!(out_str.contains("[Script Info]"));
    assert!(out_str.contains("[V4+ Styles]"));
    assert!(out_str.contains("Style: Default"));
    assert!(out_str.contains("[Events]"));
    let t = ass::parse(out_str.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    // Bold override survives.
    assert!(out_str.contains("{\\b1}"));
}

#[test]
fn ass_to_srt_strips_tags_but_keeps_bold() {
    let out = ass_to_srt(ASS_SRC.as_bytes()).unwrap();
    let out_str = String::from_utf8(out).unwrap();
    let t = srt::parse(out_str.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert!(out_str.contains("<b>Hello</b>"));
    let plain = plain_text(&t.cues[1].segments);
    assert_eq!(plain, "plain text");
}

#[test]
fn vtt_to_ass_preserves_timing_and_text() {
    let out = webvtt_to_ass(VTT_SRC.as_bytes()).unwrap();
    let out_str = String::from_utf8(out).unwrap();
    let t = ass::parse(out_str.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    let plain = plain_text(&t.cues[1].segments);
    assert!(plain.contains("Alice"), "got: {plain}");
}

#[test]
fn ass_to_vtt_timing_preserved() {
    let out = ass_to_webvtt(ASS_SRC.as_bytes()).unwrap();
    let out_str = String::from_utf8(out).unwrap();
    assert!(out_str.starts_with("WEBVTT"));
    let t = webvtt::parse(out_str.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert_eq!(t.cues[1].end_us, 7_500_000);
}
