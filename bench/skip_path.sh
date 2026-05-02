#!/usr/bin/env bash
# Time the skip-tool fast-exit path (e.g. tool=Read).  This bypasses
# `load_guardrails` and the full match pipeline — should be the fastest
# path the hook ever takes.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
N_RUNS="${N_RUNS:-100}"
ARAI="$REPO_ROOT/target/release/arai"
[[ -x "$ARAI" ]] || (cd "$REPO_ROOT" && cargo build --release --quiet)

SANDBOX="$(mktemp -d -t arai_skip_XXXXXX)"
PROJECT="$SANDBOX/project"
export ARAI_HOME="$SANDBOX/arai_base"
mkdir -p "$PROJECT" "$ARAI_HOME"
trap 'rm -rf "$SANDBOX"' EXIT

echo "- Never force-push to main" > "$PROJECT/CLAUDE.md"
(cd "$PROJECT" && "$ARAI" scan >/dev/null)

PAYLOAD='{"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"x.py"},"session_id":"s"}'

declare -a TIMES
i=0
while [[ $i -lt $N_RUNS ]]; do
  start=${EPOCHREALTIME/./}
  printf '%s' "$PAYLOAD" | (cd "$PROJECT" && "$ARAI" guardrails --match-stdin >/dev/null) || true
  end=${EPOCHREALTIME/./}
  TIMES[$i]=$((end - start))
  i=$((i + 1))
done

SORTED=($(printf '%s\n' "${TIMES[@]}" | sort -n))
COUNT=${#SORTED[@]}
MED=${SORTED[$((COUNT / 2))]}
P95=${SORTED[$((COUNT * 95 / 100))]}
MIN=${SORTED[0]}
MAX=${SORTED[$((COUNT - 1))]}
ms() { awk "BEGIN{printf \"%.2f\", $1/1000.0}"; }

echo
echo "===== Skip-tool fast-exit (Read) ====="
echo "  runs:    $COUNT"
echo "  min      $(ms "$MIN") ms"
echo "  median   $(ms "$MED") ms"
echo "  p95      $(ms "$P95") ms"
echo "  max      $(ms "$MAX") ms"
echo "======================================"
