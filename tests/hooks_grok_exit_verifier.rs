//! Verifier integration tests for the hooks-grok-exit contract (v1).
//!
//! These tests are written by the Taniwha verifier role independently of the
//! implementor's tests.  Each test references the acceptance criterion it
//! exercises and uses a fresh tmp dir so tests are fully isolated.
//!
//! AC1: Grok + PreToolUse + Block rule match → exit 2 + `"decision":"deny"` stdout
//! AC2: Grok + PreToolUse + allow (no block rule) → exit 0
//! AC3: Claude + PreToolUse + Block deny → exit 0 + `"permissionDecision":"deny"` (regression guard)
//! AC4: Grok + malformed/oversize stdin → exit 2 + Grok-shaped deny stdout
//! AC5: Claude + malformed/oversize stdin → exit 0 + Claude-shaped deny stdout
//! AC6: Grok + PostToolUse/UserPromptSubmit → exit 0 regardless of rule matches

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

/// Build a fresh isolated tmp project (with .git) + arai base dir.
fn fresh_env(label: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_verif_exit_{label}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let arai_base = root.join("arai_base");
    std::fs::create_dir_all(project.join(".git")).expect("create project dir");
    std::fs::create_dir_all(&arai_base).expect("create arai_base dir");
    (project, arai_base)
}

