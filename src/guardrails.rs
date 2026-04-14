use crate::parser::KNOWN_TOOLS;
use crate::store::Guardrail;
use serde_json::Value;

/// Tools that never need guardrails — fast exit, no DB query.
const SKIP_TOOLS: &[&str] = &["Read", "Glob", "Agent", "ToolSearch"];

/// Noise words to skip when extracting terms from Bash commands.
const NOISE_WORDS: &[&str] = &[
    "run", "sudo", "cd", "echo", "python", "bash", "sh", "cat", "head",
    "tail", "ls", "mkdir", "rm", "cp", "mv", "true", "false",
    "set", "export", "source", "exec", "xargs", "env", "time", "nice",
    "nohup", "eval",
];

/// Short tokens that are valid tool names (allowlisted despite < 3 chars).
const SHORT_TOOL_ALLOWLIST: &[&str] = &["go", "gh", "uv", "mv", "rm", "ls", "cd", "nix"];

/// Check if a tool should skip guardrail matching entirely.
pub fn should_skip_tool(tool_name: &str) -> bool {
    SKIP_TOOLS.contains(&tool_name)
}

/// Extract matching terms from a tool call's context.
pub fn extract_terms(tool_name: &str, tool_input: &Value) -> Vec<String> {
    match tool_name {
        "Bash" => extract_bash_terms(tool_input),
        "Edit" | "Write" | "NotebookEdit" => extract_file_terms(tool_input),
        "Grep" => extract_grep_terms(tool_input),
        _ => extract_generic_terms(tool_input),
    }
}

/// Enrich terms with tools discovered from the code graph.
/// For file-based operations, queries what tools sibling files import.
pub fn enrich_terms_from_graph(
    terms: &mut Vec<String>,
    tool_name: &str,
    tool_input: &Value,
    store: &crate::store::Store,
) {
    // Only enrich for file-based operations
    let file_path = match tool_name {
        "Edit" | "Write" | "NotebookEdit" => {
            tool_input.get("file_path").and_then(|v| v.as_str())
        }
        _ => None,
    };

    if let Some(path) = file_path {
        if let Ok(graph_tools) = store.query_tools_for_path(path) {
            for tool in graph_tools {
                if !terms.contains(&tool) {
                    terms.push(tool);
                }
            }
        }
    }
}

fn extract_bash_terms(tool_input: &Value) -> Vec<String> {
    let command = match tool_input.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => return Vec::new(),
    };

    let mut terms = Vec::new();

    // Split on pipes, &&, ;
    for segment in command.split(['|', ';']) {
        let segment = segment.trim();
        // Also split on &&
        for sub in segment.split("&&") {
            let sub = sub.trim();
            extract_tokens_from_segment(sub, &mut terms);
        }
    }

    terms.sort();
    terms.dedup();
    terms
}

fn extract_tokens_from_segment(segment: &str, terms: &mut Vec<String>) {
    for token in shell_tokenize(segment) {
        // Quoted strings may contain multiple words — split further
        for word in token.split_whitespace() {
            classify_token(word, terms);
        }
    }
}

fn classify_token(token: &str, terms: &mut Vec<String>) {
    // Strip path prefixes
    let base = token.rsplit('/').next().unwrap_or(token);
    let lower = base.to_lowercase();

    // Skip flags
    if lower.starts_with('-') {
        return;
    }

    // Skip pure digits
    if lower.chars().all(|c| c.is_ascii_digit()) {
        return;
    }

    // Skip noise words
    if NOISE_WORDS.contains(&lower.as_str()) {
        return;
    }

    // Check short token allowlist
    if lower.len() < 3 {
        if SHORT_TOOL_ALLOWLIST.contains(&lower.as_str()) {
            terms.push(lower);
        }
        return;
    }

    terms.push(lower);
}

fn extract_file_terms(tool_input: &Value) -> Vec<String> {
    let mut terms = Vec::new();

    if let Some(path) = tool_input.get("file_path").and_then(|v| v.as_str()) {
        for component in path.split('/') {
            // Take stem (strip extension)
            let stem = if component.contains('.') {
                component.split('.').next().unwrap_or(component)
            } else {
                component
            };

            let lower = stem.to_lowercase();

            if lower.len() < 3 {
                if SHORT_TOOL_ALLOWLIST.contains(&lower.as_str()) {
                    terms.push(lower);
                }
                continue;
            }

            terms.push(lower);
        }
    }

    // Content sniffing: scan file content for known tool names
    // This catches e.g. "from alembic import op" in a migration being written
    let content_fields = ["content", "new_string", "old_string"];
    for field in &content_fields {
        if let Some(content) = tool_input.get(*field).and_then(|v| v.as_str()) {
            sniff_content_for_tools(content, &mut terms);
        }
    }

    terms.sort();
    terms.dedup();
    terms
}

