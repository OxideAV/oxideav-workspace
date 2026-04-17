//! LZW round-trip integration tests covering short and long inputs.

use oxideav_gif::Lzw;

fn roundtrip(min_code_size: u8, input: &[u8]) -> usize {
    let mut enc = Lzw::encoder(min_code_size).expect("encoder");
    let mut compressed = Vec::new();
    enc.write(input, &mut compressed);
    enc.finish(&mut compressed);
    let dec = Lzw::decoder(min_code_size).expect("decoder");
    let decoded = dec.read(&compressed).expect("decode");
    assert_eq!(decoded.as_slice(), input, "decoded does not match input");
    compressed.len()
}

#[test]
fn rt_ascii() {
    let compressed_len = roundtrip(8, b"TOBEORNOTTOBEORTOBEORNOT");
    assert!(compressed_len > 0);
}

#[test]
fn rt_random_short() {
    let mut s: u32 = 0xC0FFEE;
    let mut buf = Vec::with_capacity(128);
    for _ in 0..128 {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        buf.push(((s >> 24) & 0xFF) as u8);
    }
    roundtrip(8, &buf);
}

#[test]
fn rt_random_64k() {
    let mut s: u32 = 0xF00DF00D;
    let mut buf = Vec::with_capacity(65_536);
    for _ in 0..65_536 {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
        buf.push(((s >> 16) & 0xFF) as u8);
    }
    let clen = roundtrip(8, &buf);
    // Pseudo-random data doesn't compress; just assert plausibility.
    assert!(clen > buf.len() / 2);
}

#[test]
fn rt_small_alphabet() {
    // 4-colour alphabet (min_code_size = 2) — this exercises the very
    // bottom of the code-width ladder, which is where off-by-one bugs
    // love to live.
    let input: Vec<u8> = (0..10_000).map(|i| (i % 4) as u8).collect();
    roundtrip(2, &input);
}

#[test]
fn rt_repeat_then_noise() {
    // First half: highly compressible. Second half: pseudo-random.
    let mut buf = Vec::new();
    buf.extend(std::iter::repeat(0xAAu8).take(8192));
    let mut s: u32 = 0xBADC0DE;
    for _ in 0..8192 {
        s = s.wrapping_mul(22_695_477).wrapping_add(1);
        buf.push(((s >> 16) & 0xFF) as u8);
    }
    roundtrip(8, &buf);
}
