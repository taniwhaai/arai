use crate::config::Config;
use crate::intent::Severity;
use crate::store::{Guardrail, Store};
use crate::{audit, compliance, config, guardrails, prompt_collector, session, store, style};
use serde_json::Value;
use std::io::Read;

/// Hard cap on the size of a hook payload from stdin.  Real Claude Code hook
/// invocations are well under 100 KB; 1 MiB is generous.  Without a cap, a
/// malicious or runaway tool that pipes gigabytes into our hook handler
/// would OOM the binary on the hot path.
const MAX_HOOK_INPUT_BYTES: u64 = 1024 * 1024;

/// Environment variable that, when set to `off` or `0`, forces Arai into
/// advise-only mode — even `Block`-severity rules fall back to
/// `permissionDecision: "allow"` with the rule attached as context.  Useful
/// when rolling Arai out incrementally: ingest rules, measure compliance for
/// a week, then flip deny mode on once you trust the rule set.
const DENY_MODE_ENV: &str = "ARAI_DENY_MODE";

/// Hard kill switch: `ARAI_DISABLED` short-circuits the hook entirely.
/// Different from `ARAI_DENY_MODE=off` (which still injects rules as
/// advisories) — `ARAI_DISABLED` is the "Arai is causing a problem,
/// turn it OFF right now" emergency lever.  We still write a single
/// `bypassed` audit entry so post-hoc inspection can tell "no rules fired"
/// from "Arai was disabled".
const DISABLED_ENV: &str = "ARAI_DISABLED";

fn deny_mode_enabled() -> bool {
    match std::env::var(DENY_MODE_ENV) {
        Ok(v) => {
            let v = v.to_lowercase();
            v != "off" && v != "0" && v != "false" && v != "no"
        }
        Err(_) => true,
    }
}

fn is_disabled_via_env() -> bool {
    match std::env::var(DISABLED_ENV) {
        Ok(v) => {
            let v = v.to_lowercase();
            matches!(v.as_str(), "1" | "true" | "on" | "yes")
        }
        Err(_) => false,
    }
}

/// Supported coding agent hosts that can invoke Arai's hook handler.
/// Detection is best-effort via environment variables injected by the host
/// (Grok TUI sets GROK_* vars; Claude Code sets CLAUDE_* vars).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Host {
    Claude,
    Grok,
    Unknown,
}

fn detect_host(hook: &Value) -> Host {
    // Grok TUI (supergrok) injects these on hook invocations.
    if std::env::var("GROK_HOOK_EVENT").is_ok() || std::env::var("GROK_SESSION_ID").is_ok() {
        return Host::Grok;
    }
    // Claude Code compatibility / native path.
    if std::env::var("CLAUDE_PROJECT_DIR").is_ok()
        || hook.get("hook_event_name").is_some()
        || std::env::var("CLAUDE_PLUGIN_ROOT").is_ok()
    {
        return Host::Claude;
    }
    Host::Unknown
}

/// Allow-list for hook event names that are safe to propagate from the
/// inner JSON to the outer fail-closed gate's `event_hint`.  Any other
/// string (typo, spoofed input, future-event-we-don't-know-yet) leaves
/// `event_hint` at its safe `"PreToolUse"` default so a downstream error
/// still fails closed instead of being silently treated as a non-deny
/// event.  Returns `Some(canonical_str)` for recognised events so the
/// caller stores the static literal rather than a heap copy of the JSON.
fn known_hook_event(event: &str) -> Option<&'static str> {
    match event {
        "PreToolUse" => Some("PreToolUse"),
        "PostToolUse" => Some("PostToolUse"),
        "UserPromptSubmit" => Some("UserPromptSubmit"),
        // Observability-only events that keep Arai's rule set in sync with
        // disk and context.  Handled in `handle_stdin_impl` before the
        // match pipeline — they never produce a `permissionDecision`.
        "FileChanged" => Some("FileChanged"),
        "InstructionsLoaded" => Some("InstructionsLoaded"),
        "CwdChanged" => Some("CwdChanged"),
        "PostToolBatch" => Some("PostToolBatch"),
        // PermissionDenied is decision-bearing (can return retry: true)
        // but isn't a tool-call event; handled in its own dispatch
        // branch alongside the observability events.
        "PermissionDenied" => Some("PermissionDenied"),
        _ => None,
    }
}

/// Does this absolute path look like an AI-coding-assistant instruction
/// file Arai cares about?  Used by the FileChanged / InstructionsLoaded
/// handlers to decide whether to trigger a background rescan.  Kept
/// permissive: a false positive (rescan when we didn't strictly need to)
/// costs a few ms of background CPU; a false negative leaves Arai with a
/// stale rule set, which is the bug we're fixing.
pub(crate) fn is_instruction_file(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let normalized = path.replace('\\', "/");
    let file_name = std::path::Path::new(&normalized)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    // Exact basenames Arai's discovery layer picks up.
    // Includes both Claude-centric and Grok-native (AGENTS.md family) files.
    if matches!(
        file_name,
        "CLAUDE.md"
            | "AGENTS.md"
            | "Agents.md"
            | "AGENT.md"
            | "agents.md"
            | ".cursorrules"
            | ".windsurfrules"
            | "copilot-instructions.md"
    ) {
        return true;
    }
    // Directory-anchored rule files: .claude/rules/*.md, .cursor/rules/*.md.
    // Matched on path substring so we catch nested and project-local cases.
    if (normalized.contains("/.claude/rules/") || normalized.contains("/.cursor/rules/"))
        && file_name.ends_with(".md")
    {
        return true;
    }
    // Per-project Claude Code memory files: ~/.claude/projects/<slug>/memory/*.md.
    if normalized.contains("/.claude/projects/")
        && normalized.contains("/memory/")
        && file_name.ends_with(".md")
    {
        return true;
    }
    false
}

/// Spawn a detached `arai scan` so the rule set picks up the edited /
/// loaded instruction file (or the new monorepo package after a `cd`)
/// before the next tool call.  Best-effort: any failure (binary not
/// found, fork EAGAIN) is silently dropped — the existing stale rule
/// set is still better than a panic on the hook path.  Both stdout
/// and stderr are nulled so the child's output doesn't leak into
/// Claude Code's hook-stdout-as-context channel.
///
/// `cwd` lets the caller scope the scan to a specific directory (used
/// by the `CwdChanged` handler so the per-project DB at the *new*
/// working directory gets populated, not the hook's launch dir).
/// `None` means "inherit the current process's CWD".
///
/// Concurrent invocations (rapid CLAUDE.md saves; FileChanged plus
/// InstructionsLoaded firing on the same edit; CwdChanged on every
/// tab-toggle in a monorepo) are safe — SQLite serialises writes, so
/// the worst case is a wasted scan, not corruption.  Worth adding a
/// debounce later if telemetry shows it matters.
fn spawn_background_scan(cwd: Option<&str>) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("scan")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let _ = cmd.spawn();
}

/// Highest severity among the matched rules, if any.  Used to pick between
/// advise (`allow`) and deny (`deny`) on PreToolUse.  Reads intent from the
/// guardrail itself — `load_guardrails` already LEFT JOINed it in.
fn highest_severity(matched: &[(Guardrail, u8)]) -> Severity {
    let mut highest = Severity::Inform;
    for (g, _) in matched {
        let sev = match g.intent.as_ref() {
            Some(intent) => intent.severity,
            // No classified intent — fall back to predicate-derived severity
            // so pre-migration stores still block on obvious `never` rules.
            None => Severity::from_predicate(&g.predicate),
        };
        if sev == Severity::Block {
            return Severity::Block;
        }
        if sev == Severity::Warn && highest == Severity::Inform {
            highest = Severity::Warn;
        }
    }
    highest
}

