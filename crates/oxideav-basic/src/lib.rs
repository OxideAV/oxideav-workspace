//! Bundled simple formats: codecs and containers that are small and standard
//! enough to share one crate. Anything larger gets its own crate.

pub mod pcm;
pub mod wav;

use oxideav_codec::CodecRegistry;
use oxideav_container::ContainerRegistry;

/// Register every codec provided by `oxideav-basic` in a [`CodecRegistry`].
pub fn register_codecs(reg: &mut CodecRegistry) {
    pcm::register(reg);
}

/// Register every container provided by `oxideav-basic` in a [`ContainerRegistry`].
pub fn register_containers(reg: &mut ContainerRegistry) {
    wav::register(reg);
}
