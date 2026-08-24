//! MPEG-TS remux round-trip through the public mux/demux surfaces.
//!
//! `oxideav-mpegts` registers a `"mpegts"` demuxer + single-program
//! muxer with the container registry. This suite drives a full
//! mux → probe → demux → REMUX → demux cycle on the registry path
//! and pins:
//!
//! * codec-id round-tripping (the muxer's CodecId → `stream_type`
//!   map mirrors the demuxer's, so a demuxed stream re-muxes as-is),
//! * per-stream payload byte-exactness across both generations,
//! * PTS carriage through the 33-bit PES timestamps,
//! * §2.4.3.5 `random_access_indicator` → core keyframe flag,
//! * §13818-1 conformance of everything we emit
//!   (`oxideav_mpegts::validate::validate_ts`).
//!
//! The oracle leg feeds an ffmpeg-authored TS (mp2 audio) through
//! our demuxer and the `oxideav-mp2` decoder, comparing PCM against
//! ffmpeg's own decode of the same stream.
//!
//! Round 451 additions over the r450 mpegts surface:
//!
//! * an all-ours mp2 chain (encode → mux → demux → decode) over the
//!   muxer's completed `stream_type` map,
//! * id round-trips for the newly mapped codec ids (mp1/mp3 → 0x03,
//!   aac 0x0F, mpeg1video, mpeg4, jpeg2000),
//! * the typed `ViolationCounts::pcr_extension_out_of_range` tally
//!   on a hostile PCR stream (§2.4.3.4 bounds the extension at 299;
//!   the 9-bit wire field can carry up to 511).

use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, ReadSeek, StreamInfo, TimeBase, WriteSeek,
};
use oxideav_tests::*;

const TB: TimeBase = TimeBase::new(1, 90_000);

fn registry() -> oxideav_core::RuntimeContext {
    let mut ctx = oxideav_core::RuntimeContext::new();
    oxideav_mpegts::registry::register(&mut ctx);
    ctx
}

/// Deterministic payload bytes for stream `s`, packet `i` — sized to
/// exercise single-TS-packet, multi-packet, and stuffing-tail paths.
fn payload(s: u32, i: usize) -> Vec<u8> {
    let len = match i % 4 {
        0 => 40,           // shorter than one TS payload
        1 => 184,          // exactly one payload
        2 => 1000,         // several packets
        _ => 4096 + i * 7, // large, forces fragmentation
    };
    (0..len)
        .map(|k| ((k * 31 + i * 17 + s as usize * 7) % 251) as u8)
        .collect()
}

fn stream_info(idx: u32, codec: &str, video: bool) -> StreamInfo {
    let params = if video {
        CodecParameters::video(CodecId::new(codec))
    } else {
        CodecParameters::audio(CodecId::new(codec))
    };
    StreamInfo {
        index: idx,
        time_base: TB,
        duration: None,
        start_time: None,
        params,
    }
}

/// The synthetic two-stream program: "h264"-labelled video with a
/// keyframe every 4th frame, "ac3"-labelled audio. (The payloads are
/// opaque to the muxer — this exercises carriage, not codecs.)
fn source_packets() -> Vec<Packet> {
    let mut pkts = Vec::new();
    for i in 0..24usize {
        let pts = 3_000 * i as i64; // 30 fps at 90 kHz
        pkts.push(
            Packet::new(0, TB, payload(0, i))
                .with_pts(pts)
                .with_dts(pts)
                .with_keyframe(i % 4 == 0),
        );
        let apts = 1_920 * i as i64; // ~46.9 packets/s audio cadence
        pkts.push(Packet::new(1, TB, payload(1, i)).with_pts(apts));
    }
    pkts
}

/// Mux packets through the registry's "mpegts" muxer.
fn mux(tag: &str, streams: &[StreamInfo], packets: &[Packet]) -> Vec<u8> {
    let ctx = registry();
    let path = tmp(&format!("oxideav-mpegts-{tag}.ts"));
    {
        let f = std::fs::File::create(&path).expect("create ts");
        let ws: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = ctx
            .containers
            .open_muxer("mpegts", ws, streams)
            .expect("open mpegts muxer");
        mux.write_header().expect("write header");
        for p in packets {
            mux.write_packet(p).expect("write packet");
        }
        mux.write_trailer().expect("write trailer");
    }
    std::fs::read(&path).expect("read ts")
}

