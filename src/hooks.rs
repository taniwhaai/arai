use crate::config::Config;
use crate::store::{Guardrail, Store};
use crate::{audit, config, guardrails, session, store};
use serde_json::Value;
use std::io::Read;

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
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("Failed to read stdin: {e}"))?;

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

    // Local audit log — records every firing for `arai audit` / `arai stats`
    let tool_input = hook
        .get("tool_input")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let prompt_preview = summarize_tool_input(&result.tool_name, &tool_input);
    let decision = match result.event.as_str() {
        "PreToolUse" => "inject",
        "PostToolUse" => "review",
        other => other,
    };
    audit::record_firing(
        &cfg,
        &result.event,
        &result.tool_name,
        &result.session_id,
        &prompt_preview,
        &result.matched,
        decision,
    );

    let context = guardrails::format_context(&result.matched);
    let response = match result.event.as_str() {
        "PostToolUse" => serde_json::json!({
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
