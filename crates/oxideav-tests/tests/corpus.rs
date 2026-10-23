//! Playback corpus tracker (umbrella issue #4, "progress trackers").
//!
//! Drives every fixture of `corpus/manifest.csv` through the two
//! user-facing binaries and grades the result the way the original
//! report did:
//!
//! * `oxideplay --vo hash --ao hash <file>` — headless playback of the
//!   default stream mapping; the hash sinks print how many frames /
//!   samples reached the output. A run that is not a clean PASS is
//!   re-run with one track at a time (`--vo hash --ao none`,
//!   `--vo none --ao hash`) so a stuck decoder on one track does not
//!   hide what the other one did.
//! * `oxideav transcode <file> <id>.y4m` and `… <id>.wav` — the CLI
//!   decode path; the raw outputs are counted (frames / samples per
//!   channel) and digested (MD5).
//!
//! Reference counts come from the black-box `ffprobe`
//! (`-count_frames` for video, summed `nb_samples` for audio). A
//! fixture PASSes when every frame and every sample came out and the
//! process reached end of stream on its own.
//!
//! The suite is a no-op unless `OXIDEAV_CORPUS_DIR` points at the
//! fixtures (`scripts/corpus-generate.py <dir>` builds them; they are
//! never checked in). It writes `report.md` + `results.csv` into
//! `OXIDEAV_CORPUS_OUT` (default `<dir>/report`) and only fails on a
//! harness-level problem, or when `OXIDEAV_CORPUS_MIN_PASS` is set
//! and fewer fixtures PASS.
//!
//! Environment:
//!
//! | variable | meaning |
//! |---|---|
//! | `OXIDEAV_CORPUS_DIR` | fixture directory (required to run) |
//! | `OXIDEAV_CORPUS_OUT` | where `report.md` / `results.csv` go |
//! | `OXIDEPLAY_BIN` / `OXIDEAV_BIN` | binaries (default: `$CARGO_TARGET_DIR` or the workspace `target/`, release then debug) |
//! | `OXIDEAV_CORPUS_TIMEOUT` | seconds per process (default 30) |
//! | `OXIDEAV_CORPUS_ONLY` | comma-separated fixture ids to (re)run; every other row is kept from the previous `results.csv` |
//! | `OXIDEAV_CORPUS_RERUN` | `failing` — re-run every row of the previous `results.csv` that is not PASS |
//! | `OXIDEAV_CORPUS_MIN_PASS` | fail the test below this PASS count |
//! | `OXIDEAV_CORPUS_PLAYER_ARGS` | extra oxideplay flags (e.g. `--hwaccel`) |
//! | `OXIDEAV_CORPUS_CLI_ARGS` | extra oxideav global flags (e.g. `--hwaccel`) |

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// A video decode counts as "first frame only" up to this many frames
/// (the original report's FIRST_FRAME bucket).
const FIRST_FRAME_MAX: u64 = 10;

#[test]
fn playback_corpus() {
    let Some(dir) = std::env::var_os("OXIDEAV_CORPUS_DIR") else {
        eprintln!(
            "SKIP: OXIDEAV_CORPUS_DIR not set (scripts/corpus-generate.py builds the fixtures)"
        );
        return;
    };
    let cfg = Config::from_env(PathBuf::from(dir)).unwrap_or_else(|e| panic!("corpus: {e}"));
    let manifest = load_manifest(&cfg.manifest).unwrap_or_else(|e| panic!("corpus: {e}"));
    fs::create_dir_all(&cfg.out).unwrap_or_else(|e| panic!("corpus: {}: {e}", cfg.out.display()));
    let scratch = cfg.out.join("cli");
    fs::create_dir_all(&scratch).unwrap_or_else(|e| panic!("corpus: {}: {e}", scratch.display()));

    // Previous rows, reused for fixtures not selected this time.
    let csv_path = cfg.out.join("results.csv");
    let previous: HashMap<String, Record> = if cfg.only.is_some() || cfg.rerun_failing {
        read_records(&csv_path)
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.id().to_owned(), r))
            .collect()
    } else {
        HashMap::new()
    };
    let selected = |f: &Fixture| -> bool {
        if let Some(only) = &cfg.only {
            return only.contains(&f.id);
        }
        if cfg.rerun_failing {
            return previous
                .get(&f.id)
                .is_none_or(|r| r.verdict() != Verdict::Pass);
        }
        true
    };

    let started = Instant::now();
    let mut records: Vec<Record> = Vec::with_capacity(manifest.len());
    let mut ran = 0usize;
    for f in &manifest {
        let rec = if selected(f) {
            ran += 1;
            let r = run_fixture(&cfg, f, &scratch).into_record(&cfg);
            eprintln!(
                "{} {:<6} {:<12} {:<12} {:>6.1}s  {}",
                f.id,
                f.ext,
                r.verdict().as_str(),
                r.cli_verdict().as_str(),
                r.f64("elapsed_s"),
                f.title
            );
            r
        } else if let Some(prev) = previous.get(&f.id) {
            prev.clone()
        } else {
            continue;
        };
        records.push(rec);
    }
    assert!(ran > 0, "corpus: no fixtures selected");
    let wall = started.elapsed();

    write_csv(&csv_path, &records)
        .unwrap_or_else(|e| panic!("corpus: {}: {e}", csv_path.display()));
    let md_path = cfg.out.join("report.md");
    fs::write(&md_path, render_report(&cfg, &records, wall, ran))
        .unwrap_or_else(|e| panic!("corpus: {}: {e}", md_path.display()));
    let pass = records
        .iter()
        .filter(|r| r.verdict() == Verdict::Pass)
        .count();
    eprintln!(
        "corpus: {pass}/{} PASS ({ran} run) — {} and {} written",
        records.len(),
        md_path.display(),
        csv_path.display()
    );
    if let Some(min) = cfg.min_pass {
        assert!(
            pass >= min,
            "corpus: {pass} fixtures PASS, OXIDEAV_CORPUS_MIN_PASS={min}"
        );
    }
}

// ───────────────────────────── config ─────────────────────────────

struct Config {
    dir: PathBuf,
    out: PathBuf,
    manifest: PathBuf,
    oxideplay: PathBuf,
    oxideav: PathBuf,
    ffprobe: Option<PathBuf>,
    timeout: Duration,
    only: Option<Vec<String>>,
    rerun_failing: bool,
    min_pass: Option<usize>,
    player_args: Vec<String>,
    cli_args: Vec<String>,
}

