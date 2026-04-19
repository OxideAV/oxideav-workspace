//! `oxideplay` — reference media player built on `oxideav`.
//!
//! Every native backend it touches (SDL2, ALSA, PulseAudio, WASAPI,
//! CoreAudio, …) is loaded at runtime through `libloading`, so the
//! binary has no C library in its NEEDED list. The library half of
//! oxideav remains fully pure Rust.
//!
//! Video output and audio output are selected independently at
//! runtime via `--vo` and `--ao`:
//!
//! - `--vo` ∈ `auto` | `winit` | `sdl2` | `none`
//! - `--ao` ∈ `auto` | `sysaudio` | `sdl2` | `none` | any sysaudio
//!   driver name (`pulse`, `alsa`, `pipewire`, `oss`, `wasapi`,
//!   `asio`, `coreaudio`)

mod decode_worker;
mod driver;
mod drivers;
mod events;
mod job_sink;
mod player;
mod tui;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Parser;
use oxideav::Registries;
use oxideav_source::SourceRegistry;

use crate::driver::{OutputDriver, PlayerEvent};
use crate::drivers::engine::{AudioEngine, Composite, VideoEngine};
use crate::player::{Player, DEFAULT_BUFFER_BYTES};

#[derive(Parser)]
#[command(
    name = "oxideplay",
    version,
    about = "Play a media file via the oxideav library — pick video + audio outputs independently with --vo / --ao"
)]
struct Cli {
    /// Input media URI: a local path, file:// URL, or http(s):// URL.
    /// Not required when `--job` is given.
    #[arg(required_unless_present = "job")]
    input: Option<String>,

    /// Run a JSON-described job. The job must declare exactly one
    /// `@display` or `@out` sink — that sink is bound to the
    /// currently-selected video + audio engines (same as
    /// `--vo auto --ao auto`).
    /// `-` reads the JSON from stdin.
    #[arg(long)]
    job: Option<String>,

    /// Probe the input, print stream info, and exit without opening
    /// any output device.
    #[arg(long)]
    dry_run: bool,

    /// Start muted.
    #[arg(long)]
    mute: bool,

    /// Force audio-only mode even if the file has a video track.
    #[arg(long)]
    no_video: bool,

    /// Prefetch buffer size in MiB (default 64).
    #[arg(long, default_value_t = (DEFAULT_BUFFER_BYTES / (1 << 20)) as u32)]
    buffer_mib: u32,

    /// Thread budget for `--job` execution. `0` = auto (logical CPUs or
    /// the job's own `threads` field). Ignored in non-`--job` mode.
    #[arg(long, default_value_t = 0)]
    threads: usize,

    /// Video output driver. `auto` picks the first compiled-in option
    /// that initialises — winit > sdl2 > none. Pass `none` to force
    /// audio-only mode regardless of the source's video tracks.
    #[arg(long, default_value = "auto")]
    vo: String,

