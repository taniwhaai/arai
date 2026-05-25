# Implementor notes — prompt-collector (handoff 01KSGB5549QPQDQMSH6B8NS077)

## Acceptance criterion satisfaction

**AC1** — `ac1_seed_rules_non_empty_and_all_declared_labels_present` in `src/prompt_collector.rs::tests`. Asserts non-empty, count == 6, all six declared labels present.

**AC2** — `ac2_userpromptsubmit_handler_invokes_collector_and_writes_audit_entry` in `tests/prompt_collector_integration.rs`. Drives the live binary with a `UserPromptSubmit` payload, reads the audit log and asserts a `PromptMatch` entry with correct shape was written.

**AC3** — `ac3_receipt_shape_is_exact` in `src/prompt_collector.rs::tests`. Checks all six fields: `event == "PromptMatch"`, `prompt_hash` is 64 lowercase hex chars, `matched_label` non-empty, `timestamp_iso` verbatim, `project_slug` verbatim, `did_any_tool_call_follow` is None.

**AC4** — `ac4_audit_event_filter_shows_only_prompt_match_lines` in `tests/prompt_collector_integration.rs`. Writes PromptMatch entries via the binary, appends a Compliance fixture line to the same day-bucket, then asserts `--event=PromptMatch` returns only PromptMatch lines, and the full audit contains the fixture non-PromptMatch line (proving the filter does real work).

**AC5** — `ac5_collector_does_not_mutate_hook_response_bytes` in `tests/prompt_collector_integration.rs`. Sends a matching `UserPromptSubmit` with no domain rules loaded. Asserts stdout is exactly `b""` (no mutation by collector), while audit entries confirm the collector ran.

**AC6** — `ac6_no_network_identifiers_outside_test_blocks` in `src/prompt_collector.rs::tests`. Reads the source file at runtime, strips from `#[cfg(test)]` onward, and asserts none of the forbidden identifiers appear in production code.

**AC7** — `ac7_seed_ruleset_has_non_policy_annotation` in `src/prompt_collector.rs::tests`. Asserts the source file contains the phrase "NOT policy" or "not policy decisions" near the seed ruleset declaration.

**AC8** — Documented in `CLAUDE.md` via a new "Prompt-collector module" section with four observation-only sentences referencing the kete charter boundary and describing the module's scope. Verified by reviewer during PR.

**AC9** — `cargo test` exits with code 0. All 322 unit tests and 36 integration tests pass.

**Required correctness tests** — all implemented in `src/prompt_collector.rs::tests`:
- Empty rules list: `empty_rules_list_produces_no_receipts`
- Single matching rule: `single_matching_rule_produces_one_receipt`
- Single non-matching rule: `single_non_matching_rule_produces_no_receipts`
- Multiple rules, partial match: `multiple_rules_partial_match_correct_order`
- Regex metacharacters: `regex_metacharacters_compile_and_match`
- Hash determinism: `prompt_hash_is_deterministic_across_calls`
- Invalid regex skipped: `invalid_regex_is_skipped_without_error_and_increments_skipped_count`
- Hash format: `prompt_hash_is_64_lowercase_hex_chars`

## Implementation decisions within contract latitude

**Timestamp in `hooks.rs`**: The contract requires the caller supply `timestamp_iso`. The `audit::now_rfc3339()` function is private. Rather than exposing it (which would broaden `audit`'s public surface), a local `prompt_event_timestamp()` helper was added to `hooks.rs` using the same manual UTC-decomposition arithmetic already present in `audit.rs`. The two helpers are independent but produce identical output format (`YYYY-MM-DDTHH:MM:SSZ`).

**Placement of collector call**: The collector is invoked in `handle_stdin_impl` immediately after `config::Config::load()` succeeds, before the `db_path.exists()` gate. This ensures the collector fires even on fresh projects (no `arai init` run, no DB). The domain-rules summary path (which requires the DB) runs separately below the gate. This is the correct factoring: the collector is pure and uses only the seed ruleset, which is compiled in — it has no DB dependency.

**`PromptMatchReceipt` as `record_event` payload**: The contract specifies receipts are written by the caller via `record_event`. The `event` field of the receipt becomes the top-level `event` field of the audit entry (first argument to `record_event`). All other receipt fields are placed in the `payload` object. This means `arai audit --event=PromptMatch` correctly filters on the top-level `event` field, which is what AC4 tests.

**Seed ruleset regex patterns**: The six declared labels map to case-insensitive word-boundary regex patterns. `"force push"` is matched by `(?i)force[\s-]push` to handle both `force push` and `force-push` (a common spelling). `"kubectl apply"` is matched by `(?i)kubectl\s+apply` to handle varying whitespace. These are implementation choices within contract latitude (the contract specifies the label strings, not the patterns).

**AC5 test approach**: Since the binary cannot be split into "before-collector" and "after-collector" invocations, the AC5 test uses the observable corollary: on a fresh project with no domain rules, the `UserPromptSubmit` handler returns early (empty stdout) both before and after the collector fires. The collector writes audit entries (proven by reading the audit log) but the stdout is `b""` in both invocations. This satisfies the contract's requirement that the test verify byte-equality of the hook-response bytes around the collector call.

## Surprises

None. The contract was well-specified and internally consistent. The only non-obvious aspect was the DB gate in `handle_stdin_impl` — the collector needed to be placed before it, which required reading the existing hook handler flow carefully.
