//! Container registry.

use oxideav_core::{Error, Result, StreamInfo};
use std::collections::HashMap;
use std::io::SeekFrom;

use crate::{
    Demuxer, Muxer, OpenDemuxerFn, OpenMuxerFn, ProbeData, ProbeFn, ProbeScore, ReadSeek,
    WriteSeek, PROBE_SCORE_EXTENSION,
};

#[derive(Default)]
pub struct ContainerRegistry {
    demuxers: HashMap<String, OpenDemuxerFn>,
    muxers: HashMap<String, OpenMuxerFn>,
    /// Lowercase file extension → container name (e.g. "wav" → "wav").
    extensions: HashMap<String, String>,
    /// Container name → content-probe function. Optional — containers
    /// without a probe still work but require an extension hint or an
    /// explicit format name.
    probes: HashMap<String, ProbeFn>,
}

impl ContainerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_demuxer(&mut self, name: &str, open: OpenDemuxerFn) {
        self.demuxers.insert(name.to_owned(), open);
    }

    pub fn register_muxer(&mut self, name: &str, open: OpenMuxerFn) {
        self.muxers.insert(name.to_owned(), open);
    }

    pub fn register_extension(&mut self, ext: &str, container_name: &str) {
        self.extensions
            .insert(ext.to_lowercase(), container_name.to_owned());
    }

    /// Attach a content-based probe to a registered demuxer. Called by
    /// the registry's [`probe_input`](Self::probe_input) to detect the
    /// container format from the first few KB of an input stream.
    pub fn register_probe(&mut self, container_name: &str, probe: ProbeFn) {
        self.probes.insert(container_name.to_owned(), probe);
    }

    pub fn demuxer_names(&self) -> impl Iterator<Item = &str> {
        self.demuxers.keys().map(|s| s.as_str())
    }

    pub fn muxer_names(&self) -> impl Iterator<Item = &str> {
        self.muxers.keys().map(|s| s.as_str())
    }

    /// Open a demuxer explicitly by format name.
    pub fn open_demuxer(&self, name: &str, input: Box<dyn ReadSeek>) -> Result<Box<dyn Demuxer>> {
        let open = self
            .demuxers
            .get(name)
            .ok_or_else(|| Error::FormatNotFound(name.to_owned()))?;
        open(input)
    }

    /// Open a muxer by format name.
    pub fn open_muxer(
        &self,
        name: &str,
        output: Box<dyn WriteSeek>,
        streams: &[StreamInfo],
    ) -> Result<Box<dyn Muxer>> {
        let open = self
            .muxers
            .get(name)
            .ok_or_else(|| Error::FormatNotFound(name.to_owned()))?;
        open(output, streams)
    }

    /// Look up a container name from a file extension (no leading dot).
    pub fn container_for_extension(&self, ext: &str) -> Option<&str> {
        self.extensions.get(&ext.to_lowercase()).map(|s| s.as_str())
    }

    /// Detect the container format by reading the first ~256 KB of the
    /// input, scoring each registered probe, and returning the highest-
    /// scoring container's name. The extension is passed to probes as a
    /// hint — they may use it to break ties when their signature is weak.
    ///
    /// Falls back to the extension table if no probe scores above zero.
    /// The input cursor is restored to its starting position on success
    /// and on the I/O failure paths that allow it.
    pub fn probe_input(&self, input: &mut dyn ReadSeek, ext_hint: Option<&str>) -> Result<String> {
        const PROBE_BUF_SIZE: usize = 256 * 1024;

        let saved_pos = input.stream_position()?;
        input.seek(SeekFrom::Start(0))?;
        let mut buf = vec![0u8; PROBE_BUF_SIZE];
        let mut got = 0;
        while got < buf.len() {
            match input.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = input.seek(SeekFrom::Start(saved_pos));
                    return Err(e.into());
                }
            }
        }
        buf.truncate(got);
        input.seek(SeekFrom::Start(saved_pos))?;

        let ext_lower = ext_hint.map(|s| s.to_ascii_lowercase());
        let probe_data = ProbeData {
            buf: &buf,
            ext: ext_lower.as_deref(),
        };

        let mut best: Option<(&str, ProbeScore)> = None;
        for (name, probe) in &self.probes {
            let score = probe(&probe_data);
            if score == 0 {
                continue;
            }
            match best {
                Some((_, prev)) if score <= prev => {}
                _ => best = Some((name.as_str(), score)),
            }
        }
        if let Some((name, _)) = best {
            return Ok(name.to_owned());
        }

        // Fall back to extension lookup with the conventional weak score.
        if let Some(ext) = ext_hint {
            if let Some(name) = self.container_for_extension(ext) {
                let _ = PROBE_SCORE_EXTENSION; // export retained for symmetry
                return Ok(name.to_owned());
            }
        }

        Err(Error::FormatNotFound(
            "no registered demuxer recognises this input".into(),
        ))
    }
}
