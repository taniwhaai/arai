//! Cursor Agent hooks (cursor.com/docs/agent/hooks): translate the host's
//! payload into the canonical envelope the matcher understands, and format
//! Cursor's decision responses.  Pure — no filesystem or network access.
//!
//! Cursor differs from the Claude-shaped hosts in four ways this module owns:
//! event names are lowerCamel (`preToolUse`), the session key is
//! `conversation_id`, some events carry no `tool_name` (`beforeShellExecution`,
//! `afterFileEdit`) so one is synthesised, and the decision output is a flat
//! `{"permission": ...}` object rather than `hookSpecificOutput`.
//!
//! Host identity comes from the payload only.  Cursor exports
//! `CLAUDE_PROJECT_DIR` as a compatibility alias, and any CLI launched from
//! Cursor's integrated terminal inherits `CURSOR_VERSION`, so neither
//! environment variable can distinguish Cursor from Claude Code, Codex or
//! Grok running inside Cursor.  Every Cursor hook carries `cursor_version`;
//! when a payload is recognised by its event spelling instead,
//! [`canonicalize`] stamps that field so one predicate,
//! [`is_cursor_payload`], answers everywhere afterwards.
//!
//! Field names for the `Shell` tool (`command`, `working_directory`),
//! `afterFileEdit` (`file_path`, `edits[] {old_string, new_string}`) and
//! `postToolUse` (`tool_output`, a JSON-stringified result) are documented.
//! Cursor does not document `tool_input` for `Write` or `Delete`; the aliases
//! in [`alias_file_fields`] are best-effort.  A file call whose path is under
//! none of them is still evaluated fail-closed by the shared validator, and
//! the deny reason names the missing field so the shape can be reported.

use serde_json::{json, Map, Value};

/// Field every Cursor hook payload carries.  Stamped by [`canonicalize`]
/// when a payload was recognised by event spelling alone.
pub const VERSION_FIELD: &str = "cursor_version";

/// Cursor event spellings that reach a handler, mapped to the canonical
/// PascalCase event the matcher and timing gate use.  Decision-bearing
/// shell/MCP/read events all become `PreToolUse`; `afterFileEdit` is a
/// post-edit observation.  Anything else Cursor emits is passive.
fn canonical_event(raw: &str) -> Option<&'static str> {
    Some(match raw {
        "preToolUse" | "beforeShellExecution" | "beforeMCPExecution" | "beforeReadFile" => {
            "PreToolUse"
        }
        "postToolUse" | "afterFileEdit" => "PostToolUse",
        "beforeSubmitPrompt" => "UserPromptSubmit",
        "sessionStart" => "SessionStart",
        _ => return None,
    })
}

/// Cursor spells events in lowerCamel.  Canonical names are PascalCase and
/// Grok's live values are snake_case, so this never matches either.
fn is_cursor_spelling(raw: &str) -> bool {
    raw.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && !raw.contains('_')
}

/// The canonical event this payload should be rewritten to, or `None` when
/// it is not Cursor-shaped or is already canonical.  Makes
/// [`canonicalize`] idempotent: a rewritten payload has a PascalCase event
/// and is left alone on the second pass.
fn pending_event(hook: &Value) -> Option<&'static str> {
    let raw = hook.get("hook_event_name").and_then(Value::as_str)?;
    if let Some(mapped) = canonical_event(raw) {
        return Some(mapped);
    }
    // Unknown Cursor lifecycle events (workspaceOpen, afterAgentResponse,
    // ...) become a passive notification rather than the PreToolUse default
    // that would fail closed on an event with no decision surface.
    if hook.get(VERSION_FIELD).is_some() && is_cursor_spelling(raw) {
        return Some("Notification");
    }
    None
}

/// True once a payload is known to come from Cursor.  Single source of
/// truth for response shape: the host detector and the outer allow gate
/// both read this.
pub fn is_cursor_payload(hook: &Value) -> bool {
    hook.get(VERSION_FIELD).is_some()
}

