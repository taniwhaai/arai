//! Translate Codex's apply_patch wire format into the file actions understood
//! by the existing matcher. No filesystem reads or mutations are performed.

use serde_json::{json, Value};

pub(crate) struct FileAction {
    pub tool: &'static str,
    pub input: Value,
}

fn action(tool: &'static str, path: &str, old: &str, new: &str) -> FileAction {
    // The matcher tokenises file paths on '/'. Windows accepts both
    // separators, so normalise its native form before matching. Backslashes
    // remain literal filename characters on Unix and must not be rewritten.
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    FileAction {
        tool,
        input: json!({"file_path": path, "old_string": old, "new_string": new, "content": new}),
    }
}

fn checked_path(path: &str) -> Result<&str, String> {
    if path.trim().is_empty() || path.chars().any(char::is_control) {
        Err("Codex patch contains an empty or invalid path".into())
    } else {
        Ok(path)
    }
}

/// A command-shaped patch always wins over a claimed file_path. Only an
/// absent command with a valid path selects the older file_path-shaped alias.
pub(crate) fn is_patch_tool(raw_tool: &str, input: &Value) -> bool {
    raw_tool.eq_ignore_ascii_case("apply_patch")
        && (input.get("command").is_some()
            || input
                .get("file_path")
                .and_then(Value::as_str)
                .filter(|path| checked_path(path).is_ok())
                .is_none())
}

