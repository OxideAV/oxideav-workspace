//! amv 0.0.10 through the framework: registry probe → demux → the
//! registered `amv_video` / `adpcm_amv` decoders on both staged
//! device-profile fixtures, then a registry-encoder → `amv` muxer →
//! demux → decode round trip.
//!
//! Surface pinned here (amv 0.0.10): the demuxer declares stream 0 as
//! `amv_video` (Yuv420P after decode) and stream 1 as `adpcm_amv` with
//! `SampleFormat::S16` — audio frames arrive as one interleaved plane
//! of little-endian 16-bit bytes (`AudioFrame::data[0]`), not as a
//! typed `i16` vector.
//!
//! Skips when `docs/container/amv/fixtures/` is not staged.

use std::path::{Path, PathBuf};

use oxideav_core::{
    CodecParameters, Error, Frame, MediaType, Packet, PixelFormat, ReadSeek, RuntimeContext,
    SampleFormat, StreamInfo, WriteSeek,
};

/// (file, width, height, fps, frames)
const PROFILES: &[(&str, u32, u32, i64, usize)] = &[
    ("comedian.amv", 128, 96, 12, 1116),
    ("noel-son-lumiere.amv", 96, 64, 16, 2928),
];

fn fixture(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../docs/container/amv/fixtures/{name}"));
    p.exists().then_some(p)
}

fn ctx() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_amv::register(&mut ctx);
    ctx
}

struct Demuxed {
    streams: Vec<StreamInfo>,
    packets: Vec<Packet>,
}

fn demux(ctx: &RuntimeContext, bytes: &[u8]) -> Demuxed {
    let mut rs: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(bytes.to_vec()));
    let format = ctx
        .containers
        .probe_input(&mut *rs, Some("amv"))
        .expect("probe amv");
    assert_eq!(format, "amv");
    let mut dmx = ctx
        .containers
        .open_demuxer(&format, rs, &oxideav_core::NullCodecResolver)
        .expect("open amv demuxer");
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

fn decode_video(ctx: &RuntimeContext, params: &CodecParameters, pkt: &Packet) -> Vec<Vec<u8>> {
    let mut dec = ctx.codecs.first_decoder(params).expect("amv_video decoder");
    dec.send_packet(pkt).expect("send video");
    let Frame::Video(v) = dec.receive_frame().expect("video frame") else {
        panic!("expected video frame");
    };
    v.planes.iter().map(|p| p.data.clone()).collect()
}

fn decode_audio(ctx: &RuntimeContext, params: &CodecParameters, pkt: &Packet) -> Vec<u8> {
    let mut dec = ctx.codecs.first_decoder(params).expect("adpcm_amv decoder");
    dec.send_packet(pkt).expect("send audio");
    let Frame::Audio(a) = dec.receive_frame().expect("audio frame") else {
        panic!("expected audio frame");
    };
    assert_eq!(a.data.len(), 1, "one interleaved S16 plane");
    assert_eq!(a.data[0].len(), a.samples as usize * 2, "S16 mono bytes");
    a.data[0].clone()
}