/// Result of matching a hook payload against the current guardrail set.
/// Pure — no audit write, no telemetry, no stdout.
pub struct HookMatch {
    pub event: String,
    pub tool_name: String,
    pub session_id: String,
    pub terms: Vec<String>,
    pub matched: Vec<(Guardrail, u8)>,
    /// True if the tool is in the skip list — matching was bypassed entirely.
    pub skipped: bool,
    /// True if the caller should emit the UserPromptSubmit summary instead of
    /// a match response.
    pub is_prompt_summary: bool,
    /// Domain rules for the UserPromptSubmit summary (populated only when
    /// `is_prompt_summary` is true).
    pub domain_rules: Vec<Guardrail>,
}

/// Apply the full match pipeline to a parsed hook payload.
///
/// Mirrors the behaviour of `handle_stdin` without performing any IO:
/// caller owns stdout, audit log, and telemetry.  Used by the hook handler
/// *and* by the `arai test` scenario runner — both paths see the same
/// matching logic so scenarios stay faithful to production.
pub fn match_hook(hook: &Value, cfg: &Config, db: &Store) -> Result<HookMatch, String> {
    let event = hook
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("PreToolUse")
        .to_string();
    let raw_tool_name = hook.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    let tool_name = guardrails::normalize_tool_name(raw_tool_name);
    // Sanitize session_id at the boundary — anything that wouldn't survive
    // path-traversal validation is treated as no-session (session features
    // silently disable, the rest of the hook still works).  See
    // `session::valid_session_id` for the accepted shape.
    let session_id = hook
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| session::valid_session_id(s))
        .unwrap_or("")
        .to_string();

    let mut out = HookMatch {
        event: event.clone(),
        tool_name: tool_name.clone(),
        session_id: session_id.clone(),
        terms: Vec::new(),
        matched: Vec::new(),
        skipped: false,
        is_prompt_summary: false,
        domain_rules: Vec::new(),
    };

    // Fast exit for tools that never need guardrails
    if !tool_name.is_empty() && guardrails::should_skip_tool(&tool_name) {
        out.skipped = true;
        return Ok(out);
    }

    let tool_input = hook
        .get("tool_input")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    // Self-exemption: `arai` CLI commands (`arai why`, `arai severity`, `arai
    // add`, `arai status`, …) are diagnostic / rule-management.  They read or
    // mutate the rule set itself and shouldn't be blocked by it.  Issue #86:
    // `arai why "git status"` was being denied by the very rule it was being
    // asked to explain, and `arai severity "git push" block` was being denied
    // when the user tried to pin a rule's severity.  Treat any Bash command
    // whose first non-flag argument is the `arai` binary as a skip — same
    // bypass channel as Read/Glob, no terms extracted, no rules consulted.
    if tool_name == "Bash" && is_arai_self_command(&tool_input) {
        out.skipped = true;
        return Ok(out);
    }

    let mut terms = guardrails::extract_terms(&tool_name, &tool_input);

    // PostToolUse: sniff results but don't mutate session state here (that's a
    // side effect the hook handler owns, not the scenario runner)
    if event == "PostToolUse" {
        if let Some(result) = hook.get("tool_result").and_then(|v| v.as_str()) {
            guardrails::sniff_content_for_tools_pub(result, &mut terms);
        }
        terms.sort();
        terms.dedup();
    }

    let is_timing_event = event == "UserPromptSubmit";
    if terms.is_empty() && !is_timing_event {
        return Ok(out);
    }

    guardrails::enrich_terms_from_graph(&mut terms, &tool_name, &tool_input, db);
    out.terms = terms.clone();

    let all_guardrails = db.load_guardrails().map_err(|e| e.to_string())?;

    // UserPromptSubmit: brief summary of active domain guardrails
    if event == "UserPromptSubmit" {
        let domain_rules: Vec<Guardrail> = all_guardrails
            .iter()
            .filter(|g| {
                g.intent
                    .as_ref()
                    .map(|i| i.timing == crate::intent::Timing::ToolCall)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        out.is_prompt_summary = true;
        out.domain_rules = domain_rules;
        return Ok(out);
    }

    let command_phrases = guardrails::extract_command_phrases(&tool_name, &tool_input);
    let matched = guardrails::match_guardrails(
        &all_guardrails,
        &terms,
        &command_phrases,
        &tool_name,
        &event,
    );

    // Filter out rules whose prerequisites have already been met
    let matched: Vec<_> = if !session_id.is_empty() && event == "PreToolUse" {
        matched
            .into_iter()
            .filter(|(g, _)| {
                let prereqs = session::extract_prerequisite(&g.object);
                if prereqs.is_empty() {
                    true
                } else {
                    !session::prerequisite_met(&cfg.arai_base_dir, &session_id, &prereqs)
                }
            })
            .collect()
    } else {
        matched
    };

    out.matched = matched;
    Ok(out)
}

/// Pure truth table: map (host, event, deny_outcome) → desired process exit
/// code.  Returns 2 iff all three conditions are met: the invoking host is
/// Grok TUI, the event is PreToolUse, and the match pipeline produced a
/// Block-severity deny.  All other combinations → 0.
///
/// This function has no side effects, performs no I/O, and never calls
/// `process::exit`.  The sole caller, `handle_stdin`, owns process-exit.
fn desired_exit_code(host: Host, event: &str, deny_outcome: bool) -> i32 {
    if host == Host::Grok && event == "PreToolUse" && deny_outcome {
        2
    } else {
        0
    }
}

pub fn handle_stdin() -> Result<(), String> {
    // Default to PreToolUse so a bad payload (oversize / non-UTF8 / non-JSON)
    // — which we can't parse to know the real event — is treated as a
    // PreToolUse failure and gets the safe-by-default deny response.
    let mut event_hint = String::from("PreToolUse");

    let exit_code = match handle_stdin_impl(&mut event_hint) {
        Ok(code) => code,
        Err(e) => {
            // Diagnostics on stderr.
            eprintln!("arai hook error: {e}");
            // Fail-closed on PreToolUse: emit a deny JSON to stdout so the
            // host blocks the tool call.  Without this, an attacker who can
            // induce a hook error (oversize input, malformed JSON, DB lock)
            // would slip past every Block-severity rule.  PostToolUse and
            // UserPromptSubmit tolerate empty stdout — those events don't have
            // a permissionDecision surface and the tool already ran (or is
            // about to be summarized).
            //
            // Host detection on the error path: since the payload may be
            // unparsed, call detect_host with Value::Null (env-var-only
            // detection, which is all that matters on the error path).
            if event_hint == "PreToolUse" {
                let host = detect_host(&Value::Null);
                match host {
                    Host::Grok => {
                        let response = emit_grok_decision(
                            false,
                            Some("Arai: an internal error occurred; blocking this action."),
                            "",
                        );
                        println!("{}", serde_json::to_string(&response).unwrap_or_default());
                    }
                    _ => {
                        let response = serde_json::json!({
                            "hookSpecificOutput": {
                                "hookEventName": "PreToolUse",
                                "permissionDecision": "deny",
                                "permissionDecisionReason":
                                    "Arai: an internal error occurred; blocking this action.",
                            }
                        });
                        println!("{}", serde_json::to_string(&response).unwrap_or_default());
                    }
                }
                // Grok PreToolUse error: fail-closed with exit 2.
                // Claude/Unknown: always exit 0 (Claude treats non-zero as
                // "hook broken", defeating the deny above).
                desired_exit_code(host, "PreToolUse", true)
            } else {
                0
            }
        }
    };

    // Flush stdout so every byte is visible to the host before we exit.
    use std::io::Write;
    let _ = std::io::stdout().flush();

    if exit_code == 2 {
        std::process::exit(2);
    }
    Ok(())
}

fn handle_stdin_impl(event_hint: &mut String) -> Result<i32, String> {
    let start = std::time::Instant::now();
    // Read up to MAX_HOOK_INPUT_BYTES + 1 so we can distinguish "natural EOF"
    // from "hit the cap mid-stream".  Reject overruns rather than silently
    // truncating the JSON (a partial JSON would parse-fail anyway, but being
    // explicit gives a clearer error and prevents memory exhaustion by a
    // hostile pipe before the parse step).
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    std::io::stdin()
        .lock()
        .take(MAX_HOOK_INPUT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Could not read stdin: {e}"))?;
    if buf.len() as u64 > MAX_HOOK_INPUT_BYTES {
        return Err(format!(
            "Hook input exceeded {MAX_HOOK_INPUT_BYTES}-byte cap"
        ));
    }
    let input =
        String::from_utf8(buf).map_err(|e| format!("Hook input was not valid UTF-8: {e}"))?;

    let hook: Value =
        serde_json::from_str(&input).map_err(|e| format!("Invalid hook JSON: {e}"))?;

    let raw_tool_name = hook.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    let tool_name = guardrails::normalize_tool_name(raw_tool_name);
    let event = hook
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("PreToolUse");
    // Tell the outer wrapper what event we're processing so a later error
    // (DB lock, store-open failure) is fail-closed only when appropriate.
    // Only propagate KNOWN events — a spoofed value like "PreToolUseFOO"
    // would cause `event_hint != "PreToolUse"` later, suppressing the deny
    // emit and letting the tool through (the very behaviour C10 fixed for
    // the byte-flip / oversize cases).  Unknown events leave event_hint at
    // its safe default so the wrapper still fails closed.
    if let Some(known) = known_hook_event(event) {
        event_hint.clear();
        event_hint.push_str(known);
    }
    // Sanitize session_id (see `session::valid_session_id`).  Hostile
    // payloads with `..` or `/` bytes in the id no longer reach the
    // session-file writer.
    let session_id = hook
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| session::valid_session_id(s))
        .unwrap_or("");

    // Global emergency short-circuit.  When `ARAI_DISABLED` is set to a
    // truthy value we skip rule matching entirely but still log a single
    // `decision="bypassed"` audit entry per invocation so `arai stats`
    // continues to see when Arai was off vs simply quiet.  No telemetry
    // and no stdout response — the model behaves exactly as if no hook
    // were installed.
    if is_disabled_via_env() {
        match config::Config::load() {
            Ok(cfg) => audit::record_bypass(&cfg, event, &tool_name, session_id),
            // Surface the config failure to stderr so an operator can
            // correlate "Arai was off but stats has no bypass entry".  The
            // hook still exits 0 — the user explicitly chose `ARAI_DISABLED`
            // so the model must proceed.
            Err(e) => {
                eprintln!("arai: ARAI_DISABLED set but could not load config to record bypass: {e}")
            }
        }
        return Ok(0);
    }

    // Fast exit mirrors match_hook — avoids loading config/db for skipped tools
    if !tool_name.is_empty() && guardrails::should_skip_tool(&tool_name) {
        return Ok(0);
    }

    // FileChanged / InstructionsLoaded: observability events Claude Code
    // fires when an instruction file is edited on disk or loaded into
    // context.  Arai's job here is to refresh its own rule set so the
    // next tool-call hook sees the updated guardrails — *not* to gate
    // anything (no `permissionDecision` surface on these events).
    //
    // Dispatched before the match pipeline because they don't have a
    // `tool_name`/`tool_input` payload to match against, and they should
    // be cheap: the steady-state behaviour on an unrelated file change
    // is "spend 1ms checking the path, exit".
    if event == "FileChanged" || event == "InstructionsLoaded" {
        let file_path = hook.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
        if !is_instruction_file(file_path) {
            return Ok(0);
        }
        // Load config best-effort — if it fails (uninitialised project)
        // we still want to silently no-op rather than break the hook.
        if let Ok(cfg) = config::Config::load() {
            audit::record_event(
                &cfg,
                event,
                "",
                session_id,
                serde_json::json!({
                    "file_path": file_path,
                    "trigger": "instruction_file_touched",
                }),
            );
            spawn_background_scan(None);
        }
        return Ok(0);
    }

    // CwdChanged: Claude Code's working directory moved (e.g. the model
    // ran `cd packages/api`).  In a monorepo, the project_slug Arai
    // derives from CWD is now wrong — the next tool-call hook would
    // load guardrails for the new dir's slug, which may have never
    // been scanned.  Trigger a scan rooted at `new_cwd` so the
    // destination dir's per-project DB is populated.
    //
    // Observability-only: no permissionDecision surface.  Logs the
    // transition into the audit trail so `arai audit --event=CwdChanged`
    // shows the per-session navigation history.
    if event == "CwdChanged" {
        let new_cwd = hook.get("new_cwd").and_then(|v| v.as_str()).unwrap_or("");
        if new_cwd.is_empty() {
            return Ok(0);
        }
        let old_cwd = hook.get("old_cwd").and_then(|v| v.as_str()).unwrap_or("");
        if let Ok(cfg) = config::Config::load() {
            audit::record_event(
                &cfg,
                "CwdChanged",
                "",
                session_id,
                serde_json::json!({
                    "old_cwd": old_cwd,
                    "new_cwd": new_cwd,
                }),
            );
            spawn_background_scan(Some(new_cwd));
        }
        return Ok(0);
    }

    // PermissionDenied: Claude Code's auto-mode classifier denied a
    // tool call.  Arai's role here is twofold:
    //   1. Log the denial into the audit trail so the unified record
    //      shows both classifiers' decisions (no silent disagreement).
    //   2. If Arai's own policy for this tool call is *Warn* (not
    //      Block) — i.e. Arai would have inject-with-warning rather
    //      than denied — return `{retry: true}` to override the
    //      auto-deny so the call proceeds.
    //
    // We deliberately do NOT retry when Arai has no matching rule
    // (Arai has no opinion → Anthropic's classifier stands) or when
    // Arai matches at Block severity (we agree with the deny).
    if event == "PermissionDenied" {
        let cfg = match config::Config::load() {
            Ok(c) => c,
            Err(_) => return Ok(0),
        };
        let db_path = cfg.db_path();
        // Synthesize a PreToolUse-shaped payload from the denied call so
        // we run through the exact same match pipeline as a normal
        // pre-call gate.  Lets us reuse extract_terms, code-graph
        // enrichment, severity-from-rule logic without forking.
        let raw_denied_tool_name = hook.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
        let denied_tool_name = guardrails::normalize_tool_name(raw_denied_tool_name);
        let synthesized = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "tool_name": denied_tool_name,
            "tool_input": hook
                .get("tool_input")
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new())),
            "session_id": session_id,
        });

        let arai_top_severity: Option<Severity> = if db_path.exists() {
            match store::Store::open(&db_path) {
                Ok(db) => match match_hook(&synthesized, &cfg, &db) {
                    Ok(r) if !r.matched.is_empty() => Some(highest_severity(&r.matched)),
                    _ => None,
                },
                Err(_) => None,
            }
        } else {
            None
        };

        // Retry iff Arai matched at Warn (and deny mode is enabled —
        // if the operator has flipped `ARAI_DENY_MODE=off`, Arai is in
        // advise-only mode and shouldn't be overriding Anthropic's
        // classifier in any direction).
        let retry = deny_mode_enabled() && arai_top_severity == Some(Severity::Warn);

        let denial_reason = hook
            .get("denial_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        audit::record_event(
            &cfg,
            "PermissionDenied",
            &denied_tool_name,
            session_id,
            serde_json::json!({
                "denial_reason": denial_reason,
                "arai_matched": arai_top_severity.is_some(),
                "arai_severity": arai_top_severity.map(|s| s.as_str()),
                "retry": retry,
            }),
        );

        if retry {
            let response = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionDenied",
                    "retry": true,
                }
            });
            println!(
                "{}",
                serde_json::to_string(&response).map_err(|e| e.to_string())?
            );
        }
        return Ok(0);
    }

    // PostToolBatch: fires once per batch of parallel tool calls (e.g.
    // a multi-Edit or several parallel Bash invocations).  Today's
    // PostToolUse correlator pairs single Pre/Post events, which under-
    // counts compliance verdicts on parallel workloads — every tool in
    // the batch shares the *batch* Post event, not individual ones.
    //
    // Strategy: iterate `tool_calls[] + tool_results[]` from the
    // payload and feed each pair through the same compliance pipeline
    // PostToolUse uses.  That way every parallel tool gets its own
    // Obeyed/Ignored/Unclear verdict against any PreToolUse firings in
    // the same session.  Observability-only — we don't block the loop
    // here; gating happened at PreToolUse already.
    if event == "PostToolBatch" {
        if let Ok(cfg) = config::Config::load() {
            let empty = Vec::new();
            let tool_calls = hook
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .unwrap_or(&empty);
            let tool_results = hook
                .get("tool_results")
                .and_then(|v| v.as_array())
                .unwrap_or(&empty);
            // Pair calls with results by index.  Both arrays come from
            // Claude Code in the same order, one entry per concurrent
            // tool invocation in the batch.
            for (idx, call) in tool_calls.iter().enumerate() {
                let raw_tool_name = call.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                let tool_name = guardrails::normalize_tool_name(raw_tool_name);
                if tool_name.is_empty() || guardrails::should_skip_tool(&tool_name) {
                    continue;
                }
                let tool_input = call
                    .get("tool_input")
                    .cloned()
                    .unwrap_or(Value::Object(serde_json::Map::new()));
                let mut terms = guardrails::extract_terms(&tool_name, &tool_input);
                // Pull the corresponding result for content-sniffing
                // (a `from alembic import op` written by tool N still
                // needs to seed terms for tool N's compliance pass).
                if let Some(res) = tool_results.get(idx) {
                    if let Some(text) = res
                        .get("output")
                        .and_then(|o| o.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        guardrails::sniff_content_for_tools_pub(text, &mut terms);
                    }
                }
                terms.sort();
                terms.dedup();
                if !session_id.is_empty() {
                    session::record_tool_call(&cfg.arai_base_dir, session_id, &tool_name, &terms);
                }
                let preview = summarize_tool_input(&tool_name, &tool_input);
                compliance::record_post_compliance(&cfg, session_id, &tool_name, &terms, &preview);
            }
            // Single audit entry per batch — keeps the log readable
            // when a batch contains dozens of tools.  Per-tool
            // compliance verdicts already get their own entries via
            // record_post_compliance.
            audit::record_event(
                &cfg,
                "PostToolBatch",
                "",
                session_id,
                serde_json::json!({
                    "tool_count": tool_calls.len(),
                }),
            );
        }
        return Ok(0);
    }

    // PostToolUse still has a side effect: it records the call into session
    // state for prerequisite tracking.  Do that *before* match_hook so scenarios
    // running through the same path don't corrupt real sessions.
    if event == "PostToolUse" && !session_id.is_empty() {
        if let Ok(cfg) = config::Config::load() {
            let tool_input = hook
                .get("tool_input")
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));
            let mut terms = guardrails::extract_terms(&tool_name, &tool_input);
            if let Some(result) = hook.get("tool_result").and_then(|v| v.as_str()) {
                guardrails::sniff_content_for_tools_pub(result, &mut terms);
            }
            terms.sort();
            terms.dedup();
            session::record_tool_call(&cfg.arai_base_dir, session_id, &tool_name, &terms);

            // Compliance tracking: correlate this PostToolUse against any
            // recent PreToolUse firings in the same session and emit one
            // Compliance audit entry per rule.  Done here (not in match_hook)
            // because scenario replays should not pollute the audit log.
            let preview = summarize_tool_input(&tool_name, &tool_input);
            compliance::record_post_compliance(&cfg, session_id, &tool_name, &terms, &preview);
        }
    }

    let cfg = config::Config::load()?;

    // Prompt-collector: runs on UserPromptSubmit regardless of whether Arai's
    // rule DB has been initialised.  The collector uses only the compiled-in
    // seed ruleset and the prompt text from the hook payload — no DB needed.
    // Best-effort: the collector is pure; only the record_event calls perform
    // I/O, and those are already silent-on-failure.
    // The hook response (stdout) is built separately below and is NOT mutated
    // here.
    if event == "UserPromptSubmit" {
        let prompt_text = hook.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        if !prompt_text.is_empty() {
            let ts = prompt_event_timestamp();
            let slug = cfg.project_slug();
            let seed = prompt_collector::seed_rules();
            let (receipts, _skipped) =
                prompt_collector::collect_prompt_matches(prompt_text, &seed, &slug, &ts);
            for receipt in receipts {
                audit::record_event(
                    &cfg,
                    receipt.event,
                    "",
                    session_id,
                    serde_json::json!({
                        "prompt_hash": receipt.prompt_hash,
                        "matched_label": receipt.matched_label,
                        "timestamp_iso": receipt.timestamp_iso,
                        "project_slug": receipt.project_slug,
                        "did_any_tool_call_follow": receipt.did_any_tool_call_follow,
                    }),
                );
            }
        }
    }

    let db_path = cfg.db_path();
    if !db_path.exists() {
        return Ok(0);
    }
    let db = store::Store::open(&db_path)?;

    let result = match_hook(&hook, &cfg, &db)?;
    if result.skipped {
        return Ok(0);
    }

    // UserPromptSubmit summary — domain-rules context injected into the response.
    // The prompt-collector already ran above (before the DB gate) so there is
    // no collector work to do here.
    if result.is_prompt_summary {
        if result.domain_rules.is_empty() {
            return Ok(0);
        }
        let mut subjects: Vec<String> = result
            .domain_rules
            .iter()
            .map(|g| g.subject.clone())
            .collect();
        subjects.sort();
        subjects.dedup();
        let summary = format!(
            "Arai: {} active rule(s) for: {}. Rules fire on relevant tool calls.",
            result.domain_rules.len(),
            subjects.join(", ")
        );
        let response = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": summary
            }
        });
        println!(
            "{}",
            serde_json::to_string(&response).map_err(|e| e.to_string())?
        );
        return Ok(0);
    }

    if result.matched.is_empty() {
        return Ok(0);
    }

    // Telemetry — aggregate counters only
    let latency = start.elapsed().as_millis();
    crate::telemetry::track_hook_latency(&cfg.arai_base_dir, &result.event, latency, true);
    for (g, pct) in &result.matched {
        let severity = g
            .intent
            .as_ref()
            .map(|i| i.severity.as_str())
            .unwrap_or_else(|| Severity::from_predicate(&g.predicate).as_str());
        crate::telemetry::track_rule_fired(
            &cfg.arai_base_dir,
            &g.subject,
            &g.predicate,
            &result.tool_name,
            &result.event,
            *pct,
            severity,
        );
    }

    // Decide whether to deny before writing the audit line so the audit log
    // reflects the actual outcome the hook emitted.
    let top_severity = highest_severity(&result.matched);
    let is_pretooluse = result.event == "PreToolUse";
    let deny_enabled = deny_mode_enabled();
    let blocking = is_pretooluse && top_severity == Severity::Block && deny_enabled;

    // Local audit log — records every firing for `arai audit` / `arai stats`
    let tool_input = hook
        .get("tool_input")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let prompt_preview = summarize_tool_input(&result.tool_name, &tool_input);
    let decision = match (result.event.as_str(), blocking) {
        ("PreToolUse", true) => "deny",
        ("PreToolUse", false) => "inject",
        ("PostToolUse", _) => "review",
        (other, _) => other,
    };
    // Per-session seen-rule tracking.  Rules already fully injected earlier
    // in this session emit a compact one-liner instead of re-injecting the
    // full source/layer/severity payload — saves tokens on long sessions
    // and avoids attention dilution from repeat re-reads.  Empty session_id
    // means we can't track, so all matches behave as first-time injections.
    let triple_ids: Vec<i64> = result.matched.iter().map(|(g, _)| g.triple_id).collect();
    let (unseen, seen) =
        session::partition_seen_rules(&cfg.arai_base_dir, &result.session_id, &triple_ids);
    let seen_set: std::collections::HashSet<i64> = seen.iter().copied().collect();

    audit::record_firing(
        &cfg,
        &result.event,
        &result.tool_name,
        &result.session_id,
        &prompt_preview,
        &result.matched,
        decision,
        Some(&db),
        &seen_set,
    );

    let context = guardrails::format_context(&result.matched, &seen_set);

    // Mark the unseen rules as seen now that we've emitted full context for
    // them.  Done after the audit write so a panic between match and write
    // doesn't permanently suppress a rule the model never actually saw.
    if !unseen.is_empty() {
        session::mark_rules_seen(&cfg.arai_base_dir, &result.session_id, &unseen);
    }
    // Detect the calling host so we can emit the correct response format.
    // Grok TUI expects a flat {"decision", "reason"} shape; Claude Code expects
    // the hookSpecificOutput + permissionDecision shape.
    let host = detect_host(&hook); // `hook` is in scope from handle_stdin_impl

    // Gateway glyphs on the hook path: colorize=false ALWAYS (no ANSI colour
    // ever emitted on the hook path — carve-out #1).  unicode derived from
    // locale/env in the usual way; the glyph characters themselves are safe
    // when piped.
    let hook_unicode = style::should_use_unicode();
    // Outcome for the additionalContext glyph: Block when deny, Warn when at
    // least one matched rule has block/warn severity, Inform otherwise.
    let ctx_outcome = if blocking {
        style::Outcome::Block
    } else {
        match top_severity {
            Severity::Block | Severity::Warn => style::Outcome::Warn,
            Severity::Inform => style::Outcome::Inform,
        }
    };
    let ctx_glyph = style::outcome_glyph(ctx_outcome, hook_unicode, false);
    let prefixed_context = format!("{ctx_glyph} {context}");

    let response = match (result.event.as_str(), blocking) {
        ("PreToolUse", true) => {
            let raw_reason = deny_reason(&result.matched);
            let block_glyph = style::outcome_glyph(style::Outcome::Block, hook_unicode, false);
            let reason = format!("{block_glyph} {raw_reason}");
            match host {
                Host::Grok => emit_grok_decision(false, Some(&reason), &prefixed_context),
                _ => emit_claude_decision(false, Some(&reason), &prefixed_context),
            }
        }
        ("PreToolUse", false) => match host {
            Host::Grok => emit_grok_decision(true, None, &prefixed_context),
            _ => emit_claude_decision(true, None, &prefixed_context),
        },
        ("PostToolUse", _) => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": format!("[Post-action] {prefixed_context}")
            }
        }),
        _ => {
            // Default / unknown events fall back to Claude shape for safety
            emit_claude_decision(true, None, &prefixed_context)
        }
    };

    println!(
        "{}",
        serde_json::to_string(&response).map_err(|e| e.to_string())?
    );
    Ok(desired_exit_code(host, &result.event, blocking))
}

