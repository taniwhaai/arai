use crate::{audit, config, guardrails, session, store};
use serde_json::Value;
use std::io::Read;

pub fn handle_stdin() -> Result<(), String> {
    let start = std::time::Instant::now();
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("Failed to read stdin: {e}"))?;

    let hook: Value = serde_json::from_str(&input)
        .map_err(|e| format!("Invalid hook JSON: {e}"))?;

    let event = hook
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("PreToolUse");

    let tool_name = hook
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let session_id = hook
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Fast exit for tools that never need guardrails
    if !tool_name.is_empty() && guardrails::should_skip_tool(tool_name) {
        return Ok(());
    }

    let tool_input = hook
        .get("tool_input")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    let mut terms = guardrails::extract_terms(tool_name, &tool_input);

    // PostToolUse: sniff results + record the tool call in session state
    if event == "PostToolUse" {
        if let Some(result) = hook.get("tool_result").and_then(|v| v.as_str()) {
            guardrails::sniff_content_for_tools_pub(result, &mut terms);
        }
        terms.sort();
        terms.dedup();

        // Record this tool call for prerequisite tracking
        if !session_id.is_empty() {
            let cfg = config::Config::load().ok();
            if let Some(cfg) = &cfg {
                session::record_tool_call(&cfg.arai_base_dir, session_id, tool_name, &terms);
            }
        }
    }

    let is_timing_event = event == "UserPromptSubmit";
    if terms.is_empty() && !is_timing_event {
        return Ok(());
    }

    let cfg = config::Config::load()?;
    let db_path = cfg.db_path();

    if !db_path.exists() {
        return Ok(());
    }

    let db = store::Store::open(&db_path)?;

    guardrails::enrich_terms_from_graph(&mut terms, tool_name, &tool_input, &db);

    let all_guardrails = db.load_guardrails().map_err(|e| e.to_string())?;

    // UserPromptSubmit: brief summary of active domain guardrails
    if event == "UserPromptSubmit" {
        let domain_rules: Vec<_> = all_guardrails
            .iter()
            .filter(|g| {
                if let Ok(Some(intent)) = db.get_rule_intent(g.triple_id) {
                    intent.timing == crate::intent::Timing::ToolCall
                } else {
                    false
                }
            })
            .collect();

        if domain_rules.is_empty() {
            return Ok(());
        }

        let mut subjects: Vec<String> = domain_rules.iter().map(|g| g.subject.clone()).collect();
        subjects.sort();
        subjects.dedup();

        let summary = format!(
            "Arai: {} active guardrail(s) for: {}. Rules will fire on relevant tool calls.",
            domain_rules.len(),
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

    // PreToolUse/PostToolUse: match guardrails against terms
    let matched = guardrails::match_guardrails(&all_guardrails, &terms, tool_name, event, &db);

    if matched.is_empty() {
        return Ok(());
    }

    // Track rule firings + latency (queued file append, ~0.1ms)
    let latency = start.elapsed().as_millis();
    crate::telemetry::track_hook_latency(&cfg.arai_base_dir, event, latency, true);
    for (g, pct) in &matched {
        crate::telemetry::track_rule_fired(&cfg.arai_base_dir, &g.subject, &g.predicate, tool_name, event, *pct);
    }

    // Filter out rules whose prerequisites have already been met
    let matched: Vec<_> = if !session_id.is_empty() && event == "PreToolUse" {
        matched
            .into_iter()
            .filter(|(g, _)| {
                let prereqs = session::extract_prerequisite(&g.object);
                if prereqs.is_empty() {
                    true
                } else {
                    !session::prerequisite_met(&cfg.arai_base_dir, session_id, &prereqs)
                }
            })
            .collect()
    } else {
        matched
    };

    if matched.is_empty() {
        return Ok(());
    }

    // Local audit log — separate from anonymous telemetry.  Records every
    // firing the user can inspect via `arai audit`.  No network egress.
    let prompt_preview = summarize_tool_input(tool_name, &tool_input);
    let decision = match event {
        "PreToolUse" => "inject",   // current enforcement is context injection
        "PostToolUse" => "review",
        _ => event,
    };
    audit::record_firing(
        &cfg,
        event,
        tool_name,
        session_id,
        &prompt_preview,
        &matched,
        decision,
    );

    let context = guardrails::format_context(&matched);

    let response = match event {
        "PostToolUse" => {
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": format!("[Post-action review] {context}")
                }
            })
        }
        _ => {
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "additionalContext": context
                }
            })
        }
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
