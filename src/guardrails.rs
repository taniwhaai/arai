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

/// Tokens that disqualify a `<tool> <next>` adjacency from forming a
/// `tool subcommand` phrase.  These are the prepositions / articles /
/// connectors that show up between a tool name and its real context in
/// English rule prose ("never use docker **for** production",
/// "always use cargo **with** --release").  When we see one of these
/// immediately after a tool, the rule is talking about the tool generically
/// — there is no subcommand to gate on, so the phrase isn't extracted and
/// the rule falls through to bag-of-words scoring.  Kept short and
/// preposition-only on purpose: short verbs like `run`, `test`, `build`
/// MUST remain extractable as subcommands.
const PHRASE_STOPWORDS: &[&str] = &[
    "for", "in", "with", "to", "the", "a", "an", "as", "from", "into",
    "on", "of", "by", "at", "is", "are", "or", "and", "via",
];

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

/// Extract `<tool> <subcommand>` adjacency phrases from a tool call.  Each
/// phrase is the lowercase string `"<tool> <subcommand>"` where `tool` is a
/// member of `KNOWN_TOOLS` and `subcommand` is the next non-flag,
/// non-empty token.  Used together with `extract_terms` to gate rules that
/// name a specific tool subcommand: a rule whose object mentions
/// `docker run` should fire on `docker run -it ubuntu` but NOT on
/// `docker compose up`.  See `relevance_score`'s phrase-gate for the
/// matching contract.
///
/// Returns an empty vec for non-Bash tools (file operations have no
/// subcommand structure).  Empty result also means "no informative phrases
/// available" — the gate inside `relevance_score` does the right thing
/// either way: a rule with phrases against a phrase-less command drops,
/// which mirrors the existing verb-mismatch behaviour for rules that
/// name a specific action.
pub fn extract_command_phrases(tool_name: &str, tool_input: &Value) -> Vec<String> {
    if tool_name != "Bash" {
        return Vec::new();
    }
    let command = match tool_input.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => return Vec::new(),
    };

    let mut phrases = Vec::new();
    for segment in command.split(['|', ';']) {
        for sub in segment.split("&&") {
            extract_phrases_from_segment(sub.trim(), &mut phrases);
        }
    }
    phrases.sort();
    phrases.dedup();
    phrases
}

