//! Container registry.

use oxideav_core::{Error, Result, StreamInfo};
use std::collections::HashMap;

use crate::{Demuxer, Muxer, OpenDemuxerFn, OpenMuxerFn, ReadSeek, WriteSeek};

#[derive(Default)]
pub struct ContainerRegistry {
    demuxers: HashMap<String, OpenDemuxerFn>,
    muxers: HashMap<String, OpenMuxerFn>,
    /// Lowercase file extension → container name (e.g. "wav" → "wav").
    extensions: HashMap<String, String>,
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
}
