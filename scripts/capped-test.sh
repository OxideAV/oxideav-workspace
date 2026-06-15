#!/usr/bin/env bash
# capped-test.sh — run a command under a hard resident-memory ceiling.
#
# Polls the launched command's process group every few seconds; if the group's
# total RSS exceeds the cap, the whole group is SIGKILLed. This bounds runaway
# test/build processes (e.g. a buggy encoder test allocating tens of GB) so they
# can never OOM-reboot the machine.
#
#   OXIDEAV_TEST_MEM_CAP_MB   per-invocation RSS ceiling in MiB (default 8192)
#   OXIDEAV_TEST_MEM_POLL_SECS poll interval in seconds        (default 3)
#
# Usage:
#   scripts/capped-test.sh cargo test --no-run
#   OXIDEAV_TEST_MEM_CAP_MB=6144 scripts/capped-test.sh cargo test
#
# Portable across macOS (no setsid) and Linux via perl setpgrp.
set -u

cap_mb="${OXIDEAV_TEST_MEM_CAP_MB:-8192}"
poll="${OXIDEAV_TEST_MEM_POLL_SECS:-3}"

if [ "$#" -eq 0 ]; then
  echo "usage: capped-test.sh <command> [args...]" >&2
  exit 2
fi

# Launch the command as the leader of a fresh process group so the whole
# subtree (cargo -> rustc/linker -> test binaries) can be group-killed.
perl -e 'setpgrp(0,0); exec @ARGV or die "exec: $!\n"' -- "$@" &
leader=$!
pgid=$leader

cleanup() { kill -- -"$pgid" 2>/dev/null; }
trap cleanup EXIT INT TERM

while kill -0 "$leader" 2>/dev/null; do
  # Sum RSS (KiB) over every process in the group; -ax covers all owners.
  total_kb=$(ps -o rss=,pgid= -ax 2>/dev/null | awk -v g="$pgid" '$2==g {s+=$1} END {print s+0}')
  total_mb=$(( total_kb / 1024 ))
  if [ "$total_mb" -gt "$cap_mb" ]; then
    echo "capped-test: pgid $pgid reached ${total_mb} MiB > ${cap_mb} MiB cap — SIGKILL" >&2
    kill -9 -- -"$pgid" 2>/dev/null
    wait "$leader" 2>/dev/null
    exit 137
  fi
  sleep "$poll"
done

wait "$leader"
