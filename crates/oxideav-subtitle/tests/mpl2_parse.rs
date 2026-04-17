//! MPL2 parse + write + probe.

use oxideav_core::Segment;
use oxideav_subtitle::mpl2;

const SAMPLE: &str = "\u{FEFF}[0][25]Hello world
[30][60]Second cue
[80][120]/italic line
[140][180]line one|line two
[200][240]trailing\n";

#[test]
fn parses_five_cues() {
    let t = mpl2::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 5);
    assert_eq!(t.cues[0].start_us, 0);
    assert_eq!(t.cues[0].end_us, 2_500_000);
    assert_eq!(t.cues[1].start_us, 3_000_000);
    assert_eq!(t.cues[1].end_us, 6_000_000);
}

#[test]
fn slash_marks_italic() {
    let t = mpl2::parse(SAMPLE.as_bytes()).unwrap();
    let italic = &t.cues[2].segments[0];
    assert!(matches!(italic, Segment::Italic(_)));
}

#[test]
fn pipe_is_linebreak() {
    let t = mpl2::parse(SAMPLE.as_bytes()).unwrap();
    let breaks = t.cues[3]
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::LineBreak))
        .count();
    assert_eq!(breaks, 1);
}

#[test]
fn write_roundtrips() {
    let t = mpl2::parse(SAMPLE.as_bytes()).unwrap();
    let bytes = mpl2::write(&t).unwrap();
    let t2 = mpl2::parse(&bytes).unwrap();
    assert_eq!(t.cues.len(), t2.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
}

#[test]
fn probe_positive_header() {
    assert!(mpl2::probe(b"[0][25]hi\n") >= 40);
}

#[test]
fn probe_zero_random() {
    assert_eq!(mpl2::probe(b"hello world\nlorem ipsum\n"), 0);
}