/// Demuxed view of a TS: per-stream codec ids and packets.
struct Demuxed {
    streams: Vec<StreamInfo>,
    packets: Vec<Packet>,
}

fn demux(data: &[u8]) -> Demuxed {
    let ctx = registry();
    let mut file: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(data.to_vec()));
    let format = ctx
        .containers
        .probe_input(&mut *file, Some("ts"))
        .expect("probe ts");
    assert_eq!(format, "mpegts", "content probe must detect MPEG-TS");
    let mut dmx = ctx
        .containers
        .open_demuxer(&format, file, &oxideav_core::NullCodecResolver)
        .expect("open mpegts demuxer");
    let streams = dmx.streams().to_vec();
    let mut packets = Vec::new();
    loop {
        match dmx.next_packet() {
            Ok(p) => packets.push(p),
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {e:?}"),
        }
    }
    Demuxed { streams, packets }
}

fn per_stream(packets: &[Packet], idx: u32) -> Vec<&Packet> {
    packets.iter().filter(|p| p.stream_index == idx).collect()
}

/// mux → demux → remux → demux: codec set, payloads, PTS, and
/// keyframes all survive both generations.
#[test]
fn remux_round_trip_preserves_streams() {
    let streams = [stream_info(0, "h264", true), stream_info(1, "ac3", false)];
    let source = source_packets();

    // Generation 1.
    let ts1 = mux("gen1", &streams, &source);
    assert_eq!(ts1.len() % 188, 0, "whole 188-byte packets only");
    let d1 = demux(&ts1);
    assert_eq!(d1.streams.len(), 2);
    let ids: Vec<&str> = d1
        .streams
        .iter()
        .map(|s| s.params.codec_id.as_str())
        .collect();
    assert!(ids.contains(&"h264") && ids.contains(&"ac3"), "{ids:?}");
    let vid = d1
        .streams
        .iter()
        .find(|s| s.params.codec_id.as_str() == "h264")
        .unwrap()
        .index;
    let aud = d1
        .streams
        .iter()
        .find(|s| s.params.codec_id.as_str() == "ac3")
        .unwrap()
        .index;

    // Payload + timing checks against the source, per stream.
    for (src_idx, dem_idx) in [(0u32, vid), (1u32, aud)] {
        let want: Vec<&Packet> = per_stream(&source, src_idx);
        let got = per_stream(&d1.packets, dem_idx);
        assert_eq!(got.len(), want.len(), "stream {src_idx} packet count");
        for (k, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(g.data, w.data, "stream {src_idx} packet {k} payload");
            assert_eq!(g.pts, w.pts, "stream {src_idx} packet {k} pts");
        }
    }
    // §2.4.3.5 random-access indicators come back as keyframes.
    let kf: Vec<usize> = per_stream(&d1.packets, vid)
        .iter()
        .enumerate()
        .filter(|(_, p)| p.flags.keyframe)
        .map(|(k, _)| k)
        .collect();
    assert_eq!(kf, vec![0, 4, 8, 12, 16, 20], "keyframe cadence");

    // Generation 2: remux the demuxed streams/packets verbatim.
    let ts2 = mux("gen2", &d1.streams, &d1.packets);
    let d2 = demux(&ts2);
    for idx in [vid, aud] {
        let want = per_stream(&d1.packets, idx);
        let got = per_stream(&d2.packets, idx);
        assert_eq!(got.len(), want.len(), "gen2 stream {idx} packet count");
        for (k, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(g.data, w.data, "gen2 stream {idx} packet {k} payload");
            assert_eq!(g.pts, w.pts, "gen2 stream {idx} packet {k} pts");
            assert_eq!(
                g.flags.keyframe, w.flags.keyframe,
                "gen2 stream {idx} packet {k} keyframe"
            );
        }
    }
}

/// Everything the muxer emits is pinned conformant against the crate's
/// own §13818-1 validation walk — on both generations.
#[test]
fn muxed_output_is_conformant() {
    let streams = [stream_info(0, "h264", true), stream_info(1, "ac3", false)];
    let source = source_packets();
    let ts1 = mux("val1", &streams, &source);
    let report = oxideav_mpegts::validate::validate_ts(&ts1);
    assert!(
        report.is_conformant(),
        "generation-1 mux not conformant: {report:?}"
    );

    let d1 = demux(&ts1);
    let ts2 = mux("val2", &d1.streams, &d1.packets);
    let report2 = oxideav_mpegts::validate::validate_ts(&ts2);
    assert!(
        report2.is_conformant(),
        "generation-2 remux not conformant: {report2:?}"
    );
}