impl Config {
    fn from_env(dir: PathBuf) -> Result<Self, String> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let manifest = std::env::var_os("OXIDEAV_CORPUS_MANIFEST")
            .map(PathBuf::from)
            .unwrap_or_else(|| manifest_dir.join("corpus").join("manifest.csv"));
        let out = std::env::var_os("OXIDEAV_CORPUS_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.join("report"));
        let oxideplay = find_bin("oxideplay", "OXIDEPLAY_BIN", &manifest_dir)?;
        let oxideav = find_bin("oxideav", "OXIDEAV_BIN", &manifest_dir)?;
        let ffprobe = find_on_path("ffprobe");
        if ffprobe.is_none() {
            eprintln!(
                "corpus: ffprobe not found — no reference counts, PASS = reached EOS with output"
            );
        }
        let timeout = std::env::var("OXIDEAV_CORPUS_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(DEFAULT_TIMEOUT_SECS));
        let only = std::env::var("OXIDEAV_CORPUS_ONLY")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .map(|x| format!("{x:0>3}"))
                    .collect()
            });
        let rerun_failing = std::env::var("OXIDEAV_CORPUS_RERUN")
            .map(|s| s == "failing")
            .unwrap_or(false);
        let min_pass = std::env::var("OXIDEAV_CORPUS_MIN_PASS")
            .ok()
            .and_then(|s| s.parse().ok());
        let split = |k: &str| -> Vec<String> {
            std::env::var(k)
                .map(|s| s.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default()
        };
        Ok(Self {
            dir,
            out,
            manifest,
            oxideplay,
            oxideav,
            ffprobe,
            timeout,
            only,
            rerun_failing,
            min_pass,
            player_args: split("OXIDEAV_CORPUS_PLAYER_ARGS"),
            cli_args: split("OXIDEAV_CORPUS_CLI_ARGS"),
        })
    }
}

fn find_bin(name: &str, env_key: &str, manifest_dir: &Path) -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os(env_key) {
        let p = PathBuf::from(p);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(format!("{env_key}={} is not a file", p.display()))
        };
    }
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(t) = std::env::var_os("CARGO_TARGET_DIR") {
        roots.push(PathBuf::from(t));
    }
    roots.push(manifest_dir.join("..").join("..").join("target"));
    for root in &roots {
        for profile in ["release", "debug"] {
            let p = root.join(profile).join(&exe);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "{name} binary not found (set {env_key}, or `cargo build --release -p {}`)",
        if name == "oxideav" {
            "oxideav-cli"
        } else {
            name
        }
    ))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

// ──────────────────────────── manifest ────────────────────────────

#[derive(Clone, Debug)]
struct Fixture {
    id: String,
    ext: String,
    section: String,
    title: String,
    ffmpeg_args: String,
}

fn load_manifest(path: &Path) -> Result<Vec<Fixture>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let records = parse_csv(&text);
    let mut it = records.into_iter();
    let header = it.next().ok_or("manifest is empty")?;
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| format!("manifest lacks column {name:?}"))
    };
    let (ci, ce, cs, ct, ca) = (
        col("id")?,
        col("ext")?,
        col("section")?,
        col("title")?,
        col("ffmpeg_args")?,
    );
    let mut out = Vec::new();
    for (n, rec) in it.enumerate() {
        if rec.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let get = |i: usize| rec.get(i).cloned().unwrap_or_default();
        if get(ci).is_empty() {
            return Err(format!("manifest row {}: empty id", n + 2));
        }
        out.push(Fixture {
            id: get(ci),
            ext: get(ce),
            section: get(cs),
            title: get(ct),
            ffmpeg_args: get(ca),
        });
    }
    Ok(out)
}

/// Minimal RFC 4180 reader: quoted fields, doubled quotes, CRLF.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

// ─────────────────────────── processes ────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Error,
    Timeout,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Error => "error",
            Status::Timeout => "timeout",
        }
    }
}

struct Run {
    status: Status,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Spawn `bin args…` with stdin closed, capture both pipes on helper
/// threads, kill it at `timeout`.
fn run_bounded(bin: &Path, args: &[String], timeout: Duration) -> Run {
    let started = Instant::now();
    let mut child: Child = match Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return Run {
                status: Status::Error,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("spawn {}: {e}", bin.display()),
            };
        }
    };
    let mut out_pipe = child.stdout.take().expect("piped stdout");
    let mut err_pipe = child.stderr.take().expect("piped stderr");
    let out_thread = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out_pipe.read_to_end(&mut v);
        v
    });
    let err_thread = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err_pipe.read_to_end(&mut v);
        v
    });
    let deadline = started + timeout;
    let mut status = Status::Ok;
    let mut exit_code = None;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                exit_code = st.code();
                if !st.success() {
                    status = Status::Error;
                }
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    status = Status::Timeout;
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                status = Status::Error;
                break;
            }
        }
    }
    let stdout = String::from_utf8_lossy(&out_thread.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).into_owned();
    Run {
        status,
        exit_code,
        stdout,
        stderr,
    }
}

// ─────────────────────────── reference ────────────────────────────

#[derive(Default, Clone, Debug)]
struct Reference {
    video_codec: Option<String>,
    video_wh: Option<String>,
    video_frames: Option<u64>,
    audio_codec: Option<String>,
    audio_ch: Option<u32>,
    audio_rate: Option<u32>,
    audio_samples: Option<u64>,
    /// Stream kinds ffprobe lists, e.g. `video,audio,data`.
    kinds: Vec<String>,
}

