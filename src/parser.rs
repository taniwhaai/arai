use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Triple {
    #[serde(rename = "s")]
    pub subject: String,
    #[serde(rename = "p")]
    pub predicate: String,
    #[serde(rename = "o")]
    pub object: String,
    pub confidence: f64,
    pub domain: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<i64>,
}

/// Known tool names for subject extraction, "use X" two-signal gate, and content sniffing.
pub const KNOWN_TOOLS: &[&str] = &[
    "alembic", "cargo", "npm", "yarn", "pnpm", "pip", "uv", "poetry",
    "docker", "git", "gh", "pytest", "jest", "vitest", "mocha", "rspec",
    "go", "rustc", "gcc", "make", "cmake", "webpack", "vite", "eslint",
    "prettier", "black", "ruff", "mypy", "pyright", "tsc", "terraform",
    "ansible", "kubectl", "helm", "gradle", "maven", "sbt", "mix",
    "bundle", "composer", "apt", "brew", "dnf", "yum", "pacman", "snap",
    "flatpak", "nix", "tmux", "sed", "awk", "grep", "curl", "wget",
    "ruby", "python", "node", "deno", "bun", "java", "dotnet", "swift",
    "clang", "bazel", "nx", "turbo", "pnpm", "pipenv", "conda",
];

/// Extract guardrail rules from markdown content.
pub fn extract_rules(content: &str, source_type: &str, base_confidence: f64) -> Vec<Triple> {
    let (_frontmatter, body) = crate::discovery::parse_frontmatter(content);
    let items = extract_list_items(&body);
    let mut triples = Vec::new();

    for item in items {
        if let Some(triple) = match_imperative(&item.text, &item.section_context) {
            triples.push(Triple {
                subject: triple.0,
                predicate: triple.1,
                object: triple.2,
                confidence: base_confidence,
                domain: format!("memory.{source_type}"),
                source_file: String::new(), // filled by caller
                line_start: Some(item.line_start as i64),
                line_end: Some(item.line_end as i64),
            });
        }
    }

    triples
}

#[derive(Debug)]
struct ListItem {
    text: String,
    line_start: usize,
    line_end: usize,
    section_context: Option<String>,
}

fn extract_list_items(body: &str) -> Vec<ListItem> {
    let mut items = Vec::new();
    let mut current_item: Option<(String, usize)> = None;
    let mut current_section: Option<String> = None;
    let mut in_code_block = false;
    let mut in_table = false;

    for (line_idx, line) in body.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();

        // Track code blocks
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        // Track table rows
        if trimmed.starts_with('|') {
            in_table = true;
            continue;
        } else if in_table && !trimmed.is_empty() {
            in_table = false;
        }

        // Track section headers
        if trimmed.starts_with('#') {
            let header = trimmed.trim_start_matches('#').trim().to_string();
            // Strip leading numbered prefix like "4. " and trailing formatting like " (§3)"
            let header = strip_list_prefix(&header);
            let header = header.split('(').next().unwrap_or(&header).trim().to_string();
            current_section = if header.is_empty() { None } else { Some(header) };
            // Flush any pending item
            if let Some((text, start)) = current_item.take() {
                items.push(ListItem {
                    text,
                    line_start: start,
                    line_end: line_num - 1,
                    section_context: current_section.clone(),
                });
            }
            continue;
        }

        // Check for list item start
        let is_list_item = is_list_start(trimmed);

        if is_list_item {
            // Flush previous item
            if let Some((text, start)) = current_item.take() {
                items.push(ListItem {
                    text,
                    line_start: start,
                    line_end: line_num - 1,
                    section_context: current_section.clone(),
                });
            }
            let item_text = strip_list_prefix(trimmed);
            current_item = Some((item_text, line_num));
        } else if !trimmed.is_empty() && current_item.is_some() {
            // Continuation line — check if indented
            if line.starts_with("  ") || line.starts_with('\t') {
                if let Some((ref mut text, _)) = current_item {
                    text.push(' ');
                    text.push_str(trimmed);
                }
            } else {
                // Not indented, flush
                if let Some((text, start)) = current_item.take() {
                    items.push(ListItem {
                        text,
                        line_start: start,
                        line_end: line_num - 1,
                        section_context: current_section.clone(),
                    });
                }
            }
        } else if trimmed.is_empty() {
            // Blank line flushes
            if let Some((text, start)) = current_item.take() {
                items.push(ListItem {
                    text,
                    line_start: start,
                    line_end: line_num - 1,
                    section_context: current_section.clone(),
                });
            }
        }
    }

    // Flush last item
    if let Some((text, start)) = current_item.take() {
        let line_end = body.lines().count();
        items.push(ListItem {
            text,
            line_start: start,
            line_end,
            section_context: current_section,
        });
    }

    items
}

