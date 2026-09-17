//! Pure translation of Cursor's native hook contract into Arai file/tool
//! actions. Cursor collapses creation and editing into Write, so a validated
//! Write is checked in both scopes. Delete supplies path-only Edit coverage;
//! it does not introduce deletion-specific policy semantics.
use serde_json::{json, Map, Value};

pub(crate) fn event(raw: &str) -> Option<&'static str> {
    match raw {
        "preToolUse" => Some("PreToolUse"),
        "postToolUse" => Some("PostToolUse"),
        _ => None,
    }
}

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Cursor {field} must be a string"))
}

fn valid_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() || path.chars().any(char::is_control) {
        return Err("Cursor paths must be nonempty strings without control characters".into());
    }
    Ok(())
}

fn aliases_agree(value: &Value, fields: &[&str]) -> Result<(), String> {
    let mut first = None;
    for field in fields {
        if let Some(actual) = value.get(*field) {
            if first.is_some_and(|expected| expected != actual) {
                return Err(format!("Conflicting Cursor aliases: {}", fields.join(", ")));
            }
            first = Some(actual);
        }
    }
    Ok(())
}

fn require_known_fields(input: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(field) = input
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("Unsupported Cursor mutation field: {field}"));
    }
    Ok(())
}

/// Response property names describe structure, not observed tool content.
/// Only string values can supply lexical evidence; separators keep distinct
/// values from accidentally forming one word.
fn response_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text);
        }
        Value::Array(values) => {
            for value in values {
                response_text(value, out);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                response_text(value, out);
            }
        }
        _ => {}
    }
}

fn file_input(input: &Value, write: bool) -> Result<Value, String> {
    let object = input
        .as_object()
        .ok_or("Cursor tool_input must be an object")?;
    require_known_fields(
        object,
        if write {
            &[
                "file_path",
                "path",
                "content",
                "old_string",
                "new_string",
                "edits",
            ]
        } else {
            &["file_path", "path"]
        },
    )?;
    let path = string(input, "file_path")?;
    valid_path(path)?;
    aliases_agree(input, &["file_path", "path"])?;
    let mut normalized = json!({"file_path":path});
    if !write {
        return Ok(normalized);
    }

    let mut has_shape = false;
    if input.get("content").is_some() {
        normalized["content"] = Value::String(string(input, "content")?.into());
        has_shape = true;
    }
    if input.get("old_string").is_some() || input.get("new_string").is_some() {
        normalized["old_string"] = Value::String(string(input, "old_string")?.into());
        normalized["new_string"] = Value::String(string(input, "new_string")?.into());
        has_shape = true;
    }
    if let Some(edits) = input.get("edits") {
        let edits = edits.as_array().ok_or("Cursor edits must be an array")?;
        if edits.is_empty() {
            return Err("Cursor edits must contain at least one validated replacement".into());
        }
        for edit in edits {
            let object = edit.as_object().ok_or("Cursor edit must be an object")?;
            require_known_fields(object, &["old_string", "new_string"])?;
            string(edit, "old_string")?;
            string(edit, "new_string")?;
        }
        normalized["edits"] = Value::Array(edits.clone());
        has_shape = true;
    }
    if !has_shape {
        return Err(
            "Unsupported Cursor Write shape: expected content, replacement strings, or edits"
                .into(),
        );
    }
    Ok(normalized)
}