fn probe_reference(ffprobe: &Path, file: &Path, timeout: Duration) -> Reference {
    let mut r = Reference::default();
    let f = file.to_string_lossy().into_owned();
    // `key=value` lines (`-of default=noprint_wrappers=1`): ffprobe
    // prints entries in its own field order, not the order requested.
    let kv = |args: &[&str]| -> Vec<(String, String)> {
        let args: Vec<String> = ["-v", "error"]
            .iter()
            .map(|s| s.to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .chain(["-of".to_owned(), "default=noprint_wrappers=1".to_owned()])
            .chain(std::iter::once(f.clone()))
            .collect();
        run_bounded(ffprobe, &args, timeout)
            .stdout
            .lines()
            .filter_map(|l| {
                l.split_once('=')
                    .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            })
            .collect()
    };
    let first = |kvs: &[(String, String)], key: &str| -> Option<String> {
        kvs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };
    for (k, v) in kv(&["-show_entries", "stream=codec_type"]) {
        if k == "codec_type" && !v.is_empty() {
            r.kinds.push(v);
        }
    }
    let v = kv(&[
        "-select_streams",
        "v:0",
        "-count_frames",
        "-show_entries",
        "stream=codec_name,width,height,nb_read_frames",
    ]);
    if let Some(codec) = first(&v, "codec_name") {
        r.video_codec = Some(codec);
        r.video_wh = Some(format!(
            "{}x{}",
            first(&v, "width").unwrap_or_default(),
            first(&v, "height").unwrap_or_default()
        ));
        r.video_frames = first(&v, "nb_read_frames").and_then(|s| s.parse().ok());
    }
    let a = kv(&[
        "-select_streams",
        "a:0",
        "-show_entries",
        "stream=codec_name,channels,sample_rate",
    ]);
    if let Some(codec) = first(&a, "codec_name") {
        r.audio_codec = Some(codec);
        r.audio_ch = first(&a, "channels").and_then(|s| s.parse().ok());
        r.audio_rate = first(&a, "sample_rate").and_then(|s| s.parse().ok());
        let mut total = 0u64;
        let mut any = false;
        for (k, v) in kv(&[
            "-select_streams",
            "a:0",
            "-show_entries",
            "frame=nb_samples",
        ]) {
            if k == "nb_samples"
                && let Ok(n) = v.parse::<u64>()
            {
                total += n;
                any = true;
            }
        }
        if any {
            r.audio_samples = Some(total);
        }
    }
    r
}

// ───────────────────────────── player ─────────────────────────────

#[derive(Default, Clone, Debug)]
struct PlayerLeg {
    status: Option<Status>,
    exit_code: Option<i32>,
    vo_frames: Option<u64>,
    ao_samples: Option<u64>,
    vo_hash: Option<String>,
    ao_hash: Option<String>,
    streams: Vec<String>,
    video_codec: Option<String>,
    video_wh: Option<String>,
    audio_codec: Option<String>,
    audio_ch: Option<u32>,
    audio_rate: Option<u32>,
    error: Option<String>,
    stderr_tail: String,
}

fn play(cfg: &Config, file: &Path, vo: &str, ao: &str) -> PlayerLeg {
    let mut args: Vec<String> = vec!["--vo".into(), vo.into(), "--ao".into(), ao.into()];
    args.extend(cfg.player_args.iter().cloned());
    args.push(file.to_string_lossy().into_owned());
    let run = run_bounded(&cfg.oxideplay, &args, cfg.timeout);
    let mut leg = PlayerLeg {
        status: Some(run.status),
        exit_code: run.exit_code,
        ..Default::default()
    };
    let mut tail: Vec<&str> = Vec::new();
    for line in run.stderr.lines().chain(run.stdout.lines()) {
        let l = line.trim();
        if l.is_empty() || l.contains("vol 100%") {
            continue;
        }
        if let Some(rest) = l.strip_prefix("hash:vo:") {
            let (h, n) = split_hash_line(rest);
            leg.vo_hash = h;
            leg.vo_frames = n;
        } else if let Some(rest) = l.strip_prefix("hash:ao:") {
            let (h, n) = split_hash_line(rest);
            leg.ao_hash = h;
            leg.ao_samples = n;
        } else if let Some(rest) = l.strip_prefix("video:") {
            // `video: h264 320x180`
            let mut it = rest.split_whitespace();
            let codec = it.next().unwrap_or("").to_owned();
            let wh = it.next().unwrap_or("").to_owned();
            if leg.video_codec.is_none() {
                leg.video_codec = Some(codec.clone());
                leg.video_wh = Some(wh.clone());
            }
            leg.streams.push(format!("video:{codec}:{wh}"));
        } else if let Some(rest) = l.strip_prefix("audio:") {
            // `audio: aac 2ch @ 48000 Hz`
            let mut it = rest.split_whitespace();
            let codec = it.next().unwrap_or("").to_owned();
            let ch = it
                .next()
                .and_then(|s| s.trim_end_matches("ch").parse::<u32>().ok());
            let rate = it.nth(1).and_then(|s| s.parse::<u32>().ok());
            if leg.audio_codec.is_none() {
                leg.audio_codec = Some(codec.clone());
                leg.audio_ch = ch;
                leg.audio_rate = rate;
            }
            leg.streams
                .push(format!("audio:{codec}:{}ch", ch.unwrap_or(0)));
        } else if let Some(rest) = l.strip_prefix("oxideplay:") {
            let rest = rest.trim();
            if !rest.starts_with("playing") && leg.error.is_none() {
                leg.error = Some(rest.to_owned());
            }
            tail.push(l);
        } else if !l.starts_with("vo:") && !l.starts_with("ao:") {
            tail.push(l);
        }
    }
    if run.status == Status::Timeout {
        let msg = format!("timeout after {}s", cfg.timeout.as_secs());
        leg.error = Some(match leg.error.take() {
            Some(e) => format!("{msg}; {e}"),
            None => msg,
        });
    } else if run.status == Status::Error && leg.error.is_none() {
        leg.error = Some(format!(
            "exit {}",
            run.exit_code.map_or("signal".to_owned(), |c| c.to_string())
        ));
    }
    let keep = tail.len().saturating_sub(8);
    leg.stderr_tail = tail[keep..].join("\n");
    leg
}

/// `hash:vo: f942f95f99bb1d3d  (50 frames)` →
/// `(Some("f942f95f99bb1d3d"), Some(50))`.
fn split_hash_line(rest: &str) -> (Option<String>, Option<u64>) {
    let rest = rest.trim();
    let mut it = rest.split_whitespace();
    let hash = it.next().map(str::to_owned);
    let n = it
        .next()
        .map(|s| s.trim_start_matches('('))
        .and_then(|s| s.parse::<u64>().ok());
    (hash, n)
}

// ─────────────────────────────── cli ──────────────────────────────

#[derive(Default, Clone, Debug)]
struct CliLeg {
    video_frames: Option<u64>,
    audio_samples: Option<u64>,
    video_md5: Option<String>,
    audio_md5: Option<String>,
    error: Option<String>,
}

fn cli_decode(
    cfg: &Config,
    f: &Fixture,
    file: &Path,
    reference: &Reference,
    scratch: &Path,
) -> CliLeg {
    let mut leg = CliLeg::default();
    let mut errors: Vec<String> = Vec::new();
    let has_video = reference.video_codec.is_some() || reference.kinds.is_empty();
    let has_audio = reference.audio_codec.is_some() || reference.kinds.is_empty();
    if has_video {
        let out = scratch.join(format!("{}.y4m", f.id));
        let _ = fs::remove_file(&out);
        match transcode(cfg, file, &out) {
            Ok(()) => {
                if let Ok(bytes) = fs::read(&out) {
                    leg.video_frames = y4m_frame_count(&bytes);
                    leg.video_md5 = Some(md5_hex(&bytes));
                }
            }
            Err(e) => errors.push(format!("y4m: {e}")),
        }
        let _ = fs::remove_file(&out);
    }
    if has_audio {
        let out = scratch.join(format!("{}.wav", f.id));
        let _ = fs::remove_file(&out);
        match transcode(cfg, file, &out) {
            Ok(()) => {
                if let Ok(bytes) = fs::read(&out) {
                    leg.audio_samples = wav_sample_count(&bytes);
                    leg.audio_md5 = Some(md5_hex(&bytes));
                }
            }
            Err(e) => errors.push(format!("wav: {e}")),
        }
        let _ = fs::remove_file(&out);
    }
    if !errors.is_empty() {
        leg.error = Some(errors.join(" | "));
    }
    leg
}

fn transcode(cfg: &Config, input: &Path, output: &Path) -> Result<(), String> {
    let mut args: Vec<String> = cfg.cli_args.clone();
    args.push("transcode".into());
    args.push(input.to_string_lossy().into_owned());
    args.push(output.to_string_lossy().into_owned());
    let run = run_bounded(&cfg.oxideav, &args, cfg.timeout);
    match run.status {
        Status::Ok => Ok(()),
        Status::Timeout => Err(format!("timeout after {}s", cfg.timeout.as_secs())),
        Status::Error => {
            let msg = run
                .stderr
                .lines()
                .map(str::trim)
                .rev()
                .find(|l| l.starts_with("oxideav:"))
                .or_else(|| {
                    run.stderr
                        .lines()
                        .map(str::trim)
                        .rev()
                        .find(|l| !l.is_empty())
                })
                .unwrap_or("")
                .to_owned();
            Err(format!(
                "exit {}: {msg}",
                run.exit_code.map_or("signal".to_owned(), |c| c.to_string())
            ))
        }
    }
}

/// Count the frames of a YUV4MPEG2 stream from its header geometry
/// (no reliance on the `FRAME` marker appearing in the payload).
fn y4m_frame_count(bytes: &[u8]) -> Option<u64> {
    let nl = bytes.iter().position(|&b| b == b'\n')?;
    let header = std::str::from_utf8(&bytes[..nl]).ok()?;
    if !header.starts_with("YUV4MPEG2") {
        return None;
    }
    let mut w = 0u64;
    let mut h = 0u64;
    let mut chroma = "420".to_owned();
    for tag in header.split_whitespace().skip(1) {
        match tag.as_bytes()[0] {
            b'W' => w = tag[1..].parse().ok()?,
            b'H' => h = tag[1..].parse().ok()?,
            b'C' => chroma = tag[1..].to_owned(),
            _ => {}
        }
    }
    if w == 0 || h == 0 {
        return None;
    }
    let deep = ["p9", "p10", "p12", "p14", "p16"]
        .iter()
        .any(|d| chroma.contains(d));
    let bytes_per_sample = if deep { 2 } else { 1 };
    let luma = w * h;
    let chroma_samples = if chroma.starts_with("mono") {
        0
    } else if chroma.starts_with("420") {
        2 * w.div_ceil(2) * h.div_ceil(2)
    } else if chroma.starts_with("422") {
        2 * w.div_ceil(2) * h
    } else if chroma.starts_with("444") {
        2 * luma
    } else if chroma.starts_with("411") {
        2 * w.div_ceil(4) * h
    } else if chroma.starts_with("440") {
        2 * w * h.div_ceil(2)
    } else {
        return None;
    };
    let alpha = if chroma.contains("alpha") { luma } else { 0 };
    let frame = (luma + chroma_samples + alpha) * bytes_per_sample;
    let payload = (bytes.len() as u64).saturating_sub(nl as u64 + 1);
    // Every frame is `FRAME\n` (6 bytes, parameters ignored) plus the
    // planes.
    Some(payload / (frame + 6))
}

/// Samples per channel of a RIFF/WAVE file (PCM `fmt ` + `data`).
fn wav_sample_count(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut block_align: Option<u64> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]) as u64;
            let bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]) as u64;
            if channels > 0 && bits > 0 {
                block_align = Some(channels * bits.div_ceil(8));
            }
        } else if id == b"data" {
            let ba = block_align?;
            let avail = bytes.len().saturating_sub(body) as u64;
            let data = (size as u64).min(avail);
            return Some(data / ba);
        }
        pos = body + size + (size & 1);
    }
    None
}

