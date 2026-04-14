use serde::{Deserialize, Serialize};

/// Structured intent for a guardrail rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleIntent {
    pub action: Action,
    pub timing: Timing,
    pub tools: Vec<String>,
    pub allow_inverse: bool,
    pub enriched_by: String,
}

/// When a rule should fire in the workflow lifecycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Timing {
    /// Fire on specific tool calls (domain rules like "never hand-write migrations")
    ToolCall,
    /// Fire when the agent is about to finish (verification/completion rules)
    Stop,
    /// Fire once at the start of work (planning/setup rules)
    Start,
    /// General principle — fire infrequently as a periodic reminder
    Principle,
}

impl Timing {
    pub fn as_str(&self) -> &str {
        match self {
            Timing::ToolCall => "tool_call",
            Timing::Stop => "stop",
            Timing::Start => "start",
            Timing::Principle => "principle",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "tool_call" => Timing::ToolCall,
            "stop" => Timing::Stop,
            "start" => Timing::Start,
            "principle" => Timing::Principle,
            _ => Timing::ToolCall,
        }
    }

    /// Which hook event this timing maps to.
    pub fn hook_event(&self) -> &str {
        match self {
            Timing::ToolCall => "PreToolUse",
            // Principles, verification, and planning rules are already in CLAUDE.md.
            // Only domain-specific (ToolCall) rules get injected by Arai.
            // UserPromptSubmit shows a brief summary of active guardrails.
            Timing::Stop => "none",
            Timing::Start => "none",
            Timing::Principle => "none",
        }
    }
}

/// The action category a rule targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Create,
    Modify,
    Execute,
    General,
}

impl Action {
    pub fn as_str(&self) -> &str {
        match self {
            Action::Create => "create",
            Action::Modify => "modify",
            Action::Execute => "execute",
            Action::General => "general",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "create" => Action::Create,
            "modify" => Action::Modify,
            "execute" => Action::Execute,
            _ => Action::General,
        }
    }
}

// ---------------------------------------------------------------------------
// Tier 1: Expanded Semantic Verb Taxonomy
// ---------------------------------------------------------------------------
// Each list is ordered longest-first so multi-word phrases match before their
// single-word substrings (e.g. "hand-write" before "write").

/// Phrases signalling *creation* of new files/resources.
const CREATION_PHRASES: &[&str] = &[
    // Multi-word (match first)
    "start from scratch",
    "write from scratch",
    "from scratch",
    "manually create",
    "manually write",
    "manually generate",
    "manually produce",
    "manually author",
    "hand-write",
    "hand write",
    "hand-craft",
    "hand craft",
    "hand-code",
    "hand code",
    "handwrite",
    "handcraft",
    "handcode",
    "create file",
    "write file",
    "generate file",
    "produce file",
    "make file",
    "author file",
    "new file",
    "fresh file",
    "brand new",
    "create migration",
    "write migration",
    "generate migration",
    "create test",
    "write test",
    "add file",
    "add new",
    "introduce file",
    "create config",
    "write config",
    "scaffold",
    "boilerplate",
    "template file",
    "initialize file",
    "init file",
    // Single-word (check after multi-word)
    "write",
    "create",
    "generate",
    "produce",
    "author",
    "compose",
    "craft",
    "construct",
    "initialize",
];

/// Phrases signalling *modification* of existing files.
const MODIFICATION_PHRASES: &[&str] = &[
    // Multi-word
    "change file",
    "edit file",
    "modify file",
    "update file",
    "alter file",
    "adjust file",
    "patch file",
    "fix file",
    "revise file",
    "amend file",
    "refactor file",
    "rewrite file",
    "rework file",
    // Single-word
    "modify",
    "edit",
    "alter",
    "adjust",
    "tweak",
    "patch",
    "revise",
    "amend",
    "refactor",
    "rewrite",
    "rework",
    "overhaul",
    "transform",
    "mutate",
];

/// Phrases signalling *execution* of commands/tools.
const EXECUTION_PHRASES: &[&str] = &[
    // Multi-word
    "run command",
    "execute command",
    "invoke command",
    "run with",
    "run without",
    "execute with",
    "execute without",
    "use command",
    "use flag",
    "pass flag",
    "command line",
    "on the cli",
    "via cli",
    "via the cli",
    "via command",
    "from the terminal",
    "in the terminal",
    "with --",
    "without --",
    "use --",
    "generated via",
    "generated by",
    "generated using",
    "created via",
    "created by",
    "created using",
    "only be generated",
    "should be generated",
    "must be generated",
    "only be created",
    "should be created",
    "must be created",
    // Multi-word command phrases
    "git push",
    "git commit",
    "npm publish",
    "cargo publish",
    "deploy to",
    // Single-word
    "run",
    "deploy",
    "publish",
    "execute",
    "invoke",
    "launch",
    "install",
    "migrate",
];

// --- Timing classification phrases ---

