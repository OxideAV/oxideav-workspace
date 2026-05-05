//! Integration test for the `--no-hwaccel` global flag.
//!
//! Runs the actual `oxideav` binary (via cargo's `CARGO_BIN_EXE_oxideav`
//! handoff) and confirms that `--no-hwaccel` parses globally (before
//! the subcommand) and the resulting `list` output never contains the
//! `[HW]` marker that `cmd_list` appends to hardware-accelerated
//! implementations.
//!
//! The runtime opt-out itself is exercised in
//! `oxideav-core::registry::slice` (filter callback semantics) and in
//! `oxideav-format-all/tests/walks_every_sibling.rs` (every sibling
//! crate's registrar appears in the slice). This test is the
//! oxideav-cli end of the chain — it confirms the clap wiring routes
//! the flag through to `with_all_features_filtered` rather than
//! silently no-op'ing.

use std::process::Command;

fn run_list(extra_args: &[&str]) -> String {
    let bin = env!("CARGO_BIN_EXE_oxideav");
    let output = Command::new(bin)
        .args(extra_args)
        .arg("list")
        .output()
        .expect("spawn oxideav binary");
    assert!(
        output.status.success(),
        "oxideav {extra_args:?} list failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("non-utf8 stdout from `oxideav list`")
}

/// `--no-hwaccel` must parse globally (before the subcommand) and the
/// resulting `list` output must not contain any `[HW]` markers,
/// regardless of host OS. On non-macOS this is trivially true (no
/// hardware backends register at all); on macOS this exercises the
/// filter passed to `oxideav::with_all_features_filtered`.
#[test]
fn no_hwaccel_strips_hw_tag_from_list() {
    let listing = run_list(&["--no-hwaccel"]);
    assert!(
        !listing.contains("[HW]"),
        "expected no `[HW]` tag in `oxideav --no-hwaccel list` output, got:\n{listing}"
    );
}

/// `--help` must mention `--no-hwaccel` so the flag is discoverable.
/// Catches accidental visibility regressions (e.g. someone adds
/// `hide = true` to the `#[arg]` attribute).
#[test]
fn help_mentions_no_hwaccel() {
    let bin = env!("CARGO_BIN_EXE_oxideav");
    let output = Command::new(bin)
        .arg("--help")
        .output()
        .expect("spawn oxideav binary");
    assert!(
        output.status.success(),
        "oxideav --help failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    let help = String::from_utf8(output.stdout).expect("non-utf8 stdout");
    assert!(
        help.contains("--no-hwaccel"),
        "expected `--no-hwaccel` in `oxideav --help` output, got:\n{help}"
    );
}
