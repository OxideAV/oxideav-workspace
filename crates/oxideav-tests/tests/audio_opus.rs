//! Opus cross-crate integration tests.
//!
//! Round 449 refresh: the old suite predated the encoder arc — its
//! prose claimed "no Opus encoder" and a "partial (SILK/CELT not
//! fully landed)" decoder, both long stale. The `oxideav-opus` crate
//! ships a complete decoder (SILK bit-exact, CELT at 1 LSB against
//! the RFC 6716 §A reference listing) and packet encoders for every
//! arm (SILK / CELT / Hybrid, CBR + VBR), including §2.1.9 DTX
//! (round 448).
//!
//! The registry registration for `opus` is tag-only (capability
//! metadata + the `OpusHead` payload-magic claim; no framework
//! decoder/encoder factories are wired), so these tests exercise the
//! crate's direct packet APIs — `CeltEncoder` / `SilkEncoderMono` /
//! `OpusDecoder` — while the *container* legs flow through the real
//! cross-crate surfaces: the `oxideav-ogg` muxer builds physical
//! Ogg/Opus streams from an `OpusHead` in `extradata` (RFC 7845
//! header synthesis is the muxer's), and the registry demux path
//! hands the packets back for decode.
//!
//! Opus always decodes at 48 kHz.

use oxideav_core::{
    CodecId, CodecParameters, Error, Packet, ReadSeek, SampleFormat, StreamInfo, TimeBase,
    WriteSeek,
};
use oxideav_opus::celt_packet_encode::CeltEncoder;
use oxideav_opus::decoder::{FrameDecodeStatus, OpusDecoder};
use oxideav_opus::opus_head::{ChannelMappingTable, OpusHead};
use oxideav_opus::silk_encoder::SilkEncoderMono;
use oxideav_opus::toc::Bandwidth;
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 48000;

/// Compose a mapping-family-0 `OpusHead` (RFC 7845 §5.1) with zero
/// pre-skip, for muxing our own packet streams.
fn opus_head(channels: u8) -> Vec<u8> {
    let head = OpusHead {
        version: 1,
        channel_count: channels,
        pre_skip: 0,
        input_sample_rate: SAMPLE_RATE,
        output_gain_q7_8: 0,
        mapping_family: 0,
        mapping: ChannelMappingTable {
            stream_count: 1,
            coupled_count: if channels == 2 { 1 } else { 0 },
            mapping: if channels == 2 { vec![0, 1] } else { vec![0] },
        },
    };
    head.compose().expect("compose OpusHead")
}

/// Mux a run of Opus packets into a physical Ogg/Opus stream through
/// the container registry. `samples_per_packet` drives the granule
/// positions (packet `pts` carries the granule for the Ogg muxer).
fn mux_to_ogg(tag: &str, packets: &[Vec<u8>], channels: u8, samples_per_packet: i64) -> Vec<u8> {
    let mut params = CodecParameters::audio(CodecId::new("opus"));
    params.sample_rate = Some(SAMPLE_RATE);
    params.channels = Some(channels as u16);
    params.sample_format = Some(SampleFormat::S16);
    params.extradata = opus_head(channels);

    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_ogg::register(&mut reg);
    let stream = StreamInfo {
        index: 0,
        time_base: TimeBase::new(1, SAMPLE_RATE as i64),
        duration: None,
        start_time: Some(0),
        params,
    };
    let mux_path = tmp(&format!("oxideav-opus-mux-{tag}-{channels}ch.ogg"));
    {
        let f = std::fs::File::create(&mux_path).expect("create mux file");
        let ws: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = reg
            .containers
            .open_muxer("ogg", ws, std::slice::from_ref(&stream))
            .expect("open ogg muxer");
        mux.write_header().expect("write header");
        let mut granule = 0i64;
        for pkt in packets {
            granule += samples_per_packet;
            let p = Packet::new(0, TimeBase::new(1, SAMPLE_RATE as i64), pkt.clone())
                .with_pts(granule)
                .with_duration(samples_per_packet);
            mux.write_packet(&p).expect("write packet");
        }
        mux.write_trailer().expect("write trailer");
    }
    std::fs::read(&mux_path).expect("read muxed ogg")
}

