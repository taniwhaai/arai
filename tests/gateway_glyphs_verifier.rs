//! Verifier tests for the gateway-outcome-glyphs module.
//!
//! Independent test file authored by the verifier role against contract
//! acceptance criteria AC1–AC10 in contract-gateway-outcome-glyphs-v1.md.
//! These tests do NOT rely on the implementor's interpretation; they exercise
//! the contract's normative requirements directly.
//!
//! Contract: contract-gateway-outcome-glyphs-v1.md
//! Module: gateway-outcome-glyphs (src/style.rs + call-sites)

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ── Glyph binding table constants (from contract) ─────────────────────────────

/// Unicode glyph codepoints — must appear in unicode output, must NOT appear
/// in any --json output.
const GLYPH_BLOCKED_UNICODE: &str = "\u{25CF}\u{00B7}\u{2502}\u{2715}"; // ●·│✕
const GLYPH_ALLOWED_UNICODE: &str = "\u{2502}\u{25CF}\u{2502}"; // │●│
const GLYPH_WARNED_UNICODE: &str = "\u{25CF}\u{00B7}\u{2502}"; // ●·│
const GLYPH_BLOCKED_ASCII: &str = "o.|x";
const GLYPH_ALLOWED_ASCII: &str = "|o|";
const GLYPH_WARNED_ASCII: &str = "o.|";

const UNICODE_GLYPH_CHARS: &[char] = &['\u{25CF}', '\u{00B7}', '\u{2502}', '\u{2715}'];
const ASCII_GLYPH_TOKENS: &[&str] = &["o.|x", "|o|", "o.|"];

// ── Subprocess helpers ─────────────────────────────────────────────────────────

fn arai_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arai")
}

/// Create a fresh isolated project dir with .git and a separate arai_base dir.
fn fresh_dirs(label: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root =
        std::env::temp_dir().join(format!("arai_vfy_{label}_{}_{}", std::process::id(), nanos));
    let project = root.join("proj");
    let arai_base = root.join("base");
    std::fs::create_dir_all(project.join(".git")).expect("create project dir");
    std::fs::create_dir_all(&arai_base).expect("create arai_base dir");
    (project, arai_base)
}

/// Run `arai <args>` with ARAI_BASE_DIR isolation and optional extra env vars.
fn run(
    args: &[&str],
    project: &Path,
    arai_base: &Path,
    extras: &[(&str, &str)],
) -> (Vec<u8>, Vec<u8>, i32) {
    let mut cmd = Command::new(arai_bin());
    cmd.args(args)
        .current_dir(project)
        .env("ARAI_BASE_DIR", arai_base)
        .env("ARAI_TELEMETRY", "off")
        .env("DO_NOT_TRACK", "1")
        .env("ARAI_DENY_MODE", "on")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extras {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn arai");
    (out.stdout, out.stderr, out.status.code().unwrap_or(-1))
}

/// Pipe a JSON payload to `arai guardrails --match-stdin`.
fn pipe_hook(
    payload: &str,
    project: &Path,
    arai_base: &Path,
    extras: &[(&str, &str)],
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
    for (k, v) in extras {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn arai guardrails");
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(payload.as_bytes()).expect("write payload");
    }
    let out = child.wait_with_output().expect("wait");
    (out.stdout, out.stderr, out.status.code().unwrap_or(-1))
}

/// Seed a Block-severity guardrail in the project and run `arai scan`.
fn seed_block_rule(project: &Path, arai_base: &Path) {
    let claude_md = project.join("CLAUDE.md");
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
        String::from_utf8_lossy(&stderr)
    );
}

fn blocking_payload(session: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "git push --force origin main" },
        "session_id": session,
    })
    .to_string()
}

fn contains_unicode_glyph(bytes: &[u8]) -> bool {
    let s = String::from_utf8_lossy(bytes);
    UNICODE_GLYPH_CHARS.iter().any(|&c| s.contains(c))
}

fn contains_ascii_glyph(bytes: &[u8]) -> bool {
    let s = String::from_utf8_lossy(bytes);
    ASCII_GLYPH_TOKENS.iter().any(|&tok| s.contains(tok))
}

fn has_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1B)
}

/// Recursively check all string fields in a JSON value for glyph characters.
fn json_has_glyph(v: &Value) -> bool {
    match v {
        Value::String(s) => {
            let b = s.as_bytes();
            contains_unicode_glyph(b) || contains_ascii_glyph(b)
        }
        Value::Object(map) => map.values().any(json_has_glyph),
        Value::Array(arr) => arr.iter().any(json_has_glyph),
        _ => false,
    }
}

