//! Headless playback of spliced streams whose shape changes mid-stream
//! (`Frame::StreamParams` through `JobSink::stream_params`): the real
//! executor, the real [`ChannelSink`] and [`PlayerEngine`], and a
//! recording driver in place of a window / audio device. The driver
//! must be re-planned exactly where the new shape starts — every
//! picture presented with the size the driver was last told, every
//! audio frame queued at the rate it was decoded at.
//!
//! Fixtures: the spliced streams `oxideav-tests` keeps under
//! `tests/fixtures/stream_params/` (same umbrella repository).

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxideav::Registries;
use oxideav::pipeline::{CodecPreferences, Executor, Job};
use oxideav_core::{AudioFrame, CodecParameters, Result, VideoFrame};

use crate::driver::{OutputDriver, PlayerEvent};
use crate::engine::{EngineMsg, PlayerEngine};
use crate::job_sink::ChannelSink;
use crate::media_controls::{NoopMediaControls, TrackInfo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    /// `set_source_video_params`: width × height.
    VideoShape(u32, u32),
    /// `present_video`: luma rows × luma stride of the picture.
    Present(usize, usize),
    /// `set_source_audio_params`: rate, channels.
    AudioShape(u32, u16),
    /// `queue_audio`: samples per channel.
    Queue(u32),
}

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Ev>>>);

impl OutputDriver for Recorder {
    fn present_video(&mut self, frame: &VideoFrame) -> Result<()> {
        let y = &frame.image_planes()[0];
        self.0
            .lock()
            .unwrap()
            .push(Ev::Present(y.data.len() / y.stride.max(1), y.stride));
        Ok(())
    }
    fn queue_audio(&mut self, frame: &AudioFrame) -> Result<()> {
        self.0.lock().unwrap().push(Ev::Queue(frame.samples));
        Ok(())
    }
    fn poll_events(&mut self) -> Vec<PlayerEvent> {
        Vec::new()
    }
    fn master_clock_pos(&self) -> Duration {
        Duration::ZERO
    }
    fn set_paused(&mut self, _paused: bool) {}
    fn set_volume(&mut self, _vol: f32) {}
    fn set_source_audio_params(&mut self, p: &CodecParameters) {
        self.0.lock().unwrap().push(Ev::AudioShape(
            p.sample_rate.unwrap_or(0),
            p.resolved_channels().unwrap_or(0),
        ));
    }
    fn set_source_video_params(&mut self, p: &CodecParameters) {
        self.0
            .lock()
            .unwrap()
            .push(Ev::VideoShape(p.width.unwrap_or(0), p.height.unwrap_or(0)));
    }
    fn video_drains_immediately(&self) -> bool {
        true
    }
}

fn fixture(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oxideav-tests/tests/fixtures/stream_params")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

/// Play `name` (its `kind` track only) to the end through the engine;
/// return what the driver saw.
fn play(name: &str, kind: &str) -> Vec<Ev> {
    let mut reg = Registries::new();
    oxideav_meta::register_all(&mut reg);
    let json = serde_json::json!({
        "@in": { "all": [{ "from": fixture(name) }] },
        "@display": { kind: [{ "from": "@in" }] },
    })
    .to_string();
    let job = Job::from_json(&json).expect("job");
    let (tx, rx) = mpsc::sync_channel::<EngineMsg>(64);
    let handle = Executor::new(&job, &reg)
        .with_codec_preferences(CodecPreferences {
            no_hardware: true,
            ..Default::default()
        })
        .with_sink_override("@display", Box::new(ChannelSink::new(tx)))
        .with_threads(2)
        .spawn()
        .expect("spawn");
    let streams = crate::wait_for_started(&rx).expect("started");
    let rec = Recorder::default();
    let log = rec.0.clone();
    PlayerEngine::new(
        Box::new(rec),
        handle,
        rx,
        &streams,
        None,
        None,
        Box::new(NoopMediaControls),
        TrackInfo::default(),
    )
    .run()
    .expect("playback");
    std::mem::take(&mut *log.lock().unwrap())
}

/// Collapse the video log into `(shape told, pictures)` runs, checking
/// each picture against the shape the driver was last told.
fn video_runs(log: &[Ev]) -> Vec<((u32, u32), usize)> {
    let mut shape = None;
    let mut runs: Vec<((u32, u32), usize)> = Vec::new();
    for ev in log {
        match *ev {
            Ev::VideoShape(w, h) => shape = Some((w, h)),
            Ev::Present(rows, stride) => {
                let (w, h) = shape.expect("a picture before any shape");
                assert_eq!(
                    rows, h as usize,
                    "picture presented with the wrong shape: {log:?}"
                );
                assert!(stride >= w as usize, "stride {stride} < {w}");
                match runs.last_mut() {
                    Some((s, n)) if *s == (w, h) => *n += 1,
                    _ => runs.push(((w, h), 1)),
                }
            }
            _ => {}
        }
    }
    runs
}

#[test]
fn h264_resize_replans_the_video_output_at_the_switch() {
    let log = play("h264_resize_64x48_96x64.h264", "video");
    assert_eq!(video_runs(&log), [((64, 48), 6), ((96, 64), 6)], "{log:?}");
}

#[test]
fn vp9_resize_replans_the_video_output_at_the_switch() {
    let log = play("vp9_resize_64x48_96x64.webm", "video");
    assert_eq!(video_runs(&log), [((64, 48), 5), ((96, 64), 5)], "{log:?}");
}

#[test]
fn mjpeg_mixed_sizes_replan_twice() {
    let log = play("mjpeg_mixed_64x48_48x32_64x48.avi", "video");
    assert_eq!(
        video_runs(&log),
        [((64, 48), 2), ((48, 32), 2), ((64, 48), 2)],
        "{log:?}"
    );
}

#[test]
fn mp3_rate_switch_replans_the_audio_output() {
    let log = play("mp3_rate_44100_32000.mka", "audio");
    let mut rate = None;
    let mut runs: Vec<(u32, usize)> = Vec::new();
    for ev in &log {
        match *ev {
            Ev::AudioShape(r, ch) => {
                assert_eq!(ch, 2, "{log:?}");
                rate = Some(r);
            }
            Ev::Queue(n) => {
                assert_eq!(n, 1152, "{log:?}");
                let r = rate.expect("audio before any shape");
                match runs.last_mut() {
                    Some((s, k)) if *s == r => *k += 1,
                    _ => runs.push((r, 1)),
                }
            }
            _ => {}
        }
    }
    assert_eq!(runs, [(44_100, 11), (32_000, 8)], "{log:?}");
}
