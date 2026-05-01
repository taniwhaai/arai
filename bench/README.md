# bench/

Performance harnesses for the hook hot path.  Run by hand — these aren't
CI gates, they're for capturing before/after numbers around perf-sensitive
changes.

## hot_path.sh

End-to-end subprocess timing for `arai guardrails --match-stdin`.  Spawns
the release binary against a synthetic project the same way Claude Code
spawns it on every tool call (fork + exec + parse + match + exit).

```bash
# Default: 200 synthetic rules, 200 invocations
bench/hot_path.sh

# Larger
N_RULES=500 N_RUNS=1000 bench/hot_path.sh
```

Output is per-percentile wall-clock in ms.  Capture before and after a
change to a hot path (`store::Store::open`, `guardrails::match_guardrails`,
`guardrails::sniff_content_for_tools`, the audit/telemetry write paths)
to quantify the win.

The harness uses a temp `ARAI_HOME` so it never touches the user's real
database or audit log; the temp dir is cleaned up on exit.

## What this measures vs what it doesn't

**Includes:** binary fork+exec, config load, store open + PRAGMAs +
migration check, JSON parse, term extraction, `load_guardrails` (with
LEFT-JOINed intent), `match_guardrails`, audit log append, telemetry
queue append, hook response serialise + write.

**Doesn't include:** Claude Code's own hook-spawn overhead (a few ms on
top of what we measure here), DNS / network (Arai's hook hot path is
network-free by design), enrichment (off-hot-path; only `arai scan
--enrich*` triggers it).

A native in-process criterion bench (no subprocess overhead) would need a
`lib.rs` split — currently arai is a binary-only crate.  Worth doing once
the perf surface stabilises; the subprocess timing here is what reflects
real per-tool-call cost in the meantime.
