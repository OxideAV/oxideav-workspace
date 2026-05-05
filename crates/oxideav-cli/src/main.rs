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
    /// Today this is single-stream only. The output codec defaults to a PCM
    /// variant matching the decoded sample format (e.g. FLAC 16-bit → pcm_s16le).
    Transcode {
        /// Input URI: local path, file:// URL, or http(s):// URL.
        input: String,
        output: PathBuf,
        /// Override the output codec id (e.g. "pcm_s16le", "pcm_f32le").
        #[arg(long)]
        codec: Option<String>,
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
    // `mut` is only needed when the `rtmp` feature wires
    // `oxideav_rtmp::register(&mut registries.sources)` below; without
    // it the registry is build-once / read-only.
    #[allow(unused_mut)]
    let mut registries = if cli.no_hwaccel {
        // Skip the hardware-accelerated sibling crates' registrars so
        // their codec implementations don't enter the registry.
        // Pure-Rust impls register at priority 100+ (vs hardware at
        // priority 0), so just removing the HW entries hands the
        // dispatch over to the pure-Rust path automatically — no
        // priority surgery required.
        oxideav::with_all_features_filtered(|name| !matches!(name, "videotoolbox" | "audiotoolbox"))
    } else {
        Registries::with_all_features()
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
            format,
        } => cmd_transcode(
            &registries,
            sources,
            &input,
            &output,
            codec.as_deref(),
            format.as_deref(),
            buffer_bytes,
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
    let mut containers: Vec<_> = reg.containers.demuxer_names().collect();
    containers.sort_unstable();
    println!("Containers (demux):");
    for c in &containers {
        println!("  {c}");
    }

    let mut muxers: Vec<_> = reg.containers.muxer_names().collect();
    muxers.sort_unstable();
    println!("Containers (mux):");
    for c in &muxers {
        println!("  {c}");
    }

    println!();
    println!("Codecs:");
    println!(" D..... = Decoding supported");
    println!(" .E.... = Encoding supported");
    println!(" ..V... = Video codec");
    println!(" ..A... = Audio codec");
    println!(" ..S... = Subtitle codec");
    println!(" ..D... = Data codec");
    println!(" ..T... = Attachment codec");
    println!(" ...I.. = Intra frame-only codec");
    println!(" ....L. = Lossy compression");
    println!(" .....S = Lossless compression");
    println!(" ------");
    let mut rows: Vec<(String, String, String, bool)> = reg
        .codecs
        .all_implementations()
        .map(|(id, im)| {
            (
                im.caps.flag_string(),
                id.as_str().to_owned(),
                im.caps.implementation.clone(),
                im.caps.hardware_accelerated,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));
    for (flags, id, implementation, hw) in rows {
        let hw_tag = if hw { "  [HW]" } else { "" };
        println!("  {flags}  {id:<14}  ({implementation}){hw_tag}");
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

fn cmd_transcode(
    reg: &Registries,
    sources: &SourceRegistry,
    input: &str,
    output: &Path,
    codec_override: Option<&str>,
    format_override: Option<&str>,
    buffer_bytes: usize,
) -> oxideav::core::Result<()> {
    use oxideav::core::SampleFormat;
    use oxideav::pipeline::{transcode_simple, StreamPlan};

    let (in_format, fin) = detect_input_format(reg, sources, input, buffer_bytes)?;
    let out_format = match format_override {
        Some(f) => f.to_owned(),
        None => format_for_output_path(reg, output)?,
    };
    let mut demuxer = reg.containers.open_demuxer(&in_format, fin, &reg.codecs)?;

    // Pick an output codec. If user supplied one, use it. Otherwise pick a
    // PCM variant that matches the input stream's natural bit depth.
    let codec = match codec_override {
        Some(c) => c.to_owned(),
        None => {
            let in_streams = demuxer.streams();
            let stream = in_streams
                .first()
                .ok_or_else(|| oxideav::core::Error::invalid("no streams"))?;
            let fmt = stream.params.sample_format.unwrap_or(SampleFormat::S16);
            match fmt {
                SampleFormat::U8 => "pcm_u8",
                SampleFormat::S16 => "pcm_s16le",
                SampleFormat::S24 => "pcm_s24le",
                SampleFormat::S32 => "pcm_s32le",
                SampleFormat::F32 => "pcm_f32le",
                SampleFormat::F64 => "pcm_f64le",
                _ => "pcm_s16le",
            }
            .to_owned()
        }
    };

    let plan = StreamPlan::Reencode {
        output_codec: codec.clone(),
    };

    let fout: Box<dyn oxideav::core::WriteSeek> = Box::new(std::fs::File::create(output)?);
    let registries_containers = &reg.containers;
    let out_format_owned = out_format.clone();
    let muxer_open = move |streams: &[oxideav::core::StreamInfo]| {
        registries_containers.open_muxer(&out_format_owned, fout, streams)
    };

    let stats = transcode_simple(&mut *demuxer, muxer_open, &reg.codecs, &plan)?;
    println!(
        "Transcoded {} → {} ({}): {} pkts in, {} frames decoded, {} pkts out",
        input,
        output.display(),
        codec,
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
