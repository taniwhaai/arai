//! Integration tests for the prompt-collector module wired into the
//! `UserPromptSubmit` hook path.
//!
//! Tests drive the live `arai` binary end-to-end:
//!   1. Seed a tmp project + tmp `ARAI_BASE_DIR`.
//!   2. Pipe a `UserPromptSubmit` hook payload whose `prompt` field matches at
//!      least one seed rule.
//!   3. Assert the expected behaviour in the audit log or the hook response.
//!
//! AC2: the collector is actually called on the UserPromptSubmit path (not
//!      just that the collector function works in isolation).
//! AC4: `arai audit --event=PromptMatch` filters correctly.
//! AC5: the hook response bytes are not mutated by the collector.

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

fn fresh_env(label: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_pc_{label}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let arai_base = root.join("arai_base");
    std::fs::create_dir_all(project.join(".git")).expect("create project");
    std::fs::create_dir_all(&arai_base).expect("create arai base");
    (project, arai_base)
}

fn run(args: &[&str], project: &Path, arai_base: &Path) -> (String, String, i32) {
    let output = Command::new(arai_bin())
        .args(args)
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
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

/// Pipe `payload` into `arai guardrails --match-stdin` and return stdout bytes.
fn pipe_hook_bytes(payload: &str, project: &Path, arai_base: &Path) -> Vec<u8> {
    let mut child = Command::new(arai_bin())
        .arg("guardrails")
        .arg("--match-stdin")
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn arai guardrails");
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(payload.as_bytes()).expect("write payload");
    }
    child.wait_with_output().expect("wait").stdout
}

/// Pipe `payload` into `arai guardrails --match-stdin` and return stdout as String.
fn pipe_hook(payload: &str, project: &Path, arai_base: &Path) -> String {
    String::from_utf8_lossy(&pipe_hook_bytes(payload, project, arai_base)).into_owned()
}

fn user_prompt_payload(prompt: &str, session_id: &str) -> String {
    serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "prompt": prompt,
        "session_id": session_id,
    })
    .to_string()
}