fn is_list_start(trimmed: &str) -> bool {
    if trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
    {
        return true;
    }
    // Numbered list: "1. ", "2. ", etc.
    let re = Regex::new(r"^\d+\.\s").unwrap();
    re.is_match(trimmed)
}

fn strip_list_prefix(trimmed: &str) -> String {
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return trimmed[2..].to_string();
    }
    let re = Regex::new(r"^\d+\.\s+").unwrap();
    re.replace(trimmed, "").to_string()
}

/// Strip markdown formatting: bold, italic, backticks, links.
fn strip_markdown(text: &str) -> String {
    let s = text.to_string();
    // Strip bold **text** and __text__
    let re_bold = Regex::new(r"\*\*(.+?)\*\*|__(.+?)__").unwrap();
    let s = re_bold.replace_all(&s, "$1$2").to_string();
    // Strip inline code `text`
    let re_code = Regex::new(r"`([^`]+)`").unwrap();
    let s = re_code.replace_all(&s, "$1").to_string();
    // Strip markdown links [text](url) → text
    let re_link = Regex::new(r"\[([^]]+)]\([^)]+\)").unwrap();
    let s = re_link.replace_all(&s, "$1").to_string();
    // Strip em-dash separators for cleaner matching
    s.replace(" — ", " - ")
}

