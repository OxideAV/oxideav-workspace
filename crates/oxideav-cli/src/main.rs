//! `oxideav` command-line frontend.

use clap::{Parser, Subcommand};
use oxideav::core::Error;
use oxideav::core::{ReadSeek, SourceOutput};
use oxideav::Registries;
use oxideav_source::{BufferedSource, SourceRegistry};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;

#[derive(Parser)]
#[command(
    name = "oxideav",
    version,
    about = "Pure-Rust media transcoding framework",
    disable_help_subcommand = true
)]
struct Cli {
    /// Prefetch buffer size in MiB for input sources. Default 0 (no
    /// buffering) — use a positive value for HTTP inputs to absorb jitter.
    #[arg(long, default_value_t = 0, global = true)]
    buffer_mib: u32,

    /// Enable debug log output from every oxideav crate that emits
    /// through the `log` facade. Useful for diagnosing startup hangs,
    /// codec dispatch, parser state, etc. Writes to stderr by default;
    /// pair with `--debug-output FILE` to redirect to a file instead.
    #[arg(long, global = true)]
    debug: bool,

    /// Write debug log output to FILE instead of stderr. Implies
    /// `--debug` if not already set. Stderr stays clean.
    #[arg(long, global = true, value_name = "FILE")]
    debug_output: Option<PathBuf>,

    /// Disable hardware-accelerated codec backends (videotoolbox /
    /// audiotoolbox on macOS). Forces the pure-Rust implementation
    /// for every codec the framework knows about. Useful when you
    /// need byte-deterministic output, are bisecting a regression
    /// against the pure-Rust path, or the hardware encoder produces
    /// a worse stream for the target bitrate.
    #[arg(long, global = true)]
    no_hwaccel: bool,

    #[command(subcommand)]
    command: Command,
}

/// `log::Log` impl that writes every record to a single shared writer
/// (stderr or a file). Sync via Mutex — the volume is low enough
/// (debug-only, opt-in) that lock contention is irrelevant.
struct DebugLogger {
    sink: Mutex<Box<dyn Write + Send>>,
}

impl log::Log for DebugLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut sink = self.sink.lock().unwrap();
        let _ = writeln!(
            sink,
            "[{} {}] {}",
            record.level(),
            record.target(),
            record.args()
        );
        let _ = sink.flush();
    }

    fn flush(&self) {
        let _ = self.sink.lock().unwrap().flush();
    }
}

fn install_debug_logger(output: Option<&Path>) -> Result<(), String> {
    let sink: Box<dyn Write + Send> = match output {
        Some(path) => {
            Box::new(File::create(path).map_err(|e| format!("--debug-output {path:?}: {e}"))?)
        }
        None => Box::new(std::io::stderr()),
    };
    let logger = Box::new(DebugLogger {
        sink: Mutex::new(sink),
    });
    log::set_boxed_logger(logger).map_err(|e| format!("logger init: {e}"))?;
    log::set_max_level(log::LevelFilter::Debug);
    Ok(())
}

