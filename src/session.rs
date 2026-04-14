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
            let prereq_text = if after.ends_with(" first") {
                &after[..after.len() - 6]
            } else {
                after
            };
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