/// Build a short deny reason Claude Code surfaces to the user.  Prefers the
/// first `Block`-severity rule (or predicate-derived fallback) and quotes its
/// source so the decision is auditable at a glance.
fn deny_reason(matched: &[(Guardrail, u8)]) -> String {
    for (g, _) in matched {
        let sev = g
            .intent
            .as_ref()
            .map(|i| i.severity)
            .unwrap_or_else(|| Severity::from_predicate(&g.predicate));
        if sev == Severity::Block {
            let src = if g.file_path.is_empty() {
                &*g.source_file
            } else {
                &*g.file_path
            };
            // Append `:N` when we know the line — saves the user a manual
            // search to the rule that just blocked their action.
            let line_suffix = g.line_start.map(|l| format!(":{l}")).unwrap_or_default();
            return format!(
                "Arai: \"{subj} {pred} {obj}\" [from {src}{line_suffix}]",
                subj = g.subject,
                pred = g.predicate,
                obj = g.object,
            );
        }
    }
    // Shouldn't reach here if highest_severity returned Block, but guard for
    // robustness.
    "Arai: a rule blocked this action.".to_string()
}

/// Emit a Grok TUI compatible decision response.
fn emit_grok_decision(
    allow: bool,
    reason: Option<&str>,
    additional_context: &str,
) -> serde_json::Value {
    if allow {
        serde_json::json!({
            "decision": "allow",
            "additionalContext": additional_context
        })
    } else {
        serde_json::json!({
            "decision": "deny",
            "reason": reason.unwrap_or("Arai: a rule blocked this action."),
            "additionalContext": additional_context
        })
    }
}