/// Demux an Ogg/Opus stream through the container registry and decode
/// every audio packet with our `OpusDecoder`. Returns interleaved
/// 48 kHz i16 PCM with the stream's declared pre-skip trimmed, plus
/// the per-frame outcome list.
fn decode_ogg_with_ours(ogg_data: &[u8]) -> (Vec<i16>, Vec<FrameDecodeStatus>) {
    let mut reg = oxideav_core::RuntimeContext::new();
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
    assert_eq!(params.codec_id.as_str(), "opus", "stream must be opus");
    let head = OpusHead::parse(&params.extradata).expect("demuxer must surface OpusHead");
    let channels = head.channel_count as usize;

    let mut dec = OpusDecoder::new();
    let mut pcm = Vec::new();
    let mut outcomes = Vec::new();
    loop {
        let pkt = match dmx.next_packet() {
            Ok(p) => p,
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {e:?}"),
        };
        if pkt.is_header() {
            continue; // OpusHead/OpusTags live in params.extradata
        }
        let out = dec.decode_packet(&pkt.data).expect("decode packet");
        pcm.extend_from_slice(&out.pcm);
        outcomes.extend(out.frame_outcomes.iter().map(|o| o.status));
    }
    let skip = head.pre_skip as usize * channels;
    (pcm.split_off(skip.min(pcm.len())), outcomes)
}

/// Encode interleaved 48 kHz stereo PCM as CELT-only fullband 20 ms
/// packets (CBR at `payload_bytes` per frame).
fn encode_celt_stereo(pcm: &[i16], payload_bytes: usize) -> Vec<Vec<u8>> {
    let mut enc = CeltEncoder::new(Bandwidth::Fb, 200, true).expect("celt encoder");
    let frame = enc.frame_samples() * enc.channels();
    pcm.chunks_exact(frame)
        .map(|c| enc.encode_packet(c, payload_bytes).expect("encode").0)
        .collect()
}

/// Deterministic voice-like content at `rate_hz`: a pitch-pulse train
/// through a resonator plus a tone, well above the §4.2.3 activity
/// floor. (Same shape the opus crate's own DTX gates use.)
fn voice(rate_hz: usize, n: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; n];
    let (mut y1, mut y2) = (0.0f64, 0.0f64);
    for (i, slot) in out.iter_mut().enumerate() {
        let t = i as f64 / rate_hz as f64;
        let period = (rate_hz as f64 / 110.0) as usize;
        let pulse = if i % period < 6 { 1.0 } else { 0.0 };
        let x = pulse + 0.4 * (2.0 * std::f64::consts::PI * 220.0 * t).sin();
        let w = 2.0 * std::f64::consts::PI * 500.0 / rate_hz as f64;
        let r = 0.94;
        let y = x + 2.0 * r * w.cos() * y1 - r * r * y2;
        y2 = y1;
        y1 = y;
        *slot = (0.15 * y) as f32;
    }
    out
}

/// SILK-WB mono voice | silence | voice, 20 ms packets, DTX on/off.
fn encode_silk_dtx_stream(voice_ms: usize, silence_ms: usize, dtx: bool) -> Vec<Vec<u8>> {
    const WB_20MS: usize = 320; // 16 kHz internal rate
    let v = voice_ms * 16;
    let s = silence_ms * 16;
    let mut sig = voice(16_000, v);
    sig.extend(std::iter::repeat(0.0f32).take(s));
    sig.extend(voice(16_000, v));
    let mut enc = SilkEncoderMono::new(Bandwidth::Wb).expect("silk encoder");
    enc.set_dtx(dtx);
    sig.chunks_exact(WB_20MS)
        .map(|c| enc.encode_packet(c).expect("encode").packet)
        .collect()
}

