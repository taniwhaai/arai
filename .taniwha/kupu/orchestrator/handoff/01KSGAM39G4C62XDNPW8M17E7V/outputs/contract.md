---
module: prompt-collector
version: 1
parent_design_version: 2
---

# Manifest: prompt-collector

## Responsibility

Tests an ordered set of labelled regex patterns against a given prompt text and returns one `PromptMatchReceipt` value per matched pattern, in pattern-set order, along with a count of patterns that were skipped because their regex string was invalid.

## Not responsible for

Writing receipts to any audit log, mutating the hook response in any form, performing any file I/O, performing any network I/O, or populating the `did_any_tool_call_follow` field beyond its initial `null` value.

## Inputs

- **`prompt_text`** (text string, required): The full user prompt string to match against. No length limit is imposed by this module. The caller owns truncation for any preview purposes; this module does not produce previews and does not shorten the text before matching.
- **`rules`** (ordered list of `PromptRule` records, required): The set of patterns to test, in evaluation order. May be empty (zero rules produces zero receipts with no error). The caller is responsible for providing a deduplicated list; this module does not deduplicate.
- **`project_slug`** (text string, required): The project identifier. Passed through verbatim into each returned receipt as `project_slug`. Not interpreted or validated by this module beyond being non-null.
- **`timestamp_iso`** (text string, required): An ISO-8601 timestamp of the prompt event provided by the caller. Passed through verbatim into each returned receipt as `timestamp_iso`. Using the caller's clock ensures receipt timestamps are consistent with the rest of the caller's audit entries.

## Outputs

- **`receipts`** (ordered list of `PromptMatchReceipt` records): Zero or more receipts, one per rule that matched, in the order matching rules appear in the input `rules` list. The list is empty when no rule matches, or when the input `rules` list is empty. Multiple rules may match a single prompt; each produces its own independent receipt.
- **`skipped_count`** (non-negative integer): The number of rules skipped because their `pattern` field was not a valid regex. Zero is the normal case. Non-zero is a signal for operator attention but does not prevent the remaining rules from being evaluated or the matching receipts from being returned.

## Side effects

None. This module is a pure computation: it tests patterns and constructs receipt values. It does not write to the audit log, does not write to any file, does not perform any network call, and does not modify any of its inputs or any shared mutable state.

## Error semantics

- **Invalid regex pattern in a `PromptRule`:** The rule is silently skipped. No receipt is emitted for it. `skipped_count` in the return value is incremented by one for each such rule. This is not a fatal condition; the remaining rules continue to be evaluated normally. The caller may log or surface `skipped_count` as operator-facing information, but the module does not do so itself.
- **No other failure modes exist in v1.** The module performs no I/O and no operations that can meaningfully fail at the scale of this module's inputs.

## Behavioural guarantees

- **Idempotency:** Yes, unconditionally. Given the same `prompt_text` and `rules`, the module always returns the same `receipts` list (same count, same field values, same order) and the same `skipped_count`. No external state affects the output.
- **Ordering:** The returned `receipts` list preserves the order of the input `rules` list. If rule at index _i_ matches and rule at index _j_ matches, and _i_ < _j_, then the receipt for rule _i_ appears before the receipt for rule _j_ in the output list.
- **Atomicity:** Not applicable. The module produces no persistent side effects. On any failure pathway (none exist in v1), there is no partial state to roll back.
- **Concurrency:** Safe under concurrent invocation. The module holds no mutable shared state. Concurrent calls with different or identical inputs do not interfere with each other.
- **Resource bounds:** Memory consumption is bounded by `O(number of matching rules)` for the returned receipt list plus `O(size of prompt_text)` for the matching operation. No allocations accumulate across calls.
- **Pattern matching semantics:** A pattern matches if the regex finds at least one match anywhere in `prompt_text` (substring / full-text match). Patterns are not implicitly anchored to the start or end of the text unless the pattern string itself includes anchoring syntax. There is no fuzzy matching, no embedding lookup, and no LLM call — regex only.
- **Hash determinism:** The `prompt_hash` field in every receipt is the lowercase hex-encoded SHA-256 digest of the complete, untruncated `prompt_text`. Two calls with byte-identical `prompt_text` values produce byte-identical `prompt_hash` values in all returned receipts.
- **No deduplication:** If the same rule appears more than once in the input `rules` list, each occurrence is evaluated independently. If both occurrences match, both produce independent receipts. The caller is responsible for providing a deduplicated rule list if deduplicated output is required.

