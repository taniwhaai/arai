//! End-to-end safety tests for the hook entry point.
//!
//! Spawns the arai binary as Claude Code would (`arai guardrails
//! --match-stdin`) with hand-crafted hook payloads, then asserts the
//! response on stdout matches the safety contract:
//!
//!   - PreToolUse with malformed input → fail-closed deny
//!   - PreToolUse with spoofed event_name → fail-closed deny
//!   - ARAI_DISABLED=1 → empty stdout (model proceeds), bypass entry written
//!
//! These complement the in-module unit tests, which exercise the matcher
//! and severity logic directly but don't see the wrapper-level stdout
//! contract.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn temp_arai_home() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "arai_hooks_safety_{}_{}",
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_hook(payload: &str, env: &[(&str, &str)]) -> (String, String, i32) {
    let bin = env!("CARGO_BIN_EXE_arai");
    let arai_home = temp_arai_home();
    let mut cmd = Command::new(bin);
    cmd.arg("guardrails")
        .arg("--match-stdin")
        .env("ARAI_HOME", &arai_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn arai");
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(payload.as_bytes()).expect("write payload");
    }
    let out = child.wait_with_output().expect("wait arai");
    let _ = std::fs::remove_dir_all(&arai_home);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// A PreToolUse payload with malformed inner JSON (missing closing brace)
/// must produce a deny response on stdout, not empty.  Empty stdout would
/// be read by Claude Code as "no objection" and the tool would proceed —
/// the bypass we explicitly closed in commit 15dcb96.
#[test]
fn pretooluse_malformed_json_produces_deny() {
    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"x"#; // truncated
    let (stdout, _stderr, code) = run_hook(payload, &[]);
    assert_eq!(code, 0, "hook must exit 0 even on error so Claude Code reads stdout");
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "expected deny in stdout, got: {stdout:?}"
    );
    assert!(
        stdout.contains("\"hookEventName\":\"PreToolUse\""),
        "expected PreToolUse in stdout, got: {stdout:?}"
    );
}

/// A spoofed `hook_event_name` like `"PreToolUseFOO"` used to defeat the
/// fail-closed wrapper because `event_hint` was overwritten with whatever
/// string the JSON contained.  The fix pins `event_hint` to PreToolUse for
/// any unknown event, so the deny still fires when the hook errors.  Here
/// we induce an error via an oversize input.
#[test]
fn spoofed_event_name_does_not_defeat_fail_closed() {
    // 1 MiB + 1 byte = oversize per MAX_HOOK_INPUT_BYTES; the hook errors
    // before it ever decides what to do.  The "event" claim in the JSON
    // is bogus on purpose.
    let mut payload = String::from(r#"{"hook_event_name":"PreToolUseFOO","tool_name":"Bash","tool_input":{"command":""#);
    payload.push_str(&"A".repeat(1024 * 1024));
    payload.push_str(r#""}}"#);
    let (stdout, _stderr, code) = run_hook(&payload, &[]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "spoofed event must not defeat fail-closed; got stdout: {stdout:?}"
    );
}

/// `ARAI_DISABLED=1` short-circuits the hook before any matching.  Stdout
/// is empty (model proceeds as if no hook were installed); the bypass
/// audit entry is written so post-hoc inspection can tell "Arai was off"
/// from "no rules fired".  Here we just assert the stdout contract
/// because the audit dir lives under ARAI_HOME which we delete.
#[test]
fn arai_disabled_short_circuits_with_empty_stdout() {
    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push --force"},"session_id":"s1"}"#;
    let (stdout, _stderr, code) = run_hook(payload, &[("ARAI_DISABLED", "1")]);
    assert_eq!(code, 0);
    assert!(
        stdout.trim().is_empty(),
        "ARAI_DISABLED should produce empty stdout, got: {stdout:?}"
    );
}

/// `ARAI_DISABLED` accepts the same set of truthy values our other env
/// flags do (1/true/on/yes).  An obviously-falsey value should NOT
/// short-circuit — confirms we haven't broken the regular path.
#[test]
fn arai_disabled_falsey_does_not_short_circuit() {
    // tool=Read hits the skip-tool fast exit, so stdout is empty even
    // without the disable.  Use a tool that DOES go through matching;
    // with no DB seeded, load_guardrails returns nothing, but the path
    // still runs to completion without error.
    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"},"session_id":"s1"}"#;
    let (stdout, _stderr, code) = run_hook(payload, &[("ARAI_DISABLED", "0")]);
    assert_eq!(code, 0);
    // No DB, no rules → empty stdout is correct here too.  The point of
    // this test is to confirm exit 0 and no panic, not the bypass path.
    assert!(stdout.trim().is_empty() || stdout.contains("permissionDecision"));
}