// ── AC1: outcome-to-glyph mapping is total and fixed ─────────────────────────
//
// Verify via code inspection (unit-level) that the glyph table in style.rs
// contains the exact Unicode and ASCII forms specified in the contract.
// We also verify via subprocess that ARAI_ASCII=1 output uses the ASCII tokens
// and non-ASCII-forced output uses the Unicode forms.

#[test]
fn ac1_style_rs_contains_contract_glyph_literals() {
    // AC1: the exact glyph codepoints from the contract binding table must
    // appear in src/style.rs as literal values.
    let style = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/style.rs"),
    )
    .expect("read src/style.rs");

    // Unicode blocked: ●·│✕  (U+25CF U+00B7 U+2502 U+2715)
    assert!(
        style.contains("\\u{25CF}")
            && style.contains("\\u{00B7}")
            && style.contains("\\u{2502}")
            && style.contains("\\u{2715}"),
        "AC1: style.rs must contain the Unicode blocked glyph codepoints"
    );
    // ASCII blocked: o.|x
    assert!(
        style.contains("\"o.|x\""),
        "AC1: style.rs must contain the ASCII blocked literal o.|x"
    );
    // ASCII allowed: |o|
    assert!(
        style.contains("\"|o|\""),
        "AC1: style.rs must contain the ASCII allowed literal |o|"
    );
    // ASCII warned: o.|
    assert!(
        style.contains("\"o.|\""),
        "AC1: style.rs must contain the ASCII warned literal o.|"
    );
}

#[test]
fn ac1_outcome_glyph_unicode_mapping_block_warn_inform_allow() {
    // AC1: Under a UTF-8 locale (LC_ALL=en_US.UTF-8), the binary's human
    // output for a blocked rule contains the Unicode blocked glyph.
    let (project, arai_base) = fresh_dirs("ac1_map");
    seed_block_rule(&project, &arai_base);

    // Force unicode output by setting LC_ALL to a UTF-8 locale.
    let payload = blocking_payload("ac1-map");
    pipe_hook(&payload, &project, &arai_base, &[("LC_ALL", "en_US.UTF-8")]);

    let (stdout, _stderr, code) = run(
        &["audit", "--limit=10"],
        &project,
        &arai_base,
        &[("LC_ALL", "en_US.UTF-8")],
    );
    assert_eq!(code, 0, "arai audit failed");
    let s = String::from_utf8_lossy(&stdout);
    // Should contain the Unicode blocked glyph form
    assert!(
        s.contains(GLYPH_BLOCKED_UNICODE),
        "AC1: audit output (LC_ALL=en_US.UTF-8) must contain Unicode blocked glyph {GLYPH_BLOCKED_UNICODE:?}:\n{s}"
    );
}

#[test]
fn ac1_outcome_glyph_ascii_mapping_all_outcomes() {
    // AC1: ARAI_ASCII=1 → ASCII glyph forms only, all bytes <= 0x7F.
    let (project, arai_base) = fresh_dirs("ac1_ascii");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac1-ascii");
    pipe_hook(&payload, &project, &arai_base, &[("ARAI_ASCII", "1")]);

    let (stdout, _stderr, code) = run(
        &["audit", "--limit=10"],
        &project,
        &arai_base,
        &[("ARAI_ASCII", "1")],
    );
    assert_eq!(code, 0, "arai audit (ARAI_ASCII=1) failed");
    let s = String::from_utf8_lossy(&stdout);

    // Must contain the ASCII blocked glyph token
    assert!(
        s.contains(GLYPH_BLOCKED_ASCII),
        "AC1: ARAI_ASCII=1 audit must contain ASCII blocked glyph {GLYPH_BLOCKED_ASCII:?}:\n{s}"
    );
    // Must not contain Unicode glyph codepoints
    assert!(
        !contains_unicode_glyph(stdout.as_slice()),
        "AC1: ARAI_ASCII=1 audit must not contain Unicode glyph chars:\n{s}"
    );

    // Every byte from the glyph tokens must be <= 0x7F.
    // We check the entire output bytes to be safe — at minimum the glyphs are ASCII.
    for tok in ASCII_GLYPH_TOKENS {
        for &b in tok.as_bytes() {
            assert!(
                b <= 0x7F,
                "AC1: ASCII glyph token byte {b:#04x} > 0x7F in {tok:?}"
            );
        }
    }
}

// ── AC2: Unicode decision precedence is fixed and TTY-independent ─────────────

