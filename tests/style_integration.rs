//! Subprocess integration tests for the brand-palette-styling module.
//!
//! Asserts that ANSI escape bytes (0x1B) are absent from:
//!   - Any `--json` output (AC4)
//!   - Hook `guardrails --match-stdin` output (AC8)
//!   - Piped / non-terminal output (AC3)
//!   - `NO_COLOR=1` output (AC2)
//!
//! Uses `env!("CARGO_BIN_EXE_arai")` for the binary path and `ARAI_BASE_DIR`
//! for temp isolation.  No new dependencies.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Create a fresh temp directory for ARAI_BASE_DIR isolation.
fn temp_arai_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "arai_style_test_{label}_{}_{}",
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create a temp dir with a `.git` marker so `Config::load` finds a project root.
fn temp_project_dir(label: &str) -> PathBuf {
    let dir = temp_arai_dir(label);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

/// Run `arai <args>` with the given environment overrides (on top of a clean env),
/// capturing stdout and stderr.  `NO_COLOR` and `CLICOLOR_FORCE` are cleared
/// by default so test isolation is complete; callers that need one set pass it
/// in `env_extras`.
fn run_arai(
    args: &[&str],
    env_extras: &[(&str, &str)],
    stdin_payload: Option<&str>,
) -> (Vec<u8>, Vec<u8>, i32) {
    let bin = env!("CARGO_BIN_EXE_arai");
    let project_dir = temp_project_dir("run");
    let arai_base = temp_arai_dir("base");

    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("ARAI_BASE_DIR", &arai_base)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .current_dir(&project_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in env_extras {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().expect("spawn arai");
    if let Some(payload) = stdin_payload {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(payload.as_bytes());
        }
    }
    let out = child.wait_with_output().expect("wait arai");

    // Clean up temp dirs (best effort).
    let _ = std::fs::remove_dir_all(&project_dir);
    let _ = std::fs::remove_dir_all(&arai_base);

    (out.stdout, out.stderr, out.status.code().unwrap_or(-1))
}

/// Assert no byte with value 0x1B (ESC) appears in the given bytes.
fn assert_no_ansi(bytes: &[u8], context: &str) {
    assert!(
        !bytes.contains(&0x1B),
        "Found ANSI escape byte (0x1B) in {context}:\n{}",
        String::from_utf8_lossy(bytes)
    );
}

// ── AC4: every `--json` output contains zero ANSI escapes ────────────────────

#[test]
fn ac4_guardrails_json_has_no_ansi() {
    let (stdout, stderr, _) = run_arai(&["guardrails", "--json"], &[], None);
    assert_no_ansi(&stdout, "guardrails --json stdout");
    assert_no_ansi(&stderr, "guardrails --json stderr");
}

#[test]
fn ac4_stats_json_has_no_ansi() {
    let (stdout, stderr, _) = run_arai(&["stats", "--json"], &[], None);
    assert_no_ansi(&stdout, "stats --json stdout");
    assert_no_ansi(&stderr, "stats --json stderr");
}

#[test]
fn ac4_audit_json_has_no_ansi() {
    let (stdout, stderr, _) = run_arai(&["audit", "--json"], &[], None);
    assert_no_ansi(&stdout, "audit --json stdout");
    assert_no_ansi(&stderr, "audit --json stderr");
}

#[test]
fn ac4_why_json_has_no_ansi() {
    let (stdout, stderr, _) = run_arai(
        &["why", "--json", "git push --force origin main"],
        &[],
        None,
    );
    assert_no_ansi(&stdout, "why --json stdout");
    assert_no_ansi(&stderr, "why --json stderr");
}

#[test]
fn ac4_lint_json_has_no_ansi() {
    // Lint requires a file path; write a temp file.
    let tmp = std::env::temp_dir().join(format!("arai_lint_test_{}.md", std::process::id()));
    std::fs::write(&tmp, "- Never force-push to main\n").unwrap();
    let path_str = tmp.to_string_lossy().to_string();

    let (stdout, stderr, _) = run_arai(&["lint", &path_str, "--json"], &[], None);
    assert_no_ansi(&stdout, "lint --json stdout");
    assert_no_ansi(&stderr, "lint --json stderr");

    let _ = std::fs::remove_file(&tmp);
}

// ── AC8: hook guardrails --match-stdin output has zero ANSI escapes ──────────

#[test]
fn ac8_hook_match_stdin_no_ansi() {
    // A valid PreToolUse hook payload (no rules will match since this is an
    // isolated temp env with no guardrails, but the output JSON structure
    // must still be ANSI-free).
    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push --force origin main"},"session_id":"test-session"}"#;
    let (stdout, stderr, _) = run_arai(&["guardrails", "--match-stdin"], &[], Some(payload));
    assert_no_ansi(&stdout, "guardrails --match-stdin stdout");
    assert_no_ansi(&stderr, "guardrails --match-stdin stderr");
}

#[test]
fn ac8_hook_match_stdin_json_string_fields_no_ansi() {
    // Verify that all string fields in the JSON response are ANSI-free.
    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push --force origin main"},"session_id":"test-session"}"#;
    let (stdout, _stderr, _) = run_arai(&["guardrails", "--match-stdin"], &[], Some(payload));

    // Output must be valid JSON and have no 0x1B bytes in any field.
    assert_no_ansi(&stdout, "hook output");

    // If the output is non-empty, parse it and check string fields recursively.
    let output_str = String::from_utf8_lossy(&stdout);
    if !output_str.trim().is_empty() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(output_str.trim()) {
            check_json_string_fields_no_ansi(&v, "hook JSON response");
        }
    }
}

/// Recursively check that no string field in a JSON value contains 0x1B.
fn check_json_string_fields_no_ansi(v: &serde_json::Value, path: &str) {
    match v {
        serde_json::Value::String(s) => {
            assert!(
                !s.contains('\x1b'),
                "ANSI escape in JSON string field at {path}: {s:?}"
            );
        }
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                check_json_string_fields_no_ansi(val, &format!("{path}.{k}"));
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                check_json_string_fields_no_ansi(val, &format!("{path}[{i}]"));
            }
        }
        _ => {}
    }
}