// ─────────────────────────── verdicts ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    Pass,
    FirstFrame,
    Partial,
    Fail,
    Timeout,
    Missing,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::FirstFrame => "FIRST_FRAME",
            Verdict::Partial => "PARTIAL",
            Verdict::Fail => "FAIL",
            Verdict::Timeout => "TIMEOUT",
            Verdict::Missing => "MISSING",
        }
    }
    fn parse(s: &str) -> Verdict {
        match s {
            "PASS" => Verdict::Pass,
            "FIRST_FRAME" => Verdict::FirstFrame,
            "PARTIAL" => Verdict::Partial,
            "FAIL" => Verdict::Fail,
            "TIMEOUT" => Verdict::Timeout,
            _ => Verdict::Missing,
        }
    }
    const ALL: [Verdict; 6] = [
        Verdict::Pass,
        Verdict::FirstFrame,
        Verdict::Partial,
        Verdict::Fail,
        Verdict::Timeout,
        Verdict::Missing,
    ];
}

/// Did `got` cover `expected`? Video tolerates one missing frame
/// (decoders disagree about a trailing non-displayed picture); audio
/// tolerates the codec's priming / padding (4096 samples or 2 %,
/// whichever is larger). Unknown reference = "anything > 0".
fn complete(expected: Option<u64>, got: Option<u64>, is_audio: bool) -> bool {
    let Some(got) = got else { return false };
    match expected {
        None => got > 0,
        Some(0) => true,
        Some(exp) => {
            if is_audio {
                let tol = (exp / 50).max(4096);
                got + tol >= exp
            } else {
                got + 1 >= exp
            }
        }
    }
}

/// The original report's ladder, applied to the best per-track
/// outcome of the player legs.
fn grade(
    status: Status,
    wants_video: bool,
    wants_audio: bool,
    exp_v: Option<u64>,
    exp_a: Option<u64>,
    got_v: Option<u64>,
    got_a: Option<u64>,
) -> Verdict {
    let v_frames = got_v.unwrap_or(0);
    let a_samples = got_a.unwrap_or(0);
    let v_ok = !wants_video || complete(exp_v, got_v, false);
    let a_ok = !wants_audio || complete(exp_a, got_a, true);
    let any_out = v_frames > 0 || a_samples > 0;
    if status == Status::Ok && v_ok && a_ok && (any_out || (!wants_video && !wants_audio)) {
        return Verdict::Pass;
    }
    if !any_out {
        return if status == Status::Timeout {
            Verdict::Timeout
        } else {
            Verdict::Fail
        };
    }
    if wants_video && v_frames > 0 && v_frames <= FIRST_FRAME_MAX && !v_ok {
        return Verdict::FirstFrame;
    }
    Verdict::Partial
}

// ─────────────────────────── per fixture ──────────────────────────