/// Both device profiles: probe, stream declarations, packet counts,
/// and every packet decodes through the registry factories with the
/// declared geometry / sample layout.
#[test]
fn both_profiles_demux_and_decode_through_registry() {
    let ctx = ctx();
    let mut seen = 0;
    for &(name, w, h, fps, frames) in PROFILES {
        let Some(path) = fixture(name) else {
            eprintln!("skipping {name}: not staged");
            continue;
        };
        let bytes = std::fs::read(path).unwrap();
        let d = demux(&ctx, &bytes);
        assert_eq!(d.streams.len(), 2, "{name}: video + audio");
        let vs = &d.streams[0].params;
        let as_ = &d.streams[1].params;
        assert_eq!(vs.media_type, MediaType::Video);
        assert_eq!(vs.codec_id.as_str(), "amv_video", "{name}");
        assert_eq!((vs.width, vs.height), (Some(w), Some(h)), "{name}");
        assert_eq!(d.streams[0].time_base.den(), fps, "{name}: fps time base");
        assert_eq!(as_.media_type, MediaType::Audio);
        assert_eq!(as_.codec_id.as_str(), "adpcm_amv", "{name}");
        assert_eq!(as_.sample_rate, Some(22_050), "{name}");
        assert_eq!(as_.channels, Some(1), "{name}");
        assert_eq!(as_.sample_format, Some(SampleFormat::S16), "{name}");

        let video: Vec<&Packet> = d.packets.iter().filter(|p| p.stream_index == 0).collect();
        let audio: Vec<&Packet> = d.packets.iter().filter(|p| p.stream_index == 1).collect();
        assert_eq!(video.len(), frames, "{name}: video packets");
        assert_eq!(audio.len(), frames, "{name}: audio blocks");

        let (cw, ch) = (w.div_ceil(2) as usize, h.div_ceil(2) as usize);
        let samples_per_block = (22_050 / fps) as usize;
        let mut total_samples = 0usize;
        for (i, (vp, ap)) in video.iter().zip(&audio).enumerate() {
            // Video: every 25th frame plus the ends (the whole-corpus
            // sweep lives in the amv crate; this pins the framework path).
            if i % 25 == 0 || i + 1 == frames {
                let planes = decode_video(&ctx, vs, vp);
                assert_eq!(planes.len(), 3, "{name}#{i}: Yuv420P planes");
                assert_eq!(planes[0].len(), (w * h) as usize, "{name}#{i}: Y");
                assert_eq!(planes[1].len(), cw * ch, "{name}#{i}: Cb");
                assert_eq!(planes[2].len(), cw * ch, "{name}#{i}: Cr");
            }
            let pcm = decode_audio(&ctx, as_, ap);
            total_samples += pcm.len() / 2;
            assert!(
                pcm.len() / 2 >= samples_per_block.saturating_sub(64),
                "{name}#{i}: block carries ~{samples_per_block} samples, got {}",
                pcm.len() / 2
            );
        }
        assert!(
            total_samples >= frames * (samples_per_block - 64),
            "{name}: total PCM"
        );
        seen += 1;
    }
    assert!(seen > 0 || fixture("comedian.amv").is_none());
}

