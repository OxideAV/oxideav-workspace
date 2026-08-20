//! Monkey's Audio through the framework registry: `'MAC '`
//! payload-magic resolution → whole-file framework decode → PCM.
//!
//! `oxideav-ape` is a complete decoder for v3.93+ files and registers
//! with the framework (codec id `ape`, `'MAC '` payload-magic claim,
//! whole-file `FrameworkDecoder`). This suite drives that wiring from
//! the consumer side: a caller holding nothing but the leading file
//! bytes resolves the codec through the core registry's payload-magic
//! index and decodes through `first_decoder`. (`oxideav-meta` does
//! not yet expose an `ape` feature, so registration goes through the
//! crate's own `register` entry rather than `register_all`.)
//!
//! The fixture is synthesized in-process from the crate's own public
//! encode mirrors (predictor-chain inverse, residual entropy encoder,
//! and header writers) — the same self-consistency shape the ape
//! crate's `synthetic_roundtrip` suite pins — so no binary fixture is
//! checked in.

use oxideav_ape::entropy::ResidualEncoder;
use oxideav_ape::frame::{crc32, FRAME_ENTROPY_INIT, FRAME_PRIME_PAD_BYTES};
use oxideav_ape::header::CompressionLevel;
use oxideav_ape::pcm::{interleave_pcm_bytes, pcm_to_coded_arrays};
use oxideav_core::{CodecParameters, Error as CoreError, Frame, Packet, TimeBase};

const VERSION: u16 = 3990;
const SAMPLE_RATE: u32 = 44100;

/// Deterministic xorshift noise in `[-bound, bound)`.
fn noise(seed: u64, len: usize, bound: i32) -> Vec<i32> {
    let mut s = seed | 1;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s >> 33) as i32) % (2 * bound) - bound
        })
        .collect()
}

/// Reshape a logical byte stream into the on-disk §4.1 layout: the
/// audio region is addressed as little-endian 32-bit words consumed
/// MSB-first — a per-word byte reversal over the whole region.
fn to_le_word_layout(mut logical: Vec<u8>) -> Vec<u8> {
    while logical.len() % 4 != 0 {
        logical.push(0);
    }
    for chunk in logical.chunks_mut(4) {
        chunk.reverse();
    }
    logical
}

/// Entropy-code one frame's coded arrays exactly as the frame layer
/// decodes them: one shared coder, per-sample channel interleave with
/// independent per-channel running states, per-frame init.
fn entropy_code_frame(arrays: &[Vec<i32>]) -> Vec<u8> {
    let mut enc = ResidualEncoder::new(VERSION, FRAME_ENTROPY_INIT);
    if arrays.len() == 2 {
        let mut states = [FRAME_ENTROPY_INIT; 2];
        for i in 0..arrays[0].len() {
            for (ch, arr) in arrays.iter().enumerate() {
                enc.reset_state(states[ch]);
                enc.encode_residual(arr[i]).unwrap();
                states[ch] = enc.running_state();
            }
        }
    } else {
        for &r in &arrays[0] {
            enc.encode_residual(r).unwrap();
        }
    }
    enc.finish()
}

/// One frame's **logical** byte stream: the 31-bit stored CRC word
/// (flags marker clear), the structural pad byte, then the
/// range-coded payload.
fn build_frame_logical(level: CompressionLevel, pcm: &[Vec<i32>]) -> Vec<u8> {
    let pcm_bytes = interleave_pcm_bytes(pcm, 16).unwrap();
    let coded = pcm_to_coded_arrays(pcm, VERSION, level).unwrap();
    let mut logical = Vec::new();
    logical.extend_from_slice(&(crc32(&pcm_bytes) >> 1).to_be_bytes());
    logical.extend_from_slice(&[0u8; FRAME_PRIME_PAD_BYTES]);
    logical.extend_from_slice(&entropy_code_frame(&coded));
    logical
}

