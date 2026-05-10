# Dispatch metrics

This document describes the metrics fields the dispatcher records after a subagent returns, and the graceful-degradation contract for fields the host doesn't expose.

This document is **host-agnostic**. It does not specify how to parse any particular host's render. The dispatcher (the LLM running this skill) extracts fields from whatever the host actually shows it; what's available depends on the host (Claude Code, Aider, Continue, custom CLI runners, future open-source agents). This document describes what to record when extraction succeeds and what to do when it doesn't.

## When metrics get recorded

After every subagent dispatch returns (whether successfully, with a re-raise, or with a failure), the dispatcher records metrics for that dispatch via `kupu.record_dispatch_metrics`. This happens once per handoff, idempotently — `record_dispatch_metrics` errors with `MetricsAlreadyRecorded` if called twice for the same handoff.

If Kupu is not installed or the metrics tools are not available, this step is skipped — there is no bash-fallback for metrics recording. Metrics are an optional capability; the build succeeds without them.

## Fields to record

The dispatcher attempts to extract the following fields from the host's subagent-return summary:

- **`tokens_used`** — total token count reported by the host for this dispatch. Sum of input and output if both reported separately.
- **`input_tokens`** — input tokens specifically, if the host reports them separately. Optional.
- **`output_tokens`** — output tokens specifically, if the host reports them separately. Optional.
- **`wall_clock_ms`** — wall-clock duration of the dispatch in milliseconds. Most hosts report some duration; convert from minutes/seconds as needed.
- **`tool_call_count`** — number of tool invocations the subagent made. Reported by some hosts.
- **`model`** — the model identifier the subagent ran on (e.g. `claude-opus-4-7`, `claude-sonnet-4-6`, or whatever model identifier the host uses). The dispatcher knows this because the dispatch action specified it.
- **`role`** — the subagent role (`leaf-implementation`, `verifier`, `composition`, etc.). The dispatcher knows this because it dispatched the subagent.
- **`recorded_at`** — current Timestamp at the moment of recording.

Of these, only `model`, `role`, and `recorded_at` are knowable to the dispatcher with certainty. The rest depend on what the host shows.

## Graceful degradation contract

Hosts vary in what they expose. Some show full token counts and tool-use counts as part of the subagent return summary. Some show only duration. Some show nothing structured at all. The dispatcher MUST NOT fail to record metrics when extraction is partial.

The contract is:

1. **Pass what you have, null what you don't.** Fields the dispatcher could extract from the host's render are recorded with their values. Fields not visible in the host's output are passed as null.
2. **Set `parse_failure: true` if any required field is missing.** A "required field" for this purpose is `tokens_used` and/or `wall_clock_ms` — the two load-bearing cost signals. If either is null because extraction failed, `parse_failure: true` is set on the metric record. If both are present, `parse_failure: false`.
3. **Never reject the dispatch over missing metrics.** A failed extraction is not a failed dispatch. The build proceeds; the metrics are recorded with whatever fields were available; downstream queries (`get_build_metrics`, `export_metrics`) handle the partial data gracefully.
4. **Never invent values.** If the host doesn't show token counts, the dispatcher does not estimate, infer, or guess. Null is the honest answer.

This is the contract the dispatcher follows regardless of host. It works for any LLM running on any host that supports MCP, because the only requirement is "extract what you can see, be honest about what you can't."

## Why this is host-agnostic

The dispatcher running this skill IS the LLM. The LLM can read its own input — including whatever the host renders for subagent returns — at runtime. There is no need for the skill text to specify a particular regex, parser, or format. The LLM reads the host's render directly, identifies metric values where it sees them, passes them to `kupu.record_dispatch_metrics`, and uses null + `parse_failure: true` for the rest.

This means:

- A skill running on Claude Code can extract from Claude Code's "Done (N tool uses · X tokens · Y duration)" summary line.
- A skill running on Aider extracts from whatever Aider shows after a `/run` or subagent return.
- A skill running on a custom CLI runner extracts from whatever that runner exposes.
- A skill running on a future host that doesn't expose subagent metrics at all records `parse_failure: true` with nulls and the build still succeeds.

The skills stay portable. No host-specific assumptions in skill text.

## What the orchestrator's action should look like

When the orchestrator emits a record_dispatch_metrics action in `next_action.yaml`, the action does NOT include pre-extracted metric values — those are the dispatcher's job to extract at execution time. The action shape is:

```yaml
action: record_dispatch_metrics
handoff_id: leaf-implementation-01ABCDEFGHIJKLMNOPQRSTUVWXYZ
# The dispatcher extracts metrics from the host's render of the subagent return
# at execution time and passes the parsed fields (or nulls + parse_failure: true)
# to kupu.record_dispatch_metrics.
```

The dispatcher, executing this action, reads its own context (the host's recent subagent return), extracts whatever fields it can identify, and constructs the call to `kupu.record_dispatch_metrics` accordingly.

## Ordering

The dispatcher records metrics AFTER the subagent's return has been fully processed (events written, status updated) and BEFORE re-invoking the orchestrator for the next round. This ensures:

- Metrics are recorded for every dispatch, not just successful ones.
- Metrics for a given dispatch are stable on disk by the time the next orchestrator round can read them.
- A failed `record_dispatch_metrics` (e.g. `MetricsAlreadyRecorded` on retry) doesn't block the build's forward progress; the dispatcher logs the failure and continues.

## Querying recorded metrics

After metrics are recorded, agents can query them via Kupu's read tools:

- `kupu.get_dispatch_metrics(handoff_id)` — retrieve a single dispatch's metrics
- `kupu.get_build_metrics()` — aggregate view across the build
- `kupu.get_dispatch_trace(handoff_id)` — full trace for one dispatch
- `kupu.export_metrics(format)` — export as JSON, NDJSON, or CSV

These tools handle partial records gracefully — `parse_failure: true` records contribute their available fields to aggregates and are reported with their `parse_failure` annotation in detail views.

## What this enables

With dispatch metrics recorded, future builds can answer questions like:

- How long does a typical leaf-implementation take? Verifier? Orchestrator?
- Which model is doing the most work? Sonnet vs Opus distribution.
- Which dispatches are outliers — abnormally slow or token-heavy?
- Across phases of a multi-phase build, are per-phase costs trending up or down?

Without dispatch metrics, all of these questions are vibes-based. With them, they are answerable from on-disk data.

The cost of recording metrics is small (one `kupu.record_dispatch_metrics` call per dispatch); the value compounds over multi-build histories.
