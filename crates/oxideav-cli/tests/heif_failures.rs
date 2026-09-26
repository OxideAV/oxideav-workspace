//! Clean failure paths of the built `oxideav` binary on HEIF work
//! (round 462): typed exit codes, a stderr line per failure, and no
//! stray output file — a failed encode or an unknown extension must
//! not leave a 0-byte file behind, and a failed transcode must not
//! clobber a pre-existing output.
//!
//! Exit-code table (also in `README.md`):
//! 2 usage · 3 invalid data · 4 unsupported · 5 format not found ·
//! 6 codec not found · 7 resource exhausted · 8 input not found ·
//! 9 permission denied · 10 other I/O · 1 anything else.

mod common;

use common::*;

const TAG: &str = "failures";

fn heic() -> String {
    fixtures()
        .join("sips_rgb_96x80.heic")
        .to_str()
        .unwrap()
        .to_owned()
}

fn png() -> String {
    fixtures()
        .join("sips_rgb_96x80.expected.png")
        .to_str()
        .unwrap()
        .to_owned()
}

fn assert_failed(r: &Run, code: i32, needle: &str) {
    assert_eq!(
        r.code,
        Some(code),
        "exit code (stderr: {})",
        r.stderr.trim()
    );
    assert_diagnostic(r);
    assert!(
        diagnostic(r).contains(needle),
        "diagnostic {:?} lacks {needle:?}",
        diagnostic(r)
    );
}

/// The `oxideav: <error>` line. Registration notes may precede it
/// (Linux prints `oxideav-nvidia: library unavailable, skipping
/// registration: …` when no CUDA runtime is present), so the
/// diagnostic is the LAST non-empty stderr line, not the first.
fn diagnostic(r: &Run) -> &str {
    r.stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
}

fn assert_diagnostic(r: &Run) {
    assert!(
        diagnostic(r).starts_with("oxideav: "),
        "last stderr line must be the `oxideav: ` diagnostic: {:?}",
        r.stderr
    );
}

/// Part files (`<name>.<pid>.oxideav-part`) in the scratch dir whose
/// name starts with `prefix`.
fn part_files(prefix: &str) -> Vec<String> {
    std::fs::read_dir(scratch(TAG))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".oxideav-part"))
        .collect()
}

// ─────────────────────── unknown extension ───────────────────────

#[test]
fn unknown_output_extension_exits_5_and_leaves_no_file() {
    let out = scratch_file(TAG, "unknown.xyz");
    let r = oxideav(&["convert", &heic(), out.to_str().unwrap()]);
    assert_failed(&r, 5, "format not found");
    assert!(!out.exists(), "a 0-byte {} was left behind", out.display());

    let out = scratch_file(TAG, "unknown_t.xyz");
    let r = oxideav(&["transcode", &heic(), out.to_str().unwrap()]);
    assert_failed(&r, 5, "format not found");
    assert!(!out.exists(), "transcode left {}", out.display());
}

// ─────────────────── unsupported compressor / option ───────────────────

#[test]
fn unknown_codec_exits_6_and_leaves_no_file() {
    let out = scratch_file(TAG, "nocodec.heic");
    let r = oxideav(&[
        "transcode",
        &heic(),
        out.to_str().unwrap(),
        "--codec-video",
        "no_such_codec",
    ]);
    assert_failed(&r, 6, "codec not found");
    assert!(!out.exists(), "transcode left {}", out.display());
}

/// A codec that cannot take the decoded picture is `unsupported` (4);
/// the output never materialises.
#[test]
fn unsupported_compressor_exits_4_and_leaves_no_file() {
    let out = scratch_file(TAG, "unsupported.heic");
    // An audio codec can never encode a video frame — whichever way the
    // codec crate phrases it, the CLI classifies it as unsupported /
    // codec-not-found, never as success or a panic.
    let r = oxideav(&[
        "transcode",
        &heic(),
        out.to_str().unwrap(),
        "--codec-video",
        "flac",
    ]);
    assert!(
        matches!(r.code, Some(3 | 4 | 6)),
        "exit {:?}: {}",
        r.code,
        r.stderr
    );
    assert_diagnostic(&r);
    assert!(!out.exists(), "transcode left {}", out.display());
}

/// Encoder options are validated by the encoder before any byte is
/// written: unknown keys, out-of-range enums and a malformed layout
/// are refused (3 = invalid data, 4 = unsupported, per the codec's
/// own classification) and name the option.
#[test]
fn refused_encoder_options_exit_typed_and_leave_no_file() {
    for (opt, needle, codes) in [
        ("nosuch=1", "nosuch", &[3][..]),
        ("codec=vp9", "codec", &[3][..]),
        ("mode=xyz", "mode", &[3][..]),
        ("tiles=abc", "tiles", &[3, 4][..]),
    ] {
        let out = scratch_file(TAG, &format!("refused_{}.heic", opt.replace('=', "_")));
        let r = oxideav(&[
            "transcode",
            &png(),
            out.to_str().unwrap(),
            "--codec-video",
            "heif",
            "--codec-option",
            opt,
        ]);
        assert!(
            r.code.is_some_and(|c| codes.contains(&c)),
            "{opt}: exit {:?} not in {codes:?}: {}",
            r.code,
            r.stderr
        );
        assert!(r.stderr.contains(needle), "{opt}: {:?}", r.stderr);
        assert!(!out.exists(), "{opt}: transcode left {}", out.display());
    }
    // The CLI's own shape check (KEY=VALUE) is invalid data.
    let out = scratch_file(TAG, "shape.heic");
    let r = oxideav(&[
        "transcode",
        &png(),
        out.to_str().unwrap(),
        "--codec-video",
        "heif",
        "-o",
        "qp",
    ]);
    assert_failed(&r, 3, "KEY=VALUE");
    assert!(!out.exists());
}