/// Does no IO and consults no environment. Expected event is supplied by the
/// owned registration; it cannot be overridden by fields in the input payload.
pub(crate) fn normalize(hook: &Value, expected_event: Option<&str>) -> Result<Vec<Value>, String> {
    if !hook.is_object() {
        return Err("Cursor hook must be a JSON object".into());
    }
    for fields in [
        ["hook_event_name", "hookEventName"],
        ["tool_name", "toolName"],
        ["tool_input", "toolInput"],
    ] {
        aliases_agree(hook, &fields)?;
    }
    let raw_event = string(hook, "hook_event_name")?;
    let canonical_event = event(raw_event).ok_or("Unsupported Cursor hook event")?;
    if let Some(expected) = expected_event {
        if event(expected).is_none() || expected != raw_event {
            return Err("Cursor hook event does not match its registration".into());
        }
    }
    let tool = string(hook, "tool_name")?;
    if tool.trim() != tool || tool.is_empty() || tool.chars().any(char::is_control) {
        return Err("Cursor tool_name must be nonempty and contain no control characters".into());
    }
    let input = hook
        .get("tool_input")
        .filter(|input| input.is_object())
        .ok_or("Cursor tool_input must be an object")?;

    let mut base = json!({"hook_event_name":canonical_event});
    for field in [
        "conversation_id",
        "generation_id",
        "tool_use_id",
        "cursor_version",
        "workspace_roots",
        "model",
        "model_id",
        "model_params",
        "transcript_path",
        "duration",
    ] {
        if let Some(value) = hook.get(field) {
            base[field] = value.clone();
        }
    }
    if let Some(cwd) = hook.get("cwd") {
        let cwd = cwd.as_str().ok_or("Cursor cwd must be a string")?;
        valid_path(cwd)?;
        base["cwd"] = Value::String(cwd.into());
    }
    if let Some(conversation) = hook.get("conversation_id") {
        let conversation = conversation
            .as_str()
            .ok_or("Cursor conversation_id must be a string")?;
        let session = format!("cursor-{conversation}");
        if !crate::session::valid_session_id(conversation)
            || !crate::session::valid_session_id(&session)
        {
            return Err("Cursor conversation_id is not a safe session identifier".into());
        }
        base["session_id"] = Value::String(session);
    }
    for field in ["generation_id", "tool_use_id"] {
        if let Some(value) = hook.get(field) {
            let value = value
                .as_str()
                .ok_or_else(|| format!("Cursor {field} must be a string"))?;
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err(format!("Cursor {field} must be a nonempty identifier"));
            }
        }
    }
    if canonical_event == "PostToolUse" {
        let parsed: Value = serde_json::from_str(string(hook, "tool_output")?)
            .map_err(|error| format!("Cursor tool_output must contain JSON: {error}"))?;
        let mut text = String::new();
        response_text(&parsed, &mut text);
        base["tool_response"] = Value::String(text);
        base["cursor_tool_output"] = parsed;
    }

    let actions = match tool {
        "Shell" => {
            require_known_fields(
                input.as_object().unwrap(),
                &[
                    "command",
                    "cmd",
                    "working_directory",
                    "workdir",
                    "cwd",
                    "description",
                    "timeout",
                ],
            )?;
            let command = string(input, "command")?;
            if command.contains('\0') {
                return Err("Cursor command must not contain NUL bytes".into());
            }
            aliases_agree(input, &["command", "cmd"])?;
            aliases_agree(input, &["working_directory", "workdir", "cwd"])?;
            if input.get("description").is_some() {
                string(input, "description")?;
            }
            if let Some(timeout) = input.get("timeout") {
                if !timeout.as_f64().is_some_and(|timeout| timeout >= 0.0) {
                    return Err("Cursor timeout must be a nonnegative number".into());
                }
            }
            let mut normalized = json!({"command": command});
            if let Some(cwd) = input
                .get("working_directory")
                .or_else(|| input.get("workdir"))
                .or_else(|| input.get("cwd"))
            {
                let cwd = cwd
                    .as_str()
                    .ok_or("Cursor working_directory must be a string")?;
                valid_path(cwd)?;
                normalized["workdir"] = Value::String(cwd.into());
            }
            vec![("Bash", normalized)]
        }
        "Write" => {
            let normalized = file_input(input, true)?;
            vec![("Write", normalized.clone()), ("Edit", normalized)]
        }
        "Delete" => vec![("Edit", file_input(input, false)?)],
        "Read" | "Grep" => vec![(tool, input.clone())],
        "Task" => vec![("Agent", input.clone())],
        name if name.starts_with("MCP:") && !name[4..].trim().is_empty() => {
            vec![(name, input.clone())]
        }
        _ => return Err(format!("Unsupported Cursor tool: {tool}")),
    };
    Ok(actions
        .into_iter()
        .map(|(tool, input)| {
            let mut action = base.clone();
            action["tool_name"] = Value::String(tool.into());
            action["tool_input"] = input;
            action
        })
        .collect())
}

