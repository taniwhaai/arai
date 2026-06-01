//! Subprocess integration tests for gateway-outcome-glyphs (AC2, AC4, AC6, AC7).
//!
//! Sets up an isolated project dir + ARAI_BASE_DIR, seeds a blocking rule via
//! a CLAUDE.md file, triggers the hook pipeline, and asserts:
//!
//!   - AC4: glyph chars present in human `arai audit` and `arai why` output.
//!   - AC2: `ARAI_ASCII=1` → all glyph-region bytes ≤ 0x7F.
//!   - AC6: `guardrails --match-stdin` output contains the glyph AND zero \x1b bytes.
//!   - AC7: every `--json` output has zero glyph codepoints.
//!
//! No new dependency — uses only `std` and `serde_json` (already in tree).

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ── The Unicode glyph codepoints from the binding table ───────────────────────
// These must not appear in any --json field value.
const GLYPH_CHARS: &[char] = &[
    '\u{25CF}', // ●
    '\u{00B7}', // ·
    '\u{2502}', // │
    '\u{2715}', // ✕
];

// ASCII glyph sequences that must not appear in --json output.
const ASCII_GLYPH_TOKENS: &[&str] = &["o.|x", "|o|", "o.|"];

// ── Test helpers ───────────────────────────────────────────────────────────────

fn fresh_env(label: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!(
        "arai_glyphs_{label}_{}_{}",
        std::process::id(),
        nanos,
    ));
    let project = root.join("project");
    let arai_base = root.join("arai_base");
    std::fs::create_dir_all(project.join(".git")).expect("create project dir");
    std::fs::create_dir_all(&arai_base).expect("create arai base dir");
    (project, arai_base)
}

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

/// Run `arai <args>` in the project dir with ARAI_BASE_DIR set.
/// Extra env vars may be supplied; telemetry is always disabled.
fn run(
    args: &[&str],
    project: &Path,
    arai_base: &Path,
    env_extras: &[(&str, &str)],
) -> (Vec<u8>, Vec<u8>, i32) {
    let mut cmd = Command::new(arai_bin());
    cmd.args(args)
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        // Deny mode on so a Block rule actually produces decision="deny".
        .env("ARAI_DENY_MODE", "on")
        // Clear colour-gate vars so output is predictable.
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env_extras {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn arai");
    (out.stdout, out.stderr, out.status.code().unwrap_or(-1))
}

/// Pipe `payload` to `arai guardrails --match-stdin`.
fn pipe_hook(
    payload: &str,
    project: &Path,
    arai_base: &Path,
    env_extras: &[(&str, &str)],
) -> (Vec<u8>, Vec<u8>, i32) {
    let mut cmd = Command::new(arai_bin());
    cmd.args(["guardrails", "--match-stdin"])
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .env("ARAI_DENY_MODE", "on")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env_extras {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn arai guardrails");
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(payload.as_bytes()).expect("write payload");
    }
    let out = child.wait_with_output().expect("wait arai guardrails");
    (out.stdout, out.stderr, out.status.code().unwrap_or(-1))
}

/// Seed the project with a CLAUDE.md containing a blocking rule, then
/// `arai scan` to load it into the DB.  Returns project_slug (for later use).
fn seed_blocking_rule(project: &Path, arai_base: &Path) {
    let claude_md = project.join("CLAUDE.md");
    // "Never run git push with --force flag" extracts a Block-severity Bash
    // rule that matches when `git push --force` appears in a command.
    std::fs::write(
        &claude_md,
        "# Rules\n\n- Never run git push with --force flag\n",
    )
    .expect("write CLAUDE.md");

    let (stdout, stderr, code) = run(&["scan"], project, arai_base, &[]);
    assert_eq!(
        code,
        0,
        "arai scan failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
}

/// The hook payload that should trigger the blocking rule.
fn blocking_payload(session_id: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "git push --force origin main" },
        "session_id": session_id,
    })
    .to_string()
}

/// True iff the bytes contain any Unicode glyph codepoints from the binding table.
fn contains_unicode_glyph(bytes: &[u8]) -> bool {
    let s = String::from_utf8_lossy(bytes);
    GLYPH_CHARS.iter().any(|&c| s.contains(c))
}

/// True iff the bytes contain any ASCII glyph token.
fn contains_ascii_glyph(bytes: &[u8]) -> bool {
    let s = String::from_utf8_lossy(bytes);
    ASCII_GLYPH_TOKENS.iter().any(|&tok| s.contains(tok))
}

/// True iff the bytes contain any glyph (unicode or ascii).
fn contains_any_glyph(bytes: &[u8]) -> bool {
    contains_unicode_glyph(bytes) || contains_ascii_glyph(bytes)
}

/// True iff any byte in `bytes` equals 0x1B.
fn contains_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1B)
}