/// A pre-existing output survives a failed transcode untouched (the
/// part file is unlinked; the rename never happens).
#[test]
fn failed_transcode_keeps_the_previous_output_intact() {
    let out = scratch_file(TAG, "keep.heic");
    std::fs::write(&out, b"previous contents").unwrap();
    let r = oxideav(&[
        "transcode",
        &png(),
        out.to_str().unwrap(),
        "--codec-video",
        "heif",
        "-o",
        "codec=vp9",
    ]);
    assert_eq!(r.code, Some(3), "{}", r.stderr);
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"previous contents",
        "the previous output was clobbered"
    );
    // No part file of THIS output lingers next to it (other tests of
    // this binary write into the same directory concurrently, so only
    // `keep.heic.*` is ours to judge).
    assert!(
        part_files("keep.heic.").is_empty(),
        "part files left: {:?}",
        part_files("keep.heic.")
    );
}

// ───────────────────────── truncated input ─────────────────────────

#[test]
fn truncated_input_exits_3_and_leaves_no_file() {
    let bytes = std::fs::read(fixtures().join("sips_rgb_96x80.heic")).unwrap();
    let trunc = scratch_file(TAG, "truncated.heic");
    std::fs::write(&trunc, &bytes[..300]).unwrap();
    let out = scratch_file(TAG, "truncated.png");
    let r = oxideav(&["convert", trunc.to_str().unwrap(), out.to_str().unwrap()]);
    assert_failed(&r, 3, "invalid data");
    assert!(!out.exists(), "convert left {}", out.display());
    let r = oxideav(&["probe", trunc.to_str().unwrap()]);
    assert_failed(&r, 3, "invalid data");

    // Truncated inside the coded payload (headers intact).
    let trunc = scratch_file(TAG, "truncated_mdat.heic");
    std::fs::write(&trunc, &bytes[..bytes.len() - 64]).unwrap();
    let out = scratch_file(TAG, "truncated_mdat.png");
    let r = oxideav(&["convert", trunc.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(
        matches!(r.code, Some(3 | 4)),
        "exit {:?}: {}",
        r.code,
        r.stderr
    );
    assert!(!out.exists(), "convert left {}", out.display());
}

#[test]
fn missing_input_exits_8() {
    let out = scratch_file(TAG, "missing.png");
    let r = oxideav(&["convert", "/no/such/dir/photo.heic", out.to_str().unwrap()]);
    assert_failed(&r, 8, "I/O error");
    assert!(!out.exists());
    let r = oxideav(&["probe", "/no/such/dir/photo.heic"]);
    assert_eq!(r.code, Some(8), "{}", r.stderr);
}

// ───────────────────────── permission denied ─────────────────────────

#[cfg(unix)]
#[test]
fn permission_denied_on_output_exits_9() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch(TAG).join("readonly");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    // Root ignores directory modes — nothing to test then.
    if std::fs::write(dir.join("probe"), b"x").is_ok() {
        let _ = std::fs::remove_file(dir.join("probe"));
        eprintln!("SKIP permission_denied_on_output_exits_9: directory modes are not enforced");
        return;
    }
    let input = heic();
    for (cmd, extra) in [
        ("convert", vec![]),
        ("transcode", vec!["--codec-video", "heif"]),
        ("remux", vec![]),
    ] {
        let out = dir.join(format!("{cmd}.heic"));
        let mut args = vec![cmd, input.as_str(), out.to_str().unwrap()];
        args.extend(extra);
        let r = oxideav(&args);
        assert_failed(&r, 9, "I/O error");
        assert!(!out.exists());
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
#[test]
fn permission_denied_on_output_exits_9() {
    eprintln!("SKIP permission_denied_on_output_exits_9: unix directory modes only");
}

// ───────────────────────── run jobs / usage ─────────────────────────

#[test]
fn failed_run_job_leaves_no_output() {
    let out = scratch_file(TAG, "job.heic");
    let job = format!(
        r#"{{"{}": {{"video": [{{"from": "{}", "codec": "no_such_codec"}}]}}}}"#,
        out.to_str().unwrap().replace('\\', "/"),
        png().replace('\\', "/")
    );
    let r = oxideav(&["run", "--inline", &job]);
    assert!(
        matches!(r.code, Some(3 | 6)),
        "exit {:?}: {}",
        r.code,
        r.stderr
    );
    assert!(!out.exists(), "run left {}", out.display());
}

#[test]
fn usage_errors_exit_2() {
    let r = oxideav(&["transcode"]);
    assert_eq!(r.code, Some(2), "{}", r.stderr);
    let r = oxideav(&["no-such-command"]);
    assert_eq!(r.code, Some(2), "{}", r.stderr);
}

/// The happy path still exits 0 and writes exactly the named file.
#[test]
fn successful_transcode_exits_0_with_only_the_named_output() {
    let out = scratch_file(TAG, "ok.heic");
    let r = oxideav(&[
        "transcode",
        &png(),
        out.to_str().unwrap(),
        "--codec-video",
        "heif",
        "-o",
        "qp=30",
    ]);
    assert_eq!(r.code, Some(0), "{}", r.stderr);
    assert!(out.is_file());
    assert!(std::fs::metadata(&out).unwrap().len() > 0);
    assert!(
        part_files("ok.heic.").is_empty(),
        "part files left: {:?}",
        part_files("ok.heic.")
    );
}