struct Outcome {
    fixture: Fixture,
    size_bytes: u64,
    reference: Reference,
    dry_format: Option<String>,
    dry_video: Option<String>,
    dry_audio: Option<String>,
    /// Combined `--vo hash --ao hash` run.
    both: PlayerLeg,
    /// Isolated legs (only run when the combined run is not a PASS).
    video_only: Option<PlayerLeg>,
    audio_only: Option<PlayerLeg>,
    verdict: Verdict,
    cli: CliLeg,
    cli_verdict: Verdict,
    elapsed: Duration,
}

impl Outcome {
    fn best_video(&self) -> Option<u64> {
        [Some(&self.both), self.video_only.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|l| l.vo_frames)
            .max()
    }
    fn best_audio(&self) -> Option<u64> {
        [Some(&self.both), self.audio_only.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|l| l.ao_samples)
            .max()
    }
    fn vo_hash(&self) -> Option<&str> {
        let best = self.best_video()?;
        [Some(&self.both), self.video_only.as_ref()]
            .into_iter()
            .flatten()
            .find(|l| l.vo_frames == Some(best))
            .and_then(|l| l.vo_hash.as_deref())
    }
    fn ao_hash(&self) -> Option<&str> {
        let best = self.best_audio()?;
        [Some(&self.both), self.audio_only.as_ref()]
            .into_iter()
            .flatten()
            .find(|l| l.ao_samples == Some(best))
            .and_then(|l| l.ao_hash.as_deref())
    }
    fn error(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(e) = &self.both.error {
            parts.push(e.clone());
        }
        if let Some(l) = &self.video_only
            && let Some(e) = &l.error
            && !parts.contains(e)
        {
            parts.push(format!("video: {e}"));
        }
        if let Some(l) = &self.audio_only
            && let Some(e) = &l.error
            && !parts.contains(e)
        {
            parts.push(format!("audio: {e}"));
        }
        (!parts.is_empty()).then(|| parts.join(" | "))
    }
    fn wants_video(&self) -> bool {
        self.reference.video_codec.is_some() || self.both.video_codec.is_some()
    }
    fn wants_audio(&self) -> bool {
        self.reference.audio_codec.is_some() || self.both.audio_codec.is_some()
    }

    fn into_record(self, cfg: &Config) -> Record {
        let leg = &self.both;
        let opt = |v: Option<&str>| v.unwrap_or("").to_owned();
        let num = |v: Option<u64>| v.map(|n| n.to_string()).unwrap_or_default();
        let mut m: BTreeMap<String, String> = BTreeMap::new();
        let mut put = |k: &str, v: String| {
            m.insert(k.to_owned(), v);
        };
        put("id", self.fixture.id.clone());
        put("ext", self.fixture.ext.clone());
        put("title", self.fixture.title.clone());
        put("section", self.fixture.section.clone());
        put("size_bytes", self.size_bytes.to_string());
        put(
            "status",
            leg.status
                .map(Status::as_str)
                .unwrap_or("missing")
                .to_owned(),
        );
        put(
            "exit_code",
            leg.exit_code.map(|c| c.to_string()).unwrap_or_default(),
        );
        put("elapsed_s", format!("{:.3}", self.elapsed.as_secs_f64()));
        put("timeout_s", cfg.timeout.as_secs().to_string());
        put("streams", leg.streams.len().to_string());
        put(
            "audio_codec",
            opt(leg
                .audio_codec
                .as_deref()
                .or(self.reference.audio_codec.as_deref())),
        );
        put(
            "audio_ch",
            leg.audio_ch
                .or(self.reference.audio_ch)
                .map(|c| c.to_string())
                .unwrap_or_default(),
        );
        put(
            "audio_rate",
            leg.audio_rate
                .or(self.reference.audio_rate)
                .map(|c| c.to_string())
                .unwrap_or_default(),
        );
        put(
            "video_codec",
            opt(leg
                .video_codec
                .as_deref()
                .or(self.reference.video_codec.as_deref())),
        );
        put(
            "video_wh",
            opt(leg
                .video_wh
                .as_deref()
                .or(self.reference.video_wh.as_deref())),
        );
        put("vo_frames", num(self.best_video()));
        put("ao_samples", num(self.best_audio()));
        put("vo_hash", opt(self.vo_hash()));
        put("ao_hash", opt(self.ao_hash()));
        put(
            "skip_reason",
            if self.size_bytes == 0 {
                "fixture missing (see generate.log)".to_owned()
            } else {
                String::new()
            },
        );
        put("dry_format", opt(self.dry_format.as_deref()));
        put("dry_video", opt(self.dry_video.as_deref()));
        put("dry_audio", opt(self.dry_audio.as_deref()));
        put("error", self.error().unwrap_or_default());
        put("stderr_tail", leg.stderr_tail.clone());
        put("verdict", self.verdict.as_str().to_owned());
        put("ref_video_frames", num(self.reference.video_frames));
        put("ref_audio_samples", num(self.reference.audio_samples));
        put(
            "data_tracks",
            self.reference
                .kinds
                .iter()
                .filter(|k| *k != "video" && *k != "audio")
                .cloned()
                .collect::<Vec<_>>()
                .join("+"),
        );
        put("cli_video_frames", num(self.cli.video_frames));
        put("cli_audio_samples", num(self.cli.audio_samples));
        put("cli_video_md5", opt(self.cli.video_md5.as_deref()));
        put("cli_audio_md5", opt(self.cli.audio_md5.as_deref()));
        put("cli_error", self.cli.error.clone().unwrap_or_default());
        put("cli_verdict", self.cli_verdict.as_str().to_owned());
        put("ffmpeg_args", self.fixture.ffmpeg_args.clone());
        Record(m)
    }
}