/// Seed a Block-severity rule via `arai add` so the match pipeline will deny.
/// The rule "Never run git push --force" triggers on tool_name=Bash + command
/// containing "git push --force".
fn seed_block_rule(project: &Path, arai_base: &Path) {
    let output = Command::new(arai_bin())
        .args(["add", "Never run git push --force"])
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawn arai add");
    assert!(
        output.status.success(),
        "arai add failed — cannot seed block rule: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Pipe a JSON payload to `arai guardrails --match-stdin` with the given
/// environment overrides; returns (stdout, stderr, exit_code).
fn run_hook(
    payload: &str,
    project: &Path,
    arai_base: &Path,
    extra_env: &[(&str, &str)],
) -> (String, String, i32) {
    let mut cmd = Command::new(arai_bin());
    cmd.args(["guardrails", "--match-stdin"])
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        // Remove all Grok and Claude env vars from the inherited environment
        // so each test starts clean.
        .env_remove("GROK_HOOK_EVENT")
        .env_remove("GROK_SESSION_ID")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn arai guardrails");
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(payload.as_bytes()).expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait for arai");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

// ─── AC1 ────────────────────────────────────────────────────────────────────

/// AC1: Grok host + PreToolUse + Block-severity rule match →
///   stdout contains `"decision":"deny"` AND process exits with code 2.
///
/// Seeds an `arai add "Never run git push --force"` rule (Block severity),
/// then fires a PreToolUse hook payload matching that rule under GROK_HOOK_EVENT.
#[test]
fn ac1_grok_pretooluse_block_deny_exits_2_with_grok_deny_stdout() {
    let (project, arai_base) = fresh_env("ac1");
    seed_block_rule(&project, &arai_base);

    let payload = r#"{
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "git push --force origin main" },
        "session_id": "verif-ac1-session"
    }"#;

    let (stdout, stderr, code) = run_hook(
        payload,
        &project,
        &arai_base,
        &[("GROK_HOOK_EVENT", "PreToolUse")],
    );

    // AC1 exit code requirement
    assert_eq!(
        code, 2,
        "AC1: Grok PreToolUse Block deny must exit 2; stderr={stderr:?} stdout={stdout:?}"
    );
    // AC1 stdout shape requirement: Grok-shaped deny
    assert!(
        stdout.contains(r#""decision":"deny""#) || stdout.contains(r#""decision": "deny""#),
        "AC1: stdout must contain Grok-shaped decision:deny; got stdout={stdout:?}"
    );
    // Must NOT contain Claude shape on Grok path
    assert!(
        !stdout.contains("permissionDecision"),
        "AC1: Grok path must not emit Claude-shaped permissionDecision; got stdout={stdout:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ─── AC2 ────────────────────────────────────────────────────────────────────

/// AC2: Grok host + PreToolUse + no Block rule match (allow) → exit 0.
///
/// Uses a fresh project with no rules seeded so `matched.is_empty()` → allow.
#[test]
fn ac2_grok_pretooluse_allow_exits_0() {
    let (project, arai_base) = fresh_env("ac2");
    // No rules seeded → no match → allow path.

    let payload = r#"{
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "ls -la" },
        "session_id": "verif-ac2-session"
    }"#;

    let (_stdout, stderr, code) = run_hook(
        payload,
        &project,
        &arai_base,
        &[("GROK_HOOK_EVENT", "PreToolUse")],
    );

    assert_eq!(
        code, 0,
        "AC2: Grok PreToolUse allow must exit 0; stderr={stderr:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ─── AC3 ────────────────────────────────────────────────────────────────────

/// AC3: Claude host + PreToolUse + Block deny → exit 0 AND
///   stdout contains `"permissionDecision":"deny"` (regression guard).
///
/// Claude Code treats any non-zero exit as "hook broken"; this must always be 0.
#[test]
fn ac3_claude_pretooluse_block_deny_exits_0_with_claude_deny_stdout() {
    let (project, arai_base) = fresh_env("ac3");
    seed_block_rule(&project, &arai_base);

    let payload = r#"{
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "git push --force origin main" },
        "session_id": "verif-ac3-session"
    }"#;

    // Claude host: no GROK_* vars; presence of hook_event_name in payload
    // triggers Claude detection. We also explicitly remove GROK vars (done
    // in run_hook by default).
    let (stdout, stderr, code) = run_hook(payload, &project, &arai_base, &[]);

    // AC3 exit code: MUST be 0 for Claude (regression guard)
    assert_eq!(
        code, 0,
        "AC3: Claude PreToolUse Block deny must exit 0 (regression guard); stderr={stderr:?} stdout={stdout:?}"
    );
    // AC3 stdout: Claude-shaped deny
    assert!(
        stdout.contains(r#""permissionDecision":"deny""#)
            || stdout.contains(r#""permissionDecision": "deny""#),
        "AC3: Claude path must emit permissionDecision:deny; got stdout={stdout:?}"
    );
    // Must NOT emit exit 2 signal (already asserted above via code==0)
    // Must NOT use Grok shape
    assert!(
        !stdout.contains(r#""decision":"deny""#)
            || stdout.contains("permissionDecision"),
        "AC3: Grok deny shape must not appear on Claude path without permissionDecision present; stdout={stdout:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ─── AC4 ────────────────────────────────────────────────────────────────────

/// AC4: Grok + malformed stdin (invalid JSON) on PreToolUse event hint →
///   stdout contains Grok-shaped `"decision":"deny"` AND process exits 2.
///
/// Fail-closed: an attacker who can induce a hook error must not slip past
/// Block rules.  On Grok host this must be exit 2.
#[test]
fn ac4_grok_malformed_json_exits_2_with_grok_deny() {
    let (project, arai_base) = fresh_env("ac4");

    // Deliberately truncated JSON (missing closing brace/quote) — handle_stdin_impl
    // returns Err; event_hint stays at "PreToolUse"; Grok host → exit 2.
    let payload =
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"x"#;

    let (stdout, stderr, code) = run_hook(
        payload,
        &project,
        &arai_base,
        &[("GROK_HOOK_EVENT", "PreToolUse")],
    );

    assert_eq!(
        code, 2,
        "AC4: Grok malformed-stdin PreToolUse must exit 2; stderr={stderr:?} stdout={stdout:?}"
    );
    assert!(
        stdout.contains(r#""decision":"deny""#) || stdout.contains(r#""decision": "deny""#),
        "AC4: Grok error path must emit Grok-shaped deny; got stdout={stdout:?}"
    );
    // The reason string must indicate an internal error (human-readable)
    assert!(
        stdout.contains("internal error") || stdout.contains("error"),
        "AC4: Grok error deny reason must indicate internal error; stdout={stdout:?}"
    );
    // Must NOT use Claude shape
    assert!(
        !stdout.contains("permissionDecision"),
        "AC4: Grok error path must not emit Claude permissionDecision shape; stdout={stdout:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

/// AC4 (oversize variant): Grok + stdin exceeding 1 MiB → exit 2 + Grok deny.
#[test]
fn ac4_grok_oversize_stdin_exits_2_with_grok_deny() {
    let (project, arai_base) = fresh_env("ac4os");

    // 1 MiB + 1 byte — exceeds MAX_HOOK_INPUT_BYTES.
    // Wrap in a JSON prefix so the hook reads it in one go.
    let mut payload = String::from(
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":""#,
    );
    payload.push_str(&"A".repeat(1024 * 1024));
    payload.push_str(r#""}}"#);

    let (stdout, stderr, code) = run_hook(
        &payload,
        &project,
        &arai_base,
        &[("GROK_HOOK_EVENT", "PreToolUse")],
    );

    assert_eq!(
        code, 2,
        "AC4 oversize: Grok oversize stdin PreToolUse must exit 2; stderr={stderr:?} stdout={stdout:?}"
    );
    assert!(
        stdout.contains(r#""decision":"deny""#) || stdout.contains(r#""decision": "deny""#),
        "AC4 oversize: Grok error path must emit Grok-shaped deny; got stdout={stdout:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ─── AC5 ────────────────────────────────────────────────────────────────────

/// AC5: Claude + malformed stdin (invalid JSON) → exit 0 + Claude-shaped deny.
///
/// Unchanged from pre-change behaviour: Claude always gets exit 0 even on error.
#[test]
fn ac5_claude_malformed_json_exits_0_with_claude_deny() {
    let (project, arai_base) = fresh_env("ac5");

    let payload =
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"x"#;

    // No GROK_* vars → Claude/Unknown host on error path.
    let (stdout, stderr, code) = run_hook(payload, &project, &arai_base, &[]);

    assert_eq!(
        code, 0,
        "AC5: Claude malformed-stdin must exit 0; stderr={stderr:?} stdout={stdout:?}"
    );
    assert!(
        stdout.contains(r#""permissionDecision":"deny""#)
            || stdout.contains(r#""permissionDecision": "deny""#),
        "AC5: Claude error path must emit Claude-shaped permissionDecision deny; got stdout={stdout:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

/// AC5 (oversize variant): Claude + oversize stdin → exit 0 + Claude deny.
#[test]
fn ac5_claude_oversize_stdin_exits_0_with_claude_deny() {
    let (project, arai_base) = fresh_env("ac5os");

    let mut payload = String::from(
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":""#,
    );
    payload.push_str(&"A".repeat(1024 * 1024));
    payload.push_str(r#""}}"#);

    let (stdout, stderr, code) = run_hook(&payload, &project, &arai_base, &[]);

    assert_eq!(
        code, 0,
        "AC5 oversize: Claude oversize stdin must exit 0; stderr={stderr:?} stdout={stdout:?}"
    );
    assert!(
        stdout.contains(r#""permissionDecision":"deny""#)
            || stdout.contains(r#""permissionDecision": "deny""#),
        "AC5 oversize: Claude error path must emit Claude-shaped deny; got stdout={stdout:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ─── AC6 ────────────────────────────────────────────────────────────────────

/// AC6: Grok host + PostToolUse → exit 0, regardless of rule matches.
///
/// Even if we seed a Block rule and the terms match, PostToolUse never exits 2.
#[test]
fn ac6_grok_posttooluse_exits_0() {
    let (project, arai_base) = fresh_env("ac6post");
    // Seed a block rule to verify it doesn't cause exit 2 on PostToolUse.
    seed_block_rule(&project, &arai_base);

    let payload = r#"{
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "git push --force origin main" },
        "tool_result": "done",
        "session_id": "verif-ac6-session"
    }"#;

    let (_stdout, stderr, code) = run_hook(
        payload,
        &project,
        &arai_base,
        &[("GROK_HOOK_EVENT", "PostToolUse")],
    );

    assert_eq!(
        code, 0,
        "AC6: Grok PostToolUse must always exit 0; stderr={stderr:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

/// AC6: Grok host + UserPromptSubmit → exit 0.
#[test]
fn ac6_grok_userprompsubmit_exits_0() {
    let (project, arai_base) = fresh_env("ac6prompt");
    // Seed a block rule to be thorough.
    seed_block_rule(&project, &arai_base);

    let payload = r#"{
        "hook_event_name": "UserPromptSubmit",
        "prompt": "git push --force origin main",
        "session_id": "verif-ac6p-session"
    }"#;

    let (_stdout, stderr, code) = run_hook(
        payload,
        &project,
        &arai_base,
        &[("GROK_HOOK_EVENT", "UserPromptSubmit")],
    );

    assert_eq!(
        code, 0,
        "AC6: Grok UserPromptSubmit must always exit 0; stderr={stderr:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ─── AC7 (structural) ───────────────────────────────────────────────────────
// AC7 is satisfied by `cargo test` passing — the test runner itself is the
// evidence.  No explicit test function is needed; a failing cargo test suite
// would mean AC7 fails.
