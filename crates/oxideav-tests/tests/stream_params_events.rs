//! `Frame::StreamParams` end to end through the workspace: real emitters
//! (h264, mp3, vp8, vp9, mjpeg) decoding spliced streams whose output
//! shape changes mid-stream.
//!
//! Every fixture under `tests/fixtures/stream_params/` is two or three
//! independently encoded segments joined back to back, stream-copied
//! into one file (`generate.sh` there rebuilds them with the black-box
//! `ffmpeg` binary): H.264 Annex B 64x48 → 96x64 (B-frames, so the
//! reorder queue spans the switch), MP3 44.1 → 32 kHz in Matroska,
//! VP8 and VP9 64x48 → 96x64 in WebM, Motion-JPEG 64x48 → 48x32 →
//! 64x48 in AVI. Each one goes through
//!
//! * (a) the facade [`StreamDecoder`] — both the opt-in
//!   `receive_frame_or_event` (the decoder's events, in place) and the
//!   media-only `receive_frame` (events folded into `stream_params`,
//!   flagged by `params_changed`);
//! * (b) a pipeline transcode (`transcode_simple_in`) into an in-memory
//!   muxer, whose output is decoded again;
//! * (c) the executor with a raw-frame [`JobSink`] — the shape a
//!   player sees, where every change arrives through
//!   `JobSink::stream_params` and never through `write_frame`.
//!
//! The checks: one event per change, the right number of frames in
//! each segment with planes / samples sized for that segment's shape,
//! and an output stream whose parameters end on the final shape — or a
//! typed `Error::Unsupported` naming both shapes when the encoder
//! cannot follow.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use oxideav::StreamDecoder;
use oxideav_core::ExecutionContext;
use oxideav_core::{
    CodecParameters, Error, Frame, MediaType, Muxer, Packet, PixelFormat, Result, RuntimeContext,
    StreamInfo,
};
use oxideav_pipeline::{CodecPreferences, Executor, Job, JobSink, StreamPlan, transcode_simple_in};

fn ctx() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_meta::register_all(&mut ctx);
    ctx
}

/// Software codecs only: the hardware bridges (VideoToolbox on macOS,
/// VA-API on Linux) are not part of what this suite pins, and the
/// suite must decode the same way on every runner.
fn prefs() -> CodecPreferences {
    CodecPreferences {
        no_hardware: true,
        ..Default::default()
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stream_params")
        .join(name)
}

/// The output-describing part of a stream's parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Video(u32, u32, Option<PixelFormat>),
    Audio(u32, u16),
}

impl Shape {
    fn of(p: &CodecParameters) -> Self {
        match p.media_type {
            MediaType::Audio => Shape::Audio(
                p.sample_rate.unwrap_or(0),
                p.resolved_channels().unwrap_or(0),
            ),
            _ => Shape::Video(p.width.unwrap_or(0), p.height.unwrap_or(0), p.pixel_format),
        }
    }

    /// A field both shapes know and disagree on — a real change, as
    /// opposed to a field filled in that was unknown before.
    fn conflicts(&self, other: &Shape) -> bool {
        fn differ<T: PartialEq + Default>(a: T, b: T) -> bool {
            a != T::default() && b != T::default() && a != b
        }
        match (*self, *other) {
            (Shape::Video(w0, h0, f0), Shape::Video(w1, h1, f1)) => {
                differ(w0, w1) || differ(h0, h1) || matches!((f0, f1), (Some(a), Some(b)) if a != b)
            }
            (Shape::Audio(r0, c0), Shape::Audio(r1, c1)) => differ(r0, r1) || differ(c0, c1),
            _ => true,
        }
    }

    fn size(&self) -> (u32, u32) {
        match *self {
            Shape::Video(w, h, _) => (w, h),
            Shape::Audio(r, c) => (r, u32::from(c)),
        }
    }
}

/// What a decode produced: one entry per run of same-shape frames.
#[derive(Debug, Default)]
struct Decoded {
    /// `(shape, frames)` in order.
    segments: Vec<(Shape, usize)>,
    /// Events seen (or changes flagged, for the media-only pull).
    changes: usize,
}