/// Self-roundtrip that runs without ffmpeg: our CELT encode → our Ogg
/// mux → our registry demux → our decode, compared against the input.
#[test]
fn self_roundtrip_celt_via_ogg() {
    let pcm = generate_audio_signal(SAMPLE_RATE, 2, 1.0);
    let packets = encode_celt_stereo(&pcm, 240); // 96 kb/s CBR
    assert!(!packets.is_empty());
    let ogg = mux_to_ogg("self", &packets, 2, 960);
    assert!(
        ogg.starts_with(b"OggS"),
        "muxed stream must start with an Ogg capture pattern"
    );

    let (decoded, outcomes) = decode_ogg_with_ours(&ogg);
    assert_eq!(
        decoded.len(),
        packets.len() * 960 * 2,
        "each 20 ms fullband packet decodes to 960 samples/channel"
    );
    assert!(
        outcomes
            .iter()
            .all(|s| *s == FrameDecodeStatus::CeltDecoded),
        "every frame must take the CELT decode path: {outcomes:?}"
    );

    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, 2, 8192);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== Opus CELT self-roundtrip (via ogg) ===");
    report("self", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.1, "self-roundtrip RMS {rms:.6} too large (> 0.1)");
}

/// Encoder test: our CELT encode + our Ogg mux, decoded by ffmpeg,
/// compared against the input signal.
#[test]
fn encoder_vs_ffmpeg_decode() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, 2, 2.0);
    let packets = encode_celt_stereo(&pcm, 240);
    let ogg = mux_to_ogg("enc", &packets, 2, 960);
    let ogg_path = tmp("oxideav-opus-enc-test.ogg");
    std::fs::write(&ogg_path, &ogg).expect("write ogg");

    let decoded_path = tmp("oxideav-opus-enc-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            ogg_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our Ogg/Opus stream"
    );

    let decoded = read_pcm_s16le(&decoded_path);
    assert!(!decoded.is_empty(), "ffmpeg produced no samples");
    let (rms, lag) = audio_rms_diff_aligned(&pcm, &decoded, 2, 8192);
    let psnr = audio_psnr(&pcm, &decoded[lag..]);
    eprintln!("=== Opus encoder comparison ===");
    report("encoder", rms, psnr, decoded.len(), pcm.len());
    assert!(rms < 0.1, "Opus encoder RMS {rms:.6} too large (> 0.1)");
}

/// Decoder test: ffmpeg-encoded Ogg/Opus, our registry demux + decode
/// vs ffmpeg's own decode of the same stream. Two complete decoders
/// of one bitstream agree closely (our decoder tracks the reference
/// listing at 1 LSB on CELT, bit-exact on SILK).
#[test]
fn decoder_vs_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let pcm = generate_audio_signal(SAMPLE_RATE, 2, 2.0);
    let raw_path = tmp("oxideav-opus-dec-input.raw");
    write_pcm_s16le(&raw_path, &pcm);

    let ogg_path = tmp("oxideav-opus-dec-test.ogg");
    if !ffmpeg(&[
        "-f",
        "s16le",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-i",
        raw_path.to_str().unwrap(),
        "-c:a",
        "libopus",
        "-b:a",
        "128k",
        ogg_path.to_str().unwrap(),
    ]) {
        eprintln!("skip: ffmpeg libopus encode failed (libopus may not be available)");
        return;
    }

    let ffmpeg_decoded_path = tmp("oxideav-opus-dec-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            ogg_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
            ffmpeg_decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg decode failed"
    );

    let ogg_data = std::fs::read(&ogg_path).expect("read ogg");
    let (our_decoded, _) = decode_ogg_with_ours(&ogg_data);
    let ffmpeg_decoded = read_pcm_s16le(&ffmpeg_decoded_path);
    assert!(!our_decoded.is_empty(), "our decoder produced no samples");

    let (rms, lag) = audio_rms_diff_aligned(&ffmpeg_decoded, &our_decoded, 2, 8192);
    let psnr = audio_psnr(&ffmpeg_decoded, &our_decoded[lag..]);
    eprintln!("=== Opus decoder comparison ===");
    report(
        "decoder",
        rms,
        psnr,
        our_decoded.len(),
        ffmpeg_decoded.len(),
    );
    assert!(rms < 0.02, "Opus decoder RMS {rms:.6} too large (> 0.02)");
}

