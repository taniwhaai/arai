# Manifest: hooks-grok-exit

## Responsibility

Extend the side-effecting hook entry point so that a Block-severity deny emitted to a Grok TUI host causes the process to exit with code 2, while every other path (allow, non-blocking event, Claude host, error on Claude host) continues to exit with code 0.

## Not responsible for

Matching rules against hook payloads, emitting decision JSON to stdout, detecting the host, writing audit records, or any behaviour on the Claude Code path — all of these remain unchanged and are delegated to existing functions.

## Inputs

- **stdin byte stream** (raw bytes, required): the serialised JSON hook payload delivered by the invoking host at hook invocation time. Read by the existing `handle_stdin` / `handle_stdin_impl` machinery; this module does not change how it is read. Hard cap: 1 MiB (1 048 576 bytes). Payloads that exceed this cap, fail UTF-8 decoding, or fail JSON parsing are routed to the error path described under Error semantics.
- **`hook_event_name` field** (string, optional within JSON): the event type carried in the parsed JSON payload. Recognised values: `"PreToolUse"`, `"PostToolUse"`, `"UserPromptSubmit"`, and the existing observability events (`"FileChanged"`, `"InstructionsLoaded"`, `"CwdChanged"`, `"PostToolBatch"`, `"PermissionDenied"`). Unrecognised values leave the internal event hint at its safe default `"PreToolUse"` (existing behaviour, unchanged). Only `"PreToolUse"` interacts with the new exit-code logic.
- **`GROK_HOOK_EVENT` environment variable** (string, optional): presence of this variable (any value) indicates the invoking host is Grok TUI. Read by the existing `detect_host` function, which is reused without modification.
- **`GROK_SESSION_ID` environment variable** (string, optional): presence of this variable (any value) also indicates the invoking host is Grok TUI. Read by the existing `detect_host` function, which is reused without modification. Either variable being set is sufficient to identify a Grok host.
- **deny/allow outcome** (derived, not a raw input): whether the match pipeline produced a Block-severity deny for this invocation. Derived from the result of the existing `highest_severity` / `match_hook` pipeline. This module does not change how that outcome is computed.

## Outputs

- **`desired_exit_code`** (integer): the process exit code to apply after all stdout output has been written and flushed. Produced by a pure computation (no I/O, no side effects) with the following mapping:

  | Condition | Value |
  |-----------|-------|
  | `host == Grok` AND `event_type == PreToolUse` AND `deny_outcome == true` | `2` |
  | All other cases (allow, non-blocking event, non-Grok host, any PostToolUse or UserPromptSubmit) | `0` |

  No other integer values are produced. The value is consumed immediately by the process-exit side effect inside `handle_stdin`; it is not written to stdout, the audit log, or any other channel.

- **stdout JSON** (byte stream, unchanged on happy path): the existing `emit_grok_decision` or `emit_claude_decision` output, written before the process exits. This module does not change the content of this output on the happy path.

## Side effects

- **Process exit with code 2**: new side effect. Emitted by `handle_stdin` when `desired_exit_code == 2` (Grok host, PreToolUse, Block deny). `process::exit(2)` is called only from `handle_stdin`; the inner function that computes `desired_exit_code` returns a value and never calls `process::exit` directly.
- **Process exit with code 0**: existing behaviour on all allow, non-blocking, and non-Grok paths. Unchanged.
- **stdout flush before exit**: stdout is always flushed before any `process::exit` call. No output is lost due to buffering.
- **Grok-shaped deny JSON on error path**: new side effect on the error branch only. When `handle_stdin_impl` returns an error and `detect_host` identifies the host as Grok, `emit_grok_decision(false, Some("Arai: internal error, blocking for safety"), "")` (or equivalent Grok-shaped JSON) is written to stdout before `process::exit(2)` is called. The specific reason string must include a human-readable indication that an internal error occurred.
- **stdout JSON on error path for Claude host**: unchanged. The existing Claude-shaped deny JSON (`permissionDecision: "deny"`) is written; process exits 0.
- **Audit write, telemetry, session state**: existing; unchanged; not modified by this slice.

