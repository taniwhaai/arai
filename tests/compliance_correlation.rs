//! Integration coverage for `compliance.rs` Pre/Post correlation.
//!
//! Drives the live `arai` binary end-to-end:
//!
//!   1. Seed a tmp project + tmp `ARAI_BASE_DIR`.
//!   2. `arai add` a rule with a prohibitive predicate.
//!   3. Pipe a PreToolUse hook payload that triggers the rule.
//!   4. Pipe a matching PostToolUse hook payload — either still showing
//!      the forbidden phrase (→ `Ignored` verdict) or showing a compliant
//!      command (→ `Obeyed` verdict).
//!   5. Run `arai audit --event=Compliance --json` and assert the verdict
//!      that landed in the local audit log.
//!
//! Why integration: the unit tests in `src/compliance.rs::evaluate` cover
//! the pure correlation function in isolation.  This test pins the
//! *pipeline* — hook ingest → session lookup → audit query → verdict
//! emit — so a future refactor that re-routes any of those stages can't
//! silently break the user-visible audit field that compliance reporting
//! depends on (e.g. `arai stats --by-rule`).

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Build a fresh tmp project root (with `.git` so `find_project_root`
/// stops there) and a fresh tmp arai base.  Returned paths are unique per
/// test invocation so concurrent `cargo test` runs don't collide.
fn fresh_env(label: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_compliance_{label}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let arai_base = root.join("arai_base");
    std::fs::create_dir_all(project.join(".git")).expect("create project");
    std::fs::create_dir_all(&arai_base).expect("create arai base");
    (project, arai_base)
}

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

/// Run `arai <args...>` with the project root as cwd and ARAI_BASE_DIR set.
/// Returns (stdout, stderr, exit_code).
fn run(args: &[&str], project: &Path, arai_base: &Path) -> (String, String, i32) {
    let output = Command::new(arai_bin())
        .args(args)
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        // Disable telemetry so the test doesn't write outside its tmp dir.
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawn arai");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// Pipe `payload` into `arai guardrails --match-stdin` and return stdout.
fn pipe_hook(payload: &str, project: &Path, arai_base: &Path) -> String {
    let mut child = Command::new(arai_bin())
        .arg("guardrails")
        .arg("--match-stdin")
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        // Force deny-mode on so the Pre firing is recorded with
        // decision="deny" — the audit log still records it either way,
        // but pinning this makes the test's expectations explicit.
        .env("ARAI_DENY_MODE", "on")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn arai guardrails");
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(payload.as_bytes()).expect("write payload");
    }
    let output = child.wait_with_output().expect("wait arai guardrails");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Read every Compliance audit entry written for the test project.
/// Parsed from `arai audit --event=Compliance --json` so we go through
/// the same query path users do.
fn read_compliance_entries(project: &Path, arai_base: &Path) -> Vec<Value> {
    let (stdout, stderr, code) = run(
        &["audit", "--event=Compliance", "--json", "--limit=100"],
        project,
        arai_base,
    );
    assert_eq!(
        code, 0,
        "arai audit exited non-zero: stderr={stderr} stdout={stdout}",
    );
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<Value>(l).unwrap_or_else(|e| panic!("non-JSON line: {e}\n{l}"))
        })
        .collect()
}

/// Collect every (outcome, predicate) tuple across every rule of every
/// Compliance entry — flattens the `payload.rules[]` array so tests can
/// assert on the shape directly.
fn flatten_outcomes(entries: &[Value]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in entries {
        let rules = entry
            .get("payload")
            .and_then(|p| p.get("rules"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        for r in rules {
            let outcome = r
                .get("outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let predicate = r
                .get("predicate")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            out.push((outcome, predicate));
        }
    }
    out
}

fn pre_payload(command: &str, session_id: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": command },
        "session_id": session_id,
    })
    .to_string()
}

fn post_payload(command: &str, session_id: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": command },
        "tool_result": "",
        "session_id": session_id,
    })
    .to_string()
}

