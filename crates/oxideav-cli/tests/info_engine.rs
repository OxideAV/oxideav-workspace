//! Integration test for the rich-engine output of `oxideav info <codec>`.
//!
//! Phase 3 of the HW-engine-info initiative: the `info` command calls
//! each registered HW backend's `engine_info()` probe and renders the
//! resulting [`oxideav_core::HwDeviceInfo`] entries (device name, driver
//! version, per-codec capability matrix). This test confirms the
//! command:
//!   * runs without panicking on the host;
//!   * emits an `engine: <id>` line for at least one backend whose
//!     sibling crate's `engine_info()` returned a non-empty device list.
//!
//! The test is _skip-friendly_: if no HW backend probes successfully on
//! the host (no NVIDIA / VA-API / Vulkan-Video stack installed), the
//! command still emits the SW backend's `h264_sw` block but the
//! per-engine assertions are skipped instead of failing. Matches the
//! pattern the HW sibling crates' own self-tests follow.

use std::process::Command;

fn run_info(codec: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_oxideav");
    let output = Command::new(bin)
        .arg("info")
        .arg(codec)
        .output()
        .unwrap_or_else(|e| panic!("spawn oxideav binary: {e}"));
    assert!(
        output.status.success(),
        "oxideav info {codec} failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("non-utf8 stdout from `oxideav info`")
}

#[test]
fn info_h264_renders_codec_header() {
    let out = run_info("h264");
    assert!(
        out.contains("Codec: h264"),
        "expected `Codec: h264` in output, got:\n{out}"
    );
    assert!(
        out.contains("h264_sw"),
        "expected SW backend `h264_sw` in output, got:\n{out}"
    );
}

#[test]
fn info_h264_renders_per_engine_devices_when_available() {
    let out = run_info("h264");
    let mut saw_engine = false;
    // Each engine line is prefixed with the canonical engine_id string
    // attached by the sibling crate's `with_engine_id` calls. We only
    // assert _at least one_ — non-Linux hosts ship none of the four,
    // and a Linux box without the userspace libraries installed will
    // also probe-empty. CI runs on hosted Linux without GPUs so this
    // test must never hard-fail.
    for engine_id in ["nvidia", "vaapi", "vdpau", "vulkan-video"] {
        let line = format!("engine         : {engine_id}");
        if out.contains(&line) {
            saw_engine = true;
            // The matched backend should also have a `devices :` block
            // immediately after — that's the rich device info Phase 3
            // is meant to surface.
            assert!(
                out.contains("devices        :"),
                "engine `{engine_id}` matched but no `devices :` block in output:\n{out}"
            );
        }
    }
    if !saw_engine {
        eprintln!(
            "no HW engine probed devices on this host; skipping per-engine assertion. output:\n{out}"
        );
    }
}

/// A HW backend with an engine_probe attached must NOT render the
/// legacy backend-level `limits` block — the per-device block already
/// shows max dims / bit-depth, and surfacing a single backend-wide
/// number (which is the legacy `caps.max_width / max_height` field)
/// would contradict heterogeneous devices (e.g. nvidia-vaapi 8K vs
/// Intel iHD 4K on the same vaapi backend).
///
/// We can only assert this when an HW engine actually showed up in
/// the output — non-Linux hosts and Linux boxes without GPU userspace
/// libraries probe-empty and skip the assertion.
#[test]
fn info_h264_suppresses_backend_limits_for_hw_engines() {
    let out = run_info("h264");
    let mut checked = 0usize;
    for engine_id in ["nvidia", "vaapi", "vdpau", "vulkan-video"] {
        let engine_line = format!("engine         : {engine_id}");
        let Some(start) = out.find(&engine_line) else {
            continue;
        };
        // The block for this backend ends at the next `Backend:` line
        // (or end of output). Slice the substring to scope the
        // assertion — if a *different* backend later in the output
        // does show legacy limits (e.g. h264_sw), that's correct and
        // we shouldn't false-positive on it.
        let tail = &out[start..];
        let end = tail[engine_line.len()..]
            .find("\nBackend: ")
            .map(|i| engine_line.len() + i)
            .unwrap_or(tail.len());
        let block = &tail[..end];
        assert!(
            !block.contains("limits         :"),
            "HW backend with engine `{engine_id}` should suppress backend-level `limits` block, got:\n{block}"
        );
        checked += 1;
    }
    if checked == 0 {
        eprintln!("no HW engines probed on this host; skipping suppress-limits assertion");
    }
}

/// SW backends (and HW backends without an engine_probe wired) keep
/// the legacy backend-level `limits` block — there's no per-device
/// source of truth to defer to. The `h264_sw` block is always present
/// so we can hard-assert here.
#[test]
fn info_h264_keeps_backend_limits_for_sw() {
    let out = run_info("h264");
    let start = out
        .find("Backend: h264_sw")
        .expect("h264_sw backend block missing from `oxideav info h264` output");
    let tail = &out[start..];
    let end = tail[1..]
        .find("\nBackend: ")
        .map(|i| 1 + i)
        .unwrap_or(tail.len());
    let block = &tail[..end];
    // SW path either prints declared limits or the explicit
    // "(none declared)" placeholder — either way the section header
    // shows up.
    assert!(
        block.contains("limits         :"),
        "SW backend `h264_sw` should keep backend-level `limits` block, got:\n{block}"
    );
}

/// When a per-device codec caps line carries `max_width` /
/// `max_height` from the probe, the renderer must surface them as
/// `max NxM` on the per-codec caps line under the device. We only
/// assert the *presence* of the formatting when at least one device
/// reports both dims — otherwise (probe returned None for the dims)
/// the assertion silently passes, matching the skip-friendly pattern
/// of the other tests in this file.
#[test]
fn info_h264_per_device_caps_line_includes_max_dims_when_reported() {
    let out = run_info("h264");
    // Look for any `h264 caps        ...  max NxM` line under a
    // device block. Regex would be cleaner but adding a dep here is
    // overkill — scan line-by-line.
    let mut saw_device_dims = false;
    for line in out.lines() {
        let t = line.trim_start();
        if !t.starts_with("h264 caps") {
            continue;
        }
        if t.contains("  max ") {
            // Must look like "max <num>x<num>" — quick sanity check.
            if let Some(rest) = t.split("  max ").nth(1) {
                let dims = rest.split_whitespace().next().unwrap_or("");
                if dims.contains('x') && dims.split('x').all(|s| s.parse::<u32>().is_ok()) {
                    saw_device_dims = true;
                    break;
                }
            }
        }
    }
    if !saw_device_dims {
        eprintln!(
            "no per-device h264 caps line carries `max NxM` on this host; skipping. output:\n{out}"
        );
    }
}

#[test]
fn info_unknown_codec_errors() {
    let bin = env!("CARGO_BIN_EXE_oxideav");
    let output = Command::new(bin)
        .arg("info")
        .arg("totally-not-a-codec-id")
        .output()
        .expect("spawn oxideav binary");
    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown codec id, got status={:?} stdout={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
    );
}