#[derive(Subcommand)]
enum Command {
    /// List compiled-in codecs and containers.
    List,
    /// Probe a media URI and print stream information.
    Probe {
        /// Input URI: local path, file:// URL, or http(s):// URL.
        input: String,
    },
    /// Remux an input to a new container (no re-encoding).
    ///
    /// Only stream copy is supported for now; both sides must use the same codec.
    Remux {
        /// Input URI: local path, file:// URL, or http(s):// URL.
        input: String,
        output: PathBuf,
        /// Override the output container format. Defaults to file extension.
        #[arg(long)]
        format: Option<String>,
    },
    /// Decode an input and re-encode to a new codec.
    ///
    /// Multi-stream inputs (e.g. an MP4 with video + audio) are handled
    /// per-stream:
    ///
    /// * `--codec` applies to **every** stream (legacy behaviour, kept
    ///   for the common single-stream case).
    /// * `--codec-audio` / `--codec-video` / `--codec-subtitle` override
    ///   the codec for a specific media type, taking precedence over
    ///   `--codec` for that type.
    /// * For any stream where no codec is specified, audio defaults to
    ///   a PCM variant matching the decoded sample format (e.g. FLAC
    ///   16-bit → pcm_s16le); video, subtitle, and data streams fall
    ///   back to stream-copy (no re-encode) so the output container
    ///   carries them unmodified.
    /// * Streams whose media type is `Data` or `Unknown` and that have
    ///   no encoder available are stream-copied; specifying a codec for
    ///   such a stream is a hard error.
    Transcode {
        /// Input URI: local path, file:// URL, or http(s):// URL.
        input: String,
        output: PathBuf,
        /// Override the output codec id for **all** streams (e.g.
        /// "pcm_s16le", "pcm_f32le"). Per-media-type flags below take
        /// precedence when set.
        #[arg(long)]
        codec: Option<String>,
        /// Override the output codec id for audio streams. Wins over
        /// `--codec` for audio.
        #[arg(long = "codec-audio", visible_alias = "c:a")]
        codec_audio: Option<String>,
        /// Override the output codec id for video streams. Wins over
        /// `--codec` for video.
        #[arg(long = "codec-video", visible_alias = "c:v")]
        codec_video: Option<String>,
        /// Override the output codec id for subtitle streams. Wins over
        /// `--codec` for subtitles.
        #[arg(long = "codec-subtitle", visible_alias = "c:s")]
        codec_subtitle: Option<String>,
        /// Override the output container format. Defaults to file extension.
        #[arg(long)]
        format: Option<String>,
    },
    /// Run a JSON-described transcode job.
    ///
    /// The job description is a JSON object keyed by output filename (or
    /// `@alias` for intermediate reuse). See the oxideav-job crate docs
    /// for the schema. Supply the JSON inline with `--inline`, or pass a
    /// file path as the positional argument. Use `-` to read from stdin.
    Run {
        /// Path to a job JSON file. Use `-` for stdin. Ignored if
        /// `--inline` is given.
        #[arg(required_unless_present = "inline")]
        file: Option<String>,
        /// Inline JSON job description.
        #[arg(long)]
        inline: Option<String>,
        /// Thread budget for the executor. `0` = auto (use the JSON's
        /// `threads` field, falling back to the number of logical CPUs).
        /// `1` forces serial execution; `≥ 2` runs pipelined.
        #[arg(long, default_value_t = 0)]
        threads: usize,
    },
    /// Validate a JSON job description without running it.
    Validate {
        #[arg(required_unless_present = "inline")]
        file: Option<String>,
        #[arg(long)]
        inline: Option<String>,
    },
    /// Resolve a JSON job to its DAG and print a human-readable summary,
    /// without opening any inputs or outputs.
    DryRun {
        #[arg(required_unless_present = "inline")]
        file: Option<String>,
        #[arg(long)]
        inline: Option<String>,
    },
    /// ImageMagick-style convert — chain filters over an input file.
    ///
    /// Syntax mirrors `imagemagick convert`: the first positional arg
    /// is the input, the last is the output, and `-op VALUE` pairs
    /// between them form a filter chain applied in source order.
    ///
    /// Works on images, video, and audio — a PNG → JPG is just a
    /// one-frame pipeline; a MP4 → MKV + resize reuses the same
    /// code path.
    ///
    /// Supported ops: -resize, -blur, -edge, -colors, -dither,
    /// -format, -quality, -strip. IM ops that we don't yet have a
    /// primitive for (-rotate, -crop, -flip, …) exit with a clear
    /// "not yet implemented" error.
    Convert {
        /// Forwarded verbatim to `oxideav-cli-convert`; the custom
        /// parser there handles the IM argument ordering rules.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Benchmark every backend of one or more codecs.
    ///
    /// For each registered impl of the given codec id (or every video +
    /// audio codec when `--all` is set), times encode + decode through
    /// the backend and reports frames-per-second. Useful for comparing
    /// hardware (videotoolbox / audiotoolbox / VAAPI / NVENC) vs the
    /// pure-Rust path, and for spotting SW codecs whose throughput is
    /// too low to be useful.
    ///
    /// Methodology: synthesises 500 unique source frames once
    /// (gradient for video, sine for audio), then loops them through
    /// the codec for ~3 s wall-clock. fps = total iterations /
    /// elapsed. The decode-side stream is produced by self-encoding
    /// the same prep set with the first available encoder, so every
    /// decoder backend benches against an identical bitstream.
    /// Inspect every backend of a codec — capabilities, limits,
    /// supported parameters — without running a benchmark.
    ///
    /// Lists, per backend: hardware-or-software flag, priority, supported
    /// sides (decode / encode), engine label (CPU brand for SW, media-engine
    /// name for HW), capability flags (intra_only / lossy / lossless),
    /// declared limits (max_width / max_height / max_bitrate / accepted
    /// pixel formats), and the full options schema for each side
    /// (parameter name + type + default + help). Once
    /// `oxideav-nvidia` / `-vaapi` / `-vdpau` / `-vulkan-video`
    /// register HW backends, they appear here automatically.
    Info {
        /// Codec id (e.g. `h264`, `aac`, `vp8`).
        codec: String,
    },
    Bench {
        /// Codec id (e.g. `h264`, `aac`, `vp8`). Required unless `--all` is set.
        codec: Option<String>,
        /// Bench every video + audio codec the runtime registers.
        #[arg(long)]
        all: bool,
        /// Video frame width (default 1920).
        #[arg(long, default_value_t = 1920)]
        width: u32,
        /// Video frame height (default 1080).
        #[arg(long, default_value_t = 1080)]
        height: u32,
        /// Wall-clock seconds per backend × side (default 3.0).
        #[arg(long, default_value_t = 3.0)]
        duration: f64,
        /// Number of unique source frames synthesised in the prep step
        /// (default 500). The bench loop cycles through this set.
        #[arg(long, default_value_t = 500)]
        prep_frames: u32,
        /// Limit which side(s) to bench. `decode`, `encode`, or `both`.
        #[arg(long, default_value = "both")]
        side: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // `--debug-output FILE` implies `--debug`; either flag opts in to
    // the log facade. Without one of them, `log::debug!` calls compile
    // to a `log_enabled!` check that always returns false (no logger
    // installed → max level stays at Off), so registered code paths
    // that emit debug logs cost nothing.
    if cli.debug || cli.debug_output.is_some() {
        if let Err(e) = install_debug_logger(cli.debug_output.as_deref()) {
            eprintln!("oxideav: {e}");
            return ExitCode::FAILURE;
        }
    }
    // Build the runtime context with EVERY sibling crate enabled at
    // build time via `oxideav-meta`. The runtime context is the source
    // of truth for what's available — `--no-hwaccel` does NOT remove
    // hardware codecs from the registry; it sets `CodecPreferences {
    // no_hardware: true, .. }` which the pipeline forwards to
    // `make_decoder_with` / `make_encoder_with` so HW impls are
    // skipped at dispatch time only. This keeps `oxideav list` showing
    // every backend regardless of the flag.
    let mut registries = Registries::new();
    oxideav_meta::register_all(&mut registries);
    // Codec resolution preferences applied to every pipeline /
    // transcode invocation in this run. Today only `--no-hwaccel`
    // feeds in; future flags (--prefer impl=h264_sw, --exclude impl=...)
    // would extend this struct.
    let codec_prefs = oxideav::pipeline::CodecPreferences {
        no_hardware: cli.no_hwaccel,
        ..Default::default()
    };
    // RTMP source driver lives outside the `oxideav` aggregator's
    // feature wall (the protocol crate is std-only and we keep its
    // `register()` call site here so the dependency tree of the
    // aggregator doesn't need to grow another optional crate). Mirrors
    // the `http` feature gating: default-on, opt-out via
    // `--no-default-features`.
    #[cfg(feature = "rtmp")]
    {
        oxideav_rtmp::register(&mut registries.sources);
    }
    // Backward-compat: keep a `sources` reference for sub-commands that
    // still take it explicitly. The unified `RuntimeContext` already
    // carries the same registry, so `&registries.sources` and `&sources`
    // resolve to the same value.
    let sources = &registries.sources;
    let buffer_bytes = (cli.buffer_mib as usize).saturating_mul(1 << 20);

    let result = match cli.command {
        Command::List => cmd_list(&registries),
        Command::Probe { input } => cmd_probe(&registries, sources, &input, buffer_bytes),
        Command::Remux {
            input,
            output,
            format,
        } => cmd_remux(
            &registries,
            sources,
            &input,
            &output,
            format.as_deref(),
            buffer_bytes,
        ),
        Command::Transcode {
            input,
            output,
            codec,
            codec_audio,
            codec_video,
            codec_subtitle,
            format,
        } => cmd_transcode(
            &registries,
            sources,
            &input,
            &output,
            TranscodeCodecOverrides {
                all: codec.as_deref(),
                audio: codec_audio.as_deref(),
                video: codec_video.as_deref(),
                subtitle: codec_subtitle.as_deref(),
            },
            format.as_deref(),
            buffer_bytes,
            &codec_prefs,
        ),
        Command::Run {
            file,
            inline,
            threads,
        } => cmd_run(&registries, sources, file, inline, threads),
        Command::Validate { file, inline } => cmd_validate(file, inline),
        Command::DryRun { file, inline } => cmd_dry_run(file, inline),
        #[cfg(feature = "convert")]
        Command::Convert { args } => oxideav_cli_convert::run(&args, &registries),
        #[cfg(not(feature = "convert"))]
        Command::Convert { args: _ } => Err(Error::unsupported(
            "convert: oxideav was built without the `convert` feature",
        )),
        Command::Info { codec } => cmd_info(&registries, &codec),
        Command::Bench {
            codec,
            all,
            width,
            height,
            duration,
            prep_frames,
            side,
        } => cmd_bench(
            &registries,
            codec,
            all,
            width,
            height,
            duration,
            prep_frames,
            &side,
        ),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxideav: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_list(reg: &Registries) -> oxideav::core::Result<()> {
    // Unified container view — one row per container name with the
    // 'D'/'M' caps flags showing which sides (demux / mux) the runtime
    // actually has. Matches the codec table's style.
    use std::collections::BTreeMap;
    let mut containers: BTreeMap<&str, (bool, bool)> = BTreeMap::new();
    for name in reg.containers.demuxer_names() {
        containers.entry(name).or_default().0 = true;
    }
    for name in reg.containers.muxer_names() {
        containers.entry(name).or_default().1 = true;
    }
    let name_w = containers.keys().map(|k| k.len()).max().unwrap_or(4).max(4);
    println!("Containers:");
    println!("  {:<nw$}   Caps", "Name", nw = name_w,);
    println!("  {:─<nw$}   ────", "", nw = name_w,);
    for (name, (demux, mux)) in &containers {
        let mut caps = String::with_capacity(2);
        caps.push(if *demux { 'D' } else { '.' });
        caps.push(if *mux { 'M' } else { '.' });
        println!("  {:<nw$}   {}", name, caps, nw = name_w);
    }

    // Group every (codec_id × backend) by media-type, then by codec id.
    // Within a codec id, sort backends by priority ascending so HW
    // implementations (priority ~10) sit above the SW fallback (~100).
    use oxideav::core::MediaType;
    let mut by_type: BTreeMap<
        &'static str,
        BTreeMap<&str, Vec<&oxideav::core::CodecImplementation>>,
    > = BTreeMap::new();
    for (id, im) in reg.codecs.all_implementations() {
        let bucket = match im.caps.media_type {
            MediaType::Video => "Video",
            MediaType::Audio => "Audio",
            MediaType::Subtitle => "Subtitle",
            MediaType::Data => "Data",
            MediaType::Unknown => "Other",
        };
        by_type
            .entry(bucket)
            .or_default()
            .entry(id.as_str())
            .or_default()
            .push(im);
    }
    for codecs in by_type.values_mut() {
        for impls in codecs.values_mut() {
            impls.sort_by_key(|i| i.caps.priority);
        }
    }

    // Stable section order — matches ffmpeg's `-codecs` grouping.
    for &kind in &["Video", "Audio", "Subtitle", "Data", "Other"] {
        let Some(codecs) = by_type.get(kind) else {
            continue;
        };
        if codecs.is_empty() {
            continue;
        }
        let codec_w = codecs.keys().map(|k| k.len()).max().unwrap_or(5).max(5);
        let backend_w = codecs
            .values()
            .flat_map(|v| v.iter().map(|i| i.caps.implementation.len()))
            .max()
            .unwrap_or(7)
            .max(7);
        println!();
        println!("{kind} codecs:");
        println!(
            "  {:<cw$}   {:<bw$}   Caps   HW   Prio",
            "Codec",
            "Backend",
            cw = codec_w,
            bw = backend_w,
        );
        println!(
            "  {:─<cw$}   {:─<bw$}   ────   ──   ────",
            "",
            "",
            cw = codec_w,
            bw = backend_w,
        );
        for (id, impls) in codecs {
            for (i, im) in impls.iter().enumerate() {
                let codec_cell = if i == 0 { *id } else { "" };
                let mut caps = String::with_capacity(2);
                caps.push(if im.caps.decode { 'D' } else { '.' });
                caps.push(if im.caps.encode { 'E' } else { '.' });
                let hw = if im.caps.hardware_accelerated {
                    "✓"
                } else {
                    "."
                };
                println!(
                    "  {:<cw$}   {:<bw$}   {:<4}   {:<2}   {}",
                    codec_cell,
                    im.caps.implementation,
                    caps,
                    hw,
                    im.caps.priority,
                    cw = codec_w,
                    bw = backend_w,
                );
            }
        }
    }
    Ok(())
}

fn cmd_probe(
    reg: &Registries,
    sources: &SourceRegistry,
    input: &str,
    buffer_bytes: usize,
) -> oxideav::core::Result<()> {
    let (format, file) = detect_input_format(reg, sources, input, buffer_bytes)?;
    // For local files we report bytes from filesystem metadata; for URI
    // sources we leave the size undetermined here (could surface from
    // Source::len() in a follow-up).
    let file_size = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    let demuxer = reg.containers.open_demuxer(&format, file, &reg.codecs)?;
    println!("Input: {input}");
    println!("Format: {}", demuxer.format_name());

    // Metadata block — ffprobe-style key/value listing. Dedupe identical
    // (key, value) pairs to absorb cases like ffmpeg's MKV writer that
    // emits the same encoder string in both Info\WritingApp and a
    // Tags\SimpleTag\ENCODER.
    let raw_md = demuxer.metadata();
    let mut md: Vec<(&String, &String)> = Vec::with_capacity(raw_md.len());
    for (k, v) in raw_md {
        if !md.iter().any(|(ek, ev)| *ek == k && *ev == v) {
            md.push((k, v));
        }
    }
    if !md.is_empty() {
        println!("Metadata:");
        let key_width = md.iter().map(|(k, _)| k.len()).max().unwrap_or(0).min(20);
        let mut prev_key: Option<&str> = None;
        for (k, v) in md {
            // For repeated keys (e.g. sample_name:*), follow ffprobe's
            // convention of showing the key once and continuation-aligned
            // subsequent values.
            let show_key = prev_key.map(|pk| k != pk).unwrap_or(true);
            let key_cell = if show_key { k.as_str() } else { "" };
            println!("    {:<kw$} : {}", key_cell, v, kw = key_width);
            prev_key = Some(k);
        }
    }

    let pictures = demuxer.attached_pictures();
    if !pictures.is_empty() {
        println!("Attached pictures:");
        for (i, pic) in pictures.iter().enumerate() {
            let mime = if pic.mime_type.is_empty() {
                "unknown"
            } else {
                pic.mime_type.as_str()
            };
            let size = human_bytes(pic.data.len());
            let desc = if pic.description.is_empty() {
                String::new()
            } else {
                format!("\"{}\"", pic.description)
            };
            println!(
                "  #{}  {:<10}  {:<18}  {:<10}  {}",
                i + 1,
                mime,
                format!("{:?}", pic.picture_type),
                desc,
                size
            );
        }
    }

    // Container-level duration + bitrate.
    let duration_us = demuxer.duration_micros().or_else(|| {
        // Fall back to longest per-stream duration.
        demuxer
            .streams()
            .iter()
            .filter_map(|s| s.duration.map(|d| (s.time_base.seconds_of(d) * 1e6) as i64))
            .max()
    });
    if let Some(us) = duration_us {
        let mut parts = format!("Duration: {}", format_duration_hhmmss(us));
        if us > 0 && file_size > 0 {
            let bitrate_bps = (file_size as u128 * 8 * 1_000_000) / (us as u128);
            parts += &format!(", bitrate: {} kb/s", bitrate_bps / 1000);
        }
        println!("{}", parts);
    } else if file_size > 0 {
        println!("Size: {} bytes", file_size);
    }

    // Stream details.
    for s in demuxer.streams() {
        let p = &s.params;
        print!(
            "  Stream #{} [{:?}]  codec={}  time_base={}",
            s.index,
            p.media_type,
            p.codec_id,
            s.time_base.as_rational()
        );
        if let (Some(ch), Some(sr)) = (p.channels, p.sample_rate) {
            print!("  audio {}ch @ {} Hz", ch, sr);
            if let Some(fmt) = p.sample_format {
                print!("  [{:?}]", fmt);
            }
            // Uncompressed PCM-style bitrate estimate when params allow.
            if let Some(fmt) = p.sample_format {
                let bps = fmt.bytes_per_sample() * 8;
                let est = (sr as u64) * (ch as u64) * (bps as u64);
                if est > 0 {
                    print!("  {} kb/s", est / 1000);
                }
            }
        }
        if let (Some(w), Some(h)) = (p.width, p.height) {
            print!("  video {}x{}", w, h);
            if let Some(pf) = p.pixel_format {
                print!("  [{:?}]", pf);
            }
        }
        if let Some(br) = p.bit_rate {
            print!("  {} bps", br);
        }
        if let Some(d) = s.duration {
            print!("  duration={} ticks ({:.3}s)", d, s.time_base.seconds_of(d));
        }
        println!();
    }
    Ok(())
}

/// Pretty-print a byte count as `N B`, `N KB`, or `N MB`. Uses
/// kilobyte-is-1024 rounding — this is for human display only, not
/// anything that round-trips.
fn human_bytes(n: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = 1024 * 1024;
    if n >= MB {
        format!("{:.1} MB", (n as f64) / (MB as f64))
    } else if n >= KB {
        format!("{} KB", n / KB)
    } else {
        format!("{} B", n)
    }
}

/// Format microseconds as `HH:MM:SS.cc` (ffprobe-compatible).
fn format_duration_hhmmss(micros: i64) -> String {
    let total_s = (micros as f64) / 1_000_000.0;
    let h = (total_s / 3600.0) as u64;
    let m = ((total_s % 3600.0) / 60.0) as u64;
    let s = total_s - (h as f64) * 3600.0 - (m as f64) * 60.0;
    format!("{:02}:{:02}:{:05.2}", h, m, s)
}

fn cmd_remux(
    reg: &Registries,
    sources: &SourceRegistry,
    input: &str,
    output: &Path,
    format_override: Option<&str>,
    buffer_bytes: usize,
) -> oxideav::core::Result<()> {
    let (in_format, fin) = detect_input_format(reg, sources, input, buffer_bytes)?;
    let out_format = match format_override {
        Some(f) => f.to_owned(),
        None => format_for_output_path(reg, output)?,
    };

    let mut demuxer = reg.containers.open_demuxer(&in_format, fin, &reg.codecs)?;

    let fout: Box<dyn oxideav::core::WriteSeek> = Box::new(std::fs::File::create(output)?);
    let mut muxer = reg
        .containers
        .open_muxer(&out_format, fout, demuxer.streams())?;

    let n = oxideav::pipeline::remux(&mut *demuxer, &mut *muxer)?;
    println!(
        "Remuxed {} packet(s) from {} ({}) → {} ({})",
        n,
        input,
        in_format,
        output.display(),
        out_format,
    );
    Ok(())
}

/// Per-media-type codec overrides for `cmd_transcode`. `all` is the
/// catch-all `--codec` flag; the typed fields supersede it for that
/// media type when set.
#[derive(Clone, Copy)]
struct TranscodeCodecOverrides<'a> {
    all: Option<&'a str>,
    audio: Option<&'a str>,
    video: Option<&'a str>,
    subtitle: Option<&'a str>,
}

impl<'a> TranscodeCodecOverrides<'a> {
    fn for_media(&self, media: oxideav::core::MediaType) -> Option<&'a str> {
        use oxideav::core::MediaType::*;
        let typed = match media {
            Audio => self.audio,
            Video => self.video,
            Subtitle => self.subtitle,
            Data | Unknown => None,
        };
        typed.or(self.all)
    }
}

fn cmd_transcode(
    reg: &Registries,
    sources: &SourceRegistry,
    input: &str,
    output: &Path,
    overrides: TranscodeCodecOverrides<'_>,
    format_override: Option<&str>,
    buffer_bytes: usize,
    prefs: &oxideav::pipeline::CodecPreferences,
) -> oxideav::core::Result<()> {
    use oxideav::core::{MediaType, SampleFormat, StreamInfo};
    use oxideav::pipeline::{transcode_simple_with, StreamPlan};

    let (in_format, fin) = detect_input_format(reg, sources, input, buffer_bytes)?;
    let out_format = match format_override {
        Some(f) => f.to_owned(),
        None => format_for_output_path(reg, output)?,
    };
    let mut demuxer = reg.containers.open_demuxer(&in_format, fin, &reg.codecs)?;

    // Per-stream plan: pick a codec for each input stream based on the
    // override flags + media-type defaults. The closure is invoked once
    // per input stream by `transcode_simple` during set-up.
    //
    // Default policy:
    //   * Audio without an override → matching PCM variant (matches the
    //     historical single-stream behaviour for FLAC → WAV etc.).
    //   * Video / subtitle / data without an override → stream-copy. Any
    //     override forces re-encode through the named codec.
    let plan_for = move |stream: &StreamInfo| -> oxideav::core::Result<StreamPlan> {
        let media = stream.params.media_type;
        if let Some(codec) = overrides.for_media(media) {
            return Ok(StreamPlan::Reencode {
                output_codec: codec.to_owned(),
            });
        }
        match media {
            MediaType::Audio => {
                let fmt = stream.params.sample_format.unwrap_or(SampleFormat::S16);
                let codec = match fmt {
                    SampleFormat::U8 => "pcm_u8",
                    SampleFormat::S16 => "pcm_s16le",
                    SampleFormat::S24 => "pcm_s24le",
                    SampleFormat::S32 => "pcm_s32le",
                    SampleFormat::F32 => "pcm_f32le",
                    SampleFormat::F64 => "pcm_f64le",
                    _ => "pcm_s16le",
                };
                Ok(StreamPlan::Reencode {
                    output_codec: codec.to_owned(),
                })
            }
            // No safe re-encode default for video / subtitle / data — fall
            // through to stream-copy so the muxer carries the source
            // packets verbatim. Users wanting transcode pass --codec-video
            // / --codec-subtitle explicitly.
            MediaType::Video | MediaType::Subtitle | MediaType::Data | MediaType::Unknown => {
                Ok(StreamPlan::Copy)
            }
        }
    };

    let fout: Box<dyn oxideav::core::WriteSeek> = Box::new(std::fs::File::create(output)?);
    let registries_containers = &reg.containers;
    let out_format_owned = out_format.clone();
    let muxer_open = move |streams: &[oxideav::core::StreamInfo]| {
        registries_containers.open_muxer(&out_format_owned, fout, streams)
    };

    let stats = transcode_simple_with(&mut *demuxer, muxer_open, &reg.codecs, prefs, plan_for)?;
    println!(
        "Transcoded {} → {} ({} stream{}): {} pkts in, {} frames decoded, {} pkts out",
        input,
        output.display(),
        out_format,
        if stats.packets_out == 1 { "" } else { "s" },
        stats.packets_in,
        stats.frames_decoded,
        stats.packets_out,
    );
    Ok(())
}

/// Detect an input file's container format by reading a probe buffer
/// and asking each registered demuxer to score it. The file extension
/// is passed as a hint so weak signatures (e.g. raw MP3 with no ID3v2)
/// still round-trip when the user names the file correctly.
///
/// Returns the detected format name plus the open file handle, with
/// the cursor positioned at byte 0 ready for `open_demuxer`.
fn detect_input_format(
    reg: &Registries,
    sources: &SourceRegistry,
    input: &str,
    buffer_bytes: usize,
) -> oxideav::core::Result<(String, Box<dyn ReadSeek>)> {
    // The cli's probe / remux / transcode commands run on a
    // bytes-shape demuxer pipeline. Packet-shape sources (e.g. the
    // `rtmp://` driver) and frame-shape sources (e.g. `generate://`
    // for video) skip the demux layer entirely — those need the
    // executor (`oxideav run` JSON job) which already branches per
    // [`SourceOutput`] variant. We surface a clear error here rather
    // than try to fake a [`ReadSeek`] over them.
    let raw = match sources.open(input)? {
        SourceOutput::Bytes(b) => b,
        SourceOutput::Packets(_) => {
            return Err(Error::unsupported(format!(
                "{input}: packet-shape source (e.g. rtmp://) — wire it through `oxideav run` JSON job"
            )));
        }
        SourceOutput::Frames(_) => {
            return Err(Error::unsupported(format!(
                "{input}: frame-shape source (e.g. generate:// video) — wire it through `oxideav run` JSON job"
            )));
        }
    };
    // BytesSource: Read + Seek + Send; ReadSeek: Read + Seek. Re-box to
    // drop the Send bound the demuxer trait doesn't ask for.
    let raw: Box<dyn ReadSeek> = Box::new(raw);
    let mut handle: Box<dyn ReadSeek> = if buffer_bytes > 0 {
        Box::new(BufferedSource::new(raw, buffer_bytes)?)
    } else {
        raw
    };
    let ext = ext_from_uri(input);
    let format = reg.containers.probe_input(&mut *handle, ext.as_deref())?;
    Ok((format, handle))
}

/// Best-effort extension hint from a URI: takes everything after the
/// last `/`-segment's `.`, ignoring `?…` query strings.
fn ext_from_uri(uri: &str) -> Option<String> {
    let last = uri.rsplit('/').next().unwrap_or(uri);
    let last = last.split('?').next().unwrap_or(last);
    let dot = last.rfind('.')?;
    Some(last[dot + 1..].to_ascii_lowercase())
}

/// Pick a container format for an output path. The file doesn't exist
/// yet, so this falls back to the extension table.
fn format_for_output_path(reg: &Registries, path: &Path) -> oxideav::core::Result<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| Error::FormatNotFound(format!("no extension on {}", path.display())))?;
    reg.containers
        .container_for_extension(ext)
        .map(|s| s.to_owned())
        .ok_or_else(|| Error::FormatNotFound(format!("no container registered for .{ext}")))
}

