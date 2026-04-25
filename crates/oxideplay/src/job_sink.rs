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
}

impl ChannelSink {
    pub fn new(tx: SyncSender<EngineMsg>) -> Self {
        Self { tx }
    }
}

impl JobSink for ChannelSink {
    fn start(&mut self, streams: &[StreamInfo]) -> Result<()> {
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
        // Cloning the frame is the price for crossing the thread
        // boundary. AudioFrame / VideoFrame are mostly Vec<u8>
        // payloads — Box<[u8]> would be cheaper but the existing
        // Frame variants own their backing storage. Acceptable for
        // a real-time player; transcode jobs don't go through here.
        self.tx
            .send(EngineMsg::Frame {
                kind,
                frame: frame.clone(),
            })
            .map_err(|_| Error::other("oxideplay: engine receiver dropped"))
    }

    fn barrier(&mut self, kind: BarrierKind) -> Result<()> {
        self.tx
            .send(EngineMsg::Barrier(kind))
            .map_err(|_| Error::other("oxideplay: engine receiver dropped during barrier"))
    }

    fn finish(&mut self) -> Result<()> {
        // Best-effort: if the engine has already exited, swallow
        // the disconnection error so the executor can wind down
        // cleanly.
        let _ = self.tx.send(EngineMsg::Finished);
        Ok(())
    }
}
