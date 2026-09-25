//! Hostile inputs through the built `oxideav` binary (round 462).
//!
//! Every file of the heif crate's fuzz corpus
//! (`crates/oxideav-heif/fuzz/corpus/*`, read-only) plus deterministic
//! truncations and single-byte flips of every vendored interop file is
//! driven through `oxideav probe` and `oxideav convert … out.png`.
//! Each run has a wall-clock budget; the verdict must always be a
//! typed exit status — never a panic (101), an abort / signal, a
//! timeout, or an untyped `1`.
//!
//! Four worker threads share the queue; the summary (per-code tally,
//! slowest run) prints under `--nocapture`.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use common::*;

const TAG: &str = "hostile";
/// Per-run budget; a debug build decodes the largest corpus file in
/// well under a second, so anything near this is a hang.
const BUDGET: Duration = Duration::from_secs(60);
const WORKERS: usize = 4;

/// Exit statuses the CLI documents for a failed command (plus success).
fn is_typed(code: i32) -> bool {
    matches!(code, 0 | 3..=10)
}

fn corpus_files() -> Vec<PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.is_file() {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(&fuzz_corpus_root(), &mut v);
    v.sort();
    v
}

/// Truncations at nine cut points and eight single-byte flips per
/// interop file, written under the scratch dir with the source's
/// extension kept (so the extension hint stays hostile too).
fn mutants() -> Vec<PathBuf> {
    let dir = scratch(TAG).join("mutants");
    std::fs::create_dir_all(&dir).expect("mutant dir");
    let mut out = Vec::new();
    for src in interop_files() {
        let bytes = std::fs::read(&src).expect("read interop file");
        let stem = src.file_stem().unwrap().to_string_lossy().into_owned();
        let ext = src.extension().unwrap().to_string_lossy().into_owned();
        for tenth in 1..10 {
            let cut = bytes.len() * tenth / 10;
            let p = dir.join(format!("{stem}.trunc{tenth}.{ext}"));
            std::fs::write(&p, &bytes[..cut]).expect("write truncation");
            out.push(p);
        }
        // xorshift32 seeded from the name: reproducible flip positions.
        let mut s: u32 = bytes
            .len()
            .wrapping_mul(2_654_435_761)
            .wrapping_add(stem.len()) as u32
            | 1;
        for i in 0..8 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let pos = (s as usize) % bytes.len();
            let bit = 1u8 << (i % 8);
            let mut m = bytes.clone();
            m[pos] ^= bit;
            let p = dir.join(format!("{stem}.flip{i}.{ext}"));
            std::fs::write(&p, &m).expect("write flip");
            out.push(p);
        }
    }
    out
}

struct Verdict {
    file: PathBuf,
    cmd: &'static str,
    outcome: Outcome,
    elapsed: Duration,
    stderr: String,
}

fn drive(files: &[PathBuf]) -> Vec<Verdict> {
    let outdir = scratch(TAG).join("out");
    std::fs::create_dir_all(&outdir).expect("out dir");
    let next = AtomicUsize::new(0);
    let verdicts = Mutex::new(Vec::with_capacity(files.len() * 2));
    std::thread::scope(|s| {
        for w in 0..WORKERS {
            let next = &next;
            let verdicts = &verdicts;
            let outdir = &outdir;
            s.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(file) = files.get(i) else { break };
                let f = file.to_str().expect("utf8 path");
                let png = outdir.join(format!("w{w}.png"));
                let _ = std::fs::remove_file(&png);
                let probe = oxideav_timed(&["probe", f], BUDGET);
                let convert = oxideav_timed(&["convert", f, png.to_str().unwrap()], BUDGET);
                let _ = std::fs::remove_file(&png);
                let mut v = verdicts.lock().unwrap();
                v.push(Verdict {
                    file: file.clone(),
                    cmd: "probe",
                    outcome: probe.outcome,
                    elapsed: probe.elapsed,
                    stderr: probe.stderr,
                });
                v.push(Verdict {
                    file: file.clone(),
                    cmd: "convert",
                    outcome: convert.outcome,
                    elapsed: convert.elapsed,
                    stderr: convert.stderr,
                });
            });
        }
    });
    verdicts.into_inner().unwrap()
}

fn report(label: &str, verdicts: &[Verdict]) {
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    for v in verdicts {
        let key = match v.outcome {
            Outcome::Exited(c) => format!("{} exit {c}", v.cmd),
            Outcome::Signalled => format!("{} SIGNAL", v.cmd),
            Outcome::TimedOut => format!("{} TIMEOUT", v.cmd),
        };
        *tally.entry(key).or_default() += 1;
    }
    let slowest = verdicts
        .iter()
        .max_by_key(|v| v.elapsed)
        .map(|v| {
            format!(
                "{:.2}s ({} {})",
                v.elapsed.as_secs_f64(),
                v.cmd,
                v.file.file_name().unwrap().to_string_lossy()
            )
        })
        .unwrap_or_default();
    let total: f64 = verdicts.iter().map(|v| v.elapsed.as_secs_f64()).sum();
    eprintln!(
        "{label}: {} runs over {} files, {total:.1}s cpu-wall summed, slowest {slowest}",
        verdicts.len(),
        verdicts.len() / 2
    );
    for (k, n) in &tally {
        eprintln!("    {k:<18} {n}");
    }
    let bad: Vec<String> = verdicts
        .iter()
        .filter(|v| !matches!(v.outcome, Outcome::Exited(c) if is_typed(c)))
        .map(|v| {
            format!(
                "{} {}: {:?} after {:.1}s — {}",
                v.cmd,
                v.file.display(),
                v.outcome,
                v.elapsed.as_secs_f64(),
                v.stderr.lines().last().unwrap_or("").trim()
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{label}: {} untyped / crashed / hung runs:\n{}",
        bad.len(),
        bad.join("\n")
    );
    // Every failure carries the `oxideav: <error>` diagnostic line (the
    // pipeline may print a "decoder skipped packet" note ahead of it).
    let silent: Vec<String> = verdicts
        .iter()
        .filter(|v| matches!(v.outcome, Outcome::Exited(c) if c != 0))
        .filter(|v| !v.stderr.lines().any(|l| l.starts_with("oxideav: ")))
        .map(|v| format!("{} {}: {:?}", v.cmd, v.file.display(), v.stderr))
        .collect();
    assert!(
        silent.is_empty(),
        "{label}: failures without a diagnostic:\n{}",
        silent.join("\n")
    );
}

#[test]
fn fuzz_corpus_never_panics_hangs_or_exits_untyped() {
    let files = corpus_files();
    // Only a handful of seeds are tracked; the full corpus lives on the
    // fuzzing hosts. Whatever is checked out gets driven.
    assert!(!files.is_empty(), "fuzz corpus directory is empty");
    let verdicts = drive(&files);
    report("fuzz corpus", &verdicts);
}

#[test]
fn truncated_and_bit_flipped_interop_files_fail_typed() {
    let files = mutants();
    assert!(files.len() >= 39 * 17, "mutant count {}", files.len());
    let verdicts = drive(&files);
    report("interop mutants", &verdicts);
}
