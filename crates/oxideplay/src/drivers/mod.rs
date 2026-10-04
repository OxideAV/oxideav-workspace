//! Output drivers split by concern. `VideoEngine` + `AudioEngine`
//! live in [`engine`]; concrete implementations sit under
//! `sdl2_*` (video + audio sharing [`sdl2_root`]), `winit_vo`, and
//! `sysaudio_ao`. The CLI picks one video + one audio engine at
//! runtime via `--vo` / `--ao` and hands them to
//! [`engine::Composite`].

// The conversion / routing helpers are only reached from the backends
// that use them; slimmer feature sets leave parts of them unused.
#[cfg_attr(not(all(feature = "sdl2", feature = "sysaudio")), allow(dead_code))]
pub mod audio_convert;
#[cfg_attr(not(feature = "sysaudio"), allow(dead_code))]
pub mod audio_routing;
pub mod engine;
pub mod hash_engines;
#[cfg_attr(not(feature = "sysaudio"), allow(dead_code))]
pub mod headphones_macos;
#[cfg_attr(not(any(feature = "winit", feature = "sdl2")), allow(dead_code))]
pub mod video_convert;

#[cfg(feature = "sdl2")]
pub mod sdl2_audio;
#[cfg(feature = "sdl2")]
pub mod sdl2_loader;
#[cfg(feature = "sdl2")]
pub mod sdl2_root;
#[cfg(feature = "sdl2")]
pub mod sdl2_video;

#[cfg(feature = "winit")]
pub mod winit_video;
#[cfg(feature = "winit")]
pub mod winit_vo;

// On-screen overlay UI. Built on egui + egui-wgpu, only meaningful
// alongside the winit/wgpu video pipeline.
#[cfg(feature = "egui")]
pub mod winit_overlay;

#[cfg(feature = "sysaudio")]
pub mod sysaudio_ao;