    /// Audio output driver. `auto` picks `oxideav-sysaudio`'s best
    /// backend when compiled in, falls back to SDL2 audio, else none.
    /// Accepts any sysaudio driver name directly (`pulse`, `alsa`,
    /// `pipewire`, `oss`, `wasapi`, `asio`, `coreaudio`), plus `sdl2`
    /// to force the SDL2 queue-based output, or `none` to mute
    /// without needing the audio stack up at all.
    #[arg(long, default_value = "auto")]
    ao: String,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxideplay: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Construct the composite driver from `--vo` and `--ao` selections.
/// Video and audio backends are chosen independently; the resulting
/// `Composite` implements the player's `OutputDriver` trait so the
/// rest of the code is oblivious to the split.
fn build_driver(
    vo: &str,
    ao: &str,
    sr: u32,
    ch: u16,
    video_dims: Option<(u32, u32)>,
) -> oxideav_core::Result<Box<dyn OutputDriver>> {
    let video = select_video(vo, video_dims)?;
    let audio = select_audio(ao, sr, ch)?;
    Ok(Box::new(Composite::new(video, audio)))
}

/// Resolve `--vo <name>` into a concrete `VideoEngine`. `video_dims`
/// is `None` when the caller forces audio-only mode.
fn select_video(
    name: &str,
    video_dims: Option<(u32, u32)>,
) -> oxideav_core::Result<Option<Box<dyn VideoEngine>>> {
    let Some(dims) = video_dims else {
        // No video stream / `--no-video`. Skip even spinning up a
        // window — `Composite` handles None cleanly.
        return Ok(None);
    };
    match name {
        "none" => Ok(None),
        "auto" => auto_video(dims),
        #[cfg(feature = "winit")]
        "winit" => Ok(Some(Box::new(
            crate::drivers::winit_vo::WinitVideoEngine::new(Some(dims))?,
        ))),
        #[cfg(feature = "sdl2")]
        "sdl2" => Ok(Some(Box::new(
            crate::drivers::sdl2_video::SdlVideoEngine::new(dims)?,
        ))),
        other => Err(oxideav_core::Error::invalid(format!(
            "--vo: unknown driver '{other}' (compiled in: {})",
            video_driver_list()
        ))),
    }
}

/// Resolve `--ao <name>` into a concrete `AudioEngine`.
fn select_audio(
    name: &str,
    sr: u32,
    ch: u16,
) -> oxideav_core::Result<Option<Box<dyn AudioEngine>>> {
    match name {
        "none" => Ok(None),
        "auto" => auto_audio(sr, ch),
        #[cfg(feature = "sysaudio")]
        "sysaudio" => Ok(Some(Box::new(
            crate::drivers::sysaudio_ao::SysAudioEngine::new(sr, ch)?,
        ))),
        #[cfg(feature = "sdl2")]
        "sdl2" => Ok(Some(Box::new(
            crate::drivers::sdl2_audio::SdlAudioEngine::new(sr, ch)?,
        ))),
        // Anything else is assumed to be a sysaudio driver name
        // (pulse / alsa / wasapi / coreaudio / …).
        #[cfg(feature = "sysaudio")]
        other => Ok(Some(Box::new(
            crate::drivers::sysaudio_ao::SysAudioEngine::with_driver(other, sr, ch)?,
        ))),
        #[cfg(not(feature = "sysaudio"))]
        other => Err(oxideav_core::Error::invalid(format!(
            "--ao: unknown driver '{other}' (compiled in: {})",
            audio_driver_list()
        ))),
    }
}

/// Auto-pick for `--vo auto`. Prefer winit when compiled in (it
/// handles high-DPI + wgpu acceleration better); fall back to SDL2.
#[allow(unused_variables)]
fn auto_video(
    dims: (u32, u32),
) -> oxideav_core::Result<Option<Box<dyn VideoEngine>>> {
    #[cfg(feature = "winit")]
    {
        if let Ok(v) = crate::drivers::winit_vo::WinitVideoEngine::new(Some(dims)) {
            return Ok(Some(Box::new(v)));
        }
    }
    #[cfg(feature = "sdl2")]
    {
        if let Ok(v) = crate::drivers::sdl2_video::SdlVideoEngine::new(dims) {
            return Ok(Some(Box::new(v)));
        }
    }
    // No video backend worked — fall through to audio-only.
    Ok(None)
}

/// Auto-pick for `--ao auto`. Prefer sysaudio (gives us actual latency
/// reporting + native APIs); fall back to SDL2 audio; else silent.
#[allow(unused_variables)]
fn auto_audio(
    sr: u32,
    ch: u16,
) -> oxideav_core::Result<Option<Box<dyn AudioEngine>>> {
    #[cfg(feature = "sysaudio")]
    {
        if let Ok(a) = crate::drivers::sysaudio_ao::SysAudioEngine::new(sr, ch) {
            return Ok(Some(Box::new(a)));
        }
    }
    #[cfg(feature = "sdl2")]
    {
        if let Ok(a) = crate::drivers::sdl2_audio::SdlAudioEngine::new(sr, ch) {
            return Ok(Some(Box::new(a)));
        }
    }
    Ok(None)
}

fn video_driver_list() -> &'static str {
    match (cfg!(feature = "winit"), cfg!(feature = "sdl2")) {
        (true, true) => "auto, winit, sdl2, none",
        (true, false) => "auto, winit, none",
        (false, true) => "auto, sdl2, none",
        (false, false) => "auto, none",
    }
}