/// True when [`canonicalize`] would rewrite this payload.  Lets the pure
/// matcher clone only Cursor-shaped input.
pub fn needs_canonicalize(hook: &Value) -> bool {
    pending_event(hook).is_some()
}

/// Copy undocumented-but-plausible file fields onto the canonical names the
/// matcher reads (`file_path`, `content`).  Never overwrites a value the
/// host already supplied under the canonical name.
fn alias_file_fields(input: &mut Map<String, Value>) {
    if !input.contains_key("file_path") {
        if let Some(path) = [
            "path",
            "filePath",
            "file",
            "target_file",
            "relative_workspace_path",
        ]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
        {
            input.insert("file_path".into(), Value::String(path));
        }
    }
    if !input.contains_key("content") {
        if let Some(content) = ["contents", "text", "code_edit", "new_string"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
            .map(str::to_owned)
        {
            input.insert("content".into(), Value::String(content));
        }
    }
}

/// Normalise a tool-bearing Cursor event: map Cursor's tool names onto the
/// canonical set, accept MCP params serialised as a string, and alias file
/// fields.  `Delete` has no action in the intent schema, so its path is
/// evaluated as an Edit (the same choice as Codex's `*** Delete File`).
fn normalise_tool_call(object: &mut Map<String, Value>) {
    let tool = match object.get("tool_name").and_then(Value::as_str) {
        Some("Shell") => Some("Bash"),
        Some("Delete") => Some("Edit"),
        _ => None,
    };
    if let Some(tool) = tool {
        object.insert("tool_name".into(), Value::String(tool.into()));
    }
    // Cursor documents MCP tool_input as "<json params>"; accept either an
    // object or its serialised form on every tool event.
    if let Some(Value::String(raw)) = object.get("tool_input") {
        let parsed = serde_json::from_str::<Value>(raw)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        object.insert("tool_input".into(), parsed);
    }
    let is_file_tool = matches!(
        object.get("tool_name").and_then(Value::as_str),
        Some("Write" | "Edit")
    );
    if is_file_tool {
        if let Some(input) = object.get_mut("tool_input").and_then(Value::as_object_mut) {
            alias_file_fields(input);
        }
    }
}

/// Rewrite a Cursor payload in place into the Claude-shaped envelope.
/// Returns `false` and leaves the payload untouched when it is not
/// Cursor-shaped or is already canonical.
pub fn canonicalize(hook: &mut Value) -> bool {
    let Some(event) = pending_event(hook) else {
        return false;
    };
    let Some(object) = hook.as_object_mut() else {
        return false;
    };
    let raw_event = object
        .remove("hook_event_name")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    object.insert("hook_event_name".into(), Value::String(event.into()));
    object
        .entry(VERSION_FIELD)
        .or_insert_with(|| Value::String("unknown".into()));
    if !object.contains_key("session_id") {
        if let Some(id) = object.get("conversation_id").cloned() {
            object.insert("session_id".into(), id);
        }
    }
    match raw_event.as_str() {
        "beforeShellExecution" => {
            // A missing command leaves tool_input.command null, which the
            // PreToolUse validator rejects: the decision fails closed.
            let command = object.remove("command").unwrap_or(Value::Null);
            object.insert("tool_name".into(), Value::String("Bash".into()));
            object.insert("tool_input".into(), json!({ "command": command }));
        }
        "beforeReadFile" => {
            let file_path = object.remove("file_path").unwrap_or(Value::Null);
            object.insert("tool_name".into(), Value::String("Read".into()));
            object.insert("tool_input".into(), json!({ "file_path": file_path }));
        }
        "afterFileEdit" => {
            // `edits[] {old_string, new_string}` is the MultiEdit shape the
            // file-term extractor already sniffs; pass it through untouched.
            let file_path = object.remove("file_path").unwrap_or(Value::Null);
            let edits = object.remove("edits").unwrap_or_else(|| json!([]));
            object.insert("tool_name".into(), Value::String("Edit".into()));
            object.insert(
                "tool_input".into(),
                json!({ "file_path": file_path, "edits": edits }),
            );
        }
        "preToolUse" | "postToolUse" | "beforeMCPExecution" => normalise_tool_call(object),
        _ => {}
    }
    true
}

/// Cursor deny response.  `user_message` is shown to the user and
/// `agent_message` is sent to the agent; Cursor delivers both only on deny.
pub fn deny(reason: &str, context: &str) -> Value {
    let agent_message = if context.is_empty() {
        reason.to_string()
    } else {
        format!("{reason}\n{context}")
    };
    json!({
        "permission": "deny",
        "user_message": reason,
        "agent_message": agent_message,
    })
}

/// Explicit allow.  Under `failClosed: true` Cursor treats empty stdout as
/// a failed hook and blocks, so every PreToolUse must answer.  Advisory
/// context is not attached: Cursor drops `agent_message` on allow.
pub fn allow() -> Value {
    json!({ "permission": "allow" })
}

/// Context injection for `postToolUse` and `sessionStart`.
pub fn context(text: &str) -> Value {
    json!({ "additional_context": text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_codex_and_grok_payloads_are_left_alone() {
        for event in [
            "PreToolUse",
            "PostToolUse",
            "pre_tool_use",
            "SessionStart",
            "stop",
            "notification",
        ] {
            let mut hook = json!({"hook_event_name": event, "tool_name": "Bash",
                "tool_input": {"command": "ls"}});
            let before = hook.clone();
            assert!(!needs_canonicalize(&hook), "{event}");
            assert!(!canonicalize(&mut hook), "{event}");
            assert_eq!(hook, before);
            assert!(!is_cursor_payload(&hook));
        }
    }

    #[test]
    fn pretooluse_shell_maps_to_canonical_envelope_and_is_idempotent() {
        // Documented preToolUse example, cursor.com/docs/agent/hooks.
        let mut hook = json!({
            "conversation_id": "conv-1", "generation_id": "gen-1",
            "hook_event_name": "preToolUse", "cursor_version": "2.4.0",
            "workspace_roots": ["/project"],
            "tool_name": "Shell",
            "tool_input": {"command": "npm install", "working_directory": "/project"},
            "tool_use_id": "abc123", "cwd": "/project"
        });
        assert!(canonicalize(&mut hook));
        assert_eq!(hook["hook_event_name"], "PreToolUse");
        assert_eq!(hook["session_id"], "conv-1");
        assert_eq!(hook["tool_name"], "Bash");
        assert_eq!(hook["tool_input"]["command"], "npm install");
        assert!(is_cursor_payload(&hook));
        let once = hook.clone();
        assert!(!needs_canonicalize(&hook));
        assert!(!canonicalize(&mut hook));
        assert_eq!(hook, once);
    }

    #[test]
    fn event_spelling_alone_stamps_the_version_field() {
        let mut hook = json!({"hook_event_name": "preToolUse", "tool_name": "Shell",
            "tool_input": {"command": "ls"}});
        assert!(!is_cursor_payload(&hook));
        assert!(canonicalize(&mut hook));
        assert!(is_cursor_payload(&hook));
        assert_eq!(hook["cursor_version"], "unknown");
    }

    #[test]
    fn shell_and_read_events_synthesise_a_tool() {
        let mut shell = json!({"hook_event_name": "beforeShellExecution",
            "command": "cargo clean", "cwd": "/p", "sandbox": false});
        assert!(canonicalize(&mut shell));
        assert_eq!(shell["tool_name"], "Bash");
        assert_eq!(shell["tool_input"]["command"], "cargo clean");
        assert!(shell.get("command").is_none());

        let mut missing = json!({"hook_event_name": "beforeShellExecution", "cwd": "/p"});
        assert!(canonicalize(&mut missing));
        assert!(missing["tool_input"]["command"].is_null());

        let mut read = json!({"hook_event_name": "beforeReadFile",
            "file_path": "/p/.env", "content": "SECRET=1"});
        assert!(canonicalize(&mut read));
        assert_eq!(read["tool_name"], "Read");
        assert_eq!(read["tool_input"]["file_path"], "/p/.env");
    }

    #[test]
    fn after_file_edit_passes_edits_through_as_a_post_edit() {
        let mut hook = json!({"hook_event_name": "afterFileEdit", "cursor_version": "2.4.0",
            "file_path": "/p/alembic/x.py",
            "edits": [{"old_string": "a", "new_string": "from alembic import op"}]});
        assert!(canonicalize(&mut hook));
        assert_eq!(hook["hook_event_name"], "PostToolUse");
        assert_eq!(hook["tool_name"], "Edit");
        assert_eq!(hook["tool_input"]["file_path"], "/p/alembic/x.py");
        assert_eq!(
            hook["tool_input"]["edits"][0]["new_string"],
            "from alembic import op"
        );
        assert!(hook.get("edits").is_none());
    }

    #[test]
    fn tool_names_params_and_file_aliases_are_normalised() {
        let mut delete = json!({"hook_event_name": "preToolUse", "tool_name": "Delete",
            "tool_input": {"target_file": "/p/a.py"}});
        assert!(canonicalize(&mut delete));
        assert_eq!(delete["tool_name"], "Edit");
        assert_eq!(delete["tool_input"]["file_path"], "/p/a.py");

        let mut write = json!({"hook_event_name": "preToolUse", "tool_name": "Write",
            "tool_input": {"path": "/p/a.py", "code_edit": "x"}});
        assert!(canonicalize(&mut write));
        assert_eq!(write["tool_input"]["file_path"], "/p/a.py");
        assert_eq!(write["tool_input"]["content"], "x");

        let mut keep = json!({"hook_event_name": "preToolUse", "tool_name": "Write",
            "tool_input": {"file_path": "/p/real.py", "path": "/p/decoy.py"}});
        assert!(canonicalize(&mut keep));
        assert_eq!(keep["tool_input"]["file_path"], "/p/real.py");

        // MCP params as a string, on the registered preToolUse event.
        let mut mcp = json!({"hook_event_name": "preToolUse",
            "tool_name": "linear_create_issue", "tool_input": "{\"title\":\"x\"}"});
        assert!(canonicalize(&mut mcp));
        assert_eq!(mcp["tool_input"]["title"], "x");
        let mut bad = json!({"hook_event_name": "beforeMCPExecution",
            "tool_name": "t", "tool_input": "not json"});
        assert!(canonicalize(&mut bad));
        assert!(bad["tool_input"].is_object());
    }

    #[test]
    fn unknown_cursor_events_become_passive_only_with_the_version_field() {
        let mut unknown = json!({"hook_event_name": "workspaceOpen", "cursor_version": "2.4.0"});
        assert!(canonicalize(&mut unknown));
        assert_eq!(unknown["hook_event_name"], "Notification");
        // Without the version field a lowercase name proves nothing.
        let mut bare = json!({"hook_event_name": "workspaceOpen"});
        assert!(!canonicalize(&mut bare));
        assert_eq!(bare["hook_event_name"], "workspaceOpen");
    }

    #[test]
    fn responses_use_cursor_field_names() {
        assert_eq!(allow(), json!({"permission": "allow"}));
        let denied = deny("why", "ctx");
        assert_eq!(denied["permission"], "deny");
        assert_eq!(denied["user_message"], "why");
        assert_eq!(denied["agent_message"], "why\nctx");
        assert_eq!(deny("why", "")["agent_message"], "why");
        assert_eq!(context("c"), json!({"additional_context": "c"}));
    }
}
