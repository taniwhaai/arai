//! Integration regression test for parser coverage.
//!
//! Drives the live `arai lint --json` binary against
//! `tests/parser_coverage/corpus.md` (a synthetic CLAUDE.md shaped to
//! exercise every pattern in `parser.rs`, including the v0.2.11
//! coverage-broadening additions).  The corpus file is the single source
//! of truth for parser behaviour expectations; this test asserts:
//!
//!   1. Total count: the corpus produces exactly the expected number of
//!      rules.  Catches regressions where a pattern starts under- or
//!      over-extracting.
//!   2. Spot-check positives: for each significant pattern (Layer 1
//!      additions, conditional-imperative, section-context-gated `use X`,
//!      Layer 6 verb expansion), assert at least one rule with the
//!      expected predicate exists.
//!   3. Spot-check negatives: rules that MUST NOT extract (bold-label
//!      `**No build process**`, `**Consider constraints:**`, prose
//!      descriptions, conditionals with unrecognised verbs) — assert the
//!      object text doesn't appear in any extracted rule.
//!
//! Why driven through the binary instead of importing the parser as a
//! library: arai is a binary-only crate (no `[lib]` target), so external
//! tests can't `use arai::parser`.  Running the actual binary has the
//! side benefit of exercising the full lint→parser→intent classification
//! pipeline end-to-end.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("parser_coverage")
        .join("corpus.md")
}

fn run_lint() -> Vec<Value> {
    let bin = env!("CARGO_BIN_EXE_arai");
    let corpus = corpus_path();
    let output = Command::new(bin)
        .arg("lint")
        .arg(&corpus)
        .arg("--json")
        .output()
        .expect("spawn arai lint");

    assert!(
        output.status.success(),
        "arai lint exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("arai lint --json output not valid JSON: {e}\n--- stdout ---\n{stdout}\n"));
    parsed
        .as_array()
        .expect("expected top-level JSON array")
        .clone()
}

fn has_rule_with(rules: &[Value], predicate: &str, object_substring: &str) -> bool {
    rules.iter().any(|r| {
        let p = r.get("predicate").and_then(|v| v.as_str()).unwrap_or("");
        let o = r.get("object").and_then(|v| v.as_str()).unwrap_or("");
        p == predicate && o.to_lowercase().contains(&object_substring.to_lowercase())
    })
}

fn has_object_substring(rules: &[Value], needle: &str) -> bool {
    rules.iter().any(|r| {
        r.get("object")
            .and_then(|v| v.as_str())
            .map(|o| o.to_lowercase().contains(&needle.to_lowercase()))
            .unwrap_or(false)
    })
}

#[test]
fn corpus_extracts_expected_total() {
    // The corpus has been hand-counted: see tests/parser_coverage/corpus.md
    // for the line-by-line layout.  Adjust both numbers together when
    // editing the corpus.
    //
    // Lower bound is the safer assertion — over-extraction (false
    // positives) is a defect we want to catch, but a future Layer 8 that
    // adds genuinely new coverage shouldn't break this test on day one.
    // Upper bound caps the over-extraction risk: more than this means a
    // pattern got loosened too far.
    let rules = run_lint();
    assert!(
        rules.len() >= 40,
        "expected at least 40 rules from corpus; got {}",
        rules.len(),
    );
    assert!(
        rules.len() <= 55,
        "expected at most 55 rules from corpus (over-extraction guard); got {}",
        rules.len(),
    );
}

// ── Layer 1 additions — must extract with the right severity-mapped predicate.

#[test]
fn corpus_should_not_extracts_as_must_not() {
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "must_not", "binary blobs")
            || has_rule_with(&rules, "must_not", "commit binary"),
        "expected `should not commit binary blobs` to extract as must_not; got {rules:#?}",
    );
}

#[test]
fn corpus_should_extracts_as_prefers() {
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "prefers", "linter before"),
        "expected `Should run linter before commits` as prefers; got {rules:#?}",
    );
}

#[test]
fn corpus_cannot_extracts_as_must_not() {
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "must_not", "private keys"),
        "expected `Cannot commit private keys` as must_not",
    );
}

#[test]
fn corpus_make_sure_extracts_as_enforces() {
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "enforces", "tests pass"),
        "expected `Make sure tests pass` as enforces",
    );
}

