//! SubViewer 1.0 parsing, writing, probe.

use oxideav_core::Segment;
use oxideav_subtitle::subviewer1;

const SAMPLE: &str = "**START SCRIPT** 00:00:00
00:00:01,0
first subtitle|second line

00:00:04,0
hello

00:00:07,5
third cue|line two

00:00:10,0
fourth

00:00:13,0
fifth
**END SCRIPT**
";

#[test]
fn five_cues() {
    let t = subviewer1::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 5);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    // End = next cue's start.
    assert_eq!(t.cues[0].end_us, 4_000_000);
    assert_eq!(t.cues[2].start_us, 7_500_000);
}

#[test]
fn pipe_is_line_break() {
    let t = subviewer1::parse(SAMPLE.as_bytes()).unwrap();
    let mut lb = 0;
    for s in &t.cues[0].segments {
        if matches!(s, Segment::LineBreak) {
            lb += 1;
        }
    }
    assert_eq!(lb, 1, "first cue has one '|' separator");
}

#[test]
fn write_roundtrips() {
    let t = subviewer1::parse(SAMPLE.as_bytes()).unwrap();
    let out = subviewer1::write(&t).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("**START SCRIPT**"));
    assert!(s.contains("**END SCRIPT**"));
    assert!(s.contains("00:00:01,0"));
    assert!(s.contains("first subtitle|second line"));
    let t2 = subviewer1::parse(s.as_bytes()).unwrap();
    assert_eq!(t2.cues.len(), t.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
    }
}

#[test]
fn probe_positive() {
    assert!(subviewer1::probe(SAMPLE.as_bytes()) > 0);
}

#[test]
fn probe_rejects_webvtt() {
    assert_eq!(subviewer1::probe(b"WEBVTT\n"), 0);
}