fn run_fixture(cfg: &Config, f: &Fixture, scratch: &Path) -> Outcome {
    let started = Instant::now();
    let file = cfg.dir.join(format!("{}{}", f.id, f.ext));
    let size_bytes = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    let missing = size_bytes == 0;
    let reference = match (&cfg.ffprobe, missing) {
        (Some(p), false) => probe_reference(p, &file, cfg.timeout),
        _ => Reference::default(),
    };
    let mut o = Outcome {
        fixture: f.clone(),
        size_bytes,
        reference,
        dry_format: None,
        dry_video: None,
        dry_audio: None,
        both: PlayerLeg::default(),
        video_only: None,
        audio_only: None,
        verdict: Verdict::Missing,
        cli: CliLeg::default(),
        cli_verdict: Verdict::Missing,
        elapsed: Duration::ZERO,
    };
    if missing {
        o.elapsed = started.elapsed();
        return o;
    }

    // Probe-only pass: what the player thinks the file is.
    let dry = run_bounded(
        &cfg.oxideplay,
        &["--dry-run".to_owned(), file.to_string_lossy().into_owned()],
        cfg.timeout,
    );
    for line in dry.stdout.lines() {
        if let Some(v) = line.strip_prefix("Format: ") {
            o.dry_format = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("Video: ") {
            if o.dry_video.is_none() {
                // `stream #0 codec=h264 320x180`
                let codec = v
                    .split("codec=")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next());
                let wh = v.split_whitespace().last();
                o.dry_video = Some(format!("{} {}", codec.unwrap_or("?"), wh.unwrap_or("")));
            }
        } else if let Some(v) = line.strip_prefix("Audio: ")
            && o.dry_audio.is_none()
        {
            let get = |k: &str| {
                v.split(k)
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .unwrap_or("?")
            };
            o.dry_audio = Some(format!(
                "{} {}ch @{}",
                get("codec="),
                get("channels="),
                get("rate=")
            ));
        }
    }

    // Default mapping, both sinks.
    o.both = play(cfg, &file, "hash", "hash");
    let wants_video = o.wants_video();
    let wants_audio = o.wants_audio();
    let exp_v = o.reference.video_frames;
    let exp_a = o.reference.audio_samples;
    let mut verdict = grade(
        o.both.status.unwrap_or(Status::Error),
        wants_video,
        wants_audio,
        exp_v,
        exp_a,
        o.both.vo_frames,
        o.both.ao_samples,
    );
    if verdict != Verdict::Pass && (wants_video || wants_audio) {
        // Attribute: one track at a time.
        if wants_video {
            o.video_only = Some(play(cfg, &file, "hash", "none"));
        }
        if wants_audio {
            o.audio_only = Some(play(cfg, &file, "none", "hash"));
        }
        let timed_out = o.both.status == Some(Status::Timeout)
            || o.video_only
                .as_ref()
                .is_some_and(|l| l.status == Some(Status::Timeout))
            || o.audio_only
                .as_ref()
                .is_some_and(|l| l.status == Some(Status::Timeout));
        let attributed = grade(
            if timed_out {
                Status::Timeout
            } else {
                Status::Error
            },
            wants_video,
            wants_audio,
            exp_v,
            exp_a,
            o.best_video(),
            o.best_audio(),
        );
        // The split legs can only refine the verdict (FAIL / TIMEOUT
        // → FIRST_FRAME / PARTIAL); the default mapping did not play,
        // so they never promote it to PASS.
        verdict = verdict.min(attributed).max(Verdict::FirstFrame);
    }
    o.verdict = verdict;

    // CLI decode-to-raw leg.
    o.cli = cli_decode(cfg, f, &file, &o.reference, scratch);
    let cli_status = if o
        .cli
        .error
        .as_deref()
        .is_some_and(|e| e.contains("timeout"))
    {
        Status::Timeout
    } else if o.cli.error.is_some() {
        Status::Error
    } else {
        Status::Ok
    };
    o.cli_verdict = grade(
        cli_status,
        o.reference.video_codec.is_some(),
        o.reference.audio_codec.is_some(),
        exp_v,
        exp_a,
        o.cli.video_frames,
        o.cli.audio_samples,
    );
    o.elapsed = started.elapsed();
    o
}

// ───────────────────────────── records ────────────────────────────

/// One `results.csv` row — the unit the report is rendered from, and
/// what an incremental run keeps for fixtures it did not re-run.
#[derive(Clone, Debug)]
struct Record(BTreeMap<String, String>);

impl Record {
    fn get(&self, k: &str) -> &str {
        self.0.get(k).map(String::as_str).unwrap_or("")
    }
    fn opt(&self, k: &str) -> Option<&str> {
        Some(self.get(k)).filter(|s| !s.is_empty())
    }
    fn u64(&self, k: &str) -> Option<u64> {
        self.get(k).parse().ok()
    }
    fn f64(&self, k: &str) -> f64 {
        self.get(k).parse().unwrap_or(0.0)
    }
    fn id(&self) -> &str {
        self.get("id")
    }
    fn verdict(&self) -> Verdict {
        Verdict::parse(self.get("verdict"))
    }
    fn cli_verdict(&self) -> Verdict {
        Verdict::parse(self.get("cli_verdict"))
    }
}

const CSV_HEADER: &[&str] = &[
    "id",
    "ext",
    "title",
    "section",
    "size_bytes",
    "status",
    "exit_code",
    "elapsed_s",
    "timeout_s",
    "streams",
    "audio_codec",
    "audio_ch",
    "audio_rate",
    "video_codec",
    "video_wh",
    "vo_frames",
    "ao_samples",
    "vo_hash",
    "ao_hash",
    "skip_reason",
    "dry_format",
    "dry_video",
    "dry_audio",
    "error",
    "stderr_tail",
    "verdict",
    "ref_video_frames",
    "ref_audio_samples",
    "data_tracks",
    "cli_video_frames",
    "cli_audio_samples",
    "cli_video_md5",
    "cli_audio_md5",
    "cli_error",
    "cli_verdict",
    "ffmpeg_args",
];

fn write_csv(path: &Path, records: &[Record]) -> std::io::Result<()> {
    let mut w = fs::File::create(path)?;
    writeln!(w, "{}", CSV_HEADER.join(","))?;
    for r in records {
        let line: Vec<String> = CSV_HEADER.iter().map(|k| csv_quote(r.get(k))).collect();
        writeln!(w, "{}", line.join(","))?;
    }
    Ok(())
}

fn read_records(path: &Path) -> Option<Vec<Record>> {
    let text = fs::read_to_string(path).ok()?;
    let mut rows = parse_csv(&text).into_iter();
    let header = rows.next()?;
    Some(
        rows.filter(|r| r.iter().any(|c| !c.is_empty()))
            .map(|r| {
                Record(
                    header
                        .iter()
                        .cloned()
                        .zip(r.into_iter().chain(std::iter::repeat(String::new())))
                        .collect(),
                )
            })
            .collect(),
    )
}

// ───────────────────────────── report ─────────────────────────────

fn pct(n: usize, total: usize) -> String {
    if total == 0 {
        "0.0%".to_owned()
    } else {
        format!("{:.1}%", 100.0 * n as f64 / total as f64)
    }
}

fn tally<'a>(
    records: impl Iterator<Item = &'a Record>,
    pick: fn(&Record) -> Verdict,
) -> BTreeMap<Verdict, usize> {
    let mut m = BTreeMap::new();
    for r in records {
        *m.entry(pick(r)).or_insert(0) += 1;
    }
    m
}