/// Walk one shell segment and emit a phrase whenever a `KNOWN_TOOLS` token
/// is followed by a non-flag, non-empty subcommand token.  Skips intervening
/// flags so that `git -c user.name=foo push origin` still produces the
/// `git push` phrase.
fn extract_phrases_from_segment(segment: &str, phrases: &mut Vec<String>) {
    let raw_tokens: Vec<String> = shell_tokenize(segment)
        .into_iter()
        .flat_map(|t| {
            t.split_whitespace()
                .map(|w| w.to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    let mut i = 0;
    while i + 1 < raw_tokens.len() {
        let tool = raw_tokens[i]
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&raw_tokens[i])
            .to_lowercase();
        if KNOWN_TOOLS.iter().any(|t| *t == tool.as_str()) {
            let mut j = i + 1;
            while j < raw_tokens.len() && raw_tokens[j].starts_with('-') {
                j += 1;
                // Defensively skip the flag's value too only if the flag was
                // a single dash with a single letter (e.g. `-c value`).  The
                // alternative — guessing arity per binary — is more wrong
                // than missing the occasional `git -c user.name=foo push`
                // phrase, and `=`-style flags don't burn a token anyway.
                if j < raw_tokens.len()
                    && !raw_tokens[j - 1].contains('=')
                    && raw_tokens[j - 1].len() == 2
                {
                    j += 1;
                }
            }
            if j < raw_tokens.len() {
                let sub = raw_tokens[j].to_lowercase();
                let clean: String = sub
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_string();
                if !clean.is_empty() {
                    phrases.push(format!("{tool} {clean}"));
                }
            }
        }
        i += 1;
    }
}

/// Extract `<tool> <subcommand>` phrases from a rule's object text.  Same
/// shape as `extract_command_phrases` but operates on English prose: there
/// are no flags to skip, and the next-token check filters preposition /
/// article stopwords (`for`, `with`, `to`, …) so a generic rule like
/// "never use docker for production" produces NO phrase and therefore
/// falls through to bag-of-words scoring.  A rule that mentions an
/// explicit subcommand — "never use docker run for local dev" — produces
/// `"docker run"` and gets gated against the command's phrases.
fn extract_rule_phrases(object: &str) -> Vec<String> {
    let lower = object.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| !w.is_empty())
        .collect();
    let mut phrases = Vec::new();
    for i in 0..words.len().saturating_sub(1) {
        let tool = words[i];
        if !KNOWN_TOOLS.iter().any(|t| *t == tool) {
            continue;
        }
        let next = words[i + 1];
        if next.len() < 2 || PHRASE_STOPWORDS.contains(&next) {
            continue;
        }
        phrases.push(format!("{tool} {next}"));
    }
    phrases.sort();
    phrases.dedup();
    phrases
}

/// Match guardrails against extracted terms, filtering by classified intent and timing.
/// Results are ranked by relevance — rules whose object text overlaps with the
/// command terms are ranked higher.
pub fn match_guardrails(
    guardrails: &[Guardrail],
    terms: &[String],
    command_phrases: &[String],
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
                    subject_matches_terms(&g.subject, terms)
                        && crate::intent::tool_matches_intent(intent, tool_name)
                } else {
                    true
                }
            } else {
                if hook_event != "PreToolUse" {
                    return false;
                }
                subject_matches_terms(&g.subject, terms)
            }
        })
        .filter_map(|g| {
            // `None` means the rule names a specific command verb the
            // command doesn't perform — drop it rather than fire on
            // substring overlap from the subject alone.  See `relevance_score`
            // for the verb-mismatch and phrase-gate contracts.
            let score = relevance_score(&g.object, terms, command_phrases)?;
            Some((g.clone(), score))
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
/// Command verbs that, when present in a rule's object, gate the rule by
/// requiring at least one of them to also appear in the extracted command
/// terms.  See `relevance_score` for the contract.
///
/// **Why `"run"` is intentionally absent.**  `run` is also in `NOISE_WORDS`
/// (filtered from extracted command terms — `cargo run`, `docker run`,
/// `npm run` all share the wrapper word, which carries no signal on its
/// own).  If `run` lived in both lists the verb-mismatch check would
/// permanently drop any rule whose only command-verb is `run`, because
/// the term is structurally absent from every command.  That broke
/// rules of the shape `Never use docker run for X` against the matching
/// `docker run …` command.  The trade-off: a rule whose only verb is
/// `run` no longer gets verb-mismatch protection against sibling
/// subcommands of the same tool — `Never use docker run for X` will
/// fire on `docker compose up` too.  The proper fix is adjacency-aware
/// multi-word verb matching (treat `docker run` as a phrase); tracked
/// separately.
const COMMAND_VERBS: &[&str] = &[
    "push", "pull", "commit", "merge", "rebase", "checkout", "clone", "fetch",
    "stash", "reset", "revert", "cherry", "bisect", "tag", "branch",
    "install", "uninstall", "build", "test", "deploy", "publish",
    "start", "stop", "create", "delete", "remove", "add", "update", "upgrade",
];

/// Check whether a rule's subject overlaps with any extracted command term as
/// a *whole word*.  Replaces an earlier `subj.contains(term)` substring check
/// which leaked across word boundaries: subject "git" matched any subject
/// containing "git" as a substring (e.g. `github`) and any rule subject also
/// containing "git" as a substring fired on every `git *` command.  The
/// substring leak in the opposite direction (term-inside-subject) was the
/// proximate cause of issue #86 — single-token subjects like "Git" matched
/// every `git <subcommand>` regardless of whether the rule was about that
/// subcommand.  Token-boundary matching closes the leak; the verb-mismatch
/// contract in `relevance_score` does the rest of the disambiguation.
fn subject_matches_terms(subject: &str, terms: &[String]) -> bool {
    let subj_lower = subject.to_lowercase();
    let subj_tokens: Vec<&str> = subj_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if subj_tokens.is_empty() {
        return false;
    }
    subj_tokens
        .iter()
        .any(|st| terms.iter().any(|t| t == st))
}

/// Score a rule's object text against the extracted command terms and
/// `<tool> <subcommand>` adjacency phrases.
///
/// Returns `None` in two cases:
///
/// 1. **Phrase mismatch (#98)** — the rule names a tool-subcommand
///    adjacency (`docker run`, `git push`, `cargo test`) and the command
///    contains no matching phrase.  This catches the case the
///    word-overlap matcher cannot: `docker run` and `docker compose`
///    share the same single-word terms (`docker`), so word overlap
///    alone can't tell the two subcommands apart.  Checked first
///    because it is the most specific.
/// 2. **Verb mismatch (#86)** — the rule names a `COMMAND_VERB`
///    (push, commit, install, …) that does NOT appear in the command
///    terms.  This lets a rule about `git push` not fire on
///    `git status` / `git diff` / `git log`.  Runs second so a rule
///    that survives the phrase gate isn't double-jeopardied by the
///    coarser verb check.
///
/// `Some(n)` is the count of overlapping words when neither gate
/// fires; `n` may be zero for general rules whose object names no
/// command verbs and no command-term overlap (e.g. "hand-write
/// migration files" firing on a Write because the *subject* matched).
fn relevance_score(
    object: &str,
    terms: &[String],
    command_phrases: &[String],
) -> Option<usize> {
    let object_words: Vec<String> = object
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(String::from)
        .collect();

    // Phrase gate (#98): if the rule's object names a `<tool> <subcommand>`
    // adjacency, the command must contain that same phrase.  Skip the gate
    // when the rule has no phrase (generic rule, falls through to bag-of-
    // words) — `extract_rule_phrases` already filters preposition stopwords
    // so "never use docker for production" produces no phrase.
    let rule_phrases = extract_rule_phrases(object);
    if !rule_phrases.is_empty() {
        let phrase_match = rule_phrases
            .iter()
            .any(|p| command_phrases.iter().any(|c| c == p));
        if !phrase_match {
            return None;
        }
    }

    let base_score: usize = terms.iter()
        .filter(|t| object_words.iter().any(|w| w == *t))
        .count();

    // Verb mismatch: rule object contains specific command verbs and NONE of
    // them appear in the command terms.  Previously this halved the score
    // (min 1) so the rule still fired; that let a `git push` rule block
    // every `git status`.  Now: drop the match entirely.
    let rule_verbs: Vec<&String> = object_words.iter()
        .filter(|w| COMMAND_VERBS.contains(&w.as_str()))
        .collect();
    let has_verb_mismatch = !rule_verbs.is_empty()
        && !rule_verbs.iter().any(|v| terms.contains(v));

    if has_verb_mismatch {
        None
    } else {
        Some(base_score)
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

    proptest::proptest! {
        /// Three properties on `sniff_content_for_tools`, each generated with
        /// arbitrary Unicode strings:
        ///   1. Never panics.  Aho-Corasick + the byte-level boundary check
        ///      must handle multi-byte sequences without indexing past a
        ///      char boundary or reading past the buffer.
        ///   2. Every term in the output IS in `KNOWN_TOOLS` — the function
        ///      can't invent strings that weren't in the haystack-pattern
        ///      table.
        ///   3. Each term appears at most once — the dedupe bitset works
        ///      regardless of how many times the substring appears.
        #[test]
        fn prop_sniff_invariants(s in ".{0,300}") {
            let mut terms = Vec::new();
            sniff_content_for_tools(&s, &mut terms);

            // 2: every produced term is a known tool.
            for t in &terms {
                proptest::prop_assert!(
                    KNOWN_TOOLS.iter().any(|k| k == t),
                    "sniff produced {t:?} which is not in KNOWN_TOOLS (input={s:?})"
                );
            }
            // 3: dedup — sorting + dedup must not shrink the list.
            let mut sorted = terms.clone();
            sorted.sort();
            sorted.dedup();
            proptest::prop_assert_eq!(sorted.len(), terms.len(),
                "duplicates leaked through dedupe bitset (input={:?}, terms={:?})",
                s, terms);
        }

        /// Idempotence: appending the same content's matches to an already-
        /// populated terms vec must not introduce duplicates of tools the
        /// vec already contained from a prior sniff call.  This is the
        /// invariant the public extract_file_terms relies on when sniffing
        /// `content`, `new_string`, and `old_string` in sequence.
        ///
        /// Note: the function pushes without checking whether the term is
        /// already in `terms` — dedup happens at the call site via
        /// `sort` + `dedup` after all sniffs.  So the property here is
        /// "calling sniff twice on the same input produces matched terms
        /// from each call independently"; each individual call is dedup'd
        /// internally.  We verify that property: 2nd call's terms are a
        /// subset of 1st call's set.
        #[test]
        fn prop_sniff_repeat_call_produces_subset(s in ".{0,300}") {
            let mut a = Vec::new();
            sniff_content_for_tools(&s, &mut a);
            let mut b = Vec::new();
            sniff_content_for_tools(&s, &mut b);
            proptest::prop_assert_eq!(&a, &b, "deterministic on same input");
        }
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

        // Alembic "hand-write" rule: ToolCall timing, create scope.  Rule
        // object has no `<tool> <subcommand>` adjacency so command_phrases
        // is irrelevant — pass &[] across the board.
        let terms = vec!["alembic".to_string()];
        let matched = match_guardrails(&guardrails, &terms, &[], "Write", "PreToolUse");
        assert_eq!(matched.len(), 1, "hand-write rule should fire on Write/PreToolUse");

        let matched = match_guardrails(&guardrails, &terms, &[], "Bash", "PreToolUse");
        assert_eq!(matched.len(), 0, "hand-write rule should not fire on Bash");

        let matched = match_guardrails(&guardrails, &terms, &[], "Edit", "PreToolUse");
        assert_eq!(matched.len(), 0, "hand-write rule should not fire on Edit (allow_inverse)");

        // Git "force-push" rule: principle timing → doesn't fire on any hook
        // (principles are already in CLAUDE.md, Arai doesn't repeat them)
        let terms = vec!["git".to_string()];
        let matched = match_guardrails(&guardrails, &terms, &[], "Bash", "PreToolUse");
        assert_eq!(matched.len(), 0, "principle rule should not fire on PreToolUse");

        let matched = match_guardrails(&guardrails, &terms, &[], "Bash", "UserPromptSubmit");
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
        // "push" and "main" both overlap, phrase matches, no verb mismatch
        let terms = vec!["git".to_string(), "push".to_string(), "origin".to_string(), "main".to_string()];
        let phrases = vec!["git push".to_string()];
        let score = relevance_score("git push to main without a PR", &terms, &phrases)
            .expect("push verb matches — should score Some");
        assert!(score >= 3, "should match git, push, main — got {score}");

        // commit ≠ push: phrase mismatch (#98) drops first; even without
        // phrases the verb-mismatch fallback would also drop.
        let score = relevance_score("git commit with signed commits", &terms, &phrases);
        assert!(score.is_none(),
            "commit-verb rule should be dropped for a push command — got {score:?}");
    }

    #[test]
    fn test_relevance_verb_mismatch_drops_rule() {
        // Rule says "push" but command is "pull" — phrase mismatch drops.
        let pull_terms = vec!["git".to_string(), "pull".to_string(), "origin".to_string(), "main".to_string()];
        let pull_phrases = vec!["git pull".to_string()];
        assert!(relevance_score("git push to main without a PR", &pull_terms, &pull_phrases).is_none());
        assert!(relevance_score("git commit with signed commits", &pull_terms, &pull_phrases).is_none());

        // Rule says "install" but command is "uninstall" — phrase mismatch.
        let terms = vec!["npm".to_string(), "uninstall".to_string(), "package".to_string()];
        let phrases = vec!["npm uninstall".to_string()];
        assert!(relevance_score("npm install with --save-exact", &terms, &phrases).is_none());
    }

    #[test]
    fn test_relevance_no_penalty_when_verb_matches() {
        // Exact phrase match — kept and scored
        let terms = vec!["cargo".to_string(), "test".to_string()];
        let phrases = vec!["cargo test".to_string()];
        let score = relevance_score("run cargo test before pushing", &terms, &phrases)
            .expect("test verb matches");
        assert!(score >= 2, "exact verb match should score high — got {score}");
    }

    #[test]
    fn test_relevance_no_verb_in_rule() {
        // Rule has no command verb AND no tool-subcommand phrase — pure term
        // overlap, no gating.  Pass &[] for phrases since the rule has none
        // and the gate is therefore inert.
        let terms = vec!["git".to_string(), "push".to_string(), "main".to_string()];
        let score = relevance_score("always use feature branches for main", &terms, &[])
            .expect("no command verb in rule → no mismatch path");
        assert!(score >= 1, "should match 'main' without penalty — got {score}");
    }

    #[test]
    fn test_relevance_zero_overlap() {
        // No terms match at all AND no command verbs in rule → Some(0)
        let terms = vec!["docker".to_string(), "compose".to_string(), "up".to_string()];
        let score = relevance_score("hand-write migration files", &terms, &[])
            .expect("no command verb in rule → keep, score 0");
        assert_eq!(score, 0, "no overlap should score 0 — got {score}");
    }

    /// `run` is a `NOISE_WORD` (filtered from extracted command terms) so it
    /// can't be a `COMMAND_VERB` for the verb-mismatch check — the term
    /// would be structurally absent from every command and the gate would
    /// permanently fire.  `run` was removed from `COMMAND_VERBS` to fix
    /// that.  Disambiguation between `docker run` and `docker compose` is
    /// instead provided by the adjacency-aware phrase gate (#98) inside
    /// `relevance_score`: the rule's `<tool> <subcommand>` phrase must
    /// match a phrase in the command.
    #[test]
    fn test_relevance_run_no_longer_blocks_docker_run_rule() {
        // Real `docker run` command — phrase matches, rule keeps.
        let docker_run_terms = vec!["docker".to_string(), "ubuntu".to_string()];
        let docker_run_phrases = vec!["docker run".to_string()];
        let score = relevance_score(
            "use docker run for local dev",
            &docker_run_terms,
            &docker_run_phrases,
        )
        .expect("phrase docker run matches the command — rule must keep");
        assert!(score >= 1, "docker term should overlap — got {score}");

        // Sibling subcommand — phrase mismatch (#98) drops the rule.  This
        // is the assertion that flipped from `Some(>=1)` to `is_none()`
        // when adjacency matching landed; the issue's acceptance criteria
        // call this out explicitly.
        let docker_compose_terms = vec!["docker".to_string(), "compose".to_string()];
        let docker_compose_phrases = vec!["docker compose".to_string()];
        let result = relevance_score(
            "use docker run for local dev",
            &docker_compose_terms,
            &docker_compose_phrases,
        );
        assert!(
            result.is_none(),
            "docker run rule must NOT fire on docker compose — got {result:?}"
        );
    }

    #[test]
    fn extract_rule_phrases_basic() {
        assert_eq!(
            extract_rule_phrases("use docker run for local dev"),
            vec!["docker run".to_string()]
        );
        assert_eq!(
            extract_rule_phrases("Always run cargo test before pushing"),
            vec!["cargo test".to_string()]
        );
        // Multiple phrases in one rule are deduplicated and sorted.
        let phrases = extract_rule_phrases("prefer cargo test over cargo run");
        assert_eq!(phrases, vec!["cargo run".to_string(), "cargo test".to_string()]);
    }

    #[test]
    fn extract_rule_phrases_skips_preposition_stopwords() {
        // "for", "with", "as", "in", "to", "from" are preposition stopwords —
        // a rule that mentions a tool followed by one of these is generic
        // and should NOT produce a phrase, so the rule falls through to
        // bag-of-words instead of being phrase-gated.
        assert!(extract_rule_phrases("never use docker for production").is_empty());
        assert!(extract_rule_phrases("always use cargo with --release").is_empty());
        assert!(extract_rule_phrases("never run docker as root").is_empty());
    }

    #[test]
    fn extract_rule_phrases_no_tool_no_phrase() {
        assert!(extract_rule_phrases("hand-write migration files").is_empty());
        assert!(extract_rule_phrases("always use feature branches for main").is_empty());
    }

    #[test]
    fn extract_command_phrases_bash_basic() {
        let input = serde_json::json!({"command": "docker run -it ubuntu"});
        assert_eq!(
            extract_command_phrases("Bash", &input),
            vec!["docker run".to_string()]
        );
        let input = serde_json::json!({"command": "git push --force origin main"});
        assert_eq!(
            extract_command_phrases("Bash", &input),
            vec!["git push".to_string()]
        );
    }

    #[test]
    fn extract_command_phrases_skips_flag_clusters() {
        // `git -c user.name=foo push origin` — the `-c` flag and its `=`-style
        // value should not break the `git push` phrase.
        let input = serde_json::json!({"command": "git -c user.name=foo push origin main"});
        let phrases = extract_command_phrases("Bash", &input);
        assert!(
            phrases.contains(&"git push".to_string()),
            "expected git push in {phrases:?}"
        );
    }

    #[test]
    fn extract_command_phrases_pipeline_segments() {
        // Pipelines, semicolons, and `&&` all produce independent segments;
        // each can contribute its own phrase.
        let input = serde_json::json!({"command": "git status && docker run -it ubuntu"});
        let phrases = extract_command_phrases("Bash", &input);
        assert!(phrases.contains(&"git status".to_string()), "got {phrases:?}");
        assert!(phrases.contains(&"docker run".to_string()), "got {phrases:?}");
    }

    #[test]
    fn extract_command_phrases_non_bash_is_empty() {
        // File operations have no `<tool> <subcommand>` structure — phrase
        // gating is intentionally Bash-only.
        let input = serde_json::json!({"file_path": "/tmp/docker-compose.yml"});
        assert!(extract_command_phrases("Write", &input).is_empty());
        assert!(extract_command_phrases("Edit", &input).is_empty());
    }

    #[test]
    fn extract_command_phrases_path_prefixed_tool() {
        // Absolute path to the binary is still recognised as the tool.
        let input = serde_json::json!({"command": "/usr/local/bin/docker run -it ubuntu"});
        assert_eq!(
            extract_command_phrases("Bash", &input),
            vec!["docker run".to_string()]
        );
    }

    /// Phrase-gate locks the sibling-subcommand case for *every* tool we
    /// extract phrases for, not just docker.  `npm install` rule must not
    /// fire on `npm test`; `cargo test` rule must not fire on `cargo build`.
    #[test]
    fn phrase_gate_blocks_sibling_subcommands_across_tools() {
        // npm install rule vs npm test command
        let result = relevance_score(
            "always pin npm install with --save-exact",
            &["npm".to_string(), "test".to_string()],
            &["npm test".to_string()],
        );
        assert!(result.is_none(), "npm install rule must drop on npm test — got {result:?}");

        // cargo test rule vs cargo build command
        let result = relevance_score(
            "always run cargo test in release mode",
            &["cargo".to_string(), "build".to_string()],
            &["cargo build".to_string()],
        );
        assert!(result.is_none(), "cargo test rule must drop on cargo build — got {result:?}");

        // Same rules survive against the matching command.
        assert!(
            relevance_score(
                "always pin npm install with --save-exact",
                &["npm".to_string(), "install".to_string()],
                &["npm install".to_string()],
            )
            .is_some(),
            "npm install rule must keep on npm install"
        );
        assert!(
            relevance_score(
                "always run cargo test in release mode",
                &["cargo".to_string(), "test".to_string()],
                &["cargo test".to_string()],
            )
            .is_some(),
            "cargo test rule must keep on cargo test"
        );
    }

    /// A generic rule (no tool-subcommand phrase) is unaffected by the
    /// phrase gate — falls through to bag-of-words scoring.  This is what
    /// keeps rules like "Always use feature branches for main" working
    /// against any command whose subject matches.
    #[test]
    fn phrase_gate_is_inert_for_phraseless_rules() {
        // Rule has no tool-subcommand phrase; phrase gate skipped entirely.
        let score = relevance_score(
            "always use feature branches for main",
            &["git".to_string(), "push".to_string(), "main".to_string()],
            &["git push".to_string()],
        )
        .expect("phraseless rule should not be gated");
        assert!(score >= 1);
    }

    #[test]
    fn subject_matches_terms_uses_token_boundary() {
        // Single-token subject only matches an exact token in the command terms,
        // never a substring or hyphen-fragment.
        let subj = "git";
        assert!(subject_matches_terms(subj, &["git".to_string(), "status".to_string()]));
        // The actual term "git-level" was produced by `gh issue create --title
        // "...git-level..."` in issue #86; subject "git" must NOT match it.
        assert!(!subject_matches_terms(subj, &["git-level".to_string()]));
        // A rule subject "github" must not match a `git ...` command via substring.
        assert!(!subject_matches_terms("github", &["git".to_string(), "status".to_string()]));
        // Multi-word subject — any constituent word can match.
        assert!(subject_matches_terms("force-push", &["force".to_string()]));
        assert!(subject_matches_terms("git push", &["push".to_string()]));
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
        let phrases = vec!["git push".to_string()];
        let matched = match_guardrails(&guardrails, &terms, &phrases, "Bash", "PreToolUse");
        assert_eq!(matched.len(), 1, "only the relevant rule should fire");
        assert!(matched[0].0.object.contains("push"), "push rule should fire, got: {}", matched[0].0.object);

        // "git commit -m test" should ONLY fire the commit rule
        let terms = vec!["git".to_string(), "commit".to_string(), "test".to_string()];
        let phrases = vec!["git commit".to_string()];
        let matched = match_guardrails(&guardrails, &terms, &phrases, "Bash", "PreToolUse");
        assert_eq!(matched.len(), 1, "only the relevant rule should fire");
        assert!(matched[0].0.object.contains("commit"), "commit rule should fire, got: {}", matched[0].0.object);

        // "git pull origin main" — both rules should be suppressed (pull ≠ push, pull ≠ commit)
        let terms = vec!["git".to_string(), "pull".to_string(), "origin".to_string(), "main".to_string()];
        let phrases = vec!["git pull".to_string()];
        let matched = match_guardrails(&guardrails, &terms, &phrases, "Bash", "PreToolUse");
        assert!(matched.is_empty(),
            "neither push nor commit rule should fire for pull command — got {matched:?}");

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
