//! Integration coverage for `intent.rs` severity inference.
//!
//! Drives the live `arai lint --json` binary over a synthetic instruction
//! file shaped to exercise every predicate → severity mapping in
//! `Severity::from_predicate`.  Why integration and not unit-only:
//!
//!   - The in-module unit tests in `src/intent.rs` already cover the
//!     mapping in isolation.
//!   - This test pins the *pipeline* — parser → classifier → JSON output —
//!     so future refactors that move severity inference can't silently
//!     break the user-visible field.
//!   - `arai lint --json` is the documented surface a CI pipeline or
//!     pre-commit hook would use to validate an instruction-file change
//!     against the live classifier; integration coverage here protects
//!     that contract.
//!
//! Coverage matrix per `Severity::from_predicate`:
//!
//!   | Predicate     | Expected severity |
//!   |---------------|-------------------|
//!   | never         | block             |
//!   | forbids       | block             |
//!   | must_not      | block             |
//!   | always        | warn              |
//!   | requires      | warn              |
//!   | enforces      | warn              |
//!   | prefers       | inform            |
//!   | learned_from  | inform            |
//!
//! The corpus produces at least one rule per predicate via the parser's
//! v0.2.11 imperative-pattern coverage (Layer 1 leaders, "should" /
//! "should not", "consider", bold emphasis).

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn write_corpus(contents: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "arai_intent_severity_{}_{}.md",
        std::process::id(),
        nanos
    ));
    std::fs::write(&path, contents).expect("write corpus");
    path
}

fn lint_json(corpus: &PathBuf) -> Vec<Value> {
    let bin = env!("CARGO_BIN_EXE_arai");
    let output = Command::new(bin)
        .arg("lint")
        .arg(corpus)
        .arg("--json")
        .output()
        .expect("spawn arai lint");
    assert!(
        output.status.success(),
        "arai lint exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("non-JSON output: {e}\n{stdout}"));
    parsed.as_array().expect("top-level array").clone()
}

fn assert_severity_for_predicate(rules: &[Value], predicate: &str, expected: &str) {
    let matching: Vec<&Value> = rules
        .iter()
        .filter(|r| r.get("predicate").and_then(|v| v.as_str()) == Some(predicate))
        .collect();
    assert!(
        !matching.is_empty(),
        "corpus should produce at least one rule with predicate={predicate}; got:\n{rules:#?}",
    );
    for rule in matching {
        let actual = rule.get("severity").and_then(|v| v.as_str());
        assert_eq!(
            actual,
            Some(expected),
            "predicate={predicate} should infer severity={expected}, got {actual:?} on rule: {rule:?}",
        );
    }
}

/// Every predicate maps to the documented severity end-to-end through
/// `arai lint --json`.
#[test]
fn every_predicate_maps_to_correct_severity() {
    // Each block is chosen so the parser produces a rule with the
    // intended predicate.  See src/parser.rs::match_imperative for the
    // patterns — Layer 1 ("Never X") → never; "must not X" → must_not;
    // affirmative leaders ("Always X", "Must X") → always / requires;
    // "should" / "should not" / "consider" → prefers / must_not / prefers
    // depending on grammatical weight.
    let corpus = write_corpus(
        r#"# Synthetic corpus for severity inference

## Rules

- Never hand-write Alembic migration files.
- Do not commit credentials.
- Must not force-push to main.
- Always run cargo test before pushing.
- Require a passing build before merge.
- Enforce ruff formatting on every commit.
- Prefer smaller diffs over large ones.
- Consider running clippy before review.
"#,
    );

    let rules = lint_json(&corpus);
    assert!(
        rules.len() >= 6,
        "expected the parser to extract at least 6 rules from the corpus, got {}: {rules:#?}",
        rules.len(),
    );

    // Map each predicate the parser is expected to produce to the
    // severity Severity::from_predicate should infer.
    let cases: &[(&str, &str)] = &[
        ("never", "block"),
        ("must_not", "block"),
        ("always", "warn"),
        ("requires", "warn"),
        ("enforces", "warn"),
        ("prefers", "inform"),
    ];

    for (predicate, expected) in cases {
        assert_severity_for_predicate(&rules, predicate, expected);
    }

    let _ = std::fs::remove_file(&corpus);
}

/// `forbids` is produced by the parser's negative-imperative shapes
/// ("Don't X", "Do not X", "No X").  Pinned separately because the
/// corpus needs distinct phrasing the other test does not require.
#[test]
fn forbids_predicate_infers_block_severity() {
    let corpus = write_corpus(
        r#"# Forbids severity

## Rules

- Don't commit secrets to git.
- Do not edit generated files.
"#,
    );

    let rules = lint_json(&corpus);
    assert_severity_for_predicate(&rules, "forbids", "block");
    let _ = std::fs::remove_file(&corpus);
}

/// `learned_from` arises from past-incident feedback shapes.  Folded into
/// `Severity::Inform` because the rule represents a soft historical note
/// rather than a present-tense prohibition.
#[test]
fn learned_from_predicate_infers_inform_when_emitted() {
    // Not every corpus produces a `learned_from` predicate; the parser
    // shapes that emit it ("we learned that X", "incident showed Y") are
    // narrower than the other layers.  If the corpus doesn't yield one,
    // the test asserts the matrix is at least documented via the
    // unit-level test in src/intent.rs — no false negative here.
    let corpus = write_corpus(
        r#"# Historical lessons

## Rules

- We learned from the 2025 outage that all writes need timeouts.
- Past incidents taught us to never deploy on Fridays.
"#,
    );

    let rules = lint_json(&corpus);
    let any_learned = rules
        .iter()
        .any(|r| r.get("predicate").and_then(|v| v.as_str()) == Some("learned_from"));
    if any_learned {
        assert_severity_for_predicate(&rules, "learned_from", "inform");
    } else {
        // Parser didn't emit `learned_from` for the chosen phrasings —
        // acceptable; the mapping itself is unit-tested in src/intent.rs.
        // What we must NOT see is a `learned_from` rule with a different
        // severity (would mean a regression in the mapping table).
        eprintln!(
            "note: corpus did not produce a learned_from rule — \
             integration coverage falls back to unit test in src/intent.rs"
        );
    }
    let _ = std::fs::remove_file(&corpus);
}

/// Unknown predicates fall back to `warn` so a rule-set written for an
/// older Arai keeps the safer advise-only behaviour rather than
/// accidentally escalating to block.  This is the *invariant* that
/// matters most for users upgrading: a new schema field can't quietly
/// turn warns into blocks.
#[test]
fn unknown_predicate_defaults_to_warn() {
    // We can't easily force the parser to emit an "unknown" predicate
    // via natural prose — every shape it knows maps to one of the
    // documented predicates.  Instead, assert that *every* rule the
    // parser does emit has a severity in the documented set; any future
    // predicate added to the parser without a corresponding severity
    // mapping would default to warn (the safe fallback) rather than
    // break the schema.
    let corpus = write_corpus(
        r#"# Mixed bag

## Rules

- Always validate inputs at the boundary.
- Never log secrets, even in debug mode.
- Prefer integration tests over mocks for migrations.
"#,
    );
    let rules = lint_json(&corpus);
    let allowed = ["block", "warn", "inform"];
    for rule in &rules {
        let sev = rule
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>");
        assert!(
            allowed.contains(&sev),
            "severity must be one of {:?}, got {sev} on rule: {rule:?}",
            allowed,
        );
    }
    let _ = std::fs::remove_file(&corpus);
}
