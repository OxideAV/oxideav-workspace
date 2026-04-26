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

mod driver;
mod drivers;
mod engine;
mod events;
mod job_sink;
mod media_controls;
mod tui;

use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use clap::Parser;
use oxideav::pipeline::{Executor, Job};
use oxideav::{Registries, RuntimeContextExt};
use oxideav_source::SourceRegistry;
use serde_json::json;

use crate::driver::OutputDriver;
use crate::drivers::audio_routing::{DownmixMode, DownmixPolicy};
use crate::drivers::engine::{AudioEngine, Composite, VideoEngine};
use crate::engine::{EngineMsg, PlayerEngine};
use crate::job_sink::ChannelSink;
use crate::media_controls::{track_info_from_demuxer, TrackInfo};

/// Default prefetch buffer for playback (bytes). Sized to absorb a few
/// seconds of typical home-broadband jitter on HD streams.
pub const DEFAULT_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// Bounded depth of the executor → engine frame channel. Big enough
/// to absorb decoder bursts; small enough that a paused engine
/// back-pressures the executor within a single audio packet.
const FRAME_CAP: usize = 32;

#[derive(Parser)]
#[command(
    name = "oxideplay",
    version,
    about = "Play a media file via the oxideav library — pick video + audio outputs independently with --vo / --ao"
)]
struct Cli {
    /// Input media URI: a local path, file:// URL, or http(s):// URL.
    /// Not required when `--job` or `--inline` is given.
    #[arg(required_unless_present_any = ["job", "inline"])]
    input: Option<String>,

    /// Run a JSON-described job. The job must declare exactly one
    /// `@display` or `@out` sink — that sink is bound to the
    /// currently-selected video + audio engines (same as
    /// `--vo auto --ao auto`).
    /// `-` reads the JSON from stdin.
    #[arg(long, conflicts_with = "inline")]
    job: Option<String>,

    /// Inline JSON job description. Parallel to `--job <file>`; pass
    /// the JSON literal on the command line instead of a file path.
    #[arg(long)]
    inline: Option<String>,

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

    /// Thread budget for executor execution. `0` = auto (logical CPUs
    /// or the job's own `threads` field).
    #[arg(long, default_value_t = 0)]
    threads: usize,

    /// Video output driver. `auto` picks the first compiled-in option
    /// that initialises — winit > sdl2 > null. `null` (or `none`)
    /// disables video entirely. Pass `help` to list compiled drivers.
    #[arg(long, default_value = "auto")]
    vo: String,

    /// Audio output driver. `auto` picks `oxideav-sysaudio`'s best
    /// backend when compiled in, falls back to SDL2 audio, else null.
    /// Accepts any sysaudio driver name directly (`pulse`, `alsa`,
    /// `pipewire`, `oss`, `wasapi`, `asio`, `coreaudio`), plus `sdl2`
    /// to force the SDL2 queue-based output. `null` (or `none`)
    /// disables audio entirely. Pass `help` to list compiled drivers.
    #[arg(long, default_value = "auto")]
    ao: String,

    /// Force a specific surround-to-stereo downmix. Values:
    /// `loro` (Lo/Ro per ATSC A/52, default for speakers),
    /// `ltrt` (Lt/Rt Pro Logic matrix), `binaural` (HRTF-style for
    /// headphones), `average` (channel mean — fallback). Without this
    /// flag the player picks automatically based on the source
    /// layout, the device's channel count, and (on macOS) headphone
    /// detection. Conflicts with `--no-downmix`.
    #[arg(long, value_name = "MODE", conflicts_with = "no_downmix")]
    downmix: Option<String>,

    /// Refuse to ever downmix. The player will fail to open the
    /// device if the source layout doesn't fit the device. Useful for
    /// a 5.1 receiver hookup where any downmix would be wrong.
    #[arg(long)]
    no_downmix: bool,
}

fn main() -> ExitCode {
    if let Some(which) = wants_driver_help(&std::env::args().collect::<Vec<_>>()) {
        match which {
            DriverHelp::Vo => print_vo_help(),
            DriverHelp::Ao => print_ao_help(),
        }
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxideplay: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Copy, Clone)]
enum DriverHelp {
    Vo,
    Ao,
}

