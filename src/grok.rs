//! Pure adaptation of Grok's search_replace operation to canonical file actions.
//! An empty old_string can create a file or replace an existing file, so both
//! creation and modification policy must be checked without inspecting disk.

use serde_json::Value;

/// Returns None for other tools. Both CLI and embedding matchers should call
/// this before the name-only search_replace -> Edit compatibility alias, then
/// fold every returned action through the existing canonical matcher.
pub(crate) fn search_replace_actions(hook: &Value) -> Result<Option<Vec<Value>>, String> {
    let names = [hook.get("tool_name"), hook.get("toolName")];
    if !names.iter().flatten().any(|name| {
        name.as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("search_replace"))
    }) {
        return Ok(None);
    }
    let name = field(hook, "tool_name", "toolName")?
        .and_then(Value::as_str)
        .ok_or("Grok search_replace requires a tool name")?;
    if !name.eq_ignore_ascii_case("search_replace") {
        return Err("Conflicting Grok search_replace tool names".into());
    }
    let input = field(hook, "tool_input", "toolInput")?
        .and_then(Value::as_object)
        .ok_or("Grok search_replace requires a tool_input object")?;
    let path = input
        .get("file_path")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty() && !path.chars().any(char::is_control))
        .ok_or("Grok search_replace requires a nonempty valid file_path string")?;
    if input
        .get("path")
        .is_some_and(|alias| alias.as_str() != Some(path))
    {
        return Err("Conflicting Grok search_replace paths".into());
    }
    let old = input
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or("Grok search_replace requires an old_string string")?;
    let new = input
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or("Grok search_replace requires a new_string string")?;
    let mut canonical_input = Value::Object(input.clone());
    // Write and Edit must see the same new bytes. This also gives existing
    // Write-only consumers a content field without discarding edit evidence.
    canonical_input["content"] = Value::String(new.to_owned());
    let tools: &[&str] = if old.is_empty() {
        &["Write", "Edit"]
    } else {
        &["Edit"]
    };
    let actions = tools
        .iter()
        .map(|tool| {
            let mut action = hook.clone();
            action["tool_name"] = Value::String((*tool).to_owned());
            action["tool_input"] = canonical_input.clone();
            if action.get("toolName").is_some() {
                action["toolName"] = action["tool_name"].clone();
            }
            if action.get("toolInput").is_some() {
                action["toolInput"] = canonical_input.clone();
            }
            action
        })
        .collect();
    Ok(Some(actions))
}

fn field<'a>(hook: &'a Value, snake: &str, camel: &str) -> Result<Option<&'a Value>, String> {
    if let (Some(left), Some(right)) = (hook.get(snake), hook.get(camel)) {
        if left != right {
            return Err(format!("Conflicting Grok {snake}/{camel} fields"));
        }
    }
    Ok(hook.get(snake).or_else(|| hook.get(camel)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hook(old: &str) -> Value {
        json!({"hook_event_name":"pre_tool_use", "tool_name":"search_replace",
            "tool_input":{"file_path":"migration.py", "old_string":old, "new_string":"from alembic import op"},
            "session_id":"grok-session", "tool_use_id":"call-1", "cwd":"/project"})
    }

    #[test]
    fn replacing_existing_text_only_matches_edit_and_preserves_metadata() {
        let original = hook("pass");
        let actions = search_replace_actions(&original).unwrap().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["tool_name"], "Edit");
        assert_eq!(actions[0]["tool_input"]["old_string"], "pass");
        for key in ["hook_event_name", "session_id", "tool_use_id", "cwd"] {
            assert_eq!(actions[0][key], original[key]);
        }
        assert_eq!(original["tool_name"], "search_replace");
        assert!(original["tool_input"].get("content").is_none());
    }

    #[test]
    fn empty_search_checks_creation_and_modification_without_filesystem_guesses() {
        let actions = search_replace_actions(&hook("")).unwrap().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["tool_name"], "Write");
        assert_eq!(actions[1]["tool_name"], "Edit");
        assert_eq!(actions[0]["tool_input"], actions[1]["tool_input"]);
        assert_eq!(
            actions[0]["tool_input"]["content"],
            "from alembic import op"
        );
        // Whitespace is a real search string, not the empty/create form.
        assert_eq!(
            search_replace_actions(&hook(" ")).unwrap().unwrap().len(),
            1
        );
    }

    #[test]
    fn malformed_required_strings_and_ambiguous_aliases_fail_closed() {
        for (key, value) in [
            ("file_path", json!("")),
            ("file_path", json!("bad\0path")),
            ("file_path", Value::Null),
            ("old_string", Value::Null),
            ("new_string", json!(false)),
        ] {
            let mut payload = hook("");
            payload["tool_input"][key] = value;
            assert!(search_replace_actions(&payload).is_err(), "{payload}");
        }
        for key in ["file_path", "old_string", "new_string"] {
            let mut payload = hook("");
            payload["tool_input"].as_object_mut().unwrap().remove(key);
            assert!(search_replace_actions(&payload).is_err(), "{payload}");
        }
        let mut payload = hook("");
        payload["toolName"] = json!("Read");
        assert!(search_replace_actions(&payload).is_err());
        payload.as_object_mut().unwrap().remove("toolName");
        payload["toolInput"] = json!({});
        assert!(search_replace_actions(&payload).is_err());
        payload.as_object_mut().unwrap().remove("toolInput");
        payload["tool_input"]["path"] = json!("elsewhere.py");
        assert!(search_replace_actions(&payload).is_err());
    }

    #[test]
    fn camel_envelopes_are_preserved_and_other_tools_are_untouched() {
        let payload = json!({"hookEventName":"PreToolUse", "toolName":"search_replace",
            "toolInput":{"file_path":"x.py", "old_string":"", "new_string":""},
            "sessionId":"grok-camel"});
        let actions = search_replace_actions(&payload).unwrap().unwrap();
        assert_eq!(actions[0]["toolName"], "Write");
        assert_eq!(actions[1]["toolName"], "Edit");
        assert_eq!(actions[0]["toolInput"], actions[0]["tool_input"]);
        assert_eq!(actions[0]["sessionId"], "grok-camel");
        assert!(search_replace_actions(&json!({"tool_name":"Read"}))
            .unwrap()
            .is_none());
    }
}
