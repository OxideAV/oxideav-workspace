//! VPlayer parse + write + probe.

use oxideav_core::Segment;
use oxideav_subtitle::vplayer;

const SAMPLE: &str = "\u{FEFF}00:00:01:Hello world
00:00:04:Second cue
00:00:07:Third cue with|line break
00:00:12:Fourth
00:00:20:Last cue
";

#[test]
fn parses_five_cues() {
    let t = vplayer::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 5);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert_eq!(t.cues[0].end_us, 4_000_000);
    assert_eq!(t.cues[1].start_us, 4_000_000);
    assert_eq!(t.cues[1].end_us, 7_000_000);
    // Trailing cue gets a 3s fallback.
    assert_eq!(t.cues[4].end_us, 20_000_000 + 3_000_000);
}

#[test]
fn pipe_as_linebreak() {
    let t = vplayer::parse(SAMPLE.as_bytes()).unwrap();
    let breaks = t.cues[2]
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::LineBreak))
        .count();
    assert_eq!(breaks, 1);
}

#[test]
fn write_roundtrips() {
    let t = vplayer::parse(SAMPLE.as_bytes()).unwrap();
    let bytes = vplayer::write(&t).unwrap();
    let t2 = vplayer::parse(&bytes).unwrap();
    assert_eq!(t.cues.len(), t2.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        // End times may differ for the trailing cue if the next cue was
        // absent; first four should match exactly.
    }
    assert_eq!(t.cues[0].end_us, t2.cues[0].end_us);
}

#[test]
fn probe_positive() {
    assert!(vplayer::probe(b"00:00:01:hello\n") >= 35);
}

#[test]
fn probe_zero_random() {
    assert_eq!(vplayer::probe(b"hello world\nrandom\n"), 0);
}
