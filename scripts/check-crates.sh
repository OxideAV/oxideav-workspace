#!/bin/bash
# Audit `cargo fmt -- --check`, `cargo clippy --all-targets --no-deps
# -- -D warnings`, and optionally `cargo +nightly miri test` across
# every crate under `crates/`. Reports which crates fail which check;
# exits non-zero if any crate fails.
#
# Usage:
#   ./scripts/check-crates.sh                # fmt + clippy (default; miri off)
#   ./scripts/check-crates.sh --fmt-fix      # auto-apply `cargo fmt` to failing crates
#   ./scripts/check-crates.sh --only fmt     # run rustfmt only
#   ./scripts/check-crates.sh --only clippy  # run clippy only
#   ./scripts/check-crates.sh --only miri    # run miri only (slow; nightly+miri required)
#   ./scripts/check-crates.sh --with-miri    # default fmt+clippy AND miri
#   ./scripts/check-crates.sh --crate NAME   # restrict to one crate (repeatable)
#
# Miri is off by default because it's slow (~5-15 min per crate). It
# matches the org-wide CI workflow: `cargo +nightly miri test --all-targets`
# with `MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-disable-isolation"`.
#
# Memory cap: cargo runs with `-j 4` (per memory_limits guidance).

set -uo pipefail

cd "$(dirname "$0")/.."
repo_root="$(pwd)"

FMT_FIX=0
RUN_FMT=1
RUN_CLIPPY=1
RUN_MIRI=0
ONLY_CRATES=()

while [ $# -gt 0 ]; do
    case "$1" in
        --fmt-fix) FMT_FIX=1; shift ;;
        --with-miri) RUN_MIRI=1; shift ;;
        --only)
            case "${2:-}" in
                fmt) RUN_CLIPPY=0; RUN_MIRI=0 ;;
                clippy) RUN_FMT=0; RUN_MIRI=0 ;;
                miri) RUN_FMT=0; RUN_CLIPPY=0; RUN_MIRI=1 ;;
                *) echo "error: --only must be 'fmt', 'clippy', or 'miri'" >&2; exit 2 ;;
            esac
            shift 2
            ;;
        --crate) ONLY_CRATES+=("$2"); shift 2 ;;
        -h|--help) sed -n '2,21p' "$0" | sed 's|^# ||; s|^#||'; exit 0 ;;
        *) echo "error: unknown arg: $1" >&2; exit 2 ;;
    esac
done

if [ $RUN_MIRI -eq 1 ]; then
    if ! cargo +nightly miri --version >/dev/null 2>&1; then
        echo "error: miri requires 'rustup toolchain install nightly' + 'rustup +nightly component add miri'" >&2
        exit 2
    fi
fi

if [ -t 1 ]; then
    R=$'\033[31m'; G=$'\033[32m'; Y=$'\033[33m'; B=$'\033[1m'; N=$'\033[0m'
else
    R=""; G=""; Y=""; B=""; N=""
fi

fmt_fail=()
clippy_fail=()
miri_fail=()
ok_count=0
total=0