/// Oracle leg: an ffmpeg-authored TS carrying mp2 audio, demuxed by
/// us, decoded by `oxideav-mp2`, compared against ffmpeg's own decode.
#[test]
fn ffmpeg_ts_demux_and_mp2_decode() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let sample_rate = 48000u32;
    let channels = 2u16;
    let pcm = generate_audio_signal(sample_rate, channels, 2.0);
    let raw_path = tmp("oxideav-mpegts-mp2-in.raw");
    write_pcm_s16le(&raw_path, &pcm);

    let ts_path = tmp("oxideav-mpegts-mp2.ts");
    assert!(
        ffmpeg(&[
            "-f",
            "s16le",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            &channels.to_string(),
            "-i",
            raw_path.to_str().unwrap(),
            "-c:a",
            "mp2",
            "-b:a",
            "192k",
            "-f",
            "mpegts",
            ts_path.to_str().unwrap(),
        ]),
        "ffmpeg mpegts authoring failed"
    );

    // ffmpeg's own decode of the TS (the fidelity reference).
    let ref_path = tmp("oxideav-mpegts-mp2-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            ts_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            &channels.to_string(),
            ref_path.to_str().unwrap(),
        ]),
        "ffmpeg decode failed"
    );
    let reference = read_pcm_s16le(&ref_path);

    // Our chain: registry demux → re-frame the PES payloads on the
    // §2.4.3.1 Layer II sync grid (the framework mp2 decoder rides a
    // one-frame-per-packet interface) → mp2 framework decode.
    let ts = std::fs::read(&ts_path).expect("read ts");
    let d = demux(&ts);
    let astream = d
        .streams
        .iter()
        .find(|s| s.params.codec_id.as_str() == "mp2")
        .expect("TS must surface an mp2 stream");
    let mut es = Vec::new();
    for p in per_stream(&d.packets, astream.index) {
        es.extend_from_slice(&p.data);
    }
    let frames = {
        use oxideav_mp2::header::{find_sync, FrameHeader};
        let mut frames = Vec::new();
        let mut cursor = find_sync(&es).expect("Layer II sync in the demuxed ES");
        while cursor + 4 <= es.len() {
            let Ok(header) = FrameHeader::parse(&es[cursor..cursor + 4]) else {
                match find_sync(&es[cursor + 1..]) {
                    Some(off) => {
                        cursor += 1 + off;
                        continue;
                    }
                    None => break,
                }
            };
            let len = header.frame_size_bytes();
            if len == 0 || cursor + len > es.len() {
                break;
            }
            frames.push(es[cursor..cursor + len].to_vec());
            cursor += len;
        }
        frames
    };
    assert!(frames.len() > 50, "expected many Layer II frames in 2 s");

    let mut params = astream.params.clone();
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    let mut dec = oxideav_mp2::codec_decoder::make_decoder(&params).expect("make mp2 decoder");
    let tb = TimeBase::new(1, sample_rate as i64);
    let mut ours = Vec::new();
    for f in &frames {
        dec.send_packet(&Packet::new(0, tb, f.clone()))
            .expect("send");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(a)) => {
                    // Planar S16 planes → interleaved.
                    let per_ch: Vec<Vec<i16>> = a
                        .data
                        .iter()
                        .map(|p| {
                            p.chunks_exact(2)
                                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                .collect()
                        })
                        .collect();
                    let n = per_ch.iter().map(Vec::len).min().unwrap_or(0);
                    for i in 0..n {
                        for ch in &per_ch {
                            ours.push(ch[i]);
                        }
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("mp2 decode error: {e:?}"),
            }
        }
    }
    assert!(!ours.is_empty(), "our chain produced no samples");

    let (rms, lag) = audio_rms_diff_aligned(&reference, &ours, channels, 8192);
    let psnr = audio_psnr(&reference, &ours[lag..]);
    eprintln!("=== MPEG-TS demux + mp2 decode vs ffmpeg ===");
    report("ts+mp2", rms, psnr, ours.len(), reference.len());
    assert!(rms < 0.05, "TS→mp2 chain RMS {rms:.6} too large (> 0.05)");
}

