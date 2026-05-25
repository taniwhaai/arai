---
version: 2
parent_brief_version: 4
tier: single_module
---

# Prompt-text collector (issue #113)

## Structural tier

**Selected:** single_module

**Justification:** The brief explicitly mandates `single_module` and confirms the work fits a single new source file plus small additive wiring into two existing files (`src/hooks.rs`, `src/audit.rs`). All operations share the same data source (the prompt-rules set), serve the same external boundary (the local JSONL audit log), and no part of the system would be independently useful or independently swappable. Total scope is a few hundred lines.

**Module count:** 1 — `prompt-collector`. No composition layer; no vocabulary file.

## Purpose

The prompt-text collector observes user prompts in the `UserPromptSubmit` hook by matching prompt free-text against a stored set of labelled regex patterns, then writes one `PromptMatch` receipt per matched rule into the existing local audit log. It collects corpus data so that future smart-matcher work (scoped to the kete repo) can be evaluated against real prompt traffic rather than synthesised assumptions. It does not enforce, block, warn, or inject content into the hook response.

## External boundaries

- **stdin (inbound):** Prompt text delivered via the `UserPromptSubmit` hook JSON payload. Read-only; the module does not alter it.
- **local filesystem — audit log (outbound):** `PromptMatch` receipts appended to the per-project JSONL audit log via the existing `record_event` path in `src/audit.rs`. One record per matched rule per prompt invocation.
- **local filesystem — rule source (inbound):** The seed ruleset is compiled into the binary as a static slice. No external file is read in v1.

## Modules

### prompt-collector

**Responsible for:** Loading the prompt-rules set and, for a given prompt text, testing each pattern against that text, then returning zero or more `PromptMatchReceipt` values — one per matched rule.