## Error semantics

- **Oversize stdin** (payload exceeds 1 MiB): `handle_stdin_impl` signals an error to `handle_stdin`. Host detection is performed using `detect_host` on an empty/default JSON value (since the payload was not successfully parsed). If the host is Grok: write Grok-shaped deny JSON to stdout, flush, exit 2. If the host is not Grok: write Claude-shaped deny JSON to stdout (existing behaviour), flush, exit 0.
- **Malformed stdin** (non-UTF-8 bytes or invalid JSON): same routing as oversize stdin — error propagates to `handle_stdin`, host is detected, Grok exits 2 with Grok-shaped deny, non-Grok exits 0 with Claude-shaped deny.
- **Database or config failure after successful parse**: same routing. The event hint (`event_hint`) is set from the successfully parsed payload before the error; if the event hint is `"PreToolUse"` and the host is Grok, the process exits 2 with Grok-shaped deny.
- **`desired_exit_code` computation itself**: does not signal errors. It is a pure function of `host` (a known-variant enumeration) and two boolean flags; all inputs are always well-defined by the time the computation runs.
- **`process::exit` is not recoverable**: callers of `handle_stdin` do not observe a return value when exit code 2 is used. The `handle_stdin` function signature returns `Result<(), String>`; when the exit code is 2, `process::exit(2)` is called directly and the `Result` return is not reached. When the exit code is 0, the existing `Ok(())` return path is used.

## Behavioural guarantees

- **Idempotency**: the process is short-lived (one hook invocation, one exit); idempotency is not applicable.
- **Ordering**: stdout JSON is always written and flushed before `process::exit` is called, on every path (happy path, error path, exit 0, exit 2). A caller observing stdout before reading the exit code will always see the complete JSON.
- **Atomicity**: the module introduces no new partial-failure modes. The only new externally observable change is the exit code. If stdout write succeeds and `process::exit` is reached, the exit code is applied atomically from the OS perspective.
- **Concurrency**: concurrent invocations are not in scope. The hook binary is invoked as a short-lived process per hook event; the operating environment never runs two hook invocations in the same process.
- **Resource bounds**: the `desired_exit_code` computation is O(1) on existing in-memory data (host variant, event string, boolean deny flag). It allocates no heap memory and makes no external calls. The existing hot-path timing target (approximately 22 ms skip-exit, approximately 32 ms full match) is unaffected.
- **Claude host regression guarantee**: when neither `GROK_HOOK_EVENT` nor `GROK_SESSION_ID` is set (Claude Code path), the process exit code is always 0 on every path including the error path. Claude Code treats any non-zero exit as "hook broken" (fail-open); this guarantee must never be broken.
- **process::exit placement**: `process::exit` is called only from `handle_stdin`. The inner function `desired_exit_code` (or equivalent inline computation) returns an integer value and has no side effects. This preserves the existing architectural rule that all process-level side effects are confined to `handle_stdin`.
- **Stdout content unchanged on happy path**: on paths where the exit code does not change (allow, non-blocking events, Claude deny), the stdout JSON content is byte-for-byte identical to the pre-change behaviour.

## Dependencies

- **`detect_host` (existing function in `src/hooks.rs`)**: called with the parsed hook `Value`. Returns `Host::Grok`, `Host::Claude`, or `Host::Unknown`. Reused without modification. On the error path where the payload could not be parsed, called with a default empty JSON object to determine which deny shape to emit.
- **`emit_grok_decision` (existing function in `src/hooks.rs`)**: called with `allow=false` and a reason string on the Grok error path. Reused without modification.
- **`emit_claude_decision` (existing function in `src/hooks.rs`)**: called on the Claude error path (unchanged from current behaviour). Reused without modification.
- **No dependencies outside `src/hooks.rs`**: all changes are confined to that single file. No new entries in `Cargo.toml`; no new crate dependencies.

