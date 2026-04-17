//! Container probe tests.

use std::io::Cursor;

use oxideav_container::ContainerRegistry;
use oxideav_gif::register_containers;

#[test]
fn probe_gif89a_scores_100() {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"GIF89a");
    buf.extend_from_slice(&1u16.to_le_bytes()); // width
    buf.extend_from_slice(&1u16.to_le_bytes()); // height
    buf.push(0); // packed (no GCT)
    buf.push(0); // bg
    buf.push(0); // aspect
    buf.push(0x3B); // trailer

    let mut reg = ContainerRegistry::new();
    register_containers(&mut reg);
    let mut cur = Cursor::new(buf);
    let name = reg.probe_input(&mut cur, None).expect("probe");
    assert_eq!(name, "gif");
}

#[test]
fn probe_random_bytes_no_match() {
    let buf = vec![0u8; 32];
    let mut reg = ContainerRegistry::new();
    register_containers(&mut reg);
    let mut cur = Cursor::new(buf);
    let result = reg.probe_input(&mut cur, None);
    assert!(result.is_err());
}

#[test]
fn probe_gif87a_scores_100() {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"GIF87a");
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.push(0);
    buf.push(0);
    buf.push(0);
    buf.push(0x3B);
    let mut reg = ContainerRegistry::new();
    register_containers(&mut reg);
    let mut cur = Cursor::new(buf);
    assert_eq!(reg.probe_input(&mut cur, None).unwrap(), "gif");
}