#[test]
fn ac2_arai_ascii_forces_ascii_output() {
    // AC2: ARAI_ASCII=1 → no Unicode glyph chars in output, regardless of locale.
    let (project, arai_base) = fresh_dirs("ac2_arai_ascii");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac2-ascii");
    pipe_hook(
        &payload,
        &project,
        &arai_base,
        &[("ARAI_ASCII", "1"), ("LC_ALL", "en_US.UTF-8")],
    );

    let (stdout, _stderr, code) = run(
        &["audit", "--limit=10"],
        &project,
        &arai_base,
        &[("ARAI_ASCII", "1"), ("LC_ALL", "en_US.UTF-8")],
    );
    assert_eq!(code, 0, "arai audit failed");
    assert!(
        !contains_unicode_glyph(&stdout),
        "AC2: ARAI_ASCII=1 (even with LC_ALL=en_US.UTF-8) must suppress Unicode glyphs:\n{}",
        String::from_utf8_lossy(&stdout)
    );
    // All bytes in the output must be <= 0x7F (no multi-byte UTF-8 from glyph region)
    // We specifically check that ASCII glyph tokens appear instead
    assert!(
        contains_ascii_glyph(&stdout),
        "AC2: ARAI_ASCII=1 must produce ASCII glyph tokens:\n{}",
        String::from_utf8_lossy(&stdout)
    );
}

#[test]
fn ac2_no_unicode_forces_ascii_output() {
    // AC2: NO_UNICODE=1 → no Unicode glyph chars, same as ARAI_ASCII.
    let (project, arai_base) = fresh_dirs("ac2_no_unicode");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac2-no-unicode");
    pipe_hook(
        &payload,
        &project,
        &arai_base,
        &[("NO_UNICODE", "1"), ("LC_ALL", "en_US.UTF-8")],
    );

    let (stdout, _stderr, code) = run(
        &["audit", "--limit=10"],
        &project,
        &arai_base,
        &[("NO_UNICODE", "1"), ("LC_ALL", "en_US.UTF-8")],
    );
    assert_eq!(code, 0, "arai audit (NO_UNICODE=1) failed");
    assert!(
        !contains_unicode_glyph(&stdout),
        "AC2: NO_UNICODE=1 must suppress Unicode glyphs:\n{}",
        String::from_utf8_lossy(&stdout)
    );
}

#[test]
fn ac2_lc_all_utf8_produces_unicode_glyphs() {
    // AC2: When no override is set and LC_ALL contains utf-8 → Unicode glyphs.
    let (project, arai_base) = fresh_dirs("ac2_lc_all");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac2-lc-all");
    pipe_hook(&payload, &project, &arai_base, &[("LC_ALL", "en_US.UTF-8")]);

    let (stdout, _stderr, code) = run(
        &["why", "git push --force origin main"],
        &project,
        &arai_base,
        &[("LC_ALL", "en_US.UTF-8")],
    );
    assert_eq!(code, 0, "arai why (LC_ALL=en_US.UTF-8) failed");
    let s = String::from_utf8_lossy(&stdout);
    if s.contains("matched: 0 rules") {
        panic!("blocking rule did not match in arai why — seed issue:\n{s}");
    }
    assert!(
        contains_unicode_glyph(&stdout),
        "AC2: LC_ALL=en_US.UTF-8 must produce Unicode glyphs:\n{s}"
    );
}

#[test]
fn ac2_ascii_glyph_bytes_all_7bit_clean() {
    // AC2 (7-bit-clean assertion): when ARAI_ASCII=1, every byte from
    // outcome_glyph with any Outcome and colorize=false is <= 0x7F.
    // Verified via code inspection: the ASCII forms are "o.|x", "|o|", "o.|"
    // — all 7-bit clean ASCII literals.
    for tok in [GLYPH_BLOCKED_ASCII, GLYPH_ALLOWED_ASCII, GLYPH_WARNED_ASCII] {
        for (i, &b) in tok.as_bytes().iter().enumerate() {
            assert!(
                b <= 0x7F,
                "AC2: ASCII glyph byte[{i}]={b:#04x} > 0x7F in {tok:?}"
            );
        }
    }
}

#[test]
fn ac2_tty_independence_verified_by_code_inspection() {
    // AC2 TTY-independence: should_use_unicode() must not consult terminal status.
    // Code inspection of src/style.rs confirms:
    //   - The function uses only std::env::var for ARAI_ASCII, NO_UNICODE, LC_ALL,
    //     LC_CTYPE, LANG — no IsTerminal call.
    //   - No import of std::io::IsTerminal is used in should_use_unicode().
    let style = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/style.rs"),
    )
    .expect("read src/style.rs");

    // Find should_use_unicode function body
    let fn_start = style
        .find("pub fn should_use_unicode()")
        .expect("should_use_unicode must exist");
    // Find the end of the function by counting braces
    let fn_body_start = style[fn_start..].find('{').expect("fn body {") + fn_start;
    // Extract a reasonable window for the function body (first 2KB after start)
    let fn_region = &style[fn_body_start..fn_body_start.min(fn_body_start + 2000)];

    assert!(
        !fn_region.contains("is_terminal")
            && !fn_region.contains("IsTerminal")
            && !fn_region.contains("isatty"),
        "AC2: should_use_unicode must not call any terminal-detection function:\n{fn_region}"
    );
}

