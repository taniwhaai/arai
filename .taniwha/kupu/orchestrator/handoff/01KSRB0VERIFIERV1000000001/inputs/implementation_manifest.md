# Implementation Manifest — hooks-grok-exit

## Target file

`/home/matt/r/arai/src/hooks.rs` (modified in place, 1604 lines after changes)

## Functions changed

### `desired_exit_code` (new, ~lines 351–364)

Pure helper added immediately before `handle_stdin`. Signature:

```rust
fn desired_exit_code(host: Host, event: &str, deny_outcome: bool) -> i32
```

Truth table: returns `2` iff `host == Host::Grok && event == "PreToolUse" && deny_outcome`; returns `0` for all other inputs. No I/O, no side effects, no `process::exit`.

### `handle_stdin` (~lines 366–430)

The `if let Err(e)` block was replaced with a `match` on `handle_stdin_impl`. On the `Ok(code)` path, `code` is the desired exit code from the inner function. On the `Err(e)` path:

- `eprintln!` the diagnostic (unchanged).
- If `event_hint == "PreToolUse"`: detect host via `detect_host(&Value::Null)`. Grok gets `emit_grok_decision(false, Some("Arai: internal error, blocking for safety"), "")` written to stdout; non-Grok gets the existing Claude-shaped deny JSON.
- Compute `desired_exit_code(host, "PreToolUse", true)` for the error exit code (Grok → 2, Claude/Unknown → 0).
- If `event_hint != "PreToolUse"`: exit code is 0 (no stdout, unchanged).

After both arms, `std::io::stdout().flush()` is called unconditionally (ordering guarantee: stdout before exit). Then `if exit_code == 2 { std::process::exit(2); }` otherwise `Ok(())`.

### `handle_stdin_impl` (~lines 432–940)

Return type changed from `Result<(), String>` to `Result<i32, String>`.

All `Ok(())` returns inside the function body changed to `Ok(0)`. These are the ~15 skip/observability/allow paths:
- `ARAI_DISABLED` bypass
- fast skip-tool exit
- `FileChanged`/`InstructionsLoaded` non-instruction-file path
- `FileChanged`/`InstructionsLoaded` after scan
- `CwdChanged` empty new_cwd path
- `CwdChanged` after scan
- `PermissionDenied` config-load failure
- `PermissionDenied` after retry response
- `PostToolBatch` after compliance loop
- DB path does not exist
- `match_hook` skipped result
- `UserPromptSubmit` prompt summary empty
- `UserPromptSubmit` prompt summary emitted
- `matched.is_empty()` early return

The final `Ok(())` at the emit site (after `println!` of the response JSON) was changed to `Ok(desired_exit_code(host, &result.event, blocking))`. At this point `host`, `result.event`, and `blocking` are all in scope.

## Tests added (~lines 1603–1694)

Two new tests added to the `#[cfg(test)] mod tests` block:

### `desired_exit_code_truth_table`

Table-driven unit test for the pure helper covering all AC cases:
- AC1: `(Grok, "PreToolUse", true)` → 2
- AC2: `(Grok, "PreToolUse", false)` → 0
- AC3: `(Claude, "PreToolUse", true)` → 0 (hard regression guard); `(Unknown, "PreToolUse", true)` → 0
- AC4 precondition: same as AC1 (Grok error path feeds deny_outcome=true, event="PreToolUse")
- AC5 precondition: same as AC3 (Claude error path → 0)
- AC6: `(Grok, "PostToolUse", *)` → 0; `(Grok, "UserPromptSubmit", *)` → 0
- Exhaustive 3×3×2 loop asserting the exact truth table

### `detect_host_grok_env_vars`

Env-var-dependent test verifying `detect_host` returns `Host::Grok` when `GROK_HOOK_EVENT` is set and `Host::Grok` when `GROK_SESSION_ID` is set, and `Host::Unknown`/`Host::Claude` when neither is set. Serialised via a `static OnceLock<Mutex<()>>` (std-only, no new dependency). Cleans up env vars before and after assertions.

## AC satisfaction

| AC | How satisfied |
|----|---------------|
| AC1 | `desired_exit_code(Host::Grok, "PreToolUse", true) == 2`; handle_stdin calls `process::exit(2)` on that code |
| AC2 | `desired_exit_code(Host::Grok, "PreToolUse", false) == 0`; handle_stdin returns `Ok(())` |
| AC3 | `desired_exit_code(Host::Claude, "PreToolUse", true) == 0`; Claude Code always exits 0 |
| AC4 | Error path in handle_stdin: Grok host + PreToolUse event_hint → Grok-shaped deny + exit 2 |
| AC5 | Error path in handle_stdin: non-Grok host + PreToolUse event_hint → Claude-shaped deny + exit 0 |
| AC6 | `desired_exit_code(Host::Grok, "PostToolUse/UserPromptSubmit", *)` always 0 |
| AC7 | `cargo test` passes — see summary below |

## cargo test summary

```
test result: ok. 340 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

All integration test suites also passed (hooks_safety, parser_coverage, prompt_collector_integration, compliance_correlation, etc.) — no regressions.