#[test]
fn corpus_consider_extracts_as_prefers() {
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "prefers", "compression"),
        "expected `Consider compression for distribution` as prefers",
    );
}

#[test]
fn corpus_recommend_extracts_as_prefers() {
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "prefers", "uv over pip"),
        "expected `Recommend using uv over pip` as prefers",
    );
}

#[test]
fn corpus_bare_no_extracts_as_must_not() {
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "must_not", "ai attribution"),
        "expected `No AI attribution in commit messages` as must_not",
    );
}

// ── Layer 5 — `use X` inside Conventions section.

#[test]
fn corpus_use_in_conventions_section_extracts() {
    let rules = run_lint();
    assert!(
        has_object_substring(&rules, "cn() utility"),
        "expected `Use the cn() utility` (under ## Conventions) to extract",
    );
}

// ── Layer 6 verb additions.

#[test]
fn corpus_create_imperative_extracts() {
    let rules = run_lint();
    assert!(
        has_object_substring(&rules, "lookup functions"),
        "expected `Create lookup functions` to extract",
    );
}

#[test]
fn corpus_implement_imperative_extracts() {
    let rules = run_lint();
    assert!(
        has_object_substring(&rules, "try_from"),
        "expected `Implement try_from` to extract",
    );
}

// ── Layer 7 conditional imperatives.

#[test]
fn corpus_conditional_when_extracts() {
    let rules = run_lint();
    assert!(
        has_object_substring(&rules, "tests in isolation"),
        "expected conditional `When working in parallel, run tests in isolation` to extract",
    );
}

#[test]
fn corpus_conditional_arrow_extracts() {
    let rules = run_lint();
    assert!(
        has_object_substring(&rules, "data download required"),
        "expected `If missing → show \"Data Download Required\" dialog` to extract",
    );
}

// ── Negative cases — the high-risk discriminators.

#[test]
fn corpus_bold_no_build_process_does_not_extract() {
    // The single most important negative: `**No build process** - this is
    // a zero-build extension.` is feature-absence DESCRIPTION, not a
    // rule.  If this regresses, the `^no` Layer 1b pattern has broken
    // its bold-label guard.
    let rules = run_lint();
    assert!(
        !has_object_substring(&rules, "zero-build extension"),
        "REGRESSION: bold-label `**No build process**` extracted as a rule",
    );
    assert!(
        !has_object_substring(&rules, "build process"),
        "REGRESSION: any object containing 'build process' suggests the bold-No guard misfired",
    );
}

#[test]
fn corpus_bold_no_cors_handling_does_not_extract() {
    let rules = run_lint();
    assert!(
        !has_object_substring(&rules, "traefik manages"),
        "REGRESSION: bold-label `**No CORS handling**` extracted as a rule",
    );
}

#[test]
fn corpus_bold_consider_label_does_not_extract() {
    let rules = run_lint();
    assert!(
        !has_object_substring(&rules, "goals and limitations"),
        "REGRESSION: `**Consider constraints:**` (section heading) extracted as a rule",
    );
}

#[test]
fn corpus_conditional_with_see_verb_does_not_extract() {
    // `When uncertain, see the troubleshooting guide` — `see` is not
    // an imperative in the whitelist, so the line should be skipped to
    // avoid extracting bullet-shaped prose continuations.
    let rules = run_lint();
    assert!(
        !has_object_substring(&rules, "troubleshooting guide"),
        "REGRESSION: conditional with non-imperative verb `see` was extracted",
    );
}

#[test]
fn corpus_descriptive_prose_does_not_extract() {
    let rules = run_lint();
    assert!(
        !has_object_substring(&rules, "same socket location"),
        "REGRESSION: descriptive prose `The same seed always gives...` extracted as a rule",
    );
    assert!(
        !has_object_substring(&rules, "share a lock"),
        "REGRESSION: descriptive prose `Thread-safe queries...` extracted as a rule",
    );
}

#[test]
fn corpus_bold_emphasis_on_always_still_extracts() {
    // Conversely, `**Always** run tests` IS a rule — bold emphasis on a
    // Layer 1 leader.  If this regresses, the bold-label guard is too
    // aggressive.
    let rules = run_lint();
    assert!(
        has_rule_with(&rules, "always", "tests before push"),
        "REGRESSION: `**Always** run tests` (emphasis-on-rule) was incorrectly skipped",
    );
}
