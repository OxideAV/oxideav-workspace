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