/// Registry encoders → `amv` muxer → registry demux → registry decoders.
/// The first 48 frames of each profile round-trip with a bounded
/// per-sample error (both codecs are lossy), and the container keeps
/// the strict 1:1 video:audio interleave.
#[test]
fn registry_encode_mux_demux_decode_round_trip() {
    let ctx = ctx();
    for &(name, w, h, fps, _) in PROFILES {
        let Some(path) = fixture(name) else {
            eprintln!("skipping {name}: not staged");
            continue;
        };
        let bytes = std::fs::read(path).unwrap();
        let src = demux(&ctx, &bytes);
        let vparams = src.streams[0].params.clone();
        let aparams = src.streams[1].params.clone();
        let take = 48usize;

        // Decode the source media through the registry.
        let mut frames = Vec::new();
        let mut blocks = Vec::new();
        for p in &src.packets {
            if p.stream_index == 0 && frames.len() < take {
                frames.push(decode_video(&ctx, &vparams, p));
            } else if p.stream_index == 1 && blocks.len() < take {
                blocks.push(decode_audio(&ctx, &aparams, p));
            }
        }
        assert_eq!(frames.len(), take);
        assert_eq!(blocks.len(), take);

        // Re-encode through the registry encoders.
        let mut venc_params = CodecParameters::video(oxideav_core::CodecId::new("amv_video"));
        venc_params.width = Some(w);
        venc_params.height = Some(h);
        venc_params.pixel_format = Some(PixelFormat::Yuv420P);
        venc_params.frame_rate = Some(oxideav_core::Rational::new(fps, 1));
        let mut venc = ctx
            .codecs
            .first_encoder(&venc_params)
            .expect("amv_video encoder");
        let mut aenc_params = CodecParameters::audio(oxideav_core::CodecId::new("adpcm_amv"));
        aenc_params.sample_rate = Some(22_050);
        aenc_params.channels = Some(1);
        aenc_params.sample_format = Some(SampleFormat::S16);
        let mut aenc = ctx
            .codecs
            .first_encoder(&aenc_params)
            .expect("adpcm_amv encoder");

        let (cw, ch) = (w.div_ceil(2) as usize, h.div_ceil(2) as usize);
        let mut vpk = Vec::new();
        let mut apk = Vec::new();
        for (i, (planes, pcm)) in frames.iter().zip(&blocks).enumerate() {
            let vf = oxideav_core::VideoFrame {
                pts: Some(i as i64),
                planes: vec![
                    oxideav_core::VideoPlane {
                        stride: w as usize,
                        data: planes[0].clone(),
                    },
                    oxideav_core::VideoPlane {
                        stride: cw,
                        data: planes[1].clone(),
                    },
                    oxideav_core::VideoPlane {
                        stride: cw,
                        data: planes[2].clone(),
                    },
                ],
            };
            let _ = ch;
            venc.send_frame(&Frame::Video(vf))
                .expect("send video frame");
            vpk.push(venc.receive_packet().expect("video packet"));
            let af = oxideav_core::AudioFrame {
                samples: (pcm.len() / 2) as u32,
                pts: Some((i as i64) * (22_050 / fps)),
                data: vec![pcm.clone()],
            };
            aenc.send_frame(&Frame::Audio(af))
                .expect("send audio frame");
            apk.push(aenc.receive_packet().expect("audio packet"));
        }

        // Mux with the demuxer-declared stream infos (video needs an
        // integer frame_rate for the amvh body).
        let mut streams = src.streams.clone();
        streams[0].params.frame_rate = Some(oxideav_core::Rational::new(fps, 1));
        let out = {
            let path = oxideav_tests::tmp(&format!("oxideav-amv-r452-{name}"));
            {
                let f = std::fs::File::create(&path).expect("create amv");
                let ws: Box<dyn WriteSeek> = Box::new(f);
                let mut mux = ctx
                    .containers
                    .open_muxer("amv", ws, &streams)
                    .expect("open amv muxer");
                mux.write_header().expect("header");
                for (v, a) in vpk.iter().zip(&apk) {
                    let mut v = v.clone();
                    v.stream_index = 0;
                    let mut a = a.clone();
                    a.stream_index = 1;
                    mux.write_packet(&v).expect("write video");
                    mux.write_packet(&a).expect("write audio");
                }
                mux.write_trailer().expect("trailer");
            }
            let bytes = std::fs::read(&path).expect("read amv");
            let _ = std::fs::remove_file(&path);
            bytes
        };
        assert_eq!(&out[..4], b"RIFF", "{name}");
        assert_eq!(&out[8..12], b"AMV ", "{name}");

        // Demux + decode the re-muxed file and compare.
        let rt = demux(&ctx, &out);
        assert_eq!(rt.streams[0].params.width, Some(w), "{name}");
        assert_eq!(rt.streams[0].params.height, Some(h), "{name}");
        assert_eq!(rt.streams[0].time_base.den(), fps, "{name}");
        let rv: Vec<&Packet> = rt.packets.iter().filter(|p| p.stream_index == 0).collect();
        let ra: Vec<&Packet> = rt.packets.iter().filter(|p| p.stream_index == 1).collect();
        assert_eq!(rv.len(), take, "{name}: video packets after remux");
        assert_eq!(ra.len(), take, "{name}: audio packets after remux");
        // Strict 1:1 interleave: video, audio, video, audio, ...
        for (i, p) in rt.packets.iter().enumerate() {
            assert_eq!(p.stream_index as usize, i % 2, "{name}: interleave at {i}");
        }

        let mut vsum = 0f64;
        let mut vn = 0u64;
        for (i, (p, orig)) in rv.iter().zip(&frames).enumerate() {
            let planes = decode_video(&ctx, &rt.streams[0].params, p);
            for (a, b) in planes.iter().zip(orig) {
                assert_eq!(a.len(), b.len(), "{name}#{i}: plane length");
                for (&x, &y) in a.iter().zip(b) {
                    vsum += x.abs_diff(y) as f64;
                    vn += 1;
                }
            }
        }
        let vmae = vsum / vn as f64;
        assert!(
            vmae < 4.0,
            "{name}: video MAE {vmae} after registry re-encode"
        );

        let mut asum = 0f64;
        let mut an = 0u64;
        for (i, (p, orig)) in ra.iter().zip(&blocks).enumerate() {
            let pcm = decode_audio(&ctx, &rt.streams[1].params, p);
            assert_eq!(pcm.len(), orig.len(), "{name}#{i}: block length");
            for (x, y) in pcm.chunks_exact(2).zip(orig.chunks_exact(2)) {
                let sx = i16::from_le_bytes([x[0], x[1]]) as f64;
                let sy = i16::from_le_bytes([y[0], y[1]]) as f64;
                asum += (sx - sy).abs();
                an += 1;
            }
        }
        let amae = asum / an as f64;
        assert!(
            amae < 400.0,
            "{name}: audio MAE {amae} after registry re-encode"
        );
    }
}