/// Scan argv for `--vo help`, `--vo=help`, `--ao help`, or `--ao=help`
/// (case-insensitive). Returns which help screen was requested; `None`
/// means there's nothing to short-circuit.
fn wants_driver_help(argv: &[String]) -> Option<DriverHelp> {
    let is_help = |v: &str| v.eq_ignore_ascii_case("help") || v == "?";
    let mut iter = argv.iter().skip(1).peekable();
    while let Some(arg) = iter.next() {
        if let Some(rest) = arg.strip_prefix("--vo=") {
            if is_help(rest) {
                return Some(DriverHelp::Vo);
            }
        } else if let Some(rest) = arg.strip_prefix("--ao=") {
            if is_help(rest) {
                return Some(DriverHelp::Ao);
            }
        } else if arg == "--vo" {
            if let Some(next) = iter.peek() {
                if is_help(next) {
                    return Some(DriverHelp::Vo);
                }
            }
        } else if arg == "--ao" {
            if let Some(next) = iter.peek() {
                if is_help(next) {
                    return Some(DriverHelp::Ao);
                }
            }
        }
    }
    None
}

fn print_vo_help() {
    println!("Video outputs (--vo):");
    println!(
        "  {:<10} pick the first compiled-in option that initialises",
        "auto"
    );
    #[cfg(feature = "winit")]
    println!("  {:<10} winit windowing + wgpu YUV→RGB", "winit");
    #[cfg(feature = "sdl2")]
    println!("  {:<10} SDL2 video (libSDL2 via libloading)", "sdl2");
    println!(
        "  {:<10} disable video (skip decoder; demuxer drops video packets)",
        "null"
    );
    println!("  {:<10} synonym for `null`", "none");
}

#[allow(unused_mut)]
fn print_ao_help() {
    println!("Audio outputs (--ao):");
    println!(
        "  {:<10} pick the first working backend (sysaudio > sdl2)",
        "auto"
    );
    #[cfg(feature = "sysaudio")]
    println!(
        "  {:<10} oxideav-sysaudio default (see driver list below)",
        "sysaudio"
    );
    #[cfg(feature = "sdl2")]
    println!("  {:<10} SDL2 audio (libSDL2 via libloading)", "sdl2");
    println!(
        "  {:<10} disable audio (skip decoder; no device open)",
        "null"
    );
    println!("  {:<10} synonym for `null`", "none");

    #[cfg(feature = "sysaudio")]
    {
        let probed: Vec<_> = oxideav_sysaudio::probe()
            .into_iter()
            .map(|d| d.name().to_string())
            .collect();
        println!();
        println!("sysaudio drivers (usable as --ao <name>):");
        for d in oxideav_sysaudio::drivers() {
            let status = if probed.iter().any(|n| n == d.name()) {
                "[ok]"
            } else {
                "[unavailable]"
            };
            println!("  {:<10} {:<13} {}", d.name(), status, d.description());
        }
    }
}

/// Construct the composite driver from `--vo` and `--ao` selections.
pub(crate) fn build_driver(
    vo: &str,
    ao: &str,
    sr: u32,
    ch: u16,
    video_dims: Option<(u32, u32)>,
    downmix_policy: DownmixPolicy,
) -> oxideav_core::Result<Box<dyn OutputDriver>> {
    let video = select_video(vo, video_dims)?;
    let audio = select_audio(ao, sr, ch, downmix_policy)?;
    Ok(Box::new(Composite::new(video, audio)))
}

/// Resolve the user's CLI flags into a single [`DownmixPolicy`]. The
/// CLI guarantees `--downmix` and `--no-downmix` are mutually exclusive
/// so the precedence here is unambiguous.
pub(crate) fn resolve_downmix_policy(
    downmix: Option<&str>,
    no_downmix: bool,
) -> oxideav_core::Result<DownmixPolicy> {
    if no_downmix {
        return Ok(DownmixPolicy::Forbid);
    }
    match downmix {
        None => Ok(DownmixPolicy::Auto),
        Some(s) => {
            let mode: DownmixMode = s.parse().map_err(oxideav_core::Error::invalid)?;
            Ok(DownmixPolicy::Force(mode))
        }
    }
}

