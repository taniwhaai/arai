#!/usr/bin/env bash
# Time the "no rules match" path: tool isn't on the skip list, so we go
# through extract_terms / load_guardrails / match_guardrails — but the
# payload doesn't trigger any matches, so we skip the audit/telemetry
# write and the response is empty.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
N_RUNS="${N_RUNS:-100}"
N_RULES="${N_RULES:-200}"
ARAI="$REPO_ROOT/target/release/arai"
[[ -x "$ARAI" ]] || (cd "$REPO_ROOT" && cargo build --release --quiet)

SANDBOX="$(mktemp -d -t arai_nomatch_XXXXXX)"
PROJECT="$SANDBOX/project"
export ARAI_HOME="$SANDBOX/arai_base"
mkdir -p "$PROJECT" "$ARAI_HOME"
trap 'rm -rf "$SANDBOX"' EXIT

# Same synthetic CLAUDE.md as bench/hot_path.sh.
TOOLS=(git cargo npm yarn pip docker pytest jest webpack vite eslint terraform kubectl helm)
PREDICATES=(never always requires prefers)
{
  echo "# Bench rules"
  i=0
  while [[ $i -lt $N_RULES ]]; do
    tool="${TOOLS[$((i % ${#TOOLS[@]}))]}"
    pred="${PREDICATES[$((i % ${#PREDICATES[@]}))]}"
    echo "- ${pred^} $tool synthetic-rule-$i"
    i=$((i + 1))
  done
} > "$PROJECT/CLAUDE.md"
(cd "$PROJECT" && "$ARAI" scan >/dev/null)

# Bash command whose terms (date, uptime, whoami) match no synthetic rule.
PAYLOAD='{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"date && uptime && whoami"},"session_id":"s"}'

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

RULE_COUNT=$(cd "$PROJECT" && "$ARAI" status 2>&1 | awk '/Rules:/ {print $2}')

echo
echo "===== Full pipeline, no match (Bash 'date && uptime') ====="
echo "  rules:   $RULE_COUNT"
echo "  runs:    $COUNT"
echo "  min      $(ms "$MIN") ms"
echo "  median   $(ms "$MED") ms"
echo "  p95      $(ms "$P95") ms"
echo "  max      $(ms "$MAX") ms"
echo "==========================================================="