## Dependencies

- **Regex engine** (provided via the project's existing dependency set, already declared in the project's build manifest): used to compile `PromptRule.pattern` strings at call time and to test compiled patterns against `prompt_text`. No new external dependency is introduced by this module.
- **SHA-256 hash function** (provided via the project's existing dependency set): used to compute `prompt_hash` from `prompt_text`. No new external dependency is introduced by this module.

## Referenced data shapes

### PromptRule

A single labelled pattern entry in the rules list passed as input.

- **`pattern`** (text string, required): A regex string. If the string is not a valid regex at evaluation time, the rule is skipped and `skipped_count` is incremented. No pre-validation of `pattern` is performed before the call; validation happens at call time during rule evaluation.
- **`label`** (text string, required, non-empty): A human-readable category name (e.g., `"deploy"`, `"secret"`). Copied verbatim as `matched_label` into any receipt this rule produces.

### PromptMatchReceipt

A single record returned when one `PromptRule` matches the prompt text. This record's structure maps directly onto the JSONL line that the caller will write to the audit log.

- **`event`** (text string): Always the literal value `"PromptMatch"`. Set by this module; never varies.
- **`prompt_hash`** (text string): The lowercase hex-encoded SHA-256 digest of the full, untruncated `prompt_text`. A valid SHA-256 hex digest is exactly 64 lowercase hexadecimal characters. The raw prompt text is not included in this record.
- **`matched_label`** (text string, non-empty): Copied verbatim from `PromptRule.label` for the rule that matched.
- **`timestamp_iso`** (text string): The ISO-8601 timestamp passed in by the caller as `timestamp_iso`. Copied verbatim; not validated or reformatted by this module.
- **`project_slug`** (text string): The project identifier passed in by the caller as `project_slug`. Copied verbatim.
- **`did_any_tool_call_follow`** (nullable boolean): Always `null` when returned by this module in v1. Population of this field with a non-null value is the responsibility of the caller's PostToolUse path and is outside the scope of this module.

## Acceptance criteria

**AC1 — Seed ruleset is non-empty and contains all declared labels:**
- Call the module's rule-loader (the function or constant that exposes the compiled-in seed rule set). Assert the returned collection is non-empty. Assert the count equals the number of declared seed rules. Assert that each of the following labels is present among the seed rules: `"deploy"`, `"production"`, `"secret"`, `"password"`, `"kubectl apply"`, `"force push"`. The test fails if any seed rule is removed without updating both the seed definition and this assertion.

**AC2 — UserPromptSubmit handler invokes the collector:**
- Construct a synthetic `UserPromptSubmit` hook invocation with a prompt text that matches at least one seed rule. Drive the full handler path with an in-memory sink replacing the live `record_event` call. Assert that the in-memory sink received at least one `PromptMatch` event for the matched rule. The test must verify the collector is actually called on the `UserPromptSubmit` path, not merely that the collector function works in isolation.

**AC3 — Receipt shape is exact:**
- From the receipt captured in AC2 (or an equivalent isolated call), inspect each field. Assert: `event` equals the literal string `"PromptMatch"`; `prompt_hash` is exactly 64 lowercase hexadecimal characters; `matched_label` is a non-empty string; `timestamp_iso` is a valid ISO-8601 string; `project_slug` is non-empty; `did_any_tool_call_follow` is `null`.

**AC4 — `arai audit --event=PromptMatch` filters correctly:**
- Write a fixture JSONL audit log containing at least one record with `event: "PromptMatch"` and at least one record with a different event kind (e.g., `"Compliance"`). Run the live `arai audit --event=PromptMatch` command against the fixture. Assert that every output line carries `event: "PromptMatch"` and that no line from the other event kind appears.

**AC5 — No hook-response mutation:**
- Construct a synthetic `UserPromptSubmit` hook invocation. Capture the serialised hook-response value as a byte sequence before the collector call. Execute the full call site including all `record_event` calls. Capture the serialised hook-response value as a byte sequence after. Assert byte-equality between the two captured sequences. This test must be located at the hook call site, not inside the collector module, because the contract being tested is about the call site's behaviour.

**AC6 — No outbound network calls in the collector source:**
- Verify that the collector source file contains none of the following identifiers outside of test-only blocks: `reqwest`, `ureq`, `hyper`, `http::`, `Client::`, `connect`, `bind`, `TcpStream`, `UdpSocket`. This check may be implemented as a test that reads the source file and scans it, or as a CI grep step. The test passes when no matches are found outside test-only blocks.

**AC7 — Seed ruleset is annotated as non-policy:**
- Verify that the source defining the seed-rule static collection contains a comment or annotation indicating the labels are starter guesses, not policy decisions. This may be verified by a structural grep in CI or by code review during the PR.

**AC8 — Documentation updated:**
- Verified by reviewer during PR review; not automated. The relevant README or project-instruction file must include a note of 2–4 sentences, framed as observation-only, referencing the kete charter boundary without making commitments beyond this issue.

**AC9 — Full test suite passes:**
- `cargo test` exits with code zero after all new tests are added. No existing test may be removed or have its assertion weakened.

**Additional required correctness tests (not ACs, no pass/fail gate number, but required for correctness):**

- Empty rules list: call the collector with an empty rules list and any non-empty prompt text; assert `receipts` is empty and `skipped_count` is zero.
- Single matching rule: call the collector with one rule whose pattern matches the prompt text; assert exactly one receipt is returned with the correct `matched_label`.
- Single non-matching rule: call the collector with one rule whose pattern does not match the prompt text; assert `receipts` is empty.
- Multiple rules, partial match: call the collector with three rules where exactly two match; assert exactly two receipts are returned in the order the matching rules appear in the input list.
- Regex metacharacters: a rule whose pattern contains regex metacharacters (for example a word-boundary anchor or escaped characters) compiles without error and matches prompt text correctly.
- `prompt_hash` determinism: call the collector twice with byte-identical inputs; assert that `prompt_hash` in every returned receipt is byte-identical across both calls.
- Invalid regex skipped without error: supply one rule whose `pattern` is `"[invalid"` (malformed regex). Assert the call completes without signalling a fatal error, assert the returned `receipts` list does not include a receipt for that rule, and assert `skipped_count` equals one.

## Out of scope

The following are explicitly excluded from this module's contract and must not be implemented as part of the collector:

- Writing receipts to the audit log. The caller calls `record_event` after inspecting the returned list.
- Mutating the hook response in any form, at any point in the call sequence.
- Populating `did_any_tool_call_follow` with a non-null value. That is the caller's PostToolUse path responsibility.
- Any form of enforcement: blocking, warning, or injecting content into the hook response.
- Fuzzy matching, embedding-based matching, LLM-based matching, or any non-regex matching strategy.
- Any outbound network call, remote aggregation, or telemetry egress.
- Storing, logging, or returning the raw prompt text. The receipt carries only the SHA-256 hash.
- Loading rules from a file, a database, or any external source. The seed ruleset is a static collection compiled into the binary. File-loaded rules are a follow-up concern not in scope for v1.
- Any schema migration of the audit log or the rule store.
- Any CHANGELOG update beyond adding an `[Unreleased]` entry in the project changelog.
- Introducing new build-time dependencies not already present in the project's build manifest.
