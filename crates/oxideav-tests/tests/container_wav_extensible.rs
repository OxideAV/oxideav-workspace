//! WAVEFORMATEXTENSIBLE end-to-end: multi-channel float WAV through
//! the `oxideav-basic` mux/demux surfaces with typed
//! `oxideav_core::ChannelLayout` assertions.
//!
//! The write side promotes the `fmt ` chunk to the 40-byte EXTENSIBLE
//! form automatically for more than two channels, deriving
//! `dwChannelMask` from the stream's typed layout; the demuxer parses
//! the 22-byte extension back and surfaces the mask both as metadata
//! (`wav:fmt.channel_mask`) and as the typed layout on
//! `CodecParameters::channel_layout` (position-set matching). All
//! fixtures are generated in-process; the ffmpeg leg cross-checks the
//! layout mapping against an independently produced EXTENSIBLE file.

use oxideav_core::{
    ChannelLayout, CodecId, CodecParameters, Error, ReadSeek, SampleFormat, StreamInfo, TimeBase,
    WriteSeek,
};
use oxideav_tests::*;

const SAMPLE_RATE: u32 = 48000;

/// Interleaved f32 test signal: each channel carries its own tone so
/// channel order survives byte-level comparison.
fn multichannel_f32(channels: u16, samples: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples * channels as usize * 4);
    for i in 0..samples {
        let t = i as f64 / SAMPLE_RATE as f64;
        for ch in 0..channels {
            let f = 220.0 * (ch as f64 + 1.0);
            let v = (0.25 * (2.0 * std::f64::consts::PI * f * t).sin()) as f32;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

fn f32_stream_info(channels: u16, layout: Option<ChannelLayout>) -> StreamInfo {
    let mut params = CodecParameters::audio(CodecId::new("pcm_f32le"));
    params.sample_rate = Some(SAMPLE_RATE);
    params.channels = Some(channels);
    params.sample_format = Some(SampleFormat::F32);
    params.channel_layout = layout;
    StreamInfo {
        index: 0,
        time_base: TimeBase::new(1, SAMPLE_RATE as i64),
        duration: None,
        start_time: Some(0),
        params,
    }
}

/// Mux one interleaved payload into a WAV file through the container
/// registry (`oxideav-basic`'s registered "wav" muxer).
fn mux_wav(tag: &str, stream: &StreamInfo, payload: &[u8]) -> Vec<u8> {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_basic::register(&mut reg);
    let path = tmp(&format!("oxideav-wavext-{tag}.wav"));
    {
        let f = std::fs::File::create(&path).expect("create wav");
        let ws: Box<dyn WriteSeek> = Box::new(f);
        let mut mux = reg
            .containers
            .open_muxer("wav", ws, std::slice::from_ref(stream))
            .expect("open wav muxer");
        mux.write_header().expect("write header");
        let pkt =
            oxideav_core::Packet::new(0, TimeBase::new(1, SAMPLE_RATE as i64), payload.to_vec())
                .with_pts(0);
        mux.write_packet(&pkt).expect("write packet");
        mux.write_trailer().expect("write trailer");
    }
    std::fs::read(&path).expect("read wav")
}

/// Demux WAV bytes through the registry: returns the stream params,
/// the concatenated payload, and the container metadata.
fn demux_wav(data: &[u8]) -> (CodecParameters, Vec<u8>, Vec<(String, String)>) {
    let mut reg = oxideav_core::RuntimeContext::new();
    oxideav_basic::register(&mut reg);
    let mut file: Box<dyn ReadSeek> = Box::new(std::io::Cursor::new(data.to_vec()));
    let format = reg
        .containers
        .probe_input(&mut *file, Some("wav"))
        .expect("probe wav");
    assert_eq!(format, "wav");
    let mut dmx = reg
        .containers
        .open_demuxer(&format, file, &oxideav_core::NullCodecResolver)
        .expect("open wav demuxer");
    let params = dmx.streams()[0].params.clone();
    let metadata = dmx.metadata().to_vec();
    let mut payload = Vec::new();
    loop {
        match dmx.next_packet() {
            Ok(p) => payload.extend_from_slice(&p.data),
            Err(Error::Eof) => break,
            Err(e) => panic!("demux error: {e:?}"),
        }
    }
    (params, payload, metadata)
}

fn metadata_value<'a>(md: &'a [(String, String)], key: &str) -> Option<&'a str> {
    md.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// 5.1 float: >2 channels auto-promotes to EXTENSIBLE; the typed
/// Surround51 layout drives the mask and survives the round trip.
#[test]
fn five_one_float_extensible_roundtrip() {
    let payload = multichannel_f32(6, 4800);
    let stream = f32_stream_info(6, Some(ChannelLayout::Surround51));
    let wav = mux_wav("51", &stream, &payload);

    let (params, decoded, md) = demux_wav(&wav);
    assert_eq!(params.codec_id.as_str(), "pcm_f32le");
    assert_eq!(params.channels, Some(6));
    assert_eq!(params.sample_rate, Some(SAMPLE_RATE));
    assert_eq!(params.sample_format, Some(SampleFormat::F32));
    assert_eq!(
        params.channel_layout,
        Some(ChannelLayout::Surround51),
        "typed layout must survive the EXTENSIBLE mask round trip"
    );
    // Surround51's canonical positions are the side pair → 0x60F.
    let mask = metadata_value(&md, "wav:fmt.channel_mask")
        .expect("EXTENSIBLE file must surface wav:fmt.channel_mask");
    assert!(
        mask.contains("60F") || mask.contains("60f") || mask.contains("1551"),
        "unexpected channel mask surface: {mask}"
    );
    assert_eq!(decoded, payload, "float PCM must round-trip byte-exact");
}

/// Quad float: a second named layout through the same auto-promotion.
#[test]
fn quad_float_extensible_roundtrip() {
    let payload = multichannel_f32(4, 2400);
    let stream = f32_stream_info(4, Some(ChannelLayout::Quad));
    let wav = mux_wav("quad", &stream, &payload);

    let (params, decoded, _md) = demux_wav(&wav);
    assert_eq!(params.channels, Some(4));
    assert_eq!(params.channel_layout, Some(ChannelLayout::Quad));
    assert_eq!(decoded, payload);
}

/// Plain stereo S16 stays legacy WAVEFORMAT (no promotion — two
/// channels, 16-bit container), while stereo *float* auto-promotes
/// (container above 16 bits) and `WavMuxOptions::with_extensible`
/// forces the form explicitly; the demuxer types the mask back as
/// Stereo either way.
#[test]
fn forced_extensible_stereo_float() {
    let payload = multichannel_f32(2, 2400);
    let stream = f32_stream_info(2, Some(ChannelLayout::Stereo));

    // Baseline: stereo S16 takes the legacy 16/18-byte fmt → no mask.
    let s16_payload: Vec<u8> = (0..2400 * 2)
        .flat_map(|i| ((i % 251) as i16 * 100).to_le_bytes())
        .collect();
    let mut s16_stream = f32_stream_info(2, Some(ChannelLayout::Stereo));
    s16_stream.params.codec_id = oxideav_core::CodecId::new("pcm_s16le");
    s16_stream.params.sample_format = Some(SampleFormat::S16);
    let plain = mux_wav("stereo-plain", &s16_stream, &s16_payload);
    let (_, _, md_plain) = demux_wav(&plain);
    assert!(
        metadata_value(&md_plain, "wav:fmt.channel_mask").is_none(),
        "plain stereo S16 must not carry an EXTENSIBLE mask"
    );

    // Forced EXTENSIBLE via the direct mux-options surface (FL|FR).
    let path = tmp("oxideav-wavext-stereo-forced.wav");
    {
        let f = std::fs::File::create(&path).expect("create wav");
        let ws: Box<dyn WriteSeek> = Box::new(f);
        let opts = oxideav_basic::wav::WavMuxOptions::default().with_extensible(0x3);
        let mut mux = oxideav_basic::wav::open_muxer_with(ws, std::slice::from_ref(&stream), opts)
            .expect("open wav muxer with options");
        mux.write_header().expect("write header");
        let pkt =
            oxideav_core::Packet::new(0, TimeBase::new(1, SAMPLE_RATE as i64), payload.clone());
        mux.write_packet(&pkt).expect("write packet");
        mux.write_trailer().expect("write trailer");
    }
    let forced = std::fs::read(&path).expect("read wav");
    let (params, decoded, md) = demux_wav(&forced);
    assert_eq!(params.channel_layout, Some(ChannelLayout::Stereo));
    assert!(
        metadata_value(&md, "wav:fmt.channel_mask").is_some(),
        "forced EXTENSIBLE must surface the mask"
    );
    assert_eq!(decoded, payload);
}

/// Cross-check against an independently produced EXTENSIBLE file:
/// ffmpeg writes a 5.1(side) float WAV; our demuxer must type the
/// layout and hand back the same samples ffmpeg decodes.
#[test]
fn ffmpeg_five_one_float_extensible_demux() {
    if !ffmpeg_available() {
        eprintln!("skip: ffmpeg not available");
        return;
    }

    // Feed ffmpeg deterministic 6ch float PCM and let it author the
    // EXTENSIBLE WAV (6ch float always takes the 0xFFFE form).
    let payload = multichannel_f32(6, 4800);
    let raw_path = tmp("oxideav-wavext-ffmpeg-in.raw");
    std::fs::write(&raw_path, &payload).expect("write raw");
    let wav_path = tmp("oxideav-wavext-ffmpeg.wav");
    assert!(
        ffmpeg(&[
            "-f",
            "f32le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-channel_layout",
            "5.1(side)",
            "-i",
            raw_path.to_str().unwrap(),
            "-c:a",
            "pcm_f32le",
            wav_path.to_str().unwrap(),
        ]),
        "ffmpeg wav authoring failed"
    );

    let wav = std::fs::read(&wav_path).expect("read wav");
    let (params, decoded, md) = demux_wav(&wav);
    assert_eq!(params.codec_id.as_str(), "pcm_f32le");
    assert_eq!(params.channels, Some(6));
    assert_eq!(
        params.channel_layout,
        Some(ChannelLayout::Surround51),
        "5.1(side) mask must type as Surround51 (mask surface: {:?})",
        metadata_value(&md, "wav:fmt.channel_mask")
    );
    assert_eq!(
        decoded, payload,
        "float samples must pass through ffmpeg's WAV byte-exact"
    );
}