/// Assemble a complete new-era (v3980+) file: §1.1 descriptor (zeroed
/// MD5), §1.2 header with explicit blocks-per-frame and bit depth,
/// seek table, no WAV blob, then the frame payload in §4.1 word
/// layout. 16-bit only.
fn build_ape_file(
    level: CompressionLevel,
    blocks_per_frame: u32,
    frames: &[&[Vec<i32>]],
) -> Vec<u8> {
    let channels = frames[0].len() as u16;
    let seek_bytes = 4 * frames.len() as u32;
    let audio_start = 52 + 24 + seek_bytes;
    let mut logical = Vec::new();
    let mut offsets = Vec::new();
    for pcm in frames {
        offsets.push(audio_start + logical.len() as u32);
        logical.extend_from_slice(&build_frame_logical(level, pcm));
    }
    let payload = to_le_word_layout(logical);
    let mut file = Vec::new();
    file.extend_from_slice(b"MAC ");
    file.extend_from_slice(&VERSION.to_le_bytes());
    file.extend_from_slice(&[0u8; 2]); // §1.1 alignment gap
    file.extend_from_slice(&52u32.to_le_bytes());
    file.extend_from_slice(&24u32.to_le_bytes());
    file.extend_from_slice(&seek_bytes.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes()); // WAV header blob
    file.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes()); // high 32 bits
    file.extend_from_slice(&0u32.to_le_bytes()); // terminating blob
    file.extend_from_slice(&[0u8; 16]); // MD5 (unverified on parse)
    file.extend_from_slice(&u16::from(level).to_le_bytes());
    file.extend_from_slice(&0u16.to_le_bytes()); // format flags
    file.extend_from_slice(&blocks_per_frame.to_le_bytes());
    file.extend_from_slice(&(frames.last().unwrap()[0].len() as u32).to_le_bytes());
    file.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    file.extend_from_slice(&16u16.to_le_bytes());
    file.extend_from_slice(&channels.to_le_bytes());
    file.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    for off in offsets {
        file.extend_from_slice(&off.to_le_bytes());
    }
    assert_eq!(file.len() as u32, audio_start);
    file.extend_from_slice(&payload);
    file
}

/// A two-frame 16-bit stereo fixture with correlated channels (so the
/// X and Y decorrelation arms both stay active).
fn fixture() -> (Vec<u8>, Vec<u8>, usize, usize) {
    let blocks = 1024usize;
    let tail = 400usize;
    let make = |seed: u64, len: usize| -> Vec<Vec<i32>> {
        let ch0 = noise(seed ^ 0xA5, len, 12000);
        let ch1: Vec<i32> = ch0
            .iter()
            .zip(noise(seed ^ 0x5A, len, 2500))
            .map(|(&a, b)| (a + b).clamp(-32768, 32767))
            .collect();
        vec![ch0, ch1]
    };
    let f0 = make(0x449, blocks);
    let f1 = make(0x1449, tail);
    let file = build_ape_file(CompressionLevel::Normal, blocks as u32, &[&f0, &f1]);
    let mut expected = interleave_pcm_bytes(&f0, 16).unwrap();
    expected.extend_from_slice(&interleave_pcm_bytes(&f1, 16).unwrap());
    (file, expected, blocks, tail)
}

/// The registry resolves the `'MAC '` payload magic to the `ape`
/// codec id.
#[test]
fn registry_resolves_mac_magic() {
    let mut ctx = oxideav_core::RuntimeContext::new();
    oxideav_ape::register(&mut ctx);
    let (file, _, _, _) = fixture();
    let id = ctx.codecs.resolve_payload_magic_ref(&file);
    assert_eq!(
        id.map(|i| i.as_str()),
        Some("ape"),
        "'MAC ' magic must resolve through the registry"
    );
    // Truncations below the 4-byte magic resolve nothing.
    assert!(ctx.codecs.resolve_payload_magic_ref(b"MAC").is_none());
}

/// Whole-file framework decode: magic → codec id → `first_decoder`
/// → byte-exact PCM (every frame's stored CRC verified inside the
/// decoder).
#[test]
fn magic_resolved_framework_decode_to_pcm() {
    let mut ctx = oxideav_core::RuntimeContext::new();
    oxideav_ape::register(&mut ctx);
    let (file, expected, blocks, tail) = fixture();

    let id = ctx
        .codecs
        .resolve_payload_magic_ref(&file)
        .expect("magic resolves")
        .clone();
    let params = CodecParameters::audio(id);
    let mut dec = ctx.codecs.first_decoder(&params).expect("make ape decoder");

    // Feed the file split across two packets to exercise accumulation.
    let mid = file.len() / 2;
    let tb = TimeBase::new(1, SAMPLE_RATE as i64);
    dec.send_packet(&Packet::new(0, tb, file[..mid].to_vec()))
        .expect("send head");
    dec.send_packet(&Packet::new(0, tb, file[mid..].to_vec()))
        .expect("send tail");
    dec.flush().expect("flush");

    let mut pcm = Vec::new();
    let mut counts = Vec::new();
    let mut pts = Vec::new();
    loop {
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                counts.push(a.samples as usize);
                pts.push(a.pts);
                assert_eq!(a.data.len(), 1, "interleaved: one plane");
                pcm.extend_from_slice(&a.data[0]);
            }
            Ok(other) => panic!("expected audio frames, got {other:?}"),
            Err(CoreError::Eof) => break,
            Err(e) => panic!("decode error: {e:?}"),
        }
    }
    assert_eq!(counts, vec![blocks, tail], "per-frame block counts");
    assert_eq!(pts, vec![Some(0), Some(blocks as i64)], "running block pts");
    assert_eq!(pcm, expected, "decoded PCM must be byte-exact");
}
