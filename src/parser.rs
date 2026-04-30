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
    /// Which of the seven `match_imperative` layers fired.  Lets `arai audit` /
    /// `arai why` surface "fired from CLAUDE.md:42 (layer-1 imperative)" so a
    /// user can trace *why* a rule exists without spelunking the parser.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub layer: Option<u8>,
    /// Optional expiry date (ISO `YYYY-MM-DD`).  Extracted from a trailing
    /// `(expires YYYY-MM-DD)` or `(until YYYY-MM-DD)` annotation in the rule
    /// text.  Expired rules are filtered out by `load_guardrails`, so a
    /// rule written "never skip tests (expires 2026-06-01)" self-prunes
    /// after that date without any manual cleanup.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expires_at: Option<String>,
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
        if let Some((subject, predicate, object, layer)) =
            match_imperative(&item.text, &item.section_context)
        {
            let (object, expires_at) = extract_expiry(&object);
            triples.push(Triple {
                subject,
                predicate,
                object,
                confidence: base_confidence,
                domain: format!("memory.{source_type}"),
                source_file: String::new(), // filled by caller
                line_start: Some(item.line_start as i64),
                line_end: Some(item.line_end as i64),
                layer: Some(layer),
                expires_at,
            });
        }
    }

    triples
}

/// Strip a trailing `(expires YYYY-MM-DD)` or `(until YYYY-MM-DD)` annotation
/// from the rule object and return the cleaned object plus the parsed date.
/// Accepts case-insensitive `expires` / `expire` / `until`.  If nothing is
/// found, the object is returned unchanged with `None`.
pub fn extract_expiry(object: &str) -> (String, Option<String>) {
    let re = match Regex::new(r"(?i)\s*\((?:expires?|until)\s+(\d{4}-\d{2}-\d{2})\)\s*$") {
        Ok(r) => r,
        Err(_) => return (object.to_string(), None),
    };
    if let Some(caps) = re.captures(object) {
        let date = caps.get(1).map(|m| m.as_str().to_string());
        let cleaned = re.replace(object, "").trim().to_string();
        (cleaned, date)
    } else {
        (object.to_string(), None)
    }
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
/// Returns (subject, predicate, object, layer) or None.  `layer` is the 1-based
/// index of the pattern family that fired, surfaced in the audit log + hook
/// output so a reviewer can trace the derivation without reading this file.
fn match_imperative(text: &str, section_context: &Option<String>) -> Option<(String, String, String, u8)> {
    // Skip items that look like pure code references
    if is_code_reference(text) {
        return None;
    }

    // Bold-label discriminator — `**No build process** - this is a zero-
    // build extension.` and `**Consider constraints:** What are the goals?`
    // are feature-absence DESCRIPTIONS or section headings, not rules.
    //
    // Heuristic: a *multi-word* bold prefix is a label; a *single-word*
    // bold prefix is emphasis on a leader word.  This handles three
    // shapes the corpus actually contains:
    //   `**No build process** - this is a zero-build extension.` (label)
    //   `**Consider constraints:** What are the goals?`           (label)
    //   `**Always** run tests before push`                         (emphasis — keep)
    // Catches the colon-inside-bold case the previous trailing-separator
    // regex missed.
    let bold_label_re = Regex::new(r"^\*\*[^*]+\s[^*]*\*\*").ok()?;
    let is_bold_label = bold_label_re.is_match(text.trim_start());

    // Strip markdown formatting before matching
    let cleaned = strip_markdown(text);
    let lower = cleaned.to_lowercase();

    // Layer 1: Start-of-sentence imperative patterns (highest confidence).
    //
    // Order matters within the list — more specific patterns precede
    // generic ones so e.g. `^should not` matches before `^should`.  The
    // predicate column maps to severity via `intent::Severity::from_predicate`:
    //   - never / forbids / must_not       → Block
    //   - always / requires / enforces     → Warn
    //   - prefers / learned_from           → Inform
    //
    // Severity rationale for the v0.2.11 additions:
    //   should not / shouldn't / cannot    → must_not (Block) — explicit
    //                                        prohibition by the writer
    //   refuse to                          → forbids  (Block) — same
    //   should                             → prefers  (Inform) — softer
    //                                        than must/always; honour that
    //   make sure / be sure                → enforces (Warn)  — synonym of
    //                                        ensure
    //   consider / recommend               → prefers  (Inform) — soft
    //                                        preference
    //   enforce                            → enforces (Warn)  — verb form
    //                                        of the predicate
    let start_patterns: Vec<(&str, &str)> = vec![
        (r"(?i)^never\s+(.+)", "never"),
        (r"(?i)^always\s+(.+)", "always"),
        (r"(?i)^don'?t\s+(.+)", "forbids"),
        (r"(?i)^do\s+not\s+(.+)", "forbids"),
        (r"(?i)^must\s+not\s+(.+)", "must_not"),
        (r"(?i)^must\s+(.+)", "requires"),
        (r"(?i)^should\s+not\s+(.+)", "must_not"),
        (r"(?i)^shouldn'?t\s+(.+)", "must_not"),
        (r"(?i)^should\s+(.+)", "prefers"),
        (r"(?i)^cannot\s+(.+)", "must_not"),
        (r"(?i)^refuse\s+to\s+(.+)", "forbids"),
        (r"(?i)^avoid\s+(.+)", "forbids"),
        (r"(?i)^ensure\s+(.+)", "enforces"),
        (r"(?i)^enforce\s+(.+)", "enforces"),
        (r"(?i)^make\s+sure\s+(?:that\s+)?(.+)", "enforces"),
        (r"(?i)^be\s+sure\s+(?:to\s+)?(.+)", "enforces"),
        (r"(?i)^requires?\s+(.+)", "requires"),
        (r"(?i)^prefer\s+(.+?)(?:\s+over\s+.+)?$", "prefers"),
        (r"(?i)^consider\s+(.+)", "prefers"),
        (r"(?i)^recommend(?:ed)?\s+(.+)", "prefers"),
        (r"(?i)^stop\s+(.+)", "forbids"),
        (r"(?i)^skip\s+(.+)", "forbids"),
        (r"(?i)^only\s+(.+)", "requires"),
    ];

    for (pattern, predicate) in &start_patterns {
        let re = Regex::new(pattern).ok()?;
        if let Some(caps) = re.captures(&cleaned) {
            // Skip `consider` when the bullet is a labelled description
            // (e.g. `**Consider constraints:** What are the goals?` is a
            // section heading inside a list, not a soft preference rule).
            if is_bold_label && pattern.contains("consider") {
                continue;
            }
            if let Some(obj_match) = caps.get(1) {
                let object = obj_match.as_str().trim().to_string();
                let subject = extract_subject(&lower, section_context);
                return Some((subject, predicate.to_string(), clean_object(&object), 1));
            }
        }
    }

    // Layer 1b: bare `^no <noun>` prohibitions — "No AI attribution in commit
    // messages" / "No emojis in commit messages".  Gated against bold-label
    // form (`**No build process** - this is a zero-build extension.`) which
    // is feature-absence description, not a rule.
    if !is_bold_label {
        let no_re = Regex::new(r"(?i)^no\s+(.+)").ok()?;
        if let Some(caps) = no_re.captures(&cleaned) {
            if let Some(obj_match) = caps.get(1) {
                let object = obj_match.as_str().trim().to_string();
                let subject = extract_subject(&lower, section_context);
                return Some((subject, "must_not".to_string(), clean_object(&object), 1));
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
                return Some((subject, predicate.to_string(), clean_object(&object), 2));
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
                return Some((subject, "requires".to_string(), clean_object(after_colon), 3));
            }
        }
    }

    // Layer 7: Conditional imperatives — evaluated *before* Layer 4 because
    // a conditional shape is more specific than a generic mid-sentence
    // imperative ("When deploying to production, never skip smoke tests"
    // wants the conditional reading, not just the comma-imperative).
    // Layer 4 still fires for non-conditional comma-imperatives that don't
    // start with a trigger word.
    //
    // Shape: ^(Before|After|When|Whenever|If|For)\s+<condition>(,|:|→|—)\s+<verb>\s+<rest>
    // The verb must be in the union of recognised imperatives so we don't
    // accidentally extract a rule from "When X, see Y" prose continuations.
    let conditional_re = Regex::new(
        r"(?ix)
        ^\s*(?:before|after|when|whenever|if|for)\s+   # trigger word
        (.+?)                                          # condition phrase
        \s*[,:\u{2192}\u{2014}]\s+                     # `,`, `:`, `→`, or `—`
        (\w+)                                          # imperative verb
        \s+(.+)                                        # rule body
        ",
    )
    .ok()?;
    if let Some(caps) = conditional_re.captures(&cleaned) {
        let verb = caps.get(2)?.as_str().to_lowercase();
        let body = caps.get(3)?.as_str().trim();
        let condition = caps.get(1)?.as_str().trim().to_lowercase();
        let allowed_verbs = [
            // From Layer 6
            "enter", "exit", "run", "check", "test", "write", "read", "review",
            "update", "delete", "add", "remove", "set", "get", "keep", "find",
            "fix", "verify", "demonstrate", "prove", "show", "deploy", "build",
            "install", "configure", "enable", "disable", "start", "stop",
            "offload", "throw", "challenge", "pause", "diff", "ask",
            "point", "create", "implement", "document", "define", "store",
            "state", "share", "explain", "describe", "connect", "apply",
            // Layer 1 leaders
            "never", "always", "don't", "dont", "do", "must", "should",
            "shouldn't", "shouldnt", "avoid", "ensure", "use", "prefer",
            "consider", "recommend", "make", "be", "no", "only",
        ];
        if allowed_verbs.contains(&verb.as_str()) {
            // Decide predicate based on the inner verb.  Bias toward
            // `requires` because most conditionals are positive imperatives;
            // map clear prohibitives explicitly.
            let predicate = match verb.as_str() {
                "never" | "don't" | "dont" | "avoid" | "stop" => "never",
                "shouldn't" | "shouldnt" => "must_not",
                "always" | "must" | "ensure" => "always",
                "should" | "prefer" | "consider" | "recommend" => "prefers",
                _ => "requires",
            };
            let object = format!("{verb} {body}");
            // Subject: prefer a known tool name in the condition phrase, else
            // fall through to the standard subject-extraction logic.
            let subject = if let Some(tool) = find_earliest_tool(&condition) {
                capitalize(tool)
            } else {
                extract_subject(&lower, section_context)
            };
            return Some((subject, predicate.to_string(), clean_object(&object), 7));
        }
        // Conditional shape but verb not in whitelist — fall through to
        // Layer 4 / Layer 6 / etc.  Don't return None here; one of the
        // later layers might still extract from prose like "When uncertain,
        // see ..." (in which case we explicitly want them to skip).
    }

    // Layer 4: Mid-sentence imperatives — "If X, STOP/never/don't Y" without
    // the trigger-word prefix Layer 7 expects (covers ", never X" and
    // "-- don't X" cases that don't lead with Before/After/When/If/For).
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
                return Some((subject, predicate.to_string(), clean_object(&object), 4));
            }
        }
    }

    // Layer 5: "use X" with three-signal gate.  The first two (known tool
    // present, co-imperative word in the line) keep the original
    // conservative behaviour.  The third (section context names a
    // conventions-shaped header) handles the very common "Use X" pattern
    // inside a `## Conventions` / `## Style` / `## Best Practices` section
    // where the writer means the section's framing, not a generic verb
    // call-out.
    let use_re = Regex::new(r"(?i)^use\s+(.+)").ok()?;
    if let Some(caps) = use_re.captures(&cleaned) {
        let object = caps.get(1)?.as_str().trim();
        let has_known_tool = contains_known_tool(&lower);
        let has_co_imperative = lower.contains("always") || lower.contains("must") || lower.contains("never");
        let in_rules_section = is_rules_section(section_context);

        if has_known_tool || has_co_imperative || in_rules_section {
            let subject = extract_subject(&lower, section_context);
            return Some((subject, "requires".to_string(), clean_object(object), 5));
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
        // v0.2.11 additions — measured against the broadened public corpus
        // (~80 additional rule extractions; see CHANGELOG).
        "create", "implement", "document", "define", "store",
        // Common imperatives observed in instruction prose (kraken,
        // arai, public corpora).  Each unambiguous in list-item-leading
        // position; low false-positive risk.  Also referenced by Layer 7's
        // allowed_verbs whitelist.
        "state", "share", "explain", "describe", "connect", "apply",
    ];

    let first_word = lower.split_whitespace().next().unwrap_or("");
    if verb_starts.contains(&first_word) {
        let subject = extract_subject(&lower, section_context);
        return Some((subject, "requires".to_string(), clean_object(&cleaned), 6));
    }

    None
}

