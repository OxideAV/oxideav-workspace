//! In-process codec registry.

use oxideav_core::{CodecId, CodecParameters, Error, Result};
use std::collections::HashMap;

use crate::{Decoder, DecoderFactory, Encoder, EncoderFactory};

#[derive(Default)]
pub struct CodecRegistry {
    decoders: HashMap<CodecId, DecoderFactory>,
    encoders: HashMap<CodecId, EncoderFactory>,
}

impl CodecRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_decoder(&mut self, id: CodecId, factory: DecoderFactory) {
        self.decoders.insert(id, factory);
    }

    pub fn register_encoder(&mut self, id: CodecId, factory: EncoderFactory) {
        self.encoders.insert(id, factory);
    }

    pub fn has_decoder(&self, id: &CodecId) -> bool {
        self.decoders.contains_key(id)
    }

    pub fn has_encoder(&self, id: &CodecId) -> bool {
        self.encoders.contains_key(id)
    }

    pub fn make_decoder(&self, params: &CodecParameters) -> Result<Box<dyn Decoder>> {
        let factory = self
            .decoders
            .get(&params.codec_id)
            .ok_or_else(|| Error::CodecNotFound(params.codec_id.to_string()))?;
        factory(params)
    }

    pub fn make_encoder(&self, params: &CodecParameters) -> Result<Box<dyn Encoder>> {
        let factory = self
            .encoders
            .get(&params.codec_id)
            .ok_or_else(|| Error::CodecNotFound(params.codec_id.to_string()))?;
        factory(params)
    }

    pub fn decoder_ids(&self) -> impl Iterator<Item = &CodecId> {
        self.decoders.keys()
    }

    pub fn encoder_ids(&self) -> impl Iterator<Item = &CodecId> {
        self.encoders.keys()
    }
}
