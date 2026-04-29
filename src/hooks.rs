use crate::config::Config;
use crate::intent::Severity;
use crate::store::{Guardrail, Store};
use crate::{audit, compliance, config, guardrails, session, store};
use serde_json::Value;
use std::io::Read;

/// Hard cap on the size of a hook payload from stdin.  Real Claude Code hook
/// invocations are well under 100 KB; 1 MiB is generous.  Without a cap, a
/// malicious or runaway tool that pipes gigabytes into our hook handler
/// would OOM the binary on the hot path.
const MAX_HOOK_INPUT_BYTES: u64 = 1024 * 1024;

/// Environment variable that, when set to `off` or `0`, forces Arai into
/// advise-only mode — even `Block`-severity rules fall back to
/// `permissionDecision: "allow"` with the rule attached as context.  Useful
/// when rolling Arai out incrementally: ingest rules, measure compliance for
/// a week, then flip deny mode on once you trust the rule set.
const DENY_MODE_ENV: &str = "ARAI_DENY_MODE";

fn deny_mode_enabled() -> bool {
    match std::env::var(DENY_MODE_ENV) {
        Ok(v) => {
            let v = v.to_lowercase();
            v != "off" && v != "0" && v != "false" && v != "no"
        }
        Err(_) => true,
    }
}

/// Highest severity among the matched rules, if any.  Used to pick between
/// advise (`allow`) and deny (`deny`) on PreToolUse.
fn highest_severity(matched: &[(Guardrail, u8)], db: &Store) -> Severity {
    let mut highest = Severity::Inform;
    for (g, _) in matched {
        if let Ok(Some(intent)) = db.get_rule_intent(g.triple_id) {
            if intent.severity == Severity::Block {
                return Severity::Block;
            }
            if intent.severity == Severity::Warn && highest == Severity::Inform {
                highest = Severity::Warn;
            }
        } else {
            // No classified intent — fall back to predicate-derived severity so
            // pre-migration stores still block on obvious `never` rules.
            let sev = Severity::from_predicate(&g.predicate);
            if sev == Severity::Block {
                return Severity::Block;
            }
            if sev == Severity::Warn && highest == Severity::Inform {
                highest = Severity::Warn;
            }
        }
    }
    highest
}

/// Result of matching a hook payload against the current guardrail set.
/// Pure — no audit write, no telemetry, no stdout.
pub struct HookMatch {
    pub event: String,
    pub tool_name: String,
    pub session_id: String,
    pub terms: Vec<String>,
    pub matched: Vec<(Guardrail, u8)>,
    /// True if the tool is in the skip list — matching was bypassed entirely.
    pub skipped: bool,
    /// True if the caller should emit the UserPromptSubmit summary instead of
    /// a match response.
    pub is_prompt_summary: bool,
    /// Domain rules for the UserPromptSubmit summary (populated only when
    /// `is_prompt_summary` is true).
    pub domain_rules: Vec<Guardrail>,
}