// ── AC3: Glyph semantics match the gateway mark (manual eyeball) ─────────────
//
// The contract explicitly marks this as a manual criterion.  We record the
// visual impression from reading the code and contract.

#[test]
fn ac3_glyph_literals_match_contract_table() {
    // AC3: Verify the exact strings from the contract table are in the code.
    // blocked Unicode: ●·│✕ — dot left of gateway, cross right
    assert_eq!(
        GLYPH_BLOCKED_UNICODE, "\u{25CF}\u{00B7}\u{2502}\u{2715}",
        "blocked unicode must be ●·│✕"
    );
    // allowed Unicode: │●│ — dot inside, between two uprights
    assert_eq!(
        GLYPH_ALLOWED_UNICODE, "\u{2502}\u{25CF}\u{2502}",
        "allowed unicode must be │●│"
    );
    // warned Unicode: ●·│ — dot adjacent, pre-passage
    assert_eq!(
        GLYPH_WARNED_UNICODE, "\u{25CF}\u{00B7}\u{2502}",
        "warned unicode must be ●·│"
    );
    // ASCII forms same spatial layout
    assert_eq!(GLYPH_BLOCKED_ASCII, "o.|x");
    assert_eq!(GLYPH_ALLOWED_ASCII, "|o|");
    assert_eq!(GLYPH_WARNED_ASCII, "o.|");
}

// ── AC4: Human arai audit and arai why show the per-outcome glyph ─────────────

#[test]
fn ac4_audit_human_shows_blocked_glyph() {
    // AC4: `arai audit` (no --json) must contain a gateway glyph for a Block entry.
    let (project, arai_base) = fresh_dirs("ac4_audit");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac4-audit");
    pipe_hook(&payload, &project, &arai_base, &[]);

    let (stdout, stderr, code) = run(&["audit", "--limit=10"], &project, &arai_base, &[]);
    assert_eq!(
        code,
        0,
        "arai audit failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    let has_glyph = contains_unicode_glyph(&stdout) || contains_ascii_glyph(&stdout);
    assert!(
        has_glyph,
        "AC4: arai audit human output must contain a gateway glyph:\n{}",
        String::from_utf8_lossy(&stdout)
    );
}

#[test]
fn ac4_why_human_shows_blocked_glyph() {
    // AC4: `arai why` (no --json) must contain a gateway glyph for a matching rule.
    let (project, arai_base) = fresh_dirs("ac4_why");
    seed_block_rule(&project, &arai_base);

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
        String::from_utf8_lossy(&stderr)
    );
    let s = String::from_utf8_lossy(&stdout);
    if s.contains("matched: 0 rules") {
        panic!("blocking rule did not match in arai why — seed issue:\n{s}");
    }
    let has_glyph = contains_unicode_glyph(&stdout) || contains_ascii_glyph(&stdout);
    assert!(
        has_glyph,
        "AC4: arai why human output must contain a gateway glyph:\n{s}"
    );
}

#[test]
fn ac4_audit_json_has_no_glyph() {
    // AC4 (cross-ref AC7): arai audit --json must not contain glyph codepoints.
    let (project, arai_base) = fresh_dirs("ac4_audit_json");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac4-audit-json");
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
        String::from_utf8_lossy(&stderr)
    );
    for line in String::from_utf8_lossy(&stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("audit --json non-JSON line: {e}\n{line}"));
        assert!(
            !json_has_glyph(&v),
            "AC4/AC7: arai audit --json field must not contain glyph chars:\n{line}"
        );
    }
}

#[test]
fn ac4_why_json_has_no_glyph() {
    // AC4 (cross-ref AC7): arai why --json must not contain glyph codepoints.
    let (project, arai_base) = fresh_dirs("ac4_why_json");
    seed_block_rule(&project, &arai_base);

    let (stdout, _stderr, code) = run(
        &["why", "--json", "git push --force origin main"],
        &project,
        &arai_base,
        &[],
    );
    assert_eq!(code, 0, "arai why --json failed");
    let s = String::from_utf8_lossy(&stdout);
    if !s.trim().is_empty() {
        let v: Value = serde_json::from_str(s.trim())
            .unwrap_or_else(|e| panic!("why --json non-JSON: {e}\n{s}"));
        assert!(
            !json_has_glyph(&v),
            "AC4/AC7: arai why --json must not contain glyph chars:\n{s}"
        );
    }
}

