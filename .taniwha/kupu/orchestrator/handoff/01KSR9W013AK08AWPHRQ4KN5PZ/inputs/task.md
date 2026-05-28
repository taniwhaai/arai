# Task — leaf-implementation (module: hooks-grok-exit)

Implement module **hooks-grok-exit** against contract v1 (`inputs/contract.md`).

## TARGET FILE — edit in place
`/home/matt/r/arai/src/hooks.rs` — an **existing 1561-line file**. Do NOT create a
greenfield file. Modify it in place. ALL changes must be confined to `src/hooks.rs`.

## What to change (the contract's non-normative notes give the edit points)
1. `handle_stdin` (side-effecting entry point, ~line 351): apply a desired exit
   code after `handle_stdin_impl` returns. Flush stdout. If the code is 2, call
   `process::exit(2)`; otherwise return `Ok(())`. On the error branch: if host is
   Grok, write Grok-shaped deny JSON before exiting 2; if not Grok, keep the
   existing Claude-shaped deny JSON then exit 0. The clean approach: have
   `handle_stdin_impl` return its desired exit code as its `Ok` payload
   (`Result<i32, String>`), computed at the emit site where host/event/deny are
   already in scope (~line 866). The ~15 existing skip/observability/allow
   `Ok(())` returns become `Ok(0)`.
2. A new **pure** helper for the truth table: `(host, event, deny_outcome) →
   2 iff Grok && PreToolUse && deny; else 0`. No side effects, no I/O, no
   `process::exit`.

## Hard constraints
- `process::exit(2)` may be called ONLY from `handle_stdin`, never the pure helper.
- No new crate dependencies; do not modify `Cargo.toml`. `std::process` already in use.
- Claude host (AC3, AC5): exit code ALWAYS 0 — hard regression guard.
- stdout content byte-for-byte unchanged on every exit-0 path.
- Reuse `detect_host`, `emit_grok_decision`, `emit_claude_decision` as-is. On the
  error path the payload may be unparsed — call `detect_host(&Value::Null)` (or an
  empty/default `Value`) for env-only host detection.

## Acceptance criteria (AC1–AC7 — full text in contract.md)
AC1 Grok PreToolUse Block deny → stdout `"decision":"deny"` + exit 2.
AC2 Grok PreToolUse allow → exit 0.
AC3 Claude PreToolUse Block deny → exit 0 + `"permissionDecision":"deny"`.
AC4 Grok PreToolUse induced error → Grok-shaped deny + exit 2.
AC5 Claude induced error → exit 0 + Claude-shaped deny.
AC6 Grok PostToolUse/UserPromptSubmit → exit 0.
AC7 `cargo test` full suite passes, no regression.

## Tests
Add tests for AC1–AC6 to the `#[cfg(test)] mod tests` block at the bottom of
`src/hooks.rs`. Table-driven unit tests for the pure helper cover the pure-function
ACs. Env-var-dependent tests (GROK_HOOK_EVENT) must not race — guard with serial
execution (the repo's existing test convention; check what's already used in the
file/Cargo.toml — do NOT add a new dependency). Build & test with the project
toolchain: `cargo build`, `cargo test`.

## Output
Write `implementation_manifest.md` to the handoff `outputs/` directory describing
the functions changed (names + line ranges), the test approach, and confirmation
that `cargo test` passed (paste the summary line). Emit `re_raise.yaml` instead
ONLY if the contract has genuine ambiguity you cannot resolve.