fn is_null_sink(name: &str) -> bool {
    matches!(name, "none" | "null")
}

fn select_video(
    name: &str,
    video_dims: Option<(u32, u32)>,
) -> oxideav_core::Result<Option<Box<dyn VideoEngine>>> {
    if is_null_sink(name) {
        return Ok(None);
    }
    let Some(dims) = video_dims else {
        return Ok(None);
    };
    match name {
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

fn select_audio(
    name: &str,
    sr: u32,
    ch: u16,
    policy: DownmixPolicy,
) -> oxideav_core::Result<Option<Box<dyn AudioEngine>>> {
    if is_null_sink(name) {
        return Ok(None);
    }
    match name {
        "auto" => auto_audio(sr, ch, policy),
        #[cfg(feature = "sysaudio")]
        "sysaudio" => Ok(Some(Box::new(
            crate::drivers::sysaudio_ao::SysAudioEngine::new_with_policy(sr, ch, policy)?,
        ))),
        #[cfg(feature = "sdl2")]
        "sdl2" => Ok(Some(Box::new(
            crate::drivers::sdl2_audio::SdlAudioEngine::new(sr, ch)?,
        ))),
        #[cfg(feature = "sysaudio")]
        other => Ok(Some(Box::new(
            crate::drivers::sysaudio_ao::SysAudioEngine::with_driver_and_policy(
                other, sr, ch, policy,
            )?,
        ))),
        #[cfg(not(feature = "sysaudio"))]
        other => Err(oxideav_core::Error::invalid(format!(
            "--ao: unknown driver '{other}' (compiled in: {})",
            audio_driver_list()
        ))),
    }
}

#[allow(unused_variables)]
fn auto_video(dims: (u32, u32)) -> oxideav_core::Result<Option<Box<dyn VideoEngine>>> {
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
    Ok(None)
}

#[allow(unused_variables)]
fn auto_audio(
    sr: u32,
    ch: u16,
    policy: DownmixPolicy,
) -> oxideav_core::Result<Option<Box<dyn AudioEngine>>> {
    #[cfg(feature = "sysaudio")]
    {
        if let Ok(a) = crate::drivers::sysaudio_ao::SysAudioEngine::new_with_policy(sr, ch, policy)
        {
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
        (true, true) => "auto, winit, sdl2, null, none",
        (true, false) => "auto, winit, null, none",
        (false, true) => "auto, sdl2, null, none",
        (false, false) => "auto, null, none",
    }
}

#[allow(dead_code)]
fn audio_driver_list() -> &'static str {
    match (cfg!(feature = "sysaudio"), cfg!(feature = "sdl2")) {
        (true, true) => "auto, sysaudio, sdl2, <any sysaudio driver name>, null, none",
        (true, false) => "auto, sysaudio, <any sysaudio driver name>, null, none",
        (false, true) => "auto, sdl2, null, none",
        (false, false) => "auto, null, none",
    }
}

fn run(cli: Cli) -> oxideav_core::Result<()> {
    if cli.dry_run {
        let registries = Registries::with_all_features();
        let input = cli
            .input
            .as_deref()
            .ok_or_else(|| oxideav_core::Error::invalid("dry-run requires an input path"))?;
        return dry_run(&registries, &registries.sources, input);
    }

    let registries = Registries::with_all_features();

    // Load or synthesise the job JSON. Plain playback emits a trivial
    // `@in → @display` graph so it goes through the same Executor
    // path every other invocation uses.
    let job_json = if let Some(j) = cli.inline.clone() {
        j
    } else if let Some(p) = cli.job.as_deref() {
        read_job_source(p)?
    } else {
        let input = cli
            .input
            .as_deref()
            .ok_or_else(|| oxideav_core::Error::invalid("no input URI (pass a path or --job)"))?;
        synthesise_playback_job(input)?
    };

    let job = Job::from_json(&job_json)?;
    job.validate()?;

    // Identify the @display/@out target. We always bind a ChannelSink
    // there — file outputs are handled by the executor's own FileSink.
    let target = ["@display", "@out"]
        .iter()
        .find(|k| job.outputs.contains_key(**k))
        .copied()
        .ok_or_else(|| {
            oxideav_core::Error::invalid(
                "oxideplay: job must declare a @display or @out output (plain playback synthesises one automatically)",
            )
        })?;

    // Set up the executor → engine channel + the sink.
    let (tx, rx) = mpsc::sync_channel::<EngineMsg>(FRAME_CAP);
    let sink = Box::new(ChannelSink::new(tx));

    // Spawn the executor on a background thread. From now on, the
    // engine drives — driver lifetime stays on the main thread (winit
    // mandates that on macOS).
    let handle = Executor::new(&job, &registries)
        .with_sink_override(target, sink)
        .with_threads(cli.threads)
        .spawn()?;

    // Wait for the first message: must be Started(streams). The
    // executor runs on a worker thread so this only blocks until the
    // demuxer has resolved its streams.
    let streams = wait_for_started(&rx)?;

    // Build the driver from what the streams + CLI flags actually
    // permit. This MUST happen on the main thread so winit's NSWindow
    // is owned correctly.
    let want_video = !cli.no_video && !is_null_sink(&cli.vo);
    let want_audio = !is_null_sink(&cli.ao);
    let (sr, ch, video_dims) = derive_driver_params(&streams, want_video);

    let downmix_policy = resolve_downmix_policy(cli.downmix.as_deref(), cli.no_downmix)?;

    let driver = build_driver(
        if want_audio || want_video {
            &cli.vo
        } else {
            "null"
        },
        if want_audio { &cli.ao } else { "null" },
        sr,
        ch,
        video_dims,
        downmix_policy,
    )?;

    // Print stream summary on stderr (TUI owns stdout).
    print_status_block(&streams, driver.engine_info());

    // Compute total duration if the source advertised one. Stream
    // metadata is what the demuxer/probe gave us; not always present.
    let duration = derive_duration(&streams);

    // Optional TUI guard.
    let tui_guard = if tui::stdout_is_tty() {
        tui::TuiGuard::enter().ok()
    } else {
        None
    };

    // Pre-extract metadata + cover art on the main thread so the
    // OS Now Playing widget can be populated. The executor opens
    // its OWN demuxer copy on the worker thread; this side-open
    // is throw-away. Skipped on `--inline` / `--job` paths because
    // the input there is synthesised and may not even be a single
    // file. Failures are non-fatal — we fall back to filename-only.
    let track_info = if cli.inline.is_none() && cli.job.is_none() {
        cli.input
            .as_deref()
            .map(|input| extract_track_info(&registries, input, duration))
            .unwrap_or_default()
    } else {
        TrackInfo::default()
    };

    let media_controls = media_controls::build();

    let mut engine = PlayerEngine::new(
        driver,
        handle,
        rx,
        &streams,
        duration,
        tui_guard,
        media_controls,
        track_info,
    );
    if cli.mute {
        engine.set_muted(true);
    }
    engine.run()
}

/// Pre-open the input on the main thread to read its metadata and
/// the first attached picture. This is wasteful in the sense that
/// the executor will re-open it for actual playback, but the cost
/// is bounded (probe + parse the container header, then close)
/// and lets us populate `Now Playing` before the first sample
/// reaches the device.
///
/// Returns a default `TrackInfo` (filename-only title) on any
/// error — the player should never fail because the OS widget
/// couldn't be filled in.
fn extract_track_info(
    registries: &Registries,
    input: &str,
    total_duration: Option<Duration>,
) -> TrackInfo {
    use oxideav_core::ReadSeek;
    use oxideav_source::BufferedSource;

    let fallback_title = filename_stem_from_uri(input);

    let raw = match registries.sources.open(input) {
        Ok(s) => s,
        Err(_) => {
            return TrackInfo {
                title: fallback_title,
                duration: total_duration,
                ..Default::default()
            };
        }
    };
    let buffered = match BufferedSource::new(raw, 1 << 20) {
        Ok(b) => b,
        Err(_) => {
            return TrackInfo {
                title: fallback_title,
                duration: total_duration,
                ..Default::default()
            };
        }
    };
    let mut handle: Box<dyn ReadSeek> = Box::new(buffered);
    let ext = ext_from_uri(input);
    let format = match registries
        .containers
        .probe_input(&mut *handle, ext.as_deref())
    {
        Ok(f) => f,
        Err(_) => {
            return TrackInfo {
                title: fallback_title,
                duration: total_duration,
                ..Default::default()
            };
        }
    };
    let demuxer = match registries
        .containers
        .open_demuxer(&format, handle, &registries.codecs)
    {
        Ok(d) => d,
        Err(_) => {
            return TrackInfo {
                title: fallback_title,
                duration: total_duration,
                ..Default::default()
            };
        }
    };
    track_info_from_demuxer(
        demuxer.metadata(),
        demuxer.attached_pictures(),
        fallback_title,
        total_duration,
    )
}

/// Pull a human-readable title out of a URI (last path segment,
/// stripped of extension). Used as the Now Playing fallback when
/// the container itself carries no title metadata. `None` if the
/// URI is empty or has no useful tail.
fn filename_stem_from_uri(uri: &str) -> Option<String> {
    let last_segment = uri.rsplit('/').next().unwrap_or(uri);
    let last_segment = last_segment.split('?').next().unwrap_or(last_segment);
    if last_segment.is_empty() {
        return None;
    }
    let stem = last_segment
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(last_segment);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

/// Wait for the first `EngineMsg`. It MUST be `Started(streams)`.
/// Anything else is a protocol error from the sink.
fn wait_for_started(
    rx: &Receiver<EngineMsg>,
) -> oxideav_core::Result<Vec<oxideav_core::StreamInfo>> {
    // 30s is generous — it covers slow HTTP probes on cold cache.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(oxideav_core::Error::other(
                "oxideplay: timed out waiting for executor to report streams",
            ));
        }
        match rx.recv_timeout(remaining.min(Duration::from_secs(1))) {
            Ok(EngineMsg::Started(s)) => return Ok(s),
            Ok(EngineMsg::Finished) => {
                return Err(oxideav_core::Error::other(
                    "oxideplay: executor finished before producing streams",
                ));
            }
            Ok(_) => {
                // Frame / Barrier before Started shouldn't happen.
                // Skip defensively; the engine will tolerate it too.
                continue;
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(oxideav_core::Error::other(
                    "oxideplay: executor exited before reporting streams",
                ));
            }
        }
    }
}