fn verdict_row(label: &str, rs: &[&Record], cli_pass: usize, with_rate: bool) -> String {
    let c = tally(rs.iter().copied(), Record::verdict);
    let g = |v: Verdict| c.get(&v).copied().unwrap_or(0);
    let mut s = format!(
        "| {label} | {} | {} | {} | {} | {} | {} | {} |",
        g(Verdict::Pass),
        g(Verdict::FirstFrame),
        g(Verdict::Partial),
        g(Verdict::Fail),
        g(Verdict::Timeout),
        g(Verdict::Missing),
        rs.len()
    );
    if with_rate {
        s.push_str(&format!(" {} |", pct(g(Verdict::Pass), rs.len())));
    }
    s.push_str(&format!(" {cli_pass} |\n"));
    s
}

fn render_report(cfg: &Config, records: &[Record], wall: Duration, ran: usize) -> String {
    let mut s = String::new();
    let total = records.len();
    let counts = tally(records.iter(), Record::verdict);
    let cli_counts = tally(records.iter(), Record::cli_verdict);
    let n = |v: Verdict| counts.get(&v).copied().unwrap_or(0);
    let cn = |v: Verdict| cli_counts.get(&v).copied().unwrap_or(0);
    let name = |p: &Path| {
        p.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    };

    s.push_str("# oxideplay / oxideav vs the playback corpus\n\n");
    s.push_str(&format!(
        "Player: `{}`  \nCLI: `{}`  \nCorpus: `{}` ({} fixtures from `corpus/manifest.csv`, {} missing on this host)  \nTimeout: {} s per process; {} fixture{} run in {:.0} s; host {} {}\n\n",
        name(&cfg.oxideplay),
        name(&cfg.oxideav),
        name(&cfg.dir),
        total,
        n(Verdict::Missing),
        cfg.timeout.as_secs(),
        ran,
        if ran == 1 { "" } else { "s" },
        wall.as_secs_f64(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    ));

    s.push_str("## Headline\n\n");
    let present = total - n(Verdict::Missing);
    s.push_str(&format!(
        "**{} / {} generated fixtures fully play in `oxideplay` ({}).** {} emit only a first frame, {} play one track, {} fail at open / decode, {} hang until killed; {} fixture{} could not be generated on this host.  \n`oxideav transcode` (decode to y4m / wav): **{} / {} PASS ({})**.\n\n",
        n(Verdict::Pass),
        present,
        pct(n(Verdict::Pass), present),
        n(Verdict::FirstFrame),
        n(Verdict::Partial),
        n(Verdict::Fail),
        n(Verdict::Timeout),
        n(Verdict::Missing),
        if n(Verdict::Missing) == 1 { "" } else { "s" },
        cn(Verdict::Pass),
        present,
        pct(cn(Verdict::Pass), present),
    ));

    s.push_str("Method: `oxideplay --dry-run`, then `oxideplay --vo hash --ao hash` (the default stream mapping, headless); a run that is not PASS is re-run per track (`--vo hash --ao none`, `--vo none --ao hash`) to attribute the failure. `oxideav transcode` decodes to `.y4m` / `.wav`. Reference counts: `ffprobe -count_frames` (video) and summed `nb_samples` (audio).\n\n");
    s.push_str("- **PASS** — every advertised A/V track produced all of its frames / samples and the process reached end of stream.\n- **FIRST_FRAME** — the video decoder produced 1–10 frames (typically the IDR only).\n- **PARTIAL** — one track decoded fully, the other did not (or the default mapping failed while the isolated legs played).\n- **FAIL** — open / probe / decode error with 0 frames and 0 samples.\n- **TIMEOUT** — process killed; no usable output.\n- **MISSING** — the fixture could not be generated (this ffmpeg build lacks the encoder).\n\n");

    s.push_str("## Overview\n\n| Verdict | oxideplay | Rate | oxideav transcode | Rate |\n|---------|------:|-----:|------:|-----:|\n");
    for v in Verdict::ALL {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            v.as_str(),
            n(v),
            pct(n(v), total),
            cn(v),
            pct(cn(v), total)
        ));
    }
    s.push_str(&format!(
        "| **Total** | **{total}** | | **{total}** | |\n\n"
    ));

    s.push_str("## By container\n\n| Ext | Pass | First-frame | Partial | Fail | Timeout | Missing | Total | CLI pass |\n|-----|-----:|------------:|--------:|-----:|--------:|--------:|------:|---------:|\n");
    let mut by_ext: BTreeMap<&str, Vec<&Record>> = BTreeMap::new();
    for r in records {
        by_ext.entry(r.get("ext")).or_default().push(r);
    }
    for (ext, rs) in &by_ext {
        let cli_pass = rs
            .iter()
            .filter(|r| r.cli_verdict() == Verdict::Pass)
            .count();
        s.push_str(&verdict_row(ext, rs, cli_pass, false));
    }
    s.push('\n');

    s.push_str("## By section\n\n| Section | Pass | First-frame | Partial | Fail | Timeout | Missing | Total | Pass rate | CLI pass |\n|---------|-----:|------------:|--------:|-----:|--------:|--------:|------:|----------:|---------:|\n");
    let mut sections: Vec<&str> = Vec::new();
    for r in records {
        if !sections.contains(&r.get("section")) {
            sections.push(r.get("section"));
        }
    }
    for sec in sections {
        let rs: Vec<&Record> = records.iter().filter(|r| r.get("section") == sec).collect();
        let cli_pass = rs
            .iter()
            .filter(|r| r.cli_verdict() == Verdict::Pass)
            .count();
        s.push_str(&verdict_row(sec, &rs, cli_pass, true));
    }
    s.push('\n');

    s.push_str("## Track-level codec scoreboard\n\nVideo (fixtures advertising the codec → any frame → all frames):\n\n| Codec | Files | Any frame | All frames |\n|-------|------:|----------:|-----------:|\n");
    let mut vcodecs: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut acodecs: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for r in records {
        if r.verdict() == Verdict::Missing {
            continue;
        }
        if let Some(c) = r.opt("video_codec") {
            let e = vcodecs.entry(c.to_owned()).or_default();
            e.0 += 1;
            let got = r.u64("vo_frames");
            if got.unwrap_or(0) > 0 {
                e.1 += 1;
            }
            if complete(r.u64("ref_video_frames"), got, false) {
                e.2 += 1;
            }
        }
        if let Some(c) = r.opt("audio_codec") {
            let e = acodecs.entry(c.to_owned()).or_default();
            e.0 += 1;
            let got = r.u64("ao_samples");
            if got.unwrap_or(0) > 0 {
                e.1 += 1;
            }
            if complete(r.u64("ref_audio_samples"), got, true) {
                e.2 += 1;
            }
        }
    }
    for (c, (files, any, all)) in &vcodecs {
        s.push_str(&format!("| {c} | {files} | {any} | {all} |\n"));
    }
    s.push_str("\nAudio:\n\n| Codec | Files | Any samples | All samples |\n|-------|------:|------------:|------------:|\n");
    for (c, (files, any, all)) in &acodecs {
        s.push_str(&format!("| {c} | {files} | {any} | {all} |\n"));
    }
    s.push('\n');

    s.push_str("## Failures / partials / timeouts (oxideplay)\n\n| Id | Ext | Title | Verdict | Video | Audio | Time | Notes |\n|----|-----|-------|---------|-------|-------|-----:|-------|\n");
    for r in records.iter().filter(|r| r.verdict() != Verdict::Pass) {
        let video = match r.opt("video_codec") {
            Some(c) => format!(
                "{c} {} frames={}/{}",
                r.opt("video_wh").unwrap_or("?"),
                r.opt("vo_frames").unwrap_or("None"),
                r.opt("ref_video_frames").unwrap_or("?")
            ),
            None => "—".to_owned(),
        };
        let audio = match r.opt("audio_codec") {
            Some(c) => format!(
                "{c} {}ch samples={}/{}",
                r.opt("audio_ch").unwrap_or("0"),
                r.opt("ao_samples").unwrap_or("None"),
                r.opt("ref_audio_samples").unwrap_or("?")
            ),
            None => "—".to_owned(),
        };
        let mut notes = r.get("error").to_owned();
        if let Some(d) = r.opt("data_tracks") {
            notes = format!("[{d} track] {notes}");
        }
        if r.verdict() == Verdict::Missing {
            notes = "fixture not generated (see generate.log)".to_owned();
        }
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.1}s | {} |\n",
            r.id(),
            r.get("ext"),
            md_cell(r.get("title")),
            r.verdict().as_str(),
            video,
            audio,
            r.f64("elapsed_s"),
            md_cell(&notes)
        ));
    }
    s.push('\n');

    s.push_str("## CLI failures (`oxideav transcode` to y4m / wav)\n\n| Id | Ext | Title | CLI verdict | Frames | Samples | Error |\n|----|-----|-------|-------------|-------:|--------:|-------|\n");
    for r in records
        .iter()
        .filter(|r| r.cli_verdict() != Verdict::Pass && r.verdict() != Verdict::Missing)
    {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {}/{} | {}/{} | {} |\n",
            r.id(),
            r.get("ext"),
            md_cell(r.get("title")),
            r.cli_verdict().as_str(),
            r.opt("cli_video_frames").unwrap_or("—"),
            r.opt("ref_video_frames").unwrap_or("—"),
            r.opt("cli_audio_samples").unwrap_or("—"),
            r.opt("ref_audio_samples").unwrap_or("—"),
            md_cell(r.get("cli_error"))
        ));
    }
    s.push('\n');

    s.push_str("## All fixtures\n\n| Id | Ext | Title | Verdict | Video frames | Audio samples | CLI | Time |\n|----|-----|-------|---------|-------------:|--------------:|-----|-----:|\n");
    for r in records {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {:.1}s |\n",
            r.id(),
            r.get("ext"),
            md_cell(r.get("title")),
            r.verdict().as_str(),
            r.get("vo_frames"),
            r.get("ao_samples"),
            r.cli_verdict().as_str(),
            r.f64("elapsed_s")
        ));
    }
    s
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