/// Match a list item against imperative patterns.
/// Returns (subject, predicate, object) or None.
fn match_imperative(text: &str, section_context: &Option<String>) -> Option<(String, String, String)> {
    // Skip items that look like pure code references
    if is_code_reference(text) {
        return None;
    }

    // Strip markdown formatting before matching
    let cleaned = strip_markdown(text);
    let lower = cleaned.to_lowercase();

    // Layer 1: Start-of-sentence imperative patterns (highest confidence)
    let start_patterns: Vec<(&str, &str)> = vec![
        (r"(?i)^never\s+(.+)", "never"),
        (r"(?i)^always\s+(.+)", "always"),
        (r"(?i)^don'?t\s+(.+)", "forbids"),
        (r"(?i)^do\s+not\s+(.+)", "forbids"),
        (r"(?i)^must\s+not\s+(.+)", "must_not"),
        (r"(?i)^must\s+(.+)", "requires"),
        (r"(?i)^avoid\s+(.+)", "forbids"),
        (r"(?i)^ensure\s+(.+)", "enforces"),
        (r"(?i)^requires?\s+(.+)", "requires"),
        (r"(?i)^prefer\s+(.+?)(?:\s+over\s+.+)?$", "prefers"),
        (r"(?i)^stop\s+(.+)", "forbids"),
        (r"(?i)^skip\s+(.+)", "forbids"),
        (r"(?i)^only\s+(.+)", "requires"),
    ];

    for (pattern, predicate) in &start_patterns {
        let re = Regex::new(pattern).ok()?;
        if let Some(caps) = re.captures(&cleaned) {
            if let Some(obj_match) = caps.get(1) {
                let object = obj_match.as_str().trim().to_string();
                let subject = extract_subject(&lower, section_context);
                return Some((subject, predicate.to_string(), clean_object(&object)));
            }
        }
    }

    // Layer 2: "X is forbidden/required" patterns
    let passive_patterns: Vec<(&str, &str)> = vec![
        (r"(?i)(.+?)\s+is\s+forbidden", "forbids"),
        (r"(?i)(.+?)\s+is\s+required", "requires"),
        (r"(?i)(.+?)\s+is\s+not\s+allowed", "forbids"),
    ];

    for (pattern, predicate) in &passive_patterns {
        let re = Regex::new(pattern).ok()?;
        if let Some(caps) = re.captures(&cleaned) {
            if let Some(obj_match) = caps.get(1) {
                let object = obj_match.as_str().trim().to_string();
                let subject = extract_subject(&lower, section_context);
                return Some((subject, predicate.to_string(), clean_object(&object)));
            }
        }
    }

    // Layer 3: Colon-separated rules — "Label: imperative instruction"
    // e.g. "Simplicity First: Make every change as simple as possible"
    if let Some(colon_pos) = cleaned.find(':') {
        let after_colon = cleaned[colon_pos + 1..].trim();
        if !after_colon.is_empty() {
            let after_lower = after_colon.to_lowercase();
            // Check if the part after colon starts with an imperative
            let colon_imperatives = [
                "never", "always", "don't", "do not", "must", "avoid", "ensure",
                "make", "find", "keep", "use", "run", "check", "add", "write",
                "update", "mark", "set", "get", "put", "send", "stop", "start",
                "enter", "exit", "verify", "test", "review", "demonstrate",
                "offload", "throw", "challenge", "pause", "ask", "diff",
                "ruthlessly", "zero", "no ", "only",
            ];
            if colon_imperatives.iter().any(|imp| after_lower.starts_with(imp)) {
                let label = cleaned[..colon_pos].trim();
                let subject = if !label.is_empty() {
                    // Use the label as subject if it's meaningful
                    let label_lower = label.to_lowercase();
                    if let Some(tool) = KNOWN_TOOLS.iter().find(|t| contains_word(&label_lower, t)) {
                        capitalize(tool)
                    } else {
                        label.to_string()
                    }
                } else {
                    extract_subject(&after_lower, section_context)
                };
                return Some((subject, "requires".to_string(), clean_object(after_colon)));
            }
        }
    }

    // Layer 4: Mid-sentence imperatives — "If X, STOP/never/don't Y"
    let mid_patterns: Vec<(&str, &str)> = vec![
        (r"(?i),\s*never\s+(.+)", "never"),
        (r"(?i),\s*always\s+(.+)", "always"),
        (r"(?i),\s*don'?t\s+(.+)", "forbids"),
        (r"(?i),\s*do\s+not\s+(.+)", "forbids"),
        (r"(?i),\s*stop\s+(.+)", "forbids"),
        (r"(?i)--\s*don'?t\s+(.+)", "forbids"),
        (r"(?i)—\s*don'?t\s+(.+)", "forbids"),
    ];

    for (pattern, predicate) in &mid_patterns {
        let re = Regex::new(pattern).ok()?;
        if let Some(caps) = re.captures(&cleaned) {
            if let Some(obj_match) = caps.get(1) {
                let object = obj_match.as_str().trim().to_string();
                let subject = extract_subject(&lower, section_context);
                return Some((subject, predicate.to_string(), clean_object(&object)));
            }
        }
    }

    // Layer 5: "use X" with two-signal gate
    let use_re = Regex::new(r"(?i)^use\s+(.+)").ok()?;
    if let Some(caps) = use_re.captures(&cleaned) {
        let object = caps.get(1)?.as_str().trim();
        let has_known_tool = contains_known_tool(&lower);
        let has_co_imperative = lower.contains("always") || lower.contains("must") || lower.contains("never");

        if has_known_tool || has_co_imperative {
            let subject = extract_subject(&lower, section_context);
            return Some((subject, "requires".to_string(), clean_object(object)));
        }
    }

    // Layer 6: Catch-all — any list item with a verb-like start is a candidate rule
    // These get lower confidence (handled by the caller via source_type)
    let verb_starts = [
        "enter", "exit", "run", "check", "test", "write", "read", "review",
        "update", "delete", "add", "remove", "set", "get", "keep", "find",
        "fix", "verify", "demonstrate", "prove", "show", "deploy", "build",
        "install", "configure", "enable", "disable", "start", "stop",
        "offload", "throw", "challenge", "pause", "diff", "ask",
        "point",
    ];

    let first_word = lower.split_whitespace().next().unwrap_or("");
    if verb_starts.contains(&first_word) {
        let subject = extract_subject(&lower, section_context);
        return Some((subject, "requires".to_string(), clean_object(&cleaned)));
    }

    None
}

fn is_code_reference(text: &str) -> bool {
    let trimmed = text.trim();
    // Pure backtick-wrapped path
    if trimmed.starts_with('`') && trimmed.ends_with('`') && !trimmed.contains(' ') {
        return true;
    }
    // file:line reference
    let re = Regex::new(r"^[\w./\\-]+:\d+$").unwrap();
    if re.is_match(trimmed) {
        return true;
    }
    false
}

fn extract_subject(lower_text: &str, section_context: &Option<String>) -> String {
    // First: find the earliest known tool name in the text (closest to the verb)
    if let Some(tool) = find_earliest_tool(lower_text) {
        return capitalize(tool);
    }

    // Second: inherit from section header
    if let Some(section) = section_context {
        let section_lower = section.to_lowercase();
        for tool in KNOWN_TOOLS {
            if section_lower.contains(tool) {
                return capitalize(tool);
            }
        }
        // Use section header as subject
        return section.clone();
    }

    // Third: extract first significant noun phrase after the verb
    extract_first_noun(lower_text)
}