impl Decoded {
    fn push(&mut self, shape: Shape) {
        match self.segments.last_mut() {
            Some((s, n)) if *s == shape => *n += 1,
            _ => self.segments.push((shape, 1)),
        }
    }

    /// Segment sizes / counts only (pixel layout and channel count
    /// are checked separately — they are the decoder's to choose).
    fn summary(&self) -> Vec<((u32, u32), usize)> {
        self.segments.iter().map(|(s, n)| (s.size(), *n)).collect()
    }
}

/// A media frame must be sized for the shape the stream announces.
fn check_frame(frame: &Frame, shape: Shape, what: &str) {
    match (frame, shape) {
        (Frame::Video(v), Shape::Video(w, h, fmt)) => {
            let planes = v.image_planes();
            assert!(!planes.is_empty(), "{what}: picture without planes");
            let y = &planes[0];
            assert!(y.stride > 0, "{what}: luma stride");
            assert_eq!(
                y.data.len() / y.stride,
                h as usize,
                "{what}: luma rows for {w}x{h}"
            );
            if let Some(f) = fmt
                && let Some(row) = f.plane_row_bytes(0, w)
            {
                assert!(y.stride >= row, "{what}: luma stride {} < {row}", y.stride);
            }
        }
        (Frame::Audio(a), Shape::Audio(_, ch)) => {
            assert!(a.samples > 0, "{what}: empty audio frame");
            assert!(ch > 0, "{what}: channel count unknown");
        }
        (f, s) => panic!("{what}: frame {f:?} does not match stream shape {s:?}"),
    }
}

fn open(ctx: &RuntimeContext, path: &Path) -> Box<dyn oxideav_core::Demuxer> {
    let mut f = std::fs::File::open(path).expect("fixture");
    let ext = path.extension().and_then(|e| e.to_str());
    let fmt = ctx.containers.probe_input(&mut f, ext).expect("probe");
    ctx.containers
        .open_demuxer(&fmt, Box::new(f), &ctx.codecs)
        .expect("open demuxer")
}

/// (a) The facade, opt-in pull: the decoder's events come out in
/// place; `stream_params` already describes the new shape on the event.
fn decode_with_events(ctx: &RuntimeContext, path: &Path) -> Decoded {
    let mut dmx = open(ctx, path);
    let stream = dmx.streams()[0].clone();
    let mut dec =
        StreamDecoder::open_with(ctx, &stream.params, &prefs(), &ExecutionContext::auto())
            .expect("decoder");
    let mut out = Decoded::default();
    let mut eof = false;
    loop {
        if !eof {
            match dmx.next_packet() {
                Ok(p) if p.stream_index == stream.index => dec.send_packet(&p).expect("send"),
                Ok(_) => continue,
                Err(Error::Eof) => {
                    dec.flush().expect("flush");
                    eof = true;
                }
                Err(e) => panic!("demux: {e}"),
            }
        }
        loop {
            match dec.receive_frame_or_event() {
                Ok(f) if f.is_stream_params() => {
                    let p = f.stream_params().unwrap();
                    assert!(dec.params_changed(), "event flagged as a change");
                    assert_eq!(
                        Shape::of(p),
                        Shape::of(dec.stream_params()),
                        "event payload is the stream's new shape"
                    );
                    assert!(
                        !out.segments.is_empty(),
                        "no event before the first frame (the first shape is output_params)"
                    );
                    assert_ne!(
                        Shape::of(p),
                        out.segments.last().unwrap().0,
                        "an event announces a real change"
                    );
                    out.changes += 1;
                }
                Ok(f) => {
                    let shape = Shape::of(dec.stream_params());
                    check_frame(&f, shape, &path.display().to_string());
                    out.push(shape);
                }
                Err(Error::NeedMore) => break,
                Err(Error::Eof) => return out,
                Err(e) => panic!("{}: decode: {e}", path.display()),
            }
        }
    }
}

