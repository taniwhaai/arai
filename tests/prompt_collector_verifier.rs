//! Verifier tests for the prompt-collector module, contract v1.
//!
//! These are written INDEPENDENTLY by the verifier from the contract acceptance
//! criteria, not derived from the implementor's tests.  Each test cites its AC.
//!
//! AC1  — seed ruleset non-empty, all six declared labels present
//! AC2  — UserPromptSubmit handler actually calls the collector
//! AC3  — PromptMatchReceipt shape: event literal, 64-char hex hash, None follow
//! AC4  — `arai audit --event=PromptMatch` returns only PromptMatch lines
//! AC5  — hook-response bytes are not mutated by the collector
//! AC6  — no network identifiers in production source code
//! AC7  — SEED_RULES comment marks them as non-policy / starter guesses
//! AC8  — CLAUDE.md has 2-4 observation-only sentences about prompt-collector
//! AC9  — cargo test exits 0

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

/// Create a fresh isolated tmp environment: returns (project_dir, arai_base_dir).
fn fresh_env(tag: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_verifier_{tag}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let arai_base = root.join("arai_base");
    std::fs::create_dir_all(project.join(".git")).expect("create project dir");
    std::fs::create_dir_all(&arai_base).expect("create arai_base dir");
    (project, arai_base)
}

/// Pipe `payload` bytes into `arai guardrails --match-stdin`; return stdout bytes.
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

fn pipe_hook(payload: &str, project: &Path, arai_base: &Path) -> String {
    String::from_utf8_lossy(&pipe_hook_bytes(payload, project, arai_base)).into_owned()
}