**Not responsible for:** Writing receipts to the audit log (that is the caller's responsibility via `record_event`), mutating the hook response in any form, or performing any network I/O.

**Inputs:**
- `prompt_text` — the full user prompt string; required; no length limit imposed by this module (the caller owns truncation for any preview purposes, which this module does not produce).
- `rules` — the ordered set of `PromptRule` records to test against; required; may be empty (zero rules → zero receipts).
- `project_slug` — the project identifier string, passed through into each receipt; required.
- `timestamp_iso` — the ISO-8601 timestamp of the prompt event, provided by the caller so receipts are consistent with the caller's clock; required.

**Outputs:**
- An ordered list (possibly empty) of `PromptMatchReceipt` records, one per rule that matched, in the order the rules appear in the input set. Multiple rules may match a single prompt; each produces its own receipt.

**Side effects:**
- None. The module is pure: it tests patterns and constructs receipt values. All audit-log writes are performed by the caller after inspecting the returned list.

**Error semantics:**
- If a stored pattern string is not a valid regex, that rule is skipped silently and no receipt is emitted for it. The module signals how many rules were skipped via an integer count in its return value (zero is the normal case; non-zero is a signal for operator attention but not a fatal error). Skipping is the right behaviour because a single malformed seed rule must not prevent the remaining rules from matching.
- No other failure modes exist in v1 (the module performs no I/O, no network calls, no allocations that can meaningfully fail at this scale).

**Behavioural guarantees:**
- Deterministic: given the same `prompt_text` and `rules`, the module always returns the same list in the same order.
- Concurrent-safe: the module holds no mutable shared state; concurrent invocations do not interfere.
- No side effects: the module does not write to files, does not call the network, does not modify the hook response, and does not alter its inputs.
- Pattern matching is substring / full-text regex (not anchored unless the pattern itself anchors): a pattern matches if it finds at least one match anywhere in the prompt text.
- The module does not deduplicate receipts if the same rule appears more than once in the rule set; the caller is responsible for providing a deduplicated rule set.

**Dependencies:**
- None within the Arai module graph. The module depends only on the regex engine already present in Cargo.toml and on standard library primitives.

## Caller-site change in src/hooks.rs (UserPromptSubmit branch)

The `UserPromptSubmit` branch in `src/hooks.rs` already emits a domain-rule summary as `additionalContext` and constructs a hook response before returning. The collector is invoked **after** that response is fully constructed and **must not alter it**.

Call-site contract:

1. After the existing `additionalContext` construction is complete and the hook-response value is finalised, invoke `prompt_collector::collect(prompt_text, rules, project_slug, timestamp_iso)`.
2. For each `PromptMatchReceipt` in the returned list, call `audit::record_event` with the receipt serialised as a `PromptMatch` event. This is the only new side effect at the call site.
3. The return value of `collect` and the `record_event` calls are the only new statements. No field of the hook response struct is read, written, or replaced after the response is finalised.
4. If `record_event` signals an error, the error is logged locally (consistent with how the existing audit path handles write failures) and the hook response is returned unchanged. The prompt is never blocked because of a collector failure.

AC5 verification: a unit test constructs a synthetic `UserPromptSubmit` hook invocation, captures the serialised hook response before the collector call and after all `record_event` calls, and asserts byte-equality between the two captured values.

## Data shapes

### PromptRule

A single labelled pattern entry in the rule set.

- `pattern` — text value; a regex string; required; must be a valid regex at load time. If invalid, the rule is skipped (see error semantics above).
- `label` — text value; a human-readable category name (e.g. `"deploy"`, `"secret"`); required; non-empty. This value appears verbatim as `matched_label` in any receipt the rule produces.

### PromptMatchReceipt

A single audit record emitted when one rule matches a prompt. Maps directly onto the JSONL line written by `record_event`.

- `event` — text value; always the literal string `"PromptMatch"`; required.
- `prompt_hash` — text value; the lowercase hex-encoded SHA-256 digest of the full prompt text (before any truncation); required. Used as the corpus identifier; the text itself is not stored in the receipt.
- `matched_label` — text value; copied from `PromptRule.label`; required.
- `timestamp_iso` — text value; ISO-8601 timestamp provided by the caller; required.
- `project_slug` — text value; the project identifier provided by the caller; required.
- `did_any_tool_call_follow` — nullable boolean; `null` in v1 at the time of writing. Populated at `PostToolUse` time using the existing session-state correlation mechanism (same lookup the compliance verdict uses). See decision OQ2 below.

## Decisions made for v1

**OQ1 — Receipt storage (same log vs. separate log):** Receipts are written to the same per-project JSONL audit log as all other events, as an additive `PromptMatch` event kind. Justification: the brief states that `arai audit --event=PromptMatch` is the retrieval lever, which requires the receipts to be in the same log the `--event` filter operates on. A separate log would require a second reader path and would defeat the filter mechanism.

**OQ2 — `did_any_tool_call_follow` population:** The field is written as `null` at receipt-creation time in v1 and populated at `PostToolUse` time using the existing session-state correlation mechanism — the same lookup the compliance verdict already performs. Justification: the brief recommends this as "cheap" and the mechanism already exists. Implementing it in v1 avoids a follow-up migration that would need to rewrite already-written receipts. `null` is a valid initial state; a PostToolUse pass that finds a matching session entry will update the value in-place (or append a correlation record, consistent with the existing compliance pattern).

**OQ3 — Prompt-text retention in the receipt:** The receipt carries only the SHA-256 hash of the full prompt text (`prompt_hash`). No `prompt_preview` field is included in `PromptMatchReceipt`. Justification: the brief recommends hash-only for v1 — the existing firing-pipeline receipts already retain previews for the "what was the user asking?" use case; `PromptMatch` receipts serve corpus-building, not ad-hoc debugging, so the hash is sufficient. This is also the strictest reading of the "no prompt-text persistence beyond what already exists" prohibition.

**OQ4 — Rule storage format:** The seed ruleset is a static slice compiled into the binary. No TOML file, no file I/O at startup. A TODO comment in the source marks the file-loaded path as a follow-up if the dogfood phase reveals demand. Justification: static slice requires no new parser, no file-not-found error path, no additional startup latency, and no new crate dependency — consistent with the brief's prohibition on new Cargo dependencies.

## Out of scope

The following are taken verbatim from brief v4 and must not be implemented:

- **No enforcement.** Not block, not warn-via-additionalContext, not retry injection. The collector's call site in `src/hooks.rs` may not mutate the existing hook response in any way.
- **No matcher quality work.** No sweep of matchers, no embedding model, no LLM enrichment, no fuzzy match. Regex only.
- **No remote aggregation.** No HTTP egress, no Kete handoff, no telemetry beat. Same locality discipline as `src/session.rs`.
- **No prompt-text persistence beyond what already exists.** The receipt carries the hash. Whatever prompt-preview the existing firing pipeline already truncates (~200 chars) may be reused for the receipt's preview field; nothing longer.
- **No new crate dependency.** `regex`, `serde`, `serde_json`, `sha2`, `rusqlite` are already in Cargo.toml — use them. Do not add new deps.
- **No `prompt_rules` schema migration** beyond v1. The table format ships as the bare minimum; expansion (priority, scope, tool-coupling) belongs to the post-observation revisit.
- **No DB/storage refactor** of the audit log. The existing JSONL audit format admits new event kinds additively.
- **No CHANGELOG rewrite** of historical entries; add a [Unreleased] entry only.

Additionally implied by the chosen decisions above:
- No TOML-file rule loader in v1.
- No `prompt_preview` field on `PromptMatchReceipt` in v1.
- No `did_any_tool_call_follow` backfill mechanism; the `null`-to-value transition happens only via the PostToolUse session-state path.
- No smart-matcher, no embedding model, no LLM call — this belongs to kete per the Arai↔Kete charter boundary.

## Test surface

Each acceptance criterion maps to at least one test class. Tests that the brief implies but does not enumerate as ACs are noted explicitly.

**AC1 — Seed ruleset loads on fresh install:**
- Unit test (in `src/prompt_collector.rs` `#[cfg(test)] mod tests`): call the rule-loader, assert the returned slice is non-empty, assert the count equals the number of seeded rules, assert that each seeded label (`"deploy"`, `"production"`, `"secret"`, `"password"`, `"kubectl apply"`, `"force push"`) is present. Fails if any seed rule is removed without updating the test.

**AC2 — UserPromptSubmit handler invokes collector:**
- Unit test (in `src/hooks.rs` tests or a dedicated integration test): construct a synthetic `UserPromptSubmit` hook invocation with a prompt that matches at least one seed rule. Drive it through the handler with a captured-audit test double (an in-memory sink replacing `record_event`). Assert that the test double received at least one `PromptMatch` event for the matched rule.

**AC3 — Receipt shape is exact:**
- Unit test (co-located with AC2 or as a separate assertion): inspect the receipt emitted by AC2's test double. Assert the presence and non-emptiness of all six required fields: `event` equals `"PromptMatch"`, `prompt_hash` is a 64-character lowercase hex string (SHA-256), `matched_label` is a non-empty string, `timestamp_iso` is a valid ISO-8601 string, `project_slug` is non-empty, `did_any_tool_call_follow` is `null`.

**AC4 — `arai audit --event=PromptMatch` filters correctly:**
- Integration test (under `tests/`): write a fixture JSONL audit log containing at least one `PromptMatch` event and at least one event of a different kind (e.g. `Compliance`). Run the live `arai audit --event=PromptMatch` binary against the fixture. Assert that all output lines have `event: "PromptMatch"` and that no lines from the other event kind appear.

**AC5 — No hook-response mutation (structural verification):**
- Unit test: construct a synthetic `UserPromptSubmit` invocation, serialise the hook response to bytes before the collector call, run the full call site (including all `record_event` calls), serialise the hook response to bytes after. Assert byte-equality of the two serialisations. This test must live at the hook call site, not inside the collector module itself, since the contract is about the call site's behaviour.

**AC6 — No outbound network call (structural verification):**
- Structural grep (in CI or as a `#[test]`-gated shell check): search the text of `src/prompt_collector.rs` for the strings `reqwest`, `ureq`, `hyper`, `http::`, `Client::`, `connect`, `bind`, `TcpStream`, `UdpSocket`. Assert zero matches outside of `#[cfg(test)]` blocks. This can be implemented as a `#[test]` that reads its own source file at a known relative path and scans it, or as a CI grep step.

**AC7 — Seed ruleset annotated as non-policy:**
- Unit test (or structural grep): assert that the source of the seed-rule static slice contains the inline comment noting the labels are starter guesses. Alternatively, a code-review gate; the brief does not require automation here, but a simple `grep` assertion in CI makes the invariant machine-checked.

**AC8 — README / CLAUDE.md updated:**
- Verified by reviewer during PR review; not automated. The note must be 2–4 sentences, observation-only framing, and must reference the kete charter boundary without making commitments beyond this issue.

**AC9 — `cargo test` passes with ≥ baseline + new test count:**
- Automated: run `cargo test` in CI. Assert exit code zero. The new test count is the sum of all tests added by this module. Existing tests must remain unchanged.

**Additional implied tests (not ACs, but required for correctness):**

- Empty rules → zero receipts: call `collect` with an empty rule slice and any prompt text; assert the returned list is empty.
- Single rule + matching prompt → one receipt: call `collect` with one rule whose pattern matches; assert exactly one receipt is returned with the correct label.
- Single rule + non-matching prompt → zero receipts: call `collect` with one rule whose pattern does not match; assert an empty list.
- Multiple rules + multi-match → multiple receipts: call `collect` with three rules where two match; assert exactly two receipts in rule-order.
- Regex special characters handled: a rule whose pattern contains regex metacharacters (e.g. `\bkubectl\b`) compiles and matches correctly.
- `prompt_hash` determinism: calling `collect` twice with identical inputs produces byte-identical `prompt_hash` values in all returned receipts.
- Invalid regex in rule set is skipped: a rule with pattern `"[invalid"` does not panic; the returned list excludes that rule's receipt; the skip-count return value is 1.
