use crate::{config, guardrails, session, store};
use serde_json::Value;
use std::io::Read;

pub fn handle_stdin() -> Result<(), String> {
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

    // Filter out rules whose prerequisites have already been met
    let matched: Vec<_> = if !session_id.is_empty() && event == "PreToolUse" {
        matched
            .into_iter()
            .filter(|g| {
                let prereqs = session::extract_prerequisite(&g.object);
                if prereqs.is_empty() {
                    true // No prerequisite → always fire
                } else {
                    // Prerequisite exists → only fire if NOT met
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