pub(crate) fn file_actions(input: &Value) -> Result<Vec<FileAction>, String> {
    let patch = input
        .get("command")
        .and_then(Value::as_str)
        .ok_or("Codex apply_patch requires tool_input.command")?;
    // Bound work for library callers too; the stdin path has the same cap.
    if patch.len() > 1024 * 1024 {
        return Err("Codex patch exceeds 1 MiB".into());
    }
    let lines: Vec<_> = patch.trim().lines().collect();
    if lines.first() != Some(&"*** Begin Patch") || lines.last() != Some(&"*** End Patch") {
        return Err("Codex patch must have Begin Patch and End Patch markers".into());
    }
    let mut actions = Vec::new();
    let mut i = 1;
    while i + 1 < lines.len() {
        let header = lines[i];
        i += 1;
        if let Some(path) = header.strip_prefix("*** Add File: ") {
            let path = checked_path(path)?;
            let mut content = String::new();
            while i + 1 < lines.len() && !lines[i].starts_with("*** ") {
                let added = lines[i]
                    .strip_prefix('+')
                    .ok_or("Codex added-file content must start with +")?;
                content.push_str(added);
                content.push('\n');
                i += 1;
            }
            actions.push(action("Write", path, "", &content));
        } else if let Some(path) = header.strip_prefix("*** Delete File: ") {
            // The current intent schema has no Delete tool. Evaluate the
            // path as Edit, so existing file/domain constraints still apply;
            // do not pretend this supplies delete-specific policy semantics.
            actions.push(action("Edit", checked_path(path)?, "", ""));
        } else if let Some(path) = header.strip_prefix("*** Update File: ") {
            let path = checked_path(path)?;
            let destination = if i + 1 < lines.len() {
                if let Some(p) = lines[i].strip_prefix("*** Move to: ") {
                    i += 1;
                    Some(checked_path(p)?)
                } else {
                    None
                }
            } else {
                None
            };
            let mut old = String::new();
            let mut new = String::new();
            let mut has_body = false;
            let mut needs_body = false;
            while i + 1 < lines.len() {
                let line = lines[i];
                if line == "*** End of File" {
                    if !has_body || needs_body {
                        return Err("Codex End of File marker requires a hunk".into());
                    }
                    i += 1;
                    break;
                }
                if line.starts_with("*** ") {
                    break;
                }
                if line == "@@" || line.starts_with("@@ ") {
                    if needs_body {
                        return Err("Codex update contains an empty hunk".into());
                    }
                    needs_body = true;
                } else if let Some(value) = line.strip_prefix('+') {
                    new.push_str(value);
                    new.push('\n');
                    has_body = true;
                    needs_body = false;
                } else if let Some(value) = line.strip_prefix('-') {
                    old.push_str(value);
                    old.push('\n');
                    has_body = true;
                    needs_body = false;
                } else if let Some(value) = line
                    .strip_prefix(' ')
                    .or_else(|| line.is_empty().then_some(""))
                {
                    old.push_str(value);
                    old.push('\n');
                    new.push_str(value);
                    new.push('\n');
                    has_body = true;
                    needs_body = false;
                } else {
                    return Err(format!("Unsupported Codex patch hunk line: {line}"));
                }
                i += 1;
            }
            if needs_body || (!has_body && destination.is_none()) {
                return Err("Codex update contains no complete hunk".into());
            }
            actions.push(action("Edit", path, &old, &new));
            if let Some(destination) = destination {
                // A move writes a new destination path. Checking only Edit
                // would let Create/Write prohibitions be bypassed by moving
                // a scratch file into a protected domain while changing it.
                actions.push(action("Write", destination, &old, &new));
                // Move-to can also overwrite an existing destination. Check
                // both scopes without a filesystem existence check/race.
                actions.push(action("Edit", destination, &old, &new));
            }
        } else {
            return Err(format!("Unsupported Codex patch operation: {header}"));
        }
    }
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_payload_cannot_downgrade_to_legacy_file_alias() {
        assert!(is_patch_tool(
            "apply_patch",
            &json!({
                "command": "*** Begin Patch\n*** Add File: alembic/x.py\n+pass\n*** End Patch",
                "file_path": "harmless.txt"
            })
        ));
        assert!(is_patch_tool(
            "apply_patch",
            &json!({
                "command": null, "file_path": "harmless.txt"
            })
        ));
        for path in [
            json!(null),
            json!(42),
            json!(""),
            json!("  "),
            json!("bad\npath"),
        ] {
            assert!(is_patch_tool("apply_patch", &json!({"file_path": path})));
        }
        assert!(!is_patch_tool(
            "apply_patch",
            &json!({"file_path": "legacy.py"})
        ));
        assert!(!is_patch_tool("Edit", &json!({"command": "not a patch"})));
    }

    #[test]
    fn patch_preserves_operations_paths_and_content() {
        let actions = file_actions(&json!({"command": "*** Begin Patch\n*** Add File: new file.py\n+from alembic import op\n*** Update File: src/old.py\n*** Move to: dst/new.py\n@@\n-old\n+new\n*** End of File\n*** Delete File: removed.py\n*** End Patch"})).unwrap();
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0].tool, "Write");
        assert_eq!(actions[0].input["file_path"], "new file.py");
        assert_eq!(actions[0].input["content"], "from alembic import op\n");
        assert_eq!(actions[1].input["old_string"], "old\n");
        assert_eq!(actions[1].tool, "Edit");
        assert_eq!(actions[2].tool, "Write");
        assert_eq!(actions[2].input["file_path"], "dst/new.py");
        assert_eq!(actions[3].tool, "Edit");
        assert_eq!(actions[3].input["file_path"], "dst/new.py");
        assert_eq!(actions[4].tool, "Edit");
    }

    #[test]
    fn windows_path_separators_follow_host_path_semantics() {
        let actions = file_actions(&json!({"command": "*** Begin Patch\n*** Add File: C:\\project\\docker\\Dockerfile\n+FROM scratch\n*** Update File: scratch.py\n*** Move to: alembic\\migration.py\n@@\n-pass\n+from alembic import op\n*** End Patch"})).unwrap();
        if cfg!(windows) {
            assert_eq!(
                actions[0].input["file_path"],
                "C:/project/docker/Dockerfile"
            );
            assert_eq!(actions[2].input["file_path"], "alembic/migration.py");
        } else {
            assert_eq!(
                actions[0].input["file_path"],
                "C:\\project\\docker\\Dockerfile"
            );
            assert_eq!(actions[2].input["file_path"], "alembic\\migration.py");
        }
        assert_eq!(actions[2].tool, "Write");
    }

    #[test]
    fn marker_like_added_content_is_not_an_operation() {
        let actions = file_actions(&json!({"command": "*** Begin Patch\n*** Add File: alembic/test.py\n+*** Delete File: harmless\n+++ harmless\n*** End Patch"})).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].input["file_path"], "alembic/test.py");
        assert_eq!(
            actions[0].input["content"],
            "*** Delete File: harmless\n++ harmless\n"
        );
    }

    #[test]
    fn malformed_patch_is_rejected() {
        for patch in [
            "",
            "*** Begin Patch\n*** Add File: x\n+hello",
            "*** Begin Patch\n*** Add File: \n+x\n*** End Patch",
            "*** Begin Patch\n*** Update File: x\n@@\n*** End Patch",
            "*** Begin Patch\n*** Update File: x\n@@\n@@\n+x\n*** End Patch",
            "*** Begin Patch\n*** Add File: x\nnot added\n*** End Patch",
            "*** Begin Patch\n*** Delete File: x\nignored\n*** End Patch",
        ] {
            assert!(file_actions(&json!({"command": patch})).is_err(), "{patch}");
        }
        assert!(file_actions(&json!({})).is_err());
    }
}