// ────────────────────────────── md5 ───────────────────────────────

/// MD5 (RFC 1321) — only used to fingerprint the CLI's raw output so
/// two runs can be diffed; no dependency needed for that.
fn md5_hex(data: &[u8]) -> String {
    let s: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let (mut a0, mut b0, mut c0, mut d0) = (
        0x6745_2301u32,
        0xefcd_ab89u32,
        0x98ba_dcfeu32,
        0x1032_5476u32,
    );
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(s[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = String::with_capacity(32);
    for v in [a0, b0, c0, d0] {
        for byte in v.to_le_bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
    }
    out
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn md5_known_answers() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn csv_roundtrip() {
        let text = "id,title,args\n001,\"a, \"\"quoted\"\" title\",-f lavfi\n002,plain,x\n";
        let rows = parse_csv(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1][1], "a, \"quoted\" title");
        assert_eq!(csv_quote("a,b"), "\"a,b\"");
        assert_eq!(csv_quote("plain"), "plain");
    }

    #[test]
    fn y4m_and_wav_counts() {
        let mut y4m = b"YUV4MPEG2 W4 H2 F25:1 Ip A0:0 C420mpeg2\n".to_vec();
        for _ in 0..3 {
            y4m.extend_from_slice(b"FRAME\n");
            // 4x2 luma + two 2x1 chroma planes.
            y4m.extend_from_slice(&[0u8; 8 + 2 * 2]);
        }
        assert_eq!(y4m_frame_count(&y4m), Some(3));

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // pcm
        wav.extend_from_slice(&2u16.to_le_bytes()); // channels
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&(48000u32 * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&40u32.to_le_bytes());
        wav.extend_from_slice(&[0u8; 40]);
        assert_eq!(wav_sample_count(&wav), Some(10));
    }

    #[test]
    fn verdict_ladder() {
        let g = grade;
        assert_eq!(
            g(
                Status::Ok,
                true,
                true,
                Some(50),
                Some(96000),
                Some(50),
                Some(97280)
            ),
            Verdict::Pass
        );
        assert_eq!(
            g(Status::Ok, true, true, Some(50), Some(96000), Some(3), None),
            Verdict::FirstFrame
        );
        assert_eq!(
            g(
                Status::Ok,
                true,
                true,
                Some(50),
                Some(96000),
                None,
                Some(96000)
            ),
            Verdict::Partial
        );
        assert_eq!(
            g(
                Status::Timeout,
                true,
                true,
                Some(50),
                Some(96000),
                None,
                None
            ),
            Verdict::Timeout
        );
        assert_eq!(
            g(Status::Error, true, false, Some(50), None, None, None),
            Verdict::Fail
        );
        assert_eq!(
            g(
                Status::Ok,
                true,
                true,
                Some(50),
                Some(96000),
                Some(30),
                Some(96000)
            ),
            Verdict::Partial
        );
        assert_eq!(
            g(
                Status::Ok,
                false,
                true,
                None,
                Some(96000),
                None,
                Some(96000)
            ),
            Verdict::Pass
        );
    }

    #[test]
    fn records_roundtrip_through_csv() {
        let mut m = BTreeMap::new();
        m.insert("id".to_owned(), "001".to_owned());
        m.insert("title".to_owned(), "a, \"b\"".to_owned());
        m.insert("verdict".to_owned(), "PASS".to_owned());
        let rec = Record(m);
        let dir = std::env::temp_dir().join(format!("oxideav-corpus-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("results.csv");
        write_csv(&path, std::slice::from_ref(&rec)).unwrap();
        let back = read_records(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id(), "001");
        assert_eq!(back[0].get("title"), "a, \"b\"");
        assert_eq!(back[0].verdict(), Verdict::Pass);
        let _ = fs::remove_dir_all(&dir);
    }
}
