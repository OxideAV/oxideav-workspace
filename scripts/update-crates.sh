#!/bin/bash
# Clone and/or fast-forward every OxideAV/oxideav{,-*} crate under
# crates/ using a single GraphQL query to discover SHAs.
#
# Usage:
#   ./scripts/update-crates.sh          # clone + update all
#   ./scripts/update-crates.sh -n       # dry-run — report only
#
# Behaviour:
#   * One GraphQL call returns every OxideAV repo's default branch + SHA.
#   * For each repo (filtered to oxideav{,-*}):
#       - if `crates/<name>/` doesn't exist, clone it.
#       - else if the upstream SHA is already an ancestor of HEAD, skip.
#       - else fetch the default branch and fast-forward HEAD.
#       - if the fast-forward would clobber a divergent local branch,
#         print a warning and skip (user's work is preserved).
#   * `.github`, `demo-repository`, `docs`, `oxideav.github.io`, and
#     `oxideav-workspace` are skipped (they don't live under crates/).
#
# Exit codes: 0 = all fine, 1 = at least one repo couldn't be updated.

set -euo pipefail

dry_run=0
case "${1:-}" in
    -n|--dry-run) dry_run=1 ;;
    "")           ;;
    *)            echo "usage: $0 [-n|--dry-run]" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."
repo_root="$(pwd)"
crates_dir="$repo_root/crates"

SKIP_NAMES=(".github" "demo-repository" "docs" "oxideav-workspace" "oxideav.github.io")

is_skipped() {
    local name="$1"
    for s in "${SKIP_NAMES[@]}"; do
        [ "$name" = "$s" ] && return 0
    done
    return 1
}

echo "Querying OxideAV repos via GraphQL…"
entries="$(gh api graphql -f query='
{ organization(login:"OxideAV"){
    repositories(first:100){
      nodes{ name defaultBranchRef{ name target{ oid } } }
      pageInfo{ hasNextPage endCursor }
}}}' --jq '.data.organization.repositories.nodes[]
  | select(.defaultBranchRef != null)
  | "\(.name) \(.defaultBranchRef.name) \(.defaultBranchRef.target.oid)"')"

# If GitHub ever grows past 100 repos we need pagination — bail out loudly
# so the next maintainer fixes it instead of silently missing repos.
has_next="$(gh api graphql -f query='
{ organization(login:"OxideAV"){
    repositories(first:100){ pageInfo{ hasNextPage } }
}}' --jq '.data.organization.repositories.pageInfo.hasNextPage')"
if [ "$has_next" = "true" ]; then
    echo "error: OxideAV has >100 repos; add pagination to this script." >&2
    exit 1
fi

cloned=0
updated=0
ahead=0
current=0
divergent=0
failed=0

mkdir -p "$crates_dir"

while IFS=' ' read -r name branch remote_sha; do
    [ -z "$name" ] && continue
    is_skipped "$name" && continue

    # Only touch oxideav aggregator + sibling crates.
    case "$name" in
        oxideav|oxideav-*) ;;
        *) continue ;;
    esac

    path="$crates_dir/$name"
    if [ ! -d "$path/.git" ]; then
        if [ "$dry_run" = 1 ]; then
            echo "would clone: $name"
            cloned=$((cloned + 1))
            continue
        fi
        echo "cloning: OxideAV/$name -> crates/$name"
        if gh repo clone "OxideAV/$name" "$path" -- --quiet; then
            cloned=$((cloned + 1))
        else
            echo "  clone failed" >&2
            failed=$((failed + 1))
        fi
        continue
    fi

    # Is the remote SHA already in our local history?
    if git -C "$path" cat-file -e "$remote_sha^{commit}" 2>/dev/null \
       && git -C "$path" merge-base --is-ancestor "$remote_sha" HEAD 2>/dev/null; then
        # Possibly HEAD is even ahead of remote.
        head_sha="$(git -C "$path" rev-parse HEAD)"
        if [ "$head_sha" = "$remote_sha" ]; then
            current=$((current + 1))
        else
            ahead=$((ahead + 1))
        fi
        continue
    fi

    # Upstream has something we don't. Fetch + fast-forward.
    if [ "$dry_run" = 1 ]; then
        echo "would update: $name ($branch → ${remote_sha:0:10})"
        updated=$((updated + 1))
        continue
    fi

    echo "updating: $name ($branch → ${remote_sha:0:10})"
    if ! git -C "$path" fetch --quiet origin "$branch"; then
        echo "  fetch failed" >&2
        failed=$((failed + 1))
        continue
    fi

    local_branch="$(git -C "$path" symbolic-ref --short -q HEAD || true)"
    if [ -z "$local_branch" ]; then
        echo "  detached HEAD — skipping merge" >&2
        divergent=$((divergent + 1))
        continue
    fi

    if [ "$local_branch" != "$branch" ]; then
        echo "  on branch '$local_branch', upstream default is '$branch' — skipping" >&2
        divergent=$((divergent + 1))
        continue
    fi

    # Fast-forward only. If local has diverged, `merge --ff-only` fails
    # and we leave the clone untouched so user work survives.
    if git -C "$path" merge --ff-only --quiet "$remote_sha" 2>/dev/null; then
        updated=$((updated + 1))
    else
        echo "  local branch has diverged from origin/$branch — not touching" >&2
        divergent=$((divergent + 1))
    fi
done <<< "$entries"

echo ""
echo "Summary: $cloned cloned, $updated updated, $ahead ahead of remote, $current up-to-date, $divergent diverged/skipped, $failed failed."

[ "$failed" -eq 0 ] || exit 1
