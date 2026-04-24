//! Compliance tracking — did the model honour the rules Arai surfaced?
//!
//! When PreToolUse fires a guardrail, Arai records the firing in the audit
//! log.  When PostToolUse runs for the same session, this module re-reads
//! the recent Pre-firings and asks: "did the model still do the thing that
//! rule was about?"  The answer (`Obeyed | Ignored | Unclear`) is appended
//! to the audit log as its own `Compliance` event so
//! `arai audit --event=Compliance` and `arai stats --compliance` can surface
//! compliance rates per rule.
//!
//! The check is cheap and best-effort: we tokenise the Post tool input and
//! look for any "evidence" word from the Pre rule's object.  If the
//! forbidden phrase is still in the executed command, the model ignored
//! the rule.  If not, the model either complied or the rule was a false
//! positive — both surface as `Obeyed`.  Aggregate behaviour over many
//! firings matters more than any single verdict.

use crate::audit;
use crate::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Compliance verdict for one rule across a Pre/Post pair.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Forbidden phrase absent from the executed command, or affirmative
    /// rule's evidence present — the model did (or avoided) the thing.
    Obeyed,
    /// Forbidden phrase still present in the executed command — the model
    /// ran the action the rule warned against.
    Ignored,
    /// Not enough signal to decide (empty rule object, evidence ambiguous).
    Unclear,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Obeyed => "obeyed",
            Outcome::Ignored => "ignored",
            Outcome::Unclear => "unclear",
        }
    }
}

/// Window in seconds during which a PostToolUse is considered correlated
/// with a recent PreToolUse firing.  Five minutes is generous enough for
/// long-running tools (test suites, builds) but tight enough to avoid
/// matching unrelated events from earlier in the session.
const CORRELATION_WINDOW_SECS: u64 = 300;

/// After a PostToolUse fires, look up recent PreToolUse firings for the same
/// session and emit a compliance verdict per rule.
pub fn record_post_compliance(
    cfg: &Config,
    session_id: &str,
    tool_name: &str,
    post_terms: &[String],
    post_preview: &str,
) {
    if session_id.is_empty() {
        return;
    }
    let now_secs = current_epoch_secs();
    let since = now_secs.saturating_sub(CORRELATION_WINDOW_SECS);

    let entries = match audit::query(
        &cfg.arai_base_dir,
        &cfg.project_slug(),
        Some(since),
        Some(tool_name),
        Some("PreToolUse"),
        50,
    ) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut items: Vec<Value> = Vec::new();
    for entry in entries {
        if entry.get("session").and_then(|v| v.as_str()) != Some(session_id) {
            continue;
        }
        let pre_ts = entry
            .get("ts")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let pre_decision = entry
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or("inject")
            .to_string();
        let rules = entry
            .get("rules")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for r in rules {
            let triple_id = r.get("triple_id").and_then(|v| v.as_i64()).unwrap_or(-1);
            let predicate = r
                .get("predicate")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let object = r
                .get("object")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let severity = r
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("warn")
                .to_string();

            let outcome = evaluate(&predicate, &object, post_terms, post_preview);

            items.push(json!({
                "pre_ts": pre_ts.clone(),
                "pre_decision": pre_decision.clone(),
                "triple_id": triple_id,
                "predicate": predicate,
                "object": object,
                "severity": severity,
                "outcome": outcome.as_str(),
            }));
        }
    }

    if items.is_empty() {
        return;
    }

    audit::record_event(
        cfg,
        "Compliance",
        tool_name,
        session_id,
        json!({ "rules": items }),
    );
}

/// Decide whether a Post tool invocation shows the model honoured a rule
/// that fired on the matching Pre event.  Pure — no IO.
pub fn evaluate(
    predicate: &str,
    object: &str,
    post_terms: &[String],
    post_preview: &str,
) -> Outcome {
    let lower_obj = object.to_lowercase();
    let lower_preview = post_preview.to_lowercase();

    // Tokenise the rule object into meaningful words (≥3 chars).  Short
    // words like "the", "a", "is" are noise — they'd match every command.
    let evidence: Vec<&str> = lower_obj
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|w| w.len() >= 3)
        .collect();

    if evidence.is_empty() {
        return Outcome::Unclear;
    }

    let post_terms_lower: Vec<String> = post_terms.iter().map(|t| t.to_lowercase()).collect();

    let matches = evidence
        .iter()
        .filter(|w| {
            post_terms_lower.iter().any(|t| t.contains(*w))
                || lower_preview.contains(*w)
        })
        .count();

    let prohibitive = matches!(predicate, "never" | "forbids" | "must_not");

    match (matches > 0, prohibitive) {
        // Forbidden phrase still present → model ignored the warning.
        (true, true) => Outcome::Ignored,
        // Forbidden phrase absent → model honoured the rule.
        (false, true) => Outcome::Obeyed,
        // Affirmative rule + evidence present → model did the required thing.
        (true, false) => Outcome::Obeyed,
        // Affirmative rule + no evidence → can't tell from this single call.
        (false, false) => Outcome::Unclear,
    }
}

fn current_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_outcome_as_str() {
        assert_eq!(Outcome::Obeyed.as_str(), "obeyed");
        assert_eq!(Outcome::Ignored.as_str(), "ignored");
        assert_eq!(Outcome::Unclear.as_str(), "unclear");
    }

    #[test]
    fn test_evaluate_prohibitive_ignored() {
        // Rule: never force-push — and the Post command still contains "force"
        let outcome = evaluate(
            "never",
            "force-push to main",
            &["git".to_string(), "push".to_string(), "--force".to_string()],
            "git push --force origin main",
        );
        assert_eq!(outcome, Outcome::Ignored);
    }

    #[test]
    fn test_evaluate_prohibitive_obeyed() {
        // Rule: never force-push — Post command has no force/push-force phrase
        let outcome = evaluate(
            "never",
            "force-push to main",
            &["git".to_string(), "status".to_string()],
            "git status",
        );
        assert_eq!(outcome, Outcome::Obeyed);
    }

    #[test]
    fn test_evaluate_affirmative_obeyed() {
        // Rule: requires running the test suite — Post command runs tests.
        // Evidence words "test" and "suite" appear in the invocation.
        let outcome = evaluate(
            "requires",
            "running the test suite before commit",
            &["cargo".to_string(), "test".to_string()],
            "cargo test --release",
        );
        assert_eq!(outcome, Outcome::Obeyed);
    }

    #[test]
    fn test_evaluate_affirmative_unclear_when_no_evidence() {
        // Rule: requires running tests — Post command unrelated
        let outcome = evaluate(
            "requires",
            "run tests before commit",
            &["ls".to_string()],
            "ls -la",
        );
        assert_eq!(outcome, Outcome::Unclear);
    }

    #[test]
    fn test_evaluate_empty_object_is_unclear() {
        let outcome = evaluate("never", "", &["anything".to_string()], "some command");
        assert_eq!(outcome, Outcome::Unclear);
    }

    #[test]
    fn test_evaluate_short_words_dont_trigger() {
        // Rule object has only short words — should return Unclear because
        // no evidence tokens passed the length filter.
        let outcome = evaluate("never", "do it", &["cargo".to_string()], "cargo build");
        assert_eq!(outcome, Outcome::Unclear);
    }
}
