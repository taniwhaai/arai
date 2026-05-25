---
version: 4
captured_at: 2026-05-25T09:30:00Z
source: github_issue
parent_version: 3
phase: phase-4-issue-113-prompt-collector
authoritative_spec: github.com/taniwhaai/arai/issues/113
---

# Brief v4 — Issue #113 prompt-text collector (log-only)

## Additive scope

Brief v3 scoped the v0.2.15 docs sweep (issue #74). This v4 amendment scopes
the next build to GitHub #113: a log-only collector that matches free-text in
user prompts against guardrails and emits a `PromptMatch` receipt into the
existing local audit log. **No enforcement. No `additionalContext` injection
from this matcher. No remote aggregation.** Data-collection-first, per the
issue author's explicit reframing on 2026-05-19.

Brief v1 remains the master picture. v2/v3 documented prior builds. v4
narrows this build to the local prompt-text collector only.

## Why now

UserPromptSubmit is already wired (`src/hooks.rs` emits the domain-rule
summary as `additionalContext` at prompt time, per cycle #111). Issue #113
deepens it: matching free text in the prompt itself, so intent ("deploy to
production") can be observed *before* any tool call.

The original plan for this was design-doc-first with an eval harness. The
issue author walked that back: we don't have the labelled prompt corpus to
drive a real eval, and synthesising one from first principles means
evaluating matcher candidates against our own assumptions. The honest first
step is to **collect data, not design enforcement**. v1 ships the collector;
the smart matcher (v2) belongs in the kete repo, not here — per the
[Arai↔Kete design doc](docs/design-http-hooks-kete-integration.md) charter
boundary.

## Scope (single_module tier)

One new module + small wiring into the existing UserPromptSubmit handler.

1. **New** prompt-rules storage: a `prompt_rules` table (or equivalent) of
   `(pattern, label)` pairs. Substring or regex — matcher choice barely
   matters when nothing enforces. Regex is simplest given the existing
   guardrail pipeline already uses regex.
2. **New** prompt-matcher: invoked from `UserPromptSubmit` handling. For each
   loaded rule, test the prompt text against the pattern; on match, emit one
   `PromptMatch` audit-log receipt per matching rule.
3. **Receipt shape** (additive to `src/audit.rs`):
   - `prompt_hash` (SHA-256 of the prompt text — text itself stays local
     only in the receipt's `prompt_preview` field, which already exists for
     the firing path and is bounded to ~200 chars)
   - `matched_label`
   - `timestamp_iso`
   - `project_slug`
   - `did_any_tool_call_follow` (initially `null`; populated by a
     correlation pass in a follow-up, OR set at PostToolUse time using the
     same session-state mechanism used for compliance verdicts)
   - `event: "PromptMatch"`
4. **CLI**: `arai audit --event=PromptMatch` must filter the audit feed to
   these receipts. The existing `--event` filter is the lever.
5. **Seed rules**: ship a small set of starter labels — `deploy`,
   `production`, `secret`, `password`, `kubectl apply`, `force push`. These
   are dogfood-only guesses; the brief explicitly does NOT commit to them
   as policy.
6. **Docs**: a short README/CLAUDE note that this is observation-only and
   the matcher will be revisited in kete per the charter boundary.

## In scope

- New module file at `src/prompt_collector.rs` (or similar — single source
  file per the project's `src/{module}.rs` convention).
- Additive change to `src/hooks.rs` (UserPromptSubmit branch) to invoke the
  collector after existing context emission.
- Additive change to `src/audit.rs` to support the `PromptMatch` event
  variant in `record_event` + `--event=PromptMatch` filter (the variant
  enum already exists if Compliance/firing are there; this adds one more).
- Additive change to `src/main.rs` to declare the new module.
- Seed `prompt_rules` data: committed as a static slice in the module, OR
  loaded from a small file under `~/.taniwha/arai/`. Implementor's call;
  static slice is simpler for v1.
- Tests in `#[cfg(test)] mod tests` covering: empty rules → no receipts,
  single rule + matching prompt → one receipt, single rule + non-matching
  prompt → zero receipts, multiple rules + multi-match → multiple
  receipts, regex special characters, hash determinism.
- An integration-level smoke test that `arai audit --event=PromptMatch`
  reads the new receipts.

## Out of scope (do NOT do)

- **No enforcement.** Not block, not warn-via-additionalContext, not retry
  injection. AC has to verify this structurally — the collector's call site
  in `src/hooks.rs` may NOT mutate the existing hook response in any way.
- **No matcher quality work.** No sweep of matchers, no embedding model, no
  LLM enrichment, no fuzzy match. Regex only.
- **No remote aggregation.** No HTTP egress, no Kete handoff, no telemetry
  beat. Same locality discipline as `src/session.rs`.
- **No prompt-text persistence beyond what already exists.** The receipt
  carries the hash. Whatever prompt-preview the existing firing pipeline
  already truncates (~200 chars) may be reused for the receipt's preview
  field; nothing longer.
- **No design doc** in the lean reading of this brief. The user has
  requested a full Taniwha cycle, so the design-doc subagent WILL run, but
  the design should be short and structural — this is not a research
  cycle.
- **No new crate dependency.** `regex`, `serde`, `serde_json`, `sha2`,
  `rusqlite` are already in Cargo.toml — use them. Do not add new deps.
- **No `prompt_rules` schema migration** beyond v1. The table format ships
  as the bare minimum; expansion (priority, scope, tool-coupling) belongs
  to the post-observation revisit.
- **No DB/storage refactor** of the audit log. The existing JSONL audit
  format admits new event kinds additively.
- **No CHANGELOG rewrite** of historical entries; add a [Unreleased] entry
  only.

## Acceptance criteria

- **AC1**: `prompt_rules` table parser/loader returns the seeded ruleset
  on a fresh install with no user customisation. Verified by a unit test
  asserting the count of seeded rules and their labels.
- **AC2**: UserPromptSubmit handler invokes the collector. Verified by a
  unit test that drives a synthetic prompt through the handler with the
  seeded rules and observes the expected receipts via a captured-audit
  test double.
- **AC3**: Receipts have the exact shape `(prompt_hash, matched_label,
  timestamp_iso, project_slug, did_any_tool_call_follow, event)`.
  `prompt_hash` is SHA-256 hex of the prompt text. Verified by inspecting
  a receipt produced by AC2.
- **AC4**: `arai audit --event=PromptMatch` filters the audit feed to only
  PromptMatch receipts. Verified by an integration test running the live
  binary against a fixture audit log containing mixed event kinds.
- **AC5**: No `additionalContext` field, no `permissionDecision` field, no
  hook-response mutation of any kind originates from the collector call
  site. Verified structurally: a unit test that captures the
  UserPromptSubmit hook response before-and-after the collector call and
  asserts byte-equality.
- **AC6**: No outbound HTTP/network call originates from the collector
  module. Verified structurally: a text search of the module for
  `reqwest`, `ureq`, `hyper`, `http::`, `Client::`, `connect`, `bind`,
  `TcpStream`, or `UdpSocket` returns zero hits in non-test code.
- **AC7**: Seed rule set is committed with an inline comment noting that
  the labels are starter guesses and not committed policy.
- **AC8**: README (or CLAUDE.md) updated with a short note that the
  collector is observation-only and the smart matcher will be revisited
  in kete. Note length: 2–4 sentences, no commitments beyond what this
  issue ships.
- **AC9**: `cargo test` passes. The full test count is ≥ the current
  baseline plus the new tests added by this module. Existing tests must
  continue to pass unchanged.

## Open questions (for design-doc round)

1. **Receipt storage**: are PromptMatch receipts written to the same JSONL
   audit log as other events, or a separate prompt-specific log? The
   issue body implies the same log (CLI filters by `--event=PromptMatch`).
   Recommended: same log, additive event kind.
2. **`did_any_tool_call_follow` population**: set at PostToolUse correlation
   time using the existing session-state mechanism, OR left as `null` in
   v1 and populated in a follow-up issue? The brief's "follow-up" framing
   suggests the latter is acceptable. Recommended: populate at PostToolUse
   using the existing session-state mechanism — the correlation is cheap
   (it's the same lookup the compliance verdict already does).
3. **Prompt-text retention in the receipt**: include a truncated
   `prompt_preview` (matching the firing pipeline's existing 200-char
   bound) or just the hash? Hash-only is the strict privacy reading. The
   firing pipeline already retains previews. Recommended: hash only for
   v1 — the existing preview pipeline serves the "what was the user
   asking?" query for tool-firing receipts; PromptMatch receipts can
   stay strictly hash-only since their purpose is corpus-building, not
   ad-hoc debugging.
4. **Rule storage format**: static Rust slice in the module, OR a small
   TOML file loaded from `~/.taniwha/arai/prompt_rules.toml`? Static
   slice is simpler for v1 (no parser, no file IO at startup); a TOML
   file enables user customisation without recompiling. Recommended:
   static slice for v1, with a TODO comment for the file-loaded path
   if the dogfood phase reveals demand.