/// (a') The facade, media-only pull: never an event, a change flagged
/// on the first frame of each new shape.
fn decode_media_only(ctx: &RuntimeContext, path: &Path) -> Decoded {
    let mut dmx = open(ctx, path);
    let stream = dmx.streams()[0].clone();
    let mut dec =
        StreamDecoder::open_with(ctx, &stream.params, &prefs(), &ExecutionContext::auto())
            .expect("decoder");
    let mut out = Decoded::default();
    let mut eof = false;
    loop {
        if !eof {
            match dmx.next_packet() {
                Ok(p) if p.stream_index == stream.index => dec.send_packet(&p).expect("send"),
                Ok(_) => continue,
                Err(Error::Eof) => {
                    dec.flush().expect("flush");
                    eof = true;
                }
                Err(e) => panic!("demux: {e}"),
            }
        }
        loop {
            match dec.receive_frame() {
                Ok(f) => {
                    assert!(!f.is_stream_params(), "receive_frame is media-only");
                    let shape = Shape::of(dec.stream_params());
                    check_frame(&f, shape, &path.display().to_string());
                    if dec.params_changed() && !out.segments.is_empty() {
                        out.changes += 1;
                    }
                    out.push(shape);
                }
                Err(Error::NeedMore) => break,
                Err(Error::Eof) => return out,
                Err(e) => panic!("{}: decode: {e}", path.display()),
            }
        }
    }
}

// ─────────────────────── (b) pipeline transcode ───────────────────────

/// An in-memory muxer: keeps the stream table it was opened with and
/// every packet written.
#[derive(Default)]
struct Captured {
    streams: Vec<StreamInfo>,
    packets: Vec<Packet>,
}

struct CaptureMuxer(Arc<Mutex<Captured>>);