/// Apply the full match pipeline to a parsed hook payload.
///
/// Mirrors the behaviour of `handle_stdin` without performing any IO:
/// caller owns stdout, audit log, and telemetry.  Used by the hook handler
/// *and* by the `arai test` scenario runner — both paths see the same
/// matching logic so scenarios stay faithful to production.
pub fn match_hook(
    hook: &Value,
    cfg: &Config,
    db: &Store,
) -> Result<HookMatch, String> {
    let event = hook
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("PreToolUse")
        .to_string();
    let tool_name = hook
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = hook
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut out = HookMatch {
        event: event.clone(),
        tool_name: tool_name.clone(),
        session_id: session_id.clone(),
        terms: Vec::new(),
        matched: Vec::new(),
        skipped: false,
        is_prompt_summary: false,
        domain_rules: Vec::new(),
    };

    // Fast exit for tools that never need guardrails
    if !tool_name.is_empty() && guardrails::should_skip_tool(&tool_name) {
        out.skipped = true;
        return Ok(out);
    }

    let tool_input = hook
        .get("tool_input")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    let mut terms = guardrails::extract_terms(&tool_name, &tool_input);

    // PostToolUse: sniff results but don't mutate session state here (that's a
    // side effect the hook handler owns, not the scenario runner)
    if event == "PostToolUse" {
        if let Some(result) = hook.get("tool_result").and_then(|v| v.as_str()) {
            guardrails::sniff_content_for_tools_pub(result, &mut terms);
        }
        terms.sort();
        terms.dedup();
    }

    let is_timing_event = event == "UserPromptSubmit";
    if terms.is_empty() && !is_timing_event {
        return Ok(out);
    }

    guardrails::enrich_terms_from_graph(&mut terms, &tool_name, &tool_input, db);
    out.terms = terms.clone();

    let all_guardrails = db.load_guardrails().map_err(|e| e.to_string())?;

    // UserPromptSubmit: brief summary of active domain guardrails
    if event == "UserPromptSubmit" {
        let domain_rules: Vec<Guardrail> = all_guardrails
            .iter()
            .filter(|g| {
                if let Ok(Some(intent)) = db.get_rule_intent(g.triple_id) {
                    intent.timing == crate::intent::Timing::ToolCall
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        out.is_prompt_summary = true;
        out.domain_rules = domain_rules;
        return Ok(out);
    }

    let matched = guardrails::match_guardrails(&all_guardrails, &terms, &tool_name, &event, db);

    // Filter out rules whose prerequisites have already been met
    let matched: Vec<_> = if !session_id.is_empty() && event == "PreToolUse" {
        matched
            .into_iter()
            .filter(|(g, _)| {
                let prereqs = session::extract_prerequisite(&g.object);
                if prereqs.is_empty() {
                    true
                } else {
                    !session::prerequisite_met(&cfg.arai_base_dir, &session_id, &prereqs)
                }
            })
            .collect()
    } else {
        matched
    };

    out.matched = matched;
    Ok(out)
}

pub fn handle_stdin() -> Result<(), String> {
    let start = std::time::Instant::now();
    // Read up to MAX_HOOK_INPUT_BYTES + 1 so we can distinguish "natural EOF"
    // from "hit the cap mid-stream".  Reject overruns rather than silently
    // truncating the JSON (a partial JSON would parse-fail anyway, but being
    // explicit gives a clearer error and prevents memory exhaustion by a
    // hostile pipe before the parse step).
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    std::io::stdin()
        .lock()
        .take(MAX_HOOK_INPUT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Failed to read stdin: {e}"))?;
    if buf.len() as u64 > MAX_HOOK_INPUT_BYTES {
        return Err(format!(
            "Hook input exceeded {MAX_HOOK_INPUT_BYTES}-byte cap"
        ));
    }
    let input = String::from_utf8(buf)
        .map_err(|e| format!("Hook input was not valid UTF-8: {e}"))?;

    let hook: Value = serde_json::from_str(&input)
        .map_err(|e| format!("Invalid hook JSON: {e}"))?;

    let tool_name = hook
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let event = hook
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("PreToolUse");
    let session_id = hook
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Fast exit mirrors match_hook — avoids loading config/db for skipped tools
    if !tool_name.is_empty() && guardrails::should_skip_tool(tool_name) {
        return Ok(());
    }

    // PostToolUse still has a side effect: it records the call into session
    // state for prerequisite tracking.  Do that *before* match_hook so scenarios
    // running through the same path don't corrupt real sessions.
    if event == "PostToolUse" && !session_id.is_empty() {
        if let Ok(cfg) = config::Config::load() {
            let tool_input = hook
                .get("tool_input")
                .cloned()
                .unwrap_or(Value::Object(serde_json::Map::new()));
            let mut terms = guardrails::extract_terms(tool_name, &tool_input);
            if let Some(result) = hook.get("tool_result").and_then(|v| v.as_str()) {
                guardrails::sniff_content_for_tools_pub(result, &mut terms);
            }
            terms.sort();
            terms.dedup();
            session::record_tool_call(&cfg.arai_base_dir, session_id, tool_name, &terms);

            // Compliance tracking: correlate this PostToolUse against any
            // recent PreToolUse firings in the same session and emit one
            // Compliance audit entry per rule.  Done here (not in match_hook)
            // because scenario replays should not pollute the audit log.
            let preview = summarize_tool_input(tool_name, &tool_input);
            compliance::record_post_compliance(&cfg, session_id, tool_name, &terms, &preview);
        }
    }

    let cfg = config::Config::load()?;
    let db_path = cfg.db_path();
    if !db_path.exists() {
        return Ok(());
    }
    let db = store::Store::open(&db_path)?;

    let result = match_hook(&hook, &cfg, &db)?;
    if result.skipped {
        return Ok(());
    }

    // UserPromptSubmit summary
    if result.is_prompt_summary {
        if result.domain_rules.is_empty() {
            return Ok(());
        }
        let mut subjects: Vec<String> = result.domain_rules.iter().map(|g| g.subject.clone()).collect();
        subjects.sort();
        subjects.dedup();
        let summary = format!(
            "Arai: {} active guardrail(s) for: {}. Rules will fire on relevant tool calls.",
            result.domain_rules.len(),
            subjects.join(", ")
        );
        let response = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": summary
            }
        });
        println!("{}", serde_json::to_string(&response).map_err(|e| e.to_string())?);
        return Ok(());
    }

    if result.matched.is_empty() {
        return Ok(());
    }

    // Telemetry — aggregate counters only
    let latency = start.elapsed().as_millis();
    crate::telemetry::track_hook_latency(&cfg.arai_base_dir, &result.event, latency, true);
    for (g, pct) in &result.matched {
        crate::telemetry::track_rule_fired(&cfg.arai_base_dir, &g.subject, &g.predicate, &result.tool_name, &result.event, *pct);
    }

    // Decide whether to deny before writing the audit line so the audit log
    // reflects the actual outcome the hook emitted.
    let top_severity = highest_severity(&result.matched, &db);
    let is_pretooluse = result.event == "PreToolUse";
    let deny_enabled = deny_mode_enabled();
    let blocking = is_pretooluse && top_severity == Severity::Block && deny_enabled;

    // Local audit log — records every firing for `arai audit` / `arai stats`
    let tool_input = hook
        .get("tool_input")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let prompt_preview = summarize_tool_input(&result.tool_name, &tool_input);
    let decision = match (result.event.as_str(), blocking) {
        ("PreToolUse", true) => "deny",
        ("PreToolUse", false) => "inject",
        ("PostToolUse", _) => "review",
        (other, _) => other,
    };
    audit::record_firing(
        &cfg,
        &result.event,
        &result.tool_name,
        &result.session_id,
        &prompt_preview,
        &result.matched,
        decision,
        Some(&db),
    );

    let context = guardrails::format_context(&result.matched);
    let response = match (result.event.as_str(), blocking) {
        ("PreToolUse", true) => {
            let reason = deny_reason(&result.matched, &db);
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": reason,
                    "additionalContext": context,
                }
            })
        }
        ("PostToolUse", _) => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": format!("[Post-action review] {context}")
            }
        }),
        _ => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "additionalContext": context
            }
        }),
    };

    println!("{}", serde_json::to_string(&response).map_err(|e| e.to_string())?);
    Ok(())
}

