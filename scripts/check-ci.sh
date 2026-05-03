#!/bin/bash
# Query the GitHub status of every OxideAV/oxideav-* repo's master in
# a single GraphQL call and print a colour-coded summary. Far faster
# than running `cargo clippy --workspace` locally — useful as a quick
# "is anything red right now?" check.
#
# Usage:
#   ./scripts/check-ci.sh                 # report all repos (sorted)
#   ./scripts/check-ci.sh --failing-only  # only show repos in non-SUCCESS state
#   ./scripts/check-ci.sh --json          # raw JSON (e.g. for jq piping)
#
# Exits non-zero if any repo's master is in FAILURE / ERROR state.

set -uo pipefail

FAILING_ONLY=0
JSON_OUT=0
while [ $# -gt 0 ]; do
    case "$1" in
        --failing-only) FAILING_ONLY=1; shift ;;
        --json) JSON_OUT=1; shift ;;
        -h|--help) sed -n '2,12p' "$0" | sed 's|^# ||; s|^#||'; exit 0 ;;
        *) echo "error: unknown arg: $1" >&2; exit 2 ;;
    esac
done

if [ -t 1 ]; then
    R=$'\033[31m'; G=$'\033[32m'; Y=$'\033[33m'; D=$'\033[2m'; B=$'\033[1m'; N=$'\033[0m'
else
    R=""; G=""; Y=""; D=""; B=""; N=""
fi

# Single GraphQL request. `statusCheckRollup` aggregates every check
# (Actions + status contexts) into one of: SUCCESS, FAILURE, PENDING,
# ERROR, EXPECTED. `null` means "no checks ran" (rare — empty repo or
# branch never CI'd). Page if the org grows past 100 repos.
read -r -d '' QUERY <<'EOF' || true
query($endCursor: String) {
  organization(login: "OxideAV") {
    repositories(first: 100, after: $endCursor, isArchived: false) {
      pageInfo { hasNextPage endCursor }
      nodes {
        name
        defaultBranchRef {
          name
          target {
            ... on Commit {
              oid
              committedDate
              # Per-workflow conclusions instead of the rollup so the
              # script can ignore Release-plz-only failures (publishing
              # plumbing) and only flag CI workflow regressions.
              checkSuites(first: 20) {
                nodes {
                  workflowRun { workflow { name } }
                  conclusion
                }
              }
            }
          }
        }
      }
    }
  }
}
EOF

raw="$(gh api graphql --paginate -f query="$QUERY" 2>&1)" || {
    echo "error: gh api graphql failed:" >&2
    echo "$raw" >&2
    exit 2
}

if [ $JSON_OUT -eq 1 ]; then
    echo "$raw"
    exit 0
fi

# Flatten paginated responses + filter to oxideav-* and sort.
rows="$(echo "$raw" | python3 -c '
import json, sys
data = sys.stdin.read()
# `gh --paginate` concatenates JSON objects; split on closing brace + opening brace.
chunks = []
depth = 0
buf = ""
for ch in data:
    buf += ch
    if ch == "{": depth += 1
    elif ch == "}":
        depth -= 1
        if depth == 0:
            chunks.append(buf)
            buf = ""
rows = []
for chunk in chunks:
    try:
        obj = json.loads(chunk)
    except json.JSONDecodeError:
        continue
    repos = obj.get("data", {}).get("organization", {}).get("repositories", {}).get("nodes", [])
    for r in repos:
        name = r.get("name", "")
        if not name.startswith("oxideav"):
            continue
        ref = r.get("defaultBranchRef") or {}
        target = ref.get("target") or {}
        suites = (target.get("checkSuites") or {}).get("nodes", []) or []
        # Distinguish "CI workflow failed" (real regression) from
        # "Release-plz workflow failed" (publishing plumbing). We
        # report the worst CI conclusion; if no CI conclusion is
        # FAILURE/ERROR but Release-plz failed, report SUCCESS — the
        # publishing side gets its own cleanup elsewhere.
        ci_state = "NONE"
        for s in suites:
            wf = ((s.get("workflowRun") or {}).get("workflow") or {}).get("name", "")
            conc = s.get("conclusion") or ""
            # Treat anything not literally "Release-plz" as a CI workflow.
            if wf == "Release-plz":
                continue
            if conc in ("FAILURE", "ERROR"):
                ci_state = conc; break
            if conc == "SUCCESS" and ci_state == "NONE":
                ci_state = "SUCCESS"
            elif conc in ("PENDING", "EXPECTED") and ci_state == "NONE":
                ci_state = "PENDING"
        sha = (target.get("oid") or "")[:7]
        date = (target.get("committedDate") or "")[:10]
        rows.append((name, ci_state, sha, date))
rows.sort()
for n, s, c, d in rows:
    print(f"{n}\t{s}\t{c}\t{d}")
')"

if [ -z "$rows" ]; then
    echo "no oxideav-* repos returned" >&2
    exit 2
fi

bad=0
ok=0
pending=0
none=0
total=0
shown=0

while IFS=$'\t' read -r name state sha date; do
    total=$((total+1))
    case "$state" in
        SUCCESS) colour="$G"; symbol="ok       "; ok=$((ok+1)) ;;
        FAILURE|ERROR) colour="$R"; symbol="$state "; bad=$((bad+1)) ;;
        PENDING|EXPECTED) colour="$Y"; symbol="$state "; pending=$((pending+1)) ;;
        *) colour="$D"; symbol="no-checks"; none=$((none+1)) ;;
    esac
    if [ $FAILING_ONLY -eq 1 ] && [ "$state" = "SUCCESS" ]; then
        continue
    fi
    shown=$((shown+1))
    printf "  %s%-12s%s  %-30s  %s%s%s\n" "$colour" "$symbol" "$N" "$name" "$D" "$sha $date" "$N"
done <<< "$rows"

echo
printf "${B}Summary:${N} %d repos — ${G}%d ok${N}, ${R}%d failing${N}, ${Y}%d pending${N}, ${D}%d no-checks${N}\n" \
    "$total" "$ok" "$bad" "$pending" "$none"

[ $bad -gt 0 ] && exit 1
exit 0
