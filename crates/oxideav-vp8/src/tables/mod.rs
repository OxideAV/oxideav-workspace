//! VP8 fixed lookup tables — coefficient probabilities, segmentation/loop
//! filter trees, quantiser steps, etc. All numbers come straight from
//! RFC 6386. Each table lives in its own submodule.

pub mod coeff_probs;
pub mod mv;
pub mod prediction;
pub mod quant;
pub mod token_tree;
pub mod trees;