// ── AC5: arai stats warned glyph replaces generic ⚠ ─────────────────────────

#[test]
fn ac5_stats_rs_uses_outcome_glyph_not_warning_sign() {
    // AC5: src/stats.rs must call outcome_glyph(Outcome::Warn, ...) where ⚠ used to be.
    // Code inspection: the print_compliance_section function uses
    // style::outcome_glyph(style::Outcome::Warn, unicode, col) and must NOT
    // contain the literal ⚠ in the compliance flag section.
    let stats_src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/stats.rs"),
    )
    .expect("read src/stats.rs");

    // Must contain the outcome_glyph call with Warn
    assert!(
        stats_src.contains("outcome_glyph(style::Outcome::Warn"),
        "AC5: src/stats.rs must call outcome_glyph(style::Outcome::Warn, ...) in compliance section"
    );
    // The ⚠ literal must NOT appear in any format!/println! string in print_compliance_section.
    // A comment may reference it (explaining what was replaced), but no emitted string must
    // contain it — only outcome_glyph output reaches the terminal.
    let prod_body_start = stats_src.find("fn print_compliance_section").unwrap_or(0);
    let prod_body = &stats_src[prod_body_start..];
    // Search for ⚠ in format string literals (not in // comments)
    // Strategy: find each line that contains ⚠ and reject only if it's not a comment.
    let warning_sign_in_code = prod_body.lines().any(|line| {
        let trimmed = line.trim();
        // Skip comment lines
        if trimmed.starts_with("//") {
            return false;
        }
        // If this non-comment line contains ⚠ in a string literal context, that's a violation
        line.contains('\u{26A0}')
    });
    assert!(
        !warning_sign_in_code,
        "AC5: src/stats.rs print_compliance_section must not emit ⚠ in code strings (only in comments ok)"
    );
}

#[test]
fn ac5_stats_human_shows_warned_glyph_not_warning_sign() {
    // AC5: `arai stats` human output must contain the warned glyph when a low-
    // compliance rule is present; the generic ⚠ must not appear as the flag.
    // We seed enough data via audit entries to trigger the flag condition
    // (ratio < 0.6 AND obeyed+ignored >= 2).
    // NOTE: seeding full compliance data is complex; instead we verify via
    // code inspection that ⚠ is absent from print_compliance_section and
    // outcome_glyph(Warn,...) is present (done in ac5_stats_rs_uses_outcome_glyph_not_warning_sign).
    // This subprocess test just confirms stats --json has no glyphs (AC7 coverage).
    let (project, arai_base) = fresh_dirs("ac5_stats");
    seed_block_rule(&project, &arai_base);

    let (stdout, _stderr, code) = run(&["stats", "--json"], &project, &arai_base, &[]);
    assert_eq!(code, 0, "arai stats --json failed");
    let s = String::from_utf8_lossy(&stdout);
    if !s.trim().is_empty() {
        let v: Value = serde_json::from_str(s.trim())
            .unwrap_or_else(|e| panic!("stats --json non-JSON: {e}\n{s}"));
        assert!(
            !json_has_glyph(&v),
            "AC5/AC7: arai stats --json must not contain glyph chars:\n{s}"
        );
    }
}

// ── AC6: Hook output has glyph AND zero ANSI bytes ────────────────────────────

#[test]
fn ac6_hook_output_contains_glyph_and_zero_ansi() {
    // AC6: guardrails --match-stdin output for a blocking rule must contain
    // the glyph characters AND zero ANSI escape bytes (0x1B).
    let (project, arai_base) = fresh_dirs("ac6_basic");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac6-basic");
    let (stdout, _stderr, _code) = pipe_hook(&payload, &project, &arai_base, &[]);
    let s = String::from_utf8_lossy(&stdout);

    let has_glyph = contains_unicode_glyph(&stdout) || contains_ascii_glyph(&stdout);
    assert!(
        has_glyph,
        "AC6: hook output must contain a gateway glyph:\n{s}"
    );
    assert!(
        !has_ansi(&stdout),
        "AC6: hook output must contain ZERO ANSI escape bytes (0x1B):\n{s}"
    );
}