// ── AC3: piped / non-terminal output has zero ANSI escapes ───────────────────
// stdout and stderr are piped (non-terminal) in all our Command invocations above,
// so every test above already covers this. The following test makes it explicit:

#[test]
fn ac3_piped_status_no_ansi() {
    // `arai status` piped through our Command (non-TTY sink) must not contain ANSI.
    let (stdout, stderr, _) = run_arai(&["status"], &[], None);
    assert_no_ansi(&stdout, "status (piped) stdout");
    assert_no_ansi(&stderr, "status (piped) stderr");
}

#[test]
fn ac3_piped_guardrails_no_ansi() {
    let (stdout, stderr, _) = run_arai(&["guardrails"], &[], None);
    assert_no_ansi(&stdout, "guardrails (piped) stdout");
    assert_no_ansi(&stderr, "guardrails (piped) stderr");
}

// ── AC2: NO_COLOR=1 produces zero ANSI output ────────────────────────────────

#[test]
fn ac2_no_color_status_no_ansi() {
    let (stdout, stderr, _) = run_arai(&["status"], &[("NO_COLOR", "1")], None);
    assert_no_ansi(&stdout, "status (NO_COLOR=1) stdout");
    assert_no_ansi(&stderr, "status (NO_COLOR=1) stderr");
}

#[test]
fn ac2_no_color_guardrails_no_ansi() {
    let (stdout, stderr, _) = run_arai(&["guardrails"], &[("NO_COLOR", "1")], None);
    assert_no_ansi(&stdout, "guardrails (NO_COLOR=1) stdout");
    assert_no_ansi(&stderr, "guardrails (NO_COLOR=1) stderr");
}

#[test]
fn ac2_no_color_hook_match_stdin_no_ansi() {
    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push"},"session_id":"s"}"#;
    let (stdout, stderr, _) = run_arai(
        &["guardrails", "--match-stdin"],
        &[("NO_COLOR", "1")],
        Some(payload),
    );
    assert_no_ansi(&stdout, "guardrails --match-stdin (NO_COLOR=1) stdout");
    assert_no_ansi(&stderr, "guardrails --match-stdin (NO_COLOR=1) stderr");
}