/// Emit a Claude Code compatible decision response (preserves original shape exactly).
fn emit_claude_decision(
    allow: bool,
    reason: Option<&str>,
    additional_context: &str,
) -> serde_json::Value {
    if allow {
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "additionalContext": additional_context
            }
        })
    } else {
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason.unwrap_or("Arai: a rule blocked this action."),
                "additionalContext": additional_context
            }
        })
    }
}

/// True when a Bash `tool_input` invokes the `arai` CLI itself.  Matches the
/// first token of the command (path-stripped) against the literal `arai` — so
/// `arai why "git push"`, `./arai status`, `/usr/local/bin/arai add ...`, and
/// `arai severity foo block` are all recognised.  Pipelines / chains: we look
/// only at the first token of the first segment.  That is intentional — a
/// user running `something && arai why ...` is composing arai with something
/// else, and we only want to exempt the standalone case.
fn is_arai_self_command(tool_input: &Value) -> bool {
    let cmd = match tool_input.get("command").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return false,
    };
    // First segment up to a pipe / chain operator; first token within that.
    let first_segment = cmd
        .split(['|', ';'])
        .next()
        .unwrap_or(cmd)
        .split("&&")
        .next()
        .unwrap_or(cmd)
        .trim();
    let first_token = first_segment.split_whitespace().next().unwrap_or("");
    let basename = first_token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first_token);
    // Strip an optional Windows `.exe` so cross-platform invocations match.
    let stripped = basename.strip_suffix(".exe").unwrap_or(basename);
    stripped.eq_ignore_ascii_case("arai")
}