// ══════════════════ round-451 legs: stream_type map + hostile PCR ═══════════

/// All-ours mp2 chain over the muxer's completed `stream_type` map:
/// our Layer II encode → mpegts mux (`mp2` → stream_type 0x03) →
/// registry demux (0x03 → `mp2`) → our framework decode, compared
/// against the input signal. No oracle required.
#[test]
fn mp2_mux_demux_decode_round_trip() {
    let sample_rate = 48_000u32;
    let channels = 2u16;
    let pcm = generate_audio_signal(sample_rate, channels, 1.0);

    // Encode with our mp2 encoder (interleaved → planar frames).
    let mut params = CodecParameters::audio(CodecId::new("mp2"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    params.bit_rate = Some(192_000);
    let mut enc = oxideav_mp2::codec_encoder::make_encoder(&params).expect("make mp2 encoder");
    let nch = channels as usize;
    for chunk in pcm.chunks(1152 * nch) {
        let mut planes: Vec<Vec<u8>> = vec![Vec::with_capacity(chunk.len() / nch * 2); nch];
        for (i, s) in chunk.iter().enumerate() {
            planes[i % nch].extend_from_slice(&s.to_le_bytes());
        }
        let frame = oxideav_core::AudioFrame {
            samples: (chunk.len() / nch) as u32,
            pts: None,
            data: planes,
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send frame");
    }
    enc.flush().expect("flush");
    let mut frames: Vec<Vec<u8>> = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(p) => frames.push(p.data),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    assert!(frames.len() > 30, "expected many Layer II frames in 1 s");

    // Mux one Layer II frame per PES packet (1152 samples @ 48 kHz =
    // 2160 ticks of the 90 kHz mux time base).
    let packets: Vec<Packet> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            Packet::new(0, TB, f.clone())
                .with_pts(i as i64 * 2_160)
                .with_keyframe(true)
        })
        .collect();
    let streams = [stream_info(0, "mp2", false)];
    let ts = mux("mp2rt", &streams, &packets);
    let vreport = oxideav_mpegts::validate::validate_ts(&ts);
    assert!(
        vreport.is_conformant(),
        "mp2 mux not conformant: {vreport:?}"
    );

    // Demux: stream_type 0x03 resolves back to `mp2`, payloads exact.
    let d = demux(&ts);
    assert_eq!(d.streams.len(), 1);
    assert_eq!(d.streams[0].params.codec_id.as_str(), "mp2");
    let got = per_stream(&d.packets, d.streams[0].index);
    assert_eq!(got.len(), frames.len(), "one PES per Layer II frame");
    for (k, (g, w)) in got.iter().zip(frames.iter()).enumerate() {
        assert_eq!(&g.data, w, "frame {k} payload");
        assert_eq!(g.pts, Some(k as i64 * 2_160), "frame {k} pts");
    }

    // Decode the demuxed frames with the framework mp2 decoder.
    let mut dparams = d.streams[0].params.clone();
    dparams.sample_rate = Some(sample_rate);
    dparams.channels = Some(channels);
    let mut dec = oxideav_mp2::codec_decoder::make_decoder(&dparams).expect("make mp2 decoder");
    let tb = TimeBase::new(1, sample_rate as i64);
    let mut ours: Vec<i16> = Vec::new();
    for p in &got {
        dec.send_packet(&Packet::new(0, tb, p.data.clone()))
            .expect("send");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(a)) => {
                    let per_ch: Vec<Vec<i16>> = a
                        .data
                        .iter()
                        .map(|pl| {
                            pl.chunks_exact(2)
                                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                .collect()
                        })
                        .collect();
                    let n = per_ch.iter().map(Vec::len).min().unwrap_or(0);
                    for i in 0..n {
                        for ch in &per_ch {
                            ours.push(ch[i]);
                        }
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("mp2 decode error: {e:?}"),
            }
        }
    }
    assert!(!ours.is_empty(), "chain produced no samples");
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &ours, channels, 4096);
    let psnr = audio_psnr(&pcm, &ours[lag..]);
    eprintln!("=== MPEG-TS all-ours mp2 chain ===");
    report("mp2 chain", rms, psnr, ours.len(), pcm.len());
    assert!(rms < 0.1, "mp2 chain RMS {rms:.6} too large (> 0.1)");
}

