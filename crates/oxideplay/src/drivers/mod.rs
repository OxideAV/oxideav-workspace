//! Output drivers split by concern. `VideoEngine` + `AudioEngine`
//! live in [`engine`]; concrete implementations sit under
//! `sdl2_*` (video + audio sharing [`sdl2_root`]), `winit_vo`, and
//! `sysaudio_ao`. The CLI picks one video + one audio engine at
//! runtime via `--vo` / `--ao` and hands them to
//! [`engine::Composite`].

pub mod audio_convert;
pub mod engine;
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

#[cfg(feature = "sysaudio")]
pub mod sysaudio_ao;