/// Pick `(sample_rate, channels, video_dims)` from the executor's
/// declared streams. `want_video=false` collapses video_dims to None
/// so the driver builder skips the video backend entirely.
fn derive_driver_params(
    streams: &[oxideav_core::StreamInfo],
    want_video: bool,
) -> (u32, u16, Option<(u32, u32)>) {
    let mut sr = 48_000u32;
    let mut ch = 2u16;
    let mut video_dims: Option<(u32, u32)> = None;
    for s in streams {
        match s.params.media_type {
            oxideav_core::MediaType::Audio => {
                sr = s.params.sample_rate.unwrap_or(48_000);
                ch = s.params.channels.unwrap_or(2);
            }
            oxideav_core::MediaType::Video => {
                if let (Some(w), Some(h)) = (s.params.width, s.params.height) {
                    video_dims = Some((w, h));
                }
            }
            _ => {}
        }
    }
    if !want_video {
        video_dims = None;
    }
    (sr, ch, video_dims)
}

/// Best-effort total duration from a stream's `duration` field, in
/// seconds. Picks audio first, then video. `None` if neither stream
/// declares a duration (live streams, some malformed files).
fn derive_duration(streams: &[oxideav_core::StreamInfo]) -> Option<Duration> {
    streams
        .iter()
        .find_map(|s| s.duration.map(|d| (s.time_base, d)))
        .map(|(tb, d)| {
            let secs = tb.seconds_of(d);
            if secs.is_finite() && secs > 0.0 {
                Duration::from_secs_f64(secs)
            } else {
                Duration::ZERO
            }
        })
}