/// Public entry point for content sniffing from hooks.
pub fn sniff_content_for_tools_pub(content: &str, terms: &mut Vec<String>) {
    sniff_content_for_tools(content, terms);
}

/// Scan text content for known tool/library names.
/// Uses word-boundary-aware matching to avoid false positives.
fn sniff_content_for_tools(content: &str, terms: &mut Vec<String>) {
    let lower = content.to_lowercase();
    for tool in KNOWN_TOOLS {
        // Word boundary check: tool must be surrounded by non-alphanumeric chars
        for (idx, _) in lower.match_indices(tool) {
            let before_ok = idx == 0
                || !lower.as_bytes()[idx - 1].is_ascii_alphanumeric();
            let after_idx = idx + tool.len();
            let after_ok = after_idx >= lower.len()
                || !lower.as_bytes()[after_idx].is_ascii_alphanumeric();
            if before_ok && after_ok {
                terms.push(tool.to_string());
                break; // Found once is enough
            }
        }
    }
}

fn extract_grep_terms(tool_input: &Value) -> Vec<String> {
    let mut terms = vec!["grep".to_string()];

    // Also extract from pattern and path if present
    if let Some(path) = tool_input.get("path").and_then(|v| v.as_str()) {
        for component in path.split('/') {
            let stem = if component.contains('.') {
                component.split('.').next().unwrap_or(component)
            } else {
                component
            };
            let lower = stem.to_lowercase();
            if lower.len() >= 3 {
                terms.push(lower);
            }
        }
    }

    terms.sort();
    terms.dedup();
    terms
}

fn extract_generic_terms(tool_input: &Value) -> Vec<String> {
    let mut terms = Vec::new();

    // Extract string values from tool_input
    if let Some(obj) = tool_input.as_object() {
        for (_key, val) in obj {
            if let Some(s) = val.as_str() {
                for word in s.split_whitespace() {
                    let lower = word.to_lowercase();
                    let clean = lower.trim_matches(|c: char| !c.is_alphanumeric());
                    if clean.len() >= 3 && !NOISE_WORDS.contains(&clean) {
                        terms.push(clean.to_string());
                    }
                }
            }
        }
    }

    terms.sort();
    terms.dedup();
    terms
}

/// Match guardrails against extracted terms, filtering by classified intent and timing.
pub fn match_guardrails(
    guardrails: &[Guardrail],
    terms: &[String],
    tool_name: &str,
    hook_event: &str,
    store: &crate::store::Store,
) -> Vec<Guardrail> {
    guardrails
        .iter()
        .filter(|g| {
            // Check timing — only fire rules meant for this hook event
            if let Ok(Some(intent)) = store.get_rule_intent(g.triple_id) {
                // Rule must match the current hook event
                if intent.timing.hook_event() != hook_event {
                    return false;
                }

                // For tool-call-timed rules, also check subject and tool scope
                if intent.timing == crate::intent::Timing::ToolCall {
                    let subj = g.subject.to_lowercase();
                    let subject_matches = terms.iter().any(|t| subj.contains(t));
                    subject_matches && crate::intent::tool_matches_intent(&intent, tool_name)
                } else {
                    // Stop/Start/Principle rules don't need subject matching —
                    // they fire based on timing, not tool context
                    true
                }
            } else {
                // No intent classified — fall back to PreToolUse with subject matching
                if hook_event != "PreToolUse" {
                    return false;
                }
                let subj = g.subject.to_lowercase();
                terms.iter().any(|t| subj.contains(t))
            }
        })
        .cloned()
        .collect()
}

/// Maximum number of rules to include in a single hook response.
const MAX_RULES_PER_HOOK: usize = 5;

/// Format matched guardrails as additionalContext string.
/// Limits output to the top N rules by confidence to avoid context bloat.
pub fn format_context(matched: &[Guardrail]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Arai guardrails:".to_string());
    let limit = matched.len().min(MAX_RULES_PER_HOOK);
    for g in &matched[..limit] {
        lines.push(format!("- {} {}: {}", g.subject, g.predicate, g.object));
    }
    if matched.len() > MAX_RULES_PER_HOOK {
        lines.push(format!("  ({} more suppressed)", matched.len() - MAX_RULES_PER_HOOK));
    }
    lines.join("\n")
}