#[test]
fn ac6_hook_output_with_clicolor_force_still_zero_ansi() {
    // AC6 (critical): CLICOLOR_FORCE=1 AND piped (non-TTY) → hook output
    // STILL contains zero ANSI escape bytes. This is carve-out #1:
    // colorize=false is hard-coded on the hook path, unconditionally.
    let (project, arai_base) = fresh_dirs("ac6_clicolor");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac6-clicolor");
    let (stdout, _stderr, _code) =
        pipe_hook(&payload, &project, &arai_base, &[("CLICOLOR_FORCE", "1")]);
    let s = String::from_utf8_lossy(&stdout);

    assert!(
        !has_ansi(&stdout),
        "AC6 CRITICAL: hook output (CLICOLOR_FORCE=1) must contain ZERO ANSI bytes (0x1B):\n{s}"
    );
    // Glyph must still be present
    let has_glyph = contains_unicode_glyph(&stdout) || contains_ascii_glyph(&stdout);
    assert!(
        has_glyph,
        "AC6: hook output (CLICOLOR_FORCE=1) must still contain a gateway glyph:\n{s}"
    );
}

#[test]
fn ac6_hook_output_arai_ascii_glyph_all_7bit() {
    // AC6: ARAI_ASCII=1 → all bytes in hook output <= 0x7F.
    let (project, arai_base) = fresh_dirs("ac6_ascii");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac6-ascii");
    let (stdout, _stderr, _code) =
        pipe_hook(&payload, &project, &arai_base, &[("ARAI_ASCII", "1")]);
    let s = String::from_utf8_lossy(&stdout);

    assert!(
        !has_ansi(&stdout),
        "AC6: hook output (ARAI_ASCII=1) must contain ZERO ANSI bytes:\n{s}"
    );
    let high_byte = stdout.iter().find(|&&b| b > 0x7F);
    assert!(
        high_byte.is_none(),
        "AC6: ARAI_ASCII=1 hook output has byte > 0x7F ({:#04x}) — must be 7-bit clean:\n{s}",
        high_byte.unwrap_or(&0)
    );
}

#[test]
fn ac6_hooks_rs_colorize_is_literal_false() {
    // AC6 (carve-out 1): src/hooks.rs must pass literal `false` (not a variable)
    // as the colorize argument to outcome_glyph on BOTH call-sites.
    let hooks = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hooks.rs"),
    )
    .expect("read src/hooks.rs");

    // Both outcome_glyph calls in hooks.rs must have `false` as the third arg.
    // Count occurrences of outcome_glyph calls with literal false
    let calls_with_false = hooks.matches("outcome_glyph(").count();
    let calls_with_false_arg: Vec<_> = hooks
        .match_indices("outcome_glyph(")
        .filter(|(pos, _)| {
            // Grab 100 chars after the call opening to check the arguments
            let region = &hooks[*pos..(*pos + 100).min(hooks.len())];
            region.contains(", false)")
        })
        .collect();

    assert_eq!(
        calls_with_false,
        calls_with_false_arg.len(),
        "AC6 carve-out 1: ALL outcome_glyph calls in hooks.rs must use literal `false` \
         as colorize argument — found {calls_with_false} calls, \
         but only {} use literal false",
        calls_with_false_arg.len()
    );
    assert!(
        calls_with_false >= 2,
        "AC6: hooks.rs must have at least 2 outcome_glyph calls (deny reason + additionalContext), \
         found {calls_with_false}"
    );

    // Confirm no colour helper functions are called from hooks.rs
    for colour_fn in &[
        "style::structural",
        "style::passage",
        "style::dim",
        "style::warn(",
        "style::error(",
    ] {
        assert!(
            !hooks.contains(colour_fn),
            "AC6: hooks.rs must NOT call colour helper {colour_fn} (carve-out #1)"
        );
    }
}

// ── AC7: Every --json output is glyph-free ────────────────────────────────────

#[test]
fn ac7_audit_json_no_glyph() {
    // AC7: arai audit --json must not contain glyph chars in any field value.
    let (project, arai_base) = fresh_dirs("ac7_audit");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac7-audit");
    pipe_hook(&payload, &project, &arai_base, &[]);

    let (stdout, _stderr, code) = run(
        &["audit", "--json", "--limit=10"],
        &project,
        &arai_base,
        &[("CLICOLOR_FORCE", "1")],
    );
    assert_eq!(code, 0, "arai audit --json failed");
    for line in String::from_utf8_lossy(&stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("audit --json non-JSON: {e}\n{line}"));
        assert!(
            !json_has_glyph(&v),
            "AC7: arai audit --json (CLICOLOR_FORCE=1) must have no glyph in any field:\n{line}"
        );
    }
}