/// Print the status block on stderr that plain `oxideplay` always
/// printed before opening the driver.
fn print_status_block(
    streams: &[oxideav_core::StreamInfo],
    (vo_info, ao_info): (Option<String>, Option<String>),
) {
    eprintln!("oxideplay: playing {} stream(s)", streams.len());
    for s in streams {
        match s.params.media_type {
            oxideav_core::MediaType::Audio => eprintln!(
                "  audio: {} {}ch @ {} Hz",
                s.params.codec_id,
                s.params.channels.unwrap_or(0),
                s.params.sample_rate.unwrap_or(0)
            ),
            oxideav_core::MediaType::Video => eprintln!(
                "  video: {} {}x{}",
                s.params.codec_id,
                s.params.width.unwrap_or(0),
                s.params.height.unwrap_or(0)
            ),
            _ => {}
        }
    }
    match vo_info {
        Some(s) => eprintln!("  vo: {s}"),
        None => eprintln!("  vo: null (video disabled)"),
    }
    match ao_info {
        Some(s) => eprintln!("  ao: {s}"),
        None => eprintln!("  ao: null (audio disabled)"),
    }
}

/// Synthesise a playback graph: `@in` consumes the user's input,
/// `@display` consumes the decoded frames from `@in`. The executor's
/// auto-attach machinery wires audio → audio decoder, video → video
/// decoder; multi-port filters (spectrogram) attach automatically
/// when present in the user's `--inline` JSON.
fn synthesise_playback_job(input: &str) -> oxideav_core::Result<String> {
    let job = json!({
        "@in": { "all": [{"from": input}] },
        "@display": { "all": [{"from": "@in"}] },
    });
    serde_json::to_string(&job).map_err(|e| {
        oxideav_core::Error::other(format!("oxideplay: failed to synthesise playback job: {e}"))
    })
}

