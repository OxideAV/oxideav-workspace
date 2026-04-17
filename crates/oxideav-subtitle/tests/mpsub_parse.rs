//! MPsub parse + write + probe.

use oxideav_core::Segment;
use oxideav_subtitle::mpsub;

const SAMPLE: &str = "\u{FEFF}TITLE=Demo
AUTHOR=Tester
FORMAT=TIME

2.5 3.0
First cue

1.0 4.0
Second cue
with two lines

0.5 2.5
Third cue

2.0 1.5
Fourth cue

0 2
Fifth cue
";

#[test]
fn parses_five_cues() {
    let t = mpsub::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 5);
    // Cue 0: start=2.5, dur=3.0 → end=5.5
    assert_eq!(t.cues[0].start_us, 2_500_000);
    assert_eq!(t.cues[0].end_us, 5_500_000);
    // Cue 1: rel 1.0 from cue0.start=2.5 → 3.5, dur 4.0 → 7.5
    assert_eq!(t.cues[1].start_us, 3_500_000);
    assert_eq!(t.cues[1].end_us, 7_500_000);
    // Cue 2: rel 0.5 from 3.5 → 4.0, dur 2.5 → 6.5
    assert_eq!(t.cues[2].start_us, 4_000_000);
    assert_eq!(t.cues[2].end_us, 6_500_000);
}

#[test]
fn preserves_multiline_text() {
    let t = mpsub::parse(SAMPLE.as_bytes()).unwrap();
    let breaks = t.cues[1]
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::LineBreak))
        .count();
    assert_eq!(breaks, 1);
}

#[test]
fn write_roundtrips() {
    let t = mpsub::parse(SAMPLE.as_bytes()).unwrap();
    let bytes = mpsub::write(&t).unwrap();
    let t2 = mpsub::parse(&bytes).unwrap();
    assert_eq!(t.cues.len(), t2.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
}

#[test]
fn probe_positive_header() {
    assert!(mpsub::probe(b"FORMAT=TIME\n\n1.0 2.0\nhi\n") >= 60);
}

#[test]
fn probe_zero_random() {
    assert_eq!(mpsub::probe(b"hello\nworld\n"), 0);
}
