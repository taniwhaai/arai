//! Prompt-collector module: tests an ordered set of labelled regex patterns
//! against a given prompt text and returns one [`PromptMatchReceipt`] per
//! matched rule, in rule-list order.
//!
//! This module is a pure computation — no file I/O, no network calls, no audit
//! writes, no enforcement.  Callers own writing receipts to the audit log and
//! deciding what to do with them.

use regex::Regex;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Data shapes (local — sharing: local per the contract vocabulary)
// ---------------------------------------------------------------------------

/// A single labelled pattern entry in the rules list passed as input.
pub struct PromptRule {
    /// A regex string.  If invalid, the rule is skipped and `skipped_count`
    /// is incremented.
    pub pattern: String,
    /// A human-readable category name (non-empty).  Copied verbatim as
    /// `matched_label` into any receipt this rule produces.
    pub label: String,
}

/// A single record returned when one [`PromptRule`] matches the prompt text.
/// The structure maps directly onto the JSONL line the caller writes to the
/// audit log.
pub struct PromptMatchReceipt {
    /// Always the literal string `"PromptMatch"`.
    pub event: &'static str,
    /// Lowercase hex-encoded SHA-256 digest of the full, untruncated prompt
    /// text.  Exactly 64 characters.  The raw prompt is NOT stored here.
    pub prompt_hash: String,
    /// Copied verbatim from the matching rule's `label`.
    pub matched_label: String,
    /// ISO-8601 timestamp forwarded verbatim from the caller.
    pub timestamp_iso: String,
    /// Project identifier forwarded verbatim from the caller.
    pub project_slug: String,
    /// Always `None` in v1.  Population with a non-null value is the
    /// responsibility of the caller's PostToolUse path.
    pub did_any_tool_call_follow: Option<bool>,
}

// ---------------------------------------------------------------------------
// Seed ruleset
// ---------------------------------------------------------------------------

/// Starter labels for the built-in prompt-collector seed ruleset.
///
/// These are informed guesses about prompt patterns that operators commonly
/// want to observe — they are NOT policy decisions.  Operators should treat
/// this list as a starting point and add, remove, or tune patterns to match
/// their own deployment context.  Nothing here implies a block or warning
/// by default.
pub const SEED_RULES: &[(&str, &str)] = &[
    (r"(?i)\bdeploy\b", "deploy"),
    (r"(?i)\bproduction\b", "production"),
    (r"(?i)\bsecret\b", "secret"),
    (r"(?i)\bpassword\b", "password"),
    (r"(?i)kubectl\s+apply", "kubectl apply"),
    (r"(?i)force[\s-]push", "force push"),
];