/// §2.1.9 DTX round-trip without any oracle: a voice|silence|voice
/// stream encoded with `set_dtx(true)` must carry 1-byte TOC-only
/// markers in the silent run, save bytes over the DTX-off encode, and
/// still decode to the exact per-packet sample count with the markers
/// routed through the concealment hold.
#[test]
fn dtx_stream_suppresses_and_decodes_exact_counts() {
    let on = encode_silk_dtx_stream(500, 2_000, true);
    let off = encode_silk_dtx_stream(500, 2_000, false);
    assert_eq!(on.len(), off.len());

    let markers = on.iter().filter(|p| p.len() == 1).count();
    assert!(markers > 0, "DTX must emit 1-byte TOC-only markers");
    let bytes_on: usize = on.iter().map(Vec::len).sum();
    let bytes_off: usize = off.iter().map(Vec::len).sum();
    assert!(
        bytes_on < bytes_off,
        "DTX stream must be smaller: {bytes_on} vs {bytes_off}"
    );

    let mut dec = OpusDecoder::new();
    let mut total = 0usize;
    let mut dtx_frames = 0usize;
    for p in &on {
        let out = dec.decode_packet(p).expect("decode DTX stream");
        total += out.samples_per_channel();
        dtx_frames += out
            .frame_outcomes
            .iter()
            .filter(|o| o.status == FrameDecodeStatus::DtxOrLost)
            .count();
    }
    assert_eq!(
        total,
        on.len() * 960,
        "every 20 ms packet (marker or coded) is 960 samples/channel"
    );
    assert!(
        dtx_frames >= markers,
        "each marker decodes through the §4.4 hold: {dtx_frames} < {markers}"
    );
}

/// DTX interop: the marker-bearing stream muxed into Ogg/Opus by our
/// muxer decodes through ffmpeg with the exact declared duration.
#[test]
fn dtx_stream_via_ogg_decodes_in_ffmpeg() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    let on = encode_silk_dtx_stream(500, 2_000, true);
    assert!(on.iter().any(|p| p.len() == 1), "stream must carry markers");
    let ogg = mux_to_ogg("dtx", &on, 1, 960);
    let ogg_path = tmp("oxideav-opus-dtx-test.ogg");
    std::fs::write(&ogg_path, &ogg).expect("write ogg");

    let decoded_path = tmp("oxideav-opus-dtx-ffmpeg.raw");
    assert!(
        ffmpeg(&[
            "-i",
            ogg_path.to_str().unwrap(),
            "-f",
            "s16le",
            "-ar",
            "48000",
            "-ac",
            "1",
            decoded_path.to_str().unwrap(),
        ]),
        "ffmpeg refused our DTX-bearing Ogg/Opus stream"
    );
    let decoded = read_pcm_s16le(&decoded_path);
    assert_eq!(
        decoded.len(),
        on.len() * 960,
        "declared granule duration must survive the DTX markers"
    );

    // The silent run stays quiet through the marker decode: measure the
    // middle of the suppressed region (packets 30..120 of 25+100+25).
    let mid = &decoded[40 * 960..100 * 960];
    let rms = audio_rms_diff(mid, &vec![0i16; mid.len()]);
    eprintln!("=== Opus DTX ffmpeg interop: silent-run RMS {rms:.6} ===");
    assert!(rms < 0.05, "suppressed region too loud: RMS {rms:.6}");
}