#[allow(dead_code)]
fn audio_driver_list() -> &'static str {
    match (cfg!(feature = "sysaudio"), cfg!(feature = "sdl2")) {
        (true, true) => "auto, sysaudio, sdl2, <any sysaudio driver name>, none",
        (true, false) => "auto, sysaudio, <any sysaudio driver name>, none",
        (false, true) => "auto, sdl2, none",
        (false, false) => "auto, none",
    }
}

/// Build the source registry with the file driver and (when compiled in)
/// HTTP/HTTPS support.
fn build_sources() -> SourceRegistry {
    let mut reg = SourceRegistry::with_defaults();
    #[cfg(feature = "http")]
    {
        oxideav::http::register(&mut reg);
    }
    reg
}

fn run(cli: Cli) -> oxideav_core::Result<()> {
    let registries = Registries::with_all_features();
    let sources = build_sources();

    if let Some(job_src) = cli.job.as_deref() {
        return run_job(
            &registries,
            &sources,
            job_src,
            cli.mute,
            !cli.no_video,
            cli.threads,
        );
    }

    let input = cli
        .input
        .as_deref()
        .ok_or_else(|| oxideav_core::Error::invalid("no input URI (pass a path or --job)"))?;

    if cli.dry_run {
        return dry_run(&registries, &sources, input);
    }

    let want_video = !cli.no_video;
    let buffer_bytes = (cli.buffer_mib as usize).saturating_mul(1 << 20);

    let vo = cli.vo.clone();
    let ao = cli.ao.clone();
    let (mut play, media) = Player::open(
        &registries,
        &sources,
        input,
        buffer_bytes,
        |sr, ch, video_dims| {
            let video_dims = if want_video { video_dims } else { None };
            build_driver(&vo, &ao, sr, ch, video_dims)
        },
    )?;

    if cli.mute {
        play.driver.set_volume(0.0);
    }

    // Print stream summary to stderr so stdout is free for the TUI.
    eprintln!(
        "oxideplay: playing {} (format: {}){}",
        input,
        media.format_name,
        match &media.duration {
            Some(d) => format!(", duration {}", tui::format_duration(*d)),
            None => String::new(),
        }
    );
    if let Some(a) = &media.audio {
        eprintln!(
            "  audio: {} {}ch @ {} Hz",
            a.params.codec_id,
            a.params.channels.unwrap_or(0),
            a.params.sample_rate.unwrap_or(0)
        );
    }
    if let Some(v) = &media.video {
        eprintln!(
            "  video: {} {}x{}",
            v.params.codec_id,
            v.params.width.unwrap_or(0),
            v.params.height.unwrap_or(0)
        );
    }

    let tty = tui::stdout_is_tty();
    let mut tui_guard: Option<tui::TuiGuard> = if tty {
        tui::TuiGuard::enter().ok()
    } else {
        None
    };

    let mut last_status = Instant::now();
    // Assume seek is supported; flipped off lazily in `apply_event` on
    // first `Unsupported` error from the demuxer.
    let mut seek_supported = true;

    let result = run_loop(
        &mut play,
        &media,
        &mut tui_guard,
        &mut last_status,
        &mut seek_supported,
    );

    // Explicitly drop TUI guard before exit so terminal is restored.
    drop(tui_guard);

    result
}