/// Read a job-JSON file, or stdin if `path == "-"`.
fn read_job_source(path: &str) -> oxideav_core::Result<String> {
    if path == "-" {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        std::fs::read_to_string(path).map_err(|e| {
            oxideav_core::Error::other(format!("oxideplay: cannot read job file: {e}"))
        })
    }
}

/// Probe-only mode: print streams and exit. Does NOT touch the
/// executor / driver.
fn dry_run(
    registries: &Registries,
    sources: &SourceRegistry,
    input: &str,
) -> oxideav_core::Result<()> {
    use oxideav_core::ReadSeek;
    use oxideav_source::BufferedSource;
    let raw = sources.open(input)?;
    let buffered = BufferedSource::new(raw, 1 << 20)?;
    let mut handle: Box<dyn ReadSeek> = Box::new(buffered);
    let ext = ext_from_uri(input);
    let format = registries
        .containers
        .probe_input(&mut *handle, ext.as_deref())?;
    let demuxer = registries
        .containers
        .open_demuxer(&format, handle, &registries.codecs)?;
    println!("Input: {input}");
    println!("Format: {}", demuxer.format_name());
    let streams = demuxer.streams();
    for s in streams {
        match s.params.media_type {
            oxideav_core::MediaType::Audio => println!(
                "Audio: stream #{} codec={} channels={} rate={}",
                s.index,
                s.params.codec_id,
                s.params.channels.unwrap_or(0),
                s.params.sample_rate.unwrap_or(0),
            ),
            oxideav_core::MediaType::Video => println!(
                "Video: stream #{} codec={} {}x{}",
                s.index,
                s.params.codec_id,
                s.params.width.unwrap_or(0),
                s.params.height.unwrap_or(0),
            ),
            _ => {}
        }
    }
    if streams.is_empty() {
        println!("(no streams)");
    }
    Ok(())
}

fn ext_from_uri(uri: &str) -> Option<String> {
    let last_segment = uri.rsplit('/').next().unwrap_or(uri);
    let last_segment = last_segment.split('?').next().unwrap_or(last_segment);
    let dot = last_segment.rfind('.')?;
    Some(last_segment[dot + 1..].to_ascii_lowercase())
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

    #[test]
    fn synthesised_playback_job_is_valid() {
        let s = synthesise_playback_job("/tmp/song.flac").unwrap();
        let job = oxideav::pipeline::Job::from_json(&s).expect("parse");
        assert!(job.outputs.contains_key("@display"));
        // Validation: @display must transitively reach @in.
        job.validate().expect("synthesised job validates");
    }
}