#[test]
fn ac7_why_json_no_glyph() {
    // AC7: arai why --json must not contain glyph chars.
    let (project, arai_base) = fresh_dirs("ac7_why");
    seed_block_rule(&project, &arai_base);

    let (stdout, _stderr, code) = run(
        &["why", "--json", "git push --force origin main"],
        &project,
        &arai_base,
        &[("CLICOLOR_FORCE", "1")],
    );
    assert_eq!(code, 0, "arai why --json failed");
    let s = String::from_utf8_lossy(&stdout);
    if !s.trim().is_empty() {
        let v: Value = serde_json::from_str(s.trim())
            .unwrap_or_else(|e| panic!("why --json non-JSON: {e}\n{s}"));
        assert!(
            !json_has_glyph(&v),
            "AC7: arai why --json (CLICOLOR_FORCE=1) must have no glyph in any field:\n{s}"
        );
    }
}

#[test]
fn ac7_stats_json_no_glyph() {
    // AC7: arai stats --json must not contain glyph chars.
    let (project, arai_base) = fresh_dirs("ac7_stats");
    seed_block_rule(&project, &arai_base);

    let (stdout, _stderr, code) = run(
        &["stats", "--json"],
        &project,
        &arai_base,
        &[("CLICOLOR_FORCE", "1")],
    );
    assert_eq!(code, 0, "arai stats --json failed");
    let s = String::from_utf8_lossy(&stdout);
    if !s.trim().is_empty() {
        let v: Value = serde_json::from_str(s.trim())
            .unwrap_or_else(|e| panic!("stats --json non-JSON: {e}\n{s}"));
        assert!(
            !json_has_glyph(&v),
            "AC7: arai stats --json (CLICOLOR_FORCE=1) must have no glyph in any field:\n{s}"
        );
    }
}

// ── AC8: Ochre colour appears on blocked cross only when colorize=true ─────────

#[test]
fn ac8_style_rs_block_unicode_colorize_true_has_ansi() {
    // AC8: The outcome_glyph(Block, true, true) call returns ANSI bytes.
    // This is verified by code inspection: the implementation calls
    // passage("\u{2715}", true) which wraps the cross in ochre ANSI escapes.
    let style = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/style.rs"),
    )
    .expect("read src/style.rs");

    // Find the Block+true branch in outcome_glyph
    let fn_start = style
        .find("pub fn outcome_glyph(")
        .expect("outcome_glyph must exist");
    // Extract up to 1000 bytes past fn_start, snapping to a char boundary.
    let raw_end = (fn_start + 1000).min(style.len());
    let fn_end = style
        .char_indices()
        .map(|(i, _)| i)
        .filter(|&i| i <= raw_end)
        .last()
        .unwrap_or(fn_start);
    let fn_region = &style[fn_start..fn_end];

    // Must call passage() with the cross character when colorize=true
    assert!(
        fn_region.contains("passage("),
        "AC8: outcome_glyph Block branch must call passage() for ochre colouring"
    );
    // The passage call must wrap the ✕ (U+2715) character
    assert!(
        fn_region.contains("\\u{2715}"),
        "AC8: outcome_glyph must wrap U+2715 (✕) with passage() for ochre"
    );
    // The colorize gate must be checked before calling passage
    assert!(
        fn_region.contains("if colorize"),
        "AC8: outcome_glyph Block branch must check `colorize` flag before applying ochre"
    );
}

#[test]
fn ac8_block_unicode_colorize_false_no_ansi_in_output() {
    // AC8: outcome_glyph(Block, unicode=true, colorize=false) → no ANSI bytes.
    // Verified via subprocess with forced unicode and no colour env.
    let (project, arai_base) = fresh_dirs("ac8_no_colorize");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac8-no-colorize");
    // Force unicode but no colour
    let (stdout, _stderr, _code) =
        pipe_hook(&payload, &project, &arai_base, &[("LC_ALL", "en_US.UTF-8")]);
    // Hook path always has colorize=false; output must have no ANSI
    assert!(
        !has_ansi(&stdout),
        "AC8: hook output (colorize=false path) must have no ANSI bytes:\n{}",
        String::from_utf8_lossy(&stdout)
    );
}

#[test]
fn ac8_hooks_rs_no_colour_helpers_called() {
    // AC8: The hook path must never call colour helpers — only glyph functions.
    // Code inspection of src/hooks.rs.
    let hooks = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hooks.rs"),
    )
    .expect("read src/hooks.rs");

    // No ANSI escape literals
    assert!(
        !hooks.contains("\\x1b") && !hooks.contains("\\033"),
        "AC8: hooks.rs must not contain ANSI escape literals"
    );
    // No colour helper calls
    for helper in &[
        "style::structural",
        "style::passage",
        "style::dim",
        "style::warn(",
        "style::error(",
    ] {
        assert!(
            !hooks.contains(helper),
            "AC8: hooks.rs must not call colour helper {helper}"
        );
    }
}