fn read_job_source(file: Option<String>, inline: Option<String>) -> oxideav::core::Result<String> {
    if let Some(s) = inline {
        return Ok(s);
    }
    let path =
        file.ok_or_else(|| Error::invalid("no job source (pass a file path or --inline)"))?;
    if path == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        return Ok(buf);
    }
    Ok(std::fs::read_to_string(&path)?)
}

fn parse_job(
    file: Option<String>,
    inline: Option<String>,
) -> oxideav::core::Result<oxideav::pipeline::Job> {
    let raw = read_job_source(file, inline)?;
    oxideav::pipeline::Job::from_json(&raw)
}

fn cmd_run(
    reg: &Registries,
    sources: &SourceRegistry,
    file: Option<String>,
    inline: Option<String>,
    threads: usize,
) -> oxideav::core::Result<()> {
    let _ = sources; // sources are already in `reg.sources`; the param is kept for back-compat.
    let job = parse_job(file, inline)?;
    let stats = oxideav::pipeline::Executor::new(&job, reg)
        .with_threads(threads)
        .run()?;
    eprintln!(
        "oxideav run: {} packets read ({} copied, {} encoded), {} frames decoded",
        stats.packets_read, stats.packets_copied, stats.packets_encoded, stats.frames_decoded,
    );
    Ok(())
}