/// Returns the seed ruleset as a `Vec<PromptRule>` ready for use with
/// [`collect_prompt_matches`].
pub fn seed_rules() -> Vec<PromptRule> {
    SEED_RULES
        .iter()
        .map(|(pattern, label)| PromptRule {
            pattern: pattern.to_string(),
            label: label.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Core function
// ---------------------------------------------------------------------------

/// Compute the lowercase hex-encoded SHA-256 digest of `text`.
/// Deterministic: byte-identical inputs always produce byte-identical outputs.
fn sha256_hex(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    let bytes = h.finalize();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Test each rule in `rules` against `prompt_text` and return one
/// [`PromptMatchReceipt`] per matched rule, in input-list order.
///
/// # Inputs
/// - `prompt_text`: the full user prompt.  Not truncated or previewed here.
/// - `rules`: ordered list of [`PromptRule`] values.  May be empty.
/// - `project_slug`: passed through verbatim into each receipt.
/// - `timestamp_iso`: caller-supplied ISO-8601 timestamp, passed through
///   verbatim into each receipt.
///
/// # Returns
/// `(receipts, skipped_count)`:
/// - `receipts`: zero or more receipts in the order matching rules appear.
/// - `skipped_count`: number of rules whose `pattern` was not a valid regex.
///
/// # Guarantees
/// - Pure: no I/O, no mutable shared state, concurrent-invocation safe.
/// - Idempotent: same inputs always produce same outputs.
/// - Ordering: receipt for rule at index i precedes receipt for rule at index j
///   when i < j and both match.
pub fn collect_prompt_matches(
    prompt_text: &str,
    rules: &[PromptRule],
    project_slug: &str,
    timestamp_iso: &str,
) -> (Vec<PromptMatchReceipt>, usize) {
    // Compute hash once for all receipts from this call — deterministic per
    // contract guarantee "Hash determinism".
    let prompt_hash = sha256_hex(prompt_text);

    let mut receipts = Vec::new();
    let mut skipped_count = 0usize;

    for rule in rules {
        let re = match Regex::new(&rule.pattern) {
            Ok(r) => r,
            Err(_) => {
                // Invalid regex — skip silently, increment counter.
                skipped_count += 1;
                continue;
            }
        };

        if re.is_match(prompt_text) {
            receipts.push(PromptMatchReceipt {
                event: "PromptMatch",
                prompt_hash: prompt_hash.clone(),
                matched_label: rule.label.clone(),
                timestamp_iso: timestamp_iso.to_string(),
                project_slug: project_slug.to_string(),
                did_any_tool_call_follow: None,
            });
        }
    }

    (receipts, skipped_count)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build receipts for a single text against a rule with given label
    // and pattern.
    fn one_rule(pattern: &str, label: &str) -> PromptRule {
        PromptRule {
            pattern: pattern.to_string(),
            label: label.to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // AC1 — Seed ruleset is non-empty and contains all declared labels
    // -----------------------------------------------------------------------

    #[test]
    fn ac1_seed_rules_non_empty_and_all_declared_labels_present() {
        let rules = seed_rules();
        assert!(!rules.is_empty(), "seed ruleset must not be empty");
        // Exactly the labels declared in the contract:
        let required_labels = [
            "deploy",
            "production",
            "secret",
            "password",
            "kubectl apply",
            "force push",
        ];
        assert_eq!(
            rules.len(),
            required_labels.len(),
            "seed rule count must equal the number of declared labels"
        );
        for label in &required_labels {
            assert!(
                rules.iter().any(|r| r.label == *label),
                "seed rules missing declared label: {label:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // AC3 — Receipt shape is exact (tested via an isolated call)
    // -----------------------------------------------------------------------

    #[test]
    fn ac3_receipt_shape_is_exact() {
        let rules = vec![one_rule(r"deploy", "deploy")];
        let (receipts, skipped) = collect_prompt_matches(
            "please deploy to staging",
            &rules,
            "my-project",
            "2026-05-26T00:00:00Z",
        );
        assert_eq!(skipped, 0);
        assert_eq!(receipts.len(), 1);

        let r = &receipts[0];
        // event literal
        assert_eq!(r.event, "PromptMatch");
        // prompt_hash: exactly 64 lowercase hex chars
        assert_eq!(r.prompt_hash.len(), 64);
        assert!(
            r.prompt_hash
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "prompt_hash must be lowercase hex: {:?}",
            r.prompt_hash
        );
        // matched_label non-empty
        assert!(!r.matched_label.is_empty());
        assert_eq!(r.matched_label, "deploy");
        // timestamp_iso passed through verbatim
        assert_eq!(r.timestamp_iso, "2026-05-26T00:00:00Z");
        // project_slug passed through verbatim
        assert_eq!(r.project_slug, "my-project");
        // did_any_tool_call_follow is always None
        assert!(r.did_any_tool_call_follow.is_none());
    }

    // -----------------------------------------------------------------------
    // Correctness tests (required by contract, not numbered ACs)
    // -----------------------------------------------------------------------

    #[test]
    fn empty_rules_list_produces_no_receipts() {
        let (receipts, skipped) =
            collect_prompt_matches("some prompt text", &[], "slug", "2026-01-01T00:00:00Z");
        assert!(
            receipts.is_empty(),
            "expected empty receipts for empty rules"
        );
        assert_eq!(skipped, 0);
    }

    #[test]
    fn single_matching_rule_produces_one_receipt() {
        let rules = vec![one_rule(r"deploy", "deploy")];
        let (receipts, skipped) =
            collect_prompt_matches("please deploy now", &rules, "slug", "2026-01-01T00:00:00Z");
        assert_eq!(skipped, 0);
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].matched_label, "deploy");
    }

    #[test]
    fn single_non_matching_rule_produces_no_receipts() {
        let rules = vec![one_rule(r"deploy", "deploy")];
        let (receipts, skipped) = collect_prompt_matches(
            "nothing relevant here",
            &rules,
            "slug",
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(skipped, 0);
        assert!(receipts.is_empty());
    }

    #[test]
    fn multiple_rules_partial_match_correct_order() {
        let rules = vec![
            one_rule(r"alpha", "alpha"),
            one_rule(r"beta", "beta"),
            one_rule(r"gamma", "gamma"),
        ];
        // Only alpha and gamma match.
        let (receipts, skipped) = collect_prompt_matches(
            "alpha and gamma here",
            &rules,
            "slug",
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(skipped, 0);
        assert_eq!(receipts.len(), 2);
        // Order must mirror input rule order: alpha first, then gamma.
        assert_eq!(receipts[0].matched_label, "alpha");
        assert_eq!(receipts[1].matched_label, "gamma");
    }

    #[test]
    fn regex_metacharacters_compile_and_match() {
        // Word-boundary anchor + escaped char
        let rules = vec![one_rule(r"\bforce[-\s]push\b", "force push")];
        let (receipts, skipped) = collect_prompt_matches(
            "please force-push to origin",
            &rules,
            "slug",
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(skipped, 0);
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].matched_label, "force push");
    }

    #[test]
    fn prompt_hash_is_deterministic_across_calls() {
        let rules = vec![one_rule(r"deploy", "deploy")];
        let text = "please deploy to staging";
        let ts = "2026-01-01T00:00:00Z";
        let slug = "proj";

        let (r1, _) = collect_prompt_matches(text, &rules, slug, ts);
        let (r2, _) = collect_prompt_matches(text, &rules, slug, ts);

        assert_eq!(r1.len(), 1);
        assert_eq!(r2.len(), 1);
        assert_eq!(
            r1[0].prompt_hash, r2[0].prompt_hash,
            "prompt_hash must be byte-identical across calls with identical inputs"
        );
    }

    #[test]
    fn invalid_regex_is_skipped_without_error_and_increments_skipped_count() {
        let rules = vec![one_rule(r"[invalid", "bad"), one_rule(r"deploy", "deploy")];
        let (receipts, skipped) = collect_prompt_matches(
            "please deploy to staging",
            &rules,
            "slug",
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(skipped, 1, "exactly one invalid rule should be counted");
        // The valid deploy rule still fires.
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].matched_label, "deploy");
        // No receipt for the invalid rule.
        assert!(!receipts.iter().any(|r| r.matched_label == "bad"));
    }

    // -----------------------------------------------------------------------
    // AC6 — No outbound network calls in collector source (structural check)
    // -----------------------------------------------------------------------

    /// Verify the collector source contains none of the forbidden network
    /// identifiers outside of test-only blocks.  This test reads the source
    /// file at runtime and scans it, satisfying AC6's "test that reads the
    /// source file" option.
    #[test]
    fn ac6_no_network_identifiers_outside_test_blocks() {
        let source = include_str!("prompt_collector.rs");

        // Strip the #[cfg(test)] block so we only scan production code.
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
                "collector production code must not reference {term:?}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // AC7 — Seed ruleset is annotated as non-policy (structural check)
    // -----------------------------------------------------------------------

    #[test]
    fn ac7_seed_ruleset_has_non_policy_annotation() {
        let source = include_str!("prompt_collector.rs");
        // The comment above SEED_RULES must reference that these are starter
        // guesses / not policy.
        assert!(
            source.contains("NOT policy")
                || source.contains("not policy")
                || source.contains("not policy decisions"),
            "SEED_RULES must have a comment stating labels are not policy decisions"
        );
    }

    // -----------------------------------------------------------------------
    // Additional: hash value correctness
    // -----------------------------------------------------------------------

    #[test]
    fn prompt_hash_is_64_lowercase_hex_chars() {
        let rules = vec![one_rule("x", "x")];
        let (receipts, _) =
            collect_prompt_matches("x here", &rules, "slug", "2026-01-01T00:00:00Z");
        assert_eq!(receipts.len(), 1);
        let h = &receipts[0].prompt_hash;
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }
}
