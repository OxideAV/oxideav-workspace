//! Vorbis roundtrip comparison tests against ffmpeg.
//!
//! Round 445 restoration: `oxideav-vorbis` re-grew its encoder
//! (`oxideav_vorbis::encoder::make_encoder` — floor/residue/codebook
//! design, block switching, channel coupling) against the staged
//! Vorbis I specification, so the harness neutralized when the old
//! encoder factory disappeared comes back.
//!
//! The encode leg is a genuine cross-crate flow: our encoder's packet
//! stream (three `header`-flagged §4.2 packets, then audio packets
//! whose `pts` carry the granule position) is muxed through the
//! `oxideav-ogg` muxer via the container registry, and the resulting
//! physical stream is decoded both by ffmpeg and by our own registry
//! demux → decode path.

use oxideav_core::{
    AudioFrame, CodecId, CodecParameters, Error, Frame, ReadSeek, SampleFormat, StreamInfo,
    TimeBase, WriteSeek,
};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 44100;
const CHANNELS: u16 = 2;
const DURATION: f32 = 2.0;

/// Encode interleaved S16 PCM into a physical Ogg/Vorbis stream:
/// our Vorbis encoder + our Ogg muxer via the container registry.
fn encode_to_ogg_with_ours(pcm: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let mut params = CodecParameters::audio(CodecId::new("vorbis"));
    params.sample_rate = Some(sample_rate);
    params.channels = Some(channels);
    params.sample_format = Some(SampleFormat::F32P);
    let mut enc = oxideav_vorbis::encoder::make_encoder(&params).expect("make_encoder for vorbis");

    // De-interleave into planar f32 planes, feed in ~1024-sample hops.
    let nch = channels as usize;
    for chunk in pcm.chunks(1024 * nch) {
        let samples = chunk.len() / nch;
        let mut planes: Vec<Vec<u8>> = vec![Vec::with_capacity(samples * 4); nch];
        for (i, s) in chunk.iter().enumerate() {
            let v = *s as f32 / 32768.0;
            planes[i % nch].extend_from_slice(&v.to_le_bytes());
        }
        let frame = AudioFrame {
            samples: samples as u32,
            pts: None,
            data: planes,
        };
        enc.send_frame(&Frame::Audio(frame)).expect("send_frame");
    }
    enc.flush().expect("flush");

    let out_params = enc.output_params().clone();
    let mut packets = Vec::new();
    loop {
        match enc.receive_packet() {
            Ok(pkt) => packets.push(pkt),
            Err(Error::NeedMore | Error::Eof) => break,
            Err(e) => panic!("encode error: {e:?}"),
        }
    }
    assert!(
        packets.iter().filter(|p| p.is_header()).count() >= 3,
        "vorbis encoder must emit the three §4.2 header packets"
    );

    // Mux through the registry's Ogg muxer.
    let mux_path = tmp("oxideav-vorbis-mux-tmp.ogg");
    {
        let mut reg = oxideav_core::RuntimeContext::new();
        oxideav_ogg::register(&mut reg);
        let stream = StreamInfo {
            index: 0,
            time_base: TimeBase::new(1, sample_rate as i64),
            duration: None,
            start_time: Some(0),
            params: out_params,
        };
        let f = std::fs::File::create(&mux_path).expect("create mux file");
        let ws: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = reg
            .containers
            .open_muxer("ogg", ws, &[stream])
            .expect("open ogg muxer");
        mux.write_header().expect("write header");
        for pkt in &packets {
            mux.write_packet(pkt).expect("write packet");
        }
        mux.write_trailer().expect("write trailer");
    }
    std::fs::read(&mux_path).expect("read muxed ogg")
}