fn cmd_validate(file: Option<String>, inline: Option<String>) -> oxideav::core::Result<()> {
    let job = parse_job(file, inline)?;
    job.validate()?;
    // Also try to build the DAG so we surface any resolve-level errors
    // without needing an execution.
    let dag = job.to_dag()?;
    println!("OK: {} output(s)", dag.roots.len());
    Ok(())
}

fn cmd_dry_run(file: Option<String>, inline: Option<String>) -> oxideav::core::Result<()> {
    let job = parse_job(file, inline)?;
    job.validate()?;
    let dag = job.to_dag()?;
    print!("{}", dag.describe());
    Ok(())
}

/// Static lookup from a codec backend's `caps.implementation` string
/// (e.g. `"h264_nvdec"`, `"vaapi-h264"`, `"h264_vulkan"`) to the
/// `(engine_id, engine_probe_fn)` pair the corresponding HW sibling
/// crate ships.
///
/// Phase 1 added `engine_id` / `engine_probe` fields to
/// [`oxideav_core::CodecInfo`] so backends could attach this metadata
/// per registration. Phase 2 wired every HW sibling's registrar to set
/// `.with_engine_id(...).with_engine_probe(engine_info)`. Phase 3
/// (this function) is the consumer side: the CLI's `info <codec>`
/// command needs to read `engine_id` + `engine_probe` back out.
///
/// The current `oxideav-core` registry stores `CodecInfo` into a
/// `CodecImplementation` and intentionally drops the engine fields on
/// the floor (`engine_id: _, engine_probe: _`) at registration time.
/// Until that registry plumbing lands, the CLI recovers the engine
/// identity from the implementation name by matching against the
/// backend-specific naming conventions every HW sibling already uses.
/// This is _not_ a separate registry — there is no
/// `#[distributed_slice]`, no init-time collection, and no per-backend
/// sign-up. It is a static table of the four (currently) known engine
/// names, each pointing at the public `engine_info()` function that
/// sibling crate already exposes.
fn engine_for_impl(impl_name: &str) -> Option<(&'static str, oxideav_core::EngineProbeFn)> {
    #[cfg(target_os = "linux")]
    {
        if impl_name.contains("nvdec") || impl_name.contains("nvenc") {
            return Some(("nvidia", oxideav_nvidia::engine_info));
        }
        if impl_name.starts_with("vaapi") || impl_name.contains("_vaapi") {
            return Some(("vaapi", oxideav_vaapi::engine_info));
        }
        if impl_name.contains("vdpau") {
            return Some(("vdpau", oxideav_vdpau::engine_info));
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        if impl_name.contains("vulkan") {
            return Some(("vulkan-video", oxideav_vulkan_video::engine_info));
        }
    }
    let _ = impl_name; // suppress unused on platforms where no arm fires
    None
}

/// Format a byte count as a short human-readable string. Mirrors the
/// IEC binary scheme (1 KiB = 1024 B). Used for `total_memory_bytes`
/// in the `info` device block.
fn format_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    const TIB: u64 = GIB * 1024;
    if n < KIB {
        format!("{n} B")
    } else if n < MIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else if n < GIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n < TIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else {
        format!("{:.1} TiB", n as f64 / TIB as f64)
    }
}