fn extract_first_noun(text: &str) -> String {
    let skip_words = [
        "the", "a", "an", "all", "any", "every", "each", "this", "that",
        "your", "my", "our", "their", "its", "for", "to", "in", "on",
        "with", "from", "of", "and", "or", "not", "no", "is", "are",
        "was", "were", "be", "been", "being",
    ];

    let words: Vec<&str> = text.split_whitespace().collect();

    // Skip the first word (it's the verb/imperative)
    for word in words.iter().skip(1) {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.len() >= 3 && !skip_words.contains(&clean) {
            return capitalize(clean);
        }
    }

    "General".to_string()
}

/// Find the earliest known tool name in text (by position).
/// Returns the tool closest to the start — typically the primary subject.
/// Ambiguous tool names that need context to confirm they mean the tool.
/// "go" can mean "go ahead" or the Go language — only match if followed by
/// a subcommand like "test", "build", "run", "fmt", "mod", "get".
const AMBIGUOUS_TOOLS: &[(&str, &[&str])] = &[
    ("go", &["test", "build", "run", "fmt", "mod", "get", "install", "vet", "generate"]),
];

fn find_earliest_tool(text: &str) -> Option<&'static str> {
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    for (i, word) in words.iter().enumerate() {
        for tool in KNOWN_TOOLS {
            if *word == *tool {
                // Check if this is an ambiguous tool
                if let Some((_, subcommands)) = AMBIGUOUS_TOOLS.iter().find(|(t, _)| t == tool) {
                    // Only match if followed by a known subcommand
                    let next_word = words.get(i + 1).unwrap_or(&"");
                    if subcommands.contains(next_word) {
                        return Some(tool);
                    }
                    // Otherwise skip — "go fix" means "go ahead and fix"
                    continue;
                }
                return Some(tool);
            }
        }
    }
    None
}

/// Check if text contains a word as a whole word (not substring).
fn contains_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|w| w == word)
}

fn contains_known_tool(lower_text: &str) -> bool {
    KNOWN_TOOLS.iter().any(|tool| contains_word(lower_text, tool))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

fn clean_object(s: &str) -> String {
    // Remove trailing punctuation and clean up
    let s = s.trim_end_matches(['.', ',', ';']);
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_never_pattern() {
        let rules = extract_rules("- Never hand-write migration files", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "never");
        assert!(rules[0].object.contains("hand-write migration files"));
    }

    #[test]
    fn test_always_pattern() {
        let rules = extract_rules("- Always use autogenerate for alembic", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "always");
        assert_eq!(rules[0].subject, "Alembic");
    }

    #[test]
    fn test_dont_pattern() {
        let rules = extract_rules("- Don't mock the database in tests", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "forbids");
    }

    #[test]
    fn test_must_pattern() {
        let rules = extract_rules("- Must run tests before merging", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "requires");
    }

    #[test]
    fn test_must_not_pattern() {
        let rules = extract_rules("- Must not skip pre-commit hooks", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "must_not");
    }

    #[test]
    fn test_avoid_pattern() {
        let rules = extract_rules("- Avoid using --no-verify", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "forbids");
    }

    #[test]
    fn test_prefer_pattern() {
        let rules = extract_rules("- Prefer cargo test over manual testing", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "prefers");
        assert_eq!(rules[0].subject, "Cargo");
    }

    #[test]
    fn test_is_forbidden_pattern() {
        let rules = extract_rules("- Force-push to main is forbidden", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "forbids");
    }

    #[test]
    fn test_use_with_known_tool() {
        let rules = extract_rules("- Use uv run for all Python commands", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "requires");
        assert_eq!(rules[0].subject, "Uv");
    }

    #[test]
    fn test_use_without_signal_skipped() {
        let rules = extract_rules("- Use the Read tool to read files", "test", 0.9);
        assert_eq!(rules.len(), 0);
    }

    #[test]
    fn test_section_context_inheritance() {
        let content = "## Alembic\n\n- Never hand-write migration files\n- Always use autogenerate";
        let rules = extract_rules(content, "test", 0.9);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].subject, "Alembic");
        assert_eq!(rules[1].subject, "Alembic");
    }

    #[test]
    fn test_code_block_skipped() {
        let content = "```\n- Never do this\n```\n\n- Always do that";
        let rules = extract_rules(content, "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "always");
    }

    #[test]
    fn test_multiple_rules() {
        let content = "\
- Never force-push to main
- Always run cargo test before merging
- Don't skip code review
- Must use feature branches";
        let rules = extract_rules(content, "test", 0.9);
        assert_eq!(rules.len(), 4);
    }
}