/// Walk every string field in a JSON value, returning true iff any contains
/// a Unicode glyph codepoint or an ASCII glyph token.
fn json_has_glyph(v: &Value) -> bool {
    match v {
        Value::String(s) => {
            let s_bytes = s.as_bytes();
            contains_unicode_glyph(s_bytes) || contains_ascii_glyph(s_bytes)
        }
        Value::Object(map) => map.values().any(json_has_glyph),
        Value::Array(arr) => arr.iter().any(json_has_glyph),
        _ => false,
    }
}

// ── AC4: glyph present in human `arai audit` and `arai why` output ────────────

/// Trigger the blocking rule via a hook payload (which writes an audit entry),
/// then assert the human-format `arai audit` output contains a glyph.
#[test]
fn ac4_audit_human_output_contains_glyph() {
    let (project, arai_base) = fresh_env("ac4_audit");
    seed_blocking_rule(&project, &arai_base);

    // Fire the hook so an audit entry is written.
    let session = "ac4-audit-test";
    let payload = blocking_payload(session);
    pipe_hook(&payload, &project, &arai_base, &[]);

    // Run `arai audit` (human, not --json) and assert glyph present.
    let (stdout, stderr, code) = run(&["audit", "--limit=10"], &project, &arai_base, &[]);
    assert_eq!(
        code,
        0,
        "arai audit failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
    assert!(
        contains_any_glyph(&stdout),
        "arai audit human output must contain at least one gateway glyph:\n{}",
        String::from_utf8_lossy(&stdout),
    );
}

/// `arai why` human output must contain the blocked glyph when a blocking rule matches.
#[test]
fn ac4_why_human_output_contains_glyph() {
    let (project, arai_base) = fresh_env("ac4_why");
    seed_blocking_rule(&project, &arai_base);

    let (stdout, stderr, code) = run(
        &["why", "git push --force origin main"],
        &project,
        &arai_base,
        &[],
    );
    assert_eq!(
        code,
        0,
        "arai why failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
    // If no rules matched, the test is invalid; assert we got a match first.
    let out_str = String::from_utf8_lossy(&stdout);
    if out_str.contains("matched: 0 rules") {
        panic!("blocking rule did not match in `arai why` — test setup issue:\n{out_str}");
    }
    assert!(
        contains_any_glyph(&stdout),
        "arai why human output must contain a gateway glyph:\n{out_str}",
    );
}

// ── AC2: ARAI_ASCII=1 → glyph-region uses only ASCII glyph tokens ────────────
//
// We assert: no Unicode glyph codepoints present AND at least one ASCII glyph
// token present.  We do NOT assert the entire output is 7-bit clean because
// pre-existing UI elements (e.g. the `─` table separator) use non-ASCII chars
// that are outside the scope of this contract.

#[test]
fn ac2_arai_ascii_audit_glyph_is_ascii() {
    let (project, arai_base) = fresh_env("ac2_audit");
    seed_blocking_rule(&project, &arai_base);

    let session = "ac2-audit-test";
    let payload = blocking_payload(session);
    pipe_hook(&payload, &project, &arai_base, &[("ARAI_ASCII", "1")]);

    let (stdout, stderr, code) = run(
        &["audit", "--limit=10"],
        &project,
        &arai_base,
        &[("ARAI_ASCII", "1")],
    );
    assert_eq!(
        code,
        0,
        "arai audit (ARAI_ASCII=1) failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
    // No Unicode glyph codepoints from the binding table.
    assert!(
        !contains_unicode_glyph(&stdout),
        "ARAI_ASCII=1: audit output must not contain Unicode glyph codepoints:\n{}",
        String::from_utf8_lossy(&stdout),
    );
    // Must contain an ASCII glyph token (proving the glyph was rendered in ASCII).
    assert!(
        contains_ascii_glyph(&stdout),
        "ARAI_ASCII=1: audit output must contain an ASCII glyph token:\n{}",
        String::from_utf8_lossy(&stdout),
    );
}

#[test]
fn ac2_arai_ascii_why_glyph_is_ascii() {
    let (project, arai_base) = fresh_env("ac2_why");
    seed_blocking_rule(&project, &arai_base);

    let (stdout, stderr, code) = run(
        &["why", "git push --force origin main"],
        &project,
        &arai_base,
        &[("ARAI_ASCII", "1")],
    );
    assert_eq!(
        code,
        0,
        "arai why (ARAI_ASCII=1) failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
    let out_str = String::from_utf8_lossy(&stdout);
    if out_str.contains("matched: 0 rules") {
        panic!("blocking rule did not match in `arai why` (ARAI_ASCII=1) — test setup issue:\n{out_str}");
    }
    // No Unicode glyph codepoints from the binding table.
    assert!(
        !contains_unicode_glyph(&stdout),
        "ARAI_ASCII=1: why output must not contain Unicode glyph codepoints:\n{out_str}",
    );
    // Must contain an ASCII glyph token.
    assert!(
        contains_ascii_glyph(&stdout),
        "ARAI_ASCII=1: why output must contain an ASCII glyph token:\n{out_str}",
    );
}

// ── AC6: hook guardrails --match-stdin output has glyph AND zero ANSI ─────────

#[test]
fn ac6_hook_output_has_glyph_and_zero_ansi() {
    let (project, arai_base) = fresh_env("ac6");
    seed_blocking_rule(&project, &arai_base);

    let session = "ac6-hook-test";
    let payload = blocking_payload(session);
    let (stdout, _stderr, _code) = pipe_hook(&payload, &project, &arai_base, &[]);

    let out_str = String::from_utf8_lossy(&stdout);

    // Must contain a glyph somewhere in the JSON string fields.
    assert!(
        contains_any_glyph(&stdout),
        "hook output must contain a gateway glyph:\n{out_str}",
    );

    // Must contain zero ANSI escape bytes — hook path is colorize=false always.
    assert!(
        !contains_ansi(&stdout),
        "hook output must contain ZERO ANSI escape bytes (0x1B):\n{out_str}",
    );
}

/// Same test with ARAI_ASCII=1: glyph bytes must all be ≤ 0x7F, still no ANSI.
#[test]
fn ac6_hook_output_arai_ascii_has_ascii_glyph_and_zero_ansi() {
    let (project, arai_base) = fresh_env("ac6_ascii");
    seed_blocking_rule(&project, &arai_base);

    let session = "ac6-ascii-hook-test";
    let payload = blocking_payload(session);
    let (stdout, _stderr, _code) =
        pipe_hook(&payload, &project, &arai_base, &[("ARAI_ASCII", "1")]);

    let out_str = String::from_utf8_lossy(&stdout);

    // No ANSI regardless.
    assert!(
        !contains_ansi(&stdout),
        "hook output (ARAI_ASCII=1) must contain ZERO ANSI escape bytes:\n{out_str}",
    );

    // Every byte must be ≤ 0x7F (ASCII glyph form).
    let high_byte = stdout.iter().find(|&&b| b > 0x7F);
    assert!(
        high_byte.is_none(),
        "ARAI_ASCII=1: hook output has byte > 0x7F ({:#04x}) — \
         glyph must be ASCII-only:\n{out_str}",
        high_byte.unwrap_or(&0),
    );
}

// ── AC7: every --json output is glyph-free ────────────────────────────────────

#[test]
fn ac7_audit_json_has_no_glyph() {
    let (project, arai_base) = fresh_env("ac7_audit");
    seed_blocking_rule(&project, &arai_base);

    // Fire the hook so there's at least one audit entry.
    let payload = blocking_payload("ac7-session");
    pipe_hook(&payload, &project, &arai_base, &[]);

    let (stdout, stderr, code) = run(
        &["audit", "--json", "--limit=10"],
        &project,
        &arai_base,
        &[],
    );
    assert_eq!(
        code,
        0,
        "arai audit --json failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );

    for line in String::from_utf8_lossy(&stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("audit --json produced non-JSON line: {e}\n{line}"));
        assert!(
            !json_has_glyph(&v),
            "arai audit --json must not contain glyph chars in any field:\n{line}"
        );
    }
}

#[test]
fn ac7_why_json_has_no_glyph() {
    let (project, arai_base) = fresh_env("ac7_why");
    seed_blocking_rule(&project, &arai_base);

    let (stdout, stderr, code) = run(
        &["why", "--json", "git push --force origin main"],
        &project,
        &arai_base,
        &[],
    );
    assert_eq!(
        code,
        0,
        "arai why --json failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );

    let out_str = String::from_utf8_lossy(&stdout);
    if !out_str.trim().is_empty() {
        let v: Value = serde_json::from_str(out_str.trim())
            .unwrap_or_else(|e| panic!("why --json non-JSON output: {e}\n{out_str}"));
        assert!(
            !json_has_glyph(&v),
            "arai why --json must not contain glyph chars in any field:\n{out_str}"
        );
    }
}

#[test]
fn ac7_stats_json_has_no_glyph() {
    let (project, arai_base) = fresh_env("ac7_stats");
    seed_blocking_rule(&project, &arai_base);

    let (stdout, stderr, code) = run(&["stats", "--json"], &project, &arai_base, &[]);
    assert_eq!(
        code,
        0,
        "arai stats --json failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );

    let out_str = String::from_utf8_lossy(&stdout);
    if !out_str.trim().is_empty() {
        let v: Value = serde_json::from_str(out_str.trim())
            .unwrap_or_else(|e| panic!("stats --json non-JSON output: {e}\n{out_str}"));
        assert!(
            !json_has_glyph(&v),
            "arai stats --json must not contain glyph chars in any field:\n{out_str}"
        );
    }
}