/// Prohibitive rule + Post still containing the forbidden phrase →
/// Compliance verdict must be `ignored`.
#[test]
fn prohibitive_rule_post_contains_forbidden_phrase_records_ignored() {
    let (project, arai_base) = fresh_env("ignored");

    let (add_out, add_err, add_code) = run(
        &["add", "Never run alembic upgrade by hand"],
        &project,
        &arai_base,
    );
    assert_eq!(
        add_code, 0,
        "arai add failed: stdout={add_out} stderr={add_err}",
    );

    let session = "test-session-ignored-aaaaaaaa";

    // Pre: trigger the rule.
    let pre_out = pipe_hook(
        &pre_payload("alembic upgrade head", session),
        &project,
        &arai_base,
    );
    // We don't strictly need to assert on the Pre stdout — the audit log
    // is the source of truth — but a sanity check that *something*
    // happened keeps a parser regression from silently passing.
    assert!(
        pre_out.contains("permissionDecision"),
        "pre-stdout should carry a permissionDecision (parser/match regression?): {pre_out}",
    );

    // Post: still doing the forbidden thing.
    let _ = pipe_hook(
        &post_payload("alembic upgrade head", session),
        &project,
        &arai_base,
    );

    let entries = read_compliance_entries(&project, &arai_base);
    let outcomes = flatten_outcomes(&entries);
    assert!(
        outcomes.iter().any(|(o, p)| o == "ignored" && p == "never"),
        "expected an (ignored, never) outcome in compliance entries; got {outcomes:?}\nentries: {entries:#?}",
    );

    let _ = std::fs::remove_dir_all(&project);
    let _ = std::fs::remove_dir_all(&arai_base);
}

/// Prohibitive rule + Post NOT containing the forbidden phrase →
/// Compliance verdict must be `obeyed`.
#[test]
fn prohibitive_rule_post_compliant_records_obeyed() {
    let (project, arai_base) = fresh_env("obeyed");

    let (_o, _e, code) = run(
        &["add", "Never run alembic upgrade by hand"],
        &project,
        &arai_base,
    );
    assert_eq!(code, 0);

    let session = "test-session-obeyed-bbbbbbbb";

    // Pre: trigger the rule.
    let _ = pipe_hook(
        &pre_payload("alembic upgrade head", session),
        &project,
        &arai_base,
    );
    // Post: the model complied — no `alembic upgrade` in this call.
    let _ = pipe_hook(&post_payload("ls", session), &project, &arai_base);

    let entries = read_compliance_entries(&project, &arai_base);
    let outcomes = flatten_outcomes(&entries);
    assert!(
        outcomes.iter().any(|(o, p)| o == "obeyed" && p == "never"),
        "expected an (obeyed, never) outcome in compliance entries; got {outcomes:?}\nentries: {entries:#?}",
    );
    // And NOT an `ignored` for the same rule — that would mean the
    // matcher is over-counting.
    assert!(
        !outcomes.iter().any(|(o, p)| o == "ignored" && p == "never"),
        "should not record ignored when the Post is compliant; got {outcomes:?}",
    );

    let _ = std::fs::remove_dir_all(&project);
    let _ = std::fs::remove_dir_all(&arai_base);
}

/// PostToolUse with no preceding PreToolUse firing in the session must
/// NOT write a spurious Compliance entry.  Otherwise the per-rule
/// `obeyed/ignored/unclear` ratios would inflate from tool calls that
/// never matched a rule in the first place.
#[test]
fn post_without_pre_emits_no_compliance_entry() {
    let (project, arai_base) = fresh_env("nopre");

    let (_o, _e, code) = run(
        &["add", "Never run alembic upgrade by hand"],
        &project,
        &arai_base,
    );
    assert_eq!(code, 0);

    let session = "test-session-nopre-cccccccc";
    // Post only — no preceding Pre on this session.
    let _ = pipe_hook(
        &post_payload("alembic upgrade head", session),
        &project,
        &arai_base,
    );

    let entries = read_compliance_entries(&project, &arai_base);
    assert!(
        entries.is_empty(),
        "Post-without-Pre should produce no Compliance entry; got {entries:#?}",
    );

    let _ = std::fs::remove_dir_all(&project);
    let _ = std::fs::remove_dir_all(&arai_base);
}