/// Phrases indicating a rule is about verification/completion (→ Stop hook).
const STOP_PHRASES: &[&str] = &[
    "before marking complete",
    "before reporting",
    "before presenting",
    "before finishing",
    "before done",
    "before you finish",
    "before merging",
    "before committing",
    "before shipping",
    "before deploying",
    "before submitting",
    "proving it works",
    "prove it works",
    "demonstrate correctness",
    "verify your work",
    "check your work",
    "approve this",
    "staff engineer",
    "run tests",
    "check logs",
    "test before",
    "review section",
    "diff behavior",
    "diff against",
];

/// Phrases indicating a rule is about starting/planning work (→ UserPromptSubmit hook).
const START_PHRASES: &[&str] = &[
    "plan mode",
    "enter plan",
    "plan first",
    "write plan",
    "check in before starting",
    "before starting",
    "before implementation",
    "before you start",
    "upfront",
    "at the start",
    "at session start",
    "when given a",
    "when starting",
];

/// Phrases indicating a general principle (→ periodic reminder).
const PRINCIPLE_PHRASES: &[&str] = &[
    "every change",
    "as simple as possible",
    "root cause",
    "no temporary fix",
    "senior developer",
    "staff engineer",
    "only touch what",
    "no side effect",
    "minimal impact",
    "simplicity",
    "no laziness",
    "elegant",
    "elegance",
    "hacky",
];

/// Check if rule text references a known tool/library (word-boundary aware).
fn contains_known_tool_ref(lower_text: &str) -> bool {
    crate::parser::KNOWN_TOOLS.iter().any(|tool| {
        lower_text
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word == *tool)
    })
}

/// Classify a rule into structured intent based on its predicate and object text.
/// Optionally pass the subject for tool-reference checking.
#[allow(dead_code)]
pub fn classify_rule(predicate: &str, object: &str) -> RuleIntent {
    classify_rule_with_subject(predicate, object, None)
}

pub fn classify_rule_with_subject(predicate: &str, object: &str, subject: Option<&str>) -> RuleIntent {
    let lower = object.to_lowercase();
    let full_text = if let Some(s) = subject {
        format!("{} {}", s.to_lowercase(), lower)
    } else {
        lower.clone()
    };

    // Classify action
    let create_score = score_category(&lower, CREATION_PHRASES);
    let modify_score = score_category(&lower, MODIFICATION_PHRASES);
    let execute_score = score_category(&lower, EXECUTION_PHRASES);

    let action = if create_score == 0 && modify_score == 0 && execute_score == 0 {
        Action::General
    } else if execute_score > create_score && execute_score > modify_score {
        Action::Execute
    } else if create_score > modify_score {
        Action::Create
    } else if modify_score > create_score {
        Action::Modify
    } else {
        disambiguate(&lower, create_score, modify_score)
    };

    // Classify timing
    // Priority: domain rules (referencing a known tool) ALWAYS get ToolCall timing.
    // They are the core value of Arai — everything else is secondary.
    let has_tool_ref = contains_known_tool_ref(&full_text);

    let timing = if action != Action::General && has_tool_ref {
        // Domain-specific rules referencing a known tool → always fire on tool calls
        Timing::ToolCall
    } else {
        // Non-domain rules: classify by timing phrases
        let stop_score = score_category(&lower, STOP_PHRASES);
        let start_score = score_category(&lower, START_PHRASES);
        let principle_score = score_category(&lower, PRINCIPLE_PHRASES);

        if stop_score > start_score && stop_score > principle_score && stop_score > 0 {
            Timing::Stop
        } else if start_score > stop_score && start_score > principle_score && start_score > 0 {
            Timing::Start
        } else if principle_score > stop_score && principle_score > start_score && principle_score > 0 {
            Timing::Principle
        } else {
            Timing::Principle
        }
    };

    // Determine tool scope
    let tools = match action {
        Action::Create => vec!["Write".to_string(), "NotebookEdit".to_string()],
        Action::Modify => vec!["Edit".to_string()],
        Action::Execute => vec!["Bash".to_string()],
        Action::General => vec!["*".to_string()],
    };

    let is_prohibition = matches!(predicate, "never" | "forbids" | "must_not");
    let allow_inverse = is_prohibition && action == Action::Create;

    RuleIntent {
        action,
        timing,
        tools,
        allow_inverse,
        enriched_by: "taxonomy".to_string(),
    }
}

/// Score how strongly text matches a category.
/// Multi-word matches score higher than single-word.
/// Uses word-boundary matching to avoid "autogenerate" matching "generate".
fn score_category(text: &str, phrases: &[&str]) -> u32 {
    let mut score = 0u32;
    for phrase in phrases {
        if contains_phrase_bounded(text, phrase) {
            // Multi-word phrases score higher — they're more specific
            let word_count = phrase.split_whitespace().count() as u32;
            let phrase_score = word_count * 2;
            if phrase_score > score {
                score = phrase_score;
            }
        }
    }
    score
}