## Referenced data shapes

All data shapes are defined inline below; no shared vocabulary file is produced for this single-module tier.

**Host** (existing enumeration): discriminates the invoking host.

| Variant | Detection condition |
|---------|-------------------|
| `Grok` | `GROK_HOOK_EVENT` or `GROK_SESSION_ID` environment variable is set |
| `Claude` | `CLAUDE_PROJECT_DIR` or `CLAUDE_PLUGIN_ROOT` env var is set, or `hook_event_name` field present in JSON |
| `Unknown` | Neither of the above |

Only `Host::Grok` triggers exit code 2. `Host::Claude` and `Host::Unknown` both produce exit code 0.

**DesiredExitCode** (integer):

| Value | Meaning |
|-------|---------|
| `0` | Allow, non-blocking event, non-Grok host on any path |
| `2` | Block deny on Grok host, PreToolUse event (happy path or error path) |

No other values are produced.

## Acceptance criteria

- **AC1**: PreToolUse + Block-rule match + `GROK_HOOK_EVENT` set in environment → stdout contains `"decision":"deny"` AND process exit code is 2.
- **AC2**: PreToolUse allow under Grok (no Block match) → process exit code is 0.
- **AC3**: PreToolUse Block deny under Claude (neither `GROK_HOOK_EVENT` nor `GROK_SESSION_ID` set) → process exit code is 0 AND stdout contains `"permissionDecision":"deny"` (regression guard; Claude must never receive a non-zero exit).
- **AC4**: Induced internal error (oversize or malformed stdin) on PreToolUse under Grok → stdout contains Grok-shaped `"decision":"deny"` AND process exit code is 2 (fail-closed).
- **AC5**: Same induced internal error under Claude → process exit code is 0 AND stdout contains Claude-shaped deny (unchanged behaviour).
- **AC6**: PostToolUse or UserPromptSubmit under Grok → process exit code is 0 regardless of rule matches.
- **AC7**: Full existing test suite passes with no regressions; hot-path timing is unaffected.

## Implementation notes (non-normative)

These notes describe the expected change surface to help an implementor locate the right edit points. They do not constrain the implementation approach.

The change has two edit points within `src/hooks.rs`:

1. **`handle_stdin`** (the side-effecting entry point): after `handle_stdin_impl` returns (whether `Ok` or `Err`), apply the desired exit code. Flush stdout. If `desired_exit_code == 2`, call `process::exit(2)`. Otherwise return `Ok(())` as today. On the error branch: if host is Grok, write Grok-shaped deny JSON before the exit; if host is not Grok, write the existing Claude-shaped deny JSON as today then exit 0. (A clean way to surface the deny outcome to `handle_stdin` is to have `handle_stdin_impl` return the desired exit code as its `Ok` payload — e.g. `Result<i32, String>` — computed at the emit site where host/event/deny are already known.)

2. **A new pure helper** (may be a named function or inline logic): accepts `host: Host`, `event: &str`, `deny_outcome: bool` and returns `u8` (or `i32`). Contains the `desired_exit_code` truth table. Has no side effects and no I/O. Covered by unit tests in `#[cfg(test)] mod tests`.

All other functions and modules are unchanged. No new `use` imports are required beyond `std::process` (already available).

Tests covering AC1–AC6 should be added to the `#[cfg(test)] mod tests` block at the bottom of `src/hooks.rs`. Tests for the `desired_exit_code` pure helper are straightforward table-driven unit tests. Tests for AC1, AC4, and AC5 require controlling the `GROK_HOOK_EVENT` environment variable; use `std::env::set_var` / `std::env::remove_var` with care around parallel test execution (set `RUST_TEST_THREADS=1` or use sequential test ordering for env-var-dependent tests). AC7 is satisfied by `cargo test` passing in full.