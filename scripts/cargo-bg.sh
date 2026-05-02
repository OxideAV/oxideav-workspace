#!/bin/bash
# Wrap `cargo` so its child rustc / linker / test binaries run at macOS
# "background" QoS tier. Under memory pressure the kernel evicts /
# jetsam-kills background-tier processes first — giving the system
# (and the rss-watchdog) a slower scenario to catch a runaway before
# the box OOMs.
#
# Usage:
#   ./scripts/cargo-bg.sh test -p oxideav-jpegxl -j 1 --test-threads=2
#   ./scripts/cargo-bg.sh build -p oxideav-prores -j 1
#
# Combine with rss-watchdog.sh for two layers of protection: kernel
# eviction first, hard SIGKILL above THRESHOLD_GB second.
#
# Note: `taskpolicy -c background` propagates to children via the QoS
# system, so rustc, the linker, and the produced test binary all
# inherit the background tier.

exec /usr/sbin/taskpolicy -c background -- cargo "$@"