/// Render the device block for a HW backend whose `engine_probe`
/// returned at least one device. `caps_for_codec` is the codec id the
/// `info` command was asked about — used to filter each device's
/// `codecs: Vec<HwCodecCaps>` down to the single relevant entry.
fn print_device_block(devices: &[oxideav_core::HwDeviceInfo], caps_for_codec: &str) {
    if devices.is_empty() {
        println!("  devices        : (none detected)");
        return;
    }
    println!("  devices        :");
    for (idx, dev) in devices.iter().enumerate() {
        println!("    [{idx}] {}", dev.name);
        if let Some(drv) = &dev.driver_version {
            println!("        driver           {drv}");
        }
        if let Some(api) = &dev.api_version {
            println!("        api              {api}");
        }
        if let Some(mem) = dev.total_memory_bytes {
            println!("        memory           {}", format_bytes(mem));
        }
        for (k, v) in &dev.extra {
            println!("        {k:<16} {v}");
        }
        // Pick the codec entry that matches the queried codec id (case-
        // insensitive). Backends sometimes ship one entry per family
        // (h264, hevc, av1, ...) and we only care about the one we're
        // showing right now.
        let matched: Vec<&oxideav_core::HwCodecCaps> = dev
            .codecs
            .iter()
            .filter(|c| c.codec.eq_ignore_ascii_case(caps_for_codec))
            .collect();
        if matched.is_empty() {
            println!("        {caps_for_codec:<16} (no caps reported)");
            continue;
        }
        for cc in matched {
            let mut sides = String::new();
            match (cc.decode, cc.encode) {
                (true, true) => sides.push_str("decode + encode"),
                (true, false) => sides.push_str("decode"),
                (false, true) => sides.push_str("encode"),
                (false, false) => sides.push_str("(no sides)"),
            }
            let mut bits = String::new();
            if let (Some(w), Some(h)) = (cc.max_width, cc.max_height) {
                bits.push_str(&format!("  max {}x{}", w, h));
            } else if let Some(w) = cc.max_width {
                bits.push_str(&format!("  max width {}", w));
            } else if let Some(h) = cc.max_height {
                bits.push_str(&format!("  max height {}", h));
            }
            if let Some(d) = cc.max_bit_depth {
                bits.push_str(&format!("  {}-bit", d));
            }
            println!("        {} caps        {}{}", cc.codec, sides, bits);
            if !cc.profiles.is_empty() {
                println!(
                    "                         profiles: {}",
                    cc.profiles.join(", ")
                );
            }
            for (k, v) in &cc.extra {
                println!("                         {k} = {v}");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_info(reg: &Registries, codec_id: &str) -> oxideav::core::Result<()> {
    use oxideav::core::{CodecId, MediaType};
    use oxideav::pipeline::bench::system_info;
    use std::collections::HashMap;

    let id = CodecId::new(codec_id);
    let impls = reg.codecs.implementations(&id);
    if impls.is_empty() {
        return Err(Error::invalid(format!(
            "info: no implementations registered for `{codec_id}`"
        )));
    }

    let media_type = impls[0].caps.media_type;
    let sys = system_info();

    println!("Codec: {codec_id}");
    println!(
        "  media_type     : {}",
        match media_type {
            MediaType::Video => "Video",
            MediaType::Audio => "Audio",
            MediaType::Subtitle => "Subtitle",
            MediaType::Data => "Data",
            MediaType::Unknown => "Unknown",
        }
    );
    println!("  backends       : {}", impls.len());
    println!();

    // Sort by priority ascending (HW typically priority 10, SW 100+).
    let mut sorted: Vec<&oxideav::core::CodecImplementation> = impls.iter().collect();
    sorted.sort_by_key(|i| i.caps.priority);

    // Probe cache: engine_id -> Vec<HwDeviceInfo>. Each engine probe
    // (e.g. `oxideav_nvidia::engine_info`) is called at most once per
    // `info` invocation even when several backends (h264_nvdec,
    // h264_nvenc, hevc_nvdec, ...) share the same engine_id. The cache
    // is local to this function — there is no global registry, the
    // probe runs are not memoised across processes.
    let mut probe_cache: HashMap<&'static str, Vec<oxideav_core::HwDeviceInfo>> = HashMap::new();

    for imp in &sorted {
        let caps = &imp.caps;
        if !caps.hardware_accelerated {
            continue;
        }
        if let Some((engine_id, probe_fn)) = engine_for_impl(&caps.implementation) {
            probe_cache.entry(engine_id).or_insert_with(probe_fn);
        }
    }

    for imp in sorted {
        let caps = &imp.caps;
        let hw_tag = if caps.hardware_accelerated {
            "✓ HW"
        } else {
            ". SW"
        };
        println!(
            "Backend: {}  {}  prio={}",
            caps.implementation, hw_tag, caps.priority
        );

        // Sides supported.
        let sides = match (caps.decode, caps.encode) {
            (true, true) => "decode + encode",
            (true, false) => "decode only",
            (false, true) => "encode only",
            _ => "registered without decode/encode (likely a probe-only entry)",
        };
        println!("  sides          : {sides}");

        // Engine + per-device block. For HW backends we look up the
        // engine_id by implementation name (the static table in
        // `engine_for_impl`) and pull the cached probe result for the
        // device list. For SW we keep the legacy CPU-brand line.
        if caps.hardware_accelerated {
            match engine_for_impl(&caps.implementation) {
                Some((engine_id, _probe_fn)) => {
                    println!("  engine         : {engine_id}");
                    if let Some(devices) = probe_cache.get(engine_id) {
                        print_device_block(devices, codec_id);
                    } else {
                        // Probe lookup table found the engine_id but the
                        // cache miss means a logic error above. Fall
                        // through to the no-probe message rather than
                        // panic.
                        println!("  engine_note    : (probe table mismatch — this is a CLI bug)");
                    }
                }
                None => {
                    // Implementation name didn't match any engine id we
                    // know about. Either a future HW backend the CLI
                    // hasn't been taught yet, or a backend that didn't
                    // wire `with_engine_probe` — fall back to the
                    // legacy single-line label.
                    let engine = sys
                        .hw_accel_engine
                        .clone()
                        .unwrap_or_else(|| "(hardware backend; engine probe not available)".into());
                    println!("  engine         : {engine}");
                }
            }
        } else {
            let engine = format!("{} (software, {} cores)", sys.cpu_brand, sys.cpu_cores);
            println!("  engine         : {engine}");
        }

        // Capability flags.
        let mut flags: Vec<&str> = Vec::new();
        if caps.intra_only {
            flags.push("intra-only");
        }
        if caps.lossy {
            flags.push("lossy");
        }
        if caps.lossless {
            flags.push("lossless");
        }
        if !flags.is_empty() {
            println!("  flags          : {}", flags.join(" + "));
        }

        // Limits — print only the fields that are set, since most
        // backends register with all-`None`. Collect first, then emit
        // the section header iff any were declared.
        let mut limits: Vec<(&str, String)> = Vec::new();
        if let Some(w) = caps.max_width {
            limits.push(("max_width", w.to_string()));
        }
        if let Some(h) = caps.max_height {
            limits.push(("max_height", h.to_string()));
        }
        if let Some(br) = caps.max_bitrate {
            limits.push(("max_bitrate", format!("{br} bps")));
        }
        if let Some(sr) = caps.max_sample_rate {
            limits.push(("max_sample_rate", format!("{sr} Hz")));
        }
        if let Some(ch) = caps.max_channels {
            limits.push(("max_channels", ch.to_string()));
        }
        if limits.is_empty() {
            println!("  limits         : (none declared)");
        } else {
            println!("  limits         :");
            for (k, v) in limits {
                println!("    {k:<16} {v}");
            }
        }

        // Accepted pixel formats — only meaningful for video.
        if !caps.accepted_pixel_formats.is_empty() {
            let fmts: Vec<String> = caps
                .accepted_pixel_formats
                .iter()
                .map(|f| format!("{f:?}"))
                .collect();
            println!("  pixel_formats  : {}", fmts.join(", "));
        }

        // Per-impl decoder + encoder option schemas.
        print_options_schema("Decoder options", imp.decoder_options_schema);
        print_options_schema("Encoder options", imp.encoder_options_schema);
        println!();
    }

    Ok(())
}

fn print_options_schema(label: &str, schema: Option<&'static [oxideav::core::OptionField]>) {
    use oxideav::core::{OptionKind, OptionValue};
    let Some(fields) = schema else {
        return;
    };
    if fields.is_empty() {
        return;
    }
    println!("  {label} ({}):", fields.len());
    let name_w = fields
        .iter()
        .map(|f| f.name.len())
        .max()
        .unwrap_or(0)
        .max(4);
    for f in fields {
        let kind = match &f.kind {
            OptionKind::Bool => "bool".to_string(),
            OptionKind::U32 => "u32".to_string(),
            OptionKind::I32 => "i32".to_string(),
            OptionKind::F32 => "f32".to_string(),
            OptionKind::String => "string".to_string(),
            OptionKind::Enum(variants) => format!("enum[{}]", variants.join("|")),
        };
        let default = match &f.default {
            OptionValue::Bool(b) => b.to_string(),
            OptionValue::U32(n) => n.to_string(),
            OptionValue::I32(n) => n.to_string(),
            OptionValue::F32(n) => format!("{n}"),
            OptionValue::String(s) => format!("\"{s}\""),
        };
        println!(
            "    {:<w$}  {:<24}  default={:<10}  {}",
            f.name,
            kind,
            default,
            f.help,
            w = name_w
        );
    }
}

fn cmd_bench(
    reg: &Registries,
    codec: Option<String>,
    all: bool,
    width: u32,
    height: u32,
    duration: f64,
    prep_frames: u32,
    side: &str,
) -> oxideav::core::Result<()> {
    use oxideav::core::MediaType;
    use oxideav::pipeline::bench::{
        run_bench_all_with, run_bench_with, system_info, BenchEvent, BenchOpts, BenchResult,
        BenchSide, Side,
    };

    let parsed_side = match side {
        "decode" => Side::Decode,
        "encode" => Side::Encode,
        "both" => Side::Both,
        other => {
            return Err(Error::invalid(format!(
                "--side must be decode|encode|both (got `{other}`)"
            )));
        }
    };
    if !all && codec.is_none() {
        return Err(Error::invalid("bench: pass a codec id, or use --all"));
    }

    let opts = BenchOpts {
        prep_frames,
        bench_duration_secs: duration,
        width,
        height,
        side: parsed_side,
        ..Default::default()
    };

    // Header: parameters + system info.
    let sys = system_info();
    println!("System:");
    println!("  OS              : {}", sys.os);
    println!(
        "  CPU             : {} ({} core{})",
        sys.cpu_brand,
        sys.cpu_cores,
        if sys.cpu_cores == 1 { "" } else { "s" }
    );
    if let Some(hw) = &sys.hw_accel_engine {
        println!("  HW accel engine : {hw}");
    }
    println!();
    println!("Bench parameters:");
    println!("  prep frames     : {}", opts.prep_frames);
    println!("  bench duration  : {:.1} s/run", opts.bench_duration_secs);
    println!("  side            : {}", side);
    println!();
    println!("Video defaults:");
    println!("  resolution      : {}×{}", opts.width, opts.height);
    println!("  pixel format    : {:?}", opts.pix_fmt);
    println!(
        "  framerate       : {} fps ({}/{})",
        opts.fps_num as f64 / opts.fps_den as f64,
        opts.fps_num,
        opts.fps_den
    );
    println!("  target bitrate  : {} kbit/s", opts.bitrate_video / 1000);
    println!();
    println!("Audio defaults:");
    println!(
        "  sample rate     : {} Hz, {:?}, {} channel{}",
        opts.sample_rate,
        opts.sample_format,
        opts.channels,
        if opts.channels == 1 { "" } else { "s" }
    );
    println!("  target bitrate  : {} kbit/s", opts.bitrate_audio / 1000);
    println!();

    // Streaming progress to stderr; full table accumulated for the
    // summary at the end.
    let on_event = |ev: BenchEvent| match ev {
        BenchEvent::CodecStart {
            codec_id,
            media_type,
            n_impls,
        } => {
            let kind = match media_type {
                MediaType::Video => "video",
                MediaType::Audio => "audio",
                _ => "other",
            };
            eprintln!(
                "[{kind}] {codec_id} ({n_impls} impl{})",
                if n_impls == 1 { "" } else { "s" }
            );
        }
        BenchEvent::PrepStart { .. } => {
            eprint!("  prep... ");
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
        BenchEvent::PrepDone {
            encoder_used,
            prep_packets,
            ..
        } => {
            if let Some(name) = encoder_used {
                eprintln!("done — {prep_packets} packets via {name}");
            } else {
                eprintln!("done");
            }
        }
        BenchEvent::PrepFailed { reason, .. } => {
            eprintln!("FAILED: {reason}");
        }
        BenchEvent::BenchStart {
            backend,
            side: bs,
            hw,
            priority,
            ..
        } => {
            let hw_tag = if hw { "✓ HW" } else { ". SW" };
            let side_tag = match bs {
                BenchSide::Decode => "decode",
                BenchSide::Encode => "encode",
            };
            eprint!("  {side_tag:<6} {backend:<22} {hw_tag} prio={priority}... ");
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
        BenchEvent::BenchDone { result } => match (result.fps, result.realtime) {
            (Some(fps), Some(rt)) => eprintln!("{fps:>10.1} fps  ({rt:>5.1}× realtime)"),
            _ => eprintln!(
                "FAIL: {}",
                result.error.as_deref().unwrap_or("unknown error")
            ),
        },
        BenchEvent::CodecDone { .. } => {
            eprintln!();
        }
    };

    let mut results: Vec<BenchResult> = if all {
        run_bench_all_with(&reg.codecs, &opts, on_event)
    } else {
        let id = codec.unwrap();
        let r = run_bench_with(&reg.codecs, &id, &opts, on_event);
        if r.is_empty() {
            return Err(Error::invalid(format!(
                "bench: no implementations registered for `{id}`"
            )));
        }
        r
    };

    println!("Summary:");

    let mut last_kind = None;
    let mut last_codec: Option<String> = None;
    results.sort_by(|a, b| {
        let mt_rank = |m: MediaType| match m {
            MediaType::Video => 0,
            MediaType::Audio => 1,
            _ => 2,
        };
        mt_rank(a.media_type)
            .cmp(&mt_rank(b.media_type))
            .then(a.codec_id.cmp(&b.codec_id))
            .then(a.priority.cmp(&b.priority))
            .then((a.side as i32).cmp(&(b.side as i32)))
    });

    for r in &results {
        let kind = r.media_type;
        if Some(kind) != last_kind {
            println!();
            println!(
                "{} codecs:",
                match kind {
                    MediaType::Video => "Video",
                    MediaType::Audio => "Audio",
                    _ => "Other",
                }
            );
            println!(
                "  {:<14}   {:<22}   {:<6}  {:<3}  {:>10}  {:>8}",
                "Codec", "Backend", "Side", "HW", "fps", "realtime"
            );
            println!(
                "  {:─<14}   {:─<22}   {:─<6}  {:─<3}  {:─>10}  {:─>8}",
                "", "", "", "", "", ""
            );
            last_kind = Some(kind);
            last_codec = None;
        }
        let codec_cell = if last_codec.as_deref() == Some(&r.codec_id) {
            String::new()
        } else {
            last_codec = Some(r.codec_id.clone());
            r.codec_id.clone()
        };
        let side_str = match r.side {
            BenchSide::Decode => "decode",
            BenchSide::Encode => "encode",
        };
        let hw_str = if r.hw { "✓" } else { "." };
        let (fps_str, rt_str) = match (r.fps, r.realtime) {
            (Some(fps), Some(rt)) => (format!("{:.1}", fps), format!("{:.1}×", rt)),
            _ => (
                r.error.clone().unwrap_or_else(|| "—".into()),
                String::from("—"),
            ),
        };
        println!(
            "  {:<14}   {:<22}   {:<6}  {:<3}  {:>10}  {:>8}",
            codec_cell, r.backend, side_str, hw_str, fps_str, rt_str
        );
    }
    Ok(())
}
