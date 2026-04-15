//! Aggregator crate for oxideav.
//!
//! Depend on this crate to pull in codecs and containers from the wider
//! oxideav ecosystem, gated by Cargo features. Each format crate maps to
//! exactly one feature here:
//!
//! ```toml
//! [dependencies]
//! oxideav = { version = "*", features = ["basic"] }
//! ```
//!
//! Today only `basic` (the bundled simple formats) exists; more features land
//! as per-format crates do.

pub use oxideav_codec as codec;
pub use oxideav_container as container;
pub use oxideav_core as core;
pub use oxideav_pipeline as pipeline;

#[cfg(feature = "basic")]
pub use oxideav_basic as basic;

/// A pair of registries populated with every format enabled at build time.
pub struct Registries {
    pub codecs: oxideav_codec::CodecRegistry,
    pub containers: oxideav_container::ContainerRegistry,
}

impl Registries {
    /// Build a fresh set of registries containing all compiled-in formats.
    pub fn with_all_features() -> Self {
        let mut codecs = oxideav_codec::CodecRegistry::new();
        let mut containers = oxideav_container::ContainerRegistry::new();

        #[cfg(feature = "basic")]
        {
            oxideav_basic::register_codecs(&mut codecs);
            oxideav_basic::register_containers(&mut containers);
        }

        Self { codecs, containers }
    }
}

impl Default for Registries {
    fn default() -> Self {
        Self::with_all_features()
    }
}
