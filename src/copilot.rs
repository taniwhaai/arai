//! GitHub Copilot command hooks (docs.github.com/copilot/reference/hooks-reference).
//!
//! Copilot's camelCase payload is not the Claude envelope. `preToolUse` carries
//! `toolName` and `toolArgs` (an object or a JSON string) and no event name.
//! The decision is a flat `permissionDecision`. This module rewrites that
//! payload into the canonical envelope and formats Copilot's responses.
//!
//! An explicit `permissionDecision: allow` suppresses Copilot's own permission
//! prompt, so a non-blocking result stays silent. Deny is the only PreToolUse
//! answer this module emits.

use serde_json::{json, Map, Value};

/// Stamped by [`canonicalize`] so host detection does not depend on fields
/// the rewrite removes (`toolArgs`).
pub const MARKER: &str = "copilot_hook";

/// True once a payload is known to come from Copilot.
pub fn is_copilot_payload(hook: &Value) -> bool {
    hook.get(MARKER).is_some() || is_copilot_shape(hook)
}

/// True when [`canonicalize`] still has work to do.
pub fn needs_canonicalize(hook: &Value) -> bool {
    is_copilot_shape(hook) && hook.get(MARKER).is_none()
}

/// Native camelCase shape. PascalCase Claude and Cursor payloads carry
/// `hook_event_name`; Grok's native payload carries `toolInput`.
fn is_copilot_shape(hook: &Value) -> bool {
    let Some(object) = hook.as_object() else {
        return false;
    };
    if object.contains_key("hook_event_name")
        || object.contains_key("hookEventName")
        || object.contains_key("cursor_version")
        || object.contains_key("tool_input")
        || object.contains_key("toolInput")
        || object.contains_key(MARKER)
    {
        return false;
    }
    if object.contains_key("toolName") && object.contains_key("toolArgs") {
        return true;
    }
    // sessionStart has no tool fields. `source` is startup, resume, or new.
    object.contains_key("sessionId")
        && matches!(
            object.get("source").and_then(Value::as_str),
            Some("startup" | "resume" | "new")
        )
}

fn canonical_tool(name: &str) -> String {
    // Names from the hooks reference tool table. Short names such as
    // `create` and `edit` stay here so the shared normalizer, which already
    // maps `bash` for every host, does not treat those words as universal.
    let mapped = match name {
        "bash" | "powershell" => "Bash",
        "view" => "Read",
        "create" => "Write",
        "edit" | "str_replace_editor" | "apply_patch" => "Edit",
        "grep" | "rg" => "Grep",
        "glob" => "Glob",
        "task" | "Task" => "Agent",
        "web_fetch" => "WebFetch",
        "web_search" => "WebSearch",
        "ask_user" => "AskUserQuestion",
        "update_todo" => "TodoWrite",
        other => return crate::guardrails::normalize_tool_name(other),
    };
    mapped.to_string()
}

fn infer_event(object: &Map<String, Value>) -> &'static str {
    if object.contains_key("toolResult") || object.get("error").and_then(Value::as_str).is_some() {
        // postToolUseFailure has `error` and no `toolResult`. The tool
        // already finished, so this is an observation, not a second gate.
        "PostToolUse"
    } else if object.contains_key("toolName") || object.contains_key("toolArgs") {
        "PreToolUse"
    } else if object.contains_key("source") {
        "SessionStart"
    } else {
        "Notification"
    }
}

fn alias_file_fields(input: &mut Map<String, Value>) {
    if !input.contains_key("file_path") {
        if let Some(path) = [
            "path",
            "filePath",
            "file_path",
            "file",
            "target_file",
            "targetPath",
        ]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
        {
            input.insert("file_path".into(), Value::String(path));
        }
    }
    if !input.contains_key("content") {
        if let Some(content) = [
            "contents",
            "content",
            "file_text",
            "text",
            "new_string",
            "new_str",
            "code",
        ]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
        {
            input.insert("content".into(), Value::String(content));
        }
    }
    // The published CLI `edit` tool names the previous body `old_str`.
    // Content sniffing reads `old_string`, not `old_str`.
    if !input.contains_key("old_string") {
        if let Some(previous) = input
            .get("old_str")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            input.insert("old_string".into(), Value::String(previous));
        }
    }
}

fn tool_input_from_args(tool: &str, args: Value) -> Value {
    let mut object = match args {
        Value::Object(map) => map,
        Value::String(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Object(map)) => map,
            _ if tool == "Bash" => return json!({ "command": raw }),
            _ => return json!({}),
        },
        _ => return json!({}),
    };
    if matches!(tool, "Write" | "Edit" | "Read") {
        alias_file_fields(&mut object);
    }
    Value::Object(object)
}