/// Section-context test for the Layer 5 third-signal gate.  Only fires on
/// canonical "rules-shaped" section names so an unrelated `## Use Cases`
/// section doesn't accidentally promote every `use X` bullet.
fn is_rules_section(section_context: &Option<String>) -> bool {
    let Some(section) = section_context else { return false; };
    let lower = section.to_lowercase();
    [
        "convention",
        "conventions",
        "rule",
        "rules",
        "style",
        "guideline",
        "guidelines",
        "best practice",
        "best practices",
        "coding standard",
        "coding standards",
        "policy",
        "policies",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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
    fn test_layer_is_tagged() {
        // "Never X" → layer 1 (start-of-sentence imperative)
        let rules = extract_rules("- Never hand-write migration files", "test", 0.9);
        assert_eq!(rules[0].layer, Some(1));

        // "X is forbidden" → layer 2 (passive)
        let rules = extract_rules("- Force-push to main is forbidden", "test", 0.9);
        assert_eq!(rules[0].layer, Some(2));

        // "Label: imperative" → layer 3 (colon-separated)
        let rules = extract_rules("- Simplicity First: Make every change minimal", "test", 0.9);
        assert_eq!(rules[0].layer, Some(3));
    }

    #[test]
    fn test_expiry_is_extracted_and_stripped() {
        let rules = extract_rules(
            "- Never skip tests (expires 2026-12-31)",
            "test",
            0.9,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].expires_at.as_deref(), Some("2026-12-31"));
        // Object should no longer contain the annotation.
        assert!(!rules[0].object.contains("expires"), "object still carries annotation: {}", rules[0].object);
        assert!(rules[0].object.contains("skip tests"), "object missing rule body: {}", rules[0].object);
    }

    #[test]
    fn test_expiry_variants() {
        // "until" variant
        let rules = extract_rules(
            "- Always use autogenerate (until 2027-01-15)",
            "test",
            0.9,
        );
        assert_eq!(rules[0].expires_at.as_deref(), Some("2027-01-15"));

        // "expire" (no s) also accepted
        let rules = extract_rules(
            "- Avoid direct sql writes (expire 2026-06-01)",
            "test",
            0.9,
        );
        assert_eq!(rules[0].expires_at.as_deref(), Some("2026-06-01"));

        // No annotation → None
        let rules = extract_rules(
            "- Never force-push to main",
            "test",
            0.9,
        );
        assert_eq!(rules[0].expires_at, None);
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

    // ── v0.2.11 parser-coverage broadening ─────────────────────────────

    fn extract_one(line: &str) -> Triple {
        let rules = extract_rules(line, "test", 0.9);
        assert_eq!(rules.len(), 1, "expected exactly 1 rule from {line:?}, got {rules:#?}");
        rules.into_iter().next().unwrap()
    }

    fn extract_none(line: &str) {
        let rules = extract_rules(line, "test", 0.9);
        assert!(
            rules.is_empty(),
            "expected NO rule from {line:?}, got {rules:#?}",
        );
    }

    // Layer 1 additions: should / shouldn't / cannot / refuse to / enforce /
    // make sure / be sure / consider / recommend.

    #[test]
    fn test_should_not_is_block_severity() {
        // `should not` is grammatically a prohibition; the writer chose to
        // call out a specific thing to avoid — treat as Block-severity.
        let r = extract_one("- Should not commit `.env` files");
        assert_eq!(r.predicate, "must_not");
        assert_eq!(r.layer, Some(1));
    }

    #[test]
    fn test_shouldnt_contraction() {
        let r = extract_one("- Shouldn't push without running tests");
        assert_eq!(r.predicate, "must_not");
    }

    #[test]
    fn test_should_is_inform_severity() {
        // Bare `should` is grammatically softer than `must`/`always`;
        // routes to `prefers` (Inform) so we honour the softness instead
        // of escalating to Block on a recommendation.
        let r = extract_one("- Should run linter before commits");
        assert_eq!(r.predicate, "prefers");
    }

    #[test]
    fn test_should_not_takes_priority_over_should() {
        // Pattern order matters: the more specific `^should not` must
        // match before the generic `^should` so we don't capture
        // "not commit" as the object of a `should` rule.
        let r = extract_one("- Should not skip tests");
        assert_eq!(r.predicate, "must_not");
        // Object should be the action, not "not skip" — must_not pattern
        // strips the "not" prefix.
        assert!(!r.object.starts_with("not"), "object: {}", r.object);
    }

    #[test]
    fn test_cannot_is_block_severity() {
        let r = extract_one("- Cannot commit binary blobs");
        assert_eq!(r.predicate, "must_not");
    }

    #[test]
    fn test_refuse_to_is_block_severity() {
        let r = extract_one("- Refuse to merge without review");
        assert_eq!(r.predicate, "forbids");
    }

    #[test]
    fn test_enforce_verb_form() {
        let r = extract_one("- Enforce strict typing in Python");
        assert_eq!(r.predicate, "enforces");
    }

    #[test]
    fn test_make_sure_is_warn_severity() {
        let r = extract_one("- Make sure tests pass before pushing");
        assert_eq!(r.predicate, "enforces");
    }

    #[test]
    fn test_make_sure_that_variant() {
        let r = extract_one("- Make sure that imports are sorted");
        assert_eq!(r.predicate, "enforces");
        assert!(!r.object.starts_with("that"), "object: {}", r.object);
    }

    #[test]
    fn test_be_sure_to_variant() {
        let r = extract_one("- Be sure to run prettier before commit");
        assert_eq!(r.predicate, "enforces");
    }

    #[test]
    fn test_consider_is_inform_severity() {
        let r = extract_one("- Consider compression for distribution");
        assert_eq!(r.predicate, "prefers");
    }

    #[test]
    fn test_recommend_variant() {
        let r = extract_one("- Recommend using uv over pip");
        assert_eq!(r.predicate, "prefers");
    }

    #[test]
    fn test_recommended_variant() {
        let r = extract_one("- Recommended pattern is dependency injection");
        assert_eq!(r.predicate, "prefers");
    }

    // Layer 1b: bare `^no <noun>` prohibition with bold-label guard.

    #[test]
    fn test_bare_no_prohibition() {
        let r = extract_one("- No AI attribution in commit messages");
        assert_eq!(r.predicate, "must_not");
        assert_eq!(r.layer, Some(1));
    }

    #[test]
    fn test_bare_no_emojis() {
        let r = extract_one("- No emojis in commit messages");
        assert_eq!(r.predicate, "must_not");
    }

    #[test]
    fn test_bold_no_label_is_descriptive_not_rule() {
        // "**No build process** - this is a zero-build extension." is a
        // feature-absence DESCRIPTION and must NOT extract as a rule.
        // This is the most important negative test in the v0.2.11 batch.
        extract_none("- **No build process** - this is a zero-build extension.");
    }

    #[test]
    fn test_bold_no_emdash_descriptive() {
        extract_none("- **No CORS handling** — Traefik manages all cross-origin handling.");
    }

    // Bold-label guard for `consider`.

    #[test]
    fn test_bold_consider_label_is_section_not_rule() {
        // "**Consider constraints:** What are the goals?" is a heading-
        // shaped bullet, not a soft-preference rule.
        extract_none("- **Consider constraints:** What are the goals and limitations?");
    }

    #[test]
    fn test_bold_emphasis_on_always_still_extracts() {
        // Bold emphasis on a non-label imperative is still a rule —
        // the guard only applies to bold + colon/dash labels.
        let rules = extract_rules("- **Always** run tests before push", "test", 0.9);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].predicate, "always");
    }

    // Layer 5 — section-context gate loosening.

    #[test]
    fn test_use_inside_conventions_section_extracts() {
        // Inside a `## Conventions` section, "Use X" should fire even
        // without a known tool name in the line.
        let content = "\
## Conventions
- Use the `cn()` utility from $lib/utils for class merging
";
        let rules = extract_rules(content, "test", 0.9);
        assert_eq!(rules.len(), 1, "expected one rule, got {rules:#?}");
        assert_eq!(rules[0].layer, Some(5));
    }

    #[test]
    fn test_use_outside_rules_section_still_gated() {
        // Outside a rules-shaped section, the tightened gate still applies —
        // "Use X" with no known tool and no co-imperative does NOT fire.
        let content = "\
## Architecture
- Use the diagram below to follow the flow
";
        let rules = extract_rules(content, "test", 0.9);
        assert!(
            rules.is_empty(),
            "expected no rule outside rules-section, got {rules:#?}",
        );
    }

    // Layer 6 — verb expansion.

    #[test]
    fn test_create_imperative() {
        let r = extract_one("- Create lookup functions for quick queries");
        assert_eq!(r.layer, Some(6));
    }

    #[test]
    fn test_implement_imperative() {
        let r = extract_one("- Implement try_from for type-specific parsing");
        assert_eq!(r.layer, Some(6));
    }

    #[test]
    fn test_document_imperative() {
        let r = extract_one("- Document decision-making processes");
        assert_eq!(r.layer, Some(6));
    }

    #[test]
    fn test_define_imperative() {
        let r = extract_one("- Define color variables in `_sass/`");
        assert_eq!(r.layer, Some(6));
    }

    #[test]
    fn test_store_imperative() {
        let r = extract_one("- Store results for each socket separately");
        assert_eq!(r.layer, Some(6));
    }

    // Layer 7 — conditional imperatives.

    #[test]
    fn test_conditional_when_comma_run() {
        let r = extract_one("- When working in parallel, run tests in isolation");
        assert_eq!(r.layer, Some(7));
        assert_eq!(r.predicate, "requires");
    }

    #[test]
    fn test_conditional_before_run() {
        let r = extract_one("- Before completing work, run the full test suite");
        assert_eq!(r.layer, Some(7));
    }

    #[test]
    fn test_conditional_after_run() {
        let r = extract_one("- After every code change, run the linter");
        assert_eq!(r.layer, Some(7));
    }

    #[test]
    fn test_conditional_if_use() {
        let r = extract_one("- If the test suite is slow, use `--release` for benchmarks");
        assert_eq!(r.layer, Some(7));
    }

    #[test]
    fn test_conditional_for_use() {
        let r = extract_one("- For tasks that need more compute, use subagents to work in parallel");
        assert_eq!(r.layer, Some(7));
    }

    #[test]
    fn test_conditional_when_colon() {
        let r = extract_one("- When suggesting changes: state impact on the broader system");
        assert_eq!(r.layer, Some(7));
    }

    #[test]
    fn test_conditional_arrow_separator() {
        let r = extract_one("- If missing → show \"Data Download Required\" dialog");
        assert_eq!(r.layer, Some(7));
    }

    #[test]
    fn test_conditional_with_never_predicate() {
        // Conditional with a Layer-1-leader on the right side — verb
        // resolves to a prohibitive predicate, not generic `requires`.
        let r = extract_one("- When deploying to production, never skip smoke tests");
        assert_eq!(r.layer, Some(7));
        assert_eq!(r.predicate, "never");
    }

    #[test]
    fn test_conditional_unrecognised_verb_skipped() {
        // "When X, see Y" — `see` is not in the imperative whitelist,
        // so the line is prose continuation, not a rule.  Skipped to
        // avoid extracting bullet headers that just happen to start
        // with a trigger word.
        extract_none("- When uncertain, see the troubleshooting guide for guidance");
    }

    #[test]
    fn test_conditional_subject_uses_known_tool_in_condition() {
        // "When using cargo, ..." — subject should be `cargo` (the tool
        // mentioned in the trigger phrase), not a fallback extraction.
        let r = extract_one("- When using cargo, run cargo test before commit");
        assert_eq!(r.layer, Some(7));
        assert_eq!(r.subject.to_lowercase(), "cargo");
    }
}
