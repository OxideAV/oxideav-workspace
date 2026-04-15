//! End-to-end decode of sine.ogg via the VorbisDecoder and a manual Ogg
//! packet walker. Writes PCM to /tmp/oxideav-test/ours-vorbis.wav if
//! sine.ogg is present. Intended for ad-hoc local validation against
//! ffmpeg's decode.

#![allow(clippy::needless_range_loop)]

use oxideav_core::{CodecId, CodecParameters, Frame, Packet, TimeBase};
use oxideav_vorbis::decoder::make_decoder;

fn collect_packets(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut i = 0usize;
    while i + 27 <= data.len() {
        if &data[i..i + 4] != b"OggS" {
            break;
        }
        let n_segs = data[i + 26] as usize;
        let lacing = &data[i + 27..i + 27 + n_segs];
        let mut off = i + 27 + n_segs;
        for &lv in lacing {
            buf.extend_from_slice(&data[off..off + lv as usize]);
            off += lv as usize;
            if lv < 255 {
                out.push(std::mem::take(&mut buf));
            }
        }
        i = off;
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

fn xiph_lace(packets: &[&[u8]]) -> Vec<u8> {
    let n = packets.len();
    let mut out = Vec::new();
    out.push((n - 1) as u8);
    for p in &packets[..n - 1] {
        let mut sz = p.len();
        while sz >= 255 {
            out.push(255);
            sz -= 255;
        }
        out.push(sz as u8);
    }
    for p in packets {
        out.extend_from_slice(p);
    }
    out
}

fn write_wav(path: &str, samples: &[i16], channels: u16, sample_rate: u32) -> std::io::Result<()> {
    use std::io::Write;
    let byte_rate = sample_rate * channels as u32 * 2;
    let block_align = channels * 2;
    let data_len = samples.len() as u32 * 2;
    let riff_len = 36 + data_len;
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"RIFF")?;
    f.write_all(&riff_len.to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

fn main() {
    let path = "/tmp/oxideav-test/sine.ogg";
    let Ok(data) = std::fs::read(path) else {
        eprintln!("skipped: {path} not present");
        return;
    };
    let pkts = collect_packets(&data);
    eprintln!("total packets: {}", pkts.len());
    assert!(pkts.len() >= 4);
    let headers = [&pkts[0][..], &pkts[1][..], &pkts[2][..]];
    let extradata = xiph_lace(&headers);
    let mut params = CodecParameters::audio(CodecId::new("vorbis"));
    params.extradata = extradata;
    let mut dec = make_decoder(&params).expect("make_decoder");
    let mut out_samples: Vec<i16> = Vec::new();
    let mut total_frames = 0usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    for (i, p) in pkts.iter().enumerate().skip(3) {
        let packet = Packet::new(0, TimeBase::new(1, 48_000), p.clone());
        if let Err(e) = dec.send_packet(&packet) {
            eprintln!("pkt{i}: send err {e}");
            continue;
        }
        match dec.receive_frame() {
            Ok(Frame::Audio(a)) => {
                total_frames += 1;
                sample_rate = a.sample_rate;
                channels = a.channels;
                if let Some(plane) = a.data.first() {
                    for chunk in plane.chunks_exact(2) {
                        out_samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                    }
                }
                if i <= 5 {
                    eprintln!(
                        "pkt{}: {} samples, {}ch, sr={}",
                        i, a.samples, a.channels, a.sample_rate
                    );
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("pkt{i}: recv err {e}"),
        }
    }
    eprintln!(
        "decoded {total_frames} audio frames, {} samples total",
        out_samples.len() / channels.max(1) as usize
    );
    if channels > 0 {
        let out = "/tmp/oxideav-test/ours-vorbis.wav";
        write_wav(out, &out_samples, channels, sample_rate).unwrap();
        eprintln!("wrote {out}");
    }
}
