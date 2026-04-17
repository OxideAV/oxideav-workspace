//! AQTitle parse + write + probe.

use oxideav_core::Segment;
use oxideav_subtitle::aqtitle;

const SAMPLE: &str = "\u{FEFF}-->> 25
Hello world
-->> 75
Second cue
with two lines
-->> 150
Third cue
-->> 225
Fourth cue
-->> 300
Last cue
-->> 400
";

#[test]
fn parses_five_cues() {
    let t = aqtitle::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 5);
    assert_eq!(t.cues[0].start_us, frame_to_us(25, 25.0));
    assert_eq!(t.cues[0].end_us, frame_to_us(75, 25.0));
    // Last cue: 300..400
    assert_eq!(t.cues[4].start_us, frame_to_us(300, 25.0));
    assert_eq!(t.cues[4].end_us, frame_to_us(400, 25.0));
}

#[test]
fn preserves_multiline_body() {
    let t = aqtitle::parse(SAMPLE.as_bytes()).unwrap();
    let breaks = t.cues[1]
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::LineBreak))
        .count();
    assert_eq!(breaks, 1);
}

#[test]
fn write_roundtrips() {
    let t = aqtitle::parse(SAMPLE.as_bytes()).unwrap();
    let bytes = aqtitle::write(&t).unwrap();
    let t2 = aqtitle::parse(&bytes).unwrap();
    assert_eq!(t.cues.len(), t2.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
}

#[test]
fn probe_positive() {
    assert!(aqtitle::probe(b"-->> 25\nhi\n") >= 50);
}

#[test]
fn probe_zero_random() {
    assert_eq!(aqtitle::probe(b"hello world\nno markers\n"), 0);
}

fn frame_to_us(frame: i64, fps: f64) -> i64 {
    ((frame as f64 / fps) * 1_000_000.0).round() as i64
}
