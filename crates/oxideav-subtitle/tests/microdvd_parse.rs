//! MicroDVD parse + write + probe.

use oxideav_core::Segment;
use oxideav_subtitle::microdvd;

const SAMPLE: &str = "\u{FEFF}{1}{1}25
{25}{75}Hello world
{100}{150}{y:i}italic text
{200}{240}{c:$0000FF}red line
{260}{320}line one|line two
{400}{450}plain trailing
";

#[test]
fn parses_five_cues() {
    let t = microdvd::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 5);
    assert_eq!(t.cues[0].start_us, frame_to_us(25, 25.0));
    assert_eq!(t.cues[0].end_us, frame_to_us(75, 25.0));
    assert_eq!(t.cues[4].end_us, frame_to_us(450, 25.0));
}

#[test]
fn recognises_inline_styling() {
    let t = microdvd::parse(SAMPLE.as_bytes()).unwrap();
    let italic = &t.cues[1].segments[0];
    assert!(matches!(italic, Segment::Italic(_)));
    let color = &t.cues[2].segments[0];
    match color {
        Segment::Color { rgb, .. } => assert_eq!(*rgb, (255, 0, 0)),
        other => panic!("want color, got {other:?}"),
    }
}

#[test]
fn write_roundtrips() {
    let t = microdvd::parse(SAMPLE.as_bytes()).unwrap();
    let bytes = microdvd::write(&t).unwrap();
    let t2 = microdvd::parse(&bytes).unwrap();
    assert_eq!(t.cues.len(), t2.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
}

#[test]
fn probe_positive_on_header() {
    let buf = b"{1}{1}25\n{25}{50}hi\n";
    assert!(microdvd::probe(buf) >= 50);
}

#[test]
fn probe_zero_on_random_text() {
    let buf = b"lorem ipsum dolor sit amet\nconsectetur adipiscing\n";
    assert_eq!(microdvd::probe(buf), 0);
}

fn frame_to_us(frame: i64, fps: f64) -> i64 {
    ((frame as f64 / fps) * 1_000_000.0).round() as i64
}