/// Check if text contains phrase with word-boundary awareness.
/// "autogenerate" should NOT match "generate", but "generate files" should match "generate".
fn contains_phrase_bounded(text: &str, phrase: &str) -> bool {
    // For multi-word phrases, just use contains — they're specific enough
    if phrase.contains(' ') || phrase.contains('-') {
        return text.contains(phrase);
    }

    // For single words, check word boundaries
    for (idx, _) in text.match_indices(phrase) {
        let before_ok = idx == 0
            || !text.as_bytes()[idx - 1].is_ascii_alphanumeric();
        let after_idx = idx + phrase.len();
        let after_ok = after_idx >= text.len()
            || !text.as_bytes()[after_idx].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Disambiguate when create and modify scores are tied.
fn disambiguate(text: &str, create_score: u32, modify_score: u32) -> Action {
    // "hand-write" is strongly creation-oriented
    if text.contains("hand-write") || text.contains("handwrite") || text.contains("hand write") {
        return Action::Create;
    }

    // "write" alone is ambiguous — could be create or modify
    // But in the context of rules, "write X" usually means "create X"
    if text.contains("write") && !text.contains("rewrite") && !text.contains("overwrite") {
        return Action::Create;
    }

    // Default: if scores are truly equal and we can't tell, go general
    if create_score == modify_score {
        Action::General
    } else if create_score > modify_score {
        Action::Create
    } else {
        Action::Modify
    }
}

/// Check if a tool name matches the intent's tool scope.
pub fn tool_matches_intent(intent: &RuleIntent, tool_name: &str) -> bool {
    // Wildcard matches everything
    if intent.tools.iter().any(|t| t == "*") {
        return true;
    }

    // Direct match
    if intent.tools.iter().any(|t| t == tool_name) {
        return true;
    }

    // allow_inverse: if rule prohibits creating, allow editing
    if intent.allow_inverse && tool_name == "Edit" {
        return false; // Explicitly skip — the rule doesn't apply to edits
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hand_write_is_create() {
        let intent = classify_rule("never", "hand-write migration files");
        assert_eq!(intent.action, Action::Create);
        assert!(intent.tools.contains(&"Write".to_string()));
        assert!(!intent.tools.contains(&"Edit".to_string()));
        assert!(intent.allow_inverse); // Editing is fine
    }

    #[test]
    fn test_create_manually_is_create() {
        let intent = classify_rule("forbids", "create migrations manually");
        assert_eq!(intent.action, Action::Create);
        assert!(intent.allow_inverse);
    }

    #[test]
    fn test_dont_create_is_create() {
        let intent = classify_rule("forbids", "manually create migration files");
        assert_eq!(intent.action, Action::Create);
    }

    #[test]
    fn test_generated_via_cli_is_execute() {
        let intent = classify_rule("requires", "migrations should only be generated via CLI");
        assert_eq!(intent.action, Action::Execute);
        assert!(intent.tools.contains(&"Bash".to_string()));
    }

    #[test]
    fn test_modify_existing_is_modify() {
        let intent = classify_rule("forbids", "modify existing configuration files");
        assert_eq!(intent.action, Action::Modify);
        assert!(intent.tools.contains(&"Edit".to_string()));
    }

    #[test]
    fn test_always_use_autogenerate_is_general() {
        let intent = classify_rule("always", "use autogenerate");
        assert_eq!(intent.action, Action::General);
        assert!(intent.tools.contains(&"*".to_string()));
    }

    #[test]
    fn test_run_tests_before_merging_is_execute() {
        let intent = classify_rule("requires", "run tests before merging");
        assert_eq!(intent.action, Action::Execute);
    }

    #[test]
    fn test_force_push_is_general() {
        let intent = classify_rule("never", "force-push to main");
        assert_eq!(intent.action, Action::General);
    }

    #[test]
    fn test_scaffold_is_create() {
        let intent = classify_rule("forbids", "scaffold new components by hand");
        assert_eq!(intent.action, Action::Create);
    }

    #[test]
    fn test_refactor_is_modify() {
        let intent = classify_rule("always", "refactor large functions into smaller ones");
        assert_eq!(intent.action, Action::Modify);
    }

    #[test]
    fn test_write_without_rewrite() {
        // "write" should be create, "rewrite" should be modify
        let intent = classify_rule("never", "write SQL queries inline");
        assert_eq!(intent.action, Action::Create);

        let intent = classify_rule("always", "rewrite complex queries");
        assert_eq!(intent.action, Action::Modify);
    }

    #[test]
    fn test_allow_inverse_only_on_prohibition() {
        let intent = classify_rule("always", "create files using templates");
        assert_eq!(intent.action, Action::Create);
        assert!(!intent.allow_inverse); // "always create" — not a prohibition

        let intent = classify_rule("never", "create files manually");
        assert_eq!(intent.action, Action::Create);
        assert!(intent.allow_inverse); // "never create" — editing is fine
    }

    #[test]
    fn test_tool_matches_intent() {
        let create_intent = classify_rule("never", "hand-write migration files");
        assert!(tool_matches_intent(&create_intent, "Write"));
        assert!(!tool_matches_intent(&create_intent, "Edit")); // allow_inverse
        assert!(!tool_matches_intent(&create_intent, "Bash"));

        let general_intent = classify_rule("never", "force-push to main");
        assert!(tool_matches_intent(&general_intent, "Write"));
        assert!(tool_matches_intent(&general_intent, "Bash"));
        assert!(tool_matches_intent(&general_intent, "Edit"));
    }
}