/// The r450 muxer `stream_type` completions round-trip through PMT
/// carriage: each newly mapped codec id muxes, and the demuxer's
/// mirror map hands back the expected id with byte-exact payloads.
/// (The three ISO/IEC 11172-3 layers share stream_type 0x03, whose
/// layer is a bitstream property — they resolve to the `mp2` family
/// id at PMT level.)
#[test]
fn completed_stream_type_map_round_trips() {
    let cases: [(&str, bool, &str); 6] = [
        ("mp1", false, "mp2"),
        ("mp3", false, "mp2"),
        ("aac", false, "aac"),
        ("mpeg1video", true, "mpeg1video"),
        ("mpeg4", true, "mpeg4"),
        ("jpeg2000", true, "jpeg2000"),
    ];
    for (codec, video, expect) in cases {
        let streams = [stream_info(0, codec, video)];
        let packets: Vec<Packet> = (0..6usize)
            .map(|i| {
                Packet::new(0, TB, payload(0, i))
                    .with_pts(3_000 * i as i64)
                    .with_keyframe(true)
            })
            .collect();
        let ts = mux(&format!("st-{codec}"), &streams, &packets);
        let vreport = oxideav_mpegts::validate::validate_ts(&ts);
        assert!(
            vreport.is_conformant(),
            "{codec}: mux not conformant: {vreport:?}"
        );
        let d = demux(&ts);
        assert_eq!(d.streams.len(), 1, "{codec}");
        assert_eq!(
            d.streams[0].params.codec_id.as_str(),
            expect,
            "{codec} must round-trip to {expect}"
        );
        let got = per_stream(&d.packets, d.streams[0].index);
        assert_eq!(got.len(), packets.len(), "{codec} packet count");
        for (k, (g, w)) in got.iter().zip(packets.iter()).enumerate() {
            assert_eq!(g.data, w.data, "{codec} packet {k} payload");
            assert_eq!(g.pts, w.pts, "{codec} packet {k} pts");
        }
    }
}

/// §2.4.3.4 bounds `program_clock_reference_extension` at 0..=299;
/// the 9-bit wire field can carry up to 511. Forge every PCR in a
/// conformant mux up to 511 and pin the typed tally: one count per
/// hostile PCR, conformance verdict false, and the interval math
/// still total (no unrelated PCR violations appear).
#[test]
fn hostile_pcr_extension_is_counted_typed() {
    let streams = [stream_info(0, "h264", true), stream_info(1, "ac3", false)];
    let mut ts = mux("pcr-hostile", &streams, &source_packets());
    let clean = oxideav_mpegts::validate::validate_ts(&ts);
    assert!(clean.is_conformant(), "baseline must be conformant");
    assert_eq!(clean.violations.pcr_extension_out_of_range, 0);

    // Forge: adaptation-field PCR (6 bytes at packet offset 6) —
    // 33-bit base, 6 reserved bits, 9-bit extension. Setting the low
    // bit of byte 4 and all of byte 5 makes the extension 511.
    let mut forged = 0u64;
    for pkt in ts.chunks_exact_mut(188) {
        let has_af = pkt[3] & 0x20 != 0;
        if !has_af || pkt[4] < 7 {
            continue; // no adaptation field, or too short for a PCR
        }
        let pcr_flag = pkt[5] & 0x10 != 0;
        if !pcr_flag {
            continue;
        }
        pkt[10] |= 0x01;
        pkt[11] = 0xFF;
        forged += 1;
    }
    assert!(forged > 0, "the mux must have emitted PCRs");

    let hostile = oxideav_mpegts::validate::validate_ts(&ts);
    assert_eq!(
        hostile.violations.pcr_extension_out_of_range, forged,
        "one typed count per forged PCR"
    );
    assert!(!hostile.is_conformant(), "hostile stream must not pass");
    // The fold-into-modulus fix keeps the wrap delta total: forging
    // the extension alone must not surface interval/underflow noise.
    let mut v = hostile.violations;
    v.pcr_extension_out_of_range = 0;
    assert!(
        v.is_clean(),
        "only the extension tally may fire: {:?}",
        hostile.violations
    );
}