/// Read PromptMatch entries from the audit log via `arai audit --event=PromptMatch --json`.
fn read_prompt_match_entries(project: &Path, arai_base: &Path) -> Vec<Value> {
    let (stdout, stderr, code) = run(
        &["audit", "--event=PromptMatch", "--json", "--limit=100"],
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
            serde_json::from_str::<Value>(l)
                .unwrap_or_else(|e| panic!("non-JSON line: {e}\n{l}"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// AC2: UserPromptSubmit handler invokes the collector
// ---------------------------------------------------------------------------

/// AC2 — sending a UserPromptSubmit payload whose prompt matches a seed rule
/// causes a PromptMatch entry to appear in the audit log.
#[test]
fn ac2_userpromptsubmit_handler_invokes_collector_and_writes_audit_entry() {
    let (project, arai_base) = fresh_env("ac2");

    // Prompt text that matches the "deploy" seed rule.
    let session = "test-session-ac2-aaaaaa";
    let prompt = "Please deploy to production now";
    let payload = user_prompt_payload(prompt, session);

    // Drive the full UserPromptSubmit handler path.
    let _ = pipe_hook(&payload, &project, &arai_base);

    // The hook should have written at least one PromptMatch audit entry.
    let entries = read_prompt_match_entries(&project, &arai_base);
    assert!(
        !entries.is_empty(),
        "expected at least one PromptMatch audit entry; audit is empty. \
         Did the UserPromptSubmit handler invoke the collector? \
         prompt={prompt:?}"
    );

    // Verify the entry has the correct event field.
    // The entry is wrapped as a record_event call, so the top-level `event`
    // is "PromptMatch" and the payload contains the receipt fields.
    let entry = &entries[0];
    assert_eq!(
        entry.get("event").and_then(|v| v.as_str()),
        Some("PromptMatch"),
        "top-level event field must be PromptMatch: {entry:#?}"
    );

    // The payload must contain a prompt_hash (64 hex chars) and matched_label.
    let payload_obj = entry
        .get("payload")
        .expect("entry must have a payload field");
    let prompt_hash = payload_obj
        .get("prompt_hash")
        .and_then(|v| v.as_str())
        .expect("payload must contain prompt_hash");
    assert_eq!(
        prompt_hash.len(),
        64,
        "prompt_hash must be 64 chars: {prompt_hash:?}"
    );
    assert!(
        prompt_hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "prompt_hash must be lowercase hex: {prompt_hash:?}"
    );

    let matched_label = payload_obj
        .get("matched_label")
        .and_then(|v| v.as_str())
        .expect("payload must contain matched_label");
    assert!(!matched_label.is_empty(), "matched_label must be non-empty");

    let did_follow = payload_obj.get("did_any_tool_call_follow");
    assert!(
        matches!(did_follow, Some(Value::Null)),
        "did_any_tool_call_follow must be null in v1: {did_follow:?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ---------------------------------------------------------------------------
// AC4: `arai audit --event=PromptMatch` filters correctly
// ---------------------------------------------------------------------------

/// AC4 — `arai audit --event=PromptMatch` shows only PromptMatch lines, not
/// other event kinds present in the same log.
///
/// Strategy: pipe a UserPromptSubmit payload that matches a seed rule (writes
/// PromptMatch entries via the binary), then also write a fixture JSONL line
/// with a different event kind directly into the same audit file.  Then assert
/// that `--event=PromptMatch` returns only PromptMatch lines.
#[test]
fn ac4_audit_event_filter_shows_only_prompt_match_lines() {
    let (project, arai_base) = fresh_env("ac4");

    // Step 1: emit PromptMatch entries by piping a UserPromptSubmit payload.
    let session = "test-session-ac4-bbbbbb";
    let prompt = "please deploy the secret to production";
    let payload = user_prompt_payload(prompt, session);
    let _ = pipe_hook(&payload, &project, &arai_base);

    // Step 2: determine the slug-derived audit directory path so we can write
    // a fixture line directly.  The slug is derived by the binary; we get it
    // by listing what was created under `arai_base/audit/`.
    let audit_root = arai_base.join("audit");
    let slug_dir = std::fs::read_dir(&audit_root)
        .expect("audit dir should exist after first hook call")
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .expect("should find one slug subdirectory")
        .path();

    // Step 3: write a fixture line with a different event kind into the same
    // day-bucket file.  We append as raw JSONL (no chain hash needed — the
    // query parser skips lines that fail to parse as JSON, but a valid JSON
    // line with a different event IS returned by `arai audit --json`).
    // Use the same day-file the binary is currently writing to.
    let today = {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let days = secs / 86400;
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
        format!("{y:04}{mo:02}{d:02}")
    };
    let day_file = slug_dir.join(format!("{today}.jsonl"));

    // Append a "Compliance"-event fixture line (a plausible non-PromptMatch
    // audit record) with valid JSON but no chain fields (they are not required
    // by the query reader — the verifier rejects them, but the query path
    // just filters on `event`).
    let fixture_line = serde_json::json!({
        "ts": "2026-05-25T00:00:00Z",
        "event": "Compliance",
        "tool": "Bash",
        "session": session,
        "payload": {
            "verdict": "Obeyed",
            "rules": []
        }
    })
    .to_string();
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&day_file)
            .expect("open day-file for fixture write");
        writeln!(f, "{fixture_line}").expect("write fixture line");
    }

    // Step 4: query only PromptMatch entries.
    let prompt_match_entries = read_prompt_match_entries(&project, &arai_base);

    assert!(
        !prompt_match_entries.is_empty(),
        "expected PromptMatch entries in audit log"
    );
    for entry in &prompt_match_entries {
        let ev = entry.get("event").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(
            ev, "PromptMatch",
            "every line returned by --event=PromptMatch must have event=PromptMatch: {entry:#?}"
        );
    }

    // Step 5: confirm the full audit log DOES contain a non-PromptMatch entry
    // (validates the fixture write and that the filter is actually doing work).
    let (all_stdout, _, _) = run(
        &["audit", "--json", "--limit=200"],
        &project,
        &arai_base,
    );
    let all_entries: Vec<Value> = all_stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    let has_non_prompt_match = all_entries.iter().any(|e| {
        e.get("event").and_then(|v| v.as_str()).unwrap_or("") != "PromptMatch"
    });
    assert!(
        has_non_prompt_match,
        "audit log should contain non-PromptMatch entries after fixture write; \
         all entries: {all_entries:#?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ---------------------------------------------------------------------------
// AC5: No hook-response mutation
// ---------------------------------------------------------------------------

/// AC5 — the serialised hook-response bytes before and after the collector
/// call are byte-identical (the collector must not mutate the hook response).
///
/// This test is located at the hook call site (the integration test that
/// drives the full handler), not inside the collector module, as specified
/// in the contract.
///
/// Implementation: send the same UserPromptSubmit payload twice.  The first
/// invocation goes through the full path (collector fires, record_event writes
/// to audit).  The second invocation is identical.  The stdout bytes must be
/// the same for both (modulo timestamps — so we compare shape, not raw bytes).
///
/// More precisely: the contract states that the hook response is not mutated
/// BY THE COLLECTOR.  Since we cannot split the binary into "before-collector"
/// and "after-collector" output, we instead verify:
///   (a) The collector fires (PromptMatch entry written).
///   (b) The hook response (stdout) for a UserPromptSubmit with no domain
///       guardrails is either empty or lacks any extra collector-injected
///       content (the response is determined solely by the domain-rules path).
///
/// When no domain rules are loaded (fresh project, no `arai add` or scan),
/// the UserPromptSubmit handler returns early with no stdout. The collector
/// still fires and writes audit entries, but the stdout must be empty.
/// That proves the hook-response bytes are not mutated by the collector.
#[test]
fn ac5_collector_does_not_mutate_hook_response_bytes() {
    let (project, arai_base) = fresh_env("ac5");

    // No rules added — the domain-rules path will exit early (empty domain_rules).
    // The collector may still fire on the seed ruleset.
    let session = "test-session-ac5-cccccc";
    let prompt = "please deploy the secret to production";
    let payload = user_prompt_payload(prompt, session);

    // First invocation: the collector fires, audit entry written.
    let response_bytes_1 = pipe_hook_bytes(&payload, &project, &arai_base);

    // Second invocation: same payload.
    let response_bytes_2 = pipe_hook_bytes(&payload, &project, &arai_base);

    // When no domain rules are loaded, the hook response must be empty.
    // The collector must not add anything to stdout.
    assert_eq!(
        response_bytes_1, b"",
        "hook response must be empty (no domain rules): got {:?}",
        String::from_utf8_lossy(&response_bytes_1)
    );
    assert_eq!(
        response_bytes_2, b"",
        "hook response must be empty on second call too: got {:?}",
        String::from_utf8_lossy(&response_bytes_2)
    );

    // The collector should still have written audit entries (proving it ran).
    let entries = read_prompt_match_entries(&project, &arai_base);
    assert!(
        !entries.is_empty(),
        "collector should have written PromptMatch entries even when response is empty"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}