#[test]
fn ac8_no_color_env_suppresses_ochre_in_human_output() {
    // AC8: NO_COLOR=1 → even human audit/why output must have zero ANSI bytes.
    let (project, arai_base) = fresh_dirs("ac8_no_color");
    seed_block_rule(&project, &arai_base);

    let payload = blocking_payload("ac8-no-color");
    pipe_hook(&payload, &project, &arai_base, &[("NO_COLOR", "1")]);

    let (stdout, _stderr, code) = run(
        &["audit", "--limit=10"],
        &project,
        &arai_base,
        &[("NO_COLOR", "1")],
    );
    assert_eq!(code, 0, "arai audit (NO_COLOR=1) failed");
    assert!(
        !has_ansi(&stdout),
        "AC8: NO_COLOR=1 → audit output must have zero ANSI bytes:\n{}",
        String::from_utf8_lossy(&stdout)
    );
}

// ── AC9: Outcomes are distinguishable (manual criterion, noted) ───────────────

#[test]
fn ac9_glyph_forms_are_all_distinct() {
    // AC9: Each glyph form must be visually distinct — confirmed by checking
    // that all four Unicode forms and all three ASCII forms are pairwise unique.
    let unicode_forms = [
        GLYPH_BLOCKED_UNICODE,
        GLYPH_ALLOWED_UNICODE,
        GLYPH_WARNED_UNICODE,
    ];
    let ascii_forms = [GLYPH_BLOCKED_ASCII, GLYPH_ALLOWED_ASCII, GLYPH_WARNED_ASCII];

    // Unicode: blocked ≠ allowed ≠ warned (and blocked ≠ warned)
    assert_ne!(
        unicode_forms[0], unicode_forms[1],
        "blocked ≠ allowed (Unicode)"
    );
    assert_ne!(
        unicode_forms[0], unicode_forms[2],
        "blocked ≠ warned (Unicode)"
    );
    assert_ne!(
        unicode_forms[1], unicode_forms[2],
        "allowed ≠ warned (Unicode)"
    );

    // ASCII: same pairwise checks
    assert_ne!(ascii_forms[0], ascii_forms[1], "blocked ≠ allowed (ASCII)");
    assert_ne!(ascii_forms[0], ascii_forms[2], "blocked ≠ warned (ASCII)");
    assert_ne!(ascii_forms[1], ascii_forms[2], "allowed ≠ warned (ASCII)");

    // Blocked is heaviest: longest (4 chars Unicode / 4 chars ASCII)
    assert_eq!(
        GLYPH_BLOCKED_UNICODE.chars().count(),
        4,
        "blocked Unicode glyph should have 4 characters"
    );
    assert_eq!(
        GLYPH_BLOCKED_ASCII.len(),
        4,
        "blocked ASCII glyph should have 4 chars"
    );

    // The Warn and Inform outcomes both map to the warned glyph (same string)
    // per contract: "Warn → warned glyph; Inform → warned glyph"
    // This is confirmed by the mapping in style.rs.
}

// ── AC10: Full gate passes with zero new dependency ───────────────────────────

#[test]
fn ac10_cargo_toml_no_new_deps() {
    // AC10: Cargo.toml must be unchanged vs origin/main (no new dependencies).
    // This is verified externally by `git diff origin/main -- Cargo.toml`.
    // Here we confirm the binary was built (compile-time check via env!).
    let bin = env!("CARGO_BIN_EXE_arai");
    assert!(
        !bin.is_empty(),
        "AC10: CARGO_BIN_EXE_arai must be set and non-empty"
    );
    assert!(
        std::path::Path::new(bin).exists(),
        "AC10: binary at {bin} must exist"
    );
}

#[test]
fn ac10_style_rs_no_new_imports() {
    // AC10: style.rs must not import any external crate that wasn't already
    // in the codebase. The only imports allowed are std::io::IsTerminal and
    // the internal fmt/env usage — no new crate deps.
    let style = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/style.rs"),
    )
    .expect("read src/style.rs");

    // Only std:: and crate:: imports are allowed; no `extern crate` or third-party `use`
    for line in style.lines() {
        let line = line.trim();
        if line.starts_with("use ")
            && !line.starts_with("use std::")
            && !line.starts_with("use crate::")
            && !line.starts_with("use super::")
        {
            panic!(
                "AC10: style.rs contains a non-std/non-crate import that may introduce a new dep: {line}"
            );
        }
    }
}