/// Rewrite a Copilot payload in place. Returns false when it is not
/// Copilot-shaped or has already been rewritten.
pub fn canonicalize(hook: &mut Value) -> bool {
    if !needs_canonicalize(hook) {
        return false;
    }
    let Some(object) = hook.as_object_mut() else {
        return false;
    };
    let event = infer_event(object);
    let tool_name = object
        .get("toolName")
        .and_then(Value::as_str)
        .map(canonical_tool);
    let tool_args = object.remove("toolArgs");
    if let Some(name) = tool_name.clone() {
        object.insert("tool_name".into(), Value::String(name));
    }
    if let (Some(name), Some(args)) = (tool_name, tool_args) {
        object.insert("tool_input".into(), tool_input_from_args(&name, args));
    }
    if !object.contains_key("session_id") {
        if let Some(id) = object.get("sessionId").cloned() {
            object.insert("session_id".into(), id);
        }
    }
    object.insert("hook_event_name".into(), Value::String(event.into()));
    object.insert(MARKER.into(), Value::Bool(true));
    true
}

/// Deny response. `permissionDecisionReason` is required when denying.
pub fn deny(reason: &str) -> Value {
    json!({
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    })
}

/// Context for `sessionStart` and `postToolUse`. Copilot reads
/// `additionalContext` on those events. PreToolUse command hooks do not
/// document that field, and an explicit allow would skip Copilot's own prompt.
pub fn additional_context(text: &str) -> Value {
    json!({ "additionalContext": text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_string_args_become_a_command() {
        let mut hook = json!({
            "sessionId": "s1",
            "cwd": "/tmp/project",
            "toolName": "bash",
            "toolArgs": "{\"command\":\"cargo clean --help\",\"description\":\"probe\"}"
        });
        assert!(canonicalize(&mut hook));
        assert!(!needs_canonicalize(&hook));
        assert!(is_copilot_payload(&hook));
        assert_eq!(hook["hook_event_name"], "PreToolUse");
        assert_eq!(hook["tool_name"], "Bash");
        assert_eq!(hook["session_id"], "s1");
        assert_eq!(hook["tool_input"]["command"], "cargo clean --help");
        let again = hook.clone();
        assert!(!canonicalize(&mut hook));
        assert_eq!(hook, again);
    }

    #[test]
    fn create_path_becomes_write() {
        let mut hook = json!({
            "sessionId": "s1",
            "toolName": "create",
            "toolArgs": { "path": "migrations/001.py", "contents": "from alembic import op\n" }
        });
        assert!(canonicalize(&mut hook));
        assert_eq!(hook["tool_name"], "Write");
        assert_eq!(hook["tool_input"]["file_path"], "migrations/001.py");
        assert_eq!(hook["tool_input"]["content"], "from alembic import op\n");
    }

    #[test]
    fn edit_cli_str_fields_become_sniffable() {
        let mut hook = json!({
            "sessionId": "s1",
            "toolName": "edit",
            "toolArgs": {
                "path": "src/main.rs",
                "old_str": "fn old() {}",
                "new_str": "from alembic import op\n"
            }
        });
        assert!(canonicalize(&mut hook));
        assert_eq!(hook["tool_name"], "Edit");
        assert_eq!(hook["tool_input"]["file_path"], "src/main.rs");
        assert_eq!(hook["tool_input"]["content"], "from alembic import op\n");
        assert_eq!(hook["tool_input"]["old_string"], "fn old() {}");
    }

    #[test]
    fn session_start_has_no_tool_fields() {
        let mut hook = json!({
            "sessionId": "s1",
            "cwd": "/tmp/project",
            "source": "startup"
        });
        assert!(canonicalize(&mut hook));
        assert_eq!(hook["hook_event_name"], "SessionStart");
        assert!(hook.get("tool_name").is_none());
    }

    #[test]
    fn other_hosts_are_left_alone() {
        let samples = [
            json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": "ls"}}),
            json!({"hookEventName": "pre_tool_use", "toolName": "run_terminal_command", "toolInput": {"command": "ls"}}),
            json!({"hook_event_name": "preToolUse", "cursor_version": "1", "tool_name": "Shell", "tool_input": {"command": "ls"}}),
        ];
        for mut sample in samples {
            let before = sample.clone();
            assert!(!canonicalize(&mut sample));
            assert!(!is_copilot_payload(&sample));
            assert_eq!(sample, before);
        }
    }

    #[test]
    fn apply_patch_becomes_edit_and_a_failure_is_post_tool_use() {
        let mut patch = json!({
            "sessionId": "s1",
            "toolName": "apply_patch",
            "toolArgs": { "path": "src/main.rs", "contents": "fn main() {}\n" }
        });
        assert!(canonicalize(&mut patch));
        assert_eq!(patch["tool_name"], "Edit");
        assert_eq!(patch["tool_input"]["file_path"], "src/main.rs");
        assert!(patch.get("toolArgs").is_none());

        let mut failed = json!({
            "sessionId": "s1",
            "toolName": "bash",
            "toolArgs": { "command": "ls" },
            "error": "command failed"
        });
        assert!(canonicalize(&mut failed));
        assert_eq!(failed["hook_event_name"], "PostToolUse");
        assert_eq!(failed["tool_name"], "Bash");
    }

    #[test]
    fn deny_is_flat_and_has_no_allow_helper() {
        let deny = deny("Arai: blocked");
        assert_eq!(deny["permissionDecision"], "deny");
        assert_eq!(deny["permissionDecisionReason"], "Arai: blocked");
        assert!(deny.get("hookSpecificOutput").is_none());
        assert!(deny.get("permission").is_none());
    }
}
