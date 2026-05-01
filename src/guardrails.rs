use crate::parser::KNOWN_TOOLS;
use crate::store::Guardrail;
use aho_corasick::{AhoCorasick, MatchKind};
use serde_json::Value;
use std::sync::OnceLock;

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

/// Compiled Aho-Corasick automaton over `KNOWN_TOOLS`, ASCII-case-insensitive.
/// Built once per process (the hook is a fresh process per Claude Code tool
/// call, so "per process" is "per call" — the build cost is paid once instead
/// of every loop iteration).  `MatchKind::LeftmostLongest` ensures `pytest`
/// is preferred over `py` when both happen to be in the pattern set.
static KNOWN_TOOLS_AUTOMATON: OnceLock<AhoCorasick> = OnceLock::new();

fn known_tools_automaton() -> &'static AhoCorasick {
    KNOWN_TOOLS_AUTOMATON.get_or_init(|| {
        AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .match_kind(MatchKind::LeftmostLongest)
            .build(KNOWN_TOOLS)
            .expect("KNOWN_TOOLS is a const &[&str] of valid ASCII patterns")
    })
}

/// Scan text content for known tool/library names.  Single Aho-Corasick pass
/// over the original (non-lowercased) bytes — replaces the old O(N_tools ×
/// content_len) loop that lowercased the entire content first.  Word
/// boundaries are checked on the byte before/after each match using
/// `is_ascii_alphanumeric`; non-ASCII bytes count as boundaries (which is
/// what we want — `KNOWN_TOOLS` are all ASCII).
fn sniff_content_for_tools(content: &str, terms: &mut Vec<String>) {
    let bytes = content.as_bytes();
    let automaton = known_tools_automaton();
    // Keep "found once is enough" semantics — track which patterns have
    // already been pushed via a small bitset over pattern indices.
    let mut seen = vec![false; KNOWN_TOOLS.len()];
    for m in automaton.find_iter(content) {
        let idx = m.start();
        let after = m.end();
        let before_ok = idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric();
        let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            let pat = m.pattern().as_usize();
            if !seen[pat] {
                seen[pat] = true;
                terms.push(KNOWN_TOOLS[pat].to_string());
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
/// Results are ranked by relevance — rules whose object text overlaps with the
/// command terms are ranked higher.
pub fn match_guardrails(
    guardrails: &[Guardrail],
    terms: &[String],
    tool_name: &str,
    hook_event: &str,
) -> Vec<(Guardrail, u8)> {
    let mut matched: Vec<(Guardrail, usize)> = guardrails
        .iter()
        .filter(|g| {
            // Check timing — only fire rules meant for this hook event.  Intent
            // is already attached to the guardrail by `load_guardrails` (LEFT
            // JOIN), so no per-rule DB round trip here.
            if let Some(intent) = g.intent.as_ref() {
                // Rule must match the current hook event
                if intent.timing.hook_event() != hook_event {
                    return false;
                }

                // For tool-call-timed rules, also check subject and tool scope
                if intent.timing == crate::intent::Timing::ToolCall {
                    let subj = g.subject.to_lowercase();
                    let subject_matches = terms.iter().any(|t| subj.contains(t));
                    subject_matches && crate::intent::tool_matches_intent(intent, tool_name)
                } else {
                    true
                }
            } else {
                if hook_event != "PreToolUse" {
                    return false;
                }
                let subj = g.subject.to_lowercase();
                terms.iter().any(|t| subj.contains(t))
            }
        })
        .map(|g| {
            let score = relevance_score(&g.object, terms);
            (g.clone(), score)
        })
        .collect();

    // Sort by relevance score (highest first), then by confidence
    matched.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.confidence.partial_cmp(&a.0.confidence).unwrap_or(std::cmp::Ordering::Equal)));

    // If we have high-relevance matches, suppress low-relevance ones
    let top_score = matched.first().map(|(_, s)| *s).unwrap_or(0);
    if top_score > 1 {
        matched.retain(|(_, s)| *s >= top_score);
    }

    matched.into_iter().map(|(g, score)| {
        let pct = relevance_percentage(score, terms.len(), g.confidence);
        (g, pct)
    }).collect()
}

/// Convert raw relevance score into a confidence percentage (0-100).
/// Combines term overlap ratio with the base rule confidence.
fn relevance_percentage(score: usize, total_terms: usize, base_confidence: f64) -> u8 {
    if total_terms == 0 {
        return (base_confidence * 100.0) as u8;
    }
    let overlap_ratio = (score as f64) / (total_terms as f64).min(5.0);
    let combined = (overlap_ratio * 0.6 + base_confidence * 0.4) * 100.0;
    (combined.clamp(1.0, 100.0)) as u8
}

/// Score how relevant a rule's object text is to the extracted terms.
/// Higher score = more term overlap = more relevant.
/// Command verbs that are semantically distinct — if a rule mentions one,
/// the command should contain that same verb for a high relevance score.
const COMMAND_VERBS: &[&str] = &[
    "push", "pull", "commit", "merge", "rebase", "checkout", "clone", "fetch",
    "stash", "reset", "revert", "cherry", "bisect", "tag", "branch",
    "install", "uninstall", "build", "test", "run", "deploy", "publish",
    "start", "stop", "create", "delete", "remove", "add", "update", "upgrade",
];

fn relevance_score(object: &str, terms: &[String]) -> usize {
    let object_words: Vec<String> = object
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(String::from)
        .collect();

    let base_score: usize = terms.iter()
        .filter(|t| object_words.iter().any(|w| w == *t))
        .count();

    // Penalty: if the rule contains command verbs and NONE of them match the terms,
    // it's probably about a different action (e.g. rule says "push" but command is "pull")
    let rule_verbs: Vec<&String> = object_words.iter()
        .filter(|w| COMMAND_VERBS.contains(&w.as_str()))
        .collect();
    let has_verb_mismatch = !rule_verbs.is_empty()
        && !rule_verbs.iter().any(|v| terms.contains(v));

    if has_verb_mismatch && base_score > 0 {
        // Still matches by subject, but penalise — halve the score (minimum 1)
        base_score.div_ceil(2).max(1)
    } else {
        base_score
    }
}

/// Maximum number of rules to include in a single hook response.
const MAX_RULES_PER_HOOK: usize = 5;

/// Format matched guardrails as additionalContext string.
/// Limits output to the top N rules by confidence to avoid context bloat.
///
/// When `seen_set` contains a rule's `triple_id`, that rule was already
/// fully injected earlier in the same session and the model has its full
/// text in context.  We emit a compact one-liner for it instead — saves
/// roughly 50 tokens per re-fire and reduces attention dilution from
/// repeated re-reads of the same rule.  Pass `&Default::default()` if
/// seen-tracking is unavailable (e.g. unit tests, empty session_id).
pub fn format_context(
    matched: &[(Guardrail, u8)],
    seen_set: &std::collections::HashSet<i64>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Arai guardrails:".to_string());
    let limit = matched.len().min(MAX_RULES_PER_HOOK);
    for (g, pct) in &matched[..limit] {
        if seen_set.contains(&g.triple_id) {
            // Compact form: model already has the full text from earlier
            // in this session; just remind it the rule is still active.
            lines.push(format!(
                "- still: {} {} {} ({}% match)",
                g.subject, g.predicate, g.object, pct
            ));
        } else {
            let trace = format_trace(g);
            lines.push(format!(
                "- {} {}: {} ({}% match){}",
                g.subject, g.predicate, g.object, pct, trace
            ));
        }
    }
    if matched.len() > MAX_RULES_PER_HOOK {
        lines.push(format!("  ({} more suppressed)", matched.len() - MAX_RULES_PER_HOOK));
    }
    lines.join("\n")
}

/// Short "source:line layer-N" suffix appended to each rule line.  Empty when
/// neither field is populated (e.g. manually-added rules before the trace
/// existed).  Kept compact so the hook additionalContext stays readable.
fn format_trace(g: &Guardrail) -> String {
    let src = if !g.file_path.is_empty() {
        g.file_path.as_str()
    } else if !g.source_file.is_empty() {
        g.source_file.as_str()
    } else {
        ""
    };
    let mut bits = String::new();
    if !src.is_empty() {
        bits.push_str(src);
        if let Some(line) = g.line_start {
            bits.push(':');
            bits.push_str(&line.to_string());
        }
    }
    if let Some(layer) = g.layer {
        if !bits.is_empty() {
            bits.push(' ');
        }
        bits.push_str(&format!("layer-{layer}"));
    }
    if bits.is_empty() {
        String::new()
    } else {
        format!(" [{bits}]")
    }
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
    fn sniff_finds_tool_at_word_boundary_case_insensitively() {
        let mut terms = Vec::new();
        sniff_content_for_tools("Git pull origin main", &mut terms);
        assert!(terms.contains(&"git".to_string()), "case-insensitive prefix match: {terms:?}");
    }

    #[test]
    fn sniff_rejects_substring_inside_word() {
        // "github" contains "git" but is its own word — must NOT match `git`.
        let mut terms = Vec::new();
        sniff_content_for_tools("see github.com/foo", &mut terms);
        assert!(!terms.contains(&"git".to_string()), "substring leak: {terms:?}");
    }

    #[test]
    fn sniff_dedupes_repeat_occurrences() {
        let mut terms = Vec::new();
        sniff_content_for_tools("cargo build && cargo test && cargo run", &mut terms);
        assert_eq!(
            terms.iter().filter(|t| t.as_str() == "cargo").count(),
            1,
            "should push each tool once: {terms:?}"
        );
    }

    #[test]
    fn sniff_handles_non_ascii_content_without_panicking() {
        let mut terms = Vec::new();
        sniff_content_for_tools("café résumé — git checkout", &mut terms);
        // The ASCII bytes around `git` are spaces/punctuation; still matches.
        assert!(terms.contains(&"git".to_string()), "got: {terms:?}");
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
                layer: None,
                expires_at: None,
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
                layer: None,
                expires_at: None,
            },
        ];

        store.upsert_file("CLAUDE.md", "test content", &triples, "test").unwrap();
        store.classify_all_guardrails().unwrap();

        let guardrails = store.load_guardrails().unwrap();

        // Alembic "hand-write" rule: ToolCall timing, create scope
        let terms = vec!["alembic".to_string()];
        let matched = match_guardrails(&guardrails, &terms, "Write", "PreToolUse");
        assert_eq!(matched.len(), 1, "hand-write rule should fire on Write/PreToolUse");

        let matched = match_guardrails(&guardrails, &terms, "Bash", "PreToolUse");
        assert_eq!(matched.len(), 0, "hand-write rule should not fire on Bash");

        let matched = match_guardrails(&guardrails, &terms, "Edit", "PreToolUse");
        assert_eq!(matched.len(), 0, "hand-write rule should not fire on Edit (allow_inverse)");

        // Git "force-push" rule: principle timing → doesn't fire on any hook
        // (principles are already in CLAUDE.md, Arai doesn't repeat them)
        let terms = vec!["git".to_string()];
        let matched = match_guardrails(&guardrails, &terms, "Bash", "PreToolUse");
        assert_eq!(matched.len(), 0, "principle rule should not fire on PreToolUse");

        let matched = match_guardrails(&guardrails, &terms, "Bash", "UserPromptSubmit");
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

    #[test]
    fn test_relevance_score() {
        // "push" and "main" both overlap
        let terms = vec!["git".to_string(), "push".to_string(), "origin".to_string(), "main".to_string()];
        let score = relevance_score("git push to main without a PR", &terms);
        assert!(score >= 3, "should match git, push, main — got {score}");

        // Only "git" overlaps (commit ≠ push, verb mismatch penalty applies)
        let score = relevance_score("git commit with signed commits", &terms);
        assert!(score <= 2, "should be penalised for verb mismatch — got {score}");

        // Push rule should score higher than commit rule for "git push origin main"
        let push_score = relevance_score("git push to main without a PR", &terms);
        let commit_score = relevance_score("git commit with signed commits", &terms);
        assert!(push_score > commit_score, "push rule ({push_score}) should rank higher than commit rule ({commit_score})");
    }

    #[test]
    fn test_relevance_verb_mismatch() {
        // Rule says "push" but command is "pull" — should be penalised
        let pull_terms = vec!["git".to_string(), "pull".to_string(), "origin".to_string(), "main".to_string()];
        let push_rule_score = relevance_score("git push to main without a PR", &pull_terms);
        let commit_rule_score = relevance_score("git commit with signed commits", &pull_terms);
        assert!(commit_rule_score >= push_rule_score,
            "commit ({commit_rule_score}) should rank >= push ({push_rule_score}) for a pull command");

        // Rule says "install" but command is "uninstall" — different verbs
        let terms = vec!["npm".to_string(), "uninstall".to_string(), "package".to_string()];
        let score = relevance_score("npm install with --save-exact", &terms);
        assert!(score <= 1, "install rule should be penalised for uninstall command — got {score}");
    }

    #[test]
    fn test_relevance_no_penalty_when_verb_matches() {
        // Exact verb match — no penalty
        let terms = vec!["cargo".to_string(), "test".to_string()];
        let score = relevance_score("run cargo test before pushing", &terms);
        assert!(score >= 2, "exact verb match should score high — got {score}");
    }

    #[test]
    fn test_relevance_no_verb_in_rule() {
        // Rule has no command verb — pure term overlap, no penalty
        let terms = vec!["git".to_string(), "push".to_string(), "main".to_string()];
        let score = relevance_score("always use feature branches for main", &terms);
        assert!(score >= 1, "should match 'main' without penalty — got {score}");
    }

    #[test]
    fn test_relevance_zero_overlap() {
        // No terms match at all
        let terms = vec!["docker".to_string(), "compose".to_string(), "up".to_string()];
        let score = relevance_score("git push to main without a PR", &terms);
        assert_eq!(score, 0, "no overlap should score 0 — got {score}");
    }

    #[test]
    fn test_relevance_percentage() {
        // High overlap: 3 of 4 terms match → high percentage
        let pct = relevance_percentage(3, 4, 0.92);
        assert!(pct >= 70, "3/4 overlap + 0.92 confidence should be >= 70% — got {pct}%");

        // Low overlap: 1 of 4 terms match → lower percentage
        let pct = relevance_percentage(1, 4, 0.92);
        assert!(pct < 60, "1/4 overlap should be < 60% — got {pct}%");

        // Zero terms → falls back to base confidence
        let pct = relevance_percentage(0, 0, 0.90);
        assert_eq!(pct, 90, "zero terms should use base confidence — got {pct}%");

        // High overlap should always beat low overlap at same confidence
        let high = relevance_percentage(3, 4, 0.90);
        let low = relevance_percentage(1, 4, 0.90);
        assert!(high > low, "high overlap ({high}%) should beat low overlap ({low}%)");
    }

    #[test]
    fn test_relevance_ranking_with_db() {
        use crate::store::Store;
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR2: AtomicU64 = AtomicU64::new(200);

        let id = CTR2.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("arai_rank_test_{}", id));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let store = Store::open(&db_path).unwrap();

        let triples = vec![
            crate::parser::Triple {
                subject: "Git".to_string(),
                predicate: "always".to_string(),
                object: "git commit with signed commits".to_string(),
                confidence: 0.95,
                domain: "test".to_string(),
                source_file: "test".to_string(),
                line_start: Some(1),
                line_end: Some(1),
                layer: None,
                expires_at: None,
            },
            crate::parser::Triple {
                subject: "Git".to_string(),
                predicate: "never".to_string(),
                object: "git push to main without a PR".to_string(),
                confidence: 0.95,
                domain: "test".to_string(),
                source_file: "test".to_string(),
                line_start: Some(2),
                line_end: Some(2),
                layer: None,
                expires_at: None,
            },
        ];

        store.upsert_file("test", "test", &triples, "test").unwrap();
        store.classify_all_guardrails().unwrap();

        let guardrails = store.load_guardrails().unwrap();

        // "git push origin main" should ONLY fire the push rule
        let terms = vec!["git".to_string(), "push".to_string(), "origin".to_string(), "main".to_string()];
        let matched = match_guardrails(&guardrails, &terms, "Bash", "PreToolUse");
        assert_eq!(matched.len(), 1, "only the relevant rule should fire");
        assert!(matched[0].0.object.contains("push"), "push rule should fire, got: {}", matched[0].0.object);

        // "git commit -m test" should ONLY fire the commit rule
        let terms = vec!["git".to_string(), "commit".to_string(), "test".to_string()];
        let matched = match_guardrails(&guardrails, &terms, "Bash", "PreToolUse");
        assert_eq!(matched.len(), 1, "only the relevant rule should fire");
        assert!(matched[0].0.object.contains("commit"), "commit rule should fire, got: {}", matched[0].0.object);

        // "git pull origin main" — push rule should be suppressed (pull ≠ push)
        let terms = vec!["git".to_string(), "pull".to_string(), "origin".to_string(), "main".to_string()];
        let matched = match_guardrails(&guardrails, &terms, "Bash", "PreToolUse");
        assert!(matched.is_empty() || matched[0].0.object.contains("commit"),
            "push rule should not fire for pull command");

        std::fs::remove_dir_all(&dir).ok();
    }

    fn mk_guardrail(triple_id: i64, subject: &str, predicate: &str, object: &str) -> Guardrail {
        Guardrail {
            triple_id,
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            confidence: 0.9,
            source_file: "CLAUDE.md".to_string(),
            file_path: "CLAUDE.md".to_string(),
            layer: Some(1),
            line_start: Some(42),
            expires_at: None,
            intent: None,
        }
    }

    #[test]
    fn format_context_emits_full_form_for_unseen_rules() {
        let matched = vec![(mk_guardrail(1, "git", "never", "force-push to main"), 95u8)];
        let seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let ctx = format_context(&matched, &seen);
        // Full form carries the source trace.
        assert!(ctx.contains("CLAUDE.md:42"), "full form should cite source: {ctx:?}");
        assert!(ctx.contains("layer-1"), "full form should cite layer: {ctx:?}");
        assert!(!ctx.contains("still:"), "first injection should NOT use compact prefix");
    }

    #[test]
    fn format_context_emits_compact_form_for_seen_rules() {
        let matched = vec![(mk_guardrail(7, "git", "never", "force-push to main"), 95u8)];
        let seen: std::collections::HashSet<i64> = [7i64].iter().copied().collect();
        let ctx = format_context(&matched, &seen);
        // Compact form drops source/layer trace; uses "still:" prefix.
        assert!(ctx.contains("still:"), "repeat injection should use compact prefix");
        assert!(!ctx.contains("CLAUDE.md:42"), "compact form should NOT re-cite source");
        assert!(!ctx.contains("layer-1"), "compact form should NOT re-cite layer");
        // Compact form is shorter — the whole point.
        let unseen_ctx = format_context(&matched, &std::collections::HashSet::new());
        assert!(
            ctx.len() < unseen_ctx.len(),
            "compact form should be shorter than full form ({} vs {})",
            ctx.len(),
            unseen_ctx.len(),
        );
    }

    #[test]
    fn format_context_mixes_full_and_compact_per_rule() {
        // Two rules, only one already seen — output should have one full
        // line and one compact line.
        let matched = vec![
            (mk_guardrail(1, "alembic", "must_not", "hand-write migrations"), 90u8),
            (mk_guardrail(2, "git", "never", "force-push to main"), 95u8),
        ];
        let seen: std::collections::HashSet<i64> = [1i64].iter().copied().collect();
        let ctx = format_context(&matched, &seen);
        // alembic is seen → compact; git is fresh → full.
        assert!(ctx.contains("- still: alembic must_not hand-write migrations"));
        assert!(ctx.contains("git never: force-push to main"));
        assert!(ctx.contains("[CLAUDE.md:42 layer-1]"));
    }
}