/// Build a short deny reason Claude Code surfaces to the user.  Prefers the
/// first `Block`-severity rule (or predicate-derived fallback) and quotes its
/// source so the decision is auditable at a glance.
fn deny_reason(matched: &[(Guardrail, u8)], db: &Store) -> String {
    for (g, _) in matched {
        let sev = db
            .get_rule_intent(g.triple_id)
            .ok()
            .flatten()
            .map(|i| i.severity)
            .unwrap_or_else(|| Severity::from_predicate(&g.predicate));
        if sev == Severity::Block {
            let src = if g.file_path.is_empty() { &*g.source_file } else { &*g.file_path };
            return format!(
                "Arai: \"{subj} {pred} {obj}\" [from {src}]",
                subj = g.subject,
                pred = g.predicate,
                obj = g.object,
            );
        }
    }
    // Shouldn't reach here if highest_severity returned Block, but guard for
    // robustness.
    "Arai: blocking rule matched".to_string()
}

/// Produce a short human-readable preview of tool input for the audit log.
/// Prefers the most-informative field per tool; truncates + strips newlines.
fn summarize_tool_input(tool_name: &str, input: &Value) -> String {
    let raw = match tool_name {
        "Bash" => input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "Edit" | "Write" | "MultiEdit" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{tool_name} {path}")
        }
        _ => input.to_string(),
    };
    let oneline = raw.replace(['\n', '\r'], " ");
    let trimmed = oneline.trim();
    if trimmed.chars().count() <= 200 {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(200).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::RuleIntent;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_db() -> (Store, PathBuf) {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("arai_hooks_test_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let store = Store::open(&db_path).unwrap();
        (store, dir)
    }

    fn mk_guardrail(id: i64, subject: &str, predicate: &str, object: &str) -> Guardrail {
        Guardrail {
            triple_id: id,
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            confidence: 0.9,
            source_file: "CLAUDE.md".to_string(),
            file_path: "CLAUDE.md".to_string(),
            layer: Some(1),
            line_start: Some(42),
            expires_at: None,
        }
    }

    #[test]
    fn test_deny_mode_env_toggle() {
        // Default on (env unset) — guard value may leak from the running test
        // harness, so assert the positive and negative values explicitly.
        for (val, expected) in [
            ("on", true),
            ("", true),
            ("off", false),
            ("0", false),
            ("false", false),
            ("no", false),
            ("yes", true),
        ] {
            std::env::set_var(DENY_MODE_ENV, val);
            assert_eq!(
                deny_mode_enabled(),
                expected,
                "ARAI_DENY_MODE={val:?} expected {expected}"
            );
        }
        std::env::remove_var(DENY_MODE_ENV);
        assert!(deny_mode_enabled(), "unset ARAI_DENY_MODE should enable deny mode");
    }

    #[test]
    fn test_highest_severity_picks_block() {
        let (store, dir) = temp_db();
        let matched = vec![
            (mk_guardrail(1, "alembic", "prefers", "autogenerate"), 90u8),
            (mk_guardrail(2, "git", "never", "force-push to main"), 100u8),
            (mk_guardrail(3, "cargo", "always", "test before commit"), 80u8),
        ];
        // No rule_intent rows → falls back to predicate-derived severity.
        assert_eq!(highest_severity(&matched, &store), Severity::Block);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_highest_severity_respects_store_override() {
        let (store, dir) = temp_db();
        // Seed a rule via the public API, then override its intent with Inform
        // severity even though the predicate ("never") would normally derive Block.
        let triple = crate::parser::Triple {
            subject: "noisy".to_string(),
            predicate: "never".to_string(),
            object: "something minor".to_string(),
            confidence: 0.9,
            domain: "test".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: Some(1),
            expires_at: None,
        };
        store.upsert_file("CLAUDE.md", "x", &[triple], "test").unwrap();
        let guardrails = store.load_guardrails().unwrap();
        let tid = guardrails[0].triple_id;

        let intent = RuleIntent {
            action: crate::intent::Action::General,
            timing: crate::intent::Timing::ToolCall,
            tools: vec!["*".to_string()],
            allow_inverse: false,
            enriched_by: "manual".to_string(),
            severity: Severity::Inform,
        };
        store.upsert_rule_intent(tid, &intent).unwrap();

        let matched = vec![(mk_guardrail(tid, "noisy", "never", "something minor"), 100u8)];
        // Store override demotes the Block to Inform.
        assert_eq!(highest_severity(&matched, &store), Severity::Inform);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_deny_reason_mentions_source_and_rule() {
        let (store, dir) = temp_db();
        let matched = vec![(mk_guardrail(1, "git", "never", "force-push to main"), 100u8)];
        let reason = deny_reason(&matched, &store);
        assert!(reason.contains("never"), "reason should quote the predicate: {reason:?}");
        assert!(reason.contains("force-push"), "reason should quote the object: {reason:?}");
        assert!(reason.contains("CLAUDE.md"), "reason should cite source: {reason:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_deny_reason_skips_non_blocking() {
        let (store, dir) = temp_db();
        let matched = vec![
            (mk_guardrail(1, "cargo", "prefers", "small commits"), 70u8),
            (mk_guardrail(2, "git", "never", "force-push to main"), 100u8),
        ];
        let reason = deny_reason(&matched, &store);
        // Should pick the `never` rule, not the `prefers` rule.
        assert!(reason.contains("force-push"), "picked wrong rule: {reason:?}");
        assert!(!reason.contains("small commits"), "non-block rule leaked: {reason:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