# Enumerate crate directories. A crate is any subdir of crates/ with a
# Cargo.toml. Sorted for deterministic output.
crates=()
for d in crates/*/; do
    [ -f "$d/Cargo.toml" ] || continue
    name="$(basename "$d")"
    if [ ${#ONLY_CRATES[@]} -gt 0 ]; then
        skip=1
        for c in "${ONLY_CRATES[@]}"; do
            [ "$name" = "$c" ] && skip=0
        done
        [ $skip -eq 1 ] && continue
    fi
    crates+=("$name")
done

if [ ${#crates[@]} -eq 0 ]; then
    echo "no crates matched" >&2
    exit 2
fi

echo "${B}Auditing ${#crates[@]} crate(s) under crates/${N}"
[ $RUN_FMT -eq 1 ] && echo "  rustfmt: cargo fmt -- --check$([ $FMT_FIX -eq 1 ] && echo ' (auto-fix on failure)')"
[ $RUN_CLIPPY -eq 1 ] && echo "  clippy:  cargo clippy --all-targets --no-deps -- -D warnings"
[ $RUN_MIRI -eq 1 ] && echo "  miri:    cargo +nightly miri test --all-targets (slow)"
echo

for name in "${crates[@]}"; do
    total=$((total+1))
    cd "$repo_root/crates/$name"
    label="${B}$name${N}"
    fmt_status=""
    clippy_status=""

    if [ $RUN_FMT -eq 1 ]; then
        if cargo fmt -- --check >/dev/null 2>&1; then
            fmt_status="${G}fmt-ok${N}"
        elif [ $FMT_FIX -eq 1 ]; then
            cargo fmt >/dev/null 2>&1
            fmt_status="${Y}fmt-fixed${N}"
            fmt_fail+=("$name")
        else
            fmt_status="${R}fmt-FAIL${N}"
            fmt_fail+=("$name")
        fi
    fi

    if [ $RUN_CLIPPY -eq 1 ]; then
        if cargo clippy --all-targets --no-deps -j 4 -- -D warnings >/dev/null 2>&1; then
            clippy_status="${G}clippy-ok${N}"
        else
            clippy_status="${R}clippy-FAIL${N}"
            clippy_fail+=("$name")
        fi
    fi

    miri_status=""
    if [ $RUN_MIRI -eq 1 ]; then
        if MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-disable-isolation" \
           cargo +nightly miri test --all-targets -j 4 -- --test-threads=4 >/dev/null 2>&1; then
            miri_status="${G}miri-ok${N}"
        else
            miri_status="${R}miri-FAIL${N}"
            miri_fail+=("$name")
        fi
    fi

    parts=()
    [ -n "$fmt_status" ] && parts+=("$fmt_status")
    [ -n "$clippy_status" ] && parts+=("$clippy_status")
    [ -n "$miri_status" ] && parts+=("$miri_status")
    line="$(IFS=" / "; echo "${parts[*]}")"
    echo "  $label  $line"

    if [[ "$fmt_status$clippy_status$miri_status" != *FAIL* ]]; then
        ok_count=$((ok_count+1))
    fi
done

cd "$repo_root"
echo
echo "${B}Summary:${N} $ok_count/$total clean"

if [ ${#fmt_fail[@]} -gt 0 ]; then
    if [ $FMT_FIX -eq 1 ]; then
        echo "${Y}fmt auto-fixed${N} (${#fmt_fail[@]}): ${fmt_fail[*]}"
        echo "  → review the diff in each crate, then commit + push per crate"
    else
        echo "${R}fmt failures${N} (${#fmt_fail[@]}): ${fmt_fail[*]}"
        echo "  → re-run with --fmt-fix to auto-apply, or fix manually"
    fi
fi

if [ ${#clippy_fail[@]} -gt 0 ]; then
    echo "${R}clippy failures${N} (${#clippy_fail[@]}): ${clippy_fail[*]}"
    echo "  → cd into each crate and run \`cargo clippy --all-targets --no-deps -- -D warnings\` for the full diagnostics"
fi

if [ ${#miri_fail[@]} -gt 0 ]; then
    echo "${R}miri failures${N} (${#miri_fail[@]}): ${miri_fail[*]}"
    echo "  → cd into each crate and run \`MIRIFLAGS='-Zmiri-strict-provenance -Zmiri-disable-isolation' cargo +nightly miri test --all-targets\` for the full diagnostics"
fi

# Exit non-zero if any FAIL (auto-fix counts as success — file changed,
# but the user asked us to fix it). Clippy + miri are never auto-fixed.
if [ ${#clippy_fail[@]} -gt 0 ] || [ ${#miri_fail[@]} -gt 0 ]; then exit 1; fi
if [ ${#fmt_fail[@]} -gt 0 ] && [ $FMT_FIX -eq 0 ]; then exit 1; fi
exit 0
