//! `JobSink` implementation that forwards executor events to the
//! main-thread [`crate::engine::PlayerEngine`] via a bounded channel.
//!
//! This is the only sink oxideplay registers — both plain playback
//! (`oxideplay file.mp4`) and `--job` / `--inline` flow through the
//! same path. The executor runs on a worker thread, the engine runs
//! on the main thread, the bounded channel between them provides
//! natural pause/back-pressure.

use std::sync::mpsc::SyncSender;

use oxideav::pipeline::{BarrierKind, JobSink};
use oxideav_core::{Error, Frame, MediaType, Packet, Result, StreamInfo};

use crate::engine::EngineMsg;

/// Cross-thread sink: forwards every JobSink callback into a
/// `SyncSender<EngineMsg>` consumed by [`crate::engine::PlayerEngine`].
///
/// Holds no driver / non-Send state — driver ownership lives entirely
/// on the main thread inside the engine.
pub struct ChannelSink {
    tx: SyncSender<EngineMsg>,
    debug: bool,
    seen_audio: usize,
    seen_video: usize,
}

impl ChannelSink {
    pub fn new(tx: SyncSender<EngineMsg>) -> Self {
        let debug = std::env::var("OXIDEPLAY_SINK_DEBUG")
            .ok()
            .filter(|v| !v.is_empty() && v != "0")
            .is_some();
        Self {
            tx,
            debug,
            seen_audio: 0,
            seen_video: 0,
        }
    }
}

impl JobSink for ChannelSink {
    fn start(&mut self, streams: &[StreamInfo]) -> Result<()> {
        if self.debug {
            eprintln!("[sink] start: {} streams", streams.len());
        }
        self.tx
            .send(EngineMsg::Started(streams.to_vec()))
            .map_err(|_| Error::other("oxideplay: engine receiver dropped before start"))
    }

    fn write_packet(&mut self, _kind: MediaType, _pkt: &Packet) -> Result<()> {
        // The `@display` reserved sink consumes raw frames. Any
        // path that delivers packets here has been mis-configured
        // (e.g. user wrote `codec: copy` for a file output that's
        // actually pointed at the player). Fail loudly with the
        // same message the legacy `PlayerSink` used.
        Err(Error::unsupported(
            "oxideplay: @display sink needs decoded frames; \
             remove `codec` or set it to the source codec with a decoder",
        ))
    }

    fn write_frame(&mut self, kind: MediaType, frame: &Frame) -> Result<()> {
        if self.debug {
            match frame {
                Frame::Audio(af) => {
                    self.seen_audio += 1;
                    if self.seen_audio <= 5 || self.seen_audio % 50 == 0 {
                        eprintln!(
                            "[sink] audio frame #{} samples={} pts={:?}",
                            self.seen_audio, af.samples, af.pts
                        );
                    }
                }
                Frame::Video(vf) => {
                    self.seen_video += 1;
                    if self.seen_video <= 5 || self.seen_video % 50 == 0 {
                        eprintln!(
                            "[sink] video frame #{} planes={} pts={:?}",
                            self.seen_video,
                            vf.planes.len(),
                            vf.pts
                        );
                    }
                }
                _ => {}
            }
        }
        self.tx
            .send(EngineMsg::Frame {
                kind,
                frame: frame.clone(),
            })
            .map_err(|_| Error::other("oxideplay: engine receiver dropped"))
    }

    fn barrier(&mut self, kind: BarrierKind) -> Result<()> {
        if self.debug {
            eprintln!("[sink] barrier: {:?}", kind);
        }
        self.tx
            .send(EngineMsg::Barrier(kind))
            .map_err(|_| Error::other("oxideplay: engine receiver dropped during barrier"))
    }

    fn finish(&mut self) -> Result<()> {
        if self.debug {
            eprintln!(
                "[sink] finish: total audio={} video={}",
                self.seen_audio, self.seen_video
            );
        }
        // Best-effort: if the engine has already exited, swallow
        // the disconnection error so the executor can wind down
        // cleanly.
        let _ = self.tx.send(EngineMsg::Finished);
        Ok(())
    }
}
