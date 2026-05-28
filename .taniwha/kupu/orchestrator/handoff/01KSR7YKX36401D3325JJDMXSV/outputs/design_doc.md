---
version: 3
parent_brief_version: 5
tier: single_module
---

# hooks-grok-exit

## Structural tier

**Selected:** single_module

**Justification:** The brief is explicitly scoped to a single source file (`src/hooks.rs`) with no new modules, no new crate dependencies, and no changes to the match pipeline, parser, or store. The change is one coherent capability — threading a desired exit code from the deny-decision site through to the process boundary — and all of the affected logic shares the same state and control flow already present in that file. No independent swappable concern is introduced.

**Module count:** 1

## Purpose

`hooks-grok-exit` extends the side-effecting hook entry point (`handle_stdin`) so that, when a Block-severity deny is emitted to a Grok TUI host, the process exits with code 2 (Grok's canonical "explicit deny" signal) rather than code 0. On every other path — allow, non-blocking hook event, or a host other than Grok — the process exits 0 as before. On the internal-error path, the same host-awareness applies: a Grok host emits Grok-shaped deny JSON and exits 2; a Claude host keeps the existing Claude-shaped deny JSON and exits 0. Claude's behaviour is unchanged throughout because Claude treats any non-zero exit as "hook broken" (fail-open).

## External boundaries

- **stdin**: inbound, byte stream — the serialised hook payload (JSON) delivered by the host (Grok TUI or Claude Code) at hook invocation time.
- **stdout**: outbound, byte stream — the serialised decision JSON read by the host to determine whether the tool call proceeds.
- **environment variables**: inbound, key-value pairs — `GROK_HOOK_EVENT` and `GROK_SESSION_ID` are read by the existing `detect_host` function to determine which host is active; no new environment variables are introduced.
- **process exit code**: outbound, integer — the value the host uses as its primary deny/allow signal; this boundary is the sole new external boundary introduced by this slice.

## Modules

### hooks-grok-exit

**Responsible for:** Computing the desired process exit code from the deny/allow outcome and the detected host, then applying that code at the process boundary inside the side-effecting entry point.

**Not responsible for:** Matching rules against hook payloads, emitting decision JSON to stdout, detecting the host, writing audit records, or any behaviour on the Claude Code path (all of these remain unchanged).

**Inputs:**

- `hook_payload`: the serialised JSON string read from stdin, passed to the existing match and decision pipeline unchanged.
- `host`: derived from the existing `detect_host` call on the hook payload; distinguishes `Grok` from `Claude` (and any other future variant).
- `event_type`: whether the hook event is `PreToolUse`, `PostToolUse`, or `UserPromptSubmit`, derived from the parsed payload.
- `blocking`: boolean indicating whether the current event is a blocking hook (PreToolUse is blocking; PostToolUse and UserPromptSubmit are not).
- `deny_outcome`: boolean indicating whether the match pipeline produced a Block-severity deny for this invocation.

**Outputs:**

- `desired_exit_code`: integer — `2` when `host == Grok AND event_type == PreToolUse AND deny_outcome == true`; `0` in all other cases. This value is the sole new output produced by this module; it is consumed immediately by the process-exit side effect.

**Side effects:**

- `process exit with code 2`: emitted by the side-effecting entry point when `desired_exit_code == 2`; this is the new side effect introduced by this slice.
- `process exit with code 0`: the existing behaviour on all allow, non-blocking, and non-Grok paths; unchanged.
- `stdout JSON emission`: the existing `emit_grok_decision` / `emit_claude_decision` calls are unchanged and continue to precede the exit; the stdout channel is not modified by this slice.
- `audit write`: existing; unchanged; side effects live only on the `handle_stdin` path; this is not modified.

**Error semantics:**

- On an internal error (oversize or malformed stdin, database lock, or any other condition that reaches the existing fail-closed error branch): if `detect_host` returns `Grok`, emit Grok-shaped deny JSON to stdout and exit with code 2. If `detect_host` returns any other host, retain the existing behaviour (emit Claude-shaped deny JSON, exit 0). The fail-closed guarantee is thereby preserved for Grok: an error that cannot be diagnosed still produces the safe outcome (deny + exit 2) rather than silently allowing the tool call.
- The computation of `desired_exit_code` itself does not signal errors; it is a pure function of its inputs.

**Behavioural guarantees:**

- The match pipeline is not invoked on the error path and is not modified; its behaviour is identical to today on every path.
- `desired_exit_code` is computed after all stdout JSON has been written; the stdout channel is always flushed before the process exits.
- `process::exit` is called only from the side-effecting entry point (`handle_stdin`); the inner implementation function that computes the desired exit code returns a value rather than calling `process::exit` directly, preserving the existing architectural rule that side effects live only on `handle_stdin`.
- Concurrent invocations are not in scope; the hook binary is invoked as a short-lived process per hook event.
- The hot-path timing guarantee (approx. 22 ms skip-exit, approx. 32 ms full match) is unaffected; the exit-code computation is O(1) on existing data.

**Dependencies:**

- Existing `detect_host` function within `src/hooks.rs` — reads environment variables, returns the active host variant.
- Existing `emit_grok_decision` function within `src/hooks.rs` — writes Grok-shaped JSON to stdout.
- Existing `emit_claude_decision` function within `src/hooks.rs` — writes Claude-shaped JSON to stdout.
- No dependencies outside `src/hooks.rs`; no new crate dependencies.

## Data shapes

### DesiredExitCode

An integer value representing the process exit code to be applied after all stdout output has been written.

| Value | Meaning |
|-------|---------|
| `0`   | Allow, or non-blocking event, or Claude host on any path |
| `2`   | Explicit deny on a Grok host PreToolUse path (happy path or error path) |

No other values are produced by this module.

## Acceptance criteria

- **AC1:** PreToolUse + Block-rule match + `GROK_HOOK_EVENT` set in environment → stdout contains `"decision":"deny"` AND process exit code is 2.
- **AC2:** PreToolUse allow under Grok (no Block match) → process exit code is 0.
- **AC3:** PreToolUse Block deny under Claude (neither `GROK_HOOK_EVENT` nor `GROK_SESSION_ID` set) → process exit code is 0 AND stdout contains `"permissionDecision":"deny"` (regression guard; Claude must never receive a non-zero exit).
- **AC4:** Induced internal error (oversize or malformed stdin) on PreToolUse under Grok → stdout contains Grok-shaped `"decision":"deny"` AND process exit code is 2 (fail-closed).
- **AC5:** Same induced internal error under Claude → process exit code is 0 AND stdout contains Claude-shaped deny (unchanged behaviour).
- **AC6:** PostToolUse or UserPromptSubmit under Grok → process exit code is 0 regardless of rule matches.
- **AC7:** Full existing test suite passes with no regressions; hot-path timing is unaffected.

## Out of scope

- Changes to any file other than `src/hooks.rs`.
- New abstractions, new traits, new modules, or new crate dependencies.
- Changes to the match pipeline, rule parser, store, audit, telemetry, or session layers.
- Host detection logic: `detect_host` is reused as-is; this slice adds no new environment variables and changes no host-detection heuristics.
- Tool-name normalisation, `.grok/hooks/arai.json` injection, and AGENTS.md discovery (already merged on `main` as part of the broader #122 work).
- Any exit-code handling beyond code 0 and code 2; no other non-zero values are produced.
- Changes to the Grok or Claude JSON payload shapes emitted to stdout.
- Behaviour on hypothetical future host variants beyond Grok and Claude; the existing `detect_host` return type governs those.
