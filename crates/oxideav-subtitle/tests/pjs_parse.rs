//! PJS parse + write + probe.

use oxideav_core::Segment;
use oxideav_subtitle::pjs;

const SAMPLE: &str = "\u{FEFF}25,75,\"Hello world\"
100,150,\"Second cue\"
200,275,\"Third|with break\"
300,360,\"Fourth\"
400,450,\"Last\"
";

#[test]
fn parses_five_cues() {
    let t = pjs::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 5);
    assert_eq!(t.cues[0].start_us, frame_to_us(25, 25.0));
    assert_eq!(t.cues[0].end_us, frame_to_us(75, 25.0));
    assert_eq!(t.cues[4].end_us, frame_to_us(450, 25.0));
}

#[test]
fn pipe_is_linebreak() {
    let t = pjs::parse(SAMPLE.as_bytes()).unwrap();
    let breaks = t.cues[2]
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::LineBreak))
        .count();
    assert_eq!(breaks, 1);
}

#[test]
fn write_roundtrips() {
    let t = pjs::parse(SAMPLE.as_bytes()).unwrap();
    let bytes = pjs::write(&t).unwrap();
    let t2 = pjs::parse(&bytes).unwrap();
    assert_eq!(t.cues.len(), t2.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
}

#[test]
fn probe_positive() {
    assert!(pjs::probe(b"25,75,\"hi\"\n") >= 40);
}

#[test]
fn probe_zero_random() {
    assert_eq!(pjs::probe(b"random words\nno quoted text\n"), 0);
}

fn frame_to_us(frame: i64, fps: f64) -> i64 {
    ((frame as f64 / fps) * 1_000_000.0).round() as i64
}
