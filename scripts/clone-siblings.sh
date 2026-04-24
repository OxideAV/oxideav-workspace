#!/bin/bash
# Clone every OxideAV/oxideav-* repo into crates/ so the workspace is
# fully self-contained — no crates.io resolution needed for any oxideav-*
# dep, local development and CI both work from a single `cargo build`.
#
# Usage:
#   ./scripts/clone-siblings.sh
#
# Behaviour:
#   * Enumerates OxideAV repos via the `gh` CLI.
#   * Skips repos that are not Rust crates (`.github`, `oxideav.github.io`,
#     `oxideav-dot-github`, `demo-repository`) and the workspace repo itself
#     (`oxideav-workspace`).
#   * For each remaining repo: if `crates/<name>` does not exist, clones
#     it; otherwise leaves it alone (doesn't fetch, doesn't touch local
#     work).
#
# CI-safe: uses HTTPS clones via `gh repo clone`, which works with the
# default `GITHUB_TOKEN` in GitHub Actions.

set -euo pipefail

cd "$(dirname "$0")/.."
repo_root="$(pwd)"
crates_dir="$repo_root/crates"

# Repos that are in the OxideAV org but are not Rust crates we need here.
# Extend this list if the org grows in other directions.
SKIP=(
    ".github"
    "demo-repository"
    "oxideav-dot-github"
    "oxideav-workspace"
    "oxideav.github.io"
    # Retired: functionality was folded into oxideav-pipeline. The repo is
    # archived upstream and kept out of the workspace to avoid reintroducing
    # stale deps on fresh clones.
    "oxideav-job"
)

is_skipped() {
    local name="$1"
    for s in "${SKIP[@]}"; do
        [ "$name" = "$s" ] && return 0
    done
    return 1
}

echo "Enumerating OxideAV/oxideav-* repos..."
names="$(gh repo list OxideAV --limit 200 --json name --jq '.[] | .name' | sort)"
if [ -z "$names" ]; then
    echo "error: gh repo list returned nothing — are you logged in (gh auth status)?" >&2
    exit 1
fi

mkdir -p "$crates_dir"

cloned=0
skipped=0
present=0

while IFS= read -r name; do
    [ -z "$name" ] && continue
    if is_skipped "$name"; then
        skipped=$((skipped + 1))
        continue
    fi
    # Only clone the aggregator and oxideav-* repos. The org may pick up
    # unrelated names over time; this check keeps those out.
    case "$name" in
        oxideav|oxideav-*) ;;
        *)
            skipped=$((skipped + 1))
            continue
            ;;
    esac
    target="$crates_dir/$name"
    if [ -e "$target/Cargo.toml" ]; then
        present=$((present + 1))
        continue
    fi
    if [ -e "$target" ]; then
        echo "warning: $target exists but has no Cargo.toml — moving aside to $target.bak" >&2
        mv "$target" "$target.bak"
    fi
    echo "clone: OxideAV/$name -> crates/$name"
    gh repo clone "OxideAV/$name" "$target" -- --quiet
    cloned=$((cloned + 1))
done <<< "$names"

echo ""
echo "Summary: $cloned cloned, $present already present, $skipped skipped."

# Best-effort clone of the private OxideAV/docs repo into docs/. Not a
# Rust crate — kept outside crates/ and gitignored. Failure is non-fatal
# (CI's default GITHUB_TOKEN and external contributors won't have access
# to the private repo; the workspace still builds without it).
docs_dir="$repo_root/docs"
if [ -e "$docs_dir/.git" ]; then
    echo "docs: already present, skipping"
elif gh repo clone "OxideAV/docs" "$docs_dir" -- --quiet 2>/dev/null; then
    echo "docs: cloned OxideAV/docs -> docs/"
else
    echo "docs: OxideAV/docs not accessible (private repo) — skipping"
    rm -rf "$docs_dir" 2>/dev/null || true
fi