impl Muxer for CaptureMuxer {
    fn format_name(&self) -> &str {
        "capture"
    }
    fn write_header(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        self.0.lock().unwrap().packets.push(packet.clone());
        Ok(())
    }
    fn write_trailer(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Transcode `path` re-encoding its only stream with `codec`. Returns
/// the transcode's result and what reached the muxer.
fn transcode(ctx: &RuntimeContext, path: &Path, codec: &str) -> (Result<u64>, Captured) {
    let mut dmx = open(ctx, path);
    let cap = Arc::new(Mutex::new(Captured::default()));
    let sink = cap.clone();
    let res = transcode_simple_in(
        &mut *dmx,
        move |streams: &[StreamInfo]| -> Result<Box<dyn Muxer>> {
            sink.lock().unwrap().streams = streams.to_vec();
            Ok(Box::new(CaptureMuxer(sink)))
        },
        ctx,
        None,
        &prefs(),
        |_| {
            Ok(StreamPlan::Reencode {
                output_codec: codec.to_owned(),
            })
        },
    )
    .map(|s| s.frames_decoded);
    let captured = std::mem::take(&mut *cap.lock().unwrap());
    (res, captured)
}

/// Decode what a transcode wrote (one stream) with the facade's
/// event-aware pull.
fn decode_packets(ctx: &RuntimeContext, cap: &Captured) -> Decoded {
    let stream = &cap.streams[0];
    let mut dec =
        StreamDecoder::open_with(ctx, &stream.params, &prefs(), &ExecutionContext::auto())
            .expect("output decoder");
    let mut out = Decoded::default();
    let drain = |dec: &mut StreamDecoder, out: &mut Decoded| {
        loop {
            match dec.receive_frame_or_event() {
                Ok(f) if f.is_stream_params() => out.changes += 1,
                Ok(f) => {
                    let shape = Shape::of(dec.stream_params());
                    check_frame(&f, shape, "transcoded output");
                    out.push(shape);
                }
                Err(Error::NeedMore | Error::Eof) => break,
                Err(e) => panic!("output decode: {e}"),
            }
        }
    };
    for p in &cap.packets {
        dec.send_packet(p).expect("send output packet");
        drain(&mut dec, &mut out);
    }
    dec.flush().expect("flush");
    drain(&mut dec, &mut out);
    out
}

// ─────────────────────── (c) executor, raw frames ───────────────────────

#[derive(Default)]
struct SinkLog {
    started: Vec<StreamInfo>,
    /// Shapes told through `stream_params`, in order (a call may also
    /// fill in a field the stream did not state at `start`).
    told: Vec<Shape>,
    /// Calls that changed a field the sink already knew.
    changes: usize,
    /// `(shape in force, frames)` runs.
    decoded: Decoded,
    current: Option<Shape>,
    events_as_frames: usize,
}

struct RecordingSink(Arc<Mutex<SinkLog>>);

impl JobSink for RecordingSink {
    fn start(&mut self, streams: &[StreamInfo]) -> Result<()> {
        let mut log = self.0.lock().unwrap();
        log.started = streams.to_vec();
        log.current = streams.first().map(|s| Shape::of(&s.params));
        Ok(())
    }
    fn write_packet(&mut self, _kind: MediaType, _pkt: &Packet) -> Result<()> {
        Err(Error::unsupported("raw-frame sink"))
    }
    fn write_frame(&mut self, _kind: MediaType, frm: &Frame) -> Result<()> {
        let mut log = self.0.lock().unwrap();
        if frm.is_stream_params() {
            log.events_as_frames += 1;
            return Ok(());
        }
        let shape = log.current.expect("started");
        check_frame(frm, shape, "executor sink");
        log.decoded.push(shape);
        Ok(())
    }
    fn stream_params(&mut self, _kind: MediaType, params: &CodecParameters) -> Result<()> {
        let mut log = self.0.lock().unwrap();
        let shape = Shape::of(params);
        if log.current.is_some_and(|c| c.conflicts(&shape)) {
            log.changes += 1;
        }
        log.told.push(shape);
        log.current = Some(shape);
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

fn run_executor(ctx: &RuntimeContext, path: &Path, kind: &str, threads: usize) -> SinkLog {
    let json = serde_json::json!({
        "@in": { "all": [{ "from": path.to_str().unwrap() }] },
        "@display": { kind: [{ "from": "@in" }] },
    })
    .to_string();
    let job = Job::from_json(&json).expect("job");
    let log = Arc::new(Mutex::new(SinkLog::default()));
    Executor::new(&job, ctx)
        .with_sink_override("@display", Box::new(RecordingSink(log.clone())))
        .with_codec_preferences(prefs())
        .with_threads(threads)
        .run()
        .unwrap_or_else(|e| panic!("{}: executor: {e}", path.display()));
    std::mem::take(&mut *log.lock().unwrap())
}

fn assert_executor(ctx: &RuntimeContext, path: &Path, kind: &str, want: &[((u32, u32), usize)]) {
    for threads in [1, 4] {
        let log = run_executor(ctx, path, kind, threads);
        let what = format!("{} (executor, {threads} threads)", path.display());
        assert_eq!(
            log.events_as_frames, 0,
            "{what}: an event reached write_frame"
        );
        assert_eq!(log.decoded.summary(), want, "{what}: frames per segment");
        assert_eq!(
            log.changes,
            want.len() - 1,
            "{what}: one stream_params call per change: {:?}",
            log.told
        );
    }
}

// ─────────────────────────────── tests ───────────────────────────────

/// The pipeline's policy for an encoder that cannot follow a change:
/// `Error::Unsupported`, naming the old and the new shape.
fn assert_unsupported_change(res: Result<u64>, old: &str, new: &str, what: &str) {
    match res {
        Err(Error::Unsupported(msg)) => {
            assert!(
                msg.contains(old) && msg.contains(new),
                "{what}: the error names both shapes: {msg}"
            );
        }
        other => panic!("{what}: expected Error::Unsupported, got {other:?}"),
    }
}

fn assert_decodes(ctx: &RuntimeContext, name: &str, want: &[((u32, u32), usize)]) {
    let path = fixture(name);
    let ev = decode_with_events(ctx, &path);
    assert_eq!(ev.summary(), want, "{name}: frames per segment (events)");
    assert_eq!(ev.changes, want.len() - 1, "{name}: one event per change");
    let media = decode_media_only(ctx, &path);
    assert_eq!(
        media.summary(),
        want,
        "{name}: frames per segment (media-only)"
    );
    assert_eq!(
        media.changes,
        want.len() - 1,
        "{name}: one flagged change per change"
    );
}

#[test]
fn h264_resolution_switch_splice() {
    let ctx = ctx();
    let want = [((64, 48), 6), ((96, 64), 6)];
    assert_decodes(&ctx, "h264_resize_64x48_96x64.h264", &want);
    assert_executor(
        &ctx,
        &fixture("h264_resize_64x48_96x64.h264"),
        "video",
        &want,
    );

    // The h264 encoder re-configures on the event: the output carries
    // both shapes, and decodes back to the same segments.
    let (res, cap) = transcode(&ctx, &fixture("h264_resize_64x48_96x64.h264"), "h264");
    assert_eq!(
        res.expect("h264 → h264 transcode"),
        12,
        "media frames decoded"
    );
    let back = decode_packets(&ctx, &cap);
    assert_eq!(back.summary(), want, "h264 re-encode keeps the segments");
    assert_eq!(back.changes, 1);
}

#[test]
fn mp3_sample_rate_switch_concat() {
    let ctx = ctx();
    let want = [((44_100, 2), 11), ((32_000, 2), 8)];
    assert_decodes(&ctx, "mp3_rate_44100_32000.mka", &want);
    assert_executor(&ctx, &fixture("mp3_rate_44100_32000.mka"), "audio", &want);
    // No audio encoder re-configures for a new rate (and the transcode
    // does not insert a resampler behind the caller's back): the run
    // fails with a typed error naming both shapes.
    for codec in ["mp3", "pcm_s16le"] {
        let (res, cap) = transcode(&ctx, &fixture("mp3_rate_44100_32000.mka"), codec);
        assert_unsupported_change(res, "44100 Hz", "32000 Hz", &format!("mp3 → {codec}"));
        assert_eq!(Shape::of(&cap.streams[0].params).size(), (44_100, 2));
    }
}

#[test]
fn vp8_resize_splice() {
    let ctx = ctx();
    let want = [((64, 48), 5), ((96, 64), 5)];
    assert_decodes(&ctx, "vp8_resize_64x48_96x64.webm", &want);
    assert_executor(
        &ctx,
        &fixture("vp8_resize_64x48_96x64.webm"),
        "video",
        &want,
    );

    let (res, cap) = transcode(&ctx, &fixture("vp8_resize_64x48_96x64.webm"), "h264");
    assert_eq!(res.expect("vp8 → h264 transcode"), 10);
    let back = decode_packets(&ctx, &cap);
    assert_eq!(back.summary(), want, "h264 re-encode keeps the segments");
    assert_eq!(back.changes, 1);
}

#[test]
fn vp9_resize_splice() {
    let ctx = ctx();
    let want = [((64, 48), 5), ((96, 64), 5)];
    assert_decodes(&ctx, "vp9_resize_64x48_96x64.webm", &want);
    assert_executor(
        &ctx,
        &fixture("vp9_resize_64x48_96x64.webm"),
        "video",
        &want,
    );

    // FFV1 cannot follow a size change mid-stream: a typed error naming
    // both shapes, after every frame of the first segment was encoded.
    let (res, cap) = transcode(&ctx, &fixture("vp9_resize_64x48_96x64.webm"), "ffv1");
    assert_unsupported_change(res, "64x48", "96x64", "vp9 → ffv1");
    assert_eq!(cap.packets.len(), 5, "the first segment was encoded");
    let back = decode_packets(&ctx, &cap);
    assert_eq!(back.summary(), [((64, 48), 5)]);
}

#[test]
fn mjpeg_mixed_sizes() {
    let ctx = ctx();
    let want = [((64, 48), 2), ((48, 32), 2), ((64, 48), 2)];
    assert_decodes(&ctx, "mjpeg_mixed_64x48_48x32_64x48.avi", &want);
    assert_executor(
        &ctx,
        &fixture("mjpeg_mixed_64x48_48x32_64x48.avi"),
        "video",
        &want,
    );

    let (res, cap) = transcode(&ctx, &fixture("mjpeg_mixed_64x48_48x32_64x48.avi"), "mjpeg");
    assert_eq!(res.expect("mjpeg → mjpeg transcode"), 6);
    let back = decode_packets(&ctx, &cap);
    assert_eq!(back.summary(), want, "mjpeg re-encode keeps the segments");
    assert_eq!(back.changes, 2);
}