pub(crate) fn response(event: &str, blocking: bool, reason: Option<&str>, context: &str) -> Value {
    if event == "postToolUse" || event == "PostToolUse" {
        return json!({"additional_context":context});
    }
    if blocking {
        let reason = reason
            .filter(|reason| !reason.is_empty())
            .unwrap_or("Arai blocked this action.");
        let agent_message = if context.is_empty() {
            reason.to_string()
        } else {
            format!("{reason}\n{context}")
        };
        json!({"permission":"deny", "user_message":reason, "agent_message":agent_message})
    } else {
        // Cursor does not document advisory context injection on allow.
        json!({"permission":"allow"})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(tool: &str, input: Value) -> Value {
        json!({"hook_event_name":"preToolUse","conversation_id":"conversation-1",
            "generation_id":"generation-1","tool_use_id":"tool-1","cwd":"/project",
            "tool_name":tool,"tool_input":input})
    }

    #[test]
    fn native_shell_fixture_keeps_identity_and_normalizes_working_directory() {
        let hook: Value =
            serde_json::from_str(include_str!("../tests/fixtures/cursor/pre-shell.json")).unwrap();
        let original = hook.clone();
        let actions = normalize(&hook, Some("preToolUse")).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["hook_event_name"], "PreToolUse");
        assert_eq!(actions[0]["tool_name"], "Bash");
        assert_eq!(
            actions[0]["tool_input"],
            json!({"command":"cargo clean","workdir":"/project/packages/api"})
        );
        assert_eq!(actions[0]["session_id"], "cursor-conversation-1");
        assert_eq!(actions[0]["generation_id"], "generation-1");
        assert_eq!(actions[0]["tool_use_id"], "tool-1");
        assert_eq!(hook, original);
    }

    #[test]
    fn native_post_fixture_parses_structured_output() {
        let hook: Value =
            serde_json::from_str(include_str!("../tests/fixtures/cursor/post-shell.json")).unwrap();
        let actions = normalize(&hook, Some("postToolUse")).unwrap();
        assert_eq!(actions[0]["hook_event_name"], "PostToolUse");
        assert_eq!(
            actions[0]["cursor_tool_output"],
            json!({"exitCode":0,"stdout":"cargo finished"})
        );
        assert_eq!(actions[0]["tool_response"], "cargo finished");
        for bad in [Value::Null, json!({"stdout":"text"}), json!("not JSON")] {
            let mut malformed = hook.clone();
            malformed["tool_output"] = bad;
            assert!(normalize(&malformed, None).is_err());
        }
    }

    #[test]
    fn response_property_names_are_not_lexical_content_or_session_evidence() {
        let mut hook = pre("Shell", json!({"command":"echo done"}));
        hook["hook_event_name"] = json!("postToolUse");
        let parsed = json!({"alembic":true,"docker":42,"cargo":null,
            "details":[false, {"review":"plain output"}, "from pytest import fixture"]});
        hook["tool_output"] = json!(parsed.to_string());
        let normalized = normalize(&hook, None).unwrap();
        let output = normalized[0]["tool_response"].as_str().unwrap();
        let mut terms = Vec::new();
        crate::guardrails::sniff_content_for_tools(output, &mut terms);
        assert!(terms.iter().any(|term| term == "pytest"), "{terms:?}");
        for key in ["alembic", "docker", "cargo", "review"] {
            assert!(
                !output.contains(key),
                "property name leaked into content: {output}"
            );
            assert!(!terms.iter().any(|term| term == key), "{terms:?}");
        }
        assert_eq!(normalized[0]["cursor_tool_output"], parsed);
    }

    #[test]
    fn shell_accepts_metadata_but_rejects_unrecognized_execution_fields() {
        let input = json!({"command":"cargo check","description":"Check source","timeout":30,
            "working_directory":"/project","workdir":"/project","cwd":"/project"});
        let normalized = normalize(&pre("Shell", input), None).unwrap();
        assert_eq!(
            normalized[0]["tool_input"],
            json!({"command":"cargo check","workdir":"/project"})
        );
        for (field, value) in [
            ("commands", json!(["cargo clean"])),
            ("script", json!("cargo clean")),
            ("args", json!(["cargo", "clean"])),
            ("stdin", json!("cargo clean")),
            ("executable", json!("other-shell")),
            ("env", json!({"COMMAND":"cargo clean"})),
            ("timeout", json!("30")),
            ("description", Value::Null),
        ] {
            let mut input = json!({"command":"safe"});
            input[field] = value;
            assert!(normalize(&pre("Shell", input), None).is_err(), "{field}");
        }
    }

    #[test]
    fn malformed_paths_are_rejected_without_io() {
        for path in ["", "  ", "a\0b", "a\nb"] {
            for tool in ["Write", "Delete"] {
                let input = if tool == "Write" {
                    json!({"file_path":path,"content":""})
                } else {
                    json!({"file_path":path})
                };
                assert!(
                    normalize(&pre(tool, input), None).is_err(),
                    "{tool}: {path:?}"
                );
            }
            assert!(normalize(
                &pre("Shell", json!({"command":"safe","working_directory":path})),
                None
            )
            .is_err());
            let mut hook = pre("Shell", json!({"command":"safe"}));
            hook["cwd"] = json!(path);
            assert!(normalize(&hook, None).is_err());
        }
        assert!(normalize(
            &pre(
                "Write",
                json!({"file_path":"folder with spaces/x.py","content":""})
            ),
            None
        )
        .is_ok());
    }

    #[test]
    fn write_checks_both_scopes_and_keeps_all_content() {
        for input in [
            json!({"file_path":"alembic/x.py","content":""}),
            json!({"file_path":"x.py","old_string":"from alembic import op","new_string":"pass"}),
            json!({"file_path":"x.py","edits":[{"old_string":"pass","new_string":"from alembic import op"},{"old_string":"docker","new_string":"cargo"}]}),
        ] {
            let actions = normalize(&pre("Write", input.clone()), None).unwrap();
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[0]["tool_name"], "Write");
            assert_eq!(actions[1]["tool_name"], "Edit");
            for action in actions {
                assert_eq!(action["tool_input"], input);
            }
        }
    }

    #[test]
    fn write_requires_a_recognized_complete_mutation_shape() {
        for input in [
            json!({"file_path":"x"}),
            json!({"path":"x","content":"x"}),
            json!({"file_path":"x","content":null}),
            json!({"file_path":"x","old_string":"x"}),
            json!({"file_path":"x","edits":[]}),
            json!({"file_path":"x","edits":{}}),
            json!({"file_path":"x","edits":[{"old_string":"x","new_string":null}]}),
            json!({"file_path":"x","edits":[{"old_string":"x","new_string":"y","file_path":"elsewhere"}]}),
            json!({"file_path":"x","content":"safe","files":[{"file_path":"dangerous"}]}),
            json!({"file_path":"x","path":"other","content":"x"}),
            json!({"file_path":"x","content":"x","workdir":"elsewhere"}),
        ] {
            assert!(
                normalize(&pre("Write", input.clone()), None).is_err(),
                "{input}"
            );
        }
    }

    #[test]
    fn delete_is_path_only_and_unknown_mutators_are_rejected() {
        let actions = normalize(&pre("Delete", json!({"file_path":"alembic/x.py"})), None).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["tool_name"], "Edit");
        assert_eq!(
            actions[0]["tool_input"],
            json!({"file_path":"alembic/x.py"})
        );
        for tool in [
            "StrReplace",
            "ApplyPatch",
            "Edit",
            "Bash",
            "MCP:",
            "FutureMutation",
        ] {
            assert!(normalize(&pre(tool, json!({})), None).is_err(), "{tool}");
        }
        assert!(normalize(
            &pre("Delete", json!({"file_path":"x","recursive":true})),
            None
        )
        .is_err());
    }

    #[test]
    fn documented_nonmutators_and_mcp_objects_remain_extensible() {
        for (tool, canonical) in [
            ("Read", "Read"),
            ("Grep", "Grep"),
            ("Task", "Agent"),
            ("MCP:custom_tool", "MCP:custom_tool"),
        ] {
            let input =
                json!({"custom_parameters":{"nested":[1,2]},"command":"not shell execution"});
            let actions = normalize(&pre(tool, input.clone()), None).unwrap();
            assert_eq!(actions[0]["tool_name"], canonical);
            assert_eq!(actions[0]["tool_input"], input);
        }
        assert!(normalize(&pre("MCP:custom_tool", json!("{\"x\":1}")), None).is_err());
    }

    #[test]
    fn event_binding_and_shell_aliases_cannot_change_the_action() {
        let hook = pre("Shell", json!({"command":"cargo clean"}));
        for expected in [
            Some("postToolUse"),
            Some("PreToolUse"),
            Some("beforeSubmitPrompt"),
            Some(""),
        ] {
            assert!(normalize(&hook, expected).is_err());
        }
        for field in ["hook_event_name", "tool_name", "tool_input"] {
            let mut malformed = hook.clone();
            malformed.as_object_mut().unwrap().remove(field);
            assert!(normalize(&malformed, None).is_err());
        }
        for input in [
            json!({}),
            json!({"command":null}),
            json!({"command":"x\0y"}),
            json!({"command":"safe","cmd":"cargo clean"}),
            json!({"command":"cargo clean","working_directory":"/safe","workdir":"/other"}),
            json!({"command":"cargo clean","working_directory":"/safe","cwd":null}),
        ] {
            assert!(
                normalize(&pre("Shell", input.clone()), None).is_err(),
                "{input}"
            );
        }
        let mut conflict = hook.clone();
        conflict["toolName"] = json!("Read");
        assert!(normalize(&conflict, None).is_err());
        assert!(normalize(&Value::Null, None).is_err());
        assert!(normalize(&json!([]), None).is_err());
    }

    #[test]
    fn session_namespace_is_safe_and_raw_session_aliases_cannot_override_it() {
        for id in ["", "../escape", "other/session", "back\\slash", "bad\0id"] {
            let mut hook = pre("Shell", json!({"command":""}));
            hook["conversation_id"] = json!(id);
            assert!(normalize(&hook, None).is_err());
        }
        let mut hook = pre("Shell", json!({"command":""}));
        hook["conversation_id"] = json!("a".repeat(122));
        assert!(normalize(&hook, None).is_err());
        hook["conversation_id"] = json!("a".repeat(121));
        hook["session_id"] = json!("another-host-session");
        hook["skip"] = json!(true);
        let actions = normalize(&hook, None).unwrap();
        assert_eq!(actions[0]["session_id"].as_str().unwrap().len(), 128);
        assert!(actions[0].get("skip").is_none());
        hook.as_object_mut().unwrap().remove("conversation_id");
        assert!(normalize(&hook, None).unwrap()[0]
            .get("session_id")
            .is_none());
    }

    #[test]
    fn responses_follow_the_native_event_schema() {
        assert_eq!(
            response("PreToolUse", false, None, "advisory"),
            json!({"permission":"allow"})
        );
        assert_eq!(
            response("preToolUse", true, Some("blocked"), "rule detail"),
            json!({"permission":"deny","user_message":"blocked","agent_message":"blocked\nrule detail"})
        );
        assert_eq!(
            response("PostToolUse", true, Some("already ran"), "review"),
            json!({"additional_context":"review"})
        );
        assert_eq!(
            response("postToolUse", false, None, ""),
            json!({"additional_context":""})
        );
        assert_eq!(event("beforeSubmitPrompt"), None);
        assert_eq!(event("PreToolUse"), None);
    }
}
