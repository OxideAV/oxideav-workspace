//! Container probe + demux/mux smoke test.

use std::io::Cursor;

use oxideav_container::ContainerRegistry;

fn make_registry() -> ContainerRegistry {
    let mut reg = ContainerRegistry::new();
    oxideav_ass::register_containers(&mut reg);
    reg
}

#[test]
fn ass_probes_ass() {
    let reg = make_registry();
    let src = b"[Script Info]\nTitle: x\n\n[V4+ Styles]\nFormat: Name\nStyle: Default\n\n[Events]\nFormat: Layer, Start, End, Text\nDialogue: 0,0:00:01.00,0:00:02.00,hi\n";
    let mut c = Cursor::new(src.to_vec());
    let name = reg.probe_input(&mut c, Some("ass")).unwrap();
    assert_eq!(name, "ass");
}

#[test]
fn demuxer_yields_one_packet_per_cue() {
    let reg = make_registry();
    let src = b"[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,first\nDialogue: 0,0:00:03.00,0:00:04.50,Default,,0,0,0,,second\n".to_vec();
    let mut c = Cursor::new(src);
    let name = reg.probe_input(&mut c, Some("ass")).unwrap();
    let mut dmx = reg.open_demuxer(&name, Box::new(c)).unwrap();
    let p1 = dmx.next_packet().unwrap();
    assert_eq!(p1.pts, Some(1_000_000));
    let p2 = dmx.next_packet().unwrap();
    assert_eq!(p2.pts, Some(3_000_000));
    assert!(dmx.next_packet().is_err());
}
