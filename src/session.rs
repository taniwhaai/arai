use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A record of a tool call within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallRecord {
    tool_name: String,
    terms: Vec<String>,
}

/// Session state — tracks what has happened in the current Claude Code session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SessionState {
    tool_calls: Vec<ToolCallRecord>,
    /// Triple IDs that have already had a full guardrail injection in this
    /// session.  When the same rule fires a second time the hook handler
    /// emits a compact one-liner instead of re-injecting source/layer/
    /// severity that the model already saw — both for token economics and
    /// because re-reading the same rule N times dilutes the model's
    /// attention to it.  Defaulted to empty for old session files so the
    /// first hook call after upgrade produces full context (the safe
    /// fallback).
    #[serde(default)]
    seen_rules: Vec<i64>,
}

/// Get the session state file path.
fn session_path(arai_base: &Path, session_id: &str) -> PathBuf {
    arai_base.join("sessions").join(format!("{session_id}.json"))
}

/// Load session state from disk.
fn load_session(arai_base: &Path, session_id: &str) -> SessionState {
    let path = session_path(arai_base, session_id);
    if let Ok(content) = std::fs::read_to_string(&path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        SessionState::default()
    }
}

/// Save session state to disk.
fn save_session(arai_base: &Path, session_id: &str, state: &SessionState) {
    let path = session_path(arai_base, session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(json) = serde_json::to_string(state) {
        std::fs::write(&path, json).ok();
    }
}

/// Record a tool call in the session state (called from PostToolUse).
pub fn record_tool_call(
    arai_base: &Path,
    session_id: &str,
    tool_name: &str,
    terms: &[String],
) {
    let mut state = load_session(arai_base, session_id);
    state.tool_calls.push(ToolCallRecord {
        tool_name: tool_name.to_string(),
        terms: terms.to_vec(),
    });

    // Cap at 200 records to prevent unbounded growth
    if state.tool_calls.len() > 200 {
        state.tool_calls = state.tool_calls.split_off(state.tool_calls.len() - 200);
    }

    save_session(arai_base, session_id, &state);
}

/// Check if prerequisite terms have been satisfied in the session.
/// Returns true if ALL prerequisite terms appear in at least one past tool call.
pub fn prerequisite_met(
    arai_base: &Path,
    session_id: &str,
    prerequisite_terms: &[String],
) -> bool {
    if prerequisite_terms.is_empty() {
        return false; // No prerequisite → not met (fire the rule)
    }

    let state = load_session(arai_base, session_id);

    // Check if any past tool call contains ALL prerequisite terms
    state.tool_calls.iter().any(|record| {
        prerequisite_terms.iter().all(|req| {
            record.terms.iter().any(|t| t == req)
        })
    })
}

/// Partition matched-rule triple_ids into `(unseen, seen)` for this session.
/// Unseen → emit full context.  Seen → emit compact "still active" form.
/// Reads session state once; doesn't mutate.  Empty `session_id` yields
/// every id as unseen (we have no way to track without a session key).
pub fn partition_seen_rules(
    arai_base: &Path,
    session_id: &str,
    triple_ids: &[i64],
) -> (Vec<i64>, Vec<i64>) {
    if session_id.is_empty() {
        return (triple_ids.to_vec(), Vec::new());
    }
    let state = load_session(arai_base, session_id);
    let seen_set: std::collections::HashSet<i64> = state.seen_rules.iter().copied().collect();
    let mut unseen = Vec::new();
    let mut seen = Vec::new();
    for id in triple_ids {
        if seen_set.contains(id) {
            seen.push(*id);
        } else {
            unseen.push(*id);
        }
    }
    (unseen, seen)
}

/// Mark a batch of rules as having had their full context injected in this
/// session.  Subsequent firings of the same triple_id in this session will
/// emit a compact form via `partition_seen_rules`.  No-op for empty
/// `session_id` (nothing to key on).
pub fn mark_rules_seen(arai_base: &Path, session_id: &str, triple_ids: &[i64]) {
    if session_id.is_empty() || triple_ids.is_empty() {
        return;
    }
    let mut state = load_session(arai_base, session_id);
    let mut existing: std::collections::HashSet<i64> = state.seen_rules.iter().copied().collect();
    let mut changed = false;
    for id in triple_ids {
        if existing.insert(*id) {
            state.seen_rules.push(*id);
            changed = true;
        }
    }
    if changed {
        // Cap at 500 ids to prevent unbounded growth — a session that touches
        // 500 distinct rules has bigger problems than the seen-rules list.
        if state.seen_rules.len() > 500 {
            let drop = state.seen_rules.len() - 500;
            state.seen_rules.drain(..drop);
        }
        save_session(arai_base, session_id, &state);
    }
}

/// Extract prerequisite terms from a rule's object text.
/// Looks for patterns like "without running X first", "before X", "unless X".
pub fn extract_prerequisite(object: &str) -> Vec<String> {
    let lower = object.to_lowercase();

    // Match patterns: "without running X first", "without X", "before running X"
    let patterns = [
        "without running ",
        "without first running ",
        "before running ",
        "unless you run ",
        "unless you've run ",
        "without first ",
        "without ",
    ];

    for pattern in &patterns {
        if let Some(pos) = lower.find(pattern) {
            let after = &lower[pos + pattern.len()..];
            // Take everything up to "first" or end of string
            let prereq_text = after.strip_suffix(" first").unwrap_or(after);
            let prereq_text = prereq_text.trim_end_matches('.').trim();

            // Extract tool-like terms from the prerequisite text
            let terms: Vec<String> = prereq_text
                .split_whitespace()
                .filter(|w| {
                    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
                    w.len() >= 2 && !matches!(w, "the" | "a" | "an" | "all" | "and" | "or" | "to" | "in")
                })
                .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
                .collect();

            if !terms.is_empty() {
                return terms;
            }
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_prerequisite_without_running() {
        let terms = extract_prerequisite("git push without running cargo test first");
        assert!(terms.contains(&"cargo".to_string()));
        assert!(terms.contains(&"test".to_string()));
    }

    #[test]
    fn test_extract_prerequisite_without() {
        let terms = extract_prerequisite("deploy without tests");
        assert!(terms.contains(&"tests".to_string()));
    }

    #[test]
    fn test_extract_prerequisite_none() {
        let terms = extract_prerequisite("force-push to main");
        assert!(terms.is_empty());
    }

    #[test]
    fn test_partition_seen_rules_empty_session_id_returns_all_unseen() {
        // Without a session_id we can't track anything, so every id reads
        // as unseen and the hook will emit full context for all of them.
        let dir = std::env::temp_dir().join("arai_seen_test_empty");
        let (unseen, seen) = partition_seen_rules(&dir, "", &[1, 2, 3]);
        assert_eq!(unseen, vec![1, 2, 3]);
        assert!(seen.is_empty());
    }

    #[test]
    fn test_partition_and_mark_round_trip() {
        let dir = std::env::temp_dir().join("arai_seen_test_round_trip");
        std::fs::create_dir_all(&dir).ok();
        let _ = std::fs::remove_file(session_path(&dir, "rt-sess"));

        // Fresh session: nothing seen.
        let (unseen, seen) = partition_seen_rules(&dir, "rt-sess", &[10, 20, 30]);
        assert_eq!(unseen, vec![10, 20, 30]);
        assert!(seen.is_empty());

        // Mark 10 and 20 seen; 30 still unseen.
        mark_rules_seen(&dir, "rt-sess", &[10, 20]);
        let (unseen, seen) = partition_seen_rules(&dir, "rt-sess", &[10, 20, 30]);
        assert_eq!(unseen, vec![30]);
        assert_eq!(seen, vec![10, 20]);

        // Marking already-seen ids is idempotent.
        mark_rules_seen(&dir, "rt-sess", &[10, 20]);
        let state = load_session(&dir, "rt-sess");
        assert_eq!(state.seen_rules.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_seen_rules_isolated_per_session() {
        let dir = std::env::temp_dir().join("arai_seen_test_isolated");
        std::fs::create_dir_all(&dir).ok();
        let _ = std::fs::remove_file(session_path(&dir, "iso-a"));
        let _ = std::fs::remove_file(session_path(&dir, "iso-b"));

        mark_rules_seen(&dir, "iso-a", &[100]);

        // Different session — id 100 is fresh again.  This is the spec:
        // a model in a new session needs the full rule context, even if
        // a previous session already saw it.
        let (unseen, _) = partition_seen_rules(&dir, "iso-b", &[100]);
        assert_eq!(unseen, vec![100]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_seen_rules_capped_at_500() {
        // Defensive: an unbounded list could grow on a session that
        // touches every rule in a 1000-rule project.  Cap at 500 to keep
        // the JSON small; the cost is that very-old `seen_before`
        // markers in a long session may roll out and a re-injection
        // happens.  Acceptable given the cap.
        let dir = std::env::temp_dir().join("arai_seen_test_cap");
        std::fs::create_dir_all(&dir).ok();
        let _ = std::fs::remove_file(session_path(&dir, "cap-sess"));

        let many: Vec<i64> = (0..600).collect();
        mark_rules_seen(&dir, "cap-sess", &many);
        let state = load_session(&dir, "cap-sess");
        assert_eq!(state.seen_rules.len(), 500, "should cap at 500 entries");
        // The newest entries must be retained — drain pulls from the front.
        assert_eq!(*state.seen_rules.last().unwrap(), 599);
        assert_eq!(*state.seen_rules.first().unwrap(), 100);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_prerequisite_met() {
        let dir = std::env::temp_dir().join("arai_session_test");
        std::fs::create_dir_all(&dir).ok();

        // Record cargo test
        record_tool_call(&dir, "sess1", "Bash", &["cargo".to_string(), "test".to_string()]);

        // Check if prerequisite is met
        let prereqs = vec!["cargo".to_string(), "test".to_string()];
        assert!(prerequisite_met(&dir, "sess1", &prereqs));

        // Different session — not met
        assert!(!prerequisite_met(&dir, "sess2", &prereqs));

        std::fs::remove_dir_all(&dir).ok();
    }
}
