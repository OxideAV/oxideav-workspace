#!/bin/bash
# RSS watchdog — kills any process whose RSS exceeds the threshold.
# Catches runaway test binaries, leaked processes, and anything else
# that grows unbounded. macOS does not enforce per-process memory limits
# reliably (`ulimit -v` is dodgy for rustc/lld), so this polls `ps`
# every few seconds and SIGKILLs offenders.
#
# Usage:
#   nohup ./scripts/rss-watchdog.sh > /dev/null 2>&1 &
#   # ... do stuff ...
#   pkill -f rss-watchdog.sh   # to stop
#
# Tuning via env vars:
#   THRESHOLD_GB    — kill at RSS above this many GB (default 24)
#   POLL_SECONDS    — scan interval (default 5)
#   LOG_FILE        — log destination (default ~/.local/state/oxideav-rss-watchdog.log)
#
# Whitelisted processes are never killed — they're either system-critical
# or the very tools we're using to develop. Add to WHITELIST_REGEX below
# if you find yours getting hit.

set -u

THRESHOLD_GB="${THRESHOLD_GB:-24}"
POLL_SECONDS="${POLL_SECONDS:-5}"
LOG_FILE="${LOG_FILE:-$HOME/.local/state/oxideav-rss-watchdog.log}"

# Convert GB threshold to KB (ps reports RSS in KB on macOS).
THRESHOLD_KB=$(( THRESHOLD_GB * 1024 * 1024 ))

# Never SIGKILL these. The watchdog is here to stop runaway dev work,
# not to take down the system or the IDE you're working in.
WHITELIST_REGEX='^(kernel_task|launchd|WindowServer|loginwindow|Finder|Dock|SystemUIServer|mds|mds_stores|mdworker|corespotlightd|Spotlight|cfprefsd|distnoted|notifyd|sandboxd|securityd|trustd|claude|node|Code|cursor|zed|rust-analyzer|Terminal|iTerm2)$'

mkdir -p "$(dirname "$LOG_FILE")"

log() {
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG_FILE"
}

log "watchdog started: threshold=${THRESHOLD_GB}GB poll=${POLL_SECONDS}s pid=$$"

trap 'log "watchdog stopping (signal received)"; exit 0' INT TERM

while true; do
    # ps -axo pid,rss,comm — for each process print pid, RSS in KB, basename of binary.
    # Skip the header. Filter to lines whose RSS exceeds the threshold.
    ps -axo pid,rss,comm | awk -v t="$THRESHOLD_KB" 'NR>1 && $2+0 > t { print }' | \
    while read -r pid rss comm; do
        # Whitelist by basename of the binary.
        base=$(basename "$comm")
        if printf '%s' "$base" | grep -Eq "$WHITELIST_REGEX"; then
            continue
        fi
        # Don't kill our own watchdog or its parent shell.
        if [ "$pid" = "$$" ] || [ "$pid" = "$PPID" ]; then
            continue
        fi
        rss_gb=$(awk -v r="$rss" 'BEGIN { printf "%.1f", r / 1024 / 1024 }')
        # Capture the full command line before killing, for the log.
        cmdline=$(ps -p "$pid" -o command= 2>/dev/null | head -c 500)
        log "KILL pid=$pid rss=${rss_gb}GB comm=$base cmd=$cmdline"
        kill -9 "$pid" 2>/dev/null
    done
    sleep "$POLL_SECONDS"
done