/// Decode an Ogg/Vorbis stream with our registry path (ogg demux →
/// vorbis decode). The decoder emits planar F32 planes; convert to
/// interleaved S16 for the shared metrics.
fn decode_with_ours(ogg_data: &[u8]) -> Vec<i16> {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_vorbis::register(&mut reg);
    oxideav_ogg::register(&mut reg);
    let mut file: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(ogg_data.to_vec()));
    let format = reg
        .containers
        .probe_input(&mut *file, Some("ogg"))
        .expect("probe ogg");
    let mut dmx = reg
        .containers
        .open_demuxer(&format, file, &oxideav_core::NullCodecResolver)
        .expect("open ogg demuxer");
    let params = dmx.streams()[0].params.clone();
    assert_eq!(params.codec_id.as_str(), "vorbis", "stream must be vorbis");
    let mut dec = reg
        .codecs
        .first_decoder(&params)
        .expect("make vorbis decoder");
    let mut out = Vec::new();
    loop {
        let pkt = match dmx.next_packet() {
            Ok(p) => p,
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {e:?}"),
        };
        if pkt.is_header() {
            continue; // §4.2 headers already live in params.extradata
        }
        dec.send_packet(&pkt).expect("send");
        loop {
            match dec.receive_frame() {
                Ok(Frame::Audio(a)) => {
                    // Planar f32 → interleaved i16.
                    let per_ch: Vec<Vec<f32>> = a
                        .data
                        .iter()
                        .map(|p| {
                            p.chunks_exact(4)
                                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                                .collect()
                        })
                        .collect();
                    let n = per_ch.iter().map(Vec::len).min().unwrap_or(0);
                    for i in 0..n {
                        for ch in &per_ch {
                            out.push((ch[i].clamp(-1.0, 1.0) * 32767.0) as i16);
                        }
                    }
                }
                Ok(_) => {}
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("decode error: {e:?}"),
            }
        }
    }
    out
}

/// Self-roundtrip that runs without ffmpeg: our encode → our Ogg mux →
/// our registry demux + decode, compared against the input.
#[test]
fn self_roundtrip_via_ogg() {
    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, 1.0);
    let ogg = encode_to_ogg_with_ours(&pcm, SAMPLE_RATE, CHANNELS);
    assert!(
        ogg.starts_with(b"OggS"),
        "muxed stream must start with an Ogg capture pattern"
    );

    let decoded = decode_with_ours(&ogg);
    assert!(!decoded.is_empty(), "our decode produced no samples");

    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, CHANNELS, 8192);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== Vorbis self-roundtrip (via ogg) ===");
    report("self", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.1, "self-roundtrip RMS {rms:.6} too large (> 0.1)");
}

/// Encoder test: our encode + our Ogg mux, decoded by ffmpeg, compared
/// against the input signal.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let ogg = encode_to_ogg_with_ours(&pcm, SAMPLE_RATE, CHANNELS);

    let ogg_path = tmp("oxideav-vorbis-enc-test.ogg");
    std::fs::write(&ogg_path, &ogg).expect("write ogg");

    let decoded_path = tmp("oxideav-vorbis-enc-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            ogg_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our Ogg/Vorbis stream"
    );

    let decoded = read_pcm_s16le(&decoded_path);
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, CHANNELS, 8192);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== Vorbis encoder comparison ===");
    report("encoder", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.15, "Vorbis encoder RMS {rms:.6} too large (> 0.15)");
}

/// Decoder test: ffmpeg-encoded Ogg/Vorbis, our registry decode vs
/// ffmpeg decode.
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, CHANNELS, DURATION);
    let raw_path = tmp("oxideav-vorbis-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    // Prefer the external-library encoder; fall back to the built-in
    // one (which requires the experimental-strictness opt-in).
    let ogg_path = tmp("oxideav-vorbis-dec-test.ogg");
    let encoded = ffmpeg(&[
        "-f",
        "s16le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        &CHANNELS.to_string(),
        "-i",
        raw_path.to_str().unwrap(),
        "-c:a",
        "libvorbis",
        "-q:a",
        "5",
        ogg_path.to_str().unwrap(),
    ]) || ffmpeg(&[
        "-f",
        "s16le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        &CHANNELS.to_string(),
        "-i",
        raw_path.to_str().unwrap(),
        "-c:a",
        "vorbis",
        "-strict",
        "-2",
        ogg_path.to_str().unwrap(),
    ]);
    if !encoded {
        eprintln!("skip: ffmpeg has no usable Vorbis encoder");
        return;
    }

    let ffmpeg_decoded_path = tmp("oxideav-vorbis-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            ogg_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            ffmpeg_decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg decode failed"
    );

    let ogg_data = std::fs::read(&ogg_path).expect("read ogg");
    let our_decoded = decode_with_ours(&ogg_data);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);
    assert!(!our_decoded.is_empty(), "our decoder produced no samples");

    let (rms, lag) = audio_rms_diff_aligned(&ffmpeg_decoded, &our_decoded, CHANNELS, 8192);
    let psnr = audio_psnr(&ffmpeg_decoded, &our_decoded[lag..]);
    eprintln!("=== Vorbis decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );
    // Two independent float implementations of the same synthesis: the
    // decodes of one bitstream agree closely.
    assert!(rms < 0.05, "Vorbis decoder RMS {rms:.6} too large (> 0.05)");
}
