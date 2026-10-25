//! `oxideplay` — reference media player built on `oxideav`.
//!
//! Every native backend it touches (SDL2, ALSA, PulseAudio, WASAPI,
//! CoreAudio, …) is loaded at runtime through `dlsym`, so the
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
mod viewer;

use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use clap::Parser;
use oxideav::Registries;
use oxideav::pipeline::{Executor, Job};
use oxideav_source::SourceRegistry;
use serde_json::json;

use crate::driver::OutputDriver;
use crate::drivers::audio_routing::{DownmixMode, DownmixPolicy};
use crate::drivers::engine::{AudioEngine, Composite, VideoEngine};
use crate::engine::{EngineMsg, PlayerEngine};
use crate::job_sink::ChannelSink;
use crate::media_controls::{TrackInfo, track_info_from_demuxer};

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

    /// Thread budget for the executor and the decoders it builds.
    /// `0` = auto: the job's own `threads` field, else one worker per
    /// logical CPU minus one (that core is left to the UI / audio
    /// output thread).
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

    /// Prefer hardware decoders (VideoToolbox / Vulkan Video / VAAPI
    /// …) over the pure-Rust ones. By default the pure-Rust decoder
    /// is tried first and a hardware bridge is only a fallback for
    /// codecs it does not implement; a bridge that cannot parse the
    /// stream falls back to the next decoder instead of ending the
    /// stream. Conflicts with `--no-hwaccel`.
    #[arg(long, conflicts_with = "no_hwaccel")]
    hwaccel: bool,

    /// Never use a hardware decoder, even for codecs that only a
    /// hardware bridge implements on this platform.
    #[arg(long)]
    no_hwaccel: bool,
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
            if let Some(next) = iter.peek()
                && is_help(next)
            {
                return Some(DriverHelp::Vo);
            }
        } else if arg == "--ao"
            && let Some(next) = iter.peek()
            && is_help(next)
        {
            return Some(DriverHelp::Ao);
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
    println!("  {:<10} SDL2 video (libSDL2 via dlsym)", "sdl2");
    println!(
        "  {:<10} hash every decoded frame (FNV-1a 64-bit), print digest at exit",
        "hash"
    );
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
    println!("  {:<10} SDL2 audio (libSDL2 via dlsym)", "sdl2");
    println!(
        "  {:<10} hash every decoded frame (FNV-1a 64-bit), print digest at exit",
        "hash"
    );
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
    // `hash` doesn't open a device or need surface dims — handle it
    // before the dims-required gate so it works on audio-only inputs
    // and on stripped fixtures the demuxer never reports W×H for.
    if name == "hash" {
        return Ok(Some(Box::new(
            crate::drivers::hash_engines::HashVideoEngine::new(),
        )));
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
    if name == "hash" {
        return Ok(Some(Box::new(
            crate::drivers::hash_engines::HashAudioEngine::new(),
        )));
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
    // 3D models (any extension the mesh3d registry claims) open the
    // interactive viewer instead of the media pipeline.
    if cli.job.is_none()
        && cli.inline.is_none()
        && let Some(input) = cli.input.as_deref().filter(|i| viewer::is_model_input(i))
    {
        if cli.dry_run {
            let scene = viewer::load_scene(input)?;
            println!(
                "{input}: 3D model — {} meshes, {} triangles, {} materials, {} animations",
                scene.meshes.len(),
                viewer::triangle_count(&scene),
                scene.materials.len(),
                scene.animations.len()
            );
            return Ok(());
        }
        return viewer::run(input, &cli.vo);
    }

    if cli.dry_run {
        let mut registries = Registries::new();
        oxideav_meta::register_all(&mut registries);
        let input = cli
            .input
            .as_deref()
            .ok_or_else(|| oxideav_core::Error::invalid("dry-run requires an input path"))?;
        return dry_run(&registries, &registries.sources, input);
    }

    let mut registries = Registries::new();
    oxideav_meta::register_all(&mut registries);
    let prefs = codec_preferences(&registries.codecs, cli.hwaccel, cli.no_hwaccel);

    // Load or synthesise the job JSON. Plain playback emits a trivial
    // `@in → @display` graph so it goes through the same Executor
    // path every other invocation uses.
    // Decide which track types the synthesised playback job should
    // pipe through. `--vo none` / `--ao none` / `--no-video` skips the
    // matching track at the executor level so its decoder never gets
    // created (the alternative — letting decoders run and dropping
    // frames at present time — wastes CPU and surfaces irrelevant
    // codec errors). User-supplied jobs (`--job` / `--inline`)
    // already declare their own tracks; we only filter the
    // auto-synthesised case.
    let mut want_video = !cli.no_video && !is_null_sink(&cli.vo);
    let mut want_audio = !is_null_sink(&cli.ao);

    let job_json = if let Some(j) = cli.inline.clone() {
        j
    } else if let Some(p) = cli.job.as_deref() {
        read_job_source(p)?
    } else {
        let input = cli
            .input
            .as_deref()
            .ok_or_else(|| oxideav_core::Error::invalid("no input URI (pass a path or --job)"))?;
        // Look at the container first so the job only maps the
        // audio / video tracks the registry can decode: a timecode,
        // chapter or subtitle track must never stop playback, and a
        // video track without a decoder must not take the audio
        // down with it.
        let probed = probe_streams(&registries, input);
        let plan = plan_playback(
            probed.as_deref(),
            &registries.codecs,
            want_audio,
            want_video,
        )?;
        for note in &plan.notes {
            eprintln!("oxideplay: {note}");
        }
        want_audio = plan.audio.is_some() || (probed.is_none() && want_audio);
        want_video = plan.video.is_some() || (probed.is_none() && want_video);
        synthesise_playback_job(input, &plan)?
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
        .with_codec_preferences(prefs)
        .with_sink_override(target, sink)
        .with_threads(playback_threads(cli.threads, job.threads))
        .spawn()?;

    // Wait for the first message: must be Started(streams). The
    // executor runs on a worker thread so this only blocks until the
    // demuxer has resolved its streams.
    let streams = wait_for_started(&rx)?;

    // Build the driver from what the streams + CLI flags actually
    // permit. This MUST happen on the main thread so winit's NSWindow
    // is owned correctly. `want_audio` / `want_video` were computed
    // above to drive the synthesised job; reuse them here.
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
    use oxideav_core::{ReadSeek, SourceOutput};
    use oxideav_source::BufferedSource;

    let fallback_title = filename_stem_from_uri(input);

    // The track-info probe needs a bytes-shape input to drive the
    // demuxer. Packet-shape (e.g. `rtmp://`) and frame-shape (e.g.
    // `generate://`) sources skip the demux layer entirely and have
    // to go through the executor's typed pipeline; for the OS media
    // widget we fall back to the filename here rather than try to
    // fake a `ReadSeek` over them.
    let raw = match registries.sources.open(input) {
        Ok(SourceOutput::Bytes(b)) => b,
        Ok(_) | Err(_) => {
            return TrackInfo {
                title: fallback_title,
                duration: total_duration,
                ..Default::default()
            };
        }
    };
    // BytesSource: Read + Seek + Send; ReadSeek: Read + Seek. Re-box
    // to drop the Send bound the demuxer trait doesn't ask for.
    let raw: Box<dyn ReadSeek> = Box::new(raw);
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

/// Decoder-selection policy for playback. Pure-Rust decoders go
/// first: every software implementation gets a priority boost that
/// out-ranks any hardware bridge, so a bridge is only reached for a
/// codec the library does not implement (and, through the pipeline's
/// init-time fallback, when the software factory refuses the
/// parameters). `--hwaccel` restores the registered priorities
/// (hardware first); `--no-hwaccel` drops hardware impls entirely.
/// The executor budget for playback: an explicit `--threads N`, else
/// the job's `threads` key (`0` hands that choice to the executor),
/// else every logical CPU but one — decoders get the rest of the
/// machine, the presentation thread keeps a core for itself.
fn playback_threads(cli_threads: usize, job_threads: Option<usize>) -> usize {
    if cli_threads > 0 {
        return cli_threads;
    }
    if job_threads.is_some_and(|n| n > 0) {
        return 0;
    }
    oxideav_core::ExecutionContext::auto()
        .threads
        .saturating_sub(1)
        .max(1)
}

pub(crate) fn codec_preferences(
    codecs: &oxideav_core::CodecRegistry,
    hwaccel: bool,
    no_hwaccel: bool,
) -> oxideav::pipeline::CodecPreferences {
    let mut prefs = oxideav::pipeline::CodecPreferences {
        no_hardware: no_hwaccel,
        ..Default::default()
    };
    if !hwaccel && !no_hwaccel {
        let mut sw: Vec<String> = codecs
            .all_implementations()
            .filter(|(_, i)| !i.caps.hardware_accelerated && i.make_decoder.is_some())
            .map(|(_, i)| i.caps.implementation.clone())
            .collect();
        sw.sort();
        sw.dedup();
        prefs.prefer = sw;
        // Registered priorities are ~10 (hardware) vs ~100 (software);
        // the boost only has to exceed that span.
        prefs.boost = 1_000_000;
    }
    prefs
}

/// Open the input through the demuxer registry and return its stream
/// list, or `None` when the source is not a plain byte stream (rtmp,
/// generate://, …) or the container is not recognised — the executor
/// reports those on its own path with a better message.
fn probe_streams(registries: &Registries, input: &str) -> Option<Vec<oxideav_core::StreamInfo>> {
    use oxideav_core::{ReadSeek, SourceOutput};
    use oxideav_source::BufferedSource;
    let raw = match registries.sources.open(input).ok()? {
        SourceOutput::Bytes(b) => b,
        _ => return None,
    };
    let raw: Box<dyn ReadSeek> = Box::new(raw);
    let buffered = BufferedSource::new(raw, 1 << 20).ok()?;
    let mut handle: Box<dyn ReadSeek> = Box::new(buffered);
    let ext = ext_from_uri(input);
    let format = registries
        .containers
        .probe_input(&mut *handle, ext.as_deref())
        .ok()?;
    let demuxer = registries
        .containers
        .open_demuxer(&format, handle, &registries.codecs)
        .ok()?;
    Some(demuxer.streams().to_vec())
}

/// Which tracks the synthesised playback job maps. `audio` / `video`
/// hold the 0-based ordinal (within that media type) of the stream
/// to play, `None` when the kind is unwanted, absent or undecodable.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PlaybackPlan {
    audio: Option<u32>,
    video: Option<u32>,
    /// Why a track was skipped — printed before playback starts.
    notes: Vec<String>,
    /// No stream list was available: map every stream (`all`) and
    /// let the executor sort it out.
    blind: bool,
}

/// Pick the audio and video streams to play. Data, subtitle and
/// unknown tracks are never mapped — the executor would otherwise
/// look for a decoder for them and fail the whole job (`codec not
/// found: text` on an MP4 timecode track). Within a kind the first
/// stream with a registered decoder wins; when none has one the kind
/// is dropped with a note instead of failing the other track.
pub(crate) fn plan_playback(
    streams: Option<&[oxideav_core::StreamInfo]>,
    codecs: &oxideav_core::CodecRegistry,
    want_audio: bool,
    want_video: bool,
) -> oxideav_core::Result<PlaybackPlan> {
    use oxideav_core::MediaType;
    let Some(streams) = streams else {
        return Ok(PlaybackPlan {
            audio: want_audio.then_some(0),
            video: want_video.then_some(0),
            notes: Vec::new(),
            blind: true,
        });
    };
    let mut plan = PlaybackPlan::default();
    let mut missing: Vec<String> = Vec::new();
    for (kind, wanted, slot) in [
        (MediaType::Audio, want_audio, &mut plan.audio),
        (MediaType::Video, want_video, &mut plan.video),
    ] {
        if !wanted {
            continue;
        }
        let pool: Vec<&oxideav_core::StreamInfo> = streams
            .iter()
            .filter(|s| s.params.media_type == kind)
            .collect();
        if pool.is_empty() {
            continue;
        }
        match pool
            .iter()
            .position(|s| codecs.has_decoder(&s.params.codec_id))
        {
            Some(ordinal) => {
                *slot = Some(ordinal as u32);
                if ordinal > 0 {
                    plan.notes.push(format!(
                        "no decoder for {kind:?} codec `{}` — playing {kind:?} stream #{} instead",
                        pool[0].params.codec_id, pool[ordinal].index
                    ));
                }
            }
            None => {
                let codec = pool[0].params.codec_id.to_string();
                plan.notes.push(format!(
                    "no decoder for {kind:?} codec `{codec}` — skipping {kind:?}"
                ));
                missing.push(codec);
            }
        }
    }
    if plan.audio.is_none() && plan.video.is_none() {
        let kinds: Vec<String> = streams
            .iter()
            .map(|s| format!("{:?}:{}", s.params.media_type, s.params.codec_id))
            .collect();
        return Err(if missing.is_empty() {
            oxideav_core::Error::invalid(format!(
                "no playable audio or video stream (streams: {})",
                kinds.join(", ")
            ))
        } else {
            oxideav_core::Error::CodecNotFound(missing.join(", "))
        });
    }
    Ok(plan)
}

/// Synthesise a playback graph: `@in` consumes the user's input,
/// `@display` consumes the decoded frames from `@in`. Only the tracks
/// the [`PlaybackPlan`] selected are mapped, each pinned to its
/// stream ordinal, so the executor never instantiates a decoder for
/// a data / subtitle track or for a codec nobody registered. The
/// executor's auto-attach machinery wires audio → audio decoder,
/// video → video decoder; multi-port filters (spectrogram) attach
/// automatically when present in the user's `--inline` JSON.
pub(crate) fn synthesise_playback_job(
    input: &str,
    plan: &PlaybackPlan,
) -> oxideav_core::Result<String> {
    let mut display = serde_json::Map::new();
    if plan.blind {
        // No stream list (packet-shape source): keep the historical
        // `all` mapping, trimmed to the wanted kinds when only one
        // is wanted.
        match (plan.audio.is_some(), plan.video.is_some()) {
            (true, true) | (false, false) => {
                display.insert("all".into(), json!([{"from": "@in"}]));
            }
            (true, false) => {
                display.insert("audio".into(), json!([{"from": "@in"}]));
            }
            (false, true) => {
                display.insert("video".into(), json!([{"from": "@in"}]));
            }
        }
    } else {
        for (key, slot) in [("audio", plan.audio), ("video", plan.video)] {
            if let Some(ordinal) = slot {
                display.insert(
                    key.into(),
                    json!([{"from": "@in", "stream_selector": {"type": key, "index": ordinal}}]),
                );
            }
        }
    }
    let job = json!({
        "@in": { "all": [{"from": input}] },
        "@display": display,
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
    use oxideav_core::{ReadSeek, SourceOutput};
    use oxideav_source::BufferedSource;
    // dry-run runs on a bytes-shape demuxer pipeline. Packet-shape
    // (e.g. `rtmp://`) and frame-shape (e.g. `generate://`) sources
    // skip the demux layer entirely and need the executor's typed
    // pipeline (a JSON job) — surface a clear error rather than try
    // to fake a `ReadSeek` over them.
    let raw = match sources.open(input)? {
        SourceOutput::Bytes(b) => b,
        SourceOutput::Packets(_) => {
            return Err(oxideav_core::Error::unsupported(format!(
                "{input}: packet-shape source (e.g. rtmp://) — wire it through a JSON job"
            )));
        }
        SourceOutput::Frames(_) => {
            return Err(oxideav_core::Error::unsupported(format!(
                "{input}: frame-shape source (e.g. generate:// video) — wire it through a JSON job"
            )));
        }
        SourceOutput::MultiTitle(_) => {
            return Err(oxideav_core::Error::unsupported(format!(
                "{input}: multi-title source (e.g. bluray:// without ?title=) — pin a title via the URI and re-open"
            )));
        }
        _ => {
            return Err(oxideav_core::Error::unsupported(format!(
                "{input}: source shape not yet supported by the bytes-driven path — wire it through a JSON job"
            )));
        }
    };
    // BytesSource: Read + Seek + Send; ReadSeek: Read + Seek. Re-box
    // to drop the Send bound the demuxer trait doesn't ask for.
    let raw: Box<dyn ReadSeek> = Box::new(raw);
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
    fn playback_budget_leaves_a_core_for_presentation() {
        let auto = oxideav_core::ExecutionContext::auto().threads;
        assert_eq!(playback_threads(0, None), auto.saturating_sub(1).max(1));
        // The job's own key wins over the default; the flag over both.
        assert_eq!(playback_threads(0, Some(3)), 0);
        assert_eq!(playback_threads(5, Some(3)), 5);
        assert_eq!(playback_threads(1, None), 1);
        let cli = Cli::try_parse_from(["oxideplay", "--threads", "6", "x.mp4"]).unwrap();
        assert_eq!(cli.threads, 6);
    }

    #[test]
    fn cli_parses_dry_run() {
        let cli = Cli::try_parse_from(["oxideplay", "--dry-run", "x.mp4"]).unwrap();
        assert!(cli.dry_run);
        assert_eq!(cli.input.as_deref(), Some("x.mp4"));
    }

    fn plan(audio: Option<u32>, video: Option<u32>) -> PlaybackPlan {
        PlaybackPlan {
            audio,
            video,
            notes: Vec::new(),
            blind: false,
        }
    }

    #[test]
    fn synthesised_playback_job_is_valid() {
        let s = synthesise_playback_job("/tmp/song.flac", &plan(Some(0), Some(0))).unwrap();
        let job = oxideav::pipeline::Job::from_json(&s).expect("parse");
        assert!(job.outputs.contains_key("@display"));
        // Validation: @display must transitively reach @in.
        job.validate().expect("synthesised job validates");
        // Never `all`: a data / subtitle track must not be mapped.
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v["@display"].get("all").is_none());
        assert_eq!(v["@display"]["audio"][0]["stream_selector"]["index"], 0);
        assert_eq!(
            v["@display"]["video"][0]["stream_selector"]["type"],
            "video"
        );
    }

    #[test]
    fn synthesised_job_audio_only_omits_video_track() {
        // --vo null → want_video=false → only the audio track key is
        // emitted under @display, so the executor never spawns the
        // video decoder. The "all" inside @in is an input declaration
        // (let demux expose every stream); we only check @display.
        let s = synthesise_playback_job("/tmp/song.mp4", &plan(Some(0), None)).unwrap();
        let job: serde_json::Value = serde_json::from_str(&s).unwrap();
        let display = &job["@display"];
        assert!(display.get("audio").is_some());
        assert!(display.get("video").is_none());
        assert!(display.get("all").is_none());
        let job = oxideav::pipeline::Job::from_json(&s).expect("parse");
        job.validate()
            .expect("audio-only synthesised job validates");
    }

    #[test]
    fn synthesised_job_video_only_omits_audio_track() {
        let s = synthesise_playback_job("/tmp/clip.mp4", &plan(None, Some(1))).unwrap();
        let job: serde_json::Value = serde_json::from_str(&s).unwrap();
        let display = &job["@display"];
        assert!(display.get("video").is_some());
        assert_eq!(display["video"][0]["stream_selector"]["index"], 1);
        assert!(display.get("audio").is_none());
        let job = oxideav::pipeline::Job::from_json(&s).expect("parse");
        job.validate()
            .expect("video-only synthesised job validates");
    }

    #[test]
    fn blind_plan_keeps_the_all_mapping() {
        let p = PlaybackPlan {
            audio: Some(0),
            video: Some(0),
            notes: Vec::new(),
            blind: true,
        };
        let s = synthesise_playback_job("rtmp://host/app/key", &p).unwrap();
        let job: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(job["@display"].get("all").is_some());
    }

    fn stream(index: u32, kind: oxideav_core::MediaType, codec: &str) -> oxideav_core::StreamInfo {
        let params = match kind {
            oxideav_core::MediaType::Audio => {
                oxideav_core::CodecParameters::audio(oxideav_core::CodecId::new(codec))
            }
            oxideav_core::MediaType::Video => {
                oxideav_core::CodecParameters::video(oxideav_core::CodecId::new(codec))
            }
            _ => {
                let mut p = oxideav_core::CodecParameters::video(oxideav_core::CodecId::new(codec));
                p.media_type = kind;
                p
            }
        };
        oxideav_core::StreamInfo {
            index,
            time_base: oxideav_core::TimeBase::MILLIS,
            duration: None,
            start_time: None,
            params,
        }
    }

    fn registry_with(decoders: &[&str]) -> oxideav_core::CodecRegistry {
        let mut reg = oxideav_core::CodecRegistry::new();
        for &d in decoders {
            fn no_decoder(
                _: &oxideav_core::CodecParameters,
            ) -> oxideav_core::Result<Box<dyn oxideav_core::Decoder>> {
                Err(oxideav_core::Error::unsupported("test stub"))
            }
            reg.register(
                oxideav_core::CodecInfo::new(oxideav_core::CodecId::new(d))
                    .capabilities(oxideav_core::CodecCapabilities::video(d))
                    .decoder(no_decoder),
            );
        }
        reg
    }

    #[test]
    fn plan_skips_data_tracks_and_undecodable_kinds() {
        use oxideav_core::MediaType;
        let reg = registry_with(&["h264", "aac"]);
        // MP4 with a timecode track: data is never mapped.
        let streams = [
            stream(0, MediaType::Video, "h264"),
            stream(1, MediaType::Audio, "aac"),
            stream(2, MediaType::Data, "unknown"),
        ];
        let p = plan_playback(Some(&streams), &reg, true, true).unwrap();
        assert_eq!((p.audio, p.video), (Some(0), Some(0)));
        assert!(p.notes.is_empty());

        // Raw-RGB AVI whose video codec nobody decodes: audio still plays.
        let streams = [
            stream(0, MediaType::Video, "rgb24"),
            stream(1, MediaType::Audio, "aac"),
        ];
        let p = plan_playback(Some(&streams), &reg, true, true).unwrap();
        assert_eq!((p.audio, p.video), (Some(0), None));
        assert_eq!(p.notes.len(), 1);

        // Second video stream is the decodable one.
        let streams = [
            stream(0, MediaType::Video, "rgb24"),
            stream(1, MediaType::Video, "h264"),
        ];
        let p = plan_playback(Some(&streams), &reg, true, true).unwrap();
        assert_eq!((p.audio, p.video), (None, Some(1)));

        // Nothing decodable at all → codec-not-found naming the codec.
        let streams = [stream(0, MediaType::Video, "rgb24")];
        let err = plan_playback(Some(&streams), &reg, true, true).unwrap_err();
        assert!(
            matches!(err, oxideav_core::Error::CodecNotFound(ref c) if c == "rgb24"),
            "{err}"
        );

        // Unwanted kinds are left out even when decodable.
        let streams = [
            stream(0, MediaType::Video, "h264"),
            stream(1, MediaType::Audio, "aac"),
        ];
        let p = plan_playback(Some(&streams), &reg, true, false).unwrap();
        assert_eq!((p.audio, p.video), (Some(0), None));
    }

    #[test]
    fn software_decoders_rank_ahead_of_hardware_by_default() {
        let reg = registry_with(&["h264"]);
        let prefs = codec_preferences(&reg, false, false);
        assert!(prefs.prefer.iter().any(|n| n == "h264"));
        assert!(prefs.boost > 100);
        assert!(!prefs.no_hardware);
        let hw = codec_preferences(&reg, true, false);
        assert!(hw.prefer.is_empty());
        let none = codec_preferences(&reg, false, true);
        assert!(none.no_hardware);
    }
}