fn run_loop<D: OutputDriver>(
    play: &mut Player<D>,
    media: &player::OpenedMedia,
    tui_guard: &mut Option<tui::TuiGuard>,
    last_status: &mut Instant,
    seek_supported: &mut bool,
) -> oxideav_core::Result<()> {
    let tick_interval = Duration::from_millis(16);
    let status_interval = Duration::from_secs(1);

    loop {
        // Gather events from driver + tui. tui::poll_events with a
        // zero-duration first poll now drains any pending key events
        // properly (Duration::ZERO is fine — it's the "non-blocking
        // check" mode, and the rest of the loop keeps ticking).
        let mut events = play.driver.poll_events();
        if tui_guard.is_some() {
            events.extend(tui::poll_events(Duration::ZERO));
        }
        let mut keep_going = true;
        for ev in events {
            if !play.apply_event(ev, seek_supported) {
                keep_going = false;
                break;
            }
        }
        if !keep_going {
            break;
        }

        // Drain the decode worker on every tick. Audio is queued to
        // SDL as fast as the worker produces it; the bounded per-
        // stream channels inside the worker provide back-pressure.
        let _ = play.pump_once()?;

        if play.eof_reached() && play.audio_drained() && !play.paused() {
            break;
        }

        // Status output.
        let now = Instant::now();
        let snap = play.timings();
        let tui_snap = tui::PlayerTimings {
            audio: snap.audio,
            video_decoded: snap.video_decoded,
            video_presented: snap.video_presented,
            video_queue_len: snap.video_queue_len,
        };
        let drift_str = tui::format_drift(snap.master, &tui_snap);
        if now.duration_since(*last_status) >= status_interval {
            if tui_guard.is_some() {
                let _ = tui::draw_status(
                    play.position(),
                    media.duration,
                    play.paused(),
                    play.volume(),
                    *seek_supported,
                    Some(&drift_str),
                );
            } else {
                let dur = media
                    .duration
                    .map(tui::format_duration)
                    .unwrap_or_else(|| "?".into());
                eprintln!(
                    "oxideplay: {} / {}  vol {:>3}%  {}{}",
                    tui::format_duration(play.position()),
                    dur,
                    (play.volume() * 100.0).round() as i32,
                    drift_str,
                    if play.paused() { "  [paused]" } else { "" },
                );
            }
            *last_status = now;
        } else if tui_guard.is_some() {
            // Still update the status bar frequently so time ticks smoothly.
            let _ = tui::draw_status(
                play.position(),
                media.duration,
                play.paused(),
                play.volume(),
                *seek_supported,
                Some(&drift_str),
            );
        }

        // Sleep a tick.
        std::thread::sleep(tick_interval);
    }
    Ok(())
}

fn run_job(
    registries: &Registries,
    sources: &SourceRegistry,
    job_src: &str,
    mute: bool,
    want_video: bool,
    threads: usize,
) -> oxideav_core::Result<()> {
    use oxideav::pipeline::{Executor, Job};

    // Load the job JSON.
    let raw = if job_src == "-" {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(job_src)?
    };
    let job = Job::from_json(&raw)?;
    job.validate()?;

    // Pick the first @display/@out target. No loop concurrency yet —
    // playback is fire-and-forget (no pause/seek).
    let target = ["@display", "@out"]
        .iter()
        .find(|k| job.outputs.contains_key(**k))
        .copied()
        .ok_or_else(|| {
            oxideav_core::Error::invalid(
                "oxideplay --job: expected a @display or @out output in the job",
            )
        })?;

    let sink = Box::new(job_sink::PlayerSink::new(mute, want_video));
    let stats = Executor::new(&job, &registries.codecs, &registries.containers, sources)
        .with_sink_override(target, sink)
        .with_threads(threads)
        .run()?;
    eprintln!(
        "oxideplay: job finished ({} pkts read, {} frames decoded, {} frames played)",
        stats.packets_read, stats.frames_decoded, stats.frames_written
    );
    Ok(())
}

fn dry_run(
    registries: &Registries,
    sources: &SourceRegistry,
    input: &str,
) -> oxideav_core::Result<()> {
    let media = player::probe(registries, sources, input)?;
    println!("Input: {input}");
    println!("Format: {}", media.format_name);
    if let Some(d) = media.duration {
        println!("Duration: {}", tui::format_duration(d));
    }
    if let Some(a) = &media.audio {
        println!(
            "Audio: stream #{} codec={} channels={} rate={}",
            a.index,
            a.params.codec_id,
            a.params.channels.unwrap_or(0),
            a.params.sample_rate.unwrap_or(0),
        );
    }
    if let Some(v) = &media.video {
        println!(
            "Video: stream #{} codec={} {}x{}",
            v.index,
            v.params.codec_id,
            v.params.width.unwrap_or(0),
            v.params.height.unwrap_or(0),
        );
    }
    if media.audio.is_none() && media.video.is_none() {
        println!("(no audio or video streams)");
    }
    let _ = PlayerEvent::Quit; // suppress unused warning
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_help_builds() {
        Cli::command().debug_assert();
    }

    #[test]
    fn cli_parses_dry_run() {
        let cli = Cli::try_parse_from(["oxideplay", "--dry-run", "x.mp4"]).unwrap();
        assert!(cli.dry_run);
        assert_eq!(cli.input.as_deref(), Some("x.mp4"));
    }
}
