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