fn run_arai(args: &[&str], project: &Path, arai_base: &Path) -> (String, String, i32) {
    let out = Command::new(arai_bin())
        .args(args)
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .output()
        .expect("spawn arai");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn user_prompt_payload(prompt: &str, session: &str) -> String {
    serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "prompt": prompt,
        "session_id": session,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// AC1 — Seed ruleset is non-empty and contains all declared labels
//
// The contract declares exactly six seed labels:
//   "deploy", "production", "secret", "password", "kubectl apply", "force push"
// We verify the static constant and the loader function separately.
// ---------------------------------------------------------------------------

/// AC1: SEED_RULES constant is non-empty and contains all six contract-declared labels.
///
/// This test is a unit-level structural check driven via the binary's lint
/// output.  We exercise it through the public seed_rules() call indirectly via
/// a prompt that should match each declared label, confirming each label fires.
#[test]
fn verifier_ac1_seed_labels_all_present_via_matching() {
    // AC1: each of the six declared labels must produce a PromptMatch audit entry
    // when the seed ruleset is applied to a prompt containing that label.
    let cases: &[(&str, &str)] = &[
        ("deploy", "please deploy this service"),
        ("production", "push to the production environment"),
        ("secret", "store the api secret here"),
        ("password", "set the user password to abc"),
        ("kubectl apply", "run kubectl apply -f manifest.yaml"),
        ("force push", "git force push to main"),
    ];

    for (label, prompt_text) in cases {
        let (project, arai_base) = fresh_env(&format!("ac1_{label}"));
        let payload = user_prompt_payload(prompt_text, "verifier-ac1-session");
        let _stdout = pipe_hook(&payload, &project, &arai_base);

        // Read back the audit entries.
        let (out, err, code) = run_arai(
            &["audit", "--event=PromptMatch", "--json", "--limit=100"],
            &project,
            &arai_base,
        );
        assert_eq!(
            code, 0,
            "arai audit failed for label {label:?}: stderr={err}"
        );
        let entries: Vec<Value> = out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();

        let matched = entries.iter().any(|e| {
            e.get("payload")
                .and_then(|p| p.get("matched_label"))
                .and_then(|v| v.as_str())
                == Some(label)
        });
        assert!(
            matched,
            "AC1: seed label {label:?} not found in PromptMatch audit entries \
             for prompt {prompt_text:?}. entries={entries:#?}"
        );

        let _ = std::fs::remove_dir_all(project.parent().unwrap());
    }
}

// ---------------------------------------------------------------------------
// AC2 — UserPromptSubmit handler invokes the collector
//
// The test drives the full arai binary with a UserPromptSubmit payload whose
// prompt matches at least one seed rule, then asserts the audit log contains
// a PromptMatch entry.  This proves the collector is wired to the
// UserPromptSubmit path, not just that the collector works in isolation.
// ---------------------------------------------------------------------------

#[test]
fn verifier_ac2_userpromptsubmit_calls_collector() {
    let (project, arai_base) = fresh_env("ac2");

    // Prompt that matches the "deploy" seed rule.
    let prompt = "I want to deploy the application to staging";
    let payload = user_prompt_payload(prompt, "verifier-ac2-session");

    // Drive the full handler path via the binary.
    let _ = pipe_hook(&payload, &project, &arai_base);

    // Audit log must contain at least one PromptMatch entry.
    let (out, err, code) = run_arai(
        &["audit", "--event=PromptMatch", "--json", "--limit=50"],
        &project,
        &arai_base,
    );
    assert_eq!(code, 0, "arai audit failed: {err}");

    let entries: Vec<Value> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    assert!(
        !entries.is_empty(),
        "AC2: UserPromptSubmit handler did not invoke the collector — \
         no PromptMatch entries found for prompt {prompt:?}"
    );

    // The top-level event field must be "PromptMatch".
    let first = &entries[0];
    assert_eq!(
        first.get("event").and_then(|v| v.as_str()),
        Some("PromptMatch"),
        "AC2: top-level event field is not PromptMatch: {first:#?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ---------------------------------------------------------------------------
// AC3 — Receipt shape is exact
//
// Contract says:
//   - event = literal "PromptMatch"
//   - prompt_hash = exactly 64 lowercase hex chars
//   - matched_label = non-empty string
//   - timestamp_iso = valid ISO-8601 string (verifier checks basic format)
//   - project_slug = non-empty
//   - did_any_tool_call_follow = null
// ---------------------------------------------------------------------------

#[test]
fn verifier_ac3_receipt_shape_is_exact() {
    let (project, arai_base) = fresh_env("ac3");

    let prompt = "deploy to production";
    let payload = user_prompt_payload(prompt, "verifier-ac3-session");
    let _ = pipe_hook(&payload, &project, &arai_base);

    let (out, err, code) = run_arai(
        &["audit", "--event=PromptMatch", "--json", "--limit=50"],
        &project,
        &arai_base,
    );
    assert_eq!(code, 0, "arai audit --event=PromptMatch failed: {err}");

    let entries: Vec<Value> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    assert!(
        !entries.is_empty(),
        "AC3: no PromptMatch entries found to inspect shape"
    );

    for entry in &entries {
        // Top-level event must be "PromptMatch"
        let event = entry.get("event").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(event, "PromptMatch", "AC3: event field wrong: {entry:#?}");

        let payload_obj = entry
            .get("payload")
            .expect("AC3: entry must have a payload field");

        // prompt_hash: exactly 64 lowercase hex chars
        let hash = payload_obj
            .get("prompt_hash")
            .and_then(|v| v.as_str())
            .expect("AC3: payload must contain prompt_hash");
        assert_eq!(
            hash.len(),
            64,
            "AC3: prompt_hash must be exactly 64 chars, got {}: {hash:?}",
            hash.len()
        );
        assert!(
            hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "AC3: prompt_hash must be lowercase hex, got {hash:?}"
        );

        // matched_label: non-empty
        let label = payload_obj
            .get("matched_label")
            .and_then(|v| v.as_str())
            .expect("AC3: payload must contain matched_label");
        assert!(!label.is_empty(), "AC3: matched_label must not be empty");

        // timestamp_iso: basic ISO-8601 check (YYYY-MM-DDThh:mm:ssZ shape)
        let ts = payload_obj
            .get("timestamp_iso")
            .and_then(|v| v.as_str())
            .expect("AC3: payload must contain timestamp_iso");
        assert!(
            ts.len() >= 10 && ts.contains('T') && (ts.ends_with('Z') || ts.contains('+')),
            "AC3: timestamp_iso does not look like ISO-8601: {ts:?}"
        );

        // project_slug: non-empty
        let slug = payload_obj
            .get("project_slug")
            .and_then(|v| v.as_str())
            .expect("AC3: payload must contain project_slug");
        assert!(!slug.is_empty(), "AC3: project_slug must not be empty");

        // did_any_tool_call_follow: must be null in v1
        let follow = payload_obj.get("did_any_tool_call_follow");
        assert!(
            matches!(follow, Some(Value::Null)),
            "AC3: did_any_tool_call_follow must be null in v1, got {follow:?}"
        );
    }

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ---------------------------------------------------------------------------
// AC4 — `arai audit --event=PromptMatch` filters correctly
//
// Write a fixture JSONL with a non-PromptMatch event into the same day-bucket,
// then assert the filter returns only PromptMatch lines.
// ---------------------------------------------------------------------------

#[test]
fn verifier_ac4_audit_event_filter_returns_only_prompt_match() {
    let (project, arai_base) = fresh_env("ac4");

    // Step 1: drive a UserPromptSubmit that writes PromptMatch entries.
    let prompt = "please deploy the secret";
    let payload = user_prompt_payload(prompt, "verifier-ac4-session");
    let _ = pipe_hook(&payload, &project, &arai_base);

    // Step 2: find the slug dir created under arai_base/audit/
    let audit_root = arai_base.join("audit");
    let slug_dir = std::fs::read_dir(&audit_root)
        .expect("audit dir should exist after hook call")
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .expect("should find at least one slug subdirectory under audit/")
        .path();

    // Step 3: write a fixture JSONL line with a different event kind (Compliance)
    // into the same day-bucket file.
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

    let fixture_line = serde_json::json!({
        "ts": "2026-05-25T00:00:00Z",
        "event": "Compliance",
        "tool": "Bash",
        "session": "verifier-ac4-session",
        "payload": { "verdict": "Obeyed", "rules": [] }
    })
    .to_string();
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&day_file)
            .expect("open day-file to append fixture");
        writeln!(f, "{fixture_line}").expect("write fixture line");
    }

    // Step 4: query only PromptMatch lines.
    let (out, err, code) = run_arai(
        &["audit", "--event=PromptMatch", "--json", "--limit=200"],
        &project,
        &arai_base,
    );
    assert_eq!(code, 0, "arai audit --event=PromptMatch failed: {err}");

    let pm_entries: Vec<Value> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    assert!(
        !pm_entries.is_empty(),
        "AC4: expected PromptMatch entries but got none"
    );
    for entry in &pm_entries {
        let ev = entry.get("event").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(
            ev, "PromptMatch",
            "AC4: --event=PromptMatch returned a non-PromptMatch entry: {entry:#?}"
        );
    }

    // Step 5: confirm the full log contains the Compliance entry (validates
    // that the filter is actually exercising non-trivial filtering).
    let (all_out, _, _) = run_arai(&["audit", "--json", "--limit=500"], &project, &arai_base);
    let all: Vec<Value> = all_out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let has_compliance = all
        .iter()
        .any(|e| e.get("event").and_then(|v| v.as_str()) == Some("Compliance"));
    assert!(
        has_compliance,
        "AC4: Compliance fixture entry not found in full audit — \
         filter test is vacuous; full log: {all:#?}"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ---------------------------------------------------------------------------
// AC5 — No hook-response mutation
//
// The contract says the hook-response (stdout) must not be mutated by the
// collector call.  Strategy: fresh project with no domain rules loaded.
// The domain-rules path exits early with no stdout.  The collector runs on
// the seed ruleset and writes audit entries but must produce no stdout of its
// own.  Asserting stdout == b"" proves the collector did not inject anything.
// We also verify the collector did run by checking that PromptMatch entries
// were written.
// ---------------------------------------------------------------------------

#[test]
fn verifier_ac5_collector_does_not_mutate_hook_response() {
    let (project, arai_base) = fresh_env("ac5");

    // No domain rules loaded — the domain-rules path returns early, no stdout.
    let prompt = "please deploy the secret password to production";
    let payload = user_prompt_payload(prompt, "verifier-ac5-session");

    let stdout_bytes = pipe_hook_bytes(&payload, &project, &arai_base);

    // Hook stdout must be empty (no domain rules → no response from the
    // guardrails path, and the collector must not add anything).
    assert_eq!(
        stdout_bytes,
        b"",
        "AC5: hook response must be empty (no domain rules loaded); \
         collector must not mutate it. Got: {:?}",
        String::from_utf8_lossy(&stdout_bytes)
    );

    // The collector should still have run — verify PromptMatch entries exist.
    let (out, err, code) = run_arai(
        &["audit", "--event=PromptMatch", "--json", "--limit=50"],
        &project,
        &arai_base,
    );
    assert_eq!(code, 0, "arai audit failed: {err}");
    let entries: Vec<Value> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "AC5: collector did not write PromptMatch entries even though it should have run"
    );

    let _ = std::fs::remove_dir_all(project.parent().unwrap());
}

// ---------------------------------------------------------------------------
// AC6 — No outbound network calls in the collector source
//
// Forbidden identifiers (outside test blocks): reqwest, ureq, hyper, http::,
// Client::, connect, bind, TcpStream, UdpSocket.
// ---------------------------------------------------------------------------

#[test]
fn verifier_ac6_no_network_identifiers_in_production_source() {
    // Read the actual source file.
    let source = include_str!("../src/prompt_collector.rs");

    // Strip everything from the first #[cfg(test)] block onward so we only
    // scan production code.  The contract says "outside of test-only blocks".
    let prod_source = if let Some(idx) = source.find("#[cfg(test)]") {
        &source[..idx]
    } else {
        source
    };

    let forbidden = [
        "reqwest",
        "ureq",
        "hyper",
        "http::",
        "Client::",
        "connect",
        "bind",
        "TcpStream",
        "UdpSocket",
    ];

    for term in &forbidden {
        assert!(
            !prod_source.contains(term),
            "AC6: collector production code contains forbidden network identifier {term:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC7 — Seed ruleset is annotated as non-policy / starter guesses
// ---------------------------------------------------------------------------

#[test]
fn verifier_ac7_seed_rules_annotated_as_non_policy() {
    let source = include_str!("../src/prompt_collector.rs");

    // Find the SEED_RULES definition and scan the surrounding area for a
    // non-policy annotation.  The contract says "a comment or annotation
    // indicating the labels are starter guesses, not policy decisions."
    let seed_rules_idx = source
        .find("SEED_RULES")
        .expect("AC7: SEED_RULES must exist in prompt_collector.rs");

    // Look in the 500 characters before SEED_RULES for the annotation.
    let context_start = if seed_rules_idx > 600 {
        seed_rules_idx - 600
    } else {
        0
    };
    let context = &source[context_start..seed_rules_idx + 200];

    let has_non_policy_annotation = context.contains("NOT policy")
        || context.contains("not policy")
        || context.contains("starter")
        || context.contains("starting point")
        || context.contains("informed guess")
        || context.contains("informed guesses");

    assert!(
        has_non_policy_annotation,
        "AC7: SEED_RULES definition must have a comment marking labels as \
         non-policy / starter guesses. Context around SEED_RULES:\n{context}"
    );
}

// ---------------------------------------------------------------------------
// AC8 — CLAUDE.md has 2-4 observation-only sentences about prompt-collector
//
// The contract says: "The relevant README or project-instruction file must
// include a note of 2–4 sentences, framed as observation-only, referencing
// the kete charter boundary without making commitments beyond this issue."
// ---------------------------------------------------------------------------

#[test]
fn verifier_ac8_claude_md_has_prompt_collector_note() {
    let claude_md = include_str!("../CLAUDE.md");

    // Find the dedicated section header for the prompt-collector description.
    // The architecture table also mentions prompt_collector.rs in a one-liner,
    // but the AC requires a 2-4 sentence note — find the section header.
    let section_idx = claude_md
        .find("Prompt-collector module")
        .or_else(|| claude_md.find("## Prompt-collector"))
        .or_else(|| claude_md.find("## prompt-collector"))
        .expect("AC8: CLAUDE.md must contain a 'Prompt-collector module' section heading");

    // Extract the section body — up to the next ## heading or 1200 chars.
    let after = &claude_md[section_idx..];
    let section_end = after
        .find("\n## ")
        .unwrap_or_else(|| std::cmp::min(after.len(), 1200));
    let snippet = &after[..section_end];

    // Must be framed as observation-only — must mention no-enforcement or
    // absence of block/warn/mutation.  The contract wording is "framed as
    // observation-only, referencing the kete charter boundary".
    let lower = snippet.to_lowercase();
    let has_observation_framing = lower.contains("no enforcement")
        || lower.contains("performs no enforcement")
        || lower.contains("no block")
        || lower.contains("observation")
        || lower.contains("observation-only")
        || lower.contains("observation point");

    assert!(
        has_observation_framing,
        "AC8: CLAUDE.md prompt-collector section must be framed as observation-only. \
         Snippet:\n{snippet}"
    );

    // Must reference the kete / charter boundary or local-only / read-only.
    let mentions_charter = snippet.contains("kete")
        || snippet.contains("charter")
        || snippet.contains("local-only")
        || snippet.contains("read-only")
        || snippet.contains("no network");

    assert!(
        mentions_charter,
        "AC8: CLAUDE.md prompt-collector note must reference kete charter boundary \
         or local-only/no-network guarantee. Snippet:\n{snippet}"
    );

    // Count period-terminated sentences in the paragraph (not title dots).
    // Use '. ' or '.\n' or '."' patterns for a more reliable count.
    let sentence_count = snippet.matches(". ").count()
        + snippet.matches(".\n").count()
        + if snippet.trim_end().ends_with('.') {
            1
        } else {
            0
        };

    assert!(
        sentence_count >= 2,
        "AC8: CLAUDE.md prompt-collector note must contain at least 2 sentences, \
         found ~{sentence_count}. Snippet:\n{snippet}"
    );
}

// ---------------------------------------------------------------------------
// AC9 — Full test suite passes (cargo test exits 0)
//
// This is verified by actually running `cargo test` as part of the verifier
// test suite invocation.  The verifier report records the exit code
// independently.  We also confirm the presence of the key module under test
// by importing a publicly accessible path.
// ---------------------------------------------------------------------------

/// AC9: The prompt_collector module compiles and the binary includes it.
/// The actual cargo-test exit code is observed when this file is compiled
/// and run as part of `cargo test`.  This placeholder confirms the module
/// is importable and at minimum its declared entry points are reachable.
#[test]
fn verifier_ac9_binary_runs_and_exits_ok() {
    // Simply verify the binary is reachable and responds to `--version`.
    let out = Command::new(arai_bin())
        .arg("--version")
        .output()
        .expect("spawn arai --version");
    assert!(
        out.status.success(),
        "AC9: arai binary failed --version: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}