/// Simple shell tokenizer — splits on whitespace, respects quotes.
fn shell_tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if !in_single_quote => {
                escaped = true;
            }
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skip_tools() {
        assert!(should_skip_tool("Read"));
        assert!(should_skip_tool("Glob"));
        assert!(should_skip_tool("Agent"));
        assert!(should_skip_tool("ToolSearch"));
        assert!(!should_skip_tool("Bash"));
        assert!(!should_skip_tool("Edit"));
    }

    #[test]
    fn test_bash_term_extraction() {
        let input = serde_json::json!({"command": "uv run alembic revision -m 'add users'"});
        let terms = extract_bash_terms(&input);
        assert!(terms.contains(&"alembic".to_string()));
        assert!(terms.contains(&"revision".to_string()));
        assert!(terms.contains(&"uv".to_string())); // allowlisted short tool
        assert!(!terms.contains(&"run".to_string())); // noise word
    }

    #[test]
    fn test_bash_pipeline() {
        let input = serde_json::json!({"command": "git log | grep 'fix' && cargo test"});
        let terms = extract_bash_terms(&input);
        assert!(terms.contains(&"git".to_string()));
        assert!(terms.contains(&"grep".to_string()));
        assert!(terms.contains(&"cargo".to_string()));
    }

    #[test]
    fn test_file_term_extraction() {
        let input = serde_json::json!({"file_path": "migrations/versions/001_add_users.py"});
        let terms = extract_file_terms(&input);
        assert!(terms.contains(&"migrations".to_string()));
        assert!(terms.contains(&"versions".to_string()));
        assert!(terms.contains(&"001_add_users".to_string()));
    }

    #[test]
    fn test_grep_terms() {
        let input = serde_json::json!({"pattern": "import alembic", "path": "src/"});
        let terms = extract_grep_terms(&input);
        assert!(terms.contains(&"grep".to_string()));
        assert!(terms.contains(&"src".to_string()));
    }

    #[test]
    fn test_match_guardrails_with_intent() {
        use crate::store::Store;
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(100);

        let id = CTR.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("arai_guard_test_{}", id));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let store = Store::open(&db_path).unwrap();

        // Insert test triples
        let triples = vec![
            crate::parser::Triple {
                subject: "Alembic".to_string(),
                predicate: "forbids".to_string(),
                object: "hand-write migration files".to_string(),
                confidence: 0.92,
                domain: "test".to_string(),
                source_file: "CLAUDE.md".to_string(),
                line_start: Some(1),
                line_end: Some(1),
            },
            crate::parser::Triple {
                subject: "Git".to_string(),
                predicate: "never".to_string(),
                object: "force-push to main".to_string(),
                confidence: 0.92,
                domain: "test".to_string(),
                source_file: "CLAUDE.md".to_string(),
                line_start: Some(2),
                line_end: Some(2),
            },
        ];

        store.upsert_file("CLAUDE.md", "test content", &triples, "test").unwrap();
        store.classify_all_guardrails().unwrap();

        let guardrails = store.load_guardrails().unwrap();

        // Alembic "hand-write" rule: ToolCall timing, create scope
        let terms = vec!["alembic".to_string()];
        let matched = match_guardrails(&guardrails, &terms, "Write", "PreToolUse", &store);
        assert_eq!(matched.len(), 1, "hand-write rule should fire on Write/PreToolUse");

        let matched = match_guardrails(&guardrails, &terms, "Bash", "PreToolUse", &store);
        assert_eq!(matched.len(), 0, "hand-write rule should not fire on Bash");

        let matched = match_guardrails(&guardrails, &terms, "Edit", "PreToolUse", &store);
        assert_eq!(matched.len(), 0, "hand-write rule should not fire on Edit (allow_inverse)");

        // Git "force-push" rule: principle timing → doesn't fire on any hook
        // (principles are already in CLAUDE.md, Arai doesn't repeat them)
        let terms = vec!["git".to_string()];
        let matched = match_guardrails(&guardrails, &terms, "Bash", "PreToolUse", &store);
        assert_eq!(matched.len(), 0, "principle rule should not fire on PreToolUse");

        let matched = match_guardrails(&guardrails, &terms, "Bash", "UserPromptSubmit", &store);
        assert_eq!(matched.len(), 0, "principle rule should not fire on UserPromptSubmit either");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_short_tool_allowlist() {
        let input = serde_json::json!({"command": "go test ./..."});
        let terms = extract_bash_terms(&input);
        assert!(terms.contains(&"go".to_string()));
    }

    #[test]
    fn test_shell_tokenize_quotes() {
        let tokens = shell_tokenize("echo 'hello world' foo");
        assert_eq!(tokens, vec!["echo", "hello world", "foo"]);
    }
}
