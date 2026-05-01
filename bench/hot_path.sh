#!/usr/bin/env bash
# Subprocess-timing harness for the Arai hook hot path.
#
# Spawns the release-mode `arai guardrails --match-stdin` binary in a tight
# loop with a fixed PreToolUse JSON payload — same shape Claude Code sends
# on every tool call — and reports wall-clock distribution stats.  This is
# the metric that actually matters: end-to-end fork+exec+matching+exit, in
# the same conditions the live hook runs under.
#
# Use:
#   bench/hot_path.sh                # default: 200 rules × 200 invocations
#   N_RULES=500 N_RUNS=1000 bench/hot_path.sh
#
# Compares well against itself across commits — capture stdout before/after
# a perf change to quantify the win.  Not a CI gate; run by hand when you
# want a number.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
N_RULES="${N_RULES:-200}"
N_RUNS="${N_RUNS:-200}"

# Build the release binary once.
echo ">>> Building release binary..."
(cd "$REPO_ROOT" && cargo build --release --quiet)
ARAI="$REPO_ROOT/target/release/arai"
if [[ ! -x "$ARAI" ]]; then
  echo "release binary not found at $ARAI" >&2
  exit 1
fi

# Sandbox: temp ARAI_HOME so this run never touches the user's real DB.
SANDBOX="$(mktemp -d -t arai_bench_XXXXXX)"
PROJECT="$SANDBOX/project"
ARAI_BASE="$SANDBOX/arai_base"
mkdir -p "$PROJECT" "$ARAI_BASE"
trap 'rm -rf "$SANDBOX"' EXIT

# Generate a synthetic CLAUDE.md with N_RULES imperatives across cycling
# subjects so subject-token matching has realistic spread.
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

# Seed the DB.  `arai init` would wire up Claude Code hooks too, which is
# noise here — we want the rule store populated and that's it.  `arai scan`
# does just that.
export ARAI_HOME="$ARAI_BASE"
(cd "$PROJECT" && "$ARAI" scan >/dev/null)

RULE_COUNT=$(cd "$PROJECT" && "$ARAI" status 2>&1 | awk '/Rules:/ {print $2}')
echo ">>> Seeded $RULE_COUNT rules"

# Fixed PreToolUse payload — Bash command that should match a couple of
# the synthetic rules (git + cargo are in TOOLS).
PAYLOAD='{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push --force origin main && cargo test"},"session_id":"bench-sess-01"}'

echo ">>> Timing $N_RUNS invocations..."

# Capture per-call wall-clock in ms; we use $EPOCHREALTIME (bash 5+) for
# microsecond resolution.  Falls back to `date +%s%N` if unavailable.
declare -a TIMES
i=0
while [[ $i -lt $N_RUNS ]]; do
  if [[ -n "${EPOCHREALTIME:-}" ]]; then
    start_us=${EPOCHREALTIME/./}
    printf '%s' "$PAYLOAD" | (cd "$PROJECT" && "$ARAI" guardrails --match-stdin >/dev/null) || true
    end_us=${EPOCHREALTIME/./}
  else
    start_us=$(($(date +%s%N) / 1000))
    printf '%s' "$PAYLOAD" | (cd "$PROJECT" && "$ARAI" guardrails --match-stdin >/dev/null) || true
    end_us=$(($(date +%s%N) / 1000))
  fi
  delta_us=$((end_us - start_us))
  TIMES[$i]=$delta_us
  i=$((i + 1))
done

# Sort and pick stats.
SORTED=($(printf '%s\n' "${TIMES[@]}" | sort -n))
COUNT=${#SORTED[@]}
SUM=0
for v in "${SORTED[@]}"; do SUM=$((SUM + v)); done
MEAN=$((SUM / COUNT))
MEDIAN=${SORTED[$((COUNT / 2))]}
P95=${SORTED[$((COUNT * 95 / 100))]}
P99=${SORTED[$((COUNT * 99 / 100))]}
MIN=${SORTED[0]}
MAX=${SORTED[$((COUNT - 1))]}

ms() { awk "BEGIN { printf \"%.2f\", $1 / 1000.0 }"; }

cat <<REPORT

===== Arai hot-path subprocess timing =====
  rules:   $RULE_COUNT
  runs:    $COUNT
  payload: PreToolUse / Bash / "git push --force ... && cargo test"

  min      $(ms "$MIN") ms
  median   $(ms "$MEDIAN") ms
  mean     $(ms "$MEAN") ms
  p95      $(ms "$P95") ms
  p99      $(ms "$P99") ms
  max      $(ms "$MAX") ms
============================================
REPORT