/// Return a minimal RFC-3339-style UTC timestamp string (YYYY-MM-DDTHH:MM:SSZ)
/// for use as the `timestamp_iso` argument to `prompt_collector::collect_prompt_matches`.
/// Matches the format written by `audit::record_event` so prompt-match receipts
/// are comparable to other audit entries.
fn prompt_event_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Manual UTC decomposition — no chrono dependency required.
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Rata Die → Gregorian (civil-calendar decomposition, valid for Unix epoch)
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Produce a short human-readable preview of tool input for the audit log.
/// Prefers the most-informative field per tool; truncates + strips newlines.
fn summarize_tool_input(tool_name: &str, input: &Value) -> String {
    let raw = match tool_name {
        "Bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Edit" | "Write" | "MultiEdit" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{tool_name} {path}")
        }
        _ => input.to_string(),
    };
    let oneline = raw.replace(['\n', '\r'], " ");
    let trimmed = oneline.trim();
    if trimmed.chars().count() <= 200 {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(200).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::RuleIntent;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_db() -> (Store, PathBuf) {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("arai_hooks_test_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let store = Store::open(&db_path).unwrap();
        (store, dir)
    }

    fn mk_guardrail(id: i64, subject: &str, predicate: &str, object: &str) -> Guardrail {
        Guardrail {
            triple_id: id,
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            confidence: 0.9,
            source_file: "CLAUDE.md".to_string(),
            file_path: "CLAUDE.md".to_string(),
            layer: Some(1),
            line_start: Some(42),
            expires_at: None,
            noenrich: false,
            intent: None,

            tier: None,

            source_label: None,
        }
    }

    #[test]
    fn test_deny_mode_env_toggle() {
        // Default on (env unset) — guard value may leak from the running test
        // harness, so assert the positive and negative values explicitly.
        for (val, expected) in [
            ("on", true),
            ("", true),
            ("off", false),
            ("0", false),
            ("false", false),
            ("no", false),
            ("yes", true),
        ] {
            std::env::set_var(DENY_MODE_ENV, val);
            assert_eq!(
                deny_mode_enabled(),
                expected,
                "ARAI_DENY_MODE={val:?} expected {expected}"
            );
        }
        std::env::remove_var(DENY_MODE_ENV);
        assert!(
            deny_mode_enabled(),
            "unset ARAI_DENY_MODE should enable deny mode"
        );
    }

    #[test]
    fn known_hook_event_accepts_canonical_three() {
        assert_eq!(known_hook_event("PreToolUse"), Some("PreToolUse"));
        assert_eq!(known_hook_event("PostToolUse"), Some("PostToolUse"));
        assert_eq!(
            known_hook_event("UserPromptSubmit"),
            Some("UserPromptSubmit")
        );
    }

    #[test]
    fn known_hook_event_rejects_spoofed_and_typos() {
        // Suffix that defeats string equality — this is the actual M1 bug.
        assert_eq!(known_hook_event("PreToolUseFOO"), None);
        // Substring / prefix variants — none should slip through.
        assert_eq!(known_hook_event("PreToolUse "), None);
        assert_eq!(known_hook_event(" PreToolUse"), None);
        assert_eq!(
            known_hook_event("pretooluse"),
            None,
            "case-sensitive on purpose"
        );
        assert_eq!(known_hook_event(""), None);
        assert_eq!(known_hook_event("PreToolUse\nPostToolUse"), None);
        assert_eq!(known_hook_event("../../../etc/passwd"), None);
    }

    proptest::proptest! {
        /// `known_hook_event` accepts EXACTLY the canonical three strings
        /// and nothing else.  Any other input — typo, prefix, suffix, case
        /// variant, control char, arbitrary Unicode — must return `None`.
        /// This is the property that closes the M1 fail-closed-bypass
        /// regression: an attacker cannot smuggle a near-miss through the
        /// JSON `hook_event_name` field.
        #[test]
        fn prop_known_hook_event_only_accepts_canonical(s in ".{0,80}") {
            let canonical = matches!(s.as_str(),
                "PreToolUse" | "PostToolUse" | "UserPromptSubmit");
            if canonical {
                proptest::prop_assert!(known_hook_event(&s).is_some());
            } else {
                proptest::prop_assert_eq!(known_hook_event(&s), None,
                    "non-canonical {:?} must be rejected", s);
            }
        }
    }

    #[test]
    fn test_highest_severity_picks_block() {
        let (_store, dir) = temp_db();
        let matched = vec![
            (mk_guardrail(1, "alembic", "prefers", "autogenerate"), 90u8),
            (mk_guardrail(2, "git", "never", "force-push to main"), 100u8),
            (
                mk_guardrail(3, "cargo", "always", "test before commit"),
                80u8,
            ),
        ];
        // No rule_intent rows → falls back to predicate-derived severity.
        assert_eq!(highest_severity(&matched), Severity::Block);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_highest_severity_respects_store_override() {
        let (store, dir) = temp_db();
        // Seed a rule via the public API, then override its intent with Inform
        // severity even though the predicate ("never") would normally derive Block.
        let triple = crate::parser::Triple {
            subject: "noisy".to_string(),
            predicate: "never".to_string(),
            object: "something minor".to_string(),
            confidence: 0.9,
            domain: "test".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: Some(1),
            expires_at: None,
            noenrich: false,

            tier: None,

            source_label: None,
        };
        store
            .upsert_file("CLAUDE.md", "x", &[triple], "test")
            .unwrap();
        let tid = store.load_guardrails().unwrap()[0].triple_id;

        let intent = RuleIntent {
            action: crate::intent::Action::General,
            timing: crate::intent::Timing::ToolCall,
            tools: vec!["*".to_string()],
            allow_inverse: false,
            enriched_by: "manual".to_string(),
            severity: Severity::Inform,
        };
        store.upsert_rule_intent(tid, &intent).unwrap();

        // Re-load via the LEFT JOIN so `g.intent` carries the just-written row;
        // before the JOIN refactor this test pulled intent via a separate
        // `get_rule_intent` call that's no longer on the hot path.
        let g = store
            .load_guardrails()
            .unwrap()
            .into_iter()
            .find(|g| g.triple_id == tid)
            .expect("rule we just inserted should reload");
        let matched = vec![(g, 100u8)];
        // Store override demotes the Block to Inform.
        assert_eq!(highest_severity(&matched), Severity::Inform);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_deny_reason_mentions_source_and_rule() {
        let (_store, dir) = temp_db();
        let matched = vec![(mk_guardrail(1, "git", "never", "force-push to main"), 100u8)];
        let reason = deny_reason(&matched);
        assert!(
            reason.contains("never"),
            "reason should quote the predicate: {reason:?}"
        );
        assert!(
            reason.contains("force-push"),
            "reason should quote the object: {reason:?}"
        );
        assert!(
            reason.contains("CLAUDE.md"),
            "reason should cite source: {reason:?}"
        );
        // Line number is included when present — `mk_guardrail` sets line 42.
        assert!(
            reason.contains(":42"),
            "reason should include :line_number: {reason:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_deny_reason_omits_line_when_unknown() {
        let mut g = mk_guardrail(1, "git", "never", "force-push");
        g.line_start = None;
        let matched = vec![(g, 100u8)];
        let reason = deny_reason(&matched);
        assert!(
            reason.contains("CLAUDE.md"),
            "still cites source: {reason:?}"
        );
        // No line:N suffix.  Look specifically for a colon followed by a
        // digit inside the source citation rather than rejecting any colon
        // (the `Arai:` prefix is fine).
        assert!(
            !reason.contains("CLAUDE.md:"),
            "no colon-suffix when line is unknown: {reason:?}"
        );
    }

    #[test]
    fn test_deny_reason_skips_non_blocking() {
        let (_store, dir) = temp_db();
        let matched = vec![
            (mk_guardrail(1, "cargo", "prefers", "small commits"), 70u8),
            (mk_guardrail(2, "git", "never", "force-push to main"), 100u8),
        ];
        let reason = deny_reason(&matched);
        // Should pick the `never` rule, not the `prefers` rule.
        assert!(
            reason.contains("force-push"),
            "picked wrong rule: {reason:?}"
        );
        assert!(
            !reason.contains("small commits"),
            "non-block rule leaked: {reason:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_load_guardrails_attaches_intent_in_one_query() {
        // After `upsert_rule_intent`, a subsequent `load_guardrails` should
        // populate `Guardrail.intent` directly via the LEFT JOIN — no extra
        // round trip needed.  This is the mechanism the hot path relies on.
        let (store, dir) = temp_db();
        let triple = crate::parser::Triple {
            subject: "git".to_string(),
            predicate: "never".to_string(),
            object: "force-push to main".to_string(),
            confidence: 0.92,
            domain: "test".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: Some(1),
            expires_at: None,
            noenrich: false,

            tier: None,

            source_label: None,
        };
        store
            .upsert_file("CLAUDE.md", "x", &[triple], "test")
            .unwrap();

        // Pre-classification: intent should be None on every guardrail.
        let pre = store.load_guardrails().unwrap();
        assert_eq!(pre.len(), 1);
        assert!(pre[0].intent.is_none(), "no rule_intent yet → None");

        // Classify, re-load, intent should be Some.
        let tid = pre[0].triple_id;
        let intent = RuleIntent {
            action: crate::intent::Action::General,
            timing: crate::intent::Timing::ToolCall,
            tools: vec!["Bash".to_string()],
            allow_inverse: false,
            enriched_by: "test".to_string(),
            severity: Severity::Block,
        };
        store.upsert_rule_intent(tid, &intent).unwrap();
        let post = store.load_guardrails().unwrap();
        let attached = post[0].intent.as_ref().expect("intent should be attached");
        assert_eq!(attached.severity, Severity::Block);
        assert_eq!(attached.tools, vec!["Bash".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Issue #86: `arai why "git push --force origin main"` was denied by the
    /// very rule it was being asked to explain.  `arai severity "git push"
    /// block` (rule management) was denied the same way.  Both should now
    /// be exempted at the hook gate — they are diagnostic / configuration
    /// commands operating on the rule set, not actions the rule set governs.
    #[test]
    fn arai_self_command_recognised_by_first_token() {
        // Bare invocations
        assert!(is_arai_self_command(
            &serde_json::json!({ "command": "arai why \"git push\"" })
        ));
        assert!(is_arai_self_command(
            &serde_json::json!({ "command": "arai severity \"git push\" block" })
        ));
        assert!(is_arai_self_command(
            &serde_json::json!({ "command": "arai status" })
        ));
        assert!(is_arai_self_command(
            &serde_json::json!({ "command": "arai" })
        ));

        // Path-prefixed invocations
        assert!(is_arai_self_command(
            &serde_json::json!({ "command": "/usr/local/bin/arai why x" })
        ));
        assert!(is_arai_self_command(
            &serde_json::json!({ "command": "./arai add 'Never X'" })
        ));
        // Windows-style separator + .exe suffix
        assert!(is_arai_self_command(
            &serde_json::json!({ "command": "C:\\bin\\arai.exe status" })
        ));

        // Negatives
        assert!(!is_arai_self_command(
            &serde_json::json!({ "command": "git push --force origin main" })
        ));
        assert!(!is_arai_self_command(
            &serde_json::json!({ "command": "echo arai" })
        ));
        // Compose with arai in the middle of a pipeline — only the first
        // segment counts.  `git status && arai why` is still a `git`
        // command from the rule-engine's point of view.
        assert!(!is_arai_self_command(
            &serde_json::json!({ "command": "git status && arai why x" })
        ));
        assert!(!is_arai_self_command(&serde_json::json!({ "command": "" })));
        assert!(!is_arai_self_command(&serde_json::json!({})));
    }

    /// End-to-end regression for issue #86: a rule with single-token subject
    /// `Git` extracted from "Git never: git push to main without a PR" must
    /// NOT fire on read-only `git` subcommands.  The fix has two layers:
    /// token-boundary subject matching closes the substring leak, and
    /// dropping verb-mismatched rules closes the "git push rule blocks git
    /// status" failure.
    #[test]
    fn issue_86_git_push_rule_does_not_fire_on_read_only_subcommands() {
        let (store, dir) = temp_db();
        let triple = crate::parser::Triple {
            subject: "Git".to_string(),
            predicate: "never".to_string(),
            object: "git push to main without a PR".to_string(),
            confidence: 0.95,
            domain: "test".to_string(),
            source_file: "manual".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: Some(1),
            expires_at: None,
            noenrich: false,

            tier: None,

            source_label: None,
        };
        store
            .upsert_file("manual", "x", &[triple], "manual")
            .unwrap();
        store.classify_all_guardrails().unwrap();
        let rules = store.load_guardrails().unwrap();

        // The push rule SHOULD still fire on `git push --force origin main`.
        let push_terms = vec![
            "git".to_string(),
            "push".to_string(),
            "force".to_string(),
            "origin".to_string(),
            "main".to_string(),
        ];
        let push_phrases = vec!["git push".to_string()];
        let matched =
            guardrails::match_guardrails(&rules, &push_terms, &push_phrases, "Bash", "PreToolUse");
        assert_eq!(matched.len(), 1, "push rule must fire on the push command");

        // Read-only subcommands MUST NOT fire it.
        for (read_only, phrase) in [
            (vec!["git".to_string(), "status".to_string()], "git status"),
            (vec!["git".to_string(), "diff".to_string()], "git diff"),
            (vec!["git".to_string(), "log".to_string()], "git log"),
        ] {
            let phrases = vec![phrase.to_string()];
            let matched =
                guardrails::match_guardrails(&rules, &read_only, &phrases, "Bash", "PreToolUse");
            assert!(
                matched.is_empty(),
                "git push rule must not fire on read-only `{read_only:?}` (issue #86)"
            );
        }

        // `gh issue create --title "...git-level..."` extracts `git-level` as
        // a single token.  Subject "Git" must not match it via substring.
        let gh_terms = vec![
            "gh".to_string(),
            "issue".to_string(),
            "create".to_string(),
            "title".to_string(),
            "git-level".to_string(),
            "scope".to_string(),
        ];
        let gh_phrases = vec!["gh issue".to_string()];
        let matched =
            guardrails::match_guardrails(&rules, &gh_terms, &gh_phrases, "Bash", "PreToolUse");
        assert!(
            matched.is_empty(),
            "git push rule must not fire on a `gh issue create` whose title contains `git-level` (issue #86)"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_is_instruction_file_known_basenames() {
        // The canonical filenames Arai's discovery layer picks up across
        // Claude Code, Cursor, Windsurf, and Copilot.  These must trigger
        // a rescan no matter where in the tree they live.
        for path in [
            "/home/dev/project/CLAUDE.md",
            "C:\\Users\\dev\\project\\CLAUDE.md",
            "/repo/.cursorrules",
            "/repo/.windsurfrules",
            "/repo/.github/copilot-instructions.md",
        ] {
            assert!(
                is_instruction_file(path),
                "{path} should be classed as an instruction file"
            );
        }
    }

    #[test]
    fn test_is_instruction_file_rule_dirs() {
        // Per-project rules dirs under .claude/rules and .cursor/rules
        // are matched on path substring so nested layouts (workspace +
        // package) still rescan.
        for path in [
            "/repo/.claude/rules/security.md",
            "/repo/packages/api/.claude/rules/auth.md",
            "/repo/.cursor/rules/style.md",
            "C:\\repo\\.cursor\\rules\\style.md",
        ] {
            assert!(
                is_instruction_file(path),
                "{path} should be classed as a rules-dir instruction file"
            );
        }
    }

    #[test]
    fn test_is_instruction_file_memory_files() {
        // Per-project Claude Code memory files (the auto-memory the
        // assistant maintains).
        let path = "/home/tim/.claude/projects/some-slug/memory/feedback_xyz.md";
        assert!(is_instruction_file(path));
    }

    #[test]
    fn test_is_instruction_file_negative_cases() {
        // Files Arai must NOT spend a scan on — the common build / source
        // / test files that show up in FileChanged firings.  A false
        // positive here means we spawn `arai scan` on every unrelated
        // edit, which we explicitly avoid.
        for path in [
            "",
            "/repo/src/main.rs",
            "/repo/Cargo.toml",
            "/repo/README.md", // README is not an instruction file
            "/repo/.git/HEAD",
            "/repo/target/debug/build.log",
            "/repo/.claude/settings.json", // settings, not rules
            "/repo/notes/CLAUDE.txt",      // wrong extension
        ] {
            assert!(
                !is_instruction_file(path),
                "{path} should NOT be classed as an instruction file"
            );
        }
    }

    #[test]
    fn test_known_hook_event_covers_new_observability_events() {
        // Regression guard: the fail-closed wrapper uses this allow-list
        // to decide whether to propagate the event name to `event_hint`.
        // Dropping any of these would cause their stdouts to be treated
        // as PreToolUse-default in the panic path, which is the wrong
        // fail mode for observability events.
        assert_eq!(known_hook_event("FileChanged"), Some("FileChanged"));
        assert_eq!(
            known_hook_event("InstructionsLoaded"),
            Some("InstructionsLoaded")
        );
        assert_eq!(known_hook_event("CwdChanged"), Some("CwdChanged"));
        assert_eq!(known_hook_event("PostToolBatch"), Some("PostToolBatch"));
        assert_eq!(
            known_hook_event("PermissionDenied"),
            Some("PermissionDenied")
        );
        assert_eq!(known_hook_event("BogusEvent"), None);
    }

    // ---------------------------------------------------------------------------
    // Grok exit-code tests (AC1–AC6)
    // ---------------------------------------------------------------------------

    /// AC1: Grok + PreToolUse + deny → exit 2.
    /// AC2: Grok + PreToolUse + allow → exit 0.
    /// AC3: Claude + PreToolUse + deny → exit 0 (regression guard: never non-zero
    ///      for Claude).
    /// AC6: Grok + PostToolUse/UserPromptSubmit → exit 0.
    /// These are all pure table-driven tests against `desired_exit_code`.
    #[test]
    fn desired_exit_code_truth_table() {
        // AC1: Grok PreToolUse deny → 2
        assert_eq!(
            desired_exit_code(Host::Grok, "PreToolUse", true),
            2,
            "AC1: Grok PreToolUse deny must be exit 2"
        );
        // AC2: Grok PreToolUse allow → 0
        assert_eq!(
            desired_exit_code(Host::Grok, "PreToolUse", false),
            0,
            "AC2: Grok PreToolUse allow must be exit 0"
        );
        // AC3: Claude PreToolUse deny → 0 (hard regression guard)
        assert_eq!(
            desired_exit_code(Host::Claude, "PreToolUse", true),
            0,
            "AC3: Claude PreToolUse deny must be exit 0"
        );
        assert_eq!(
            desired_exit_code(Host::Unknown, "PreToolUse", true),
            0,
            "AC3-variant: Unknown host PreToolUse deny must be exit 0"
        );
        // AC4/AC5: error-path host detection feeds into desired_exit_code with
        // deny_outcome=true, event="PreToolUse".  Grok error → 2, Claude → 0.
        assert_eq!(
            desired_exit_code(Host::Grok, "PreToolUse", true),
            2,
            "AC4: Grok error path PreToolUse → exit 2"
        );
        assert_eq!(
            desired_exit_code(Host::Claude, "PreToolUse", true),
            0,
            "AC5: Claude error path PreToolUse → exit 0"
        );
        // AC6: Grok PostToolUse/UserPromptSubmit → 0 regardless of deny
        assert_eq!(
            desired_exit_code(Host::Grok, "PostToolUse", true),
            0,
            "AC6: Grok PostToolUse deny must be exit 0"
        );
        assert_eq!(
            desired_exit_code(Host::Grok, "PostToolUse", false),
            0,
            "AC6: Grok PostToolUse allow must be exit 0"
        );
        assert_eq!(
            desired_exit_code(Host::Grok, "UserPromptSubmit", true),
            0,
            "AC6: Grok UserPromptSubmit deny must be exit 0"
        );
        assert_eq!(
            desired_exit_code(Host::Grok, "UserPromptSubmit", false),
            0,
            "AC6: Grok UserPromptSubmit allow must be exit 0"
        );
        // Exhaustive: allow is always 0 for every host/event except Grok+Pre+deny
        for host in [Host::Grok, Host::Claude, Host::Unknown] {
            for event in ["PreToolUse", "PostToolUse", "UserPromptSubmit"] {
                for deny in [false, true] {
                    let code = desired_exit_code(host, event, deny);
                    let expect_2 = host == Host::Grok && event == "PreToolUse" && deny;
                    assert_eq!(
                        code,
                        if expect_2 { 2 } else { 0 },
                        "desired_exit_code({host:?}, {event}, {deny}) wrong"
                    );
                }
            }
        }
    }

    /// Grok host detection via `GROK_HOOK_EVENT` and `GROK_SESSION_ID`.
    /// Serialised via a static mutex to avoid races with other env-var tests.
    /// Uses only std — no new crate dependency introduced.
    #[test]
    fn detect_host_grok_env_vars() {
        // Serialise all env-var-dependent tests in this module.
        static ENV_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let mutex = ENV_MUTEX.get_or_init(|| std::sync::Mutex::new(()));
        let _guard = mutex.lock().unwrap_or_else(|p| p.into_inner());

        // Ensure clean state first.
        std::env::remove_var("GROK_HOOK_EVENT");
        std::env::remove_var("GROK_SESSION_ID");
        std::env::remove_var("CLAUDE_PROJECT_DIR");
        std::env::remove_var("CLAUDE_PLUGIN_ROOT");

        // Neither Grok nor Claude env vars set → Unknown (no hook_event_name in Null).
        assert_eq!(detect_host(&Value::Null), Host::Unknown);

        // GROK_HOOK_EVENT set → Grok (AC1, AC4 precondition).
        std::env::set_var("GROK_HOOK_EVENT", "PreToolUse");
        assert_eq!(detect_host(&Value::Null), Host::Grok);
        std::env::remove_var("GROK_HOOK_EVENT");

        // GROK_SESSION_ID set → Grok.
        std::env::set_var("GROK_SESSION_ID", "abc123");
        assert_eq!(detect_host(&Value::Null), Host::Grok);
        std::env::remove_var("GROK_SESSION_ID");

        // Neither set → not Grok; Claude detection via hook_event_name field
        // (AC3, AC5 precondition: no GROK_* vars → Claude or Unknown host).
        let with_hook_field = serde_json::json!({ "hook_event_name": "PreToolUse" });
        assert_eq!(detect_host(&with_hook_field), Host::Claude);

        // Neither GROK nor CLAUDE vars; no hook_event_name → Unknown.
        assert_eq!(detect_host(&Value::Null), Host::Unknown);
    }
}
